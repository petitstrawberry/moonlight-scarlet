//! Platform audio renderer selection.

use moonlight_sys::AudioRenderer;

/// Build the audio renderer used by the next stream connection.
pub(crate) fn renderer() -> Option<Box<dyn AudioRenderer>> {
    platform::renderer()
}

#[cfg(not(target_os = "scarlet"))]
mod platform {
    use moonlight_sys::AudioRenderer;

    pub(super) fn renderer() -> Option<Box<dyn AudioRenderer>> {
        None
    }
}

#[cfg(target_os = "scarlet")]
mod platform {
    use std::ptr::NonNull;

    use moonlight_sys::{AudioRenderer, AudioSetup};
    use opus_head_sys::{
        OPUS_OK, OpusMSDecoder, opus_multistream_decode, opus_multistream_decoder_create,
        opus_multistream_decoder_destroy,
    };
    use sas_client::{SasClient, SasStream, StreamConfig};

    const AUDIO_PCM_FORMAT_S16LE: u32 = 1;
    const SAS_PERIOD_MILLISECONDS: u32 = 10;
    const SAS_BUFFER_MILLISECONDS: u32 = 200;
    const OPUS_MAX_FRAME_MILLISECONDS: u32 = 120;

    pub(super) fn renderer() -> Option<Box<dyn AudioRenderer>> {
        Some(Box::new(ScarletAudioRenderer::new()))
    }

    struct ScarletAudioRenderer {
        _client: Option<SasClient>,
        stream: Option<SasStream>,
        decoder: Option<OpusDecoder>,
        pcm: Vec<i16>,
        pcm_bytes: Vec<u8>,
        reported_overflow: bool,
    }

    impl ScarletAudioRenderer {
        fn new() -> Self {
            Self {
                _client: None,
                stream: None,
                decoder: None,
                pcm: Vec::new(),
                pcm_bytes: Vec::new(),
                reported_overflow: false,
            }
        }
    }

    impl AudioRenderer for ScarletAudioRenderer {
        fn initialize(&mut self, setup: AudioSetup) -> Result<(), String> {
            if setup.sample_rate() != 48_000 || setup.channel_count() != 2 {
                return Err(format!(
                    "Scarlet audio requires 48 kHz stereo, got {} Hz and {} channel(s)",
                    setup.sample_rate(),
                    setup.channel_count()
                ));
            }
            let maximum_frame_samples = setup
                .sample_rate()
                .saturating_mul(OPUS_MAX_FRAME_MILLISECONDS)
                / 1_000;
            if setup.samples_per_frame() > maximum_frame_samples {
                return Err(format!(
                    "Opus frame contains too many samples: {}",
                    setup.samples_per_frame()
                ));
            }

            let decoder = OpusDecoder::new(setup)?;
            let mut client = SasClient::connect()
                .map_err(|error| format!("failed to connect to SAS: {}", error.as_str()))?;
            let config = StreamConfig {
                format: AUDIO_PCM_FORMAT_S16LE,
                rate: setup.sample_rate(),
                channels: u16::from(setup.channel_count()),
                period_frames: setup.sample_rate() * SAS_PERIOD_MILLISECONDS / 1_000,
                buffer_frames: setup.sample_rate() * SAS_BUFFER_MILLISECONDS / 1_000,
            };
            let stream = client
                .configure(&config)
                .map_err(|error| format!("failed to configure SAS: {}", error.as_str()))?;
            let sample_capacity = usize::try_from(setup.samples_per_frame())
                .ok()
                .and_then(|frames| frames.checked_mul(usize::from(setup.channel_count())))
                .ok_or_else(|| String::from("Opus PCM buffer size overflow"))?;

            self.pcm.resize(sample_capacity, 0);
            self.pcm_bytes.clear();
            self.pcm_bytes.reserve(sample_capacity.saturating_mul(2));
            self.decoder = Some(decoder);
            self.stream = Some(stream);
            self._client = Some(client);
            self.reported_overflow = false;
            eprintln!(
                "moonlight: Scarlet audio ready Opus {} Hz {}ch frame={} stream(s)={}/{}",
                setup.sample_rate(),
                setup.channel_count(),
                setup.samples_per_frame(),
                setup.streams(),
                setup.coupled_streams()
            );
            Ok(())
        }

