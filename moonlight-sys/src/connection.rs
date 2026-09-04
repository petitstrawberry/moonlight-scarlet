//! Safe ownership around the singleton `moonlight-common-c` connection.

use std::ffi::{CString, c_char, c_int, c_short, c_void};
use std::fmt;
use std::marker::PhantomData;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::audio::{
    AudioRenderer, clear_audio_renderer, current_audio_error, install_audio_renderer,
};

const STREAM_CFG_AUTO: c_int = 2;
const AUDIO_CONFIGURATION_STEREO: c_int = (0x3 << 16) | (2 << 8) | 0xCA;
const VIDEO_FORMAT_H264: c_int = 0x0001;
const COLORSPACE_REC_709: c_int = 1;
const COLOR_RANGE_LIMITED: c_int = 0;
const ENCRYPTION_ALL: c_int = -1;
const DR_OK: c_int = 0;
const DR_NEED_IDR: c_int = -1;
const NO_REPORTED_ERROR: c_int = c_int::MIN;
const MAX_ACCESS_UNIT_LENGTH: usize = 64 * 1024 * 1024;
const BUTTON_ACTION_PRESS: c_char = 0x07;
const BUTTON_ACTION_RELEASE: c_char = 0x08;
const KEY_ACTION_DOWN: c_char = 0x03;
const KEY_ACTION_UP: c_char = 0x04;
const MODIFIER_SHIFT: u8 = 0x01;
const MODIFIER_CONTROL: u8 = 0x02;
const MODIFIER_ALT: u8 = 0x04;
const MODIFIER_META: u8 = 0x08;
const KEY_CODE_VIRTUAL_KEY: u16 = 0x8000;

static CONNECTION_ACTIVE: AtomicBool = AtomicBool::new(false);
static CORE_CONNECTION_LOCK: Mutex<()> = Mutex::new(());

unsafe extern "C" {
    fn mls_start_connection(
        address: *const c_char,
        app_version: *const c_char,
        gfe_version: *const c_char,
        rtsp_session_url: *const c_char,
        server_codec_mode_support: c_int,
        width: c_int,
        height: c_int,
        fps: c_int,
        bitrate: c_int,
        packet_size: c_int,
        streaming_remotely: c_int,
        audio_configuration: c_int,
        supported_video_formats: c_int,
        client_refresh_rate_x100: c_int,
        color_space: c_int,
        color_range: c_int,
        encryption_flags: c_int,
        remote_input_aes_key: *const u8,
        remote_input_aes_iv: *const u8,
    ) -> c_int;
    fn mls_stop_connection();
    fn mls_wake_video_frame();
    fn mls_wait_video_frame(frame_handle: *mut *mut c_void, decode_unit: *mut *mut c_void) -> bool;
    fn mls_video_frame_number(decode_unit: *const c_void) -> c_int;
    fn mls_video_frame_type(decode_unit: *const c_void) -> c_int;
    fn mls_video_frame_presentation_time_us(decode_unit: *const c_void) -> u64;
    fn mls_video_frame_full_length(decode_unit: *const c_void) -> c_int;
    fn mls_video_frame_hdr_active(decode_unit: *const c_void) -> bool;
    fn mls_video_frame_colorspace(decode_unit: *const c_void) -> u8;
    fn mls_copy_video_frame(
        decode_unit: *const c_void,
        destination: *mut u8,
        destination_length: usize,
    ) -> c_int;
    fn mls_complete_video_frame(frame_handle: *mut c_void, decoder_status: c_int);
    fn mls_last_stage_value() -> c_int;
    fn mls_stage_error_value() -> c_int;
    fn mls_termination_error_value() -> c_int;
    fn mls_connection_started_value() -> bool;
    fn mls_video_format_value() -> c_int;
    fn mls_video_width_value() -> c_int;
    fn mls_video_height_value() -> c_int;
    fn mls_video_fps_value() -> c_int;
    fn LiSendMouseMoveEvent(delta_x: c_short, delta_y: c_short) -> c_int;
    fn LiSendMouseButtonEvent(action: c_char, button: c_int) -> c_int;
    fn LiSendKeyboardEvent2(
        key_code: c_short,
        key_action: c_char,
        modifiers: c_char,
        flags: c_char,
    ) -> c_int;
    fn LiSendHighResScrollEvent(scroll_amount: c_short) -> c_int;
    fn LiSendHighResHScrollEvent(scroll_amount: c_short) -> c_int;
}

