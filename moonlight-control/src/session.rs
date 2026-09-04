//! GameStream application launch, resume, and cancellation.

use std::time::Duration;

use crate::Application;
use crate::client::{ConnectedHost, ControlError, GameStreamClient, ServerInfo};
use crate::crypto::random_bytes;
use crate::xml::response_text;

const LAUNCH_TIMEOUT: Duration = Duration::from_secs(120);
const RESUME_TIMEOUT: Duration = Duration::from_secs(30);
const CANCEL_TIMEOUT: Duration = Duration::from_secs(30);
const STEREO_SURROUND_AUDIO_INFO: u32 = (0x3 << 16) | 2;
const MOONLIGHT_CORE_VERSION: u32 = 1;

/// Video, audio, and controller preferences sent while preparing a stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaunchConfig {
    /// Requested video width in pixels.
    pub width: u32,
    /// Requested video height in pixels.
    pub height: u32,
    /// Requested video frame rate.
    pub fps: u32,
    /// Allow Sunshine to adjust the host display for the requested mode.
    pub optimize_game_settings: bool,
    /// Continue playing audio on the Sunshine host.
    pub play_audio_on_host: bool,
    /// Bitmap of controllers currently attached to the client.
    pub gamepad_mask: u32,
    /// Keep virtual controllers attached after the stream disconnects.
    pub persist_gamepads: bool,
}

impl Default for LaunchConfig {
    fn default() -> Self {
        Self {
            width: 1_920,
            height: 1_080,
            fps: 60,
            optimize_game_settings: true,
            play_audio_on_host: false,
            gamepad_mask: 0,
            persist_gamepads: false,
        }
    }
}

