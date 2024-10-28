use cpal::{
    Device,
    StreamConfig,
    traits::{DeviceTrait, StreamTrait},
};
use byteorder::{BigEndian, ReadBytesExt};
use selflib::mdns_service::MdnsService;
#[allow(unused_imports)]
use log::{debug, info, warn, error};
use selflib::settings::{Settings, ApplicationSettings};
use std::{
    collections::VecDeque,
    io::Cursor,
    net::{UdpSocket, IpAddr},
    sync::{Arc, Mutex},
    time::Instant,
    error::Error,
    thread::JoinHandle,
};
use std::sync::mpsc::{channel, Sender, Receiver};
#[allow(unused_imports)]
use colored::*;
use opus::Decoder;
use cpal::SampleFormat;

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
    let udp_thread = start_udp_thread(socket, sender_udp);

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
    let service_type = "udp_voice._udp.local.";
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
fn start_udp_thread(socket: UdpSocket, sender_udp: Sender<Vec<u8>>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut prev_packet_time: Option<Instant> = None;

        loop {
            let mut header = [0u8; 4];
            if let Ok(_) = socket.recv_from(&mut header) {
                let current_packet_time = Instant::now();
                if let Some(prev_time) = prev_packet_time {
                    println!(
                        "Time since last packet: {:.3} ms",
                        current_packet_time.duration_since(prev_time).as_secs_f64() * 1000.0
                    );
                }
                prev_packet_time = Some(current_packet_time);

                let data_len = Cursor::new(&header).read_u32::<BigEndian>().unwrap();
                let mut encoded_data = vec![0; data_len as usize];
                if let Ok((amount, _)) = socket.recv_from(&mut encoded_data) {
                    if let Err(e) = sender_udp.send(pad_data(encoded_data, data_len, amount)) {
                        eprintln!("SERVER: Failed to send data to audio thread: {:?}", e);
                    }
                }
            }
        }
    })
}

fn pad_data(mut data: Vec<u8>, expected_len: u32, received_len: usize) -> Vec<u8> {
    if received_len < expected_len as usize {
        data.extend(vec![0; expected_len as usize - received_len]);
    }
    data
}

fn start_decoder_thread(
    receiver_audio: Receiver<Vec<u8>>,
    sender_decoder: Sender<Vec<f32>>,
    sample_rate: f32,
    channels: u16,
    buffer_size: usize,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let opus_channels = if channels == 1 {
            opus::Channels::Mono
        } else {
            opus::Channels::Stereo
        };
        let mut opus_decoder = Decoder::new(sample_rate as u32, opus_channels).unwrap();
        let mut accumulated_samples = Vec::with_capacity(1920);

        while let Ok(packet) = receiver_audio.recv() {
            process_packet(
                packet,
                &mut opus_decoder,
                &mut accumulated_samples,
                sender_decoder.clone(),
                buffer_size,
                channels as usize
            );
        }
    })
}

fn process_packet(
    packet: Vec<u8>,
    opus_decoder: &mut Decoder,
    accumulated_samples: &mut Vec<f32>,
    sender_decoder: Sender<Vec<f32>>,
    buffer_size: usize,
    channels: usize,
) {
    let frame_size = 160;
    let num_frames = (packet.len() + frame_size - 1) / frame_size;

    for i in 0..num_frames {
        let frame_start = i * frame_size;
        if frame_start + frame_size <= packet.len() {
            if let Ok(len) = opus_decoder.decode_float(
                &packet[frame_start..frame_start + frame_size],
                &mut vec![0.0; buffer_size * channels],
                false,
            ) {
                accumulated_samples.extend_from_slice(&vec![0.0; len]);
                if accumulated_samples.len() >= 1920 {
                    sender_decoder.send(accumulated_samples.drain(..1920).collect()).expect("Failed to send decoded data");
                }
            }
        }
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







