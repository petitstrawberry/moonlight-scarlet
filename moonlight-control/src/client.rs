//! Sunshine/GameStream HTTP client and pairing state machine.

use std::io;
use std::path::Path;
use std::time::Duration;

use reqwest::Url;
use reqwest::blocking::Client;
use thiserror::Error;

use crate::Application;
use crate::crypto::{
    ClientIdentity, aes_decrypt, aes_encrypt_zero_padded, certificate_signature,
    default_identity_directory, hash_for_server, random_bytes, verify_signature,
};
use crate::xml::{parse_applications, parse_server_info, require_paired, response_hex};

const DEFAULT_HTTP_PORT: u16 = 47_989;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const PAIRING_TIMEOUT: Duration = Duration::from_secs(120);

/// Error returned by the portable control plane.
#[derive(Debug, Error)]
pub enum ControlError {
    /// The user-provided host address is invalid.
    #[error("invalid host: {0}")]
    InvalidHost(String),
    /// An HTTP or TLS request failed.
    #[error("network request failed: {0}")]
    Http(#[from] reqwest::Error),
    /// A local identity file could not be read or written.
    #[error("identity storage failed: {0}")]
    Io(#[from] io::Error),
    /// Client identity generation or parsing failed.
    #[error("client identity failed: {0}")]
    Identity(String),
    /// Persistent client configuration was malformed or unsupported.
    #[error("client configuration failed: {0}")]
    Configuration(String),
    /// XML returned by the host was malformed or incomplete.
    #[error("invalid host response: {0}")]
    Xml(String),
    /// The host returned an explicit GameStream protocol error.
    #[error("host rejected request ({code}): {message}")]
    Protocol {
        /// GameStream XML status code.
        code: u32,
        /// GameStream XML status message.
        message: String,
    },
    /// A cryptographic operation failed.
    #[error("pairing cryptography failed: {0}")]
    Crypto(String),
    /// The pairing handshake was rejected or failed validation.
    #[error("pairing failed: {0}")]
    Pairing(String),
    /// An application session could not be launched, resumed, or cancelled.
    #[error("session control failed: {0}")]
    Session(String),
}

/// Parsed manual host endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    host: String,
    http_port: u16,
}

impl Endpoint {
    /// Parse a hostname, IPv4 address, or HTTP URL.
    ///
    /// # Arguments
    ///
    /// * `input` - Host text entered by the user. Port 47989 is used when omitted.
    ///
    /// # Returns
    ///
    /// A normalized endpoint suitable for GameStream requests.
    pub fn parse(input: &str) -> Result<Self, ControlError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(ControlError::InvalidHost("host is empty".to_owned()));
        }
        let candidate = if input.contains("://") {
            input.to_owned()
        } else {
            format!("http://{input}")
        };
        let url =
            Url::parse(&candidate).map_err(|error| ControlError::InvalidHost(error.to_string()))?;
        let host = url
            .host_str()
            .ok_or_else(|| ControlError::InvalidHost("host name is missing".to_owned()))?;
        if url.username() != "" || url.password().is_some() || url.query().is_some() {
            return Err(ControlError::InvalidHost(
                "credentials and query strings are not supported".to_owned(),
            ));
        }
        if url.path() != "/" && !url.path().is_empty() {
            return Err(ControlError::InvalidHost(
                "a host address must not contain a path".to_owned(),
            ));
        }
        Ok(Self {
            host: host.to_owned(),
            http_port: url.port().unwrap_or(DEFAULT_HTTP_PORT),
        })
    }

    /// Return the normalized host name or IP address.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Return the HTTP control port.
    pub fn http_port(&self) -> u16 {
        self.http_port
    }

    fn url(
        &self,
        scheme: &str,
        port: u16,
        command: &str,
        unique_id: &str,
        arguments: &[(&str, String)],
    ) -> Result<Url, ControlError> {
        let authority = if self.host.contains(':') {
            format!("[{host}]", host = self.host)
        } else {
            self.host.clone()
        };
        let mut url = Url::parse(&format!("{scheme}://{authority}:{port}/{command}"))
            .map_err(|error| ControlError::InvalidHost(error.to_string()))?;
        let request_uuid = hex::encode(random_bytes::<16>());
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("uniqueid", unique_id);
            query.append_pair("uuid", &request_uuid);
            for (name, value) in arguments {
                query.append_pair(name, value);
            }
        }
        Ok(url)
    }
}