/// Host fields required by `moonlight-common-c` after launch or resume.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostConnectionInfo {
    address: String,
    app_version: String,
    gfe_version: Option<String>,
    rtsp_session_url: String,
    server_codec_mode_support: u32,
}

impl HostConnectionInfo {
    /// Construct the host information consumed by the streaming core.
    ///
    /// # Arguments
    ///
    /// * `address` - Hostname or numeric address used for streaming sockets.
    /// * `app_version` - GameStream `appversion` from `serverinfo`.
    /// * `gfe_version` - Optional `GfeVersion` compatibility string.
    /// * `rtsp_session_url` - `sessionUrl0` returned by launch or resume.
    /// * `server_codec_mode_support` - Raw `ServerCodecModeSupport` bitmap.
    ///
    /// # Returns
    ///
    /// Owned host information whose strings can safely outlive connection setup.
    pub fn new(
        address: impl Into<String>,
        app_version: impl Into<String>,
        gfe_version: Option<String>,
        rtsp_session_url: impl Into<String>,
        server_codec_mode_support: u32,
    ) -> Self {
        Self {
            address: address.into(),
            app_version: app_version.into(),
            gfe_version,
            rtsp_session_url: rtsp_session_url.into(),
            server_codec_mode_support,
        }
    }
}

/// Video and transport preferences passed to `moonlight-common-c`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamConfiguration {
    width: u32,
    height: u32,
    fps: u32,
    bitrate_kbps: u32,
    packet_size: u32,
    remote_input_aes_key: [u8; 16],
    remote_input_aes_iv: [u8; 16],
}

impl StreamConfiguration {
    /// Construct an H.264 stereo stream configuration.
    ///
    /// The initial defaults are a 20 Mbps bitrate, a 1024-byte video packet,
    /// automatic local/remote detection, Rec. 709 limited range, and transport
    /// encryption wherever Sunshine supports it.
    ///
    /// # Arguments
    ///
    /// * `width` - Requested video width in pixels.
    /// * `height` - Requested video height in pixels.
    /// * `fps` - Requested frame rate.
    /// * `remote_input_aes_key` - Key returned by the control-plane launch.
    /// * `remote_input_aes_iv` - IV returned by the control-plane launch.
    ///
    /// # Returns
    ///
    /// A configuration suitable for the current Scarlet H.264 decoder path.
    pub const fn new(
        width: u32,
        height: u32,
        fps: u32,
        remote_input_aes_key: [u8; 16],
        remote_input_aes_iv: [u8; 16],
    ) -> Self {
        Self {
            width,
            height,
            fps,
            bitrate_kbps: 20_000,
            packet_size: 1_024,
            remote_input_aes_key,
            remote_input_aes_iv,
        }
    }

    /// Override the requested video bitrate.
    ///
    /// # Arguments
    ///
    /// * `bitrate_kbps` - Video bitrate in kilobits per second.
    ///
    /// # Returns
    ///
    /// The updated stream configuration.
    pub const fn with_bitrate_kbps(mut self, bitrate_kbps: u32) -> Self {
        self.bitrate_kbps = bitrate_kbps;
        self
    }

    /// Override the maximum video packet size.
    ///
    /// # Arguments
    ///
    /// * `packet_size` - Packet size in bytes; the core rounds it down to a
    ///   multiple of 16.
    ///
    /// # Returns
    ///
    /// The updated stream configuration.
    pub const fn with_packet_size(mut self, packet_size: u32) -> Self {
        self.packet_size = packet_size;
        self
    }

    fn validated(self) -> Result<RawStreamConfiguration, ConnectionError> {
        let width = positive_int("width", self.width)?;
        let height = positive_int("height", self.height)?;
        let fps = positive_int("FPS", self.fps)?;
        let bitrate = positive_int("bitrate", self.bitrate_kbps)?;
        let packet_size = positive_int("packet size", self.packet_size)?;
        let refresh_rate = self
            .fps
            .checked_mul(100)
            .ok_or_else(|| invalid_configuration("refresh rate is too large"))?;

        Ok(RawStreamConfiguration {
            width,
            height,
            fps,
            bitrate,
            packet_size,
            client_refresh_rate_x100: positive_int("refresh rate", refresh_rate)?,
            remote_input_aes_key: self.remote_input_aes_key,
            remote_input_aes_iv: self.remote_input_aes_iv,
        })
    }
}

#[derive(Debug)]
struct RawStreamConfiguration {
    width: c_int,
    height: c_int,
    fps: c_int,
    bitrate: c_int,
    packet_size: c_int,
    client_refresh_rate_x100: c_int,
    remote_input_aes_key: [u8; 16],
    remote_input_aes_iv: [u8; 16],
}

