// ============================================
//                  Scope/Imports
// ============================================
use cpal::platform::Host;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use opus::{Encoder, Decoder, Application};
use opus::Channels;
use std::sync::mpsc::{self, Sender, Receiver};

const SAMPLE_RATE: u32 = 48000;
const CHANNELS: usize = 2;
const FRAME_SIZE: usize = 960;

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
    let config = match device.default_output_config() {
        Ok(cnfg) => cnfg,
        Err(e) => {
            println!("AUDIO SYNC I: Unable to get default config: {}", e);

            // Try to find a supported configuration
            if let Ok(mut supported_configs) = device.supported_output_configs() {
                if let Some(supported_config) = supported_configs.next() {
                    println!("Using a supported configuration instead.");
                    return Ok(cpal::StreamConfig {
                        channels: supported_config.channels(),
                        sample_rate: supported_config.min_sample_rate(),
                        buffer_size: cpal::BufferSize::Fixed(256),
                    });
                }
            }
            
            let config =  cpal::StreamConfig {
                channels: 2,
                sample_rate: cpal::SampleRate(44100),
                buffer_size: cpal::BufferSize::Fixed(256),
            };
            return Ok(config);
        }
    };

    let config =  cpal::StreamConfig {
        channels: config.channels(),
        sample_rate: config.sample_rate(),
        buffer_size: cpal::BufferSize::Fixed(256),
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
    config: &cpal::StreamConfig
    ) -> Result<(cpal::Stream, Receiver<Vec<u8>>), cpal::BuildStreamError> {
    // Start the audio input/output stream

    let (sender, receiver): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = mpsc::channel();

    let audio_buffer = Arc::new(Mutex::new(Vec::new()));

    let buffer_clone = Arc::clone(&audio_buffer);
    let timeout: Duration = Duration::from_secs(5);

    let stream = input_device.build_input_stream(
            &config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                let mut buffer = buffer_clone.lock()
                    .expect("AUDIO SYNC I: Failed to lock buffer"); 
                buffer.extend_from_slice(data);
                // Process the buffer if it has enough data for one frame
                while buffer.len() >= FRAME_SIZE * CHANNELS {
                    let frame: Vec<f32> = buffer.drain(0..FRAME_SIZE * CHANNELS).collect();
                    match convert_audio_stream_to_opus(&frame) {
                        Ok(opus_data) => {
                            if let Err(e) = sender.send(opus_data) {
                                eprintln!("AUDIO SYNC I: Failed to send Opus data: {}", e);
                            }
                        }
                        Err(e) => println!("AUDIO SYNC I: Failed to encode Opus data: {}", e)
                    }
                }
            },
            |err| println!("AUDIO SYNC I: An error occured on the input audio stream: {}", err),
                Some(timeout)
                );

    match stream {
        Ok(s) => {
            if let Err(err) = s.play() {
                println!("AUDIO SYNC I: Failed to start input stream: {}", err);
            }
            Ok((s, receiver))
        }
        Err(e) => {
            println!("AUDIO SYNC I: Failed to build input stream: {}", e);
            Err(e)
        }
    }

}
// ============================================
//        Start Output Stream
// ============================================
pub fn start_output_stream(output_device: &cpal::Device, config: &cpal::StreamConfig,
    received_data: Arc<Mutex<Vec<u8>>>) -> Result<cpal::Stream, cpal::BuildStreamError> {
    // Start the audio input/output stream
    let output_buffer_clone = Arc::clone(&received_data);

    let stream = output_device.build_output_stream(
        &config,
        move |output_data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            let mut buffer = output_buffer_clone.lock().unwrap();
            if !buffer.is_empty() {
                    match decode_opus_to_pcm(&buffer) {
                        Ok(pcm_data) => {
                            for (i, sample) in output_data.iter_mut().enumerate() {
                                if i < pcm_data.len() {
                                    *sample = pcm_data[i];
                                } else {
                                    *sample = 0.0;
                                }
                            }
                            buffer.clear();
                        }
                        Err(err) =>
                            println!("AUDIO SYNC I: Opus decoding error: {}", err),
                    }
                }
            },
        |err| println!("AUDIO SYNC I: An error occured on the output audio stream: {}", err),
        None
    );

    match stream {
        Ok(s) => {
            if let Err(err) = s.play() {
                println!("AUDIO SYNC I: Failed to start output stream: {}", err);
            }
            Ok(s)
        }
        Err(e) => {
            println!("AUDIO SYNC I: Failed to build output stream: {}", e);
            Err(e)
        }
    }
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
    let mut opus_encoder = Encoder::new(SAMPLE_RATE, Channels::Stereo, Application::Audio)?;
    let mut encoded_data = vec![0; 4000];
    let len = opus_encoder.encode_float(input_stream, &mut encoded_data)?;
    Ok(encoded_data[..len].to_vec())
}
// ============================================
//    Decode Opus to PCM Format
// ============================================
// Decode an audio stream  from Oputs format to PCM format
pub fn decode_opus_to_pcm(opus_data: &[u8]) -> Result<Vec<f32>, opus::Error> {
    let mut decoder = Decoder::new(SAMPLE_RATE, Channels::Stereo)?;
    let mut pcm_data = vec![0.0; opus_data.len() * 1];
    // FEC (Forward Error Correction) set to false
    let decoded_samples = decoder.decode_float(opus_data, &mut pcm_data, false)?;
    pcm_data.truncate(decoded_samples * 1);
    Ok(pcm_data)
}

