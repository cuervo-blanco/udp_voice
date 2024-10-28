use cpal::{
    Device,
    StreamConfig,
    traits::{DeviceTrait, StreamTrait},
};
#[allow(unused_imports)]
use byteorder::{BigEndian, ReadBytesExt, ByteOrder};
use selflib::mdns_service::MdnsService;
#[allow(unused_imports)]
use log::{debug, info, warn, error};
use selflib::settings::{Settings, ApplicationSettings};
#[allow(unused_imports)]
use std::{
    collections::{VecDeque, BTreeMap},
    io::{Cursor, Read},
    net::{UdpSocket, IpAddr},
    sync::{
        Arc,
        Mutex,
        mpsc::{channel, Sender, Receiver}
    },
    time::Instant,
    error::Error,
    thread::JoinHandle,
};
#[allow(unused_imports)]
use colored::*;
use opus::Decoder;
use cpal::SampleFormat;

#[allow(dead_code)]
#[derive(Debug)]
struct PacketData {
    sequence_number: u32,
    frame_size: u32,
    payload: Vec<u8>
}

const DATA_LEN_SIZE: usize = 4;
const SEQUENCE_NUM_SIZE: usize = 8;
const TIMESTAMP_SIZE: usize = 20;
const HEADER_SIZE: usize = DATA_LEN_SIZE + SEQUENCE_NUM_SIZE + TIMESTAMP_SIZE;
const PAYLOAD_SIZE: usize = 160 * 20;
const MAX_PACKET_SIZE: usize = HEADER_SIZE + PAYLOAD_SIZE;

fn main (){
    env_logger::init();
    let settings: ApplicationSettings = Settings::get_default_settings();
    let stream_config = settings.create_stream_config();
    let (channels, sample_rate, buffer_size, sample_format) = get_audio_config(&settings);

    let (_, output_device) = settings.get_devices();
    let output_device = Arc::new(Mutex::new(output_device));

    let ip =  local_ip_address::local_ip().unwrap();
    let port: u16 = 18521;
    let ip_port = format!("{}:{}", ip, port);

    let _mdns = setup_mdns(ip, port);
    println!("SERVER: Binding to UDP socket on {}", ip_port);
    let socket = UdpSocket::bind(ip_port).expect("UDP: Failed to bind socket");
    println!("SERVER: UDP socket bound successfully");

    let (sender_udp, receiver_audio) = channel();
    let (sender_decoder, receiver_dac) = channel();

    let delay_buffer_size = buffer_size * 100;
    let delay_buffer = Arc::new(
        Mutex::new(
            VecDeque::<f32>::with_capacity(delay_buffer_size)
        )
    );
    let delay_buffer_producer = Arc::clone(&delay_buffer);
    let playback_buffer = Arc::clone(&delay_buffer);

    // UDP Thread
    let udp_thread = start_udp_thread(socket, sender_udp, buffer_size * 20);

    // Decoder Thread
    let decoder_thread = start_decoder_thread(
        receiver_audio,
        sender_decoder,
        sample_rate,
        channels,
        buffer_size
    );

    // Producer Thread
    let producer_thread = start_producer_thread(
        receiver_dac,
        delay_buffer_producer,
        buffer_size
    );

    // DAC Thread
    let dac_thread = start_dac_thread(
        output_device,
        playback_buffer,
        stream_config,
        sample_format,
        buffer_size
    );

    let _ = udp_thread.join();
    let _ = decoder_thread.join();
    let _ = dac_thread.unwrap().join();
    let _ = producer_thread.join();
}