/// Host capabilities returned by the `serverinfo` endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerInfo {
    /// User-visible Sunshine host name.
    pub hostname: String,
    /// Stable identifier advertised by the host.
    pub unique_id: String,
    /// GameStream protocol version string.
    pub app_version: String,
    /// GeForce Experience compatibility version, when advertised.
    pub gfe_version: Option<String>,
    /// Codec modes advertised through `ServerCodecModeSupport`.
    ///
    /// Legacy hosts that omit the field are treated as H.264-only.
    pub server_codec_mode_support: u32,
    /// HTTPS control port.
    pub https_port: u16,
    /// Whether the current client certificate is paired.
    pub paired: bool,
    /// Currently running application ID, or zero.
    pub current_game: u32,
    /// Raw host state string.
    pub state: String,
}

impl ServerInfo {
    /// Parse the major GameStream generation from `app_version`.
    pub fn server_major_version(&self) -> Result<u32, ControlError> {
        self.app_version
            .split('.')
            .next()
            .and_then(|component| component.parse().ok())
            .ok_or_else(|| ControlError::Xml(format!("invalid appversion: {}", self.app_version)))
    }
}

/// Progress emitted while connecting, pairing, and loading applications.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectProgress {
    /// Fetching the public HTTP `serverinfo` document.
    FetchingServerInfo,
    /// Waiting for the user to enter this PIN on the Sunshine host.
    WaitingForPin(String),
    /// Executing one of the five pairing handshake stages.
    PairingStage(u8),
    /// Fetching the authenticated application list.
    FetchingApplications,
}

/// Authenticated control-plane result ready for application selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectedHost {
    /// Parsed endpoint used for this connection.
    pub endpoint: Endpoint,
    /// Latest server information.
    pub server: ServerInfo,
    /// Applications advertised by Sunshine.
    pub applications: Vec<Application>,
    /// True when this call completed a new pairing handshake.
    pub newly_paired: bool,
}

/// Portable synchronous GameStream control client.
///
/// Call blocking methods from a worker thread when integrating with a UI.
pub struct GameStreamClient {
    identity: ClientIdentity,
}

impl GameStreamClient {
    /// Load or create the default persistent client identity.
    pub fn load_default() -> Result<Self, ControlError> {
        let directory = default_identity_directory()?;
        Self::load_or_create_identity(directory)
    }

    /// Load or create a persistent client identity under `directory`.
    ///
    /// # Arguments
    ///
    /// * `directory` - Directory containing the client certificate, key, and unique ID.
    pub fn load_or_create_identity(directory: impl AsRef<Path>) -> Result<Self, ControlError> {
        Self::from_identity(ClientIdentity::load_or_create(directory.as_ref())?)
    }

    /// Create an ephemeral identity, primarily for isolated tests.
    pub fn ephemeral() -> Result<Self, ControlError> {
        Self::from_identity(ClientIdentity::generate()?)
    }

    fn from_identity(identity: ClientIdentity) -> Result<Self, ControlError> {
        // Validate the generated certificate and key before a pairing request
        // can be left waiting on the host.
        let _ = identity.reqwest_identity()?;
        Ok(Self { identity })
    }

    /// Query public server information without pairing.
    pub fn server_info(&self, address: &str) -> Result<(Endpoint, ServerInfo), ControlError> {
        let endpoint = Endpoint::parse(address)?;
        let xml = self.get(
            &endpoint,
            "http",
            endpoint.http_port,
            "serverinfo",
            &[],
            REQUEST_TIMEOUT,
        )?;
        Ok((endpoint, parse_server_info(&xml)?))
    }

    /// Connect to a host, pair when required, and return its application list.
    ///
    /// # Arguments
    ///
    /// * `address` - Manual hostname, IP address, or HTTP URL.
    /// * `progress` - Callback invoked synchronously as connection state advances.
    pub fn connect<F>(&self, address: &str, mut progress: F) -> Result<ConnectedHost, ControlError>
    where
        F: FnMut(ConnectProgress),
    {
        progress(ConnectProgress::FetchingServerInfo);
        let (endpoint, mut server) = self.server_info(address)?;

        if let Ok(authenticated) = self.authenticated_server_info(&endpoint, server.https_port)
            && authenticated.paired
        {
            server = authenticated;
            progress(ConnectProgress::FetchingApplications);
            let applications = self.applications(&endpoint, server.https_port)?;
            return Ok(ConnectedHost {
                endpoint,
                server,
                applications,
                newly_paired: false,
            });
        }

        if server.current_game != 0 {
            return Err(ControlError::Pairing(
                "Sunshine is currently streaming; stop that session before pairing".to_owned(),
            ));
        }

        let pin = format!("{:04}", u16::from_be_bytes(random_bytes::<2>()) % 10_000);
        progress(ConnectProgress::WaitingForPin(pin.clone()));
        self.pair(&endpoint, &server, &pin, &mut progress)?;

        progress(ConnectProgress::FetchingApplications);
        server = self.authenticated_server_info(&endpoint, server.https_port)?;
        if !server.paired {
            return Err(ControlError::Pairing(
                "host did not report the client as paired".to_owned(),
            ));
        }
        let applications = self.applications(&endpoint, server.https_port)?;
        Ok(ConnectedHost {
            endpoint,
            server,
            applications,
            newly_paired: true,
        })
    }

