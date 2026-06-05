use cpal::traits::{DeviceTrait, HostTrait};
use cpal::{
    ChannelCount, Device, SampleFormat, SampleRate, SupportedStreamConfig,
    SupportedStreamConfigRange,
};
use std::{error::Error, io};

pub const OPUS_SAMPLE_RATE: u32 = 48_000;
pub const NETWORK_CHANNELS: u16 = 1;
pub const OPUS_FRAME_SIZE: usize = 960;
pub const CLIENT_DISCOVERY_PORT: u16 = 18_522;
pub const SERVER_AUDIO_PORT: u16 = 18_521;
pub const OPUS_BITRATE_BPS: i32 = 32_000;
pub const JITTER_BUFFER_PACKETS: usize = 3;
pub const PLAYBACK_BUFFER_PACKETS: usize = 6;
pub const MAX_OPUS_PACKET_SIZE: usize = 1_275;

pub trait Settings {
    fn get_default_settings() -> Result<Self, Box<dyn Error>>
    where
        Self: Sized;
}

#[derive(Clone, Debug, Default)]
pub struct AudioDeviceSelection {
    pub input_device_name: Option<String>,
    pub output_device_name: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct AudioDeviceInventory {
    pub input_devices: Vec<String>,
    pub output_devices: Vec<String>,
    pub default_input_device: Option<String>,
    pub default_output_device: Option<String>,
}

pub struct ApplicationSettings {
    devices: (Device, Device),
    config_files: (SupportedStreamConfig, SupportedStreamConfig),
    device_names: (String, String),
}

impl Settings for ApplicationSettings {
    fn get_default_settings() -> Result<Self, Box<dyn Error>> {
        Self::from_device_selection(AudioDeviceSelection::default())
    }
}

impl ApplicationSettings {
    pub fn from_device_selection(selection: AudioDeviceSelection) -> Result<Self, Box<dyn Error>> {
        let host = cpal::default_host();
        let inventory = Self::device_inventory_for_host(&host)?;

        let (input_device, input_name) =
            select_input_device(&host, selection.input_device_name.as_deref(), &inventory)?;
        let (output_device, output_name) =
            select_output_device(&host, selection.output_device_name.as_deref(), &inventory)?;

        let input_config = select_input_config(
            &input_device,
            &input_name,
            selection.input_device_name.is_some(),
        )?;
        let output_config = select_output_config(
            &output_device,
            &output_name,
            selection.output_device_name.is_some(),
        )?;

        Ok(Self {
            devices: (input_device, output_device),
            config_files: (input_config, output_config),
            device_names: (input_name, output_name),
        })
    }

    pub fn device_inventory() -> Result<AudioDeviceInventory, Box<dyn Error>> {
        let host = cpal::default_host();
        Self::device_inventory_for_host(&host)
    }

    pub fn device_inventory_for_host(
        host: &cpal::Host,
    ) -> Result<AudioDeviceInventory, Box<dyn Error>> {
        let mut input_devices = Vec::new();
        let mut output_devices = Vec::new();
        let default_input_device = host.default_input_device().and_then(|device| {
            if supports_input(&device) {
                device.name().ok()
            } else {
                None
            }
        });
        let default_output_device = host.default_output_device().and_then(|device| {
            if supports_output(&device) {
                device.name().ok()
            } else {
                None
            }
        });

        for device in host.devices().map_err(|error| {
            io::Error::other(format!("Unable to enumerate audio devices: {error}"))
        })? {
            let device_name = device
                .name()
                .unwrap_or_else(|_| "<unavailable device name>".to_string());

            if supports_input(&device) {
                input_devices.push(device_name.clone());
            }

            if supports_output(&device) {
                output_devices.push(device_name);
            }
        }

        input_devices.sort();
        input_devices.dedup();
        output_devices.sort();
        output_devices.dedup();

        Ok(AudioDeviceInventory {
            input_devices,
            output_devices,
            default_input_device,
            default_output_device,
        })
    }

    pub fn input_device_name(&self) -> &str {
        &self.device_names.0
    }

    pub fn output_device_name(&self) -> &str {
        &self.device_names.1
    }

