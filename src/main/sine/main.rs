use cpal::SampleFormat;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use selflib::config::*;
use std::env;
use std::f32::consts::PI;
use ringbuf::{
    traits::{Consumer, Producer, Split, Observer}, 
    HeapRb,
};

fn main() {

    let args: Vec<String> = env::args().collect();
    let frequency = &args[1];
    let frequency: f32 = frequency.parse().unwrap();

    let host = cpal::default_host();
    let device = host.default_output_device().expect("no output device available");
    
    // Default audio config
    let config = device.default_output_config().unwrap();
    let sample_format = config.sample_format();
    let sample_rate = config.sample_rate().0;
    let channels = config.channels();

    let ring = HeapRb::<f32>::new(BUFFER_SIZE * channels as usize);
    let (mut producer, mut consumer) = ring.split();

    let _buffer_duration: u64 = (1000 / sample_rate as u64) * BUFFER_SIZE as u64;

    let _producer_thread = std::thread::spawn( move || {
        let mut phase = 0.0 as f32;
        let phase_increment = 2.0 * PI * frequency / sample_rate as f32;
        loop {
            let block: Vec<f32> = (0..BUFFER_SIZE)
                .flat_map(|_| {
                    let sample = (phase).sin();
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
                producer.try_push(sample).expect("Failed to push into producer");
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
        std::thread::sleep(std::time::Duration::from_millis(2000));
    }

}
