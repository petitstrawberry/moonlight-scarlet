//! Persistent Sunshine host history.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::client::{ControlError, Endpoint};
use crate::crypto::default_identity_directory;

const HOSTS_FILE: &str = "hosts.json";
const HOSTS_FORMAT_VERSION: u32 = 1;

/// Saved Sunshine endpoints and the most recently connected endpoint.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SavedHosts {
    hosts: Vec<String>,
    last_connected: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct SavedHostsDocument {
    version: u32,
    #[serde(default)]
    hosts: Vec<String>,
    #[serde(default)]
    last_connected: Option<String>,
}

impl SavedHosts {
    /// Load the saved host history from the platform-default configuration directory.
    ///
    /// # Returns
    ///
    /// The saved history, or an empty history when the file does not yet exist.
    pub fn load_default() -> Result<Self, ControlError> {
        Self::load_from(default_identity_directory()?)
    }

    /// Load the saved host history from a configuration directory.
    ///
    /// # Arguments
    ///
    /// * `directory` - Directory containing `hosts.json`.
    ///
    /// # Returns
    ///
    /// The saved history, or an empty history when the file does not yet exist.
    pub fn load_from(directory: impl AsRef<Path>) -> Result<Self, ControlError> {
        let path = directory.as_ref().join(HOSTS_FILE);
        if !path.try_exists()? {
            return Ok(Self::default());
        }
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error.into()),
        };
        let document: SavedHostsDocument = serde_json::from_slice(&bytes)
            .map_err(|error| ControlError::Configuration(error.to_string()))?;
        if document.version != HOSTS_FORMAT_VERSION {
            return Err(ControlError::Configuration(format!(
                "unsupported hosts.json version {}",
                document.version
            )));
        }

        let mut saved = Self::default();
        for host in document.hosts {
            saved.remember_without_selecting(&host)?;
        }
        if let Some(last_connected) = document.last_connected {
            saved.remember(&last_connected)?;
        }
        Ok(saved)
    }

    /// Save the host history to the platform-default configuration directory.
    ///
    /// # Returns
    ///
    /// Success after `hosts.json` has been written.
    pub fn save_default(&self) -> Result<(), ControlError> {
        self.save_to(default_identity_directory()?)
    }

    /// Save the host history under a configuration directory.
    ///
    /// # Arguments
    ///
    /// * `directory` - Directory that will contain `hosts.json`.
    ///
    /// # Returns
    ///
    /// Success after the directory and file have been written.
    pub fn save_to(&self, directory: impl AsRef<Path>) -> Result<(), ControlError> {
        let directory = directory.as_ref();
        fs::create_dir_all(directory)?;
        let document = SavedHostsDocument {
            version: HOSTS_FORMAT_VERSION,
            hosts: self.hosts.clone(),
            last_connected: self.last_connected.clone(),
        };
        let mut bytes = serde_json::to_vec_pretty(&document)
            .map_err(|error| ControlError::Configuration(error.to_string()))?;
        bytes.push(b'\n');
        fs::write(directory.join(HOSTS_FILE), bytes)?;
        Ok(())
    }

    /// Remember a successfully connected endpoint and select it as the latest host.
    ///
    /// # Arguments
    ///
    /// * `host` - Hostname, address, or control endpoint accepted by [`Endpoint`].
    ///
    /// # Returns
    ///
    /// Success after validation and in-memory insertion.
    pub fn remember(&mut self, host: &str) -> Result<(), ControlError> {
        let host = normalized_host(host)?;
        self.remember_without_selecting(&host)?;
        self.last_connected = Some(host);
        Ok(())
    }

    /// Return all remembered endpoints in insertion order.
    ///
    /// # Returns
    ///
    /// Saved endpoint strings suitable for reconnecting.
    pub fn hosts(&self) -> &[String] {
        &self.hosts
    }

    /// Return the endpoint used by the most recent successful connection.
    ///
    /// # Returns
    ///
    /// The endpoint string, or `None` before any connection has been saved.
    pub fn last_connected(&self) -> Option<&str> {
        self.last_connected.as_deref()
    }

    fn remember_without_selecting(&mut self, host: &str) -> Result<(), ControlError> {
        let host = normalized_host(host)?;
        if !self.hosts.iter().any(|saved| saved == &host) {
            self.hosts.push(host);
        }
        Ok(())
    }
}

fn normalized_host(host: &str) -> Result<String, ControlError> {
    let host = host.trim();
    Endpoint::parse(host)?;
    Ok(host.to_owned())
}

#[cfg(test)]
mod tests {
    use super::SavedHosts;

    #[test]
    fn missing_host_file_loads_as_empty_history() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let saved = SavedHosts::load_from(directory.path()).expect("load missing host history");

        assert!(saved.hosts().is_empty());
        assert_eq!(saved.last_connected(), None);
    }

    #[test]
    fn host_history_round_trips_and_deduplicates() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let mut saved = SavedHosts::default();
        saved.remember("192.168.0.10").expect("remember first host");
        saved
            .remember("sunshine.local:47989")
            .expect("remember second host");
        saved.remember("192.168.0.10").expect("select first host");
        saved.save_to(directory.path()).expect("save host history");

        let loaded = SavedHosts::load_from(directory.path()).expect("reload host history");
        assert_eq!(
            loaded.hosts(),
            &[
                String::from("192.168.0.10"),
                String::from("sunshine.local:47989")
            ]
        );
        assert_eq!(loaded.last_connected(), Some("192.168.0.10"));
    }

    #[test]
    fn invalid_saved_host_is_rejected() {
        let mut saved = SavedHosts::default();
        assert!(saved.remember("http://host/path").is_err());
    }
}
