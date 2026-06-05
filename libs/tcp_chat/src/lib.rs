mod auth;
mod error;
mod protocol;

use std::collections::HashMap;
use std::io::{BufReader, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use auth::{
    auth_mode, build_auth_proof, derive_room_id, normalize_room_name, normalize_shared_secret,
    verify_auth_proof, AUTH_MODE_NONE as INTERNAL_AUTH_MODE_NONE,
    DEFAULT_ROOM_NAME as INTERNAL_DEFAULT_ROOM_NAME,
};
use mdns_sd::{DaemonEvent, IfKind, ServiceDaemon, ServiceEvent, ServiceInfo};
use protocol::{encode_frame, read_frame, WireEnvelope, WireMessage, PROTOCOL_VERSION};

pub use auth::{AUTH_MODE_HMAC_SHA256, AUTH_MODE_NONE, DEFAULT_ROOM_NAME};
pub use error::{LanChatError, Result};

const DEFAULT_PORT: u16 = 0;
const DEFAULT_SERVICE_TYPE: &str = "_tcp_chat._tcp.local.";
const IO_POLL_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone)]
pub struct ChatConfig {
    pub display_name: String,
    pub bind_addr: IpAddr,
    pub advertise_addr: IpAddr,
    pub hostname: String,
    pub port: u16,
    pub service_type: String,
    room_name: String,
    room_id: String,
    shared_secret: Option<String>,
    advertise_addr_explicit: bool,
    mdns_interface: Option<String>,
}

impl ChatConfig {
    pub fn new(display_name: impl Into<String>) -> Result<Self> {
        let display_name = normalize_display_name(display_name.into())?;
        let room_name = normalize_room_name(INTERNAL_DEFAULT_ROOM_NAME.to_string())?;
        let advertise_addr = local_ip_address::local_ip().map_err(|error| {
            LanChatError::Config(format!("failed to determine local IP address: {error}"))
        })?;
        let hostname = format!("{}.local.", hostname::get()?.to_string_lossy());

        Ok(Self {
            display_name,
            bind_addr: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            advertise_addr,
            hostname,
            port: DEFAULT_PORT,
            service_type: DEFAULT_SERVICE_TYPE.to_string(),
            room_id: derive_room_id(&room_name),
            room_name,
            shared_secret: None,
            advertise_addr_explicit: false,
            mdns_interface: None,
        })
    }

    pub fn with_room(mut self, room_name: impl Into<String>) -> Result<Self> {
        self.set_room(room_name)?;
        Ok(self)
    }

    pub fn set_room(&mut self, room_name: impl Into<String>) -> Result<()> {
        let room_name = normalize_room_name(room_name.into())?;
        self.room_id = derive_room_id(&room_name);
        self.room_name = room_name;
        Ok(())
    }

    pub fn with_shared_secret(mut self, shared_secret: impl Into<String>) -> Result<Self> {
        self.shared_secret = normalize_shared_secret(Some(shared_secret.into()))?;
        Ok(self)
    }

    pub fn with_bind_addr(mut self, bind_addr: IpAddr) -> Self {
        self.bind_addr = bind_addr;
        self
    }

    pub fn with_advertise_addr(mut self, advertise_addr: IpAddr) -> Self {
        self.advertise_addr = advertise_addr;
        self.advertise_addr_explicit = true;
        self
    }

    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    pub fn with_interface(mut self, interface_name: impl Into<String>) -> Result<Self> {
        self.set_interface(interface_name)?;
        Ok(self)
    }

    pub fn set_interface(&mut self, interface_name: impl Into<String>) -> Result<()> {
        self.mdns_interface = Some(normalize_interface_name(interface_name.into())?);
        Ok(())
    }

    pub fn clear_interface(mut self) -> Self {
        self.mdns_interface = None;
        self
    }

    pub fn interface_name(&self) -> Option<&str> {
        self.mdns_interface.as_deref()
    }

    pub fn advertises_explicit_addr(&self) -> bool {
        self.advertise_addr_explicit
    }

    pub fn clear_shared_secret(mut self) -> Self {
        self.shared_secret = None;
        self
    }

    pub fn room_name(&self) -> &str {
        &self.room_name
    }

    pub fn room_id(&self) -> &str {
        &self.room_id
    }

