use cpal::Device;
use cpal::traits::{DeviceTrait, HostTrait};


// Let's objectify this, so that instead of it being global variables is
// an object that returns these computations
//

pub struct Settings {
    // System Config
    #[allow(dead_code)]
    host: cpal::Host,
    devices: (cpal::Device, cpal::Device),
    config_files: (cpal::SupportedStreamConfig, cpal::SupportedStreamConfig),
    sample_rate: cpal::SampleRate,
    channels: cpal::ChannelCount,
    buffer_size: usize,
    // Test Tone
    frequency: f32,
    amplitude: f32
}

impl Settings {
    pub fn get_default_settings() -> Self {

        let host = cpal::default_host();
        let (input_device, output_device) = (
            host.default_input_device().unwrap(), 
            host.default_output_device().unwrap(),
            );

        let (input_config, output_config) = (
            input_device.default_input_config()
            .expect("Failed to get output_config"),
            output_device.default_output_config()
            .expect("Failed to get output_config")
            );
        let _sample_rate = output_config
            .sample_rate();
        let channels = output_config
            .channels();


        let settings = Self {
            host,
            devices: (input_device, output_device),
            config_files: (
                input_config, 
                output_config,
                ),
            sample_rate: cpal::SampleRate(48000),
            channels,
            buffer_size: 960 as usize,
            //test-tone
            frequency: 256.0,
            amplitude: 1.0,

        };

        settings
    }

    // Get functions: 
    pub fn get_buffer_size(&self) -> usize {
        self.buffer_size
    }
    pub fn get_sample_rate(&self)-> f32 {
        self.sample_rate.0 as f32
    }
    pub fn get_devices(&self) -> (Device, Device) {
        self.devices.clone()
    }
    pub fn get_channels(&self) -> cpal::ChannelCount {
        self.channels
    }
    pub fn get_config_files(&self) -> (
        cpal::SupportedStreamConfig, 
        cpal::SupportedStreamConfig
        ) {
        self.config_files.clone()
    }
    pub fn get_amplitude(&self) -> f32 {
        self.amplitude
    }
        

    // Set Functions
    pub fn set_amplitude(mut self, quantity: f32) {
        self.amplitude = quantity;
    }
    pub fn set_frequency(mut self, frequency: f32) {
        self.frequency = frequency;

    }
    pub fn set_output_device(self) {
        todo!()
    }
    pub fn set_input_device(self) {
        todo!()
    }
    pub fn set_buffer_size(mut self, buffer_size: usize){
       self.buffer_size = buffer_size;
    }

}