fn get_audio_config(settings: &ApplicationSettings) -> (u16, f32, usize, SampleFormat) {
    (
        settings.get_channels(),
        settings.get_sample_rate(),
        settings.get_buffer_size(),
        settings.get_config_files().1.sample_format(),
    )
}
fn setup_mdns(ip: IpAddr, port: u16) -> MdnsService {
    let service_type = "_udp_voice._udp.local.";
    let properties = vec![
        ("service name", "udp voice"),
        ("service type", service_type),
        ("version", "0.0.0"),
        ("interface", "server"),
    ];
    let mdns = MdnsService::new(service_type, properties);
    mdns.register_service("udp_server", ip, port);
    mdns.browse_services();
    mdns
}
fn start_udp_thread(socket: UdpSocket, sender_udp: Sender<(Vec<u8>, u32)>, min_buffer_fill: usize) -> JoinHandle<()> {
    // [Data Length (4 bytes)] + [Sequence Number (8 bytes)] + [Timestamp (20 bytes)] + [Payload (variable length)]
    let jitter_buffer: Arc<Mutex<BTreeMap<u32, PacketData>>> = Arc::new(Mutex::new(BTreeMap::new()));
    std::thread::spawn(move || {
        let mut prev_packet_time: Option<Instant> = None;
        loop {
            let mut buf = [0u8; MAX_PACKET_SIZE];
            while let Ok((amount, _src)) = socket.recv_from(&mut buf) {
                let current_packet_time = Instant::now();
                if let Some(prev_time) = prev_packet_time {
                println!(
                    "Time since last packet: {:.3} ms",
                    current_packet_time.duration_since(prev_time).as_secs_f64() * 1000.0);
                }
                prev_packet_time = Some(current_packet_time);
                let mut cursor = Cursor::new(&buf[0..amount]);
                let packet_len = match cursor.read_u32::<BigEndian>() {
                    Ok(len) => len,
                    Err(e) => {
                        eprintln!("Error reading packet length: {:?}", e);
                        continue;
                    }
                };
                let mut sequence_num_buf = [0u8; 8];
                if let Err(e) = cursor.read_exact(&mut sequence_num_buf) {
                    eprintln!("Error reading sequence number: {:?}", e);
                    continue;
                }
                if sequence_num_buf[0..2] != [0xCC, 0xDD] || sequence_num_buf[6..8] != [0xDD, 0xCC] {
                    eprintln!("Invalid sequence number header");
                    continue;
                }
                let sequence_number = BigEndian::read_u32(&sequence_num_buf[2..6]);

                let mut time_in_ms_buf = [0u8; 20];
                if let Err(e) = cursor.read_exact(&mut time_in_ms_buf) {
                    eprintln!("Error reading sequence number: {:?}", e);
                    continue;
                }
                if time_in_ms_buf[0..2] != [0xAA, 0xBB] || time_in_ms_buf[18..20] != [0xBB, 0xAA] {
                    eprintln!("Invalid timestamp header");
                    continue;
                }
                let timestamp = BigEndian::read_u128(&time_in_ms_buf[2..18]);

                let mut frame_size_buf = [0u8; 8];
                if let Err(e) = cursor.read_exact(&mut frame_size_buf) {
                    eprintln!("Error reading frame size number: {:?}", e);
                    continue;
                }
                if frame_size_buf[0..2] != [0xEE, 0xFF] || frame_size_buf[6..8] != [0xFF, 0xEE] {
                    eprintln!("Invalid frame size number header");
                    continue;
                }
                let frame_size = BigEndian::read_u32(&frame_size_buf[2..6]);
                println!("packet_len: {packet_len}, sequence_number: {sequence_number}, timestamp: {timestamp}, frame_size: {frame_size}");

                let payload_start = cursor.position() as usize;
                let payload = &buf[payload_start..amount];
                // let payload = pad_data(payload, data_len, amount);
                let packet = PacketData {
                    sequence_number,
                    frame_size,
                    payload: payload.to_vec(),
                };

                {
                    let mut buffer = jitter_buffer.lock().expect("Unable to acquire jitter buffer lock");
                    buffer.insert(sequence_number, packet);
                }
                handle_jitter_buffer(jitter_buffer.clone(), sender_udp.clone(), min_buffer_fill, frame_size);
            }
        }
    })
}

fn _pad_data(mut data: Vec<u8>, expected_len: u32, received_len: usize) -> Vec<u8> {
    if received_len < expected_len as usize {
        data.extend(vec![0; expected_len as usize - received_len]);
    }
    data
}

fn handle_jitter_buffer(
    jitter_buffer: Arc<Mutex<BTreeMap<u32, PacketData>>>,
    sender_udp: Sender<(Vec<u8>, u32)>,
    min_buffer_fill: usize,
    frame_size: u32,
) {
    let mut buffer = jitter_buffer.lock().expect("Unable to get lock for jitter buffer");
    // if the buffer is filled
    if buffer.len() >= min_buffer_fill {
        let keys: Vec<_> = buffer.keys().cloned().collect();
        let min_seq = *keys.first().unwrap();
        let max_seq = *keys.last().unwrap();

        for expected_seq in min_seq..=max_seq {
            if !buffer.contains_key(&expected_seq) {
                let reference_payload = match buffer.range(..expected_seq).next_back() {
                    Some((_seq, packet)) => &packet.payload,
                    None => {
                        continue;
                    }
                };
                let interpolated_data = interpolate_placeholder(reference_payload);
                buffer.insert(expected_seq, PacketData {
                    sequence_number: expected_seq,
                    frame_size,
                    payload: interpolated_data,
                });
            }
        }
        println!("Final Buffer: {:?}", buffer);
        for (_, packet) in buffer.iter() {
            if let Err(e) = sender_udp.send((packet.payload.clone(), frame_size)) {
                eprintln!("SERVER: Failed to send data to audio thread: {:?}", e);
            }
        }
        buffer.clear();
    }
}
fn interpolate_placeholder(prev_payload: &[u8]) -> Vec<u8> {
    prev_payload.to_vec()
}

