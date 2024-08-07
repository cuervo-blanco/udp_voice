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

    let mut sample_clock = 0f32;

    thread::spawn(move || { 
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

    let sine_wave = receiver.recv().unwrap();

    match start_output_stream(&output_device, &output_config, sine_wave.clone()){
        Ok(_) => {
            println!("Playing audio...");
            println!("Sound Received: {:?}", sine_wave);
            clear_terminal();
        },
        Err(e) => eprintln!("Error starting stream: {:?}", e),
    }


    loop {
        // Prevent the main thread from exiting
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
