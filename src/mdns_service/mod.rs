use hostname;
use log::{debug, warn};
use mdns_sd::{IfKind, ServiceDaemon, ServiceEvent, ServiceInfo};
use std::sync::{Arc, Mutex};
use std::thread;
use std::{collections::HashMap, error::Error, net::SocketAddr};

#[derive(Clone, Debug)]
pub struct PeerInfo {
    pub instance_name: String,
    pub socket_addr: SocketAddr,
    pub interface: String,
}

pub type SharedPeerTable = Arc<Mutex<HashMap<String, PeerInfo>>>;

pub struct MdnsService {
    daemon: ServiceDaemon,
    service_type: String,
    host_name: String,
    properties: Vec<(&'static str, &'static str)>,
    user_table: SharedPeerTable,
}

impl MdnsService {
    pub fn new(
        service_type: &str,
        properties: Vec<(&'static str, &'static str)>,
        interface_name: Option<&str>,
    ) -> Result<Self, Box<dyn Error>> {
        let daemon = ServiceDaemon::new().expect("mDNS: Failed to create Daemon");

        if let Some(interface_name) = interface_name {
            daemon.disable_interface(IfKind::All)?;
            daemon.enable_interface(interface_name)?;
        }

        daemon.disable_interface(IfKind::IPv6)?;
        let host_name = hostname::get()
            .expect("mDNS: Unable to get host name")
            .to_str()
            .expect("mDNS: Unable to convert host name to string")
            .to_owned()
            + ".local.";
        Ok(MdnsService {
            daemon,
            service_type: service_type.to_string(),
            host_name,
            properties,
            user_table: Arc::new(Mutex::new(HashMap::new())),
        })
    }
    pub fn register_service(&self, instance_name: &str, socket_addr: SocketAddr) {
        let service_info = ServiceInfo::new(
            &self.service_type,
            instance_name,
            &self.host_name,
            socket_addr.ip(),
            socket_addr.port(),
            &self.properties[..],
        )
        .unwrap();
        self.daemon
            .register(service_info)
            .expect("mDNS: Failed to register service");
        debug!("mDNS: Service registered: {}", instance_name);
    }
    pub fn browse_services(&self) {
        let receiver = self
            .daemon
            .browse(&self.service_type)
            .expect("Failed to browse");
        let user_table = self.user_table.clone();

        thread::spawn(move || {
            while let Ok(event) = receiver.recv() {
                match event {
                    ServiceEvent::ServiceResolved(info) => {
                        let interface = info
                            .get_property_val_str("interface")
                            .unwrap_or("unknown")
                            .to_string();
                        let instance_name = info
                            .get_fullname()
                            .split('.')
                            .next()
                            .unwrap_or_default()
                            .to_string();

                        for address in info.get_addresses() {
                            let peer = PeerInfo {
                                instance_name: instance_name.clone(),
                                socket_addr: SocketAddr::new(*address, info.get_port()),
                                interface: interface.clone(),
                            };
                            user_table
                                .lock()
                                .unwrap()
                                .insert(info.get_fullname().to_string(), peer);
                        }
                    }
                    ServiceEvent::ServiceRemoved(_, full_name) => {
                        debug!("mDNS: Removing peer {}", full_name);
                        user_table.lock().unwrap().remove(&full_name);
                    }
                    other => {
                        warn!("mDNS: Ignoring service event: {:?}", other);
                    }
                }
            }
        });
    }

    pub fn get_user_table(&self) -> SharedPeerTable {
        Arc::clone(&self.user_table)
    }
}
