use std::{
    env,
    error::Error,
    fs, io,
    path::{Path, PathBuf},
};

const PREFERENCES_FILE_NAME: &str = ".udp_voice_preferences";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppPreferences {
    pub username: Option<String>,
    pub input_device_name: Option<String>,
    pub output_device_name: Option<String>,
    pub network_interface_name: Option<String>,
    pub bind_port: Option<u16>,
}

impl AppPreferences {
    pub fn load() -> Result<Self, Box<dyn Error>> {
        let path = preferences_file_path()?;
        Self::load_from_path(&path)
    }

    pub fn load_from_path(path: &Path) -> Result<Self, Box<dyn Error>> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = fs::read_to_string(path)?;
        Self::parse(&contents)
    }

    pub fn save(&self) -> Result<PathBuf, Box<dyn Error>> {
        let path = preferences_file_path()?;
        fs::write(&path, self.serialize())?;
        Ok(path)
    }

    fn parse(contents: &str) -> Result<Self, Box<dyn Error>> {
        let mut preferences = Self::default();

        for raw_line in contents.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let (key, value) = line.split_once('=').ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Invalid preferences line: {line}"),
                )
            })?;

            let value = value.trim();
            let normalized = if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            };

            match key.trim() {
                "username" => preferences.username = normalized,
                "input_device" => preferences.input_device_name = normalized,
                "output_device" => preferences.output_device_name = normalized,
                "network_interface" => preferences.network_interface_name = normalized,
                "bind_port" => {
                    preferences.bind_port = match normalized {
                        Some(port) => Some(port.parse()?),
                        None => None,
                    };
                }
                unknown => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("Unknown preferences key: {unknown}"),
                    )
                    .into())
                }
            }
        }

        Ok(preferences)
    }

    fn serialize(&self) -> String {
        [
            format!("username={}", self.username.as_deref().unwrap_or("")),
            format!(
                "input_device={}",
                self.input_device_name.as_deref().unwrap_or("")
            ),
            format!(
                "output_device={}",
                self.output_device_name.as_deref().unwrap_or("")
            ),
            format!(
                "network_interface={}",
                self.network_interface_name.as_deref().unwrap_or("")
            ),
            format!(
                "bind_port={}",
                self.bind_port
                    .map(|port| port.to_string())
                    .unwrap_or_default()
            ),
        ]
        .join("\n")
            + "\n"
    }
}

pub fn preferences_file_path() -> Result<PathBuf, io::Error> {
    Ok(env::current_dir()?.join(PREFERENCES_FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_round_trip_preserves_values() {
        let preferences = AppPreferences {
            username: Some("alice".to_string()),
            input_device_name: Some("USB Mic".to_string()),
            output_device_name: Some("Speakers".to_string()),
            network_interface_name: Some("en0".to_string()),
            bind_port: Some(18_521),
        };

        let serialized = preferences.serialize();
        let parsed = AppPreferences::parse(&serialized).unwrap();
        assert_eq!(parsed, preferences);
    }

    #[test]
    fn empty_values_parse_as_missing() {
        let parsed = AppPreferences::parse(
            "username=\ninput_device=\noutput_device=\nnetwork_interface=\nbind_port=\n",
        )
        .unwrap();

        assert_eq!(parsed, AppPreferences::default());
    }
}
