use std::sync::{Arc, Mutex};
use ringbuf::HeapRb;
use ringbuf::traits::{Consumer, Producer, Split};
use selflib::mdns_service::MdnsService;
use log::debug;
use std::net::UdpSocket;
use selflib::settings::Settings;
use selflib::sound::{dac, decode_opus};
use std::sync::mpsc::channel;

fn main (){
    env_logger::init();

    let settings = Settings::get_default_settings();
    let channels = settings.get_channels();
    let (_input_device, output_device) = settings.get_devices();
    let output_device = Arc::new(Mutex::new(output_device));
    let (_input_config, output_config) = settings.get_config_files();
    let buffer_size = settings.get_buffer_size();

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
    let mut buffer = vec![0; buffer_size * channels as usize];

    let ring = HeapRb::<u8>::new(buffer_size * channels as usize);
    let (mut producer, mut consumer) = ring.split();

    let (output_socket, input_decoder) = channel();
    let (output_decoder, input_dac) = channel();

    std::thread::spawn( move ||{
        loop {
            if let Ok((amount, _source)) = socket.recv_from(&mut buffer) {
                producer.push_slice(&mut buffer[..amount]);
            }
        }
    });

    let output_device_clone = output_device.clone();

    std::thread::spawn(move || {
        let output_device = output_device_clone.lock().unwrap();
        loop {
            let mut decode_buffer = vec![0; buffer_size * channels as usize];
            consumer.pop_slice(&mut decode_buffer);
            if let Ok(_) = output_socket.send(decode_buffer) {
                // Now the dec
                decode_opus(input_decoder, output_decoder);
                dac(input_dac, buffer_size, output_device);

            }
        }
    });

    // Send this data to a port, have a different Application in Front End.

}