        fn decode_and_play(&mut self, packet: Option<&[u8]>) -> Result<(), String> {
            let decoder = self
                .decoder
                .as_mut()
                .ok_or_else(|| String::from("Opus decoder is not initialized"))?;
            let decoded_frames = decoder.decode(packet, &mut self.pcm)?;
            let channels = usize::from(decoder.channel_count);
            let decoded_samples = decoded_frames
                .checked_mul(channels)
                .ok_or_else(|| String::from("decoded Opus sample count overflow"))?;

            self.pcm_bytes.clear();
            for sample in &self.pcm[..decoded_samples] {
                self.pcm_bytes.extend_from_slice(&sample.to_le_bytes());
            }

            let stream = self
                .stream
                .as_mut()
                .ok_or_else(|| String::from("SAS stream is not initialized"))?;
            if stream.is_closed() {
                return Err(String::from("SAS closed the Moonlight audio stream"));
            }
            let written_frames = stream.write(&self.pcm_bytes);
            if written_frames < decoded_frames && !self.reported_overflow {
                eprintln!(
                    "moonlight: SAS ring full; dropped {} audio frame(s)",
                    decoded_frames - written_frames
                );
                self.reported_overflow = true;
            }
            Ok(())
        }
    }

    struct OpusDecoder {
        raw: NonNull<OpusMSDecoder>,
        channel_count: u8,
        samples_per_frame: i32,
    }

    impl OpusDecoder {
        fn new(setup: AudioSetup) -> Result<Self, String> {
            let mut error = OPUS_OK as i32;
            // SAFETY: every scalar and mapping value was validated at the
            // moonlight-sys boundary and libopus copies the mapping at creation.
            let raw = unsafe {
                opus_multistream_decoder_create(
                    setup.sample_rate() as i32,
                    i32::from(setup.channel_count()),
                    i32::from(setup.streams()),
                    i32::from(setup.coupled_streams()),
                    setup.mapping().as_ptr(),
                    &mut error,
                )
            };
            let raw = NonNull::new(raw).ok_or_else(|| {
                format!("failed to create Opus multistream decoder (error {error})")
            })?;
            if error != OPUS_OK as i32 {
                // SAFETY: libopus returned a non-null decoder allocation which
                // must be released even though it also reported an error.
                unsafe { opus_multistream_decoder_destroy(raw.as_ptr()) };
                return Err(format!(
                    "failed to initialize Opus multistream decoder (error {error})"
                ));
            }
            Ok(Self {
                raw,
                channel_count: setup.channel_count(),
                samples_per_frame: setup.samples_per_frame() as i32,
            })
        }

        fn decode(&mut self, packet: Option<&[u8]>, pcm: &mut [i16]) -> Result<usize, String> {
            let (data, length) = match packet {
                Some(packet) => (
                    packet.as_ptr(),
                    i32::try_from(packet.len())
                        .map_err(|_| String::from("Opus packet is too large"))?,
                ),
                None => (std::ptr::null(), 0),
            };
            let required_samples = usize::try_from(self.samples_per_frame)
                .ok()
                .and_then(|frames| frames.checked_mul(usize::from(self.channel_count)))
                .ok_or_else(|| String::from("Opus PCM buffer size overflow"))?;
            if pcm.len() < required_samples {
                return Err(String::from("Opus PCM buffer is too small"));
            }
            // SAFETY: `raw` is an exclusively borrowed live decoder, packet
            // bytes remain valid for this call, and `pcm` has the advertised
            // frame-size times channel-count capacity.
            let decoded = unsafe {
                opus_multistream_decode(
                    self.raw.as_ptr(),
                    data,
                    length,
                    pcm.as_mut_ptr(),
                    self.samples_per_frame,
                    0,
                )
            };
            usize::try_from(decoded)
                .map_err(|_| format!("Opus multistream decode failed with {decoded}"))
        }
    }

    // SAFETY: the decoder is only accessed through `&mut self`; the owning
    // renderer is moved between core threads behind moonlight-sys's mutex.
    unsafe impl Send for OpusDecoder {}

    impl Drop for OpusDecoder {
        fn drop(&mut self) {
            // SAFETY: `raw` was returned by the matching create function and
            // remains uniquely owned until this destructor.
            unsafe { opus_multistream_decoder_destroy(self.raw.as_ptr()) };
        }
    }
}