fn start_decoder_thread(
    receiver_audio: Receiver<(Vec<u8>, u32)>,
    sender_decoder: Sender<Vec<f32>>,
    sample_rate: f32,
    channels: u16,
    target_fill_rate: usize,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let opus_channels = if channels == 1 {
            opus::Channels::Mono
        } else {
            opus::Channels::Stereo
        };
        let mut opus_decoder = Decoder::new(sample_rate as u32, opus_channels).unwrap();
        let mut accumulated_samples = Vec::with_capacity(1920);

        while let Ok((packet, frame_size)) = receiver_audio.recv() {
            let mut offset = 0;
            while offset + 2 <= packet.len() {
                offset += 2;
                if offset + frame_size as usize > packet.len() {
                    eprintln!("Incomplete frame detected");
                    break;
                }
                let frame = &packet[offset..offset+frame_size as usize];
                offset += frame_size as usize;
                let decoded_samples = decode_packet(frame.to_vec(), &mut opus_decoder, channels);
                if decoded_samples.is_empty(){
                    let interpolated_samples = repeat_previous_frame(&accumulated_samples, target_fill_rate);
                    accumulated_samples.extend_from_slice(&interpolated_samples);
                } else {
                    accumulated_samples.extend(decoded_samples);
                }
                if accumulated_samples.len() >= target_fill_rate {
                    sender_decoder
                        .send(accumulated_samples
                            .drain(..target_fill_rate)
                            .collect::<Vec<f32>>()
                        )
                        .expect("Failed to send decoded data");
                }
            }
        }
    })
}

fn decode_packet(
    packet: Vec<u8>,
    decoder: &mut Decoder,
    channels: u16,
) -> Vec<f32> {
    let max_frame_size = 5760 * channels as usize;
    let mut decoded = vec![0.0; max_frame_size];
    match decoder.decode_float(&packet, &mut decoded, false) {
        Ok(len) => {
            decoded.truncate(len * channels as usize);
            decoded
        },
        Err(e) => {
            eprintln!("Decoding failed: {:?}", e);
            vec![] // Return empty vec if decoding fails
        }
    }
}

fn repeat_previous_frame(
    prev_samples: &[f32],
    target_fill_rate: usize
) -> Vec<f32> {
    if prev_samples.is_empty() {
        vec![0.0; target_fill_rate] // Fill with silence if no previous data
    } else {
        prev_samples.iter().cycle().take(target_fill_rate).cloned().collect()
    }
}

fn start_producer_thread(
    receiver_dac: Receiver<Vec<f32>>,
    delay_buffer: Arc<Mutex<VecDeque<f32>>>,
    buffer_size: usize,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        while let Ok(block) = receiver_dac.recv() {
            let mut buffer = delay_buffer.lock().expect("Failed to lock delay buffer for producer");
            buffer.extend(block);
            while buffer.len() > buffer_size * 100 {
                buffer.drain(..buffer_size);
            }
        }
    })
}

fn start_dac_thread(
    device: Arc<Mutex<Device>>,
    delay_buffer: Arc<Mutex<VecDeque<f32>>>,
    stream_config: StreamConfig,
    sample_format: SampleFormat,
    buffer_size: usize,
) -> Result<JoinHandle<()>, Box<dyn Error>> {
    let delay_buffer_clone = Arc::clone(&delay_buffer);
    let dac_thread = std::thread::spawn(move || {
        wait_for_buffer_fill(&delay_buffer_clone, stream_config.channels as usize * buffer_size);
        play_stream(device, delay_buffer_clone, stream_config, sample_format);
    });

    Ok(dac_thread)
}

fn wait_for_buffer_fill(buffer: &Arc<Mutex<VecDeque<f32>>>, target_size: usize) {
    loop {
        let buffer_len = buffer.lock().expect("Failed to acquire playback buffer lock").len();
        if buffer_len >= target_size * 4 / 5 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn play_stream(
    device: Arc<Mutex<cpal::Device>>,
    buffer: Arc<Mutex<VecDeque<f32>>>,
    stream_config: cpal::StreamConfig,
    sample_format: SampleFormat,
) {
    let stream = match sample_format {
        SampleFormat::F32 => {
            device.lock().unwrap().build_output_stream(
                &stream_config,
                move |data: &mut [f32], _| fill_audio_data(data, &buffer),
                |err| eprintln!("DAC: Stream error: {}", err),
                None,
            )
        }
        SampleFormat::I16 => todo!(),
        SampleFormat::U16 => todo!(),
        _ => panic!("DAC: Unsupported sample format '{:?}'", sample_format),
    }
    .unwrap();

    stream.play().expect("Failed to play stream");
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

fn fill_audio_data(data: &mut [f32], buffer: &Arc<Mutex<VecDeque<f32>>>) {
    let mut buffer = buffer.lock().expect("Failed to lock buffer for playback");
    for sample in data.iter_mut() {
        *sample = buffer.pop_front().unwrap_or(0.0);
    }
}

