use crate::{
    mdns_service::{MdnsService, PeerInfo, SharedPeerTable},
    preferences::{preferences_file_path, AppPreferences},
    settings::{
        ApplicationSettings, AudioDeviceInventory, AudioDeviceSelection, MAX_OPUS_PACKET_SIZE,
        OPUS_BITRATE_BPS, OPUS_FRAME_SIZE, OPUS_SAMPLE_RATE, SERVER_AUDIO_PORT,
    },
    transport::{current_time_in_ms, deserialize_packet, serialize_packet, PACKET_HEADER_SIZE},
    utils::{clear_terminal, sanitize_username},
};
use colored::*;
use cpal::{
    traits::{DeviceTrait, StreamTrait},
    FromSample, Sample, SampleFormat, SizedSample, Stream,
};
use local_ip_address::{list_afinet_netifas, local_ip};
use log::warn;
use opus::{Application, Bitrate, Decoder, Encoder};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    error::Error,
    io::{self, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{channel, Receiver, Sender},
        Arc, Mutex,
    },
    thread::JoinHandle,
    time::Instant,
};
use tcp_chat::{ChatConfig, ChatEvent, ChatRuntimeInfo, LanChat};

const MAX_PACKET_SIZE: usize = PACKET_HEADER_SIZE + MAX_OPUS_PACKET_SIZE;

pub fn run_from_env() -> Result<(), Box<dyn Error>> {
    let args = CliOptions::parse(std::env::args().skip(1))?;

    if args.show_help {
        print_usage();
        return Ok(());
    }

    if args.list_devices {
        print_available_devices()?;
        return Ok(());
    }

    if args.list_network {
        print_available_network()?;
        return Ok(());
    }

    match run(args) {
        Err(error) if is_user_canceled(error.as_ref()) => Ok(()),
        other => other,
    }
}

fn run(args: CliOptions) -> Result<(), Box<dyn Error>> {
    env_logger::init();

    let startup = resolve_startup_config(&args)?;
    let network_inventory = NetworkInventory::load()?;
    let network = resolve_network_interface(
        &network_inventory,
        startup.network_interface_name.as_deref(),
    )?;
    let settings = ApplicationSettings::from_device_selection(AudioDeviceSelection {
        input_device_name: startup.input_device_name.clone(),
        output_device_name: startup.output_device_name.clone(),
    })?;
    let inventory = ApplicationSettings::device_inventory()?;
    let username = startup.username;

    let bind_port = startup.bind_port.unwrap_or(SERVER_AUDIO_PORT);
    let socket = UdpSocket::bind(SocketAddr::new(IpAddr::V4(network.ip), bind_port))?;
    let local_addr = socket.local_addr()?;
    let local_peer = LocalPeer {
        instance_name: username.clone(),
        socket_addr: local_addr,
    };
    let runtime_network = RuntimeNetworkConfig {
        interface: network.clone(),
        bind_port,
        preferences_path: preferences_file_path()?,
    };

    let mdns = setup_mdns(&username, local_addr, Some(&network.name))?;
    let text_chat = start_text_chat(&username, &network)?;
    let peers = mdns.get_user_table();
    let route_selection = Arc::new(Mutex::new(RouteSelection::All));
    let tx_enabled = Arc::new(AtomicBool::new(false));
    let stats = Arc::new(RuntimeStats::default());

    let (frame_sender, frame_receiver) = channel();
    let playback_buffer = Arc::new(Mutex::new(VecDeque::with_capacity(OPUS_FRAME_SIZE * 16)));

    let input_stream =
        start_input_stream(&settings, frame_sender, tx_enabled.clone(), stats.clone())?;
    let output_stream = start_output_stream(&settings, playback_buffer.clone(), stats.clone())?;
    let sender_thread = start_sender_thread(
        socket.try_clone()?,
        frame_receiver,
        peers.clone(),
        route_selection.clone(),
        local_peer.clone(),
        stats.clone(),
    );
    let receiver_thread = start_receiver_thread(socket, playback_buffer, stats.clone());

    input_stream.play()?;
    output_stream.play()?;

    print_startup(
        &settings,
        &local_peer,
        &runtime_network,
        &text_chat.runtime_info,
    );
    print_help();

    let _runtime = PeerRuntime {
        _input_stream: input_stream,
        _output_stream: output_stream,
        _sender_thread: sender_thread,
        _receiver_thread: receiver_thread,
        _mdns: mdns,
        _text_chat: text_chat,
    };

    command_loop(
        peers,
        route_selection,
        tx_enabled,
        stats,
        local_peer,
        inventory,
        runtime_network,
        &_runtime._text_chat,
        &settings,
    )
}

struct PeerRuntime {
    _input_stream: Stream,
    _output_stream: Stream,
    _sender_thread: JoinHandle<()>,
    _receiver_thread: JoinHandle<()>,
    _mdns: MdnsService,
    _text_chat: TextChatRuntime,
}

#[derive(Clone, Debug)]
struct LocalPeer {
    instance_name: String,
    socket_addr: SocketAddr,
}

#[derive(Clone, Debug)]
struct StartupConfig {
    username: String,
    input_device_name: Option<String>,
    output_device_name: Option<String>,
    network_interface_name: Option<String>,
    bind_port: Option<u16>,
}

#[derive(Clone, Copy, Debug)]
enum DeviceKind {
    Input,
    Output,
}

