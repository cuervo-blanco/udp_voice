use std::sync::{Arc, Mutex};
use selflib::config::{BUFFER_SIZE, FREQUENCY, SAMPLE_RATE, AMPLITUDE};
use std::f32::consts::PI;
use selflib::utils::clear_terminal;
use selflib::audio::*;
use std::thread;
use std::sync::mpsc::channel;

fn main() {

    let (Some(_input_device), Some(output_device)) = initialize_audio_interface() else {
        return;
    };

    let output_config = get_audio_config(&output_device)
        .expect("Failed to get audio output config");

    let (sender, receiver) = channel();
    let pcm_data = Arc::new(Mutex::new(Vec::new()));
    let data_index = Arc::new(Mutex::new(0));


    let pcm_data_clone = Arc::clone(&pcm_data);
    let data_index_clone = Arc::clone(&data_index);

    thread::spawn(move || { 
        let mut sample_clock = 0f32;
        loop {
            let sine_wave: Vec<f32> = (0..BUFFER_SIZE)
            .map(|_| {
                let value = (sample_clock * FREQUENCY * 2.0 * PI / SAMPLE_RATE).sin() * AMPLITUDE;
                sample_clock = (sample_clock + 1.0) % SAMPLE_RATE;
                value
            }).collect();

            println!("Sine Wave being generated: {:?}", sine_wave);
            
            sender.send(sine_wave).unwrap();
        }
    });

    thread::spawn(move || {
        while let Ok(sine_wave) = receiver.recv() {
            let mut buffer = pcm_data_clone.lock().unwrap();
            *buffer = sine_wave;
            let mut index = data_index_clone.lock().unwrap();
            *index = 0;
        }
    });


    match start_output_stream(&output_device, &output_config, Arc::clone(&pcm_data), Arc::clone(&data_index)){
        Ok(_) => {
            println!("Playing audio...");
            clear_terminal();
        },
        Err(e) => eprintln!("Error starting stream: {:?}", e),
    }


    loop {
        // Prevent the main thread from exiting
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
