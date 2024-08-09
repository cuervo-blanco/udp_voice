use cpal::traits::{DeviceTrait, HostTrait};
use selflib::sine::Sine;
use selflib::settings::*;
use std::sync::mpsc::channel;

fn main () {

    let host = cpal::default_host();
    let device = host.default_output_device().expect("no output device available");
    let config = device.default_output_config().unwrap();
    let sample_rate = config.sample_rate();

    let (sender, receiver) = channel();
    let sine = Sine::new(220.0, 1.0, sample_rate.0, CHANNELS as usize, sender, BUFFER_SIZE );
    sine.play(receiver, BUFFER_SIZE, device, config);

    std::thread::sleep(std::time::Duration::from_millis(1000));
}