#[derive(Debug)]
struct HostStrings {
    address: CString,
    app_version: CString,
    gfe_version: Option<CString>,
    rtsp_session_url: CString,
}

impl HostStrings {
    fn new(host: &HostConnectionInfo) -> Result<Self, ConnectionError> {
        Ok(Self {
            address: c_string("address", &host.address)?,
            app_version: c_string("app version", &host.app_version)?,
            gfe_version: host
                .gfe_version
                .as_deref()
                .map(|value| c_string("GFE version", value))
                .transpose()?,
            rtsp_session_url: c_string("RTSP session URL", &host.rtsp_session_url)?,
        })
    }

    fn gfe_version_pointer(&self) -> *const c_char {
        self.gfe_version
            .as_ref()
            .map_or(ptr::null(), |value| value.as_ptr())
    }
}

/// Error raised while configuring, starting, or consuming a core connection.
#[derive(Debug, PartialEq, Eq)]
pub enum ConnectionError {
    /// The process-wide `moonlight-common-c` connection is already owned.
    AlreadyActive,
    /// A configuration value cannot be represented by the C core.
    InvalidConfiguration(String),
    /// `LiStartConnection()` failed during the reported stage.
    StartFailed {
        /// Raw error returned by `LiStartConnection()`.
        code: i32,
        /// Last initialization stage entered by the core.
        stage: i32,
        /// Stage-specific failure code when one was reported.
        stage_error: Option<i32>,
    },
    /// A queued video frame had invalid pointers, lengths, or buffer contents.
    InvalidVideoFrame(String),
    /// The configured audio renderer could not initialize.
    Audio(String),
}

impl fmt::Display for ConnectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyActive => formatter.write_str("a Moonlight stream is already active"),
            Self::InvalidConfiguration(message) => {
                write!(formatter, "invalid stream configuration: {message}")
            }
            Self::StartFailed {
                code,
                stage,
                stage_error,
            } => {
                write!(
                    formatter,
                    "stream connection failed with {code} at stage {stage}"
                )?;
                if let Some(stage_error) = stage_error {
                    write!(formatter, " (stage error {stage_error})")?;
                }
                Ok(())
            }
            Self::InvalidVideoFrame(message) => write!(formatter, "invalid video frame: {message}"),
            Self::Audio(message) => write!(formatter, "audio initialization failed: {message}"),
        }
    }
}

impl std::error::Error for ConnectionError {}

struct ConnectionState {
    active: AtomicBool,
    stop_requested: AtomicBool,
}

/// Press or release state for a remote key or mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputAction {
    /// Press the key or mouse button.
    Press,
    /// Release the key or mouse button.
    Release,
}

/// Mouse buttons supported by the GameStream input protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    /// Primary mouse button.
    Left,
    /// Middle mouse button.
    Middle,
    /// Secondary mouse button.
    Right,
}

impl MouseButton {
    const fn raw(self) -> c_int {
        match self {
            Self::Left => 0x01,
            Self::Middle => 0x02,
            Self::Right => 0x03,
        }
    }
}

/// Modifier flags accompanying a remote keyboard event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyboardModifiers {
    bits: u8,
}

impl KeyboardModifiers {
    /// Construct the modifier flags reported with a remote key event.
    ///
    /// # Arguments
    ///
    /// * `shift` - Whether either Shift key is held.
    /// * `control` - Whether either Control key is held.
    /// * `alt` - Whether either Alt key is held.
    /// * `meta` - Whether either Windows, Super, or Command key is held.
    ///
    /// # Returns
    ///
    /// Packed modifier state accepted by `moonlight-common-c`.
    pub const fn new(shift: bool, control: bool, alt: bool, meta: bool) -> Self {
        let mut bits = 0;
        if shift {
            bits |= MODIFIER_SHIFT;
        }
        if control {
            bits |= MODIFIER_CONTROL;
        }
        if alt {
            bits |= MODIFIER_ALT;
        }
        if meta {
            bits |= MODIFIER_META;
        }
        Self { bits }
    }

    /// Return the packed GameStream modifier bitmap.
    ///
    /// # Returns
    ///
    /// A combination of the Shift, Control, Alt, and Meta bits.
    pub const fn bits(self) -> u8 {
        self.bits
    }
}

