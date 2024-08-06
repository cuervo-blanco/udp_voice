use mdns_sd::{ServiceDaemon, ServiceInfo, ServiceEvent};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use hostname;
use log::debug;

pub struct MdnsService {
    daemon: ServiceDaemon,
    service_type: String,
    host_name: String,
    properties: Vec<(&'static str, &'static str)>,
    user_table: Arc<Mutex<HashMap<String, String>>>,
}

impl MdnsService {
    pub fn new(
        service_type: &str, 
        properties: Vec<(&'static str, &'static str)>) 
        -> Self {
            let daemon = ServiceDaemon::new().expect("mDNS: Failed to create Daemon");
            let host_name = hostname::get()
                .expect("mDNS: Unable to get host name")
                .to_str()
                .expect("mDNS: Unable to convert host name to string")
                .to_owned() + ".local.";
            MdnsService {
                daemon,
                service_type: service_type.to_string(),
                host_name,
                properties,
                user_table: Arc::new(Mutex::new(HashMap::new())),
            }
        
    }
    pub fn register_service(&self, instance_name: &str, ip: IpAddr, port: u16) {
        let service_info = ServiceInfo::new(
            &self.service_type,
            instance_name,
            &self.host_name,
            ip,
            port,
            &self.properties[..],
            ).unwrap();
        self.daemon.register(service_info).expect("mDNS: Failed to register service");
        debug!("mDNS: Service registered: {}", instance_name);
    }
    pub fn browse_services(&self) {
        let receiver = self.daemon.browse(&self.service_type).expect("Failed to browse");
        let user_table = self.user_table.clone();

        thread::spawn( move || {
            loop {
                debug!("mDNS: Starting mDNS loop");
                while let Ok(event) = receiver.recv() {
                    match event {
                        ServiceEvent::ServiceResolved(info) => {
                            debug!("mDNS: Service Resolved: {:?}", info);
                            let addresses = info.get_addresses_v4();
                            debug!("mDNS: Addresses found: {:?}", addresses);
                            for address in addresses {
                                debug!("mDNS: Found User in IP Address: {:?}", address);
                                user_table.lock().unwrap().insert(info.get_fullname().to_string(), address.to_string());
                                debug!("mDNS: Inserted New User into User Table: {:?}", info.get_fullname());
                                let mut username = String::new();
                                for char in info.get_fullname().chars() {
                                    if char != '.' {
                                        username.push(char);
                                    } else {
                                        break;
                                    }
                                }
                                debug!("{} just connected", username);
                            }
                        },
                        _ => {}
                    }
                }
                thread::sleep(Duration::from_secs(1));
            }
        });
    }
    pub fn get_user_table(&self) -> Arc<Mutex<HashMap<String, String>>> {
        Arc::clone(&self.user_table)
    }
}
