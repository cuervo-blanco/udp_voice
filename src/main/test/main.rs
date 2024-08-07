use selflib::audio::*;
use std::sync::mpsc::channel;

fn main() {

    let (Some(input_device), Some(output_device)) = initialize_audio_interface() else {
        return;
    };

    let output_config = get_audio_config(&output_device)
        .expect("Failed to get audio output config");
    let input_config = get_audio_config(&input_device)
        .expect("Failed to get audio output config");

    let (sender, receiver) = channel();

    match start_input_stream(&input_device, &input_config, sender) {
        Ok(_) => {
            println!("Input stream started successfully.");
        }
        Err(e) => {
            eprintln!("Failed to start input stream: {:?}", e);
        }
    }

    match start_output_stream(&output_device, &output_config, receiver){
        Ok(_) => {
            println!("Playing audio...");
        },
        Err(e) => eprintln!("Error starting stream: {:?}", e),
    }


    loop {
        // Prevent the main thread from exiting
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