impl DeviceKind {
    fn label(self) -> &'static str {
        match self {
            Self::Input => "microphone",
            Self::Output => "speakers",
        }
    }

    fn available_devices<'a>(self, inventory: &'a AudioDeviceInventory) -> &'a [String] {
        match self {
            Self::Input => &inventory.input_devices,
            Self::Output => &inventory.output_devices,
        }
    }

    fn default_device_name<'a>(self, inventory: &'a AudioDeviceInventory) -> Option<&'a str> {
        match self {
            Self::Input => inventory.default_input_device.as_deref(),
            Self::Output => inventory.default_output_device.as_deref(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NetworkInterfaceInfo {
    name: String,
    ip: Ipv4Addr,
}

impl NetworkInterfaceInfo {
    fn label(&self) -> String {
        format!("{} ({})", self.name, self.ip)
    }
}

#[derive(Clone, Debug, Default)]
struct NetworkInventory {
    interfaces: Vec<NetworkInterfaceInfo>,
    default_interface_name: Option<String>,
}

#[derive(Clone, Debug)]
struct RuntimeNetworkConfig {
    interface: NetworkInterfaceInfo,
    bind_port: u16,
    preferences_path: PathBuf,
}

struct TextChatRuntime {
    chat: LanChat,
    runtime_info: ChatRuntimeInfo,
    event_thread: Option<JoinHandle<()>>,
}

impl TextChatRuntime {
    fn send(&self, body: impl Into<String>) -> Result<(), Box<dyn Error>> {
        self.chat.send(body.into())?;
        Ok(())
    }

    fn peers(&self) -> Vec<tcp_chat::Peer> {
        self.chat.peers()
    }

    fn runtime_info(&self) -> Result<ChatRuntimeInfo, Box<dyn Error>> {
        Ok(self.chat.runtime_info()?)
    }
}

impl Drop for TextChatRuntime {
    fn drop(&mut self) {
        self.chat.shutdown();
        if let Some(handle) = self.event_thread.take() {
            let _ = handle.join();
        }
    }
}

#[derive(Clone, Debug)]
enum RouteSelection {
    All,
    None,
    Named(BTreeSet<String>),
}

impl RouteSelection {
    fn description(&self) -> String {
        match self {
            Self::All => "all discovered peers".to_string(),
            Self::None => "no peers".to_string(),
            Self::Named(names) => {
                let label = names.iter().cloned().collect::<Vec<_>>().join(", ");
                format!("selected peers: {label}")
            }
        }
    }

    fn selects(&self, peer_name: &str) -> bool {
        match self {
            Self::All => true,
            Self::None => false,
            Self::Named(names) => names.contains(peer_name),
        }
    }
}

#[derive(Default)]
struct RuntimeStats {
    captured_frames: AtomicU64,
    encoded_frames: AtomicU64,
    packets_sent: AtomicU64,
    packets_received: AtomicU64,
    decoded_packets: AtomicU64,
    concealed_packets: AtomicU64,
    malformed_packets: AtomicU64,
    stale_packets: AtomicU64,
    playback_underflows: AtomicU64,
    jitter_target_packets: AtomicUsize,
    jitter_ms: AtomicU64,
    rough_latency_ms: AtomicU64,
}

impl RuntimeStats {
    fn snapshot(&self) -> RuntimeStatsSnapshot {
        RuntimeStatsSnapshot {
            captured_frames: self.captured_frames.load(Ordering::Relaxed),
            encoded_frames: self.encoded_frames.load(Ordering::Relaxed),
            packets_sent: self.packets_sent.load(Ordering::Relaxed),
            packets_received: self.packets_received.load(Ordering::Relaxed),
            decoded_packets: self.decoded_packets.load(Ordering::Relaxed),
            concealed_packets: self.concealed_packets.load(Ordering::Relaxed),
            malformed_packets: self.malformed_packets.load(Ordering::Relaxed),
            stale_packets: self.stale_packets.load(Ordering::Relaxed),
            playback_underflows: self.playback_underflows.load(Ordering::Relaxed),
            jitter_target_packets: self.jitter_target_packets.load(Ordering::Relaxed),
            jitter_ms: self.jitter_ms.load(Ordering::Relaxed),
            rough_latency_ms: self.rough_latency_ms.load(Ordering::Relaxed),
        }
    }
}

struct RuntimeStatsSnapshot {
    captured_frames: u64,
    encoded_frames: u64,
    packets_sent: u64,
    packets_received: u64,
    decoded_packets: u64,
    concealed_packets: u64,
    malformed_packets: u64,
    stale_packets: u64,
    playback_underflows: u64,
    jitter_target_packets: usize,
    jitter_ms: u64,
    rough_latency_ms: u64,
}

struct JitterController {
    last_arrival: Option<Instant>,
    jitter_ms: f32,
    underflow_boost: usize,
    last_underflow_count: u64,
    target_packets: usize,
}

impl JitterController {
    fn new() -> Self {
        Self {
            last_arrival: None,
            jitter_ms: 0.0,
            underflow_boost: 0,
            last_underflow_count: 0,
            target_packets: 2,
        }
    }

    fn observe_packet(&mut self, now: Instant, stats: &RuntimeStats) {
        if let Some(previous_arrival) = self.last_arrival {
            let inter_arrival_ms = now.duration_since(previous_arrival).as_secs_f32() * 1_000.0;
            let expected_ms = OPUS_FRAME_SIZE as f32 / OPUS_SAMPLE_RATE as f32 * 1_000.0;
            let deviation = (inter_arrival_ms - expected_ms).abs();
            self.jitter_ms = self.jitter_ms * 0.85 + deviation * 0.15;
        }
        self.last_arrival = Some(now);

        let underflows = stats.playback_underflows.load(Ordering::Relaxed);
        if underflows > self.last_underflow_count {
            self.underflow_boost = (self.underflow_boost + 1).min(4);
            self.last_underflow_count = underflows;
        } else if self.underflow_boost > 0 && self.jitter_ms < 4.0 {
            self.underflow_boost -= 1;
        }

        let adaptive_target = 2 + (self.jitter_ms / 10.0).ceil() as usize + self.underflow_boost;
        self.target_packets = adaptive_target.clamp(2, 8);

        stats
            .jitter_target_packets
            .store(self.target_packets, Ordering::Relaxed);
        stats
            .jitter_ms
            .store(self.jitter_ms.round() as u64, Ordering::Relaxed);
    }

    fn target_packets(&self) -> usize {
        self.target_packets
    }
}

#[derive(Clone, Debug, Default)]
struct CliOptions {
    username: Option<String>,
    input_device_name: Option<String>,
    output_device_name: Option<String>,
    network_interface_name: Option<String>,
    bind_port: Option<u16>,
    list_devices: bool,
    list_network: bool,
    setup: bool,
    show_help: bool,
}

impl CliOptions {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self, Box<dyn Error>> {
        let mut options = Self::default();
        let mut args = args.peekable();

        while let Some(argument) = args.next() {
            match argument.as_str() {
                "--username" => {
                    options.username = Some(next_arg(&mut args, "--username")?);
                }
                "--input-device" => {
                    options.input_device_name = Some(next_arg(&mut args, "--input-device")?);
                }
                "--output-device" => {
                    options.output_device_name = Some(next_arg(&mut args, "--output-device")?);
                }
                "--interface" => {
                    options.network_interface_name = Some(next_arg(&mut args, "--interface")?);
                }
                "--bind-port" => {
                    let raw = next_arg(&mut args, "--bind-port")?;
                    options.bind_port = Some(raw.parse()?);
                }
                "--list-devices" => options.list_devices = true,
                "--list-network" => options.list_network = true,
                "--setup" => options.setup = true,
                "--help" | "-h" => options.show_help = true,
                unknown => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("Unknown argument: {unknown}. Run with --help for usage."),
                    )
                    .into())
                }
            }
        }

        Ok(options)
    }

    fn has_complete_audio_and_identity_setup(&self, preferences: &AppPreferences) -> bool {
        merge_optional_string(self.username.as_deref(), preferences.username.as_deref())
            .as_deref()
            .map(sanitize_username)
            .filter(|username| !username.is_empty())
            .is_some()
            && merge_optional_string(
                self.input_device_name.as_deref(),
                preferences.input_device_name.as_deref(),
            )
            .is_some()
            && merge_optional_string(
                self.output_device_name.as_deref(),
                preferences.output_device_name.as_deref(),
            )
            .is_some()
    }
}

fn next_arg(
    args: &mut std::iter::Peekable<impl Iterator<Item = String>>,
    flag: &str,
) -> Result<String, Box<dyn Error>> {
    args.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Missing value for {flag}."),
        )
        .into()
    })
}

