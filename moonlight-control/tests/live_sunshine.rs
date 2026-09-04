//! Opt-in integration checks against a real Sunshine host.

use std::time::Duration;

use moonlight_control::{ConnectProgress, ConnectedHost, GameStreamClient, LaunchConfig};
use moonlight_sys::{Connection, HostConnectionInfo, StreamConfiguration, VideoFrameStatus};

struct HostApplicationGuard<'client> {
    client: &'client GameStreamClient,
    host: &'client ConnectedHost,
}

impl Drop for HostApplicationGuard<'_> {
    fn drop(&mut self) {
        if let Err(error) = self.client.cancel(self.host) {
            eprintln!("moonlight-control: cleanup cancellation failed: {error}");
        }
    }
}

fn configured_host() -> String {
    std::env::var("MOONLIGHT_TEST_HOST")
        .expect("set MOONLIGHT_TEST_HOST to a Sunshine hostname or IP address")
}

#[test]
#[ignore = "requires MOONLIGHT_TEST_HOST and a reachable Sunshine server"]
fn reads_live_server_info() {
    let client = GameStreamClient::load_default().expect("load client identity");
    let (_, info) = client
        .server_info(&configured_host())
        .expect("fetch serverinfo");

    assert!(!info.hostname.is_empty());
    assert_ne!(info.https_port, 0);
}

#[test]
#[ignore = "requires entering the displayed PIN on the Sunshine host"]
fn pairs_and_reads_live_application_list() {
    let client = GameStreamClient::load_default().expect("load client identity");
    let connected = client
        .connect(&configured_host(), |progress| match progress {
            ConnectProgress::WaitingForPin(pin) => {
                eprintln!("MOONLIGHT TEST PIN: {pin}");
            }
            other => eprintln!("moonlight-control: {other:?}"),
        })
        .expect("pair and fetch applist");

    assert!(connected.server.paired);
    eprintln!(
        "moonlight-control: connected to {} with {} application(s)",
        connected.server.hostname,
        connected.applications.len()
    );
    for application in connected.applications {
        eprintln!("{}\t{}", application.id, application.title);
    }
}

#[test]
#[ignore = "launches and cancels a real Sunshine application"]
fn launches_resumes_and_cancels_live_application() {
    let application_id = std::env::var("MOONLIGHT_TEST_APP_ID")
        .expect("set MOONLIGHT_TEST_APP_ID to an application ID advertised by Sunshine")
        .parse::<u32>()
        .expect("MOONLIGHT_TEST_APP_ID must be an integer");
    let client = GameStreamClient::load_default().expect("load client identity");
    let connected = client
        .connect(&configured_host(), |progress| {
            eprintln!("moonlight-control: {progress:?}");
        })
        .expect("connect to paired host");
    assert_eq!(
        connected.server.current_game, 0,
        "the host must be idle before this integration test"
    );
    let application = connected
        .applications
        .iter()
        .find(|application| application.id == application_id)
        .cloned()
        .expect("configured application must be advertised by Sunshine");

    let launched = client
        .start_session(&connected, &application, LaunchConfig::default())
        .expect("launch application");
    let resumed = client.start_session(&connected, &application, LaunchConfig::default());
    let cancelled = client.cancel(&connected);

    eprintln!("launch session URL: {}", launched.session_url);
    assert!(!launched.resumed);
    let resumed = resumed.expect("resume application");
    eprintln!("resume session URL: {}", resumed.session_url);
    assert!(resumed.resumed);
    let server = cancelled.expect("cancel application");
    assert_eq!(server.current_game, 0);
}

#[test]
#[ignore = "launches a real Sunshine stream and consumes compressed video frames"]
fn starts_live_transport_and_pulls_h264_frames() {
    let application_id = std::env::var("MOONLIGHT_TEST_APP_ID")
        .expect("set MOONLIGHT_TEST_APP_ID to an application ID advertised by Sunshine")
        .parse::<u32>()
        .expect("MOONLIGHT_TEST_APP_ID must be an integer");
    let client = GameStreamClient::load_default().expect("load client identity");
    let connected = client
        .connect(&configured_host(), |progress| {
            eprintln!("moonlight-control: {progress:?}");
        })
        .expect("connect to paired host");
    assert_eq!(
        connected.server.current_game, 0,
        "the host must be idle before this integration test"
    );
    let application = connected
        .applications
        .iter()
        .find(|application| application.id == application_id)
        .cloned()
        .expect("configured application must be advertised by Sunshine");
    let session = client
        .start_session(&connected, &application, LaunchConfig::default())
        .expect("launch application");
    let _application_guard = HostApplicationGuard {
        client: &client,
        host: &connected,
    };

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
    let mut connection = Connection::start(&host, stream).expect("start streaming transport");
    assert!(connection.is_started());
    let setup = connection.video_setup().expect("negotiated video setup");
    assert!(setup.is_h264());
    assert_eq!(setup.width(), session.config.width);
    assert_eq!(setup.height(), session.config.height);

    let timeout_control = connection.control();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(15));
        timeout_control.request_stop();
    });

    let mut frame_count = 0;
    while frame_count < 3 {
        let Some(frame) = connection
            .wait_for_video_frame()
            .expect("wait for compressed video")
        else {
            break;
        };
        let access_unit = frame.copy_access_unit().expect("copy access unit");
        assert!(
            access_unit.starts_with(&[0, 0, 1]) || access_unit.starts_with(&[0, 0, 0, 1]),
            "H.264 access unit must use Annex B framing"
        );
        frame.complete(VideoFrameStatus::Complete);
        frame_count += 1;
    }
    connection.stop();

    assert_eq!(frame_count, 3, "expected three compressed video frames");
}
