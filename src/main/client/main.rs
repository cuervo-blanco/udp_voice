#[allow(unused_imports)]
use std::{
    io::{Write, stdout},
    sync::{
        Arc, Mutex,
        mpsc::channel,
    },
    net::UdpSocket,
};
#[allow(unused_imports)]
use ringbuf::{
    traits::{Consumer, Producer, Split, Observer},
    HeapRb,
};
use opus::{Encoder, Application};
use byteorder::{BigEndian, WriteBytesExt};
#[allow(unused_imports)]
use log::{debug, info, warn, error};
#[allow(unused_imports)]
use selflib::{
    utils::{clear_terminal, username_take},
    mdns_service::MdnsService,
    settings::{Settings, ApplicationSettings},
    sine::Sine,
};
use colored::*;

fn main () -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    println!("Logger initialized");
    let settings: ApplicationSettings = Settings::get_default_settings();
    println!("Settings loaded");
    let sample_rate = settings.get_sample_rate();
    let channels = settings.get_channels();
    let buffer_size = settings.get_buffer_size();

    println!("");
    println!("{}", "Enter Username:".cyan());
    // Add validation process?
    let instance_name = Arc::new(Mutex::new(username_take()));

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
        let opus_channels = if channels == 1 { opus::Channels::Mono } else { opus::Channels::Stereo };

        if input == "send" {
            let ip_port = format!("{}:{}", ip, port);
            let ip_check = format!("UDP: IP Address & Port: {}", ip_port).blue();
            println!("{}", &ip_check);
            // The sine's output goes into the encoder's input
            // The encoder's output goes into the buffer's input
            let (output_sine, input_encoder) = channel();
            let (output_encoder, input_buffer) = channel();
            let (len_out, len_in) = channel();

            println!("{}", "Generating Sound Wave...".cyan());
            let _sine = Sine::new(440.0, 1.0, sample_rate as u32, channels as usize, output_sine, buffer_size);

            // Encode to Opus
            println!("{}", "Starting Opus Encoding...".cyan());
            // let _ = encode_opus(input_encoder, output_encoder);
            std::thread::spawn(move || {
                let mut opus_encoder = Encoder::new(
                    sample_rate as u32,
                    opus_channels,
                    Application::Audio
                ).unwrap();
                while let Ok(block) = input_encoder.recv() {
                    opus_encoder.set_bitrate(opus::Bitrate::Bits(64000)).expect("Failed to set bitrate");
                    opus_encoder.set_vbr(false).expect("Failed to set CBR mode");
                    // Swap active buffers to avoid blocking
                    // println!("Copied new audio block of size {} into inactive buffer", block.len());
                    let mut encoded_block = vec![0; buffer_size * channels as usize];
                    if let Ok(len) = opus_encoder.encode_float(&block, &mut encoded_block){
                        //let data_len = format!("ENCODER: Encoded block of size: {}", len).magenta();
                        //println!("{data_len}");
                        let encoded_data = encoded_block[..len].to_vec();
                        // println!("{}", format!("Block: {:?}", encoded_data).magenta());
                        output_encoder.send(encoded_data).expect("Failed to send encoded data");
                        let _ = len_out.send(len);
                        // println!("Encoded data sent to output channel");
                    }
                }
            });

            let user_table_clone = user_table.clone();
            let mut batch_buffer = vec![0u8; len_in.recv().unwrap() * 6];
            println!("BATCH SIZE: {}", batch_buffer.len());
            let mut offset = 0;

            std::thread::spawn(move || {
                let socket = UdpSocket::bind(&ip_port).expect("UDP: Failed to bind to socket");
                let user_table = user_table_clone.lock().unwrap();
                loop {
                    if let Ok(block) = input_buffer.recv() {
                        for &sample in &block {
                            batch_buffer[offset] = sample;
                            offset += 1;

                            if offset == batch_buffer.len() {
                                // Send the full batch buffer as a packet
                                for (_user, address) in user_table.clone() {
                                    let port = format!("{}:18521", address);
                                    let data_len = batch_buffer.len() as u32;
                                    let mut packet = Vec::with_capacity(4 + batch_buffer.len());

                                    // Write the length to the header
                                    packet.write_u32::<BigEndian>(data_len).unwrap();
                                    packet.extend_from_slice(&batch_buffer);

                                    // println!("{}", format!("Packet: {:?}", packet).yellow());
                                    // println!("Packet length: {:?}", packet.len());
                                    socket.send_to(&packet, &port).expect("Failed to send data");
                                    println!("Sent packet of size: {}", packet.len());
                                }
                                offset = 0;  // Reset offset after sending
                            }
                        }
                    }
                }
            });
        } else if input == "exit" {
            return Ok(());
        } else {
            println!("{}", "Not a permitted command".red());
            continue;
        }
    }

}