/// Error returned when remote input cannot be queued.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputError {
    /// The connection has already stopped or is not ready for input.
    ConnectionInactive,
    /// `moonlight-common-c` rejected or could not allocate an input packet.
    Core {
        /// Name of the input operation that failed.
        operation: &'static str,
        /// Raw error returned by `moonlight-common-c`.
        code: i32,
    },
}

impl fmt::Display for InputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConnectionInactive => formatter.write_str("stream input is not active"),
            Self::Core { operation, code } => {
                write!(formatter, "{operation} input failed with {code}")
            }
        }
    }
}

impl std::error::Error for InputError {}

/// Cloneable cancellation handle for a running stream worker.
#[derive(Clone)]
pub struct ConnectionControl {
    state: Arc<ConnectionState>,
}

impl ConnectionControl {
    /// Request orderly shutdown and wake a worker blocked on the next frame.
    pub fn request_stop(&self) {
        self.state.stop_requested.store(true, Ordering::Release);
        if self.state.active.load(Ordering::Acquire) {
            // SAFETY: waking the singleton pull-renderer queue is valid while
            // the corresponding connection state remains active.
            unsafe { mls_wake_video_frame() };
        }
    }

    /// Test whether shutdown has been requested.
    ///
    /// # Returns
    ///
    /// `true` after [`ConnectionControl::request_stop`] is called.
    pub fn stop_requested(&self) -> bool {
        self.state.stop_requested.load(Ordering::Acquire)
    }

    /// Queue relative mouse movement for the host.
    ///
    /// # Arguments
    ///
    /// * `delta_x` - Horizontal relative motion.
    /// * `delta_y` - Vertical relative motion.
    ///
    /// # Returns
    ///
    /// Success when the input packet was queued or coalesced.
    pub fn send_mouse_move(&self, delta_x: i16, delta_y: i16) -> Result<(), InputError> {
        self.send_input("mouse move", || {
            // SAFETY: the active core owns its input queue and both scalar
            // arguments exactly match the C `short` parameters.
            unsafe { LiSendMouseMoveEvent(delta_x, delta_y) }
        })
    }

    /// Queue a mouse-button transition for the host.
    ///
    /// # Arguments
    ///
    /// * `button` - Mouse button to update.
    /// * `action` - Whether the button was pressed or released.
    ///
    /// # Returns
    ///
    /// Success when the input packet was queued.
    pub fn send_mouse_button(
        &self,
        button: MouseButton,
        action: InputAction,
    ) -> Result<(), InputError> {
        let action = match action {
            InputAction::Press => BUTTON_ACTION_PRESS,
            InputAction::Release => BUTTON_ACTION_RELEASE,
        };
        self.send_input("mouse button", || {
            // SAFETY: the action and button values are constants defined by
            // `Limelight.h`, and the core input queue is active.
            unsafe { LiSendMouseButtonEvent(action, button.raw()) }
        })
    }

    /// Queue a Win32 virtual-key transition for the host.
    ///
    /// # Arguments
    ///
    /// * `key_code` - Win32 virtual-key code interpreted using the US layout.
    /// * `action` - Whether the key was pressed or released.
    /// * `modifiers` - Modifier state accompanying this event.
    ///
    /// # Returns
    ///
    /// Success when the input packet was queued.
    pub fn send_keyboard(
        &self,
        key_code: u16,
        action: InputAction,
        modifiers: KeyboardModifiers,
    ) -> Result<(), InputError> {
        let action = match action {
            InputAction::Press => KEY_ACTION_DOWN,
            InputAction::Release => KEY_ACTION_UP,
        };
        let key_code = wire_key_code(key_code);
        self.send_input("keyboard", || {
            // SAFETY: the key code carries moonlight-common's Win32 VK marker,
            // modifier bits are defined by `Limelight.h`, and zero requests
            // the standard normalized-key path.
            unsafe {
                LiSendKeyboardEvent2(key_code as c_short, action, modifiers.bits() as c_char, 0)
            }
        })
    }

    /// Queue high-resolution vertical wheel motion for the host.
    ///
    /// # Arguments
    ///
    /// * `amount` - Windows wheel units; 120 represents one wheel detent.
    ///
    /// # Returns
    ///
    /// Success when the input packet was queued or coalesced.
    pub fn send_vertical_scroll(&self, amount: i16) -> Result<(), InputError> {
        self.send_input("vertical scroll", || {
            // SAFETY: the active core accepts one C `short` wheel amount.
            unsafe { LiSendHighResScrollEvent(amount) }
        })
    }

