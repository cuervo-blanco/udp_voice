use selflib::mdns_service::MdnsService;
use log::debug;
use std::net::UdpSocket;
use selflib::config::FRAME_SIZE;
use std::time::SystemTime;

fn main (){
    env_logger::init();

    // System Information 
    let ip =  local_ip_address::local_ip().unwrap(); debug!("UDP: Local IP Address: {}", ip);
    let port: u16 = 18521;
    let ip_port = format!("{}:{}", ip, port); debug!("UDP: IP Address & Port: {}", ip);


    // mDNS
    let service_type = "udp_voice._udp.local.";

    let properties = vec![
        ("service name", "udp voice"), 
        ("service type", service_type), 
        ("version", "0.0.0"),
        ("interface", "server"),

    ];

    let mdns = MdnsService::new("_udp_voice._udp.local.", properties);
    let _user_table = mdns.get_user_table();
    mdns.register_service("udp_server", ip, port);
    mdns.browse_services();

    // 1. Listen for udp messages in a port
    let socket = UdpSocket::bind(ip_port).expect("UDP: Failed to bind socket");
    let mut buffer = [0; FRAME_SIZE]; // Modify this to work with a FRAME_SIZE

    loop { 
        let now = SystemTime::now();
        if let Ok((amount, source)) = socket.recv_from(&mut buffer){
            let received = &mut buffer[..amount];
            println!("FROM: {}, DATA: {:?}", source, received);
        }
        match now.elapsed() {
            Ok(elapsed) => {
                println!("TOTAL TIME ELAPSED: {}", elapsed.as_secs());
            }
            Err(e) => {
                println!("ERROR CALCULATING ELAPSED TIME: {}", e);
            }
        }
    }
    
    // Send this data to a port, have a different Application in Front End.

}