    pub(crate) fn authenticated_server_info(
        &self,
        endpoint: &Endpoint,
        https_port: u16,
    ) -> Result<ServerInfo, ControlError> {
        let xml = self.get(
            endpoint,
            "https",
            https_port,
            "serverinfo",
            &[],
            REQUEST_TIMEOUT,
        )?;
        parse_server_info(&xml)
    }

    fn applications(
        &self,
        endpoint: &Endpoint,
        https_port: u16,
    ) -> Result<Vec<Application>, ControlError> {
        let xml = self.get(
            endpoint,
            "https",
            https_port,
            "applist",
            &[],
            REQUEST_TIMEOUT,
        )?;
        parse_applications(&xml)
    }

    fn pair<F>(
        &self,
        endpoint: &Endpoint,
        server: &ServerInfo,
        pin: &str,
        progress: &mut F,
    ) -> Result<(), ControlError>
    where
        F: FnMut(ConnectProgress),
    {
        let result = self.pair_inner(endpoint, server, pin, progress);
        if result.is_err() {
            let _ = self.get(
                endpoint,
                "http",
                endpoint.http_port,
                "unpair",
                &[],
                REQUEST_TIMEOUT,
            );
        }
        result
    }

    fn pair_inner<F>(
        &self,
        endpoint: &Endpoint,
        server: &ServerInfo,
        pin: &str,
        progress: &mut F,
    ) -> Result<(), ControlError>
    where
        F: FnMut(ConnectProgress),
    {
        let server_major_version = server.server_major_version()?;
        let hash_length = if server_major_version >= 7 { 32 } else { 20 };
        let salt = random_bytes::<16>();
        let mut salted_pin = salt.to_vec();
        salted_pin.extend_from_slice(pin.as_bytes());
        let digest = hash_for_server(&salted_pin, server_major_version);
        let aes_key: [u8; 16] = digest[..16]
            .try_into()
            .map_err(|_| ControlError::Crypto("pairing digest is too short".to_owned()))?;

        progress(ConnectProgress::PairingStage(1));
        let stage_one = self.get(
            endpoint,
            "http",
            endpoint.http_port,
            "pair",
            &[
                ("devicename", "roth".to_owned()),
                ("updateState", "1".to_owned()),
                ("phrase", "getservercert".to_owned()),
                ("salt", hex::encode(salt)),
                (
                    "clientcert",
                    hex::encode(self.identity.certificate_pem().as_bytes()),
                ),
            ],
            PAIRING_TIMEOUT,
        )?;
        require_paired(&stage_one, "stage 1")?;
        let server_certificate = response_hex(&stage_one, "plaincert")?;
        if server_certificate.is_empty() {
            return Err(ControlError::Pairing(
                "another pairing session is already active".to_owned(),
            ));
        }

        progress(ConnectProgress::PairingStage(2));
        let random_challenge = random_bytes::<16>();
        let encrypted_challenge = aes_encrypt_zero_padded(&random_challenge, &aes_key)?;
        let stage_two = self.get(
            endpoint,
            "http",
            endpoint.http_port,
            "pair",
            &[
                ("devicename", "roth".to_owned()),
                ("updateState", "1".to_owned()),
                ("clientchallenge", hex::encode(encrypted_challenge)),
            ],
            REQUEST_TIMEOUT,
        )?;
        require_paired(&stage_two, "stage 2")?;
        let encrypted_response = response_hex(&stage_two, "challengeresponse")?;
        let decrypted_response = aes_decrypt(&encrypted_response, &aes_key)?;
        if decrypted_response.len() < hash_length + 16 {
            return Err(ControlError::Pairing(
                "stage 2 challenge response is too short".to_owned(),
            ));
        }
        let server_response = &decrypted_response[..hash_length];
        let server_challenge = &decrypted_response[hash_length..hash_length + 16];

        progress(ConnectProgress::PairingStage(3));
        let client_secret = random_bytes::<16>();
        let mut challenge_hash_input = Vec::new();
        challenge_hash_input.extend_from_slice(server_challenge);
        challenge_hash_input.extend_from_slice(self.identity.certificate_signature());
        challenge_hash_input.extend_from_slice(&client_secret);
        let mut challenge_hash = hash_for_server(&challenge_hash_input, server_major_version);
        challenge_hash.resize(32, 0);
        let encrypted_challenge_hash = aes_encrypt_zero_padded(&challenge_hash, &aes_key)?;
        let stage_three = self.get(
            endpoint,
            "http",
            endpoint.http_port,
            "pair",
            &[
                ("devicename", "roth".to_owned()),
                ("updateState", "1".to_owned()),
                ("serverchallengeresp", hex::encode(encrypted_challenge_hash)),
            ],
            REQUEST_TIMEOUT,
        )?;
        require_paired(&stage_three, "stage 3")?;
        let pairing_secret = response_hex(&stage_three, "pairingsecret")?;
        if pairing_secret.len() <= 16 {
            return Err(ControlError::Pairing(
                "stage 3 pairing secret is too short".to_owned(),
            ));
        }
        let (server_secret, server_signature) = pairing_secret.split_at(16);
        if let Err(error) = verify_signature(&server_certificate, server_secret, server_signature) {
            return Err(ControlError::Pairing(error.to_string()));
        }

        let mut expected_response_input = Vec::new();
        expected_response_input.extend_from_slice(&random_challenge);
        expected_response_input.extend_from_slice(&certificate_signature(&server_certificate)?);
        expected_response_input.extend_from_slice(server_secret);
        let expected_response = hash_for_server(&expected_response_input, server_major_version);
        if server_response != &expected_response[..hash_length] {
            return Err(ControlError::Pairing("incorrect PIN".to_owned()));
        }

        progress(ConnectProgress::PairingStage(4));
        let mut client_pairing_secret = client_secret.to_vec();
        client_pairing_secret.extend_from_slice(&self.identity.sign(&client_secret));
        let stage_four = self.get(
            endpoint,
            "http",
            endpoint.http_port,
            "pair",
            &[
                ("devicename", "roth".to_owned()),
                ("updateState", "1".to_owned()),
                ("clientpairingsecret", hex::encode(client_pairing_secret)),
            ],
            REQUEST_TIMEOUT,
        )?;
        require_paired(&stage_four, "stage 4")?;

        progress(ConnectProgress::PairingStage(5));
        let stage_five = self.get(
            endpoint,
            "https",
            server.https_port,
            "pair",
            &[
                ("devicename", "roth".to_owned()),
                ("updateState", "1".to_owned()),
                ("phrase", "pairchallenge".to_owned()),
            ],
            REQUEST_TIMEOUT,
        )?;
        require_paired(&stage_five, "stage 5")
    }