    /// Queue high-resolution horizontal wheel motion for the host.
    ///
    /// # Arguments
    ///
    /// * `amount` - Windows wheel units; 120 represents one wheel detent.
    ///
    /// # Returns
    ///
    /// Success when the input packet was queued or coalesced.
    pub fn send_horizontal_scroll(&self, amount: i16) -> Result<(), InputError> {
        self.send_input("horizontal scroll", || {
            // SAFETY: the active Sunshine core accepts one C `short` wheel
            // amount for its horizontal-scroll extension.
            unsafe { LiSendHighResHScrollEvent(amount) }
        })
    }

    fn send_input(
        &self,
        operation: &'static str,
        send: impl FnOnce() -> c_int,
    ) -> Result<(), InputError> {
        let _guard = lock_core_connection();
        if !self.state.active.load(Ordering::Acquire) {
            return Err(InputError::ConnectionInactive);
        }
        let result = send();
        if result == 0 {
            Ok(())
        } else {
            Err(InputError::Core {
                operation,
                code: result,
            })
        }
    }
}

/// Negotiated video parameters reported by the core's renderer setup callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoSetup {
    video_format: i32,
    width: u32,
    height: u32,
    fps: u32,
}

impl VideoSetup {
    /// Return the raw `VIDEO_FORMAT_*` bitmap selected by the host.
    pub const fn video_format(self) -> i32 {
        self.video_format
    }

    /// Return the negotiated video width.
    pub const fn width(self) -> u32 {
        self.width
    }

    /// Return the negotiated video height.
    pub const fn height(self) -> u32 {
        self.height
    }

    /// Return the negotiated redraw rate.
    pub const fn fps(self) -> u32 {
        self.fps
    }

    /// Test whether the selected format is an H.264 profile.
    pub const fn is_h264(self) -> bool {
        self.video_format & 0x000F != 0
    }
}

/// Active ownership of the process-wide `moonlight-common-c` connection.
pub struct Connection {
    _host_strings: HostStrings,
    state: Arc<ConnectionState>,
    stopped: bool,
}

impl Connection {
    /// Start the streaming transport and negotiate an H.264 pull renderer.
    ///
    /// # Arguments
    ///
    /// * `host` - Host metadata from the control-plane `serverinfo` and launch.
    /// * `stream` - Requested video mode and remote-input encryption material.
    ///
    /// # Returns
    ///
    /// Exclusive connection ownership after all core stages have started.
    pub fn start(
        host: &HostConnectionInfo,
        stream: StreamConfiguration,
    ) -> Result<Self, ConnectionError> {
        Self::start_with_audio(host, stream, None)
    }

    /// Start the streaming transport with an optional audio renderer.
    ///
    /// # Arguments
    ///
    /// * `host` - Host metadata from the control-plane `serverinfo` and launch.
    /// * `stream` - Requested video mode and remote-input encryption material.
    /// * `audio_renderer` - Sink for negotiated Opus packets, or `None` to discard audio.
    ///
    /// # Returns
    ///
    /// Exclusive connection ownership after all core stages have started.
    pub fn start_with_audio(
        host: &HostConnectionInfo,
        stream: StreamConfiguration,
        audio_renderer: Option<Box<dyn AudioRenderer>>,
    ) -> Result<Self, ConnectionError> {
        let host_strings = HostStrings::new(host)?;
        let stream = stream.validated()?;
        let codec_support = positive_int("server codec support", host.server_codec_mode_support)?;

        if CONNECTION_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(ConnectionError::AlreadyActive);
        }
        install_audio_renderer(audio_renderer);

        // SAFETY: all pointers remain valid for the call and the owned C
        // strings are retained by `Connection` for the full session. Array
        // pointers refer to exactly 16 initialized bytes.
        let result = unsafe {
            mls_start_connection(
                host_strings.address.as_ptr(),
                host_strings.app_version.as_ptr(),
                host_strings.gfe_version_pointer(),
                host_strings.rtsp_session_url.as_ptr(),
                codec_support,
                stream.width,
                stream.height,
                stream.fps,
                stream.bitrate,
                stream.packet_size,
                STREAM_CFG_AUTO,
                AUDIO_CONFIGURATION_STEREO,
                VIDEO_FORMAT_H264,
                stream.client_refresh_rate_x100,
                COLORSPACE_REC_709,
                COLOR_RANGE_LIMITED,
                ENCRYPTION_ALL,
                stream.remote_input_aes_key.as_ptr(),
                stream.remote_input_aes_iv.as_ptr(),
            )
        };
        if result != 0 {
            let audio_error = current_audio_error();
            clear_audio_renderer();
            CONNECTION_ACTIVE.store(false, Ordering::Release);
            if let Some(audio_error) = audio_error {
                return Err(ConnectionError::Audio(audio_error));
            }
            return Err(ConnectionError::StartFailed {
                code: result,
                // SAFETY: these bridge accessors atomically read scalar state.
                stage: unsafe { mls_last_stage_value() },
                // SAFETY: these bridge accessors atomically read scalar state.
                stage_error: reported_error(unsafe { mls_stage_error_value() }),
            });
        }

