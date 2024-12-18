use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{Sender, Receiver};
use opus::{Encoder, Decoder, Application};
use cpal::SampleFormat;
use cpal::traits::{DeviceTrait, StreamTrait};
use ringbuf::{
    traits::{Consumer, Producer, Split, Observer},
    HeapRb,
};
use colored::*;
use log::{info, warn, error, debug};
#[allow(unused_imports)]
use crate::settings::{Settings, ApplicationSettings};

pub fn dac(
    receiver: Receiver<Vec<f32>>,
    buffer_size: usize,
    device: &Arc<Mutex<cpal::Device>>,
    ) {

    let settings: ApplicationSettings = Settings::get_default_settings();
    let channels = settings.get_channels();
    let (_, config) = settings.get_config_files();
    debug!("DAC: Initialized with Channels: {}, Buffer Size: {}", channels, buffer_size);

    let buffer = Arc::new(Mutex::new(VecDeque::with_capacity(buffer_size * channels as usize)));

    let buffer_buffer = Arc::clone(&buffer);
    std::thread::spawn(move || {
        while let Ok(block) = receiver.recv() {
            let mut buffer = buffer_buffer.lock().expect("Failed to lock buffer for producer");
            for sample in block {
                buffer.push_back(sample);
            }
        }
    });
    let sample_format = config.sample_format();
    let config: cpal::StreamConfig = config.into();

    let device = device.lock().unwrap();
    info!("DAC: Output device locked and ready");

    let buffer_for_playback = Arc::clone(&buffer);

    let stream = match sample_format {
        SampleFormat::F32 => {
            info!("DAC: Building output stream with format F32");
            device.build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let mut buffer = buffer_for_playback.lock().expect("Failed to lock buffer for consumer");
                for sample in data.iter_mut() {
                    println!("Sample: {sample}");
                    *sample = buffer.pop_front().unwrap_or(0.0);
                }
            },
            move |err| {
                // react to errors here.
                error!("DAC: Failed to output samples into stream: {}", err);
            },
            None //None=blocking, Some(Duration)=timeout
        )},
        SampleFormat::I16 => {
            info!("DAC: Not yet implemented(I16)");
            todo!();
        },
        SampleFormat::U16 => {
            info!("DAC: Not yet implemented (U16)");
            todo!();
        }
        sample_format => panic!("DAC: Unsupported sample format '{sample_format}'")
    }.unwrap();

    info!("DAC: Starting the audio stream");
    stream.play().expect("Failed to play stream");
    loop {
        std::thread::sleep(std::time::Duration::from_millis(1000));
    }

}
pub fn encode_opus_v1(
    receiver: Receiver<Vec<f32>>,
    sender: Sender<Vec<u8>>,
    ) -> Result<(), opus::Error> {
    info!("ENCODER: Encoder started");

    // Set settings
    let settings: ApplicationSettings = Settings::get_default_settings();
    let channels = settings.get_channels();
    let buffer_size = settings.get_buffer_size();
    let sample_rate = settings.get_sample_rate();
    let specs = format!("ENCODER: CHANNELS: {}, BUFFER_RATE: {}, SAMPLE_RATE: {}",
        channels, buffer_size, sample_rate);
    println!("{}", &specs);

    // Initialize Ring Buffer
    // This ring buffer stores incoming PCM data
    let ring = HeapRb::<f32>::new(buffer_size * channels as usize);
    let (mut producer, mut consumer) = ring.split();
    let ring_init = format!("ENCODER: Ring Buffer initialized with size: {} bytes",
        buffer_size * channels as usize);
    println!("{}", &ring_init);

    std::thread::spawn( move || {
        println!("ENCODER: Spawned producer thread");
        let mut counter = 0;
        while let Ok(block) = receiver.recv() {
            for sample in block {
                while producer.is_full() {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                counter += 1;
                producer.try_push(sample).expect("Failed to push into producer");

                if counter % 48000 == 0 {
                    println!("ENCODER: Pushing into buffer: {}", &sample);
                }

            }
            info!("ENCODER: Block successfully pushed to producer");
        }
        info!("ENCODER: Receiver channels closed, producer thread exiting");
    });

    let mut opus_channels: opus::Channels = opus::Channels::Stereo;

    if channels == 1 {
        opus_channels = opus::Channels::Mono;
    } else if channels > 2 {
        warn!("ENCODER: Channels are more than 2");
    }

    let _opus_info = format!("ENCODER: Opus encoder channels set to: {:?}", opus_channels);

    // Here the Application can be Voip, Audio, LowDelay
    let mut opus_encoder = Encoder::new(sample_rate as u32, opus_channels, Application::Audio)?;
    println!("ENCODER: Opus encoder initialized");

    loop {
        // Consumer fills encoded_block_buffer, once filled it is returned
        // "Decoded" meaning it hasn't been coded yet
        let mut counter = 0;
        let mut decoded_block: Vec<f32> = vec![0.0; buffer_size * channels as usize];

        println!("ENCODER: Prepared decoded block with size: {}", decoded_block.len());

        for sample in decoded_block.iter_mut() {
            while let Some(bit) = consumer.try_pop() {
                *sample = bit;
                counter += 1;
                if counter % 48000 == 0 {
                    println!("ENCODER: Pushing into decoded_block: {}", *sample);
                }

            }
        }
        println!("ENCODER: Filled decoded block buffer");

        let mut encoded_block = vec![0; buffer_size * channels as usize];
        let len = opus_encoder.encode_float(&decoded_block, &mut encoded_block)?;
        let encoded_data = encoded_block[..len].to_vec();
        println!("ENCODER: Encoded block of size: {}", len);

        sender.send(encoded_data.clone()).unwrap();
        println!("ENCODER: Encoded block sent through sender channel");
    }

}

pub fn encode_opus(
    receiver: Receiver<Vec<f32>>,
    sender: Sender<Vec<u8>>,
) -> Result<(), opus::Error> {
    let settings: ApplicationSettings = Settings::get_default_settings();
    let channels = settings.get_channels();
    let buffer_size = settings.get_buffer_size();
    let sample_rate = settings.get_sample_rate();
    let opus_channels = if channels == 1 { opus::Channels::Mono } else { opus::Channels::Stereo };

    // println!("Encoder initialized with sample rate: {}, channels: {}", sample_rate, channels);
    // Double buffers for storing audio chunks
    while let Ok(block) = receiver.recv() {
        let mut opus_encoder = Encoder::new(
            sample_rate as u32,
            opus_channels,
            Application::Audio
            )?;
        // Swap active buffers to avoid blocking
        // println!("Copied new audio block of size {} into inactive buffer", block.len());
        let mut encoded_block = vec![0; buffer_size * channels as usize];
        if let Ok(len) = opus_encoder.encode_float(&block, &mut encoded_block) {
            let data_len = format!("ENCODER: Encoded block of size: {}", len).magenta();
            println!("{data_len}");
            let encoded_data = encoded_block[..len].to_vec();
            // println!("Block: {:?}", encoded_data);
            sender.send(encoded_data).expect("Failed to send encoded data");
            // println!("Encoded data sent to output channel");
        }
    }
    Ok(())
}


pub fn de_encode_opus(
    receiver: Receiver<Vec<u8>>,
    sender: Sender<Vec<f32>>,
) -> Result<(), opus::Error> {
    let settings: ApplicationSettings = Settings::get_default_settings();
    let channels = settings.get_channels();
    let buffer_size = settings.get_buffer_size();
    let sample_rate = settings.get_sample_rate();
    let opus_channels = if channels == 1 { opus::Channels::Mono } else { opus::Channels::Stereo };

    // println!("Encoder initialized with sample rate: {}, channels: {}", sample_rate, channels);
    // Double buffers for storing audio chunks
    while let Ok(block) = receiver.recv() {
        let mut opus_decoder = Decoder::new(
            sample_rate as u32,
            opus_channels,
            )?;
        // Swap active buffers to avoid blocking
        // println!("Copied new audio block of size {} into inactive buffer", block.len());
        let mut decoded_block = vec![0.0; buffer_size * channels as usize];
        if let Ok(len) = opus_decoder.decode_float(&block, &mut decoded_block, true) {
            let data_len = format!("ENCODER: Decoded block of size: {}", len).magenta();
            println!("{data_len}");
            let encoded_data = decoded_block[..len].to_vec();
            // println!("Block: {:?}", encoded_data);
            sender.send(encoded_data).expect("Failed to send encoded data");
            // println!("Encoded data sent to output channel");
        }
    }
    Ok(())
}

pub fn decode_opus(
    receiver: Receiver<Vec<u8>>,
    sender: Sender<Vec<f32>>,
    ) -> Result<Vec<f32>, opus::Error> {
    // Set settings
    let settings: ApplicationSettings = Settings::get_default_settings();
    let channels = settings.get_channels();
    let buffer_size = settings.get_buffer_size();
    let sample_rate = settings.get_sample_rate();

    debug!("DECODER: Decoding Opus with settings: CH {}, BF {}, SR {}", channels, buffer_size, sample_rate);

    let mut opus_channels: opus::Channels = opus::Channels::Stereo;
    // Convert to Channels Enum
    if channels == 1 {
        opus_channels = opus::Channels::Mono;
    }

    let receiver = Arc::new(Mutex::new(receiver));

    // Initialize Ring Buffer
    let ring = HeapRb::<u8>::new(buffer_size * channels as usize);
    let (mut producer, mut consumer) = ring.split();
    debug!("DECODER: Initialized ring buffer with size: {}", buffer_size * channels as usize);


    let receiver_copy = Arc::clone(&receiver);

    std::thread::spawn( move || {
        info!("DECODER: Started producer thread for decoding Opus");
        let receiver = receiver_copy.lock().unwrap();
        while let Ok(block) = receiver.recv() {
            debug!("DECODER: Received block of size: {}", block.len());
            for sample in block {
                while producer.is_full() {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                producer.try_push(sample).expect("DECODER: Failed to push into producer");
            }
            info!("DECODER: Block successfully pushed to producer");
        }
        info!("DECODER: Receiver channel closed, producer thread exiting");
    });

    let mut decoder = Decoder::new(sample_rate as u32, opus_channels)?;
    info!("DECODER: Opus decoder initialized successfully");

    loop {
        // "Decoded" meaning it hasn't been coded yet
        let mut encoded_block: Vec<u8> = vec![0; buffer_size * channels as usize];
        for sample in encoded_block.iter_mut() {
            while let Some(bit) = consumer.try_pop() {
                *sample = bit;
            }
        }
        debug!("DECODER: Popped {} bytes from ring buffer for decoding", encoded_block.len());

        let mut decoded_block: Vec<f32> = vec![0.0; buffer_size * channels as usize];
        let length = decoder.decode_float(&encoded_block, &mut decoded_block, false)?;
        debug!("DECODER: Decoded block length: {}", length); info!("Decoded block length: {}", length);

        sender.send(decoded_block[..length].to_vec()).unwrap();
        info!("DECODER: Decoded data sent to next stage");
    }
}