fn is_user_canceled(error: &(dyn Error + 'static)) -> bool {
    error
        .downcast_ref::<io::Error>()
        .map(|error| error.kind() == io::ErrorKind::Interrupted)
        .unwrap_or(false)
}

fn print_usage() {
    println!("udp_voice peer");
    println!("Usage:");
    println!("  cargo run --bin peer -- [options]");
    println!();
    println!("Without saved or explicit setup values, the app opens an interactive setup screen.");
    println!();
    println!("Options:");
    println!("  --username <name>         Set the local peer name");
    println!("  --input-device <name>     Choose a specific microphone/input device");
    println!("  --output-device <name>    Choose a specific speaker/output device");
    println!("  --interface <name>        Choose a specific network interface");
    println!("  --bind-port <port>        Bind the peer's UDP socket to a fixed local port");
    println!("  --list-devices            Print available input/output devices and exit");
    println!("  --list-network            Print available network interfaces and exit");
    println!("  --setup                   Force the interactive setup screen");
    println!("  --help                    Show this message");
}

fn print_available_devices() -> Result<(), Box<dyn Error>> {
    let inventory = ApplicationSettings::device_inventory()?;
    println!("Input devices:");
    println!(
        "  System default: {}",
        inventory
            .default_input_device
            .as_deref()
            .unwrap_or("(unavailable)")
    );
    if inventory.input_devices.is_empty() {
        println!("  (none)");
    } else {
        for device in &inventory.input_devices {
            println!("  {device}");
        }
    }

    println!();
    println!("Output devices:");
    println!(
        "  System default: {}",
        inventory
            .default_output_device
            .as_deref()
            .unwrap_or("(unavailable)")
    );
    if inventory.output_devices.is_empty() {
        println!("  (none)");
    } else {
        for device in &inventory.output_devices {
            println!("  {device}");
        }
    }

    Ok(())
}

fn print_available_network() -> Result<(), Box<dyn Error>> {
    let inventory = NetworkInventory::load()?;
    let preferences = AppPreferences::load()?;
    let preferences_path = preferences_file_path()?;

    println!("Network interfaces:");
    println!(
        "  Preferred default: {}",
        inventory
            .default_interface_name
            .as_deref()
            .unwrap_or("(unavailable)")
    );
    if inventory.interfaces.is_empty() {
        println!("  (none)");
    } else {
        for interface in &inventory.interfaces {
            let mut labels = Vec::new();
            if inventory.default_interface_name.as_deref() == Some(interface.name.as_str()) {
                labels.push("default");
            }
            if preferences.network_interface_name.as_deref() == Some(interface.name.as_str()) {
                labels.push("saved");
            }

            let label_suffix = if labels.is_empty() {
                String::new()
            } else {
                format!(" [{}]", labels.join(", "))
            };

            println!("  {}{}", interface.label(), label_suffix);
        }
    }

    println!();
    println!("Port preference:");
    println!(
        "  {}",
        describe_bind_port(preferences.bind_port.unwrap_or(SERVER_AUDIO_PORT))
    );
    println!("Preferences file: {}", preferences_path.display());

    Ok(())
}

fn resolve_startup_config(args: &CliOptions) -> Result<StartupConfig, Box<dyn Error>> {
    let preferences = AppPreferences::load()?;
    let base = merge_startup_config(args, &preferences);
    let network_inventory = NetworkInventory::load()?;
    let needs_network_reselection =
        resolve_network_interface(&network_inventory, base.network_interface_name.as_deref())
            .is_err();

    if args.setup
        || !args.has_complete_audio_and_identity_setup(&preferences)
        || needs_network_reselection
    {
        launch_setup_wizard(base, &preferences)
    } else {
        Ok(base)
    }
}

fn merge_startup_config(args: &CliOptions, preferences: &AppPreferences) -> StartupConfig {
    StartupConfig {
        username: sanitize_or_default_username(
            merge_optional_string(args.username.as_deref(), preferences.username.as_deref())
                .as_deref(),
        ),
        input_device_name: merge_optional_string(
            args.input_device_name.as_deref(),
            preferences.input_device_name.as_deref(),
        ),
        output_device_name: merge_optional_string(
            args.output_device_name.as_deref(),
            preferences.output_device_name.as_deref(),
        ),
        network_interface_name: merge_optional_string(
            args.network_interface_name.as_deref(),
            preferences.network_interface_name.as_deref(),
        ),
        bind_port: args
            .bind_port
            .or(preferences.bind_port)
            .or(Some(SERVER_AUDIO_PORT)),
    }
}

fn launch_setup_wizard(
    mut config: StartupConfig,
    loaded_preferences: &AppPreferences,
) -> Result<StartupConfig, Box<dyn Error>> {
    let mut inventory = ApplicationSettings::device_inventory()?;
    let mut network_inventory = NetworkInventory::load()?;
    let mut saved_preferences = loaded_preferences.clone();
    let mut status =
        Some("Pick your username, audio devices, network interface, and UDP port.".to_string());

    loop {
        render_setup_wizard(
            &config,
            &inventory,
            &network_inventory,
            &saved_preferences,
            status.as_deref(),
        );
        let input = read_user_input()?;

        match input.as_str() {
            "1" => {
                if let Some(username) = prompt_for_username(&config.username)? {
                    config.username = username;
                    status = Some(format!("Username set to {}.", config.username));
                }
            }
            "2" => {
                config.input_device_name = choose_device(
                    DeviceKind::Input,
                    &inventory,
                    config.input_device_name.as_deref(),
                )?;
                status = Some(format!(
                    "Microphone set to {}.",
                    describe_selected_device(
                        config.input_device_name.as_deref(),
                        inventory.default_input_device.as_deref()
                    )
                ));
            }
            "3" => {
                config.output_device_name = choose_device(
                    DeviceKind::Output,
                    &inventory,
                    config.output_device_name.as_deref(),
                )?;
                status = Some(format!(
                    "Speakers set to {}.",
                    describe_selected_device(
                        config.output_device_name.as_deref(),
                        inventory.default_output_device.as_deref()
                    )
                ));
            }
            "4" => {
                config.network_interface_name = choose_network_interface(
                    &network_inventory,
                    config.network_interface_name.as_deref(),
                )?;
                status = Some(format!(
                    "Network interface set to {}.",
                    describe_network_selection(
                        &network_inventory,
                        config.network_interface_name.as_deref()
                    )
                ));
            }
            "5" => match prompt_for_bind_port(config.bind_port) {
                Ok(port) => {
                    config.bind_port = port;
                    status = Some(format!(
                        "UDP port set to {}.",
                        describe_bind_port(config.bind_port.unwrap_or(SERVER_AUDIO_PORT))
                    ));
                }
                Err(error) => {
                    status = Some(error.to_string());
                }
            },
            "6" => {
                saved_preferences = AppPreferences {
                    username: Some(config.username.clone()),
                    input_device_name: config.input_device_name.clone(),
                    output_device_name: config.output_device_name.clone(),
                    network_interface_name: config.network_interface_name.clone(),
                    bind_port: config.bind_port,
                };
                match saved_preferences.save() {
                    Ok(saved_path) => {
                        status = Some(format!("Preferences saved to {}.", saved_path.display()));
                    }
                    Err(error) => {
                        status = Some(format!("Could not save preferences: {error}"));
                    }
                }
            }
            "7" => {
                inventory = ApplicationSettings::device_inventory()?;
                network_inventory = NetworkInventory::load()?;
                status = Some("Audio and network lists refreshed.".to_string());
            }
            "8" | "start" => {
                if config.username.trim().is_empty() {
                    status = Some("Username cannot be empty.".to_string());
                    continue;
                }

                if resolve_network_interface(
                    &network_inventory,
                    config.network_interface_name.as_deref(),
                )
                .is_err()
                {
                    status =
                        Some("Select a usable IPv4 network interface before starting.".to_string());
                    continue;
                }

                match ApplicationSettings::from_device_selection(AudioDeviceSelection {
                    input_device_name: config.input_device_name.clone(),
                    output_device_name: config.output_device_name.clone(),
                }) {
                    Ok(_) => {
                        clear_terminal();
                        return Ok(config);
                    }
                    Err(error) => {
                        status = Some(format!("Cannot start yet: {error}"));
                    }
                }
            }
            "9" | "q" | "quit" | "exit" => {
                return Err(
                    io::Error::new(io::ErrorKind::Interrupted, "Setup canceled by user.").into(),
                );
            }
            _ => {
                status = Some("Choose 1-9 to continue.".to_string());
            }
        }
    }
}

fn render_setup_wizard(
    config: &StartupConfig,
    inventory: &AudioDeviceInventory,
    network_inventory: &NetworkInventory,
    loaded_preferences: &AppPreferences,
    status: Option<&str>,
) {
    clear_terminal();
    println!("{}", "UDP Voice Setup".green().bold());
    println!("Configure your local name, audio, and networking before joining.");
    println!();
    println!("  1. Username      {}", config.username.cyan());
    println!(
        "  2. Microphone    {}",
        describe_selected_device(
            config.input_device_name.as_deref(),
            inventory.default_input_device.as_deref()
        )
    );
    println!(
        "  3. Speakers      {}",
        describe_selected_device(
            config.output_device_name.as_deref(),
            inventory.default_output_device.as_deref()
        )
    );
    println!(
        "  4. Interface     {}",
        describe_network_selection(network_inventory, config.network_interface_name.as_deref())
    );
    println!(
        "  5. UDP port      {}",
        describe_bind_port(config.bind_port.unwrap_or(SERVER_AUDIO_PORT))
    );
    println!("  6. Save preferences");
    println!("  7. Refresh devices/interfaces");
    println!("  8. Start peer");
    println!("  9. Quit");
    println!();
    println!(
        "System default mic: {}",
        inventory
            .default_input_device
            .as_deref()
            .unwrap_or("(unavailable)")
    );
    println!(
        "System default speakers: {}",
        inventory
            .default_output_device
            .as_deref()
            .unwrap_or("(unavailable)")
    );
    println!(
        "Preferred interface: {}",
        network_inventory
            .default_interface_name
            .as_deref()
            .unwrap_or("(unavailable)")
    );
    println!(
        "Detected devices: {} input, {} output, {} network",
        inventory.input_devices.len(),
        inventory.output_devices.len(),
        network_inventory.interfaces.len()
    );
    println!(
        "Preferences file: {}",
        preferences_file_path()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| "<unavailable>".to_string())
    );
    println!(
        "Saved defaults: user={}, mic={}, speakers={}, interface={}, port={}",
        loaded_preferences.username.as_deref().unwrap_or("(none)"),
        loaded_preferences
            .input_device_name
            .as_deref()
            .unwrap_or("(none)"),
        loaded_preferences
            .output_device_name
            .as_deref()
            .unwrap_or("(none)"),
        loaded_preferences
            .network_interface_name
            .as_deref()
            .unwrap_or("(none)"),
        loaded_preferences
            .bind_port
            .map(|port| port.to_string())
            .unwrap_or_else(|| "(none)".to_string()),
    );
    if let Some(status) = status {
        println!();
        println!("Status: {status}");
    }
    println!();
    print!("Choose an option: ");
    io::stdout().flush().unwrap();
}