impl LaunchConfig {
    fn validate(self) -> Result<(), ControlError> {
        if self.width == 0 || self.height == 0 || self.fps == 0 {
            return Err(ControlError::Session(
                "stream width, height, and FPS must be non-zero".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Prepared GameStream session ready to pass to `moonlight-common-c`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamSession {
    /// Endpoint used for the control connection.
    pub endpoint: crate::Endpoint,
    /// Fresh host state used to prepare this session.
    pub server: ServerInfo,
    /// Application that was launched or resumed.
    pub application: Application,
    /// Stream preferences sent to the host.
    pub config: LaunchConfig,
    /// RTSP URL returned by Sunshine.
    pub session_url: String,
    /// Remote-input AES key shared with Sunshine.
    pub remote_input_aes_key: [u8; 16],
    /// Remote-input AES IV expected by `moonlight-common-c`.
    pub remote_input_aes_iv: [u8; 16],
    /// Whether the request resumed an already-running application.
    pub resumed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StartVerb {
    Launch,
    Resume,
}

impl StartVerb {
    fn command(self) -> &'static str {
        match self {
            Self::Launch => "launch",
            Self::Resume => "resume",
        }
    }

    fn success_tag(self) -> &'static str {
        match self {
            Self::Launch => "gamesession",
            Self::Resume => "resume",
        }
    }

    fn timeout(self) -> Duration {
        match self {
            Self::Launch => LAUNCH_TIMEOUT,
            Self::Resume => RESUME_TIMEOUT,
        }
    }
}

impl GameStreamClient {
    /// Launch or resume the selected application according to current host state.
    ///
    /// # Arguments
    ///
    /// * `host` - Paired host returned by [`GameStreamClient::connect`].
    /// * `application` - Application selected from the host's advertised list.
    /// * `config` - Requested stream mode and input/audio preferences.
    ///
    /// # Returns
    ///
    /// A prepared session containing the RTSP URL and remote-input key material.
    pub fn start_session(
        &self,
        host: &ConnectedHost,
        application: &Application,
        config: LaunchConfig,
    ) -> Result<StreamSession, ControlError> {
        config.validate()?;
        validate_application(host, application)?;
        let server = self.session_server_info(host)?;
        let verb = match server.current_game {
            0 => StartVerb::Launch,
            current if current == application.id => StartVerb::Resume,
            current => {
                return Err(ControlError::Session(format!(
                    "host is already running application {current}; cancel it before starting {}",
                    application.id
                )));
            }
        };
        self.request_session(host, server, application, config, verb)
    }

    /// Launch an application when the host is idle.
    ///
    /// # Arguments
    ///
    /// * `host` - Paired host returned by [`GameStreamClient::connect`].
    /// * `application` - Application selected from the host's advertised list.
    /// * `config` - Requested stream mode and input/audio preferences.
    ///
    /// # Returns
    ///
    /// A prepared session containing the RTSP URL and remote-input key material.
    pub fn launch(
        &self,
        host: &ConnectedHost,
        application: &Application,
        config: LaunchConfig,
    ) -> Result<StreamSession, ControlError> {
        config.validate()?;
        validate_application(host, application)?;
        let server = self.session_server_info(host)?;
        if server.current_game != 0 {
            return Err(ControlError::Session(format!(
                "host is already running application {}",
                server.current_game
            )));
        }
        self.request_session(host, server, application, config, StartVerb::Launch)
    }

    /// Resume the selected application when it is already running on the host.
    ///
    /// # Arguments
    ///
    /// * `host` - Paired host returned by [`GameStreamClient::connect`].
    /// * `application` - Running application selected from the host's list.
    /// * `config` - Requested stream mode and input/audio preferences.
    ///
    /// # Returns
    ///
    /// A prepared session containing the RTSP URL and remote-input key material.
    pub fn resume(
        &self,
        host: &ConnectedHost,
        application: &Application,
        config: LaunchConfig,
    ) -> Result<StreamSession, ControlError> {
        config.validate()?;
        validate_application(host, application)?;
        let server = self.session_server_info(host)?;
        match server.current_game {
            0 => Err(ControlError::Session(
                "host has no running application to resume".to_owned(),
            )),
            current if current != application.id => Err(ControlError::Session(format!(
                "host is running application {current}, not {}",
                application.id
            ))),
            _ => self.request_session(host, server, application, config, StartVerb::Resume),
        }
    }

    /// Cancel the running GameStream session and host application.
    ///
    /// # Arguments
    ///
    /// * `host` - Paired host returned by [`GameStreamClient::connect`].
    ///
    /// # Returns
    ///
    /// Fresh host information confirming that no application remains active.
    pub fn cancel(&self, host: &ConnectedHost) -> Result<ServerInfo, ControlError> {
        let server = self.session_server_info(host)?;
        let xml = self.get(
            &host.endpoint,
            "https",
            server.https_port,
            "cancel",
            &[],
            CANCEL_TIMEOUT,
        )?;
        require_success_flag(&xml, "cancel")?;

        let server = self.session_server_info(host)?;
        if server.current_game != 0 {
            return Err(ControlError::Session(format!(
                "host still reports application {} after cancellation",
                server.current_game
            )));
        }
        Ok(server)
    }

    fn session_server_info(&self, host: &ConnectedHost) -> Result<ServerInfo, ControlError> {
        let server = self.authenticated_server_info(&host.endpoint, host.server.https_port)?;
        if !server.paired {
            return Err(ControlError::Pairing(
                "host no longer recognizes this client as paired".to_owned(),
            ));
        }
        Ok(server)
    }

    fn request_session(
        &self,
        host: &ConnectedHost,
        mut server: ServerInfo,
        application: &Application,
        config: LaunchConfig,
        verb: StartVerb,
    ) -> Result<StreamSession, ControlError> {
        let key = random_bytes::<16>();
        let mut iv = [0_u8; 16];
        iv[..4].copy_from_slice(&random_bytes::<4>());
        let key_id = i32::from_be_bytes(iv[..4].try_into().expect("four-byte RI key ID"));
        let arguments = session_arguments(application.id, config, &key, key_id);
        let xml = self.get(
            &host.endpoint,
            "https",
            server.https_port,
            verb.command(),
            &arguments,
            verb.timeout(),
        )?;
        require_success_flag(&xml, verb.success_tag())?;
        let session_url = response_text(&xml, "sessionUrl0")?;
        if session_url.is_empty() {
            return Err(ControlError::Session(
                "host returned an empty sessionUrl0".to_owned(),
            ));
        }
        server.current_game = application.id;

        Ok(StreamSession {
            endpoint: host.endpoint.clone(),
            server,
            application: application.clone(),
            config,
            session_url,
            remote_input_aes_key: key,
            remote_input_aes_iv: iv,
            resumed: verb == StartVerb::Resume,
        })
    }
}

fn validate_application(
    host: &ConnectedHost,
    application: &Application,
) -> Result<(), ControlError> {
    if application.id == 0 {
        return Err(ControlError::Session(
            "application ID must be non-zero".to_owned(),
        ));
    }
    if !host
        .applications
        .iter()
        .any(|candidate| candidate.id == application.id)
    {
        return Err(ControlError::Session(format!(
            "application {} is not in the connected host's application list",
            application.id
        )));
    }
    Ok(())
}

fn session_arguments(
    application_id: u32,
    config: LaunchConfig,
    remote_input_key: &[u8; 16],
    remote_input_key_id: i32,
) -> Vec<(&'static str, String)> {
    vec![
        ("appid", application_id.to_string()),
        (
            "mode",
            format!("{}x{}x{}", config.width, config.height, config.fps),
        ),
        ("additionalStates", "1".to_owned()),
        ("sops", u8::from(config.optimize_game_settings).to_string()),
        ("rikey", hex::encode(remote_input_key)),
        ("rikeyid", remote_input_key_id.to_string()),
        (
            "localAudioPlayMode",
            u8::from(config.play_audio_on_host).to_string(),
        ),
        ("surroundAudioInfo", STEREO_SURROUND_AUDIO_INFO.to_string()),
        ("remoteControllersBitmap", config.gamepad_mask.to_string()),
        ("gcmap", config.gamepad_mask.to_string()),
        ("gcpersist", u8::from(config.persist_gamepads).to_string()),
        ("corever", MOONLIGHT_CORE_VERSION.to_string()),
    ]
}

fn require_success_flag(xml: &str, tag: &str) -> Result<(), ControlError> {
    if response_text(xml, tag)? == "1" {
        Ok(())
    } else {
        Err(ControlError::Session(format!(
            "host returned an unsuccessful {tag} response"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        LaunchConfig, STEREO_SURROUND_AUDIO_INFO, require_success_flag, session_arguments,
    };

    #[test]
    fn default_launch_arguments_match_sunshine_requirements() {
        let key = [0x5a; 16];
        let arguments = session_arguments(42, LaunchConfig::default(), &key, -123);
        let value = |name: &str| {
            arguments
                .iter()
                .find_map(|(candidate, value)| (*candidate == name).then_some(value.as_str()))
                .expect("required launch argument")
        };

        assert_eq!(value("appid"), "42");
        assert_eq!(value("mode"), "1920x1080x60");
        assert_eq!(value("rikey"), "5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a");
        assert_eq!(value("rikeyid"), "-123");
        assert_eq!(
            value("surroundAudioInfo"),
            STEREO_SURROUND_AUDIO_INFO.to_string()
        );
        assert_eq!(value("corever"), "1");
    }

    #[test]
    fn parses_success_flags() {
        require_success_flag(
            r#"<root status_code="200"><gamesession>1</gamesession></root>"#,
            "gamesession",
        )
        .expect("successful launch");
        assert!(
            require_success_flag(
                r#"<root status_code="200"><cancel>0</cancel></root>"#,
                "cancel"
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_empty_stream_dimensions() {
        let config = LaunchConfig {
            width: 0,
            ..LaunchConfig::default()
        };
        assert!(config.validate().is_err());
    }
}