    pub fn get_buffer_size(&self) -> usize {
        OPUS_FRAME_SIZE
    }

    pub fn get_sample_rate(&self) -> f32 {
        OPUS_SAMPLE_RATE as f32
    }

    pub fn get_devices(&self) -> (Device, Device) {
        self.devices.clone()
    }

    pub fn input_device(&self) -> Device {
        self.devices.0.clone()
    }

    pub fn output_device(&self) -> Device {
        self.devices.1.clone()
    }

    pub fn get_channels(&self) -> ChannelCount {
        self.config_files.1.channels()
    }

    pub fn input_channels(&self) -> ChannelCount {
        self.config_files.0.channels()
    }

    pub fn network_channels(&self) -> u16 {
        NETWORK_CHANNELS
    }

    pub fn get_config_files(&self) -> (SupportedStreamConfig, SupportedStreamConfig) {
        self.config_files.clone()
    }

    pub fn input_config(&self) -> SupportedStreamConfig {
        self.config_files.0.clone()
    }

    pub fn output_config(&self) -> SupportedStreamConfig {
        self.config_files.1.clone()
    }

    pub fn create_stream_config(&self) -> cpal::StreamConfig {
        self.output_config().into()
    }
}

fn supports_input(device: &Device) -> bool {
    device
        .supported_input_configs()
        .map(|mut configs| configs.next().is_some())
        .unwrap_or(false)
}

fn supports_output(device: &Device) -> bool {
    device
        .supported_output_configs()
        .map(|mut configs| configs.next().is_some())
        .unwrap_or(false)
}

fn select_input_device(
    host: &cpal::Host,
    requested_name: Option<&str>,
    inventory: &AudioDeviceInventory,
) -> Result<(Device, String), Box<dyn Error>> {
    if let Some(requested_name) = requested_name {
        return select_named_device(host, requested_name, true, inventory);
    }

    let device = host.default_input_device().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            missing_device_message("input", inventory),
        )
    })?;
    let name = device
        .name()
        .unwrap_or_else(|_| "<default input device>".to_string());

    Ok((device, name))
}

fn select_output_device(
    host: &cpal::Host,
    requested_name: Option<&str>,
    inventory: &AudioDeviceInventory,
) -> Result<(Device, String), Box<dyn Error>> {
    if let Some(requested_name) = requested_name {
        return select_named_device(host, requested_name, false, inventory);
    }

    let device = host.default_output_device().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            missing_device_message("output", inventory),
        )
    })?;
    let name = device
        .name()
        .unwrap_or_else(|_| "<default output device>".to_string());

    Ok((device, name))
}

fn select_named_device(
    host: &cpal::Host,
    requested_name: &str,
    is_input: bool,
    inventory: &AudioDeviceInventory,
) -> Result<(Device, String), Box<dyn Error>> {
    for device in host
        .devices()
        .map_err(|error| io::Error::other(format!("Unable to enumerate audio devices: {error}")))?
    {
        let device_name = match device.name() {
            Ok(name) => name,
            Err(_) => continue,
        };

        if !device_name.eq_ignore_ascii_case(requested_name) {
            continue;
        }

        let supports_direction = if is_input {
            supports_input(&device)
        } else {
            supports_output(&device)
        };

        if supports_direction {
            return Ok((device, device_name));
        }

        let direction = if is_input { "input" } else { "output" };
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "Audio device \"{device_name}\" does not expose a usable {direction} stream at the moment.{}",
                permission_hint(is_input)
            ),
        )
        .into());
    }

    let direction = if is_input { "input" } else { "output" };
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!(
            "Could not find {direction} device \"{requested_name}\". Available {direction} devices: {}",
            join_device_names(if is_input {
                &inventory.input_devices
            } else {
                &inventory.output_devices
            })
        ),
    )
    .into())
}

