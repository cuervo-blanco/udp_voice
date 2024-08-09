use cpal::traits::{DeviceTrait, HostTrait};
use selflib::sine::Sine;
use selflib::settings::*;
use std::sync::mpsc::channel;

fn main () {

    let (sender, receiver) = channel();
    let sine = Sine::new(220.0, 1.0, SAMPLE_RATE.0, CHANNELS as usize, sender, BUFFER_SIZE );
    sine.play(receiver, BUFFER_SIZE, DEVICE, CONFIG);

    std::thread::sleep(std::time::Duration::from_millis(3000));

    std::process::exit(0);
}
