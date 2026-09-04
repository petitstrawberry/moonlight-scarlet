//! Safe audio-renderer boundary for `moonlight-common-c` callbacks.

use std::ffi::{c_int, c_uchar};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::slice;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

const MAX_AUDIO_CHANNELS: usize = 8;
const MAX_AUDIO_PACKET_LENGTH: usize = 64 * 1024;

static AUDIO_RENDERER: Mutex<Option<Box<dyn AudioRenderer>>> = Mutex::new(None);
static AUDIO_ERROR: Mutex<Option<String>> = Mutex::new(None);
static AUDIO_FAILED: AtomicBool = AtomicBool::new(false);

/// Opus multistream parameters negotiated by the GameStream core.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioSetup {
    configuration: i32,
    sample_rate: u32,
    channel_count: u8,
    streams: u8,
    coupled_streams: u8,
    samples_per_frame: u32,
    mapping: [u8; MAX_AUDIO_CHANNELS],
}

impl AudioSetup {
    /// Return the raw negotiated GameStream audio configuration.
    ///
    /// # Returns
    ///
    /// The `AUDIO_CONFIGURATION_*` value selected by the host.
    pub const fn configuration(self) -> i32 {
        self.configuration
    }

    /// Return the decoded PCM sample rate.
    ///
    /// # Returns
    ///
    /// Samples per second for each output channel.
    pub const fn sample_rate(self) -> u32 {
        self.sample_rate
    }

    /// Return the decoded PCM channel count.
    ///
    /// # Returns
    ///
    /// Number of interleaved output channels.
    pub const fn channel_count(self) -> u8 {
        self.channel_count
    }

    /// Return the number of coded Opus streams.
    ///
    /// # Returns
    ///
    /// Total independent and coupled Opus streams.
    pub const fn streams(self) -> u8 {
        self.streams
    }

    /// Return the number of stereo-coupled Opus streams.
    ///
    /// # Returns
    ///
    /// Number of coupled streams within [`Self::streams`].
    pub const fn coupled_streams(self) -> u8 {
        self.coupled_streams
    }

    /// Return the expected decoded frame size per channel.
    ///
    /// # Returns
    ///
    /// PCM samples per channel produced for one audio packet.
    pub const fn samples_per_frame(self) -> u32 {
        self.samples_per_frame
    }

    /// Return the Opus output-channel mapping.
    ///
    /// # Returns
    ///
    /// Exactly [`Self::channel_count`] mapping entries.
    pub fn mapping(&self) -> &[u8] {
        &self.mapping[..usize::from(self.channel_count)]
    }

    fn from_raw(
        configuration: c_int,
        sample_rate: c_int,
        channel_count: c_int,
        streams: c_int,
        coupled_streams: c_int,
        samples_per_frame: c_int,
        mapping: *const c_uchar,
    ) -> Result<Self, String> {
        let channel_count = usize::try_from(channel_count)
            .ok()
            .filter(|count| (1..=MAX_AUDIO_CHANNELS).contains(count))
            .ok_or_else(|| String::from("invalid Opus channel count"))?;
        let sample_rate = u32::try_from(sample_rate)
            .ok()
            .filter(|rate| *rate != 0)
            .ok_or_else(|| String::from("invalid Opus sample rate"))?;
        let streams = u8::try_from(streams)
            .ok()
            .filter(|count| *count != 0)
            .ok_or_else(|| String::from("invalid Opus stream count"))?;
        let coupled_streams = u8::try_from(coupled_streams)
            .ok()
            .filter(|count| *count <= streams)
            .ok_or_else(|| String::from("invalid Opus coupled-stream count"))?;
        let samples_per_frame = u32::try_from(samples_per_frame)
            .ok()
            .filter(|count| *count != 0)
            .ok_or_else(|| String::from("invalid Opus frame size"))?;
        if mapping.is_null() {
            return Err(String::from("missing Opus channel mapping"));
        }
        let mut copied_mapping = [0_u8; MAX_AUDIO_CHANNELS];
        // SAFETY: the C bridge passes `opusConfig->mapping`, whose live array
        // contains at least `channel_count` entries for the duration of this call.
        let mapping = unsafe { slice::from_raw_parts(mapping, channel_count) };
        copied_mapping[..channel_count].copy_from_slice(mapping);

        Ok(Self {
            configuration,
            sample_rate,
            channel_count: channel_count as u8,
            streams,
            coupled_streams,
            samples_per_frame,
            mapping: copied_mapping,
        })
    }
}