    pub fn auth_required(&self) -> bool {
        self.shared_secret.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkInterface {
    pub name: String,
    pub address: IpAddr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatRuntimeInfo {
    pub display_name: String,
    pub peer_id: String,
    pub room_name: String,
    pub room_id: String,
    pub auth_required: bool,
    pub bind_addr: IpAddr,
    pub listen_addr: SocketAddr,
    pub service_type: String,
    pub advertise_addr: Option<IpAddr>,
    pub mdns_interface: Option<String>,
    pub local_interfaces: Vec<NetworkInterface>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub peer_id: String,
    pub display_name: String,
    pub service_name: String,
    pub addresses: Vec<SocketAddr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub peer_id: String,
    pub display_name: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatEvent {
    PeerDiscovered(Peer),
    PeerConnected(Peer),
    PeerDisconnected(Peer),
    MessageReceived(ChatMessage),
    Warning(String),
}

pub struct LanChat {
    config: ChatConfig,
    identity: Arc<LocalIdentity>,
    running: Arc<AtomicBool>,
    peers: Arc<Mutex<HashMap<String, PeerState>>>,
    event_tx: Sender<ChatEvent>,
    mdns: ServiceDaemon,
    service_fullname: String,
    listen_addr: SocketAddr,
    accept_thread: Option<JoinHandle<()>>,
    discovery_thread: Option<JoinHandle<()>>,
    monitor_thread: Option<JoinHandle<()>>,
}

#[derive(Debug, Clone)]
struct LocalIdentity {
    peer_id: String,
    display_name: String,
    room_id: String,
    shared_secret: Option<String>,
}

#[derive(Debug)]
struct PeerState {
    peer: Peer,
    writer: Option<Arc<Mutex<TcpStream>>>,
    connecting: bool,
}

#[derive(Debug, Clone)]
struct PeerAdvertisement {
    peer: Peer,
    room_id: String,
    auth_mode: String,
    protocol_version: u8,
}

#[derive(Debug, Clone, Copy)]
enum ConnectionOrigin {
    Inbound,
    Outbound,
}

#[derive(Clone)]
struct RuntimeContext {
    peers: Arc<Mutex<HashMap<String, PeerState>>>,
    running: Arc<AtomicBool>,
    event_tx: Sender<ChatEvent>,
    identity: Arc<LocalIdentity>,
}

impl LocalIdentity {
    fn from_config(config: &ChatConfig, peer_id: String) -> Self {
        Self {
            peer_id,
            display_name: config.display_name.clone(),
            room_id: config.room_id.clone(),
            shared_secret: config.shared_secret.clone(),
        }
    }

    fn auth_mode(&self) -> &'static str {
        auth_mode(self.shared_secret.as_deref())
    }

    fn hello_envelope(&self) -> Result<WireEnvelope> {
        Ok(WireEnvelope::hello(
            self.peer_id.clone(),
            self.display_name.clone(),
            self.room_id.clone(),
            build_auth_proof(
                self.shared_secret.as_deref(),
                &self.room_id,
                &self.peer_id,
                &self.display_name,
            )?,
        ))
    }

    fn text_envelope(&self, body: impl Into<String>) -> WireEnvelope {
        WireEnvelope::text(
            self.peer_id.clone(),
            self.display_name.clone(),
            self.room_id.clone(),
            body,
        )
    }

    fn validate_hello(
        &self,
        envelope: &WireEnvelope,
        expected_peer_id: Option<&str>,
    ) -> Result<()> {
        self.validate_common(envelope, expected_peer_id)?;
        match &envelope.message {
            WireMessage::Hello { auth_proof } => verify_auth_proof(
                self.shared_secret.as_deref(),
                &self.room_id,
                &envelope.peer_id,
                &envelope.display_name,
                auth_proof.as_deref(),
            ),
            _ => Err(LanChatError::Protocol(
                "expected a hello frame during authentication".to_string(),
            )),
        }
    }

    fn validate_text(
        &self,
        envelope: &WireEnvelope,
        expected_peer_id: Option<&str>,
    ) -> Result<String> {
        self.validate_common(envelope, expected_peer_id)?;
        match &envelope.message {
            WireMessage::Text { body } => Ok(body.clone()),
            _ => Err(LanChatError::Protocol(
                "expected a text frame after authentication".to_string(),
            )),
        }
    }

    fn validate_common(
        &self,
        envelope: &WireEnvelope,
        expected_peer_id: Option<&str>,
    ) -> Result<()> {
        if envelope.room_id != self.room_id {
            return Err(LanChatError::Protocol(
                "received a frame for a different room".to_string(),
            ));
        }

        if envelope.display_name.trim().is_empty() {
            return Err(LanChatError::Protocol(
                "peer sent an empty display name".to_string(),
            ));
        }

        if let Some(expected_peer_id) = expected_peer_id {
            if envelope.peer_id != expected_peer_id {
                return Err(LanChatError::Protocol(format!(
                    "peer identity changed from {expected_peer_id} to {}",
                    envelope.peer_id
                )));
            }
        }

        Ok(())
    }
}

impl LanChat {
    pub fn start(config: ChatConfig) -> Result<(Self, Receiver<ChatEvent>)> {
        let local_interfaces = local_interfaces()?;
        validate_interface_selection(&config, &local_interfaces)?;

        let listener = TcpListener::bind(SocketAddr::new(config.bind_addr, config.port))?;
        listener.set_nonblocking(true)?;
        let listen_addr = listener.local_addr()?;

        let peer_id = new_peer_id(&config.display_name, config.advertise_addr);
        let identity = Arc::new(LocalIdentity::from_config(&config, peer_id));
        let service_name = service_instance_name(&config.display_name, &identity.peer_id);
        let txt_properties = HashMap::from([
            ("peer_id".to_string(), identity.peer_id.clone()),
            ("display_name".to_string(), config.display_name.clone()),
            ("room_name".to_string(), config.room_name.clone()),
            ("room_id".to_string(), config.room_id.clone()),
            ("auth_mode".to_string(), identity.auth_mode().to_string()),
            ("protocol_version".to_string(), PROTOCOL_VERSION.to_string()),
        ]);

        let mdns = ServiceDaemon::new()?;
        if should_limit_mdns_to_ipv4(&config) {
            mdns.disable_interface(IfKind::IPv6)?;
        }

        if let Some(interface_name) = config.interface_name() {
            mdns.disable_interface(IfKind::All)?;
            mdns.enable_interface(interface_name)?;
        }

        let service = if config.advertises_explicit_addr() {
            ServiceInfo::new(
                &config.service_type,
                &service_name,
                &config.hostname,
                config.advertise_addr,
                listen_addr.port(),
                txt_properties,
            )?
        } else {
            ServiceInfo::new(
                &config.service_type,
                &service_name,
                &config.hostname,
                "",
                listen_addr.port(),
                txt_properties,
            )?
            .enable_addr_auto()
        };

        let service_fullname = service.get_fullname().to_string();
        let monitor_receiver = mdns.monitor()?;
        mdns.register(service)?;

        let browse_receiver = mdns.browse(&config.service_type)?;
        let peers = Arc::new(Mutex::new(HashMap::new()));
        let running = Arc::new(AtomicBool::new(true));
        let (event_tx, event_rx) = mpsc::channel();
        let app_event_tx = event_tx.clone();
        let accept_runtime = RuntimeContext {
            peers: Arc::clone(&peers),
            running: Arc::clone(&running),
            event_tx: event_tx.clone(),
            identity: Arc::clone(&identity),
        };
        let discovery_runtime = RuntimeContext {
            peers: Arc::clone(&peers),
            running: Arc::clone(&running),
            event_tx,
            identity: Arc::clone(&identity),
        };

        let accept_thread = Some(spawn_accept_thread(listener, accept_runtime));
        let monitor_thread = Some(spawn_monitor_thread(
            monitor_receiver,
            Arc::clone(&running),
            app_event_tx.clone(),
        ));
        let discovery_thread = Some(spawn_discovery_thread(
            browse_receiver,
            discovery_runtime,
            service_fullname.clone(),
        ));

        if local_interfaces.len() > 1
            && config.interface_name().is_none()
            && !config.advertises_explicit_addr()
        {
            let interface_names = local_interfaces
                .iter()
                .map(|interface| interface.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let _ = app_event_tx.send(ChatEvent::Warning(format!(
                "multiple LAN interfaces detected ({interface_names}); if peers on other machines do not discover this instance, try --interface <name> or --advertise <ipv4-address>"
            )));
        }

        Ok((
            Self {
                config,
                identity,
                running,
                peers,
                event_tx: app_event_tx,
                mdns,
                service_fullname,
                listen_addr,
                accept_thread,
                discovery_thread,
                monitor_thread,
            },
            event_rx,
        ))
    }

    pub fn send(&self, body: impl Into<String>) -> Result<()> {
        let body = body.into();
        if body.trim().is_empty() {
            return Ok(());
        }

        let frame = encode_frame(&self.identity.text_envelope(body))?;
        let writers = {
            let peers = self
                .peers
                .lock()
                .expect("peer registry should not be poisoned");
            peers
                .iter()
                .filter_map(|(peer_id, state)| {
                    state
                        .writer
                        .as_ref()
                        .map(|writer| (peer_id.clone(), Arc::clone(writer)))
                })
                .collect::<Vec<_>>()
        };

        let mut disconnected = Vec::new();
        for (peer_id, writer) in writers {
            let mut writer = writer.lock().expect("writer lock should not be poisoned");
            if let Err(error) = writer.write_all(&frame).and_then(|_| writer.flush()) {
                disconnected.push((peer_id, error.to_string()));
            }
        }

        let mut first_error = None;
        for (peer_id, error) in disconnected {
            if let Some(peer) = mark_peer_disconnected(&self.peers, &peer_id) {
                let _ = self
                    .event_tx
                    .send(ChatEvent::PeerDisconnected(peer.clone()));
                if first_error.is_none() {
                    first_error = Some(LanChatError::Io(std::io::Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        format!("failed to send to {}: {error}", peer.display_name),
                    )));
                }
            }
        }

        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub fn peers(&self) -> Vec<Peer> {
        let mut peers = self
            .peers
            .lock()
            .expect("peer registry should not be poisoned")
            .values()
            .map(|state| state.peer.clone())
            .collect::<Vec<_>>();
        peers.sort_by(|left, right| left.display_name.cmp(&right.display_name));
        peers
    }

    pub fn room_name(&self) -> &str {
        self.config.room_name()
    }

    pub fn auth_required(&self) -> bool {
        self.config.auth_required()
    }

    pub fn listen_addr(&self) -> SocketAddr {
        self.listen_addr
    }

    pub fn runtime_info(&self) -> Result<ChatRuntimeInfo> {
        Ok(ChatRuntimeInfo {
            display_name: self.config.display_name.clone(),
            peer_id: self.identity.peer_id.clone(),
            room_name: self.config.room_name.clone(),
            room_id: self.config.room_id.clone(),
            auth_required: self.config.auth_required(),
            bind_addr: self.config.bind_addr,
            listen_addr: self.listen_addr,
            service_type: self.config.service_type.clone(),
            advertise_addr: self
                .config
                .advertises_explicit_addr()
                .then_some(self.config.advertise_addr),
            mdns_interface: self.config.interface_name().map(str::to_string),
            local_interfaces: local_interfaces()?,
        })
    }

    pub fn local_interfaces() -> Result<Vec<NetworkInterface>> {
        local_interfaces()
    }

    pub fn shutdown(&mut self) {
        self.running.store(false, Ordering::SeqCst);

        let _ = self.mdns.unregister(&self.service_fullname);
        let _ = self.mdns.shutdown();

        if let Some(handle) = self.accept_thread.take() {
            let _ = handle.join();
        }

        if let Some(handle) = self.discovery_thread.take() {
            let _ = handle.join();
        }

        if let Some(handle) = self.monitor_thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for LanChat {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn spawn_accept_thread(listener: TcpListener, ctx: RuntimeContext) -> JoinHandle<()> {
    thread::spawn(move || {
        while ctx.running.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, remote_addr)) => {
                    if let Err(error) = stream.set_nodelay(true) {
                        emit_warning(
                            &ctx.event_tx,
                            format!("failed to configure TCP stream: {error}"),
                        );
                    }
                    if let Err(error) = stream.set_read_timeout(Some(IO_POLL_INTERVAL)) {
                        emit_warning(
                            &ctx.event_tx,
                            format!("failed to configure TCP read timeout: {error}"),
                        );
                    }

                    let ctx = ctx.clone();
                    thread::spawn(move || {
                        handle_connection(
                            stream,
                            remote_addr,
                            ctx,
                            ConnectionOrigin::Inbound,
                            None,
                        );
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(IO_POLL_INTERVAL);
                }
                Err(error) => {
                    emit_warning(&ctx.event_tx, format!("listener error: {error}"));
                    thread::sleep(IO_POLL_INTERVAL);
                }
            }
        }
    })
}

fn spawn_monitor_thread(
    monitor_receiver: mdns_sd::Receiver<DaemonEvent>,
    running: Arc<AtomicBool>,
    event_tx: Sender<ChatEvent>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        while running.load(Ordering::SeqCst) {
            match monitor_receiver.recv() {
                Ok(DaemonEvent::Error(error)) => {
                    emit_warning(&event_tx, format!("mDNS daemon error: {error}"));
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    })
}

fn spawn_discovery_thread(
    browse_receiver: mdns_sd::Receiver<ServiceEvent>,
    ctx: RuntimeContext,
    self_service_fullname: String,
) -> JoinHandle<()> {
    thread::spawn(move || {
        while ctx.running.load(Ordering::SeqCst) {
            match browse_receiver.recv() {
                Ok(ServiceEvent::ServiceResolved(info)) => {
                    let Some(advertisement) = advertisement_from_service_info(&info) else {
                        emit_warning(
                            &ctx.event_tx,
                            format!(
                                "ignoring service {} because it is missing peer metadata",
                                info.get_fullname()
                            ),
                        );
                        continue;
                    };

                    if advertisement.peer.peer_id == ctx.identity.peer_id
                        || info.get_fullname() == self_service_fullname
                        || !is_compatible_advertisement(&advertisement, &ctx.identity)
                    {
                        continue;
                    }

                    let (should_connect, discovered_peer) = {
                        let mut registry = ctx
                            .peers
                            .lock()
                            .expect("peer registry should not be poisoned");
                        if let Some(existing) = registry.get_mut(&advertisement.peer.peer_id) {
                            existing.peer =
                                merge_peer(existing.peer.clone(), advertisement.peer.clone());
                            if existing.writer.is_none()
                                && !existing.connecting
                                && ctx.identity.peer_id < advertisement.peer.peer_id
                            {
                                existing.connecting = true;
                                (true, existing.peer.clone())
                            } else {
                                (false, existing.peer.clone())
                            }
                        } else {
                            let mut state = PeerState {
                                peer: advertisement.peer.clone(),
                                writer: None,
                                connecting: false,
                            };
                            let should_connect = ctx.identity.peer_id < advertisement.peer.peer_id;
                            if should_connect {
                                state.connecting = true;
                            }
                            let discovered_peer = state.peer.clone();
                            registry.insert(advertisement.peer.peer_id.clone(), state);
                            (should_connect, discovered_peer)
                        }
                    };

                    let _ = ctx
                        .event_tx
                        .send(ChatEvent::PeerDiscovered(discovered_peer.clone()));

                    if should_connect {
                        match connect_to_peer(discovered_peer.clone(), ctx.clone()) {
                            Ok(()) => {}
                            Err(error) => {
                                clear_connecting(&ctx.peers, &discovered_peer.peer_id);
                                emit_warning(
                                    &ctx.event_tx,
                                    format!(
                                        "failed to connect to discovered peer {}: {error}",
                                        discovered_peer.display_name
                                    ),
                                );
                            }
                        }
                    }
                }
                Ok(ServiceEvent::ServiceRemoved(_, fullname)) => {
                    let maybe_peer = {
                        let mut registry = ctx
                            .peers
                            .lock()
                            .expect("peer registry should not be poisoned");
                        let peer_id = registry.iter().find_map(|(peer_id, state)| {
                            (state.peer.service_name == fullname
                                && state.writer.is_none()
                                && !state.connecting)
                                .then(|| peer_id.clone())
                        });

                        peer_id
                            .and_then(|peer_id| registry.remove(&peer_id).map(|state| state.peer))
                    };

                    if let Some(peer) = maybe_peer {
                        let _ = ctx.event_tx.send(ChatEvent::PeerDisconnected(peer));
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    })
}

fn connect_to_peer(peer: Peer, ctx: RuntimeContext) -> Result<()> {
    let mut last_error = None;

    for address in peer.addresses.clone() {
        match TcpStream::connect(address) {
            Ok(stream) => {
                stream.set_nodelay(true)?;
                stream.set_read_timeout(Some(IO_POLL_INTERVAL))?;

                thread::spawn(move || {
                    handle_connection(stream, address, ctx, ConnectionOrigin::Outbound, Some(peer));
                });

                return Ok(());
            }
            Err(error) => last_error = Some(error),
        }
    }

    Err(LanChatError::Io(last_error.unwrap_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotConnected,
            format!("peer {} has no reachable addresses", peer.display_name),
        )
    })))
}

fn handle_connection(
    stream: TcpStream,
    remote_addr: SocketAddr,
    ctx: RuntimeContext,
    origin: ConnectionOrigin,
    mut known_peer: Option<Peer>,
) {
    let mut writer = match stream.try_clone() {
        Ok(writer) => writer,
        Err(error) => {
            emit_warning(
                &ctx.event_tx,
                format!("failed to clone stream for {remote_addr}: {error}"),
            );
            if let Some(peer) = known_peer {
                clear_connecting(&ctx.peers, &peer.peer_id);
            }
            return;
        }
    };

    let mut reader = BufReader::new(stream);
    let mut frame_buffer = Vec::new();
    let mut active_peer_id = None::<String>;
    let mut hello_sent = false;

    if matches!(origin, ConnectionOrigin::Outbound) {
        if let Err(error) = send_hello(&mut writer, &ctx.identity) {
            emit_warning(
                &ctx.event_tx,
                format!("failed to start authenticated handshake with {remote_addr}: {error}"),
            );
            if let Some(peer) = known_peer {
                clear_connecting(&ctx.peers, &peer.peer_id);
            }
            return;
        }
        hello_sent = true;
    }

    while ctx.running.load(Ordering::SeqCst) {
        match read_frame(&mut reader, &mut frame_buffer) {
            Ok(Some(envelope)) => match &envelope.message {
                WireMessage::Hello { .. } => {
                    let expected_peer_id = active_peer_id
                        .as_deref()
                        .or_else(|| known_peer.as_ref().map(|peer| peer.peer_id.as_str()));

                    if let Err(error) = ctx.identity.validate_hello(&envelope, expected_peer_id) {
                        emit_warning(
                            &ctx.event_tx,
                            format!(
                                "rejected unauthenticated peer {} at {remote_addr}: {error}",
                                envelope.display_name
                            ),
                        );
                        break;
                    }

                    if !hello_sent {
                        if let Err(error) = send_hello(&mut writer, &ctx.identity) {
                            emit_warning(
                                &ctx.event_tx,
                                format!(
                                    "failed to complete authenticated handshake with {remote_addr}: {error}"
                                ),
                            );
                            break;
                        }
                        hello_sent = true;
                    }

                    let peer = known_peer.clone().map_or_else(
                        || peer_from_envelope(&envelope, remote_addr),
                        |peer| merge_peer(peer, peer_from_envelope(&envelope, remote_addr)),
                    );
                    let (peer, connected_now) =
                        register_authenticated_peer(&ctx.peers, peer, writer.try_clone());
                    known_peer = Some(peer.clone());
                    active_peer_id = Some(peer.peer_id.clone());

                    if connected_now {
                        let _ = ctx.event_tx.send(ChatEvent::PeerConnected(peer));
                    }
                }
                WireMessage::Text { .. } => {
                    let Some(expected_peer_id) = active_peer_id.as_deref() else {
                        emit_warning(
                            &ctx.event_tx,
                            format!(
                                "rejected text frame from {remote_addr} before authentication completed"
                            ),
                        );
                        break;
                    };

                    let body = match ctx
                        .identity
                        .validate_text(&envelope, Some(expected_peer_id))
                    {
                        Ok(body) => body,
                        Err(error) => {
                            emit_warning(
                                &ctx.event_tx,
                                format!("rejected text frame from {remote_addr}: {error}"),
                            );
                            break;
                        }
                    };

                    let peer = known_peer.clone().map_or_else(
                        || peer_from_envelope(&envelope, remote_addr),
                        |peer| merge_peer(peer, peer_from_envelope(&envelope, remote_addr)),
                    );
                    let (peer, connected_now) =
                        register_authenticated_peer(&ctx.peers, peer, writer.try_clone());
                    known_peer = Some(peer.clone());

                    if connected_now {
                        let _ = ctx.event_tx.send(ChatEvent::PeerConnected(peer.clone()));
                    }

                    let _ = ctx.event_tx.send(ChatEvent::MessageReceived(ChatMessage {
                        peer_id: peer.peer_id,
                        display_name: peer.display_name,
                        body,
                    }));
                }
            },
            Ok(None) => break,
            Err(LanChatError::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(LanChatError::Io(error)) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => {
                emit_warning(
                    &ctx.event_tx,
                    format!("connection error from {remote_addr}: {error}"),
                );
                break;
            }
        }
    }

    if let Some(peer_id) = active_peer_id {
        if let Some(peer) = mark_peer_disconnected(&ctx.peers, &peer_id) {
            let _ = ctx.event_tx.send(ChatEvent::PeerDisconnected(peer));
        }
    } else if let Some(peer) = known_peer {
        clear_connecting(&ctx.peers, &peer.peer_id);
    }
}

fn send_hello(writer: &mut TcpStream, identity: &LocalIdentity) -> Result<()> {
    let hello_frame = encode_frame(&identity.hello_envelope()?)?;
    writer.write_all(&hello_frame)?;
    writer.flush()?;
    Ok(())
}

fn register_authenticated_peer(
    peers: &Arc<Mutex<HashMap<String, PeerState>>>,
    peer: Peer,
    writer: std::io::Result<TcpStream>,
) -> (Peer, bool) {
    let mut registry = peers.lock().expect("peer registry should not be poisoned");
    let entry = registry
        .entry(peer.peer_id.clone())
        .or_insert_with(|| PeerState {
            peer: peer.clone(),
            writer: None,
            connecting: false,
        });

    entry.peer = merge_peer(entry.peer.clone(), peer);
    entry.connecting = false;
    let connected_now = entry.writer.is_none();
    if let Ok(writer) = writer {
        entry.writer = Some(Arc::new(Mutex::new(writer)));
    }

    (entry.peer.clone(), connected_now)
}

fn clear_connecting(peers: &Arc<Mutex<HashMap<String, PeerState>>>, peer_id: &str) {
    let mut registry = peers.lock().expect("peer registry should not be poisoned");
    if let Some(state) = registry.get_mut(peer_id) {
        state.connecting = false;
    }
}

fn mark_peer_disconnected(
    peers: &Arc<Mutex<HashMap<String, PeerState>>>,
    peer_id: &str,
) -> Option<Peer> {
    let mut registry = peers.lock().expect("peer registry should not be poisoned");
    let state = registry.get_mut(peer_id)?;
    state.connecting = false;
    if state.writer.take().is_some() {
        Some(state.peer.clone())
    } else {
        None
    }
}

fn advertisement_from_service_info(info: &ServiceInfo) -> Option<PeerAdvertisement> {
    let peer = peer_from_service_info(info)?;
    let room_id = info.get_property_val_str("room_id")?.to_string();
    let auth_mode = info
        .get_property_val_str("auth_mode")
        .unwrap_or(INTERNAL_AUTH_MODE_NONE)
        .to_string();
    let protocol_version = info
        .get_property_val_str("protocol_version")
        .and_then(|value| value.parse::<u8>().ok())
        .unwrap_or(0);

    Some(PeerAdvertisement {
        peer,
        room_id,
        auth_mode,
        protocol_version,
    })
}

fn is_compatible_advertisement(
    advertisement: &PeerAdvertisement,
    identity: &LocalIdentity,
) -> bool {
    advertisement.room_id == identity.room_id
        && advertisement.auth_mode == identity.auth_mode()
        && advertisement.protocol_version == PROTOCOL_VERSION
}

fn peer_from_service_info(info: &ServiceInfo) -> Option<Peer> {
    let peer_id = info.get_property_val_str("peer_id")?.to_string();
    let display_name = info
        .get_property_val_str("display_name")
        .map(str::to_owned)
        .unwrap_or_else(|| peer_id.clone());
    let mut addresses = info
        .get_addresses()
        .iter()
        .map(|address| SocketAddr::new(*address, info.get_port()))
        .collect::<Vec<_>>();

    addresses.sort();
    addresses.dedup();

    Some(Peer {
        peer_id,
        display_name,
        service_name: info.get_fullname().to_string(),
        addresses,
    })
}

fn peer_from_envelope(envelope: &WireEnvelope, remote_addr: SocketAddr) -> Peer {
    Peer {
        peer_id: envelope.peer_id.clone(),
        display_name: envelope.display_name.clone(),
        service_name: String::new(),
        addresses: vec![remote_addr],
    }
}

fn merge_peer(existing: Peer, incoming: Peer) -> Peer {
    let mut addresses = existing.addresses;
    addresses.extend(incoming.addresses);
    addresses.sort();
    addresses.dedup();

    Peer {
        peer_id: incoming.peer_id,
        display_name: incoming.display_name,
        service_name: if incoming.service_name.is_empty() {
            existing.service_name
        } else {
            incoming.service_name
        },
        addresses,
    }
}

fn emit_warning(event_tx: &Sender<ChatEvent>, message: impl Into<String>) {
    let _ = event_tx.send(ChatEvent::Warning(message.into()));
}

fn normalize_display_name(display_name: String) -> Result<String> {
    let display_name = display_name.trim();
    if display_name.is_empty() {
        return Err(LanChatError::Config(
            "display name must contain at least one visible character".to_string(),
        ));
    }

    Ok(display_name.to_string())
}

fn normalize_interface_name(interface_name: String) -> Result<String> {
    let interface_name = interface_name.trim();
    if interface_name.is_empty() {
        return Err(LanChatError::Config(
            "interface name must contain at least one visible character".to_string(),
        ));
    }

    Ok(interface_name.to_string())
}

fn local_interfaces() -> Result<Vec<NetworkInterface>> {
    let mut interfaces = local_ip_address::list_afinet_netifas()
        .map_err(|error| {
            LanChatError::Config(format!(
                "failed to enumerate local IPv4 network interfaces: {error}"
            ))
        })?
        .into_iter()
        .filter(|(_, address)| address.is_ipv4() && !address.is_loopback())
        .map(|(name, address)| NetworkInterface { name, address })
        .collect::<Vec<_>>();

    interfaces.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.address.to_string().cmp(&right.address.to_string()))
    });
    interfaces.dedup_by(|left, right| left.name == right.name && left.address == right.address);
    Ok(interfaces)
}

fn validate_interface_selection(
    config: &ChatConfig,
    interfaces: &[NetworkInterface],
) -> Result<()> {
    let Some(interface_name) = config.interface_name() else {
        return Ok(());
    };

    if interfaces
        .iter()
        .any(|interface| interface.name == interface_name)
    {
        return Ok(());
    }

    let available = if interfaces.is_empty() {
        "none detected".to_string()
    } else {
        interfaces
            .iter()
            .map(|interface| format!("{} ({})", interface.name, interface.address))
            .collect::<Vec<_>>()
            .join(", ")
    };

    Err(LanChatError::Config(format!(
        "network interface {interface_name:?} was not found among active IPv4 interfaces: {available}"
    )))
}

fn should_limit_mdns_to_ipv4(config: &ChatConfig) -> bool {
    config.bind_addr.is_ipv4()
        && (!config.advertises_explicit_addr() || config.advertise_addr.is_ipv4())
}

fn service_instance_name(display_name: &str, peer_id: &str) -> String {
    let slug = display_name
        .chars()
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' => character.to_ascii_lowercase(),
            _ => '-',
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();

    let slug = if slug.is_empty() {
        "peer".to_string()
    } else {
        slug
    };

    format!("{slug}-{}", &peer_id[..8])
}

fn new_peer_id(display_name: &str, advertise_addr: IpAddr) -> String {
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = display_name
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .take(12)
        .collect::<String>()
        .to_ascii_lowercase();
    let name = if name.is_empty() { "peer" } else { &name };
    let address = advertise_addr
        .to_string()
        .chars()
        .map(|character| match character {
            '0'..='9' | 'a'..='f' | 'A'..='F' => character.to_ascii_lowercase(),
            _ => '-',
        })
        .collect::<String>();

    format!("{name}-{address}-{seed:x}")
}

#[cfg(test)]
mod tests {
    use super::{derive_room_id, merge_peer, service_instance_name, ChatConfig, Peer};
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    #[test]
    fn config_builder_populates_defaults() {
        let config = ChatConfig::new("Cuervo").expect("config should build");

        assert_eq!(config.display_name, "Cuervo");
        assert_eq!(config.bind_addr, IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        assert_eq!(config.room_name(), "default");
        assert_eq!(config.room_id(), derive_room_id("default"));
        assert!(!config.hostname.is_empty());
    }

    #[test]
    fn room_and_secret_builders_work() {
        let config = ChatConfig::new("Cuervo")
            .expect("config should build")
            .with_room("Studio A")
            .expect("room should be set")
            .with_shared_secret("secret")
            .expect("secret should be set");

        assert_eq!(config.room_name(), "Studio A");
        assert!(config.auth_required());
    }

    #[test]
    fn network_builders_track_explicit_settings() {
        let config = ChatConfig::new("Cuervo")
            .expect("config should build")
            .with_interface("en0")
            .expect("interface should be set")
            .with_advertise_addr(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 23)));

        assert_eq!(config.interface_name(), Some("en0"));
        assert!(config.advertises_explicit_addr());
    }

    #[test]
    fn service_names_are_stable_and_safe() {
        let service_name =
            service_instance_name("Cuervo Blanco", "12345678-1234-1234-1234-123456789abc");
        assert_eq!(service_name, "cuervo-blanco-12345678");
    }

    #[test]
    fn peer_merging_keeps_unique_addresses() {
        let left = Peer {
            peer_id: "peer-1".to_string(),
            display_name: "Left".to_string(),
            service_name: "left._tcp_chat._tcp.local.".to_string(),
            addresses: vec![SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 18_521)],
        };
        let right = Peer {
            peer_id: "peer-1".to_string(),
            display_name: "Left".to_string(),
            service_name: "left._tcp_chat._tcp.local.".to_string(),
            addresses: vec![
                SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 18_521),
                SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 40)), 18_521),
            ],
        };

        let merged = merge_peer(left, right);
        assert_eq!(merged.addresses.len(), 2);
    }
}