        let state = Arc::new(ConnectionState {
            active: AtomicBool::new(true),
            stop_requested: AtomicBool::new(false),
        });
        Ok(Self {
            _host_strings: host_strings,
            state,
            stopped: false,
        })
    }

    /// Create a cloneable handle that can wake this connection's video worker.
    ///
    /// # Returns
    ///
    /// A cancellation handle tied to this connection instance.
    pub fn control(&self) -> ConnectionControl {
        ConnectionControl {
            state: Arc::clone(&self.state),
        }
    }

    /// Return video parameters selected during RTSP negotiation.
    ///
    /// # Returns
    ///
    /// Negotiated parameters, or `None` before the video setup callback ran.
    pub fn video_setup(&self) -> Option<VideoSetup> {
        // SAFETY: the bridge accessors atomically read scalar callback state.
        let video_format = unsafe { mls_video_format_value() };
        if video_format == 0 {
            return None;
        }
        // SAFETY: the bridge accessors atomically read scalar callback state.
        let width = unsafe { mls_video_width_value() };
        // SAFETY: the bridge accessors atomically read scalar callback state.
        let height = unsafe { mls_video_height_value() };
        // SAFETY: the bridge accessors atomically read scalar callback state.
        let fps = unsafe { mls_video_fps_value() };
        Some(VideoSetup {
            video_format,
            width: u32::try_from(width).ok()?,
            height: u32::try_from(height).ok()?,
            fps: u32::try_from(fps).ok()?,
        })
    }

    /// Test whether the core invoked its connection-started callback.
    pub fn is_started(&self) -> bool {
        // SAFETY: this bridge accessor atomically reads scalar callback state.
        unsafe { mls_connection_started_value() }
    }

    /// Return an asynchronous termination error reported by the core.
    ///
    /// # Returns
    ///
    /// `None` while no termination has been reported; otherwise the raw core
    /// error code. Zero represents a graceful host-side termination.
    pub fn termination_error(&self) -> Option<i32> {
        // SAFETY: this bridge accessor atomically reads scalar callback state.
        reported_error(unsafe { mls_termination_error_value() })
    }

    /// Return the first runtime error reported by the audio renderer.
    ///
    /// # Returns
    ///
    /// The renderer error, or `None` while audio remains healthy or disabled.
    pub fn audio_error(&self) -> Option<String> {
        current_audio_error()
    }

    /// Block until the next complete coded frame is available.
    ///
    /// # Returns
    ///
    /// A frame that must be completed before requesting another, or `None`
    /// when the queue is woken for shutdown.
    pub fn wait_for_video_frame(&mut self) -> Result<Option<VideoFrame<'_>>, ConnectionError> {
        if self.stopped || self.state.stop_requested.load(Ordering::Acquire) {
            return Ok(None);
        }
        let mut frame_handle = ptr::null_mut();
        let mut decode_unit = ptr::null_mut();
        // SAFETY: both output pointers are valid and the connection exclusively
        // owns the pull renderer until the returned frame is completed.
        let available = unsafe { mls_wait_video_frame(&mut frame_handle, &mut decode_unit) };
        if !available {
            return Ok(None);
        }
        if frame_handle.is_null() || decode_unit.is_null() {
            if !frame_handle.is_null() {
                // SAFETY: a non-null handle returned by the core must be
                // completed exactly once even if its decode-unit pointer is bad.
                unsafe { mls_complete_video_frame(frame_handle, DR_NEED_IDR) };
            }
            return Err(ConnectionError::InvalidVideoFrame(String::from(
                "core returned a null frame pointer",
            )));
        }

        // SAFETY: metadata accessors only read the live decode unit retained by
        // `frame_handle` until completion.
        let full_length = unsafe { mls_video_frame_full_length(decode_unit) };
        let Ok(full_length) = usize::try_from(full_length) else {
            // SAFETY: `frame_handle` is live and has not been completed.
            unsafe { mls_complete_video_frame(frame_handle, DR_NEED_IDR) };
            return Err(ConnectionError::InvalidVideoFrame(String::from(
                "core returned a non-positive access-unit length",
            )));
        };
        if full_length == 0 || full_length > MAX_ACCESS_UNIT_LENGTH {
            // SAFETY: `frame_handle` is live and has not been completed.
            unsafe { mls_complete_video_frame(frame_handle, DR_NEED_IDR) };
            return Err(ConnectionError::InvalidVideoFrame(format!(
                "access-unit length {full_length} is outside the supported range"
            )));
        }

        // SAFETY: all metadata accessors read the same live decode unit.
        let frame_number = unsafe { mls_video_frame_number(decode_unit) };
        // SAFETY: all metadata accessors read the same live decode unit.
        let frame_type = unsafe { mls_video_frame_type(decode_unit) };
        // SAFETY: all metadata accessors read the same live decode unit.
        let presentation_time_us = unsafe { mls_video_frame_presentation_time_us(decode_unit) };
        // SAFETY: all metadata accessors read the same live decode unit.
        let hdr_active = unsafe { mls_video_frame_hdr_active(decode_unit) };
        // SAFETY: all metadata accessors read the same live decode unit.
        let colorspace = unsafe { mls_video_frame_colorspace(decode_unit) };

        Ok(Some(VideoFrame {
            frame_handle,
            decode_unit,
            frame_number,
            frame_type,
            presentation_time_us,
            full_length,
            hdr_active,
            colorspace,
            completed: false,
            _connection: PhantomData,
        }))
    }

    /// Stop all core streams and release singleton ownership.
    pub fn stop(&mut self) {
        if self.stopped {
            return;
        }
        self.state.stop_requested.store(true, Ordering::Release);
        let _guard = lock_core_connection();
        // SAFETY: this object exclusively owns the active singleton connection.
        unsafe { mls_stop_connection() };
        clear_audio_renderer();
        self.state.active.store(false, Ordering::Release);
        CONNECTION_ACTIVE.store(false, Ordering::Release);
        self.stopped = true;
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Completion status returned for a pulled compressed frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoFrameStatus {
    /// The frame was accepted and processed successfully.
    Complete,
    /// The decoder could not consume the frame and needs a fresh IDR.
    NeedIdr,
}

impl VideoFrameStatus {
    const fn raw(self) -> c_int {
        match self {
            Self::Complete => DR_OK,
            Self::NeedIdr => DR_NEED_IDR,
        }
    }
}

/// One compressed frame borrowed from the core's pull-renderer queue.
pub struct VideoFrame<'connection> {
    frame_handle: *mut c_void,
    decode_unit: *mut c_void,
    frame_number: i32,
    frame_type: i32,
    presentation_time_us: u64,
    full_length: usize,
    hdr_active: bool,
    colorspace: u8,
    completed: bool,
    _connection: PhantomData<&'connection mut Connection>,
}

