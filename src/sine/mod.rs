use std::sync::mpsc::{Sender, Receiver};
use cpal::SampleFormat;
use cpal::traits::{DeviceTrait, StreamTrait};
use std::f32::consts::PI;
use ringbuf::{
    traits::{Consumer, Producer, Split, Observer}, 
    HeapRb,
};

pub struct Sine {
    frequency: f32,
    amplitude: f32,
    sample_rate: u32,
    channels: usize,
}
impl Sine {

    pub fn new (
        frequency: f32, 
        amplitude: f32, 
        sample_rate: u32,
        channels: usize,
        sender: Sender<Vec<f32>>,
        buffer_size: usize,
        ) -> Self {

        let sine = Self {
            frequency,
            amplitude,
            sample_rate,
            channels
        };

        std::thread::spawn( move || {
            let mut phase = 0.0 as f32;
            let phase_increment = 2.0 * PI * sine.frequency / sine.sample_rate as f32;
            loop {
                let block: Vec<f32> = (0..buffer_size)
                    .flat_map(|_| {
                        let sample = (phase).sin() * sine.amplitude;
                        phase += phase_increment;
                        if phase > 2.0 * PI {
                            phase -= 2.0 * PI;
                        }
                        std::iter::repeat(sample).take(channels as usize)

                    })
                .collect();
                if sender.send(block).is_err(){
                    break;
                }
            }
        });
        sine
    }

    #[allow(dead_code)]
    pub fn play(
        self, 
        receiver: Receiver<Vec<f32>>,
        buffer_size: usize,
        device: cpal::Device,
        config: cpal::SupportedStreamConfig,
        ) { 

        let ring = HeapRb::<f32>::new(buffer_size * self.channels);
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
}

