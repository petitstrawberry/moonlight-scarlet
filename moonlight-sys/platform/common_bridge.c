#include "Limelight.h"

#include <limits.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#define MLS_NO_ERROR INT_MIN

static atomic_int mls_last_stage;
static atomic_int mls_stage_error;
static atomic_int mls_termination_error;
static atomic_bool mls_connection_started;
static atomic_int mls_video_format;
static atomic_int mls_video_width;
static atomic_int mls_video_height;
static atomic_int mls_video_fps;

extern int mls_rust_audio_init(int configuration, int sample_rate,
                               int channel_count, int streams,
                               int coupled_streams, int samples_per_frame,
                               const unsigned char* mapping);
extern void mls_rust_audio_decode(const unsigned char* sample_data,
                                  int sample_length);

static void mls_stage_starting(int stage) {
    atomic_store_explicit(&mls_last_stage, stage, memory_order_release);
}

static void mls_stage_complete(int stage) {
    atomic_store_explicit(&mls_last_stage, stage, memory_order_release);
}

static void mls_stage_failed(int stage, int error_code) {
    atomic_store_explicit(&mls_last_stage, stage, memory_order_release);
    atomic_store_explicit(&mls_stage_error, error_code, memory_order_release);
}

static void mls_connection_started_callback(void) {
    atomic_store_explicit(&mls_connection_started, true, memory_order_release);
}

static void mls_connection_terminated(int error_code) {
    atomic_store_explicit(&mls_termination_error, error_code, memory_order_release);
}

static int mls_video_setup_callback(int video_format, int width, int height,
                                    int redraw_rate, void* context, int flags) {
    (void)context;
    (void)flags;
    atomic_store_explicit(&mls_video_format, video_format, memory_order_release);
    atomic_store_explicit(&mls_video_width, width, memory_order_release);
    atomic_store_explicit(&mls_video_height, height, memory_order_release);
    atomic_store_explicit(&mls_video_fps, redraw_rate, memory_order_release);
    return 0;
}

static int mls_audio_init_callback(
    int audio_configuration,
    const POPUS_MULTISTREAM_CONFIGURATION opus_config,
    void* context,
    int flags) {
    (void)context;
    (void)flags;
    if (opus_config == NULL) {
        return -1;
    }
    return mls_rust_audio_init(
        audio_configuration,
        opus_config->sampleRate,
        opus_config->channelCount,
        opus_config->streams,
        opus_config->coupledStreams,
        opus_config->samplesPerFrame,
        opus_config->mapping);
}

static void mls_audio_start_callback(void) {}

static void mls_audio_stop_callback(void) {}

static void mls_audio_cleanup_callback(void) {}

static void mls_audio_decode_callback(char* sample_data, int sample_length) {
    mls_rust_audio_decode((const unsigned char*)sample_data, sample_length);
}

int mls_start_connection(
    const char* address,
    const char* app_version,
    const char* gfe_version,
    const char* rtsp_session_url,
    int server_codec_mode_support,
    int width,
    int height,
    int fps,
    int bitrate,
    int packet_size,
    int streaming_remotely,
    int audio_configuration,
    int supported_video_formats,
    int client_refresh_rate_x100,
    int color_space,
    int color_range,
    int encryption_flags,
    const unsigned char remote_input_aes_key[16],
    const unsigned char remote_input_aes_iv[16]) {
    SERVER_INFORMATION server;
    STREAM_CONFIGURATION stream;
    CONNECTION_LISTENER_CALLBACKS listener;
    DECODER_RENDERER_CALLBACKS video;
    AUDIO_RENDERER_CALLBACKS audio;

    atomic_store_explicit(&mls_last_stage, STAGE_NONE, memory_order_release);
    atomic_store_explicit(&mls_stage_error, MLS_NO_ERROR, memory_order_release);
    atomic_store_explicit(&mls_termination_error, MLS_NO_ERROR, memory_order_release);
    atomic_store_explicit(&mls_connection_started, false, memory_order_release);
    atomic_store_explicit(&mls_video_format, 0, memory_order_release);
    atomic_store_explicit(&mls_video_width, 0, memory_order_release);
    atomic_store_explicit(&mls_video_height, 0, memory_order_release);
    atomic_store_explicit(&mls_video_fps, 0, memory_order_release);

    LiInitializeServerInformation(&server);
    server.address = address;
    server.serverInfoAppVersion = app_version;
    server.serverInfoGfeVersion = gfe_version;
    server.rtspSessionUrl = rtsp_session_url;
    server.serverCodecModeSupport = server_codec_mode_support;

    LiInitializeStreamConfiguration(&stream);
    stream.width = width;
    stream.height = height;
    stream.fps = fps;
    stream.bitrate = bitrate;
    stream.packetSize = packet_size;
    stream.streamingRemotely = streaming_remotely;
    stream.audioConfiguration = audio_configuration;
    stream.supportedVideoFormats = supported_video_formats;
    stream.clientRefreshRateX100 = client_refresh_rate_x100;
    stream.colorSpace = color_space;
    stream.colorRange = color_range;
    stream.encryptionFlags = encryption_flags;
    memcpy(stream.remoteInputAesKey, remote_input_aes_key,
           sizeof(stream.remoteInputAesKey));
    memcpy(stream.remoteInputAesIv, remote_input_aes_iv,
           sizeof(stream.remoteInputAesIv));

    LiInitializeConnectionCallbacks(&listener);
    listener.stageStarting = mls_stage_starting;
    listener.stageComplete = mls_stage_complete;
    listener.stageFailed = mls_stage_failed;
    listener.connectionStarted = mls_connection_started_callback;
    listener.connectionTerminated = mls_connection_terminated;

    LiInitializeVideoCallbacks(&video);
    video.setup = mls_video_setup_callback;
    video.capabilities = CAPABILITY_PULL_RENDERER;

    LiInitializeAudioCallbacks(&audio);
    audio.init = mls_audio_init_callback;
    audio.start = mls_audio_start_callback;
    audio.stop = mls_audio_stop_callback;
    audio.cleanup = mls_audio_cleanup_callback;
    audio.decodeAndPlaySample = mls_audio_decode_callback;

    return LiStartConnection(&server, &stream, &listener, &video, &audio, NULL, 0,
                             NULL, 0);
}