fn prompt_for_username(current_username: &str) -> Result<Option<String>, Box<dyn Error>> {
    clear_terminal();
    println!("{}", "Edit Username".green().bold());
    println!("Current username: {}", current_username.cyan());
    println!("Enter a new username, or leave it blank to keep the current one.");
    println!();
    print!("Username: ");
    io::stdout().flush()?;

    let input = read_user_input()?;
    if input.is_empty() {
        return Ok(None);
    }

    let username = sanitize_username(&input);
    if username.is_empty() {
        return Ok(None);
    }

    Ok(Some(username))
}

fn choose_device(
    kind: DeviceKind,
    inventory: &AudioDeviceInventory,
    current_selection: Option<&str>,
) -> Result<Option<String>, Box<dyn Error>> {
    let mut status: Option<String> = None;

    loop {
        clear_terminal();
        println!("{}", format!("Select {}", kind.label()).green().bold());
        println!(
            "Current selection: {}",
            describe_selected_device(current_selection, kind.default_device_name(inventory))
        );
        println!();
        println!(
            "  0. System default ({})",
            kind.default_device_name(inventory).unwrap_or("unavailable")
        );

        let devices = kind.available_devices(inventory);
        if devices.is_empty() {
            println!("  No {} devices detected.", kind.label());
        } else {
            for (index, device) in devices.iter().enumerate() {
                let mut labels = Vec::new();
                if current_selection == Some(device.as_str()) {
                    labels.push("current");
                }
                if kind.default_device_name(inventory) == Some(device.as_str()) {
                    labels.push("system default");
                }

                let label_suffix = if labels.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", labels.join(", "))
                };

                println!("  {}. {}{}", index + 1, device, label_suffix);
            }
        }

        println!("  b. Back");
        if let Some(status) = status.as_deref() {
            println!();
            println!("Status: {status}");
        }
        println!();
        print!("Choose {}: ", kind.label());
        io::stdout().flush()?;

        let input = read_user_input()?;
        if matches!(input.as_str(), "b" | "back") {
            return Ok(current_selection.map(str::to_owned));
        }

        match input.parse::<usize>() {
            Ok(0) => return Ok(None),
            Ok(choice) if choice >= 1 && choice <= devices.len() => {
                return Ok(Some(devices[choice - 1].clone()))
            }
            _ => {
                status = Some(format!("Enter 0-{} or b to go back.", devices.len()));
            }
        }
    }
}

fn choose_network_interface(
    inventory: &NetworkInventory,
    current_selection: Option<&str>,
) -> Result<Option<String>, Box<dyn Error>> {
    let mut status: Option<String> = None;

    loop {
        clear_terminal();
        println!("{}", "Select network interface".green().bold());
        println!(
            "Current selection: {}",
            describe_network_selection(inventory, current_selection)
        );
        println!();
        println!(
            "  0. Preferred default ({})",
            inventory
                .default_interface_name
                .as_deref()
                .unwrap_or("unavailable")
        );

        if inventory.interfaces.is_empty() {
            println!("  No usable IPv4 interfaces detected.");
        } else {
            for (index, interface) in inventory.interfaces.iter().enumerate() {
                let mut labels = Vec::new();
                if current_selection == Some(interface.name.as_str()) {
                    labels.push("current");
                }
                if inventory.default_interface_name.as_deref() == Some(interface.name.as_str()) {
                    labels.push("default");
                }

                let label_suffix = if labels.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", labels.join(", "))
                };

                println!("  {}. {}{}", index + 1, interface.label(), label_suffix);
            }
        }

        println!("  b. Back");
        if let Some(status) = status.as_deref() {
            println!();
            println!("Status: {status}");
        }
        println!();
        print!("Choose interface: ");
        io::stdout().flush()?;

        let input = read_user_input()?;
        if matches!(input.as_str(), "b" | "back") {
            return Ok(current_selection.map(str::to_owned));
        }

        match input.parse::<usize>() {
            Ok(0) => return Ok(None),
            Ok(choice) if choice >= 1 && choice <= inventory.interfaces.len() => {
                return Ok(Some(inventory.interfaces[choice - 1].name.clone()))
            }
            _ => {
                status = Some(format!(
                    "Enter 0-{} or b to go back.",
                    inventory.interfaces.len()
                ));
            }
        }
    }
}

fn prompt_for_bind_port(current_port: Option<u16>) -> Result<Option<u16>, Box<dyn Error>> {
    let mut status: Option<String> = None;

    loop {
        clear_terminal();
        println!("{}", "Set UDP Port".green().bold());
        println!(
            "Current port: {}",
            describe_bind_port(current_port.unwrap_or(SERVER_AUDIO_PORT))
        );
        println!(
            "Enter a UDP port from 1-65535, or leave it blank to use {}.",
            SERVER_AUDIO_PORT
        );
        println!("Type `b` to go back.");
        if let Some(status) = status.as_deref() {
            println!();
            println!("Status: {status}");
        }
        println!();
        print!("Port: ");
        io::stdout().flush()?;

        let input = read_user_input()?;
        if matches!(input.as_str(), "b" | "back") {
            return Ok(current_port);
        }
        if input.is_empty() {
            return Ok(current_port.or(Some(SERVER_AUDIO_PORT)));
        }

        match input.parse::<u16>() {
            Ok(0) => {
                status = Some("Port 0 is not allowed here.".to_string());
            }
            Ok(port) => return Ok(Some(port)),
            Err(_) => {
                status = Some("Port must be a number between 1 and 65535.".to_string());
            }
        }
    }
}

fn merge_optional_string(primary: Option<&str>, fallback: Option<&str>) -> Option<String> {
    primary.or(fallback).map(str::to_owned)
}

impl NetworkInventory {
    fn load() -> Result<Self, Box<dyn Error>> {
        let mut interfaces = list_afinet_netifas()?
            .into_iter()
            .filter_map(|(name, ip)| match ip {
                IpAddr::V4(ipv4) if !ipv4.is_loopback() => {
                    Some(NetworkInterfaceInfo { name, ip: ipv4 })
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        interfaces.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.ip.octets().cmp(&right.ip.octets()))
        });
        interfaces.dedup_by(|left, right| left.name == right.name && left.ip == right.ip);

        let default_interface_name = local_ip()
            .ok()
            .and_then(|ip| match ip {
                IpAddr::V4(ipv4) => interfaces
                    .iter()
                    .find(|interface| interface.ip == ipv4)
                    .map(|interface| interface.name.clone()),
                _ => None,
            })
            .or_else(|| interfaces.first().map(|interface| interface.name.clone()));

        Ok(Self {
            interfaces,
            default_interface_name,
        })
    }
}

fn resolve_network_interface(
    inventory: &NetworkInventory,
    requested_name: Option<&str>,
) -> Result<NetworkInterfaceInfo, Box<dyn Error>> {
    if let Some(requested_name) = requested_name {
        if let Some(interface) = inventory
            .interfaces
            .iter()
            .find(|interface| interface.name.eq_ignore_ascii_case(requested_name))
        {
            return Ok(interface.clone());
        }

        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "Could not find network interface \"{requested_name}\". Available interfaces: {}",
                join_network_names(&inventory.interfaces)
            ),
        )
        .into());
    }

    if let Some(default_name) = inventory.default_interface_name.as_deref() {
        if let Some(interface) = inventory
            .interfaces
            .iter()
            .find(|interface| interface.name == default_name)
        {
            return Ok(interface.clone());
        }
    }

    inventory.interfaces.first().cloned().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "No usable IPv4 network interfaces were detected.",
        )
        .into()
    })
}

