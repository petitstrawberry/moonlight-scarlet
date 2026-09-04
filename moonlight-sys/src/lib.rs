//! Rust ownership and FFI boundary for `moonlight-common-c`.
//!
//! The C core and its platform bridge will remain private to this crate. Safe
//! callers must not depend on C structure layout or global lifetime rules.

use std::ffi::CStr;
use std::time::Duration;

mod audio;
mod connection;

#[cfg(target_os = "scarlet")]
mod scarlet;

pub use audio::{AudioRenderer, AudioSetup};
pub use connection::{
    Connection, ConnectionControl, ConnectionError, HostConnectionInfo, InputAction, InputError,
    KeyboardModifiers, MouseButton, StreamConfiguration, VideoFrame, VideoFrameStatus, VideoSetup,
};

/// Pinned upstream `moonlight-common-c` revision used by this port.
pub const UPSTREAM_REVISION: &str = "874ac9548f1bd6f095ef2b435c42cdde460e7821";

unsafe extern "C" {
    fn LiGetLaunchUrlQueryParameters() -> *const std::ffi::c_char;
    fn LiGetMicroseconds() -> u64;
}

/// Return the launch-query suffix required by the bundled streaming core.
///
/// # Returns
///
/// A process-lifetime string owned by `moonlight-common-c`.
pub fn launch_url_query_parameters() -> &'static CStr {
    // SAFETY: The upstream API returns a pointer to a static NUL-terminated
    // string and documents that callers must not free it.
    unsafe { CStr::from_ptr(LiGetLaunchUrlQueryParameters()) }
}

/// Return elapsed time from the streaming core's opaque monotonic epoch.
///
/// # Returns
///
/// A monotonic duration suitable for relative timing.
pub fn monotonic_time() -> Duration {
    // SAFETY: `LiGetMicroseconds` has no arguments and initializes its internal
    // monotonic epoch on first use.
    Duration::from_micros(unsafe { LiGetMicroseconds() })
}

#[cfg(test)]
mod tests {
    use super::{launch_url_query_parameters, monotonic_time};

    #[test]
    fn exposes_upstream_launch_parameters() {
        assert_eq!(launch_url_query_parameters().to_bytes(), b"&corever=1");
    }

    #[test]
    fn core_clock_is_monotonic() {
        let first = monotonic_time();
        let second = monotonic_time();
        assert!(second >= first);
    }
}
