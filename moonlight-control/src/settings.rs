//! Persistent client input preferences, independent of host pairing.

use std::{fs, io::ErrorKind, path::Path};

use serde::{Deserialize, Serialize};

use crate::{ControlError, crypto::default_identity_directory};

/// Client preferences. Missing fields retain the original input behavior.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct ClientSettings {
    /// Exchange the gamepad A and B buttons sent to the host.
    pub swap_ab: bool,
}

impl ClientSettings {
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
        serde_json::from_slice(&bytes)
            .map_err(|error| ControlError::Configuration(error.to_string()))
    }

    fn save_to(&self, directory: impl AsRef<Path>) -> Result<(), ControlError> {
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
            let settings = ClientSettings { swap_ab };
            settings.save_to(directory.path()).unwrap();
            assert_eq!(
                ClientSettings::load_from(directory.path()).unwrap(),
                settings
            );
        }
    }
}