fn describe_network_selection(inventory: &NetworkInventory, selected_name: Option<&str>) -> String {
    match resolve_network_interface(inventory, selected_name) {
        Ok(interface) => interface.label(),
        Err(_) => match selected_name {
            Some(name) => format!("{name} (unavailable)"),
            None => "Preferred default (unavailable)".to_string(),
        },
    }
}

fn describe_bind_port(port: u16) -> String {
    format!("{port}")
}

fn join_network_names(interfaces: &[NetworkInterfaceInfo]) -> String {
    if interfaces.is_empty() {
        "none".to_string()
    } else {
        interfaces
            .iter()
            .map(NetworkInterfaceInfo::label)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn sanitize_or_default_username(username: Option<&str>) -> String {
    username
        .map(sanitize_username)
        .filter(|username| !username.is_empty())
        .or_else(|| {
            std::env::var("USER")
                .ok()
                .map(|username| sanitize_username(&username))
                .filter(|username| !username.is_empty())
        })
        .unwrap_or_else(|| "peer".to_string())
}

fn describe_selected_device(selected_name: Option<&str>, default_name: Option<&str>) -> String {
    match selected_name {
        Some(name) => name.to_string(),
        None => match default_name {
            Some(name) => format!("System default ({name})"),
            None => "System default (unavailable)".to_string(),
        },
    }
}

fn setup_mdns(
    instance_name: &str,
    service_addr: SocketAddr,
    interface_name: Option<&str>,
) -> Result<MdnsService, Box<dyn Error>> {
    let service_type = "_udp_voice._udp.local.";
    let properties = vec![
        ("service name", "udp voice"),
        ("service type", service_type),
        ("version", "0.1.0"),
        ("interface", "peer"),
    ];
    let mdns = MdnsService::new(service_type, properties, interface_name)?;
    mdns.register_service(instance_name, service_addr);
    mdns.browse_services();
    Ok(mdns)
}

fn start_text_chat(
    username: &str,
    network: &NetworkInterfaceInfo,
) -> Result<TextChatRuntime, Box<dyn Error>> {
    let config = ChatConfig::new(username.to_string())?
        .with_interface(network.name.clone())?
        .with_advertise_addr(IpAddr::V4(network.ip));
    let (chat, event_receiver) = LanChat::start(config)?;
    let runtime_info = chat.runtime_info()?;
    let event_thread = Some(spawn_text_chat_event_thread(event_receiver));

    Ok(TextChatRuntime {
        chat,
        runtime_info,
        event_thread,
    })
}

fn spawn_text_chat_event_thread(
    event_receiver: std::sync::mpsc::Receiver<ChatEvent>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        while let Ok(event) = event_receiver.recv() {
            match event {
                ChatEvent::PeerDiscovered(peer) => {
                    println!();
                    println!(
                        "{}",
                        format!("[text discovered] {}", peer.display_name).cyan()
                    );
                }
                ChatEvent::PeerConnected(peer) => {
                    println!();
                    println!(
                        "{}",
                        format!("[text connected] {}", peer.display_name).green()
                    );
                }
                ChatEvent::PeerDisconnected(peer) => {
                    println!();
                    println!(
                        "{}",
                        format!("[text disconnected] {}", peer.display_name).yellow()
                    );
                }
                ChatEvent::MessageReceived(message) => {
                    println!();
                    println!(
                        "{}",
                        format!("[text] {}: {}", message.display_name, message.body).magenta()
                    );
                }
                ChatEvent::Warning(message) => {
                    println!();
                    println!("{}", format!("[text warning] {message}").yellow());
                }
            }
            print!("> ");
            let _ = io::stdout().flush();
        }
    })
}

fn print_startup(
    settings: &ApplicationSettings,
    local_peer: &LocalPeer,
    network: &RuntimeNetworkConfig,
    text_chat: &ChatRuntimeInfo,
) {
    println!();
    println!("{}", "UDP Voice Peer".green().bold());
    println!("Local peer: {}", local_peer.instance_name.cyan());
    println!("Listening on: {}", local_peer.socket_addr);
    println!("Network interface: {}", network.interface.label());
    println!("UDP port: {}", network.bind_port);
    println!("Text chat room: {}", text_chat.room_name);
    println!("Text chat listen: {}", text_chat.listen_addr);
    println!("Input device: {}", settings.input_device_name());
    println!("Output device: {}", settings.output_device_name());
    println!("Push-to-talk: off");
    println!("Route: all discovered peers");
    println!();
}

fn print_help() {
    println!("{}", "Commands:".cyan());
    println!("  help                 Show commands");
    println!("  peers                List currently discovered peers");
    println!("  select all           Route audio to every discovered peer");
    println!("  select none          Route audio to nobody");
    println!("  select <a,b,c>       Route audio only to the named peer(s)");
    println!("  talk on              Start transmitting microphone audio");
    println!("  talk off             Stop transmitting microphone audio");
    println!("  talk toggle          Toggle transmission on/off");
    println!("  msg <text>           Send a text message to connected text peers");
    println!("  text peers           List connected/discovered text peers");
    println!("  text status          Show text chat room and listener info");
    println!("  stats                Show live packet/jitter stats");
    println!("  devices              Show current and available audio devices");
    println!("  network              Show current network interface and port");
    println!("  exit                 Quit");
    println!();
}

fn command_loop(
    peers: SharedPeerTable,
    route_selection: Arc<Mutex<RouteSelection>>,
    tx_enabled: Arc<AtomicBool>,
    stats: Arc<RuntimeStats>,
    local_peer: LocalPeer,
    inventory: AudioDeviceInventory,
    network: RuntimeNetworkConfig,
    text_chat: &TextChatRuntime,
    settings: &ApplicationSettings,
) -> Result<(), Box<dyn Error>> {
    loop {
        print!("> ");
        io::stdout().flush()?;

        let input = read_user_input()?;
        match parse_user_command(&input) {
            UserCommand::Help => print_help(),
            UserCommand::Peers => print_peers(&peers, &route_selection, &local_peer),
            UserCommand::SelectAll => {
                *route_selection.lock().unwrap() = RouteSelection::All;
                println!("{}", "Routing to all discovered peers.".green());
            }
            UserCommand::SelectNone => {
                *route_selection.lock().unwrap() = RouteSelection::None;
                println!("{}", "Routing disabled.".yellow());
            }
            UserCommand::SelectPeers(requested_names) => {
                apply_peer_selection(&peers, &route_selection, &local_peer, requested_names);
            }
            UserCommand::TalkSet(enabled) => {
                tx_enabled.store(enabled, Ordering::Relaxed);
                println!(
                    "{}",
                    if enabled {
                        "Push-to-talk enabled.".green()
                    } else {
                        "Push-to-talk disabled.".yellow()
                    }
                );
            }
            UserCommand::TalkToggle => {
                let next = !tx_enabled.load(Ordering::Relaxed);
                tx_enabled.store(next, Ordering::Relaxed);
                println!(
                    "{}",
                    if next {
                        "Push-to-talk enabled.".green()
                    } else {
                        "Push-to-talk disabled.".yellow()
                    }
                );
            }
            UserCommand::SendText(message) => {
                if text_chat.peers().is_empty() {
                    println!("{}", "No text peers discovered yet.".yellow());
                    continue;
                }
                text_chat.send(message)?;
                println!("{}", "Text message sent to current text peers.".green());
            }
            UserCommand::TextPeers => print_text_peers(text_chat),
            UserCommand::TextStatus => print_text_status(text_chat)?,
            UserCommand::Stats => {
                print_stats(
                    &stats,
                    &peers,
                    &route_selection,
                    &tx_enabled,
                    &local_peer,
                    &network,
                    text_chat,
                    settings,
                );
            }
            UserCommand::Devices => print_devices(&inventory, settings),
            UserCommand::Network => print_network(&network),
            UserCommand::Exit => return Ok(()),
            UserCommand::Noop => {}
            UserCommand::Unknown(message) => println!("{}", message.red()),
        }
    }
}