impl VideoFrame<'_> {
    /// Return the Moonlight frame sequence number.
    pub const fn frame_number(&self) -> i32 {
        self.frame_number
    }

    /// Test whether this is an IDR frame.
    pub const fn is_idr(&self) -> bool {
        self.frame_type == 1
    }

    /// Return the presentation timestamp in microseconds.
    pub const fn presentation_time_us(&self) -> u64 {
        self.presentation_time_us
    }

    /// Return whether the host marks this frame as HDR.
    pub const fn hdr_active(&self) -> bool {
        self.hdr_active
    }

    /// Return the raw Moonlight colorspace identifier.
    pub const fn colorspace(&self) -> u8 {
        self.colorspace
    }

    /// Copy the linked C buffers into one complete coded access unit.
    ///
    /// # Returns
    ///
    /// Contiguous Annex B bytes for H.264, or an invalid-frame error.
    pub fn copy_access_unit(&self) -> Result<Vec<u8>, ConnectionError> {
        let mut access_unit = vec![0_u8; self.full_length];
        // SAFETY: `decode_unit` remains live until this frame is completed and
        // the destination covers exactly `full_length` initialized bytes.
        let copied = unsafe {
            mls_copy_video_frame(
                self.decode_unit,
                access_unit.as_mut_ptr(),
                access_unit.len(),
            )
        };
        if usize::try_from(copied).ok() != Some(access_unit.len()) {
            return Err(ConnectionError::InvalidVideoFrame(String::from(
                "buffer chain did not match the advertised access-unit length",
            )));
        }
        Ok(access_unit)
    }

    /// Complete this frame and release its core-owned buffers.
    ///
    /// # Arguments
    ///
    /// * `status` - Whether the decoder accepted the frame or needs an IDR.
    pub fn complete(mut self, status: VideoFrameStatus) {
        // SAFETY: this live handle has not been completed and `self` is
        // consumed so safe callers cannot complete it again.
        unsafe { mls_complete_video_frame(self.frame_handle, status.raw()) };
        self.completed = true;
    }
}

