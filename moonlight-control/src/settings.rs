//! Persistent client preferences, independent of host pairing.

use std::{fs, io::ErrorKind, path::Path};

use serde::{Deserialize, Serialize};

use crate::{ControlError, crypto::default_identity_directory};

/// Client preferences. Missing fields retain the original input behavior.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct ClientSettings {
    /// Exchange the gamepad A and B buttons sent to the host.
    pub swap_ab: bool,
    /// Requested video bitrate in Mbps, applied on the next stream connection.
    pub bitrate_mbps: u32,
}

impl Default for ClientSettings {
    fn default() -> Self {
        Self {
            swap_ab: false,
            bitrate_mbps: 20,
        }
    }
}

impl ClientSettings {
    pub const MIN_BITRATE_MBPS: u32 = 1;
    pub const MAX_BITRATE_MBPS: u32 = 100;

    fn validate(&self) -> Result<(), ControlError> {
        if !(Self::MIN_BITRATE_MBPS..=Self::MAX_BITRATE_MBPS).contains(&self.bitrate_mbps) {
            return Err(ControlError::Configuration(
                "video bitrate must be between 1 and 100 Mbps".into(),
            ));
        }
        Ok(())
    }

    /// Load preferences from the platform configuration directory.
    pub fn load_default() -> Result<Self, ControlError> {
        Self::load_from(default_identity_directory()?)
    }

    /// Save preferences in the platform configuration directory.
    pub fn save_default(&self) -> Result<(), ControlError> {
        self.save_to(default_identity_directory()?)
    }

    fn load_from(directory: impl AsRef<Path>) -> Result<Self, ControlError> {
        let path = directory.as_ref().join("settings.json");
        if !path.try_exists()? {
            return Ok(Self::default());
        }
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error.into()),
        };
        let settings: Self = serde_json::from_slice(&bytes)
            .map_err(|error| ControlError::Configuration(error.to_string()))?;
        settings.validate()?;
        Ok(settings)
    }

    fn save_to(&self, directory: impl AsRef<Path>) -> Result<(), ControlError> {
        self.validate()?;
        let directory = directory.as_ref();
        fs::create_dir_all(directory)?;
        let mut bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| ControlError::Configuration(error.to_string()))?;
        bytes.push(b'\n');
        fs::write(directory.join("settings.json"), bytes)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_preserve_positions_and_preference_survives_reload() {
        let directory = tempfile::tempdir().unwrap();
        assert!(!ClientSettings::load_from(directory.path()).unwrap().swap_ab);
        fs::write(directory.path().join("settings.json"), b"{}").unwrap();
        assert!(!ClientSettings::load_from(directory.path()).unwrap().swap_ab);
        for swap_ab in [true, false] {
            let settings = ClientSettings {
                swap_ab,
                bitrate_mbps: 10,
            };
            settings.save_to(directory.path()).unwrap();
            assert_eq!(
                ClientSettings::load_from(directory.path()).unwrap(),
                settings
            );
        }
    }

    #[test]
    fn old_settings_keep_the_original_bitrate_and_invalid_rates_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        fs::write(&path, br#"{"swap_ab":true}"#).unwrap();
        assert_eq!(
            ClientSettings::load_from(directory.path()).unwrap(),
            ClientSettings {
                swap_ab: true,
                bitrate_mbps: 20
            }
        );
        for bitrate_mbps in [0, 101, u32::MAX] {
            let settings = ClientSettings {
                bitrate_mbps,
                ..ClientSettings::default()
            };
            assert!(settings.save_to(directory.path()).is_err());
            fs::write(&path, serde_json::to_vec(&settings).unwrap()).unwrap();
            assert!(ClientSettings::load_from(directory.path()).is_err());
        }
    }
}
