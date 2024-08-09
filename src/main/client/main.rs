use selflib::mdns_service::MdnsService;
use selflib::settings::Settings;
use std::sync::mpsc::channel;
use selflib::sound::encode_opus;
use selflib::sine::Sine;
use std::sync::{Arc, Mutex};
use std::net::UdpSocket;
use log::debug;
use selflib::utils::{clear_terminal, username_take};


fn main () {

    env_logger::init();

    let settings = Settings::get_default_settings();
    
    let sample_rate = settings.get_sample_rate();
    let channels = settings.get_channels();
    let buffer_size = settings.get_buffer_size();

    println!("");
    println!("Enter Username:");
    // Add validation process? 
    let instance_name = Arc::new(Mutex::new(username_take()));
    clear_terminal();

    // -------- Input Thread ------- //

    // Gather information from client
    let ip =  local_ip_address::local_ip().unwrap(); debug!("UDP: Local IP Address: {}", ip);
    let port: u16 = 18522;
    let ip_port = format!("{}:{}", ip, port); debug!("UDP: IP Address & Port: {}", ip);


    // mDNS
    let properties = vec![
        ("service name", "udp voice"), 
        ("service type", "_udp_voice._udp_local."), 
        ("version", "0.0.0"),
        ("interface", "client")
    ];
    let mdns = MdnsService::new("_udp_voice._udp.local.", properties);
    mdns.register_service(&instance_name.lock().unwrap(), ip, port);
    mdns.browse_services();
    let user_table = mdns.get_user_table();

    loop {
        // Take user input
        let reader = std::io::stdin();
        let mut buffer: String = String::new();
        reader.read_line(&mut buffer).unwrap();
        let input = buffer.trim();

        if input == "send" {
            loop {

                let (output_sine, input_encoder) = channel();
                let (output_encoder, _input_socket) = channel();
                let _sine = Sine::new(220.0, 1.0, sample_rate as u32, channels as usize, output_sine, buffer_size);

                // Encode to Opus
                let chunk = encode_opus(input_encoder, output_encoder).expect("Failed to convert into Opus");

                let socket = UdpSocket::bind(&ip_port).expect("UDP: Failed to bind to socket");
                for (user, address) in user_table.lock().unwrap().clone() {
                    if address == ip.to_string() {
                        continue;
                    } else {
                        let port = format!("{}:18521", address);
                        // Calculate Time
                        socket.send_to(&chunk, port.clone()).expect("UDP: Failed to send data");
                        println!("Sent chunk to {}: {:?}", user, chunk);
                    }
                }
                clear_terminal();
            }
        } else {
            println!("Not a permitted command");
            continue;
        }
    }
    
}
