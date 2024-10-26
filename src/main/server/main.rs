use std::io::Cursor;
use byteorder::{BigEndian, ReadBytesExt};
use std::sync::{Arc, Mutex};
use ringbuf::HeapRb;
use ringbuf::traits::{Consumer, Producer, Split};
use selflib::mdns_service::MdnsService;
#[allow(unused_imports)]
use log::{debug, info, warn, error};
use std::net::UdpSocket;
use selflib::settings::{Settings, ApplicationSettings};
use selflib::sound::{dac, decode_opus};
use std::sync::mpsc::channel;
use std::time::Duration;

fn main (){
    env_logger::init();

    let settings: ApplicationSettings = Settings::get_default_settings();
    let channels = settings.get_channels();
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
        // Admin, etc.
    ];

    let mdns = MdnsService::new("_udp_voice._udp.local.", properties);
    let _user_table = mdns.get_user_table();
    mdns.register_service("udp_server", ip, port);
    mdns.browse_services();

    // 1. Listen for udp messages in a port
    println!("SERVER: Binding to UDP socket on {}", ip_port);
    let socket = UdpSocket::bind(ip_port).expect("UDP: Failed to bind socket");
    println!("SERVER: UDP socket bound successfully");

    let ring = HeapRb::<u8>::new(buffer_size * channels as usize);
    let (mut producer, mut consumer) = ring.split();
    println!("SERVER: Initialized ring buffer with size: {}", buffer_size * channels as usize);


    std::thread::spawn( move ||{
        println!("SERVER: Started UDP receiving thread");
        loop {
            let mut header = [0u8; 4];
            // Receive the header first (4 bytes indicating the length of the data)
            if let Ok((_,source)) = socket.recv_from(&mut header) {
                println!("SERVER: Received header from {}", source);

                let mut cursor = Cursor::new(&header);
                let data_len = cursor.read_u32::<BigEndian>().unwrap();
                println!("SERVER: Expecting to recieve {} bytes of data", data_len);
                
                let mut encoded_data = vec![0; data_len as usize];

                if let Ok((amount, source)) = socket.recv_from(&mut encoded_data) {
                    println!("SERVER: Received {} bytes from {}", amount, source);
                    producer.push_slice(&mut encoded_data[..amount]);
                    println!("SERVER: Pushed {} bytes into ring buffer", amount);
                } else {
                    println!("SERVER: Failed to receive data on UDP socket");
                }
            } else {
                error!("SERVER: Failed to receive header on UDP socket");
            }
        }
    });


    let output_device_copy = output_device.clone();
    std::thread::spawn(move || {
        println!("SERVER: Started audio processing thread");
        loop {
            let (sender_socket, receiver_decoder) = channel();
            let (sender_decoder, receiver_dac) = channel();

            let mut decode_buffer = vec![0; buffer_size * channels as usize];
            let bytes_popped = consumer.pop_slice(&mut decode_buffer);
            println!("SERVER: Popped {} bytes from ring buffer", bytes_popped);
            if let Ok(_) = sender_socket.send(decode_buffer) {
                // Now the dec
                let _ = decode_opus(receiver_decoder, sender_decoder);
                println!("SERVER: Opus decoding completed, sending to DAC");
                dac(receiver_dac, buffer_size, &output_device_copy);
                println!("SERVER: Audio sent to DAC");
            } else {
                println!("SERVER: Failed to send data to decoder");
            }
        }
    });

    // Send this data to a port, have a different Application in Front End.
    loop {
        // Do Something;
        std::thread::sleep(Duration::from_millis(10000));
    }

}
