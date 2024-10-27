use cpal::Device;
use cpal::traits::{DeviceTrait, HostTrait};

pub trait Settings {
    fn get_default_settings() -> Self;
}

pub struct ApplicationSettings {
    // System Config
    #[allow(dead_code)]
    host: cpal::Host,
    devices: (cpal::Device, cpal::Device),
    config_files: (cpal::SupportedStreamConfig, cpal::SupportedStreamConfig),
    sample_rate: cpal::SampleRate,
    channels: cpal::ChannelCount,
    buffer_size: usize,
}

impl Settings for ApplicationSettings {
    fn get_default_settings() -> Self {
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
         let default_buffer_size = 960;
         let buffer_size = match output_config.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } => {
                if default_buffer_size >= *min as usize && default_buffer_size <= *max as usize {
                    default_buffer_size
                } else {
                    *max as usize
                }
             }
            cpal::SupportedBufferSize::Unknown => default_buffer_size,
        };


        let settings = Self {
            host,
            devices: (input_device, output_device),
            config_files: (
                input_config,
                output_config,
                ),
            sample_rate: cpal::SampleRate(48000),
            channels,
            buffer_size,
        };

        settings
    }
}

impl ApplicationSettings {

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
    pub fn create_stream_config(&self) -> cpal::StreamConfig {
        cpal::StreamConfig {
            channels: self.channels,
            sample_rate: self.sample_rate,
            buffer_size: cpal::BufferSize::Fixed(self.buffer_size as u32),
        }
    }

    // Set Functions
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

pub struct TestToneSettings {
    frequency: f32,
    amplitude: f32
}

impl Settings for TestToneSettings {
    fn get_default_settings() -> Self {
        let test_tone = Self {
            amplitude: 1.0,
            frequency: 400.0,
        };
        test_tone
    }
}

impl TestToneSettings {
    pub fn get_amplitude(&self) -> f32 {
        self.amplitude
    }
    pub fn set_amplitude(mut self, quantity: f32) {
        self.amplitude = quantity;
    }
    pub fn set_frequency(mut self, frequency: f32) {
        self.frequency = frequency;

    }
}