fn read_user_input() -> Result<String, io::Error> {
    let mut buffer = String::new();
    std::io::stdin().read_line(&mut buffer)?;
    Ok(buffer.trim().to_string())
}

enum UserCommand {
    Help,
    Peers,
    SelectAll,
    SelectNone,
    SelectPeers(Vec<String>),
    TalkSet(bool),
    TalkToggle,
    SendText(String),
    TextPeers,
    TextStatus,
    Stats,
    Devices,
    Network,
    Exit,
    Noop,
    Unknown(String),
}

fn parse_user_command(input: &str) -> UserCommand {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return UserCommand::Noop;
    }

    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let command = parts.next().unwrap_or_default();
    let remainder = parts.next().unwrap_or("").trim();

    match command {
        "help" => UserCommand::Help,
        "peers" => UserCommand::Peers,
        "stats" => UserCommand::Stats,
        "devices" => UserCommand::Devices,
        "network" => UserCommand::Network,
        "exit" | "quit" => UserCommand::Exit,
        "msg" | "say" => {
            if remainder.is_empty() {
                UserCommand::Unknown("Usage: msg <text>".to_string())
            } else {
                UserCommand::SendText(remainder.to_string())
            }
        }
        "text" => match remainder {
            "peers" => UserCommand::TextPeers,
            "status" => UserCommand::TextStatus,
            _ => UserCommand::Unknown("Usage: text peers | text status".to_string()),
        },
        "select" => {
            if remainder.eq_ignore_ascii_case("all") {
                UserCommand::SelectAll
            } else if remainder.eq_ignore_ascii_case("none") {
                UserCommand::SelectNone
            } else if remainder.is_empty() {
                UserCommand::Unknown(
                    "Usage: select all | select none | select <peer1,peer2>".to_string(),
                )
            } else {
                let names = remainder
                    .split(',')
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>();
                if names.is_empty() {
                    UserCommand::Unknown(
                        "Usage: select all | select none | select <peer1,peer2>".to_string(),
                    )
                } else {
                    UserCommand::SelectPeers(names)
                }
            }
        }
        "talk" | "ptt" => match remainder {
            "on" => UserCommand::TalkSet(true),
            "off" => UserCommand::TalkSet(false),
            "toggle" | "" => UserCommand::TalkToggle,
            _ => UserCommand::Unknown("Usage: talk on | talk off | talk toggle".to_string()),
        },
        _ => UserCommand::Unknown(format!("Unknown command: {trimmed}")),
    }
}

fn apply_peer_selection(
    peers: &SharedPeerTable,
    route_selection: &Arc<Mutex<RouteSelection>>,
    local_peer: &LocalPeer,
    requested_names: Vec<String>,
) {
    let visible = visible_peers(peers, local_peer);
    let mut selected_names = BTreeSet::new();
    let mut missing = Vec::new();

    for requested_name in requested_names {
        if let Some(peer) = visible
            .iter()
            .find(|peer| peer.instance_name.eq_ignore_ascii_case(&requested_name))
        {
            selected_names.insert(peer.instance_name.clone());
        } else {
            missing.push(requested_name);
        }
    }

    if selected_names.is_empty() {
        println!(
            "{}",
            format!(
                "No matching peers are currently present. Missing: {}",
                missing.join(", ")
            )
            .red()
        );
        return;
    }

    *route_selection.lock().unwrap() = RouteSelection::Named(selected_names.clone());
    println!(
        "{}",
        format!(
            "Routing to: {}",
            selected_names.into_iter().collect::<Vec<_>>().join(", ")
        )
        .green()
    );

    if !missing.is_empty() {
        println!(
            "{}",
            format!(
                "These requested peers were not present: {}",
                missing.join(", ")
            )
            .yellow()
        );
    }
}

fn print_peers(
    peers: &SharedPeerTable,
    route_selection: &Arc<Mutex<RouteSelection>>,
    local_peer: &LocalPeer,
) {
    let visible = visible_peers(peers, local_peer);
    let selection = route_selection.lock().unwrap().clone();

    if visible.is_empty() {
        println!("{}", "No other peers discovered yet.".yellow());
        return;
    }

    println!("{}", "Discovered peers:".cyan());
    for peer in visible {
        let marker = if selection.selects(&peer.instance_name) {
            " [selected]".green().to_string()
        } else {
            String::new()
        };
        println!("  {} -> {}{}", peer.instance_name, peer.socket_addr, marker);
    }
}

fn print_devices(inventory: &AudioDeviceInventory, settings: &ApplicationSettings) {
    println!("{}", "Current devices:".cyan());
    println!("  Input: {}", settings.input_device_name());
    println!("  Output: {}", settings.output_device_name());
    println!();
    println!("{}", "Available input devices:".cyan());
    println!(
        "  System default: {}",
        inventory
            .default_input_device
            .as_deref()
            .unwrap_or("(unavailable)")
    );
    if inventory.input_devices.is_empty() {
        println!("  (none)");
    } else {
        for device in &inventory.input_devices {
            println!("  {device}");
        }
    }
    println!();
    println!("{}", "Available output devices:".cyan());
    println!(
        "  System default: {}",
        inventory
            .default_output_device
            .as_deref()
            .unwrap_or("(unavailable)")
    );
    if inventory.output_devices.is_empty() {
        println!("  (none)");
    } else {
        for device in &inventory.output_devices {
            println!("  {device}");
        }
    }
}

fn print_network(network: &RuntimeNetworkConfig) {
    println!("{}", "Current network:".cyan());
    println!("  Interface: {}", network.interface.label());
    println!("  Bind address: {}", network.interface.ip);
    println!("  UDP port: {}", network.bind_port);
    println!("  Preferences file: {}", network.preferences_path.display());
}

fn print_text_peers(text_chat: &TextChatRuntime) {
    let peers = text_chat.peers();
    if peers.is_empty() {
        println!("{}", "No text peers discovered yet.".yellow());
        return;
    }

    println!("{}", "Text peers:".cyan());
    for peer in peers {
        let addresses = peer
            .addresses
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "  {} [{}] -> {}",
            peer.display_name, peer.peer_id, addresses
        );
    }
}

fn print_text_status(text_chat: &TextChatRuntime) -> Result<(), Box<dyn Error>> {
    let runtime = text_chat.runtime_info()?;
    println!("{}", "Text chat:".cyan());
    println!("  Display name: {}", runtime.display_name);
    println!("  Room: {}", runtime.room_name);
    println!("  Auth required: {}", runtime.auth_required);
    println!("  Listen address: {}", runtime.listen_addr);
    println!(
        "  Interface: {}",
        runtime.mdns_interface.as_deref().unwrap_or("(automatic)")
    );
    println!(
        "  Advertise address: {}",
        runtime
            .advertise_addr
            .map(|address| address.to_string())
            .unwrap_or_else(|| "(automatic)".to_string())
    );
    println!("  Connected peers: {}", text_chat.peers().len());
    Ok(())
}

fn print_stats(
    stats: &RuntimeStats,
    peers: &SharedPeerTable,
    route_selection: &Arc<Mutex<RouteSelection>>,
    tx_enabled: &Arc<AtomicBool>,
    local_peer: &LocalPeer,
    network: &RuntimeNetworkConfig,
    text_chat: &TextChatRuntime,
    settings: &ApplicationSettings,
) {
    let snapshot = stats.snapshot();
    let peer_count = visible_peers(peers, local_peer).len();
    let route = route_selection.lock().unwrap().description();

    println!("{}", "Runtime stats:".cyan());
    println!("  Transmitting: {}", tx_enabled.load(Ordering::Relaxed));
    println!("  Route: {route}");
    println!("  Visible peers: {peer_count}");
    println!("  Text peers: {}", text_chat.peers().len());
    println!("  Network interface: {}", network.interface.label());
    println!("  UDP port: {}", network.bind_port);
    println!("  Input device: {}", settings.input_device_name());
    println!("  Output device: {}", settings.output_device_name());
    println!("  Captured frames: {}", snapshot.captured_frames);
    println!("  Encoded frames: {}", snapshot.encoded_frames);
    println!("  Packets sent: {}", snapshot.packets_sent);
    println!("  Packets received: {}", snapshot.packets_received);
    println!("  Packets decoded: {}", snapshot.decoded_packets);
    println!("  Concealed packets: {}", snapshot.concealed_packets);
    println!("  Stale packets dropped: {}", snapshot.stale_packets);
    println!(
        "  Malformed packets dropped: {}",
        snapshot.malformed_packets
    );
    println!("  Playback underflows: {}", snapshot.playback_underflows);
    println!(
        "  Adaptive jitter target: {} packets (~{} ms)",
        snapshot.jitter_target_packets,
        snapshot.jitter_target_packets * 20
    );
    println!("  Estimated arrival jitter: {} ms", snapshot.jitter_ms);
    println!(
        "  Rough one-way latency: {} ms (clock dependent)",
        snapshot.rough_latency_ms
    );
}

