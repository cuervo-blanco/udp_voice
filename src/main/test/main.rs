use selflib::config::{BUFFER_SIZE, FREQUENCY, SAMPLE_RATE, AMPLITUDE};
use std::f32::consts::PI;
use selflib::audio::*;
use std::thread;
use std::time::Duration;
use std::sync::mpsc::{self, Sender};

fn generate_sine_wave(tx: Sender<Vec<f32>>) {
    let mut sample_clock = 0.0;

    loop {
        let sine_wave: Vec<f32> = (0..BUFFER_SIZE)
            .map(|_| {
                let value = (sample_clock * FREQUENCY * 2.0 * PI / SAMPLE_RATE).sin() * AMPLITUDE;
                sample_clock = (sample_clock + 1.0) % SAMPLE_RATE;
                value
            })
            .collect();

        if tx.send(sine_wave).is_err() {
            break;
        }

        thread::sleep(Duration::from_millis(10)); // Adjust as needed
    }
}
fn main() {
    let (tx, rx) = mpsc::channel();

    let (Some(_input_device), Some(output_device)) = initialize_audio_interface() else {
        return;
    };

    let output_config = get_audio_config(&output_device)
        .expect("Failed to get audio output config");

    thread::spawn(move || generate_sine_wave(tx));


    std::thread::sleep(std::time::Duration::from_millis(100));
    match start_output_stream(&output_device, &output_config, rx.try_recv().unwrap()){
        Ok(_) => {
            println!("Playing audio...");
            println!("Sound Received: {:?}", rx);
        },
        Err(e) => eprintln!("Error starting stream: {:?}", e),
    }

    loop {
        // Prevent the main thread from exiting
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
