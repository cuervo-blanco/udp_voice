use selflib::mdns_service::MdnsService;
use log::debug;
use std::net::UdpSocket;
use selflib::config::BUFFER_SIZE;
use selflib::audio::*;

fn main (){
    env_logger::init();


    let (Some(_input_device), Some(output_device)) = initialize_audio_interface() else {
        return;
    };

    let output_config = get_audio_config(&output_device)
        .expect("Failed to get audio output config");

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
    let mut buffer = [0; BUFFER_SIZE]; // Modify this to work with a FRAME_SIZE

    loop { 
        if let Ok((amount, _source)) = socket.recv_from(&mut buffer){
            let received = &mut buffer[..amount];
            if let Ok(audio) = decode_opus_to_pcm(received) {
                match start_output_stream(&output_device, &output_config, audio){
                    Ok(_) => println!("Playing audio..."),
                    Err(e) => eprintln!("Error starting stream: {:?}", e),
                }
            } else {
                eprintln!("Failed to decode Opus to PCM");
            }

        }
    }
    
    // Send this data to a port, have a different Application in Front End.

}
