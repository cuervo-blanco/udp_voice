// ============================================
//                  Scope/Imports
// ============================================
use cpal::platform::Host;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};
use opus::{Encoder, Decoder, Application};
use opus::Channels;
use std::thread;
use crate::config::{CHANNELS, SAMPLE_RATE, BUFFER_SIZE};

pub type FormattedAudio = Result<Vec<u8>, opus::Error>;

// ============================================
//       Initialize Audio Interface
// Open communication with the default audio
// interface.
// ============================================

pub fn initialize_audio_interface() -> (Option<cpal::Device>, Option<cpal::Device>) {

    // Get the default host
    let host: Host = cpal::default_host();
    println!("AUDIO SYNC I: Initializing System...");

    // Get the default input device
    let input_device: Option<cpal::Device> = host.default_input_device();
    println!("AUDIO SYNC I: Default Input Device Procured");
    match &input_device {
        Some(device) => {
            match device.name() {
                Ok(name) => println!("AUDIO SYNC I: Default input device: {}", name),
                Err(err) => println!("AUDIO SYNC I: Failed to get input device name: {}", err),
            }
        },
        None => println!("AUDIO SYNC I: Default input device found"),
    }

    // Get the default output device
    let output_device: Option<cpal::Device> = host.default_output_device();
    match &output_device {
        Some(device) => {
            match device.name() {
                Ok(name) => println!("AUDIO SYNC I: Default output device: {}", name),
                Err(err) => println!("AUDIO SYNC I: Failed to get output device name: {}", err),
            }
        },
        None => println!("AUDIO SYNC I: Default input device found"),
    }

    (input_device, output_device)
}
// ============================================
//            Get Audio Config
// ============================================
pub fn get_audio_config(device: &cpal::Device) -> Result<cpal::StreamConfig, cpal::DefaultStreamConfigError> {
    let _config = match device.default_output_config() {
        Ok(cnfg) => cnfg,
        Err(e) => {
            println!("AUDIO SYNC I: Unable to get default config: {}", e);

            // Try to find a supported configuration
            let config =  cpal::StreamConfig {
                channels: CHANNELS as u16,
                sample_rate: cpal::SampleRate(SAMPLE_RATE as u32),
                buffer_size: cpal::BufferSize::Fixed(BUFFER_SIZE.try_into().unwrap()),
            };
            return Ok(config);
        }
    };

    let config =  cpal::StreamConfig {
        channels: CHANNELS as u16,
        sample_rate: cpal::SampleRate(SAMPLE_RATE as u32),
        buffer_size: cpal::BufferSize::Fixed(BUFFER_SIZE.try_into().unwrap()),
    };

    Ok(config)
}
pub fn list_supported_configs(device: &cpal::Device) {
    let supported_configs_range = device.supported_output_configs().unwrap();
    println!("Suported output configurations:");
    for config in supported_configs_range {
        println!("Channels: {}, Min Sample Rate: {}, Max Sample Rate: {}",
            config.channels(),
            config.min_sample_rate().0,
            config.max_sample_rate().0);
    }
}
// ============================================
//        Start Input Stream
// ============================================
pub fn start_input_stream(
    input_device: &cpal::Device, 
    config: &cpal::StreamConfig,
    sender: std::sync::mpsc::Sender<Vec<f32>>
    ) -> Result<cpal::Stream, cpal::BuildStreamError> {

    let stream = input_device.build_input_stream(
        config,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            sender.send(data.to_vec()).unwrap();
        },
        |err| {
            eprintln!("An error occurred on the input audio stream: {}", err);
        },
        None
    )?;
    stream.play().expect("Failed to play input stream");
    Ok(stream)

}
// ============================================
//        Start Output Stream
// ============================================
pub fn start_output_stream(output_device: &cpal::Device, config: &cpal::StreamConfig,
    receiver: std::sync::mpsc::Receiver<Vec<f32>>) -> Result<cpal::Stream, cpal::BuildStreamError> {
    println!("DEBUG: Starting to build output stream...");
    
    let pcm_data = Arc::new(Mutex::new(Vec::new()));
    let pcm_data_clone = Arc::clone(&pcm_data);

    let stream = output_device.build_output_stream(
        &config,
        move |output_data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            let mut buffer = pcm_data_clone.lock().unwrap();
            if buffer.len() >= output_data.len() {
                output_data.copy_from_slice(&buffer[..output_data.len()]);
                buffer.drain(..output_data.len());
            } else {
                output_data.fill(0.0);
            }
        },
        |err| println!("An error occurred on the output audio stream: {}", err),
        None
    )?;

    stream.play().expect("Failed to play stream");

    thread::spawn(move || {
        while let Ok(data) = receiver.recv() {
            let mut buffer = pcm_data.lock().unwrap();
            buffer.extend(data);
        }
    });

    Ok(stream)
}
// ============================================
//        Stop Audio Stream
// ============================================
// Stop the audio stream.
pub fn stop_audio_stream(stream: cpal::Stream) {
    match stream.pause() {
        Ok(_) => {
            // Dropping the stream to release resources
            // Stream will be dropped automatically when it goes out of scope
        }
        Err(e) => {
            println!("AUDIO SYNC I: Unable to pause audio stream: {}", e);
        }
    }
    // Explicitly dropping the stream after attempting to pause it
    drop(stream);
}
// ============================================
//    Convert PCM to Opus Format
// ============================================
// Convert audio stream from PCM format to Opus format
pub fn convert_audio_stream_to_opus(input_stream: &[f32]) -> Result<Vec<u8>, opus::Error> {
    let mut channels = Channels::Mono;
    if CHANNELS == 2.0 {
        channels = Channels::Stereo;
    }

    let mut opus_encoder = Encoder::new(SAMPLE_RATE as u32, channels, Application::Audio)?;
    let mut encoded_data = vec![0; BUFFER_SIZE * CHANNELS as usize];
    let len = opus_encoder.encode_float(input_stream, &mut encoded_data)?;
    Ok(encoded_data[..len].to_vec())
}
// ============================================
//    Decode Opus to PCM Format
// ============================================
// Decode an audio stream  from Oputs format to PCM format
pub fn decode_opus_to_pcm(opus_data: &[u8]) -> Result<Vec<f32>, opus::Error> {
    let mut decoder = Decoder::new(SAMPLE_RATE as u32, Channels::Mono)?;
    let mut pcm_data = vec![0.0; BUFFER_SIZE * CHANNELS as usize];
    // FEC (Forward Error Correction) set to false
    let decoded_samples = decoder.decode_float(opus_data, &mut pcm_data, false)?;
    pcm_data.truncate(decoded_samples * CHANNELS as usize);
    Ok(pcm_data)
}

pub fn play_test_tone() -> Vec<u8> {
    let frequency = 440.0;
    let duration = 2.0; //seconds
    let amplitude = 0.5;
    let num_samples = (SAMPLE_RATE as f64 * duration) as usize;

    let mut samples = Vec::with_capacity(num_samples);
    for i in 0..num_samples {
        let sample = (i as f64 * frequency * 2.0 * std::f64::consts::PI / SAMPLE_RATE as f64).sin() * amplitude;
        samples.push(sample as f32);
    }

    bincode::serialize(&samples).unwrap()

}
