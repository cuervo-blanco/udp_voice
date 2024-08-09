use selflib::sine::Sine;
use selflib::settings::Settings;
use std::sync::mpsc::channel;


fn main () {

    let settings = Settings::get_default_settings();
    let channels = settings.get_channels();
    let buffer_size = settings.get_buffer_size();
    let sample_rate = settings.get_sample_rate();
    let (_input_device, output_device) = settings.get_devices();
    let (_input_config, output_config) = settings.get_config_files();

    let (sender, receiver) = channel();
    let sine = Sine::new(220.0, 1.0, sample_rate as u32, channels as usize, sender, buffer_size );
    sine.play(receiver, buffer_size, output_device, output_config);

    std::thread::sleep(std::time::Duration::from_millis(3000));

    std::process::exit(0);
}
