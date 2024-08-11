use std::sync::{Arc, Mutex};
use std::sync::mpsc::{Sender, Receiver};
use opus::{Encoder, Decoder, Application};
use cpal::SampleFormat;
use cpal::traits::{DeviceTrait, StreamTrait};
use ringbuf::{
    traits::{Consumer, Producer, Split, Observer}, 
    HeapRb,
};
use crate::settings::Settings;

pub fn dac(
    receiver: Receiver<Vec<f32>>,
    buffer_size: usize,
    device: &Arc<Mutex<cpal::Device>>,
    ) { 

    let settings = Settings::get_default_settings();
    let channels = settings.get_channels();
    let (_, config) = settings.get_config_files();

    // Receives decoded audio it is not a decoder
    let ring = HeapRb::<f32>::new(buffer_size * channels as usize);
    let (mut producer, mut consumer) = ring.split();

    std::thread::spawn(move || {
        while let Ok(block) = receiver.recv() {
            for sample in block {
                while producer.is_full() {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                producer.try_push(sample).expect("Failed to push into producer");
            }
        }
    });


    let sample_format = config.sample_format();
    let config: cpal::StreamConfig = config.into();

    let device = device.lock().unwrap();

    let stream = match sample_format {
        SampleFormat::F32 => device.build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                for sample in data {
                    *sample = consumer.try_pop().unwrap_or(0.0);
                }
            },
            move |err| {
                // react to errors here.
                eprintln!("Failed to output samples into stream: {}", err);
            },
            None //None=blocking, Some(Duration)=timeout
        ),
        SampleFormat::I16 => {
            println!("Not yet implemented(I16)");
            todo!();
        },
        SampleFormat::U16 => {
            println!("Not yet implemented (U16)");
            todo!();
        }
        sample_format => panic!("Unsupported sample format '{sample_format}'")
    }.unwrap();

    stream.play().expect("Failed to play stream");
    loop {
        std::thread::sleep(std::time::Duration::from_millis(1000));
    }

}
pub fn encode_opus(
    receiver: Receiver<Vec<f32>>,
    sender: Sender<Vec<u8>>,
    ) -> Result<Vec<u8>, opus::Error> {
    println!("Encoder started");

    // Set settings
    let settings = Settings::get_default_settings();
    let channels = settings.get_channels();
    let buffer_size = settings.get_buffer_size();
    let sample_rate = settings.get_sample_rate();
    println!("Settings: CH {}, BF {}, SR {}", channels, buffer_size, sample_rate);

    // Initialize Ring Buffer
    let ring = HeapRb::<f32>::new(buffer_size * channels as usize);
    let (mut producer, mut consumer) = ring.split();
    println!("Ring Buffer initialized with size: {}", buffer_size * channels as usize);

    std::thread::spawn( move || {
        println!("Spawned producer thread");
        while let Ok(block) = receiver.recv() {
            println!("Received a block of size {}", block.len());
            for sample in block {
                while producer.is_full() {
                    println!("Producer is full, waiting to push...");
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                producer.try_push(sample).expect("Failed to push into producer");
            }
            println!("Block successfully pushed to producer");
        }
        println!("Receiver channels closed, producer thread exiting");
    });

    let mut opus_channels: opus::Channels = opus::Channels::Stereo;

    if channels == 1 {
        opus_channels = opus::Channels::Mono;
    }
    println!("Opus encoder channels set to: {:?}", opus_channels);

    // Here the Application can be Voip, Audio, LowDelay
    let mut opus_encoder = Encoder::new(sample_rate as u32, opus_channels, Application::Audio)?;
    println!("Opus encoder initialized");

    loop {
        // Consumer fills encoded_block_buffer, once filled it is returned
        // "Decoded" meaning it hasn't been coded yet
        let mut decoded_block: Vec<f32> = vec![0.0; buffer_size * channels as usize];
        println!("Prepared decoded block with size: {}", decoded_block.len());
        for sample in decoded_block.iter_mut() {
            while let Some(bit) = consumer.try_pop() {
                *sample = bit;
            }
        }
        println!("Filled decoded block buffer");
        
        let mut encoded_block = vec![0; buffer_size * channels as usize];
        let len = opus_encoder.encode_float(&decoded_block, &mut encoded_block)?;
        println!("Encoded block of size {}", len);

        sender.send(encoded_block[..len].to_vec()).unwrap();
        println!("Encoded block sent through sender channel");

        return Ok(encoded_block[..len].to_vec());
    }

}
pub fn decode_opus(
    receiver: Receiver<Vec<u8>>,
    sender: Sender<Vec<f32>>,
    ) -> Result<Vec<f32>, opus::Error> {
    // Set settings
    let settings = Settings::get_default_settings();
    let channels = settings.get_channels();
    let buffer_size = settings.get_buffer_size();
    let sample_rate = settings.get_sample_rate();

    let mut opus_channels: opus::Channels = opus::Channels::Stereo;
    // Convert to Channels Enum
    if channels == 1 {
        opus_channels = opus::Channels::Mono;
    }

    let receiver = Arc::new(Mutex::new(receiver));

    // Initialize Ring Buffer
    let ring = HeapRb::<u8>::new(buffer_size * channels as usize);
    let (mut producer, mut consumer) = ring.split();


    let receiver_copy = Arc::clone(&receiver);

    std::thread::spawn( move || {
        let receiver = receiver_copy.lock().unwrap();
        while let Ok(block) = receiver.recv() {
            for sample in block {
                while producer.is_full() {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                producer.try_push(sample).expect("Failed to push into producer");
            }
        }
    });

    let mut decoder = Decoder::new(sample_rate as u32, opus_channels)?;

    loop {
        // "Decoded" meaning it hasn't been coded yet
        let mut encoded_block: Vec<u8> = vec![0; buffer_size * channels as usize];
        for sample in encoded_block.iter_mut() {
            while let Some(bit) = consumer.try_pop() {
                *sample = bit;
            }
        }
        
        let mut decoded_block: Vec<f32> = vec![0.0; buffer_size * channels as usize];
        let length = decoder.decode_float(&encoded_block, &mut decoded_block, false)?;
        sender.send(decoded_block[..length].to_vec()).unwrap();
        return Ok(decoded_block[..length].to_vec());
    }
}