fn visible_peers(peers: &SharedPeerTable, local_peer: &LocalPeer) -> Vec<PeerInfo> {
    let mut visible = peers
        .lock()
        .map(|table| {
            table
                .values()
                .filter(|peer| peer.interface == "peer")
                .filter(|peer| {
                    peer.socket_addr != local_peer.socket_addr
                        || peer.instance_name != local_peer.instance_name
                })
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    visible.sort_by(|left, right| left.instance_name.cmp(&right.instance_name));
    visible
}

fn start_input_stream(
    settings: &ApplicationSettings,
    frame_sender: Sender<Vec<f32>>,
    tx_enabled: Arc<AtomicBool>,
    stats: Arc<RuntimeStats>,
) -> Result<Stream, Box<dyn Error>> {
    let device = settings.input_device();
    let supported_config = settings.input_config();
    let input_channels = supported_config.channels() as usize;
    let sample_format = supported_config.sample_format();
    let stream_config: cpal::StreamConfig = supported_config.into();

    let stream = match sample_format {
        SampleFormat::F32 => build_input_stream::<f32>(
            &device,
            &stream_config,
            input_channels,
            frame_sender,
            tx_enabled,
            stats,
        )?,
        SampleFormat::I16 => build_input_stream::<i16>(
            &device,
            &stream_config,
            input_channels,
            frame_sender,
            tx_enabled,
            stats,
        )?,
        SampleFormat::U16 => build_input_stream::<u16>(
            &device,
            &stream_config,
            input_channels,
            frame_sender,
            tx_enabled,
            stats,
        )?,
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("Unsupported input sample format: {sample_format:?}"),
            )
            .into())
        }
    };

    Ok(stream)
}

fn build_input_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    input_channels: usize,
    frame_sender: Sender<Vec<f32>>,
    tx_enabled: Arc<AtomicBool>,
    stats: Arc<RuntimeStats>,
) -> Result<Stream, cpal::BuildStreamError>
where
    T: Sample + SizedSample,
    f32: FromSample<T>,
{
    let mut pending_samples = VecDeque::with_capacity(OPUS_FRAME_SIZE * 2);

    device.build_input_stream(
        config,
        move |data: &[T], _| {
            capture_input_block(
                data,
                input_channels,
                &frame_sender,
                &mut pending_samples,
                &tx_enabled,
                &stats,
            );
        },
        log_input_stream_error,
        None,
    )
}

fn capture_input_block<T>(
    data: &[T],
    input_channels: usize,
    frame_sender: &Sender<Vec<f32>>,
    pending_samples: &mut VecDeque<f32>,
    tx_enabled: &AtomicBool,
    stats: &RuntimeStats,
) where
    T: Sample,
    f32: FromSample<T>,
{
    for frame in data.chunks(input_channels.max(1)) {
        let mono_sample = frame
            .iter()
            .map(|sample| f32::from_sample(*sample))
            .sum::<f32>()
            / frame.len() as f32;
        pending_samples.push_back(mono_sample);

        while pending_samples.len() >= OPUS_FRAME_SIZE {
            let opus_frame = pending_samples.drain(..OPUS_FRAME_SIZE).collect::<Vec<_>>();
            if tx_enabled.load(Ordering::Relaxed) {
                stats.captured_frames.fetch_add(1, Ordering::Relaxed);
                if frame_sender.send(opus_frame).is_err() {
                    return;
                }
            }
        }
    }

    if !tx_enabled.load(Ordering::Relaxed) {
        pending_samples.clear();
    }
}

fn log_input_stream_error(error: cpal::StreamError) {
    warn!("Input stream error: {}", error);
}

fn start_sender_thread(
    socket: UdpSocket,
    frame_receiver: Receiver<Vec<f32>>,
    peers: SharedPeerTable,
    route_selection: Arc<Mutex<RouteSelection>>,
    local_peer: LocalPeer,
    stats: Arc<RuntimeStats>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut encoder =
            match Encoder::new(OPUS_SAMPLE_RATE, opus::Channels::Mono, Application::Voip) {
                Ok(encoder) => encoder,
                Err(error) => {
                    warn!("Failed to initialize Opus encoder: {}", error);
                    return;
                }
            };

        if let Err(error) = encoder.set_bitrate(Bitrate::Bits(OPUS_BITRATE_BPS)) {
            warn!("Failed to set Opus bitrate: {}", error);
        }
        if let Err(error) = encoder.set_vbr(true) {
            warn!("Failed to enable Opus VBR: {}", error);
        }
        if let Err(error) = encoder.set_inband_fec(true) {
            warn!("Failed to enable Opus in-band FEC: {}", error);
        }
        if let Err(error) = encoder.set_packet_loss_perc(10) {
            warn!("Failed to configure Opus packet loss percentage: {}", error);
        }

        let mut encoded_buffer = vec![0_u8; MAX_OPUS_PACKET_SIZE];
        let mut sequence_number = 0_u32;

        while let Ok(frame) = frame_receiver.recv() {
            let target_peers = collect_target_peers(&peers, &route_selection, &local_peer);
            if target_peers.is_empty() {
                continue;
            }

            let encoded_len = match encoder.encode_float(&frame, &mut encoded_buffer) {
                Ok(len) => len,
                Err(error) => {
                    warn!("Opus encode failed: {}", error);
                    continue;
                }
            };

            let packet = match serialize_packet(
                sequence_number,
                current_time_in_ms(),
                &encoded_buffer[..encoded_len],
            ) {
                Ok(packet) => packet,
                Err(error) => {
                    warn!("Failed to serialize packet: {}", error);
                    continue;
                }
            };

            stats.encoded_frames.fetch_add(1, Ordering::Relaxed);

            for peer in target_peers {
                if let Err(error) = socket.send_to(&packet, peer.socket_addr) {
                    warn!(
                        "Failed to send packet {} to {}: {}",
                        sequence_number, peer.socket_addr, error
                    );
                    continue;
                }

                stats.packets_sent.fetch_add(1, Ordering::Relaxed);
            }

            sequence_number = sequence_number.wrapping_add(1);
        }
    })
}

fn collect_target_peers(
    peers: &SharedPeerTable,
    route_selection: &Arc<Mutex<RouteSelection>>,
    local_peer: &LocalPeer,
) -> Vec<PeerInfo> {
    let selection = route_selection.lock().unwrap().clone();
    visible_peers(peers, local_peer)
        .into_iter()
        .filter(|peer| selection.selects(&peer.instance_name))
        .collect()
}

