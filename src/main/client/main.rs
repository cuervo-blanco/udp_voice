#[allow(unused_imports)]
use std::{
    io::{Write, stdout},
    collections::HashMap,
    sync::{
        Arc, Mutex,
        mpsc::{channel, Sender, Receiver},
    },
    net::{UdpSocket, IpAddr},
    error::Error,
    time::{SystemTime, UNIX_EPOCH},
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
    let settings: ApplicationSettings = Settings::get_default_settings();
    let (sample_rate, channels, buffer_size) = get_audio_config(&settings);
    println!("");
    println!("{}", "Enter Username:".cyan());
    println!("");
    let username = username_take();
    let instance_name = Arc::new(Mutex::new(username));
    let ip =  local_ip_address::local_ip().unwrap();
    let port: u16 = 18522;

    let mdns = setup_mdns(instance_name, ip, port);
    let user_table = mdns.get_user_table();

    event_loop(sample_rate, channels, buffer_size, ip, port, user_table)

}

fn get_audio_config(settings: &ApplicationSettings) -> (f32, u16, usize) {
    (
        settings.get_sample_rate(),
        settings.get_channels(),
        settings.get_buffer_size(),
    )
}

fn setup_mdns(instance_name: Arc<Mutex<String>>, ip: IpAddr, port: u16) -> MdnsService {
    let properties = vec![
        ("service name", "udp voice"),
        ("service type", "_udp_voice._udp_local."),
        ("version", "0.0.2"),
        ("interface", "client")
    ];
    let mdns = MdnsService::new("_udp_voice._udp.local.", properties);
    mdns.register_service(&instance_name.lock().unwrap(), ip, port);
    mdns.browse_services();
    mdns
}

fn event_loop (
    sample_rate: f32,
    channels: u16,
    buffer_size: usize,
    ip: IpAddr,
    port: u16,
    user_table: Arc<Mutex<HashMap<String, String>>>,
) -> Result<(), Box<dyn Error>> {
    loop {
        let input = get_user_input();
        let opus_channels = if channels == 1 {
            opus::Channels::Mono
        } else {
            opus::Channels::Stereo
        };

        match input.as_str() {
            "send" => start_sending(
                sample_rate,
                channels,
                buffer_size,
                ip.clone(),
                port,
                opus_channels,
                user_table.clone(),
            ),
            "exit" => return Ok(()),
            _ => println!("{}", "Not a permitted command".red()),
        }
    }
}
fn get_user_input() -> String {
    let mut buffer = String::new();
    std::io::stdin().read_line(&mut buffer).unwrap();
    buffer.trim().to_string()
}
fn start_sending(
    sample_rate: f32,
    channels: u16,
    buffer_size: usize,
    ip: IpAddr,
    port: u16,
    opus_channels: opus::Channels,
    user_table: Arc<Mutex<HashMap<String, String>>>,
) {
    let (output_sine, input_encoder) = channel();
    let (output_encoder, input_buffer) = channel();

    Sine::new(440.0, 1.0, sample_rate as u32, channels as usize, output_sine, buffer_size);

    std::thread::spawn(move || encode_opus(input_encoder, output_encoder, sample_rate, opus_channels, buffer_size));

    std::thread::spawn(move || batch_and_send_udp(ip, port, input_buffer, user_table));
}
fn encode_opus(
    input_encoder: Receiver<Vec<f32>>,
    output_encoder: Sender<Vec<u8>>,
    sample_rate: f32,
    opus_channels: opus::Channels,
    buffer_size: usize,
) {
    let mut opus_encoder = Encoder::new(sample_rate as u32, opus_channels, Application::Audio).unwrap();
    opus_encoder.set_bitrate(opus::Bitrate::Bits(64000)).unwrap();
    opus_encoder.set_vbr(false).unwrap();

    while let Ok(block) = input_encoder.recv() {
        let mut encoded_block = vec![0; buffer_size];
        if let Ok(len) = opus_encoder.encode_float(&block, &mut encoded_block) {
            output_encoder.send(encoded_block[..len].to_vec()).expect("Failed to send encoded data");
        }
    }
}

fn batch_and_send_udp(
    ip: IpAddr,
    port: u16,
    input_buffer: Receiver<Vec<u8>>,
    user_table: Arc<Mutex<HashMap<String, String>>>,
) {
    let ip_port = format!("{}:{}", ip, port);
    let socket = UdpSocket::bind(&ip_port).expect("UDP: Failed to bind to socket");

    let packet_amount = 20;
    let mut batch_buffer = Vec::new(); // Start with an empty vector
    let mut sequence_number = 0;

    loop {
        if let Ok(block) = input_buffer.recv() {
            let frame_length = block.len() as u16; // Assuming frame length fits in u16

            // Include the length of the frame before the frame data
            batch_buffer.extend_from_slice(&frame_length.to_be_bytes());
            batch_buffer.extend_from_slice(&block);

            if batch_buffer.len() >= packet_amount * (frame_length as usize + 2) {
                for (_user, address) in user_table.lock().unwrap().clone() {
                    send_packet(&socket, &address, &batch_buffer, sequence_number);
                    sequence_number += 1;
                }
                batch_buffer.clear();
            }
        }
    }
}
fn send_packet(socket: &UdpSocket, address: &str, batch_buffer: &[u8], sequence_number: u32) {
    let port = format!("{}:18521", address);
    let packet = create_packet(batch_buffer, sequence_number);
    socket.send_to(&packet, &port).expect("Failed to send data");
}
fn current_time_in_ms() -> Vec<u8> {
    let start = SystemTime::now();
    let since_epoch = start.duration_since(UNIX_EPOCH)
        .expect("Time went backwards");
    let ms = since_epoch.as_millis();
    let mut bytes = ms.to_be_bytes().to_vec();
    let mut timestamp = vec![0xAA, 0xBB];
    timestamp.append(&mut bytes);
    timestamp.extend_from_slice(&[0xBB, 0xAA]);
    timestamp
}

fn sequencer(sequence_number: u32) -> Vec<u8> {
    let mut bytes = sequence_number.to_be_bytes().to_vec();
    let mut sequence = vec![0xCC, 0xDD];
    sequence.append(&mut bytes);
    sequence.extend_from_slice(&[0xDD, 0xCC]);
    sequence
}

fn create_packet(batch_buffer: &[u8], sequence_number: u32) -> Vec<u8> {
    let data_len = batch_buffer.len() as u32;
    let time_in_ms = current_time_in_ms();
    let sequence_num  = sequencer(sequence_number);

    let sequence_num_len = sequence_num.len() as u8;
    let time_in_ms_len = time_in_ms.len() as u8;

    let mut packet = Vec::with_capacity(
        4 + sequence_num_len as usize + time_in_ms_len as usize + batch_buffer.len(),
    );

    packet.write_u32::<BigEndian>(data_len).unwrap();
    packet.extend_from_slice(&sequence_num);
    packet.extend_from_slice(&time_in_ms);
    packet.extend_from_slice(batch_buffer);
    packet

}


