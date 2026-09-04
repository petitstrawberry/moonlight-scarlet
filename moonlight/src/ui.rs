//! Cross-platform ScarletUI application shell.

use std::thread;

use moonlight_control::{
    Application as StreamApplication, ConnectProgress, ConnectedHost, GameStreamClient,
    LaunchConfig, SavedHosts, SessionPhase, StreamSession,
};
use moonlight_sys::{ConnectionControl, InputError};
use scarlet_ui::prelude::*;
use scarlet_ui::{
    ComponentElement, Event, KeyEvent, MouseButton, MouseEvent, PlatformWindow, RenderElement,
    WindowContext, hstack, vstack, zstack,
};

use crate::input::{RemoteInput, ShortcutDispatch, StreamShortcut};
use crate::video::VideoOutput;

const APP_ID: &str = "org.scarlet-os.moonlight";
const DEFAULT_WINDOW_TITLE: &str = "Moonlight";
const WINDOW_WIDTH: f32 = 960.0;
const WINDOW_HEIGHT: f32 = 720.0;
const TOOLBAR_HEIGHT: f32 = 48.0;
const TOOLBAR_CONTROL_SIZE: f32 = 36.0;
const SETTINGS_CONTENT_HEIGHT: f32 = 550.0;
const LICENSES_CONTENT_HEIGHT: f32 = 775.0;
const LICENSE_LINE_HEIGHT: f32 = 18.0;
const LICENSE_TEXT_VERTICAL_PADDING: f32 = 44.0;
const LICENSE_WRAP_WIDTH: usize = 92;
const BACKGROUND_COLOR: Color = Color::rgb_f32(0.070, 0.070, 0.080);
const SURFACE_COLOR: Color = Color::rgb_f32(0.125, 0.125, 0.145);
const SURFACE_RAISED_COLOR: Color = Color::rgb_f32(0.165, 0.165, 0.190);
const TOOLBAR_COLOR: Color = Color::rgb_f32(63.0 / 255.0, 81.0 / 255.0, 181.0 / 255.0);
const ACCENT_COLOR: Color = Color::rgb_f32(0.612, 0.153, 0.690);
const TEXT_COLOR: Color = Color::WHITE;
const MUTED_TEXT_COLOR: Color = Color::rgb_f32(0.720, 0.730, 0.780);
const BORDER_COLOR: Color = Color::rgb_f32(0.255, 0.265, 0.310);
const ONLINE_COLOR: Color = Color::rgb_f32(0.300, 0.690, 0.390);
const DANGER_COLOR: Color = Color::rgb_f32(0.925, 0.325, 0.310);
const COMPUTERS_PAGE: usize = 0;
const GAMES_PAGE: usize = 1;
const SETTINGS_PAGE: usize = 2;
const ADD_PC_PAGE: usize = 3;
const STREAM_PAGE: usize = 4;
const LICENSES_PAGE: usize = 5;
const LICENSE_DETAIL_PAGE: usize = 6;

/// Run the Moonlight Scarlet application.
///
/// # Returns
///
/// Success after the application exits, or a ScarletUI platform error.
pub fn run() -> scarlet_ui::Result<()> {
    let mut app = MoonlightApp::new();
    let saved_host = match SavedHosts::load_default() {
        Ok(saved) => saved.last_connected().map(str::to_owned),
        Err(error) => {
            eprintln!("moonlight: failed to load saved hosts: {error}");
            None
        }
    };
    if let Some(host) = std::env::args().nth(1).or(saved_host) {
        app.host.set(host);
        app.connect();
    }
    app.run()
}

#[derive(Clone)]
struct MoonlightApp {
    host: State<String>,
    phase: State<SessionPhase>,
    status: State<String>,
    pairing: State<String>,
    applications: State<String>,
    application_items: State<Vec<StreamApplication>>,
    selected_application: State<Option<usize>>,
    connected_host: State<Option<ConnectedHost>>,
    prepared_session: State<Option<StreamSession>>,
    session_details: State<String>,
    stream_control: State<Option<ConnectionControl>>,
    selected_page: State<usize>,
    return_page: State<usize>,
    selected_license: State<usize>,
    video_output: VideoOutput,
    remote_input: RemoteInput,
    stream_input_focused: State<bool>,
    pointer_lock_desired: State<bool>,
    pointer_lock_applied: State<bool>,
    pointer_lock_pending: State<bool>,
    fullscreen_desired: State<bool>,
    fullscreen_applied: State<bool>,
    fullscreen_pending: State<bool>,
    decorations_hidden: State<bool>,
    window_title: State<String>,
    applied_window_title: String,
}

impl MoonlightApp {
    fn new() -> Self {
        Self {
            host: State::new(StateId::new(1), String::new()),
            phase: State::new(StateId::new(2), SessionPhase::Idle),
            status: State::new(StateId::new(3), String::from("Enter a Sunshine host")),
            pairing: State::new(StateId::new(4), String::new()),
            applications: State::new(StateId::new(5), String::new()),
            application_items: State::new(StateId::new(6), Vec::new()),
            selected_application: State::new(StateId::new(7), None),
            connected_host: State::new(StateId::new(8), None),
            prepared_session: State::new(StateId::new(9), None),
            session_details: State::new(StateId::new(10), String::new()),
            selected_page: State::new(StateId::new(11), COMPUTERS_PAGE),
            return_page: State::new(StateId::new(12), COMPUTERS_PAGE),
            selected_license: State::new(StateId::new(23), 0),
            stream_control: State::new(StateId::new(13), None),
            video_output: VideoOutput::new(),
            remote_input: RemoteInput::default(),
            stream_input_focused: State::new(StateId::new(14), false),
            pointer_lock_desired: State::new(StateId::new(15), false),
            pointer_lock_applied: State::new(StateId::new(16), false),
            pointer_lock_pending: State::new(StateId::new(17), false),
            fullscreen_desired: State::new(StateId::new(18), false),
            fullscreen_applied: State::new(StateId::new(19), false),
            fullscreen_pending: State::new(StateId::new(20), false),
            decorations_hidden: State::new(StateId::new(21), false),
            window_title: State::new(StateId::new(22), String::from(DEFAULT_WINDOW_TITLE)),
            applied_window_title: String::from(DEFAULT_WINDOW_TITLE),
        }
    }