    pub(crate) fn get(
        &self,
        endpoint: &Endpoint,
        scheme: &str,
        port: u16,
        command: &str,
        arguments: &[(&str, String)],
        timeout: Duration,
    ) -> Result<String, ControlError> {
        let url = endpoint.url(scheme, port, command, self.identity.unique_id(), arguments)?;
        // GameStream hosts are sensitive to persistent HTTP connections and
        // resumed mutual-TLS sessions. Official Moonlight clients disable both,
        // so give each request a fresh rustls session cache.
        let http = Client::builder()
            .identity(self.identity.reqwest_identity()?)
            .danger_accept_invalid_certs(true)
            .http1_only()
            .pool_max_idle_per_host(0)
            .no_proxy()
            .timeout(PAIRING_TIMEOUT)
            .build()?;
        Ok(http
            .get(url)
            .timeout(timeout)
            .send()?
            .error_for_status()?
            .text()?)
    }
}

#[cfg(test)]
mod tests {
    use super::{Endpoint, GameStreamClient};

    #[test]
    fn endpoint_defaults_to_gamestream_http_port() {
        let endpoint = Endpoint::parse("192.168.0.100").expect("endpoint");
        assert_eq!(endpoint.host(), "192.168.0.100");
        assert_eq!(endpoint.http_port(), 47_989);
    }

    #[test]
    fn endpoint_preserves_manual_port() {
        let endpoint = Endpoint::parse("http://sunshine.local:48000").expect("endpoint");
        assert_eq!(endpoint.host(), "sunshine.local");
        assert_eq!(endpoint.http_port(), 48_000);
    }

    #[test]
    fn creates_ephemeral_http_client() {
        GameStreamClient::ephemeral().expect("ephemeral client");
    }
}
