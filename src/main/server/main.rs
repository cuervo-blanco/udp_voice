use std::io::Cursor;
use byteorder::{BigEndian, ReadBytesExt};
use std::sync::{Arc, Mutex};
#[allow(unused_imports)]
use ringbuf::HeapRb;
#[allow(unused_imports)]
use ringbuf::traits::{Consumer, Producer, Split};
use selflib::mdns_service::MdnsService;
#[allow(unused_imports)]
use log::{debug, info, warn, error};
use std::net::UdpSocket;
use selflib::settings::{Settings, ApplicationSettings};
use selflib::sound::dac;
use std::sync::mpsc::channel;
use colored::*;
use opus::Decoder;

fn main (){
    env_logger::init();

    let settings: ApplicationSettings = Settings::get_default_settings();
    let channels = settings.get_channels();
    let sample_rate = settings.get_sample_rate();
    let (_input_device, output_device) = settings.get_devices();
    let output_device = Arc::new(Mutex::new(output_device));
    let (_input_config, _output_config) = settings.get_config_files();
    let buffer_size = settings.get_buffer_size();

    // System Information
    let ip =  local_ip_address::local_ip().unwrap();
    println!("UDP: Local IP Address: {}", ip);
    let port: u16 = 18521;
    let ip_port = format!("{}:{}", ip, port);
    println!("UDP: IP Address & Port: {}", ip);

    // mDNS
    let service_type = "udp_voice._udp.local.";

    let properties = vec![
        ("service name", "udp voice"),
        ("service type", service_type),
        ("version", "0.0.0"),
        ("interface", "server"),
        // Define more properties relevant to the service, such as Room Name
    ];

    let mdns = MdnsService::new("_udp_voice._udp.local.", properties);
    let _user_table = mdns.get_user_table();
    mdns.register_service("udp_server", ip, port);
    mdns.browse_services();

    // 1. Listen for udp messages in a port
    println!("SERVER: Binding to UDP socket on {}", ip_port);
    let socket = UdpSocket::bind(ip_port).expect("UDP: Failed to bind socket");
    println!("SERVER: UDP socket bound successfully");

    let output_device_copy = output_device.clone();
    let (sender_udp, receiver_audio) = channel();

    let udp_thread = std::thread::spawn( move ||{
        println!("SERVER: Started UDP receiving thread");
        loop {
            let mut header = [0u8; 4];
            // Receive the header first (4 bytes indicating the length of the data)
            if let Ok((_,_source)) = socket.recv_from(&mut header) {
                let mut cursor = Cursor::new(&header);
                let data_len = cursor.read_u32::<BigEndian>().unwrap();

                let mut encoded_data = vec![0; data_len as usize];
                if let Ok((amount, _source)) = socket.recv_from(&mut encoded_data) {
                    // send the block
                    // println!("Encoded data: {:?}", encoded_data);
                    if let Err(e) = sender_udp.send(encoded_data[..amount].to_vec()) {
                       eprintln!("SERVER: Failed to send data to audio thread: {:?}", e);
                    }
                } else {
                    println!("SERVER: Failed to receive data on UDP socket");
                }
            } else {
                error!("SERVER: Failed to receive header on UDP socket");
            }
        }
    });

    let (sender_decoder, receiver_dac) = channel();
    let decode_thread = std::thread::spawn(move || {
        let opus_channels = if channels == 1 {
            opus::Channels::Mono
        } else { opus::Channels::Stereo };
        // Double buffers for storing audio chunks
        let mut opus_decoder = Decoder::new(
            sample_rate as u32,
            opus_channels,
        ).unwrap();


        while let Ok(packet) = receiver_audio.recv() {

            let frame_size = 160;
            let num_frames = packet.len() as usize / frame_size;
            for i in 0..num_frames {
                let frame_start = i * frame_size;
                let frame_end = frame_start + frame_size;

                if frame_end <= packet.len() {
                    let frame = &packet[frame_start..frame_end];
                    let mut decoded_frame = vec![0.0; buffer_size * channels as usize];
                    match opus_decoder.decode_float(frame, &mut decoded_frame, true) {
                        Ok(len) => {
                            let decoded_data = decoded_frame[..len].to_vec();
                            println!("{}", format!("Decoded block of size: {}", len).magenta());
                            sender_decoder.send(decoded_data).expect("Failed to send decoded data");
                        }
                        Err(e) => {
                            eprintln!("Decoding error: {:?}", e);
                        }
                    }
                } else {
                    eprint!("Decoding error on frame {}", i);
                }
            }
        }
    });

    let dac_thread = std::thread::spawn(move || {
        println!("SERVER: Opus decoding completed, sending to DAC");
        dac(receiver_dac, buffer_size, &output_device_copy);
        println!("SERVER: Audio sent to DAC");
    });

    let _ = udp_thread.join();
    let _ = decode_thread.join();
    let _ = dac_thread.join();
}
