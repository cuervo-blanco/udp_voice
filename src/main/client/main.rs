use std::sync::mpsc::channel;
use std::io::Write;
use selflib::mdns_service::MdnsService;
use selflib::config::{SAMPLE_RATE, BUFFER_SIZE};
use selflib::audio::convert_audio_stream_to_opus;
use std::sync::{Arc, Mutex};
use std::net::UdpSocket;
use log::debug;
use std::f32::consts::PI;


fn  clear_terminal() {
    print!("\x1B[2J");
    std::io::stdout().flush().unwrap();
}

fn username_take()-> String {
    // Take user input (instance name)
    let reader = std::io::stdin();
    let mut instance_name = String::new();
    reader.read_line(&mut instance_name).unwrap();
    let instance_name = instance_name.replace("\n", "").replace(" ", "_");
    instance_name
}

const FREQUENCY: f32 = 440.0;
const AMPLITUDE: f32 = 0.5;

fn main () {
    env_logger::init();

    println!("");
    println!("Enter Username:");
    // Add validation process? 
    let instance_name = Arc::new(Mutex::new(username_take()));
    clear_terminal();
    let (_tx, _rx) = channel::<String>();

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

    let mut sample_clock = 0f32;

    loop {
        // Take user input
        let reader = std::io::stdin();
        let mut buffer: String = String::new();
        reader.read_line(&mut buffer).unwrap();
        let input = buffer.trim();

        if input == "send" {
            loop {
                let period: Vec<f32> = (0..BUFFER_SIZE)
                    .map(|_| {
                        let value = (sample_clock * FREQUENCY * 2.0 * PI / SAMPLE_RATE).sin() * AMPLITUDE;
                        sample_clock = (sample_clock + 1.0) % SAMPLE_RATE;
                        value
                    }).collect();

                // Encode to Opus
                let chunk = convert_audio_stream_to_opus(&period).unwrap();

                let socket = UdpSocket::bind(&ip_port).expect("UDP: Failed to bind to socket");
                for (_user, address) in user_table.lock().unwrap().clone() {
                    if address == ip.to_string() {
                        continue;
                    } else {
                        let port = format!("{}:18521", address);
                        // Calculate Time
                        socket.send_to(&chunk, port.clone()).expect("UDP: Failed to send data");
                    }
                }
            }
        } else {
            println!("Not a permitted command");
            continue;
        }
    }
    
}
