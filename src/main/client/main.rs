#[allow(unused_imports)]
use std::{
    io::{Write, stdout},
    sync::{
        Arc, Mutex,
        mpsc::channel,
    },
    net::UdpSocket,
};
use ringbuf::{
    traits::{Consumer, Producer, Split, Observer},
    HeapRb,
};
use byteorder::{BigEndian, WriteBytesExt};
#[allow(unused_imports)]
use log::{debug, info, warn, error};
#[allow(unused_imports)]
use selflib::{
    utils::{clear_terminal, username_take},
    mdns_service::MdnsService,
    settings::Settings,
    sine::Sine,
    sound::encode_opus,
};
use colored::*;

fn main () -> Result<(), Box<dyn std::error::Error>> {

    env_logger::init();

    let settings = Settings::get_default_settings();
    let sample_rate = settings.get_sample_rate();
    let channels = settings.get_channels();
    let buffer_size = settings.get_buffer_size();

    println!("");
    println!("{}", "Enter Username:".cyan());
    // Add validation process? 
    let instance_name = Arc::new(Mutex::new(username_take()));
    clear_terminal();

    // Gather information from client
    let ip =  local_ip_address::local_ip().unwrap(); 
    let _ip_check = format!("UDP: Local IP Address: {}", ip); 
    println!("");
    let port: u16 = 18522;

    // mDNS
    let properties = vec![
        ("service name", "udp voice"), 
        ("service type", "_udp_voice._udp_local."), 
        ("version", "0.0.2"),
        ("interface", "client")
    ];
    let mdns = MdnsService::new("_udp_voice._udp.local.", properties);
    mdns.register_service(&instance_name.lock().unwrap(), ip, port);
    mdns.browse_services();
    let user_table = mdns.get_user_table();

    // Main Loop
    loop {
        // Update the window buffer with the current status

        // Take user input
        let reader = std::io::stdin();
        let mut buffer: String = String::new();
        reader.read_line(&mut buffer).unwrap();
        let input = buffer.trim();

        if input == "send" {
            loop {
                let ip_port = format!("{}:{}", ip, port); 
                let ip_check = format!("UDP: IP Address & Port: {}", ip_port).blue();
                println!("{}", &ip_check);
                // The sine's output goes into the encoder's input
                // The encoder's output goes into the buffer's input
                let (output_sine, input_encoder) = channel();
                let (output_encoder, input_buffer) = channel();

                println!("{}", "Generating Sound Wave...".cyan());
                let _sine = Sine::new(220.0, 1.0, sample_rate as u32, channels as usize, output_sine, buffer_size);

                // Encode to Opus
                println!("{}", "Starting Opus Encoding...".cyan());
                match encode_opus(input_encoder, output_encoder) {
                    Ok(_) => {
                        println!("{}","Opus Encoding started successfully...".green());
                    }
                    Err(e) => {
                        println!("{}", "Failed to start Opus encoding".red());
                        error!("Failed to start Opus encoding: {:?}", e);
                    }
                }

                // Initialize Ring Buffer to store encoded_samples
                let ring = HeapRb::<u8>::new(buffer_size * channels as usize);
                let (mut producer, mut consumer) = ring.split();

                std::thread::spawn( move || {
                    let mut counter = 0;
                    loop {
                        while let Ok(block) = input_buffer.recv() {
                            println!("{}", format!("CLIENT: RECEIVED block of size: {}", block.len()).yellow());
                            for sample in block {
                                while producer.is_full() {
                                    std::thread::sleep(std::time::Duration::from_millis(1));
                                }
                                counter += 1;
                                producer.try_push(sample).expect("CLIENT:Failed to push into producer");
                                if counter % 48000 == 0 {
                                    print!("\x1B[23;1H");
                                    print!("\r{}", format!("ENCODER: Pushing into buffer: {:.5}", &sample).magenta());
                                    std::io::stdout().flush().unwrap();
                                }
                            }
                            println!("{}", "CLIENT: Block successfully pushed to producer".green());
                        }
                        // println!("{}", "CLIENT: Input buffer channel closed, producer thread exiting".yellow());
                    }

                });

                let user_table_clone = user_table.clone();

                std::thread::spawn(move || {
                    println!("{}", "CLIENT: UDP sender thread started".cyan());
                    let user_table = user_table_clone.lock().unwrap();
                    loop {
                        for (user, address) in user_table.clone() {
                            let socket = UdpSocket::bind(&ip_port).expect("UDP: Failed to bind to socket");
                            let mut buffer: Vec<u8> = vec![0; buffer_size * channels as usize];
                            let size = consumer.pop_slice(&mut buffer);
                            let block = &buffer[..size];
                            println!("{}", format!("CLIENT: Popped slice of size: {}", size).cyan());

                            //if address == ip.to_string() {
                                // println!("{}", format!("CLIENT: Skipping send to local address: {}", address).blue());
                                //continue;
                            //} else {
                                let port = format!("{}:18521", address);
                                println!("{}", format!("CLIENT: Sending data to {}: {}", user, port).cyan());

                                let data_len = block.len() as u32;
                                let mut packet = Vec::with_capacity(4 + block.len());

                                // Write the length to the header
                                packet.write_u32::<BigEndian>(data_len).unwrap();

                                // Append the encoded data to the packet
                                packet.extend_from_slice(&block);

                                // Send the packet
                                socket.send_to(&packet, port.clone()).expect("CLIENT: Failed to send data");

                                println!("{}", format!("CLIENT: Data sent successfully to {}", port).green());
                            //}
                        }
                    }
                });

            }
        } else if input == "exit" {
            return Ok(());
        } else {
            println!("{}", "Not a permitted command".red());
            continue;
        } 
    }

}