/// Audio sink invoked serially by `moonlight-common-c`'s decoder worker.
pub trait AudioRenderer: Send {
    /// Initialize the renderer for negotiated Opus multistream audio.
    ///
    /// # Arguments
    ///
    /// * `setup` - Negotiated stream, channel, mapping, and frame parameters.
    ///
    /// # Returns
    ///
    /// Success when the renderer is ready to accept packets.
    fn initialize(&mut self, setup: AudioSetup) -> Result<(), String>;

    /// Decode and present one Opus packet.
    ///
    /// # Arguments
    ///
    /// * `packet` - Encoded bytes, or `None` to request packet-loss concealment.
    ///
    /// # Returns
    ///
    /// Success after the packet was consumed or intentionally dropped.
    fn decode_and_play(&mut self, packet: Option<&[u8]>) -> Result<(), String>;
}

pub(crate) fn install_audio_renderer(renderer: Option<Box<dyn AudioRenderer>>) {
    let previous = {
        let mut installed = lock_renderer();
        std::mem::replace(&mut *installed, renderer)
    };
    drop(previous);
    *lock_error() = None;
    AUDIO_FAILED.store(false, Ordering::Release);
}

pub(crate) fn clear_audio_renderer() {
    let renderer = lock_renderer().take();
    drop(renderer);
}

pub(crate) fn current_audio_error() -> Option<String> {
    lock_error().clone()
}

fn record_audio_error(error: String) {
    AUDIO_FAILED.store(true, Ordering::Release);
    let mut current = lock_error();
    if current.is_none() {
        *current = Some(error);
    }
}

fn lock_renderer() -> MutexGuard<'static, Option<Box<dyn AudioRenderer>>> {
    AUDIO_RENDERER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn lock_error() -> MutexGuard<'static, Option<String>> {
    AUDIO_ERROR
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[unsafe(no_mangle)]
extern "C" fn mls_rust_audio_init(
    configuration: c_int,
    sample_rate: c_int,
    channel_count: c_int,
    streams: c_int,
    coupled_streams: c_int,
    samples_per_frame: c_int,
    mapping: *const c_uchar,
) -> c_int {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let setup = AudioSetup::from_raw(
            configuration,
            sample_rate,
            channel_count,
            streams,
            coupled_streams,
            samples_per_frame,
            mapping,
        )?;
        let mut installed = lock_renderer();
        let Some(renderer) = installed.as_mut() else {
            return Ok(());
        };
        renderer.initialize(setup)
    }));
    match result {
        Ok(Ok(())) => 0,
        Ok(Err(error)) => {
            record_audio_error(error);
            -1
        }
        Err(_) => {
            record_audio_error(String::from(
                "audio renderer panicked during initialization",
            ));
            -1
        }
    }
}

#[unsafe(no_mangle)]
extern "C" fn mls_rust_audio_decode(sample_data: *const c_uchar, sample_length: c_int) {
    if AUDIO_FAILED.load(Ordering::Acquire) {
        return;
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        let packet = if sample_length == 0 {
            None
        } else {
            let length = usize::try_from(sample_length)
                .ok()
                .filter(|length| *length <= MAX_AUDIO_PACKET_LENGTH)
                .ok_or_else(|| String::from("invalid Opus packet length"))?;
            if sample_data.is_null() {
                return Err(String::from("missing Opus packet data"));
            }
            // SAFETY: moonlight-common-c keeps the packet allocation alive for
            // the duration of this synchronous renderer callback.
            Some(unsafe { slice::from_raw_parts(sample_data, length) })
        };
        let mut installed = lock_renderer();
        let Some(renderer) = installed.as_mut() else {
            return Ok(());
        };
        renderer.decode_and_play(packet)
    }));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => record_audio_error(error),
        Err(_) => record_audio_error(String::from("audio renderer panicked while decoding")),
    }
}

#[cfg(test)]
mod tests {
    use super::AudioSetup;

    #[test]
    fn copies_the_negotiated_channel_mapping() {
        let mapping = [0_u8, 1];
        let setup = AudioSetup::from_raw(0, 48_000, 2, 1, 1, 240, mapping.as_ptr())
            .expect("valid stereo Opus setup");

        assert_eq!(setup.sample_rate(), 48_000);
        assert_eq!(setup.channel_count(), 2);
        assert_eq!(setup.streams(), 1);
        assert_eq!(setup.coupled_streams(), 1);
        assert_eq!(setup.samples_per_frame(), 240);
        assert_eq!(setup.mapping(), &[0, 1]);
    }

    #[test]
    fn rejects_an_out_of_range_channel_count() {
        let mapping = [0_u8; 8];
        assert!(AudioSetup::from_raw(0, 48_000, 9, 1, 1, 240, mapping.as_ptr()).is_err());
    }
}
