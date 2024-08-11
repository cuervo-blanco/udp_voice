use ringbuf::HeapRb;
use ringbuf::traits::{Consumer, Producer, Split, Observer};
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

                let ip_port = format!("{}:{}", ip, port); debug!("UDP: IP Address & Port: {}", ip);
                let (output_sine, input_encoder) = channel();
                let (output_encoder, input_buffer) = channel();
                let _sine = Sine::new(220.0, 1.0, sample_rate as u32, channels as usize, output_sine, buffer_size);

                // Initialize Ring Buffer to store encoded_samples
                let ring = HeapRb::<u8>::new(buffer_size * channels as usize);
                let (mut producer, mut consumer) = ring.split();

                // Encode to Opus
                println!("Starting Opus encoding");
                let _chunk = encode_opus(input_encoder, output_encoder).expect("Failed to convert into Opus");
                println!("Opus encoding started successfully");

                std::thread::spawn( move || {
                    println!("CLIENT: Producer thread started");
                    while let Ok(block) = input_buffer.recv() {
                        println!("CLIENT: Received block of size: {}", block.len());
                        for sample in block {
                            while producer.is_full() {
                                std::thread::sleep(std::time::Duration::from_millis(1));
                            }
                            producer.try_push(sample).expect("Failed to push into producer");
                        }
                        println!("CLIENT: Block successfully pushed to producer");
                    }
                    println!("CLIENT: Input buffer channel closed, producer thread exiting");

                });

                let user_table_clone = user_table.clone();

                std::thread::spawn(move || {
                    println!("CLIENT: UDP sender thread started");
                    let user_table = user_table_clone.lock().unwrap();
                    loop {
                        for (user, address) in user_table.clone() {
                            println!("CLIENT: Preparing to send to address: {}", address);
                            let socket = UdpSocket::bind(&ip_port).expect("UDP: Failed to bind to socket");
                            let mut buffer: Vec<u8> = vec![0; buffer_size * channels as usize];
                            let size = consumer.pop_slice(&mut buffer);
                            let block = &buffer[..size];
                            println!("CLIENT: Popped slice of size: {}", size);

                            if address == ip.to_string() {
                                println!("CLIENT: Skipping send to local address: {}", address);
                                continue;
                            } else {
                                let port = format!("{}:18521", address);
                                println!("CLIENT: Sending data to {}: {}", user, port);
                                socket.send_to(block, port.clone()).expect("UDP: Failed to send data");
                                println!("CLIENT: Data sent successfully to {}", port);
                            }
                        }
                    }
                });

            }
        } else {
            println!("Not a permitted command");
            continue;
        }
    }
    
}