fn select_input_config(
    device: &Device,
    device_name: &str,
    is_named_selection: bool,
) -> Result<SupportedStreamConfig, Box<dyn Error>> {
    let default_config = device.default_input_config().ok();
    let supported_configs = device.supported_input_configs().map_err(|error| {
        io::Error::other(format!(
            "{}{}{}",
            stream_query_error("input", device_name, error),
            if is_named_selection {
                String::new()
            } else {
                " Try `--list-devices` and then `--input-device <name>` if the default device is wrong.".to_string()
            },
            permission_hint(true),
        ))
    })?;

    select_supported_config(default_config, supported_configs, "input", device_name)
}

fn select_output_config(
    device: &Device,
    device_name: &str,
    is_named_selection: bool,
) -> Result<SupportedStreamConfig, Box<dyn Error>> {
    let default_config = device.default_output_config().ok();
    let supported_configs = device.supported_output_configs().map_err(|error| {
        io::Error::other(format!(
            "{}{}",
            stream_query_error("output", device_name, error),
            if is_named_selection {
                String::new()
            } else {
                " Try `--list-devices` and then `--output-device <name>` if the default device is wrong.".to_string()
            },
        ))
    })?;

    select_supported_config(default_config, supported_configs, "output", device_name)
}

fn select_supported_config(
    default_config: Option<SupportedStreamConfig>,
    supported_configs: impl Iterator<Item = SupportedStreamConfigRange>,
    kind: &str,
    device_name: &str,
) -> Result<SupportedStreamConfig, Box<dyn Error>> {
    if let Some(config) = default_config.filter(|config| {
        config.sample_rate().0 == OPUS_SAMPLE_RATE
            && is_supported_sample_format(config.sample_format())
    }) {
        return Ok(config);
    }

    supported_configs
        .filter_map(|config| config.try_with_sample_rate(SampleRate(OPUS_SAMPLE_RATE)))
        .filter(|config| is_supported_sample_format(config.sample_format()))
        .max_by_key(|config| (sample_format_rank(config.sample_format()), config.channels()))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                format!(
                    "Audio device \"{device_name}\" has no {kind} stream config that supports {} Hz with a PCM sample format handled by this app.",
                    OPUS_SAMPLE_RATE
                ),
            )
            .into()
        })
}

fn is_supported_sample_format(sample_format: SampleFormat) -> bool {
    matches!(
        sample_format,
        SampleFormat::F32 | SampleFormat::I16 | SampleFormat::U16
    )
}

fn sample_format_rank(sample_format: SampleFormat) -> u8 {
    match sample_format {
        SampleFormat::F32 => 3,
        SampleFormat::I16 => 2,
        SampleFormat::U16 => 1,
        _ => 0,
    }
}

fn stream_query_error(
    kind: &str,
    device_name: &str,
    error: cpal::SupportedStreamConfigsError,
) -> String {
    format!("Unable to query {kind} configs for audio device \"{device_name}\": {error}.")
}

fn missing_device_message(kind: &str, inventory: &AudioDeviceInventory) -> String {
    let device_names = if kind == "input" {
        &inventory.input_devices
    } else {
        &inventory.output_devices
    };

    format!(
        "No default {kind} device available. Available {kind} devices: {}",
        join_device_names(device_names)
    )
}

fn join_device_names(devices: &[String]) -> String {
    if devices.is_empty() {
        "none".to_string()
    } else {
        devices.join(", ")
    }
}

fn permission_hint(is_input: bool) -> String {
    #[cfg(target_os = "macos")]
    {
        if is_input {
            " On macOS, also check System Settings > Privacy & Security > Microphone and allow the terminal or app that launches this binary.".to_string()
        } else {
            String::new()
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = is_input;
        String::new()
    }
}

pub struct TestToneSettings {
    frequency: f32,
    amplitude: f32,
}

impl Settings for TestToneSettings {
    fn get_default_settings() -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            amplitude: 1.0,
            frequency: 400.0,
        })
    }
}

impl TestToneSettings {
    pub fn get_amplitude(&self) -> f32 {
        self.amplitude
    }

    pub fn get_frequency(&self) -> f32 {
        self.frequency
    }

    pub fn set_amplitude(mut self, quantity: f32) {
        self.amplitude = quantity;
    }

    pub fn set_frequency(mut self, frequency: f32) {
        self.frequency = frequency;
    }
}
