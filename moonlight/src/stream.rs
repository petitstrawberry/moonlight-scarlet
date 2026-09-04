//! Streaming transport and platform video consumption.

use moonlight_control::StreamSession;
use moonlight_sys::{
    Connection, ConnectionControl, HostConnectionInfo, StreamConfiguration, VideoFrameStatus,
    VideoSetup,
};

use crate::video::VideoOutput;

/// Start a prepared session and consume video until shutdown is requested.
///
/// # Arguments
///
/// * `session` - Control-plane launch result with RTSP and input-key data.
/// * `video_output` - Destination for decoded video frames.
/// * `started` - Called once with a cancellation handle after transport setup.
/// * `progress` - Receives user-visible transport and video progress updates.
///
/// # Returns
///
/// Success after an orderly stop, or a connection/decoder error.
pub fn run(
    session: &StreamSession,
    video_output: &VideoOutput,
    started: impl FnOnce(ConnectionControl),
    mut progress: impl FnMut(String),
) -> Result<(), String> {
    let host = HostConnectionInfo::new(
        session.endpoint.host(),
        &session.server.app_version,
        session.server.gfe_version.clone(),
        &session.session_url,
        session.server.server_codec_mode_support,
    );
    let stream = StreamConfiguration::new(
        session.config.width,
        session.config.height,
        session.config.fps,
        session.remote_input_aes_key,
        session.remote_input_aes_iv,
    );
    progress(format!(
        "Connecting stream for {}",
        session.application.title
    ));
    let mut connection = Connection::start_with_audio(&host, stream, crate::audio::renderer())
        .map_err(|error| error.to_string())?;
    let setup = connection
        .video_setup()
        .ok_or_else(|| String::from("stream started without negotiated video parameters"))?;
    if !setup.is_h264() {
        return Err(format!(
            "host selected unsupported video format 0x{:x}",
            setup.video_format()
        ));
    }

    let control = connection.control();
    started(control.clone());
    progress(format!(
        "Streaming {}x{} at {} FPS",
        setup.width(),
        setup.height(),
        setup.fps()
    ));

    let video_result = consume_video(
        &mut connection,
        &control,
        setup,
        video_output,
        &mut progress,
    );
    let termination_error = connection.termination_error();
    connection.stop();

    video_result?;
    match termination_error {
        Some(0) | None if control.stop_requested() => Ok(()),
        Some(0) | None => Ok(()),
        Some(error) => Err(format!("stream terminated with error {error}")),
    }
}

#[cfg(not(target_os = "scarlet"))]
fn consume_video(
    connection: &mut Connection,
    control: &ConnectionControl,
    _setup: VideoSetup,
    _video_output: &VideoOutput,
    progress: &mut impl FnMut(String),
) -> Result<(), String> {
    let mut frame_count = 0_u64;
    let mut audio_error_reported = false;
    while !control.stop_requested() {
        let Some(frame) = connection
            .wait_for_video_frame()
            .map_err(|error| error.to_string())?
        else {
            break;
        };

        if frame_count == 0 {
            let access_unit = frame
                .copy_access_unit()
                .map_err(|error| error.to_string())?;
            if !is_annex_b(&access_unit) {
                frame.complete(VideoFrameStatus::NeedIdr);
                return Err(String::from(
                    "first H.264 frame is not an Annex B access unit",
                ));
            }
            progress(String::from(
                "Video transport active (preview is unsupported on this host)",
            ));
        }
        frame_count = frame_count.saturating_add(1);
        frame.complete(VideoFrameStatus::Complete);
        report_audio_error(connection, &mut audio_error_reported, progress);
    }
    Ok(())
}

#[cfg(target_os = "scarlet")]
fn consume_video(
    connection: &mut Connection,
    control: &ConnectionControl,
    setup: VideoSetup,
    video_output: &VideoOutput,
    progress: &mut impl FnMut(String),
) -> Result<(), String> {
    use scarlet_video_client::{
        DecoderOptions, ScarletVideoDecoder, VideoBufferRequest, VideoFormat,
        recommended_input_buffer_len, recommended_nv12_output_buffer_len,
    };

    const MAX_CODED_ACCESS_UNIT: usize = 4 * 1024 * 1024;

    let input_len = recommended_input_buffer_len(MAX_CODED_ACCESS_UNIT)
        .ok_or_else(|| String::from("failed to size the hardware decoder input buffer"))?;
    let output_len = recommended_nv12_output_buffer_len(setup.width(), setup.height())
        .ok_or_else(|| String::from("failed to size the hardware decoder output buffer"))?;
    let mut decoder = ScarletVideoDecoder::open_with_options(
        DecoderOptions::new().with_buffer_request(VideoBufferRequest::new(input_len, output_len)),
    )?;
    decoder.configure(VideoFormat::H264)?;
    progress(String::from("Scarlet H.264 decoder ready"));

    let mut frame_count = 0_u64;
    let mut audio_error_reported = false;
    while !control.stop_requested() {
        let Some(frame) = connection
            .wait_for_video_frame()
            .map_err(|error| error.to_string())?
        else {
            break;
        };
        let presentation_time_us = frame.presentation_time_us();
        let access_unit = match frame.copy_access_unit() {
            Ok(access_unit) => access_unit,
            Err(error) => {
                frame.complete(VideoFrameStatus::NeedIdr);
                return Err(error.to_string());
            }
        };
        if let Err(error) = decoder.submit(&access_unit, presentation_time_us) {
            frame.complete(VideoFrameStatus::NeedIdr);
            return Err(error);
        }
        let decoded = match decoder.dequeue() {
            Ok(decoded) => decoded,
            Err(error) => {
                frame.complete(VideoFrameStatus::NeedIdr);
                return Err(error);
            }
        };

        if let Some(decoded) = decoded {
            if let Err(error) = video_output.present_nv12(
                decoded.width(),
                decoded.height(),
                decoded.timestamp(),
                decoded.payload(),
            ) {
                frame.complete(VideoFrameStatus::NeedIdr);
                return Err(error);
            }
            frame_count = frame_count.saturating_add(1);
            if frame_count == 1 {
                progress(format!(
                    "Decoded first {}x{} NV12 frame",
                    decoded.width(),
                    decoded.height()
                ));
            }
        }
        frame.complete(VideoFrameStatus::Complete);
        report_audio_error(connection, &mut audio_error_reported, progress);
    }
    Ok(())
}

fn report_audio_error(
    connection: &Connection,
    reported: &mut bool,
    progress: &mut impl FnMut(String),
) {
    if *reported {
        return;
    }
    if let Some(error) = connection.audio_error() {
        progress(format!("Audio disabled: {error}"));
        *reported = true;
    }
}

#[cfg(not(target_os = "scarlet"))]
fn is_annex_b(access_unit: &[u8]) -> bool {
    access_unit.starts_with(&[0, 0, 1]) || access_unit.starts_with(&[0, 0, 0, 1])
}

#[cfg(test)]
mod tests {
    #[cfg(not(target_os = "scarlet"))]
    use super::is_annex_b;

    #[cfg(not(target_os = "scarlet"))]
    #[test]
    fn recognizes_three_and_four_byte_annex_b_start_codes() {
        assert!(is_annex_b(&[0, 0, 1, 0x67]));
        assert!(is_annex_b(&[0, 0, 0, 1, 0x67]));
        assert!(!is_annex_b(&[0, 0, 2, 0x67]));
    }
}