fn start_receiver_thread(
    socket: UdpSocket,
    playback_buffer: Arc<Mutex<VecDeque<f32>>>,
    stats: Arc<RuntimeStats>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut jitter_buffer = BTreeMap::<u32, Vec<u8>>::new();
        let mut next_sequence: Option<u32> = None;
        let mut decoder = match Decoder::new(OPUS_SAMPLE_RATE, opus::Channels::Mono) {
            Ok(decoder) => decoder,
            Err(error) => {
                warn!("Failed to initialize Opus decoder: {}", error);
                return;
            }
        };
        let mut decoded_frame = vec![0.0_f32; OPUS_FRAME_SIZE];
        let mut jitter_controller = JitterController::new();

        loop {
            let mut datagram = [0_u8; MAX_PACKET_SIZE];
            match socket.recv_from(&mut datagram) {
                Ok((amount, source)) => match deserialize_packet(&datagram[..amount]) {
                    Ok(packet) => {
                        stats.packets_received.fetch_add(1, Ordering::Relaxed);
                        jitter_controller.observe_packet(Instant::now(), &stats);

                        if let Some(latency_ms) =
                            current_time_in_ms().checked_sub(packet.timestamp_ms)
                        {
                            if latency_ms < 60_000 {
                                stats.rough_latency_ms.store(latency_ms, Ordering::Relaxed);
                            }
                        }

                        if let Some(expected_sequence) = next_sequence {
                            if packet.sequence_number < expected_sequence {
                                stats.stale_packets.fetch_add(1, Ordering::Relaxed);
                                continue;
                            }
                        }

                        let _ = source;
                        jitter_buffer.insert(packet.sequence_number, packet.payload);
                        drain_jitter_buffer(
                            &mut jitter_buffer,
                            &mut next_sequence,
                            jitter_controller.target_packets(),
                            &mut decoder,
                            &mut decoded_frame,
                            &playback_buffer,
                            &stats,
                        );
                    }
                    Err(error) => {
                        stats.malformed_packets.fetch_add(1, Ordering::Relaxed);
                        warn!("Dropping malformed packet: {}", error);
                    }
                },
                Err(error) => warn!("UDP receive error: {}", error),
            }
        }
    })
}

fn drain_jitter_buffer(
    jitter_buffer: &mut BTreeMap<u32, Vec<u8>>,
    next_sequence: &mut Option<u32>,
    target_packets: usize,
    decoder: &mut Decoder,
    decoded_frame: &mut [f32],
    playback_buffer: &Arc<Mutex<VecDeque<f32>>>,
    stats: &RuntimeStats,
) {
    if next_sequence.is_none() {
        if jitter_buffer.len() <= target_packets {
            return;
        }

        *next_sequence = jitter_buffer.keys().next().copied();
    }

    while jitter_buffer.len() > target_packets {
        let expected_sequence = next_sequence.expect("sequence number should be initialized");
        let decode_result = if let Some(payload) = jitter_buffer.remove(&expected_sequence) {
            stats.decoded_packets.fetch_add(1, Ordering::Relaxed);
            decoder.decode_float(&payload, decoded_frame, false)
        } else {
            stats.concealed_packets.fetch_add(1, Ordering::Relaxed);
            decoder.decode_float(&[], decoded_frame, false)
        };

        match decode_result {
            Ok(samples) => {
                let mut buffer = playback_buffer
                    .lock()
                    .expect("Failed to acquire playback buffer lock");
                buffer.extend(decoded_frame.iter().take(samples).copied());

                while buffer.len() > OPUS_FRAME_SIZE * 32 {
                    buffer.pop_front();
                }
            }
            Err(error) => warn!("Opus decode failed: {}", error),
        }

        *next_sequence = Some(expected_sequence.wrapping_add(1));
    }
}

fn start_output_stream(
    settings: &ApplicationSettings,
    playback_buffer: Arc<Mutex<VecDeque<f32>>>,
    stats: Arc<RuntimeStats>,
) -> Result<Stream, Box<dyn Error>> {
    let device = settings.output_device();
    let supported_config = settings.output_config();
    let output_channels = supported_config.channels() as usize;
    let sample_format = supported_config.sample_format();
    let stream_config: cpal::StreamConfig = supported_config.into();

    let stream = match sample_format {
        SampleFormat::F32 => build_output_stream::<f32>(
            &device,
            &stream_config,
            output_channels,
            playback_buffer,
            stats,
        )?,
        SampleFormat::I16 => build_output_stream::<i16>(
            &device,
            &stream_config,
            output_channels,
            playback_buffer,
            stats,
        )?,
        SampleFormat::U16 => build_output_stream::<u16>(
            &device,
            &stream_config,
            output_channels,
            playback_buffer,
            stats,
        )?,
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("Unsupported output sample format: {sample_format:?}"),
            )
            .into())
        }
    };

    Ok(stream)
}

fn build_output_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    output_channels: usize,
    playback_buffer: Arc<Mutex<VecDeque<f32>>>,
    stats: Arc<RuntimeStats>,
) -> Result<Stream, cpal::BuildStreamError>
where
    T: Sample + SizedSample + FromSample<f32>,
{
    device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            fill_output_data(data, output_channels, &playback_buffer, &stats);
        },
        log_output_stream_error,
        None,
    )
}

fn fill_output_data<T>(
    data: &mut [T],
    output_channels: usize,
    playback_buffer: &Arc<Mutex<VecDeque<f32>>>,
    stats: &RuntimeStats,
) where
    T: Sample + FromSample<f32>,
{
    let mut buffer = playback_buffer
        .lock()
        .expect("Failed to acquire playback buffer lock");
    let mut current_sample = 0.0_f32;
    let mut underflowed = false;

    for (index, sample) in data.iter_mut().enumerate() {
        if index % output_channels.max(1) == 0 {
            match buffer.pop_front() {
                Some(next_sample) => current_sample = next_sample,
                None => {
                    current_sample = 0.0;
                    underflowed = true;
                }
            }
        }
        *sample = T::from_sample(current_sample);
    }

    if underflowed {
        stats.playback_underflows.fetch_add(1, Ordering::Relaxed);
    }
}

fn log_output_stream_error(error: cpal::StreamError) {
    warn!("Output stream error: {}", error);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_setup_requires_username_and_audio_devices() {
        assert!(!CliOptions::default()
            .has_complete_audio_and_identity_setup(&AppPreferences::default()));

        assert!(!CliOptions {
            username: Some("alice".to_string()),
            ..CliOptions::default()
        }
        .has_complete_audio_and_identity_setup(&AppPreferences::default()));

        assert!(!CliOptions {
            username: Some("alice".to_string()),
            input_device_name: Some("Mic".to_string()),
            ..CliOptions::default()
        }
        .has_complete_audio_and_identity_setup(&AppPreferences::default()));
    }

    #[test]
    fn fully_specified_cli_or_preferences_can_skip_setup() {
        assert!(CliOptions {
            username: Some("alice".to_string()),
            input_device_name: Some("Mic".to_string()),
            output_device_name: Some("Speakers".to_string()),
            ..CliOptions::default()
        }
        .has_complete_audio_and_identity_setup(&AppPreferences::default()));

        assert!(
            CliOptions::default().has_complete_audio_and_identity_setup(&AppPreferences {
                username: Some("alice".to_string()),
                input_device_name: Some("Mic".to_string()),
                output_device_name: Some("Speakers".to_string()),
                ..AppPreferences::default()
            })
        );
    }

    #[test]
    fn device_description_falls_back_to_system_default_label() {
        assert_eq!(
            describe_selected_device(None, Some("Built-in Output")),
            "System default (Built-in Output)"
        );
        assert_eq!(
            describe_selected_device(None, None),
            "System default (unavailable)"
        );
        assert_eq!(
            describe_selected_device(Some("USB Mic"), Some("Built-in Mic")),
            "USB Mic"
        );
    }

    #[test]
    fn startup_config_prefers_cli_over_saved_defaults() {
        let config = merge_startup_config(
            &CliOptions {
                username: Some("cli-user".to_string()),
                bind_port: Some(19_000),
                ..CliOptions::default()
            },
            &AppPreferences {
                username: Some("saved-user".to_string()),
                input_device_name: Some("Saved Mic".to_string()),
                output_device_name: Some("Saved Speakers".to_string()),
                network_interface_name: Some("en0".to_string()),
                bind_port: Some(18_521),
            },
        );

        assert_eq!(config.username, "cli-user");
        assert_eq!(config.input_device_name.as_deref(), Some("Saved Mic"));
        assert_eq!(config.output_device_name.as_deref(), Some("Saved Speakers"));
        assert_eq!(config.network_interface_name.as_deref(), Some("en0"));
        assert_eq!(config.bind_port, Some(19_000));
    }

    #[test]
    fn parser_accepts_text_commands() {
        match parse_user_command("msg hello world") {
            UserCommand::SendText(message) => assert_eq!(message, "hello world"),
            _ => panic!("expected SendText command"),
        }

        assert!(matches!(
            parse_user_command("text status"),
            UserCommand::TextStatus
        ));
        assert!(matches!(
            parse_user_command("text peers"),
            UserCommand::TextPeers
        ));
    }
}
