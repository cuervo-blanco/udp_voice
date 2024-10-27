use std::collections::VecDeque;
use cpal::traits::{DeviceTrait, StreamTrait};
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
#[allow(unused_imports)]
use selflib::sound::dac;
use std::sync::mpsc::channel;
#[allow(unused_imports)]
use colored::*;
use opus::Decoder;
use cpal::SampleFormat;

fn main (){
    env_logger::init();

    let settings: ApplicationSettings = Settings::get_default_settings();
    let channels = settings.get_channels();
    let sample_rate = settings.get_sample_rate();
    println!("Sample Rate: {:?}", sample_rate);
    let (_input_device, output_device) = settings.get_devices();
    let output_device = Arc::new(Mutex::new(output_device));
    let (_input_config, output_config) = settings.get_config_files();
    let buffer_size = settings.get_buffer_size();
    let sample_format = output_config.sample_format();

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

    let (sender_udp, receiver_audio) = channel();

    let udp_thread = std::thread::spawn( move ||{
        println!("SERVER: Started UDP receiving thread");
        loop {
            let mut header = [0u8; 4];
            // Receive the header first (4 bytes indicating the length of the data)
            if let Ok((_,_source)) = socket.recv_from(&mut header) {
                let mut cursor = Cursor::new(&header);
                let data_len = cursor.read_u32::<BigEndian>().unwrap();
                println!("Data len (from header): {data_len}");

                let mut encoded_data = vec![0; data_len as usize];
                if let Ok((amount, _source)) = socket.recv_from(&mut encoded_data) {
                    println!("UDP: Received data (with header): {}", amount);
                    if amount != data_len as usize {
                        eprint!("{}", format!("Warning: Expected {} bytes but received {}", data_len, amount).red());
                    }
                    let audio_data = if amount > 4 {
                        encoded_data[4..amount].to_vec()
                    } else {
                        vec![0; amount]
                    };
                    println!("UDP: Received data (without header): {}", audio_data.len());
                    if let Err(e) = sender_udp.send(audio_data) {
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
            // println!("Packet to decode: {:?}", packet);
            //println!("Packet Size: {}", packet.len());
            let frame_size = 160;
            let num_frames = (packet.len() + frame_size - 1) / frame_size;
            println!("Number of frames : {}", num_frames);
            for i in 0..num_frames {
                let frame_start = i * frame_size;
                //println!("FRAME {} Start: {}", i + 1, frame_start);
                let frame_end = frame_start + frame_size;
                //println!("FRAME {} End: {}", i + 1, frame_end);

                if frame_end <= packet.len() {
                    let frame = &packet[frame_start..frame_end];
                    println!("DECODER: FRAME {} Size: {:?}", i + 1,  frame.len());
                    //println!("FRAME {}: {:?}", i + 1, frame);
                    let mut decoded_frame = vec![0.0; buffer_size * channels as usize];
                    match opus_decoder.decode_float(frame, &mut decoded_frame, false) {
                        Ok(len) => {
                            let decoded_data = decoded_frame[..len].to_vec();
                            //println!("{}", format!("DECODED FRAME {}: {:?}", i+1, decoded_data).yellow());
                            println!("{}", format!("DECODER: Decoded block of size: {}", len).magenta());
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
        //println!("SERVER: Opus decoding completed, sending to DAC");
        let config: cpal::StreamConfig = output_config.into();
        let config_channels = config.channels as usize;
        let expected_data_len = buffer_size * config_channels;

        println!("Expected data length per callback: {}", expected_data_len);

        debug!("DAC: Initialized with Channels: {}, Buffer Size: {}", channels, buffer_size);

        let buffer = Arc::new(Mutex::new(VecDeque::with_capacity(buffer_size * channels as usize)));

        {
            let buffer = Arc::clone(&buffer);
            std::thread::spawn(move || {
                while let Ok(block) = receiver_dac.recv() {
                    println!("BUFFER: Buffer block received length: {}", block.len());
                    let mut buffer = buffer.lock().expect("Failed to lock buffer for producer");
                    println!("BUFFER: Initial Size: {}", buffer.len());
                    if buffer.len() + block.len() > buffer_size * 4 {
                        buffer.drain(..block.len());
                    }
                    buffer.extend(block);
                    println!("BUFFER: Buffer to send to callback: {}", buffer.len());
                }
            });
        }

        let device = output_device.lock().unwrap();
        info!("DAC: Output device locked and ready");

        let buffer_for_playback = Arc::clone(&buffer);

        let stream = match sample_format {
            SampleFormat::F32 => {
                device.build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let mut buffer = buffer_for_playback.lock().expect("Failed to lock buffer for playback");
                        let available_samples = buffer.len();
                        let requested_samples = data.len();
                        println!("CALLBACK: Buffer: {:?}", available_samples);
                        println!("CALLBACK: Data: {:?}", requested_samples);

                    if buffer.len() >= data.len() {
                        for i in 0..requested_samples {
                            data[i] = if i < available_samples {
                                buffer.pop_front().unwrap()
                            } else {
                                    0.0
                                };
                        }
                    } else {
                        for sample in data.iter_mut() {
                            *sample = 0.0;
                        }
                    }

                },
                move |err| {
                    println!("DAC: Failed to output samples into stream: {}", err);
                },
                None //None=blocking, Some(Duration)=timeout
            )},
            SampleFormat::I16 => {
                println!("DAC: Not yet implemented(I16)");
                todo!();
            },
            SampleFormat::U16 => {
                println!("DAC: Not yet implemented (U16)");
                todo!();
            }
            sample_format => panic!("DAC: Unsupported sample format '{sample_format}'")
        }.unwrap();

        //println!("DAC: Starting the audio stream");
        stream.play().expect("Failed to play stream");
        println!("Stream is playing");
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }

    });

    let _ = udp_thread.join();
    let _ = decode_thread.join();
    let _ = dac_thread.join();
}
