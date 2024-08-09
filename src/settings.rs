use cpal::traits::{DeviceTrait, HostTrait};
use std::sync::LazyLock;

pub static HOST: LazyLock<cpal::Host> = LazyLock::new(|| cpal::default_host());
pub static DEVICE: LazyLock<cpal::Device> = LazyLock::new(|| HOST.default_output_device().expect("no output device available"));
pub static CONFIG: LazyLock<cpal::SupportedStreamConfig> = LazyLock::new(|| DEVICE.default_output_config().unwrap());
pub static SAMPLE_RATE: LazyLock<cpal::SampleRate> = LazyLock::new(|| CONFIG.sample_rate());
pub static CHANNELS: LazyLock<cpal::ChannelCount> = LazyLock::new(|| CONFIG.channels());
pub const BUFFER_SIZE: usize = 960; 
//TEST TONE
pub const FREQUENCY: f32 = 440.0;
pub const AMPLITUDE: f32 = 0.5;