impl Drop for VideoFrame<'_> {
    fn drop(&mut self) {
        if !self.completed {
            // SAFETY: dropping is the final owner of this uncompleted handle.
            // Requesting an IDR is safer than treating an unprocessed frame as
            // successfully decoded.
            unsafe { mls_complete_video_frame(self.frame_handle, DR_NEED_IDR) };
            self.completed = true;
        }
    }
}

fn positive_int(name: &str, value: u32) -> Result<c_int, ConnectionError> {
    if value == 0 {
        return Err(invalid_configuration(format!("{name} must be non-zero")));
    }
    c_int::try_from(value)
        .map_err(|_| invalid_configuration(format!("{name} does not fit in a C integer")))
}

fn c_string(name: &str, value: &str) -> Result<CString, ConnectionError> {
    if value.is_empty() {
        return Err(invalid_configuration(format!("{name} is empty")));
    }
    CString::new(value)
        .map_err(|_| invalid_configuration(format!("{name} contains an interior NUL byte")))
}

fn invalid_configuration(message: impl Into<String>) -> ConnectionError {
    ConnectionError::InvalidConfiguration(message.into())
}

fn reported_error(value: c_int) -> Option<i32> {
    (value != NO_REPORTED_ERROR).then_some(value)
}

fn wire_key_code(key_code: u16) -> u16 {
    KEY_CODE_VIRTUAL_KEY | (key_code & 0x00FF)
}

fn lock_core_connection() -> MutexGuard<'static, ()> {
    CORE_CONNECTION_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use super::{
        ConnectionControl, ConnectionState, HostConnectionInfo, HostStrings, InputAction,
        InputError, KeyboardModifiers, MouseButton, StreamConfiguration, wire_key_code,
    };

    #[test]
    fn validates_default_h264_stream_configuration() {
        let stream = StreamConfiguration::new(1_920, 1_080, 60, [1; 16], [2; 16])
            .validated()
            .expect("stream configuration");

        assert_eq!(stream.width, 1_920);
        assert_eq!(stream.height, 1_080);
        assert_eq!(stream.fps, 60);
        assert_eq!(stream.bitrate, 20_000);
        assert_eq!(stream.packet_size, 1_024);
        assert_eq!(stream.client_refresh_rate_x100, 6_000);
    }

    #[test]
    fn rejects_zero_stream_dimensions() {
        let error = StreamConfiguration::new(0, 1_080, 60, [0; 16], [0; 16])
            .validated()
            .expect_err("zero width must fail");

        assert!(error.to_string().contains("width must be non-zero"));
    }

    #[test]
    fn rejects_interior_nul_in_host_fields() {
        let host = HostConnectionInfo::new(
            "sunshine\0invalid",
            "7.1.431.-1",
            Some(String::from("3.23.0.74")),
            "rtspenc://sunshine:48010",
            1,
        );
        let error = HostStrings::new(&host).expect_err("interior NUL must fail");

        assert!(
            error
                .to_string()
                .contains("address contains an interior NUL")
        );
    }

    #[test]
    fn packs_keyboard_modifiers_for_the_core() {
        let modifiers = KeyboardModifiers::new(true, false, true, true);

        assert_eq!(modifiers.bits(), 0x0D);
    }

    #[test]
    fn marks_win32_virtual_key_codes_for_the_wire() {
        assert_eq!(wire_key_code(0x41), 0x8041);
        assert_eq!(wire_key_code(0x8041), 0x8041);
    }

    #[test]
    fn rejects_input_after_connection_shutdown_without_calling_c() {
        let control = ConnectionControl {
            state: Arc::new(ConnectionState {
                active: AtomicBool::new(false),
                stop_requested: AtomicBool::new(true),
            }),
        };

        assert_eq!(
            control.send_mouse_move(4, -3),
            Err(InputError::ConnectionInactive)
        );
        assert_eq!(
            control.send_mouse_button(MouseButton::Left, InputAction::Press),
            Err(InputError::ConnectionInactive)
        );
        assert_eq!(
            control.send_keyboard(0x41, InputAction::Press, KeyboardModifiers::default(),),
            Err(InputError::ConnectionInactive)
        );
    }
}