    fn connect(&self) {
        let requested_host = self.host.get().trim().to_owned();
        if requested_host.is_empty() {
            self.phase.set(SessionPhase::Idle);
            self.status.set(String::from("Enter a host first"));
            return;
        }
        if matches!(
            self.phase.get(),
            SessionPhase::Connecting | SessionPhase::Pairing
        ) {
            return;
        }

        self.phase.set(SessionPhase::Connecting);
        self.status.set(format!("Querying {requested_host}"));
        self.pairing.set(String::new());
        self.applications.set(String::new());
        self.application_items.set(Vec::new());
        self.selected_application.set(None);
        self.connected_host.set(None);
        self.prepared_session.set(None);
        self.session_details.set(String::new());
        self.window_title.set(String::from(DEFAULT_WINDOW_TITLE));

        let phase = self.phase.clone();
        let status = self.status.clone();
        let pairing = self.pairing.clone();
        let applications = self.applications.clone();
        let application_items = self.application_items.clone();
        let selected_application = self.selected_application.clone();
        let connected_host = self.connected_host.clone();
        let selected_page = self.selected_page.clone();
        let window_title = self.window_title.clone();
        thread::spawn(move || {
            let result = GameStreamClient::load_default().and_then(|client| {
                client.connect(&requested_host, |progress| match progress {
                    ConnectProgress::FetchingServerInfo => {
                        phase.set(SessionPhase::Connecting);
                        status.set(format!("Contacting {requested_host}"));
                    }
                    ConnectProgress::WaitingForPin(pin) => {
                        phase.set(SessionPhase::Pairing);
                        pairing.set(format!("Enter PIN {pin} on the Sunshine host"));
                        status.set(String::from("Waiting for Sunshine to accept pairing"));
                    }
                    ConnectProgress::PairingStage(stage) => {
                        phase.set(SessionPhase::Pairing);
                        status.set(format!("Pairing stage {stage}/5"));
                    }
                    ConnectProgress::FetchingApplications => {
                        status.set(String::from("Loading applications"));
                    }
                })
            });

            match result {
                Ok(connected) => {
                    let app_count = connected.applications.len();
                    let hostname = connected.server.hostname.clone();
                    let running_title = connected
                        .applications
                        .iter()
                        .find(|application| application.id == connected.server.current_game)
                        .map(|application| application.title.as_str());
                    eprintln!(
                        "moonlight: connected to {} with {app_count} application(s)",
                        hostname
                    );
                    if let Err(error) = remember_connected_host(&requested_host) {
                        eprintln!("moonlight: failed to save connected host: {error}");
                    }
                    let app_text = if connected.applications.is_empty() {
                        String::from("No applications advertised by Sunshine")
                    } else {
                        connected
                            .applications
                            .iter()
                            .map(|application| {
                                format!("{}  ·  {}", application.id, application.title)
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    };
                    pairing.set(String::new());
                    applications.set(app_text);
                    application_items.set(connected.applications.clone());
                    selected_application.set(None);
                    window_title.set(format_window_title(running_title));
                    connected_host.set(Some(connected));
                    status.set(format!(
                        "Connected to {} — {app_count} application(s)",
                        hostname
                    ));
                    phase.set(SessionPhase::Ready);
                    selected_page.set(COMPUTERS_PAGE);
                }
                Err(error) => {
                    let message = error.to_string();
                    eprintln!("moonlight: {message}");
                    status.set(message.clone());
                    phase.set(SessionPhase::Failed(message));
                }
            }
        });
    }

    fn start_selected_application(&self) {
        if matches!(
            self.phase.get(),
            SessionPhase::Connecting
                | SessionPhase::Pairing
                | SessionPhase::Launching
                | SessionPhase::Streaming
        ) {
            return;
        }
        let Some(connected) = self.connected_host.get() else {
            self.status.set(String::from("Connect to a host first"));
            return;
        };
        let Some(selected_index) = self.selected_application.get() else {
            self.status.set(String::from("Select an application first"));
            return;
        };
        let Some(application) = self.application_items.get().get(selected_index).cloned() else {
            self.status.set(String::from("Select an application first"));
            return;
        };

        self.phase.set(SessionPhase::Launching);
        self.status.set(format!("Preparing {}", application.title));
        self.session_details.set(String::new());
        self.video_output.reset();
        self.remote_input.reset();
        self.stream_input_focused.set(false);
        self.pointer_lock_desired.set(false);

        let phase = self.phase.clone();
        let status = self.status.clone();
        let connected_host = self.connected_host.clone();
        let prepared_session = self.prepared_session.clone();
        let session_details = self.session_details.clone();
        let stream_control = self.stream_control.clone();
        let selected_page = self.selected_page.clone();
        let video_output = self.video_output.clone();
        let stream_ui = self.clone();
        let window_title = self.window_title.clone();
        thread::spawn(move || {
            let result = GameStreamClient::load_default().and_then(|client| {
                client.start_session(&connected, &application, LaunchConfig::default())
            });

            match result {
                Ok(session) => {
                    let action = if session.resumed {
                        "resumed"
                    } else {
                        "launched"
                    };
                    eprintln!(
                        "moonlight: {action} {}; session URL {}",
                        session.application.title, session.session_url
                    );
                    window_title.set(format_window_title(Some(&session.application.title)));
                    let mut updated_host = connected;
                    updated_host.server = session.server.clone();
                    connected_host.set(Some(updated_host));
                    session_details.set(session.session_url.clone());
                    prepared_session.set(Some(session.clone()));

                    let started_control = stream_control.clone();
                    let streaming_phase = phase.clone();
                    let streaming_page = selected_page.clone();
                    let progress_status = status.clone();
                    let stream_result = crate::stream::run(
                        &session,
                        &video_output,
                        move |control| {
                            started_control.set(Some(control));
                            streaming_phase.set(SessionPhase::Streaming);
                            streaming_page.set(STREAM_PAGE);
                        },
                        move |message| {
                            eprintln!("moonlight: {message}");
                            progress_status.set(message);
                        },
                    );
                    stream_control.set(None);
                    stream_ui.reset_stream_window_state();

                    match stream_result {
                        Ok(()) if matches!(phase.get(), SessionPhase::Streaming) => {
                            status.set(format!(
                                "Stream stopped; {} remains open on the host",
                                session.application.title
                            ));
                            phase.set(SessionPhase::SessionPrepared);
                            selected_page.set(GAMES_PAGE);
                        }
                        Ok(()) => {}
                        Err(message) => {
                            eprintln!("moonlight: {message}");
                            status.set(message.clone());
                            phase.set(SessionPhase::Failed(message));
                            selected_page.set(GAMES_PAGE);
                        }
                    }
                }
                Err(error) => {
                    let message = error.to_string();
                    eprintln!("moonlight: {message}");
                    status.set(message.clone());
                    phase.set(SessionPhase::Failed(message));
                }
            }
        });
    }

    fn cancel_host_application(&self) {
        if matches!(
            self.phase.get(),
            SessionPhase::Connecting | SessionPhase::Pairing | SessionPhase::Launching
        ) {
            return;
        }
        let Some(mut connected) = self.connected_host.get() else {
            self.status.set(String::from("Connect to a host first"));
            return;
        };

        self.leave_stream_mode();
        if let Some(control) = self.stream_control.get() {
            control.request_stop();
            self.stream_control.set(None);
        }

        self.phase.set(SessionPhase::Launching);
        self.status
            .set(String::from("Stopping the host application"));
        self.selected_page.set(GAMES_PAGE);

        let phase = self.phase.clone();
        let status = self.status.clone();
        let connected_host = self.connected_host.clone();
        let prepared_session = self.prepared_session.clone();
        let session_details = self.session_details.clone();
        let selected_page = self.selected_page.clone();
        let window_title = self.window_title.clone();
        thread::spawn(move || {
            let result =
                GameStreamClient::load_default().and_then(|client| client.cancel(&connected));
            match result {
                Ok(server) => {
                    eprintln!("moonlight: host application stopped");
                    connected.server = server;
                    connected_host.set(Some(connected));
                    prepared_session.set(None);
                    session_details.set(String::new());
                    window_title.set(String::from(DEFAULT_WINDOW_TITLE));
                    status.set(String::from("Host application stopped"));
                    phase.set(SessionPhase::Ready);
                    selected_page.set(GAMES_PAGE);
                }
                Err(error) => {
                    let message = error.to_string();
                    eprintln!("moonlight: {message}");
                    status.set(message.clone());
                    phase.set(SessionPhase::Failed(message));
                }
            }
        });
    }

    fn open_games(&self) {
        if self.connected_host.get().is_some() {
            self.selected_page.set(GAMES_PAGE);
        } else {
            self.open_add_pc();
        }
    }

    fn open_add_pc(&self) {
        self.return_page.set(COMPUTERS_PAGE);
        self.selected_page.set(ADD_PC_PAGE);
    }

    fn open_settings(&self) {
        self.return_page.set(self.selected_page.get());
        self.selected_page.set(SETTINGS_PAGE);
    }

    fn open_licenses(&self) {
        self.selected_page.set(LICENSES_PAGE);
    }

    fn open_license(&self, index: usize) {
        self.selected_license.set(index);
        self.selected_page.set(LICENSE_DETAIL_PAGE);
    }

    fn go_back(&self) {
        let destination = match self.selected_page.get() {
            GAMES_PAGE => COMPUTERS_PAGE,
            STREAM_PAGE => GAMES_PAGE,
            LICENSE_DETAIL_PAGE => LICENSES_PAGE,
            LICENSES_PAGE => SETTINGS_PAGE,
            SETTINGS_PAGE | ADD_PC_PAGE => self.return_page.get(),
            _ => COMPUTERS_PAGE,
        };
        self.selected_page.set(destination);
    }

    fn stop_stream(&self) {
        self.leave_stream_mode();
        if let Some(control) = self.stream_control.get() {
            control.request_stop();
            self.status.set(String::from("Disconnecting stream"));
        }
        self.selected_page.set(GAMES_PAGE);
    }

    fn request_pointer_lock(&self) {
        if self.stream_control.get().is_none() {
            return;
        }
        self.pointer_lock_desired.set(true);
        self.status.set(String::from("Requesting input capture…"));
    }

    fn release_pointer_lock(&self) {
        let control = self.stream_control.get();
        if let Err(error) = self.remote_input.release_all(control.as_ref()) {
            report_input_error(error);
        }
        self.pointer_lock_desired.set(false);
        self.status.set(String::from(
            "Input released · Ctrl+Alt+Shift+Z to capture again",
        ));
    }

    fn toggle_pointer_lock(&self) {
        if self.pointer_lock_desired.get() || self.pointer_lock_applied.get() {
            self.release_pointer_lock();
        } else {
            self.request_pointer_lock();
        }
    }

    fn toggle_fullscreen(&self) {
        self.fullscreen_desired.set(!self.fullscreen_desired.get());
    }

    fn leave_stream_mode(&self) {
        let control = self.stream_control.get();
        if let Err(error) = self.remote_input.release_all(control.as_ref()) {
            report_input_error(error);
        }
        self.pointer_lock_desired.set(false);
        self.fullscreen_desired.set(false);
        self.stream_input_focused.set(false);
    }

    fn reset_stream_window_state(&self) {
        self.remote_input.reset();
        self.pointer_lock_desired.set(false);
        self.fullscreen_desired.set(false);
        self.stream_input_focused.set(false);
    }

    fn handle_stream_key(&self, event: KeyEvent) -> bool {
        match self.remote_input.classify_shortcut(event) {
            ShortcutDispatch::Command(StreamShortcut::ToggleCapture) => {
                self.toggle_pointer_lock();
                return true;
            }
            ShortcutDispatch::Command(StreamShortcut::Disconnect) => {
                self.stop_stream();
                return true;
            }
            ShortcutDispatch::Command(StreamShortcut::ToggleFullscreen) => {
                self.toggle_fullscreen();
                return true;
            }
            ShortcutDispatch::Consumed => return true,
            ShortcutDispatch::NotShortcut => {}
        }
        if !self.pointer_lock_applied.get() {
            return false;
        }
        let Some(control) = self.stream_control.get() else {
            return false;
        };
        match self.remote_input.send_key(&control, event) {
            Ok(handled) => handled,
            Err(error) => {
                report_input_error(error);
                true
            }
        }
    }

    fn handle_stream_mouse_delta(&self, delta_x: i32, delta_y: i32) {
        if !self.pointer_lock_applied.get() {
            return;
        }
        let Some(control) = self.stream_control.get() else {
            return;
        };
        if let Err(error) = self
            .remote_input
            .send_mouse_delta(&control, delta_x, delta_y)
        {
            report_input_error(error);
        }
    }

    fn handle_stream_mouse_button(&self, button: MouseButton, pressed: bool) -> bool {
        if !pressed && self.remote_input.consume_suppressed_mouse_release(button) {
            return true;
        }
        if !self.pointer_lock_applied.get() {
            if pressed {
                self.remote_input.suppress_capture_click(button);
                self.request_pointer_lock();
                return true;
            }
            return false;
        }
        let Some(control) = self.stream_control.get() else {
            return false;
        };
        if let Err(error) = self
            .remote_input
            .send_mouse_button(&control, button, pressed)
        {
            report_input_error(error);
        }
        true
    }

    fn handle_stream_event(&self, event: &Event) -> bool {
        if !self.pointer_lock_applied.get() {
            return false;
        }
        let Event::Mouse(MouseEvent::Wheel {
            delta_x, delta_y, ..
        }) = event
        else {
            return false;
        };
        let Some(control) = self.stream_control.get() else {
            return false;
        };
        if let Err(error) = self.remote_input.send_wheel(&control, *delta_x, *delta_y) {
            report_input_error(error);
        }
        true
    }

    fn start_application(&self, index: usize) {
        self.selected_application.set(Some(index));
        self.start_selected_application();
    }

    fn computers_screen(&self) -> impl View + Clone + use<> {
        let add_pc = self.clone();
        let settings = self.clone();
        let toolbar = moonlight_toolbar(
            String::from("Computers"),
            IconView::new(Icon::Moon)
                .size(IconSize::Large)
                .filled()
                .color(TEXT_COLOR)
                .frame(TOOLBAR_CONTROL_SIZE, TOOLBAR_CONTROL_SIZE),
            hstack! {
                Button::new("Add PC")
                    .icon(Icon::Plus)
                    .icon_color(TEXT_COLOR)
                    .header_style()
                    .text_color(TEXT_COLOR)
                    .font_size(14.0)
                    .on_click(move || add_pc.open_add_pc()),
                Button::icon_only(Icon::Settings)
                    .icon_color(TEXT_COLOR)
                    .header_style()
                    .on_click(move || settings.open_settings()),
            }
            .spacing(8.0),
        );

        MoonlightScreen::new(toolbar, self.computers_body())
    }

    fn computers_body(&self) -> impl View + Clone + use<> {
        if let Some(host) = self.connected_host.get() {
            let open_host = self.clone();
            Either::A(
                zstack! {
                    Rectangle::new()
                        .fill(BACKGROUND_COLOR)
                        .frame(f32::INFINITY, f32::INFINITY),
                    vstack! {
                        vstack! {
                            IconView::new(Icon::DeviceDesktop)
                                .size(IconSize::Pixels(150))
                                .weight(IconWeight::Thin)
                                .color(TEXT_COLOR),
                            Text::new(host.server.hostname)
                                .font_size(27.0)
                                .color(TEXT_COLOR),
                            hstack! {
                                Rectangle::new()
                                    .fill(ONLINE_COLOR)
                                    .frame(9.0, 9.0)
                                    .clip_radius(5.0),
                                Text::new("Online · Paired")
                                    .font_size(13.0)
                                    .color(MUTED_TEXT_COLOR),
                            }
                            .spacing(7.0),
                            Text::new(host.endpoint.host().to_owned())
                                .font_size(12.0)
                                .color(MUTED_TEXT_COLOR),
                        }
                        .alignment(Alignment::Center)
                        .spacing(10.0)
                        .padding(18.0)
                        .frame(310.0, 330.0)
                        .background(SURFACE_COLOR)
                        .clip_radius(8.0)
                        .border_rounded(BORDER_COLOR, 1.0, 8.0)
                        .on_click(move || open_host.open_games())
                        .repaint_boundary(),
                        StatusLine::new(self, 13.0),
                    }
                    .alignment(Alignment::Center)
                    .spacing(16.0),
                }
                .alignment(Alignment::Center),
            )
        } else {
            let add_pc = self.clone();
            Either::B(
                zstack! {
                    Rectangle::new()
                        .fill(BACKGROUND_COLOR)
                        .frame(f32::INFINITY, f32::INFINITY),
                    vstack! {
                        IconView::new(Icon::DeviceDesktop)
                            .size(IconSize::Pixels(112))
                            .weight(IconWeight::Thin)
                            .color(MUTED_TEXT_COLOR),
                        ConnectionHeading::new(self.phase.clone()),
                        StatusLine::new(self, 14.0),
                        PairingLine::new(self.pairing.clone()),
                        Button::new("Add PC manually")
                            .icon(Icon::Plus)
                            .background_color(ACCENT_COLOR)
                            .text_color(TEXT_COLOR)
                            .icon_color(TEXT_COLOR)
                            .font_size(15.0)
                            .padding(11.0)
                            .on_click(move || add_pc.open_add_pc()),
                    }
                    .alignment(Alignment::Center)
                    .spacing(15.0),
                }
                .alignment(Alignment::Center),
            )
        }
    }

    fn games_screen(&self) -> impl View + Clone + use<> {
        let hostname = self
            .connected_host
            .get()
            .map_or_else(|| String::from("Applications"), |host| host.server.hostname);
        let current_game = self
            .connected_host
            .get()
            .map(|host| host.server.current_game)
            .unwrap_or(0);
        let back = self.clone();
        let quit = self.clone();
        let settings = self.clone();
        let quit_button = if current_game == 0 {
            Either::A(Spacer::new().frame(0.0, 40.0))
        } else {
            Either::B(
                Button::new("Quit Game")
                    .icon(Icon::Power)
                    .icon_color(TEXT_COLOR)
                    .header_style()
                    .text_color(TEXT_COLOR)
                    .font_size(14.0)
                    .on_click(move || quit.cancel_host_application()),
            )
        };
        let toolbar = moonlight_toolbar(
            hostname,
            Button::icon_only(Icon::ArrowLeft)
                .icon_color(TEXT_COLOR)
                .header_style()
                .on_click(move || back.go_back()),
            hstack! {
                quit_button,
                Button::icon_only(Icon::Settings)
                    .icon_color(TEXT_COLOR)
                    .header_style()
                    .on_click(move || settings.open_settings()),
            }
            .spacing(8.0),
        );

        MoonlightScreen::new(toolbar, self.games_body())
    }

    fn games_body(&self) -> impl View + Clone + use<> {
        let current_game = self
            .connected_host
            .get()
            .map(|host| host.server.current_game)
            .unwrap_or(0);
        let app = self.clone();
        let grid = GridView::new(
            self.application_items.clone(),
            self.selected_application.clone(),
            4,
            288.0,
            move |index, application, selected| {
                let is_running = current_game == application.id;
                let is_selected = selected == Some(index);
                let launch = app.clone();
                vstack! {
                    zstack! {
                        Rectangle::new()
                            .fill(placeholder_color(index))
                            .frame(198.0, 238.0)
                            .clip_radius(4.0),
                        vstack! {
                            IconView::new(if application.title.eq_ignore_ascii_case("Desktop") {
                                Icon::DeviceDesktop
                            } else {
                                Icon::Apps
                            })
                            .size(IconSize::Pixels(82))
                            .weight(IconWeight::Thin)
                            .color(TEXT_COLOR),
                            Text::new(application.title)
                                .font_size(19.0)
                                .color(TEXT_COLOR),
                        }
                        .alignment(Alignment::Center)
                        .spacing(18.0),
                    }
                    .frame(202.0, 242.0)
                    .border_rounded(
                        if is_selected { ACCENT_COLOR } else { Color::CLEAR },
                        if is_selected { 3.0 } else { 0.0 },
                        5.0,
                    ),
                    Text::new(if is_running {
                        "RUNNING · CLICK TO RESUME"
                    } else {
                        "CLICK TO STREAM"
                    })
                    .font_size(10.0)
                    .color(if is_running { TEXT_COLOR } else { MUTED_TEXT_COLOR }),
                }
                .alignment(Alignment::Center)
                .spacing(8.0)
                .frame(f32::INFINITY, 280.0)
                .on_click(move || launch.start_application(index))
            },
        )
        .minimum_cell_width(210.0)
        .spacing(12.0)
        .row_spacing(12.0)
        .padding(20.0)
        .frame(f32::INFINITY, f32::INFINITY)
        .repaint_boundary();

        zstack! {
            Rectangle::new()
                .fill(BACKGROUND_COLOR)
                .frame(f32::INFINITY, f32::INFINITY),
            grid,
            StatusLine::new(self, 12.0)
                .padding(8.0)
                .background(SURFACE_COLOR)
                .clip_radius(4.0),
        }
        .alignment(Alignment::Bottom)
    }

    fn add_pc_screen(&self) -> impl View + Clone + use<> {
        let back = self.clone();
        let connect = self.clone();
        let toolbar = moonlight_toolbar(
            String::from("Add PC"),
            Button::icon_only(Icon::ArrowLeft)
                .icon_color(TEXT_COLOR)
                .header_style()
                .on_click(move || back.go_back()),
            Spacer::new().frame(TOOLBAR_CONTROL_SIZE, TOOLBAR_CONTROL_SIZE),
        );
        let body = zstack! {
            Rectangle::new()
                .fill(BACKGROUND_COLOR)
                .frame(f32::INFINITY, f32::INFINITY),
            vstack! {
                Text::new("Enter the IP address of your host PC")
                    .font_size(21.0)
                    .color(TEXT_COLOR),
                Text::new("Moonlight will connect and pair with Sunshine.")
                    .font_size(13.0)
                    .color(MUTED_TEXT_COLOR),
                hstack! {
                    TextField::new(self.host.clone())
                        .placeholder("192.168.1.100")
                        .background_color(SURFACE_RAISED_COLOR)
                        .border_color(BORDER_COLOR)
                        .focused_border_color(ACCENT_COLOR)
                        .text_color(TEXT_COLOR)
                        .frame_width(430.0),
                    Button::new("Add")
                        .background_color(ACCENT_COLOR)
                        .text_color(TEXT_COLOR)
                        .font_size(15.0)
                        .padding(11.0)
                        .on_click(move || connect.connect()),
                }
                .alignment(Alignment::Center)
                .spacing(12.0),
                StatusLine::new(self, 13.0),
                PairingLine::new(self.pairing.clone()),
            }
            .alignment(Alignment::Leading)
            .spacing(12.0)
            .padding(24.0)
            .frame(620.0, 245.0)
            .background(SURFACE_COLOR)
            .clip_radius(8.0)
            .border_rounded(BORDER_COLOR, 1.0, 8.0),
        }
        .alignment(Alignment::Center);

        MoonlightScreen::new(toolbar, body)
    }

    fn settings_screen(&self) -> impl View + Clone + use<> {
        let back = self.clone();
        let licenses = self.clone();
        let toolbar = moonlight_toolbar(
            String::from("Settings"),
            Button::icon_only(Icon::ArrowLeft)
                .icon_color(TEXT_COLOR)
                .header_style()
                .on_click(move || back.go_back()),
            Spacer::new().frame(TOOLBAR_CONTROL_SIZE, TOOLBAR_CONTROL_SIZE),
        );
        let body = ScrollView::new(
            vstack! {
                Text::new("STREAM SETTINGS")
                    .font_size(12.0)
                    .color(MUTED_TEXT_COLOR),
                settings_row("Resolution", "1920 × 1080"),
                settings_row("Frame rate", "60 FPS"),
                settings_row("Video codec", "H.264"),
                settings_row("Audio", "Stereo · 48 kHz"),
                Text::new("ABOUT THIS BUILD")
                    .font_size(12.0)
                    .color(MUTED_TEXT_COLOR),
                settings_row("Control plane", "macOS and Scarlet"),
                settings_row("Video decoding", platform_video_summary()),
                settings_link_row("Open source licenses", "View", move || {
                    licenses.open_licenses()
                }),
            }
            .alignment(Alignment::Leading)
            .spacing(10.0)
            .padding(28.0)
            .frame(f32::INFINITY, f32::INFINITY)
            .background(BACKGROUND_COLOR),
        )
        .vertical()
        .content_size(0.0, SETTINGS_CONTENT_HEIGHT)
        .frame(f32::INFINITY, f32::INFINITY);

        MoonlightScreen::new(toolbar, body)
    }

    fn licenses_screen(&self) -> impl View + Clone + use<> {
        let back = self.clone();
        let toolbar = moonlight_toolbar(
            String::from("Open Source Licenses"),
            Button::icon_only(Icon::ArrowLeft)
                .icon_color(TEXT_COLOR)
                .header_style()
                .on_click(move || back.go_back()),
            Spacer::new().frame(TOOLBAR_CONTROL_SIZE, TOOLBAR_CONTROL_SIZE),
        );
        let body = ScrollView::new(
            vstack! {
                Text::new("LICENSES AND ATTRIBUTIONS")
                    .font_size(12.0)
                    .color(MUTED_TEXT_COLOR),
                Text::new(
                    "Core native components and Scarlet libraries used by this build."
                )
                .font_size(13.0)
                .color(MUTED_TEXT_COLOR),
                vstack! {
                    license_navigation_row(self, 0),
                    license_navigation_row(self, 1),
                    license_navigation_row(self, 2),
                    license_navigation_row(self, 3),
                    license_navigation_row(self, 4),
                    license_navigation_row(self, 5),
                    license_navigation_row(self, 6),
                    license_navigation_row(self, 7),
                    license_navigation_row(self, 8),
                }
                .alignment(Alignment::Leading)
                .spacing(10.0),
            }
            .alignment(Alignment::Leading)
            .spacing(10.0)
            .padding(22.0)
            .frame(f32::INFINITY, f32::INFINITY)
            .background(BACKGROUND_COLOR),
        )
        .vertical()
        .content_size(0.0, LICENSES_CONTENT_HEIGHT)
        .frame(f32::INFINITY, f32::INFINITY);

        MoonlightScreen::new(toolbar, body)
    }

    fn license_detail_screen(&self) -> impl View + Clone + use<> {
        let selected_license = self.selected_license.get();
        let notice = crate::licenses::notice(selected_license);
        let lines = crate::licenses::display_lines(selected_license, LICENSE_WRAP_WIDTH);
        let line_count = lines.len();
        let content_height =
            line_count as f32 * LICENSE_LINE_HEIGHT + LICENSE_TEXT_VERTICAL_PADDING;
        let back = self.clone();
        let toolbar = moonlight_toolbar(
            notice.title.to_owned(),
            Button::icon_only(Icon::ArrowLeft)
                .icon_color(TEXT_COLOR)
                .header_style()
                .on_click(move || back.go_back()),
            Spacer::new().frame(TOOLBAR_CONTROL_SIZE, TOOLBAR_CONTROL_SIZE),
        );
        let body = ScrollView::new(
            LazyVStack::new(line_count, LICENSE_LINE_HEIGHT, move |index| {
                Text::new(lines.get(index).cloned().unwrap_or_default())
                    .font_size(11.0)
                    .color(if index < 3 {
                        TEXT_COLOR
                    } else {
                        MUTED_TEXT_COLOR
                    })
                    .frame(f32::INFINITY, LICENSE_LINE_HEIGHT)
            })
            .padding(22.0)
            .background(BACKGROUND_COLOR),
        )
        .vertical()
        .content_size(0.0, content_height)
        .frame(f32::INFINITY, f32::INFINITY);

        MoonlightScreen::new(toolbar, body)
    }

    fn stream_screen(&self) -> impl View + Clone + use<> {
        let title = self.prepared_session.get().map_or_else(
            || String::from("Moonlight"),
            |session| session.application.title,
        );
        let disconnect = self.clone();
        let quit = self.clone();
        let capture = self.clone();
        let fullscreen = self.clone();
        let key_input = self.clone();
        let pointer_input = self.clone();
        let button_input = self.clone();
        let wheel_input = self.clone();
        let pointer_locked = self.pointer_lock_applied.get();
        let fullscreen_enabled = self.fullscreen_desired.get();
        let overlay = if pointer_locked {
            Either::A(Spacer::new().frame(0.0, 0.0))
        } else {
            Either::B(
                hstack! {
                    Button::icon_only(Icon::ArrowLeft)
                        .icon_color(TEXT_COLOR)
                        .header_style()
                        .on_click(move || disconnect.stop_stream()),
                    Text::new(title).font_size(16.0).color(TEXT_COLOR),
                    Text::new("Ctrl+Alt+Shift+Z · Release input")
                        .font_size(11.0)
                        .color(MUTED_TEXT_COLOR),
                    Spacer::new(),
                    Button::new("Capture Input")
                        .header_style()
                        .text_color(TEXT_COLOR)
                        .font_size(12.0)
                        .on_click(move || capture.request_pointer_lock()),
                    Button::new(if fullscreen_enabled {
                        "Windowed"
                    } else {
                        "Fullscreen"
                    })
                    .header_style()
                    .text_color(TEXT_COLOR)
                    .font_size(12.0)
                    .on_click(move || fullscreen.toggle_fullscreen()),
                    Button::new("Quit Game")
                        .icon(Icon::Power)
                        .icon_color(TEXT_COLOR)
                        .header_style()
                        .text_color(TEXT_COLOR)
                        .font_size(12.0)
                        .on_click(move || quit.cancel_host_application()),
                }
                .alignment(Alignment::Center)
                .spacing(8.0)
                .padding(6.0)
                .frame(f32::INFINITY, 48.0)
                .background(Color::rgba_f32(0.055, 0.055, 0.070, 0.94)),
            )
        };
        let video = self
            .video_output
            .view()
            .on_event(move |event| wheel_input.handle_stream_event(event));

        zstack! {
            Rectangle::new()
                .fill(Color::BLACK)
                .frame(f32::INFINITY, f32::INFINITY),
            video,
            overlay,
        }
        .alignment(Alignment::Top)
        .frame(f32::INFINITY, f32::INFINITY)
        .on_mouse_delta(move |delta_x, delta_y| {
            pointer_input.handle_stream_mouse_delta(delta_x, delta_y)
        })
        .on_mouse_button(move |button, pressed| {
            button_input.handle_stream_mouse_button(button, pressed)
        })
        .focusable(self.stream_input_focused.clone())
        .on_key(move |event| key_input.handle_stream_key(event))
    }

    fn content(&self) -> Box<dyn View> {
        Box::new(AppPage::new(self.clone(), self.selected_page.get()))
    }
}

impl View for MoonlightApp {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(ComponentElement::new_with_builder(
            self.clone(),
            build_content,
        ))
    }

    fn listenables(&self) -> Vec<&dyn scarlet_ui::Listenable> {
        vec![
            &self.selected_page as &dyn scarlet_ui::Listenable,
            &self.decorations_hidden as &dyn scarlet_ui::Listenable,
            &self.window_title as &dyn scarlet_ui::Listenable,
        ]
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

#[derive(Clone)]
struct AppPage {
    app: MoonlightApp,
    page: usize,
}

impl AppPage {
    fn new(app: MoonlightApp, page: usize) -> Self {
        Self { app, page }
    }
}

impl View for AppPage {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(ComponentElement::new_with_builder(
            self.clone(),
            build_app_page,
        ))
    }

    fn listenables(&self) -> Vec<&dyn scarlet_ui::Listenable> {
        match self.page {
            COMPUTERS_PAGE | GAMES_PAGE => {
                vec![&self.app.connected_host as &dyn scarlet_ui::Listenable]
            }
            STREAM_PAGE => vec![
                &self.app.pointer_lock_applied as &dyn scarlet_ui::Listenable,
                &self.app.fullscreen_desired as &dyn scarlet_ui::Listenable,
            ],
            LICENSE_DETAIL_PAGE => {
                vec![&self.app.selected_license as &dyn scarlet_ui::Listenable]
            }
            _ => Vec::new(),
        }
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

fn build_app_page(page: &AppPage) -> Box<dyn View> {
    match page.page {
        GAMES_PAGE => Box::new(page.app.games_screen()),
        SETTINGS_PAGE => Box::new(page.app.settings_screen()),
        ADD_PC_PAGE => Box::new(page.app.add_pc_screen()),
        STREAM_PAGE => Box::new(page.app.stream_screen()),
        LICENSES_PAGE => Box::new(page.app.licenses_screen()),
        LICENSE_DETAIL_PAGE => Box::new(page.app.license_detail_screen()),
        _ => Box::new(page.app.computers_screen()),
    }
}

#[derive(Clone)]
struct StatusLine {
    phase: State<SessionPhase>,
    status: State<String>,
    font_size: f32,
}

impl StatusLine {
    fn new(app: &MoonlightApp, font_size: f32) -> Self {
        Self {
            phase: app.phase.clone(),
            status: app.status.clone(),
            font_size,
        }
    }
}

impl View for StatusLine {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(ComponentElement::new_with_builder(
            self.clone(),
            build_status_line,
        ))
    }

    fn listenables(&self) -> Vec<&dyn scarlet_ui::Listenable> {
        vec![
            &self.phase as &dyn scarlet_ui::Listenable,
            &self.status as &dyn scarlet_ui::Listenable,
        ]
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

fn build_status_line(line: &StatusLine) -> Box<dyn View> {
    Box::new(
        Text::new(line.status.get())
            .font_size(line.font_size)
            .color(status_color(&line.phase.get())),
    )
}

#[derive(Clone)]
struct PairingLine {
    pairing: State<String>,
}

impl PairingLine {
    fn new(pairing: State<String>) -> Self {
        Self { pairing }
    }
}

impl View for PairingLine {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(ComponentElement::new_with_builder(
            self.clone(),
            build_pairing_line,
        ))
    }

    fn listenables(&self) -> Vec<&dyn scarlet_ui::Listenable> {
        vec![&self.pairing as &dyn scarlet_ui::Listenable]
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

fn build_pairing_line(line: &PairingLine) -> Box<dyn View> {
    Box::new(
        Text::new(line.pairing.get())
            .font_size(17.0)
            .color(TEXT_COLOR),
    )
}

#[derive(Clone)]
struct ConnectionHeading {
    phase: State<SessionPhase>,
}

impl ConnectionHeading {
    fn new(phase: State<SessionPhase>) -> Self {
        Self { phase }
    }
}

impl View for ConnectionHeading {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(ComponentElement::new_with_builder(
            self.clone(),
            build_connection_heading,
        ))
    }

    fn listenables(&self) -> Vec<&dyn scarlet_ui::Listenable> {
        vec![&self.phase as &dyn scarlet_ui::Listenable]
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

fn build_connection_heading(heading: &ConnectionHeading) -> Box<dyn View> {
    let label = if matches!(
        heading.phase.get(),
        SessionPhase::Connecting | SessionPhase::Pairing
    ) {
        "Connecting to your PC…"
    } else {
        "No PCs found"
    };
    Box::new(Text::new(label).font_size(25.0).color(TEXT_COLOR))
}

impl Application for MoonlightApp {
    fn on_window_sync(&mut self, _ctx: &WindowContext, window: &mut dyn PlatformWindow) {
        let window_title = self.window_title.get();
        if window_title != self.applied_window_title {
            window.set_title(&window_title);
            self.applied_window_title = window_title;
        }

        let desired_pointer_lock = self.pointer_lock_desired.get();
        if desired_pointer_lock != self.pointer_lock_applied.get()
            && !self.pointer_lock_pending.get()
        {
            self.pointer_lock_pending.set(true);
            if window.set_pointer_lock(desired_pointer_lock).is_err() {
                self.pointer_lock_pending.set(false);
                self.pointer_lock_desired
                    .set(self.pointer_lock_applied.get());
                self.status
                    .set(String::from("Input capture is unavailable for this window"));
            }
        }

        let desired_fullscreen = self.fullscreen_desired.get();
        if desired_fullscreen != self.fullscreen_applied.get() && !self.fullscreen_pending.get() {
            self.fullscreen_pending.set(true);
            if window.set_fullscreen(desired_fullscreen).is_err() {
                self.fullscreen_pending.set(false);
                self.fullscreen_desired.set(self.fullscreen_applied.get());
                self.status
                    .set(String::from("Fullscreen request was not accepted"));
            }
        }
    }

    fn on_window_pointer_lock_changed(&mut self, _ctx: &WindowContext, locked: bool) {
        self.pointer_lock_pending.set(false);
        self.pointer_lock_applied.set(locked);
        self.pointer_lock_desired.set(locked);
        if locked {
            self.status
                .set(String::from("Input captured · Ctrl+Alt+Shift+Z to release"));
        } else {
            let control = self.stream_control.get();
            if let Err(error) = self.remote_input.release_all(control.as_ref()) {
                report_input_error(error);
            }
            if self.selected_page.get() == STREAM_PAGE {
                self.status.set(String::from(
                    "Input released · click video or press Ctrl+Alt+Shift+Z",
                ));
            }
        }
    }

    fn on_window_fullscreen_changed(&mut self, _ctx: &WindowContext, fullscreen: bool) {
        self.fullscreen_pending.set(false);
        self.fullscreen_applied.set(fullscreen);
        self.fullscreen_desired.set(fullscreen);
        self.decorations_hidden.set(fullscreen);
    }

    fn on_window_resize(&mut self, _ctx: &WindowContext, _width: u32, _height: u32) {
        if self.fullscreen_applied.get() == self.fullscreen_desired.get()
            && self.decorations_hidden.get() != self.fullscreen_desired.get()
        {
            self.decorations_hidden.set(self.fullscreen_desired.get());
        }
    }

    fn on_focus_changed(&mut self, _window_id: u32, _app_name: &str, _menu_titles: &str) {
        let control = self.stream_control.get();
        if let Err(error) = self.remote_input.release_all(control.as_ref()) {
            report_input_error(error);
        }
    }

    fn on_shutdown(&mut self) {
        self.leave_stream_mode();
        if let Some(control) = self.stream_control.get() {
            control.request_stop();
        }
    }

    fn scenes(&self) -> impl Scene {
        WindowGroup::new(
            "main",
            Window::new(self.window_title.get(), self.clone())
                .app_id(APP_ID)
                .decorated(!self.decorations_hidden.get())
                .size(Size::new(WINDOW_WIDTH, WINDOW_HEIGHT))
                .min_size(Size::new(680.0, 480.0))
                .resizable(true)
                .background_color(BACKGROUND_COLOR),
        )
    }

    fn debug_logging(&self) -> bool {
        false
    }
}

#[derive(Clone)]
struct MoonlightScreen<H: View + Clone, B: View + Clone> {
    toolbar: H,
    body: B,
}

impl<H: View + Clone, B: View + Clone> MoonlightScreen<H, B> {
    fn new(toolbar: H, body: B) -> Self {
        Self { toolbar, body }
    }
}

impl<H, B> View for MoonlightScreen<H, B>
where
    H: View + Clone + 'static,
    B: View + Clone + 'static,
{
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children(
            self.clone(),
            |_| MoonlightScreenRenderObject::default(),
            |view| vec![view.toolbar.clone_view(), view.body.clone_view()],
        ))
    }

    fn listenables(&self) -> Vec<&dyn scarlet_ui::Listenable> {
        let mut listenables = self.toolbar.listenables();
        listenables.extend(self.body.listenables());
        listenables
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

#[derive(Default)]
struct MoonlightScreenRenderObject {
    size: Size,
}

#[allow(deprecated)]
impl ElementRenderObject for MoonlightScreenRenderObject {
    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        self.size = Size::new(
            responsive_extent(constraints.min_width, constraints.max_width),
            responsive_extent(constraints.min_height, constraints.max_height),
        );
        self.size
    }

    fn layout_with_children(
        &mut self,
        constraints: LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        let size = self.layout(constraints);
        let toolbar_height = TOOLBAR_HEIGHT.min(size.height);
        if let Some(toolbar) = children.get_mut(0) {
            toolbar.layout(LayoutConstraints::tight(size.width, toolbar_height));
            toolbar.set_position(Point::ZERO);
        }
        if let Some(body) = children.get_mut(1) {
            let body_height = (size.height - toolbar_height).max(0.0);
            body.layout(LayoutConstraints::tight(size.width, body_height));
            body.set_position(Point::new(0.0, toolbar_height));
        }
        size
    }

    fn size(&self) -> Size {
        self.size
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }

    fn render(&mut self) {}
}

fn responsive_extent(minimum: f32, maximum: f32) -> f32 {
    if maximum.is_finite() {
        maximum.max(minimum).max(0.0)
    } else {
        minimum.max(0.0)
    }
}

fn format_window_title(game_title: Option<&str>) -> String {
    game_title.map_or_else(
        || String::from(DEFAULT_WINDOW_TITLE),
        |title| format!("{title} - {DEFAULT_WINDOW_TITLE}"),
    )
}

fn build_content(app: &MoonlightApp) -> Box<dyn View> {
    app.content()
}

fn moonlight_toolbar<L, R>(title: String, left: L, right: R) -> impl View + Clone
where
    L: View + Clone + 'static,
    R: View + Clone + 'static,
{
    HeaderBar::new(
        zstack! {
            Text::new(title).font_size(18.0).color(TEXT_COLOR),
            hstack! {
                left,
                Spacer::new(),
                right,
            }
            .alignment(Alignment::Center)
            .padding(6.0)
            .frame(f32::INFINITY, f32::INFINITY),
        }
        .frame(f32::INFINITY, f32::INFINITY),
    )
    .height(TOOLBAR_HEIGHT)
    .surface(TOOLBAR_COLOR)
    .separator(TOOLBAR_COLOR)
}

fn settings_row(label: &'static str, value: &'static str) -> impl View + Clone + use<> {
    hstack! {
        Text::new(label).font_size(15.0).color(TEXT_COLOR),
        Spacer::new(),
        Text::new(value).font_size(14.0).color(MUTED_TEXT_COLOR),
    }
    .alignment(Alignment::Center)
    .padding(15.0)
    .frame(f32::INFINITY, 54.0)
    .background(SURFACE_COLOR)
    .border_rounded(BORDER_COLOR, 1.0, 3.0)
}

fn settings_link_row<F>(
    label: &'static str,
    value: &'static str,
    on_click: F,
) -> impl View + Clone + use<F>
where
    F: Fn() + Clone + 'static,
{
    hstack! {
        Text::new(label).font_size(15.0).color(TEXT_COLOR),
        Spacer::new(),
        Text::new(value).font_size(14.0).color(MUTED_TEXT_COLOR),
        IconView::new(Icon::ChevronRight)
            .size(IconSize::Small)
            .color(MUTED_TEXT_COLOR),
    }
    .alignment(Alignment::Center)
    .spacing(8.0)
    .padding(15.0)
    .frame(f32::INFINITY, 54.0)
    .background(SURFACE_COLOR)
    .border_rounded(BORDER_COLOR, 1.0, 3.0)
    .on_click(on_click)
}

fn license_navigation_row(app: &MoonlightApp, index: usize) -> impl View + Clone + use<> {
    let notice = crate::licenses::notice(index);
    let open = app.clone();
    hstack! {
        vstack! {
            Text::new(notice.title).font_size(15.0).color(TEXT_COLOR),
            Text::new(format!("{} · {}", notice.components, notice.license))
                .font_size(12.0)
                .color(MUTED_TEXT_COLOR),
        }
        .alignment(Alignment::Leading)
        .spacing(4.0),
        Spacer::new(),
        IconView::new(Icon::ChevronRight)
            .size(IconSize::Small)
            .color(MUTED_TEXT_COLOR),
    }
    .alignment(Alignment::Center)
    .spacing(10.0)
    .padding(14.0)
    .frame(f32::INFINITY, 64.0)
    .background(SURFACE_COLOR)
    .border_rounded(BORDER_COLOR, 1.0, 4.0)
    .on_click(move || open.open_license(index))
}

fn report_input_error(error: InputError) {
    if error != InputError::ConnectionInactive {
        eprintln!("moonlight: {error}");
    }
}

fn remember_connected_host(host: &str) -> std::result::Result<(), moonlight_control::ControlError> {
    let mut saved = SavedHosts::load_default()?;
    saved.remember(host)?;
    saved.save_default()
}

fn status_color(phase: &SessionPhase) -> Color {
    match phase {
        SessionPhase::Failed(_) => DANGER_COLOR,
        _ => MUTED_TEXT_COLOR,
    }
}

fn placeholder_color(index: usize) -> Color {
    match index % 4 {
        0 => Color::rgb_f32(0.170, 0.235, 0.430),
        1 => Color::rgb_f32(0.275, 0.175, 0.390),
        2 => Color::rgb_f32(0.125, 0.315, 0.330),
        _ => Color::rgb_f32(0.360, 0.195, 0.230),
    }
}

#[cfg(target_os = "scarlet")]
fn platform_video_summary() -> &'static str {
    "Scarlet hardware decode · BGRA presentation"
}

#[cfg(not(target_os = "scarlet"))]
fn platform_video_summary() -> &'static str {
    "Unavailable on this host"
}

#[cfg(test)]
mod tests {
    use super::*;
    use scarlet_ui::{RenderingPipeline, ScrollSource, WheelPhase};

    fn consumes_vertical_wheel(view: impl View + 'static) -> bool {
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_paint_enabled(false);
        pipeline.set_root(view.create_element());
        pipeline
            .element_tree_mut()
            .layout(LayoutConstraints::tight(680.0, 480.0));
        pipeline.handle_event(&Event::Mouse(MouseEvent::Wheel {
            delta_x: 0,
            delta_y: -240,
            x: 340,
            y: 240,
            phase: WheelPhase::Moved,
            source: ScrollSource::Wheel,
        }))
    }

    #[test]
    fn screen_layout_tracks_available_window_size() {
        let mut layout = MoonlightScreenRenderObject::default();

        assert_eq!(
            layout.layout(LayoutConstraints::loose(958.0, 658.0)),
            Size::new(958.0, 658.0)
        );
        assert_eq!(
            layout.layout(LayoutConstraints::tight(678.0, 418.0)),
            Size::new(678.0, 418.0)
        );
    }

    #[test]
    fn responsive_extent_falls_back_to_minimum_without_a_finite_maximum() {
        assert_eq!(responsive_extent(320.0, f32::INFINITY), 320.0);
    }

    #[test]
    fn window_title_identifies_the_running_application() {
        assert_eq!(format_window_title(None), "Moonlight");
        assert_eq!(format_window_title(Some("Desktop")), "Desktop - Moonlight");
    }

    #[test]
    fn root_component_only_rebuilds_for_page_and_window_chrome() {
        let app = MoonlightApp::new();

        assert_eq!(app.listenables().len(), 3);
        assert_eq!(
            AppPage::new(app.clone(), COMPUTERS_PAGE)
                .listenables()
                .len(),
            1
        );
        assert_eq!(AppPage::new(app.clone(), GAMES_PAGE).listenables().len(), 1);
        assert!(
            AppPage::new(app.clone(), ADD_PC_PAGE)
                .listenables()
                .is_empty()
        );
        assert!(
            AppPage::new(app.clone(), SETTINGS_PAGE)
                .listenables()
                .is_empty()
        );
        assert_eq!(AppPage::new(app, STREAM_PAGE).listenables().len(), 2);
    }

    #[test]
    fn settings_and_license_pages_scroll_in_the_minimum_window() {
        let app = MoonlightApp::new();

        assert!(consumes_vertical_wheel(app.settings_screen()));
        assert!(consumes_vertical_wheel(app.licenses_screen()));
        assert!(consumes_vertical_wheel(app.license_detail_screen()));
    }
}
