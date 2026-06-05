use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::SampleFormat;
use ringbuf::{
    traits::{Consumer, Observer, Producer, Split},
    HeapRb,
};
use selflib::settings::{ApplicationSettings, Settings, TestToneSettings};
use std::env;
use std::error::Error;
use std::f32::consts::PI;

pub fn main() -> Result<(), Box<dyn Error>> {
    let settings = ApplicationSettings::get_default_settings()?;
    let tone_settings = TestToneSettings::get_default_settings()?;
    let buffer_size = settings.get_buffer_size();
    let amplitude = tone_settings.get_amplitude();
    // Command Line Arguments
    let args: Vec<String> = env::args().collect();
    let frequency = &args[1];
    let frequency: f32 = frequency.parse().unwrap();
    let duration = &args[2];
    let duration: u64 = duration.parse().unwrap();

    let (_input_device, device) = settings.get_devices();
    let (_input_config, config) = settings.get_config_files();
    let sample_format = config.sample_format();
    let sample_rate = config.sample_rate().0;
    let channels = config.channels();

    let ring = HeapRb::<f32>::new(buffer_size * channels as usize);
    let (mut producer, mut consumer) = ring.split();

    let _buffer_duration: u64 = (1000 / sample_rate as u64) * buffer_size as u64;

    let _producer_thread = std::thread::spawn(move || {
        let mut phase = 0.0 as f32;
        let phase_increment = 2.0 * PI * frequency / sample_rate as f32;
        // Sine Equation:
        // sine = sin(phase * 2.0 * PI * Frecuencia / Sample Rate)
        loop {
            let block: Vec<f32> = (0..buffer_size)
                .flat_map(|_| {
                    let sample = (phase).sin() * amplitude;
                    phase += phase_increment;
                    if phase > 2.0 * PI {
                        phase -= 2.0 * PI;
                    }
                    std::iter::repeat(sample).take(channels as usize)
                })
                .collect();

            for sample in block {
                while producer.is_full() {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                producer
                    .try_push(sample)
                    .expect("Failed to push into producer");
            }
        }
    });

    // We can send the consumer through a channel mpsc and use
    // to send it as input for the network Client

    std::thread::sleep(std::time::Duration::from_millis(1000));
    let config = config.into();

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
            None, //None=blocking, Some(Duration)=timeout
        ),
        SampleFormat::I16 => {
            println!("Not yet implemented(I16)");
            todo!();
        }
        SampleFormat::U16 => {
            println!("Not yet implemented (U16)");
            todo!();
        }
        sample_format => panic!("Unsupported sample format '{sample_format}'"),
    }
    .unwrap();

    stream.play().expect("Failed to play stream");

    std::thread::sleep(std::time::Duration::from_millis(duration));

    Ok(())
}
