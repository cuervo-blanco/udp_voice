use cpal::SampleFormat;
use std::sync::{Arc, Mutex, Condvar};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use selflib::config::*;
use std::f32::consts::PI;
use std::collections::LinkedList;
use ringbuf::{
    traits::{Consumer, Producer, Split, Observer}, 
    HeapRb,
};

// As Chunks are generated they are Stored in a Linked List FIFO
// This is then processed by the CPAL Output Stream one at a time, 
fn main() {
    // Create ring buffer to cycle through frames in the generated blocks

    let host = cpal::default_host();
    let device = host.default_output_device().expect("no output device available");
    
    // Default audio config
    let config = device.default_output_config().unwrap();
    let sample_format = config.sample_format();
    let sample_rate = config.sample_rate().0;

    let ring = HeapRb::<f32>::new(BUFFER_SIZE);
    let (mut producer, mut consumer) = ring.split();

    let chunk_buffer =  Arc::new((Mutex::new(LinkedList::new()), Condvar::new()));
    let chunk_buffer_clone = Arc::clone(&chunk_buffer);
    let buffer_duration: u64 = (1000 / sample_rate as u64) * BUFFER_SIZE as u64;

    std::thread::spawn( move || {
        let mut clock = 0.0;
        loop {
            let block: Vec<f32> = (0..BUFFER_SIZE)
                .map(|_| {
                    let sample = (clock * 2.0  * PI * FREQUENCY / sample_rate as f32).sin();
                    clock = (clock + 1.0) % sample_rate as f32;
                    sample
                })
            .collect();
            
            {
                let (lock, cvar) = &*chunk_buffer_clone;
                let mut chunk = lock.lock().expect("Failed to get chunk");
                chunk.push_back(block);
                cvar.notify_one();
            }

            // Make delay to not overwhelm the memory
            std::thread::sleep(std::time::Duration::from_millis(buffer_duration));
        }
    });

   
    let chunk_buffer_clone = Arc::clone(&chunk_buffer);
    std::thread::spawn( move || {
        // Sleep to ensure enough data is generated
        let (lock, cvar) = &*chunk_buffer_clone;
        loop {
            let buffer = {
                let mut chunk = lock.lock().expect("Failed to get chunk");
                while chunk.is_empty() {
                    chunk = cvar.wait(chunk).unwrap()
                }
                chunk.pop_front()
            };
            println!("2: Iterating through buffer");
            if let Some(block) = buffer {
                for frame in block.iter() {
                    while producer.is_full() {
                         std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    producer.try_push(*frame).expect("Failed to push into producer");
                }
            }
            println!("2: Finished pushing into producer");
            std::thread::sleep(std::time::Duration::from_millis(buffer_duration));
        }
    });

    std::thread::sleep(std::time::Duration::from_millis(1000));
    let config = config.into();

    let stream = match sample_format {
        SampleFormat::F32 => device.build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                for sample in data {
                    *sample = match consumer.try_pop() {
                        Some(s) => s,
                        None => {
                            0.0
                        }
                    }
                }
            },
            move |_err| {
                // react to errors here.
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