void mls_stop_connection(void) {
    LiStopConnection();
}

void mls_interrupt_connection(void) {
    LiInterruptConnection();
}

void mls_wake_video_frame(void) {
    LiWakeWaitForVideoFrame();
}

bool mls_wait_video_frame(void** frame_handle, void** decode_unit) {
    VIDEO_FRAME_HANDLE handle = NULL;
    PDECODE_UNIT unit = NULL;
    bool available = LiWaitForNextVideoFrame(&handle, &unit);
    if (!available) {
        *frame_handle = NULL;
        *decode_unit = NULL;
        return false;
    }
    *frame_handle = handle;
    *decode_unit = unit;
    return true;
}

int mls_video_frame_number(const void* decode_unit) {
    return ((const DECODE_UNIT*)decode_unit)->frameNumber;
}

int mls_video_frame_type(const void* decode_unit) {
    return ((const DECODE_UNIT*)decode_unit)->frameType;
}

uint64_t mls_video_frame_presentation_time_us(const void* decode_unit) {
    return ((const DECODE_UNIT*)decode_unit)->presentationTimeUs;
}

int mls_video_frame_full_length(const void* decode_unit) {
    return ((const DECODE_UNIT*)decode_unit)->fullLength;
}

bool mls_video_frame_hdr_active(const void* decode_unit) {
    return ((const DECODE_UNIT*)decode_unit)->hdrActive;
}

uint8_t mls_video_frame_colorspace(const void* decode_unit) {
    return ((const DECODE_UNIT*)decode_unit)->colorspace;
}

int mls_copy_video_frame(const void* decode_unit, unsigned char* destination,
                         size_t destination_length) {
    const DECODE_UNIT* unit = (const DECODE_UNIT*)decode_unit;
    size_t expected;
    size_t offset = 0;

    if (unit == NULL || destination == NULL || unit->fullLength <= 0) {
        return -1;
    }
    expected = (size_t)unit->fullLength;
    if (destination_length < expected) {
        return -1;
    }

    for (const LENTRY* entry = unit->bufferList; entry != NULL;
         entry = entry->next) {
        size_t length;
        if (entry->data == NULL || entry->length <= 0) {
            return -1;
        }
        length = (size_t)entry->length;
        if (length > expected - offset) {
            return -1;
        }
        memcpy(destination + offset, entry->data, length);
        offset += length;
    }

    return offset == expected ? unit->fullLength : -1;
}

void mls_complete_video_frame(void* frame_handle, int decoder_status) {
    LiCompleteVideoFrame(frame_handle, decoder_status);
}

int mls_last_stage_value(void) {
    return atomic_load_explicit(&mls_last_stage, memory_order_acquire);
}

int mls_stage_error_value(void) {
    return atomic_load_explicit(&mls_stage_error, memory_order_acquire);
}

int mls_termination_error_value(void) {
    return atomic_load_explicit(&mls_termination_error, memory_order_acquire);
}

bool mls_connection_started_value(void) {
    return atomic_load_explicit(&mls_connection_started, memory_order_acquire);
}

int mls_video_format_value(void) {
    return atomic_load_explicit(&mls_video_format, memory_order_acquire);
}

int mls_video_width_value(void) {
    return atomic_load_explicit(&mls_video_width, memory_order_acquire);
}

int mls_video_height_value(void) {
    return atomic_load_explicit(&mls_video_height, memory_order_acquire);
}

int mls_video_fps_value(void) {
    return atomic_load_explicit(&mls_video_fps, memory_order_acquire);
}
