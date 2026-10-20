//! The app: a main window (live preview, sources, record/pause, timer,
//! level meters), a settings window and a menu bar item, in egui on wgpu
//! with a transparent titlebar and Inter (the paper-rs look). Global
//! shortcuts (⇧⌘R record and ⇧⌘S save replay by default) are set in
//! Settings; ⇧⌘S is only held while the replay buffer is on.
//!
//! One live encode (`crate::live`) is attached to every capture session:
//! it feeds the replay buffer and the streams, so they keep running across
//! preview and recording sessions and while the window is closed.

mod controls;
mod dock;
mod macos;
mod menubar;
mod onboarding;
mod preview;
mod scenes;
mod theme;
mod ui;
mod widgets;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::Result;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

use crate::compose::Scene;
use crate::record::{self, Session};
use crate::settings::Settings;
use crate::writer::Stats;

/// Things that happen to the app from menus, the hotkey, signals and
/// buttons; handled on the main thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    ToggleRecording,
    TogglePause,
    /// Throw away the running recording and start a new one.
    RestartRecording,
    /// Export the last recording (as `State::gif_opts` says) to a GIF.
    ExportGif,
    /// Stop the running recording and move its file to the Trash.
    DeleteRecording,
    ShowWindow,
    ShowSettings,
    RevealLast,
    OpenFolder,
    SaveReplay,
    ToggleStream,
    NextScene,
    PrevScene,
    Transition,
    /// Gather logs and system details into a folder to send.
    ReportProblem,
    OpenLog,
    Quit,
}

impl Cmd {
    fn for_action(a: crate::hotkey::Action) -> Cmd {
        use crate::hotkey::Action;
        match a {
            Action::Record => Cmd::ToggleRecording,
            Action::Pause => Cmd::TogglePause,
            Action::SaveReplay => Cmd::SaveReplay,
            Action::Stream => Cmd::ToggleStream,
            Action::NextScene => Cmd::NextScene,
            Action::PrevScene => Cmd::PrevScene,
            Action::Transition => Cmd::Transition,
        }
    }
}

static QUEUE: Mutex<Vec<Cmd>> = Mutex::new(Vec::new());
static PROXY: OnceLock<Mutex<EventLoopProxy<()>>> = OnceLock::new();
/// A capture device was plugged in or removed.
static DEVICES_CHANGED: AtomicBool = AtomicBool::new(false);

/// REC_GUI_BACKGROUND: open windows without taking focus from the app in
/// use (scripted runs and screenshots while someone works).
fn background() -> bool {
    std::env::var_os("REC_GUI_BACKGROUND").is_some()
}

/// Wake the event loop (from any thread).
fn wake() {
    if let Some(p) = PROXY.get() {
        let _ = p.lock().unwrap().send_event(());
    }
}

pub fn push(c: Cmd) {
    QUEUE.lock().unwrap().push(c);
    wake();
}

/// SIGINT (`simple-record stop`) stops the recording; SIGTERM/SIGHUP quit after
/// saving it. SIGUSR1/2 (`simple-record pause`/`resume`) use record's flags.
static SIG_STOP: AtomicBool = AtomicBool::new(false);
static SIG_QUIT: AtomicBool = AtomicBool::new(false);
extern "C" fn on_sigint(_: libc::c_int) {
    SIG_STOP.store(true, Ordering::SeqCst);
}
extern "C" fn on_sigterm(_: libc::c_int) {
    SIG_QUIT.store(true, Ordering::SeqCst);
}

/// One OS window rendered with egui.
struct Win {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    egui: egui_winit::State,
    ctx: egui::Context,
    renderer: egui_wgpu::Renderer,
    repaint_at: Option<Instant>,
    /// Fully covered, minimized or the screen locked: nothing would be
    /// presented, so don't draw.
    occluded: bool,
}

struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

/// Level meter state for one source.
#[derive(Default, Clone, Copy)]
pub struct Meter {
    /// Displayed level in dBFS (decays smoothly).
    pub db: f32,
    pub peak_db: f32,
    pub peak_at: Option<Instant>,
}

impl Meter {
    fn feed(&mut self, linear: f32, dt: f32) {
        let db = if linear <= 1e-5 { -90.0 } else { 20.0 * linear.log10() };
        self.db = db.max(self.db - 24.0 * dt).max(-90.0);
        if db >= self.peak_db || self.peak_at.is_none_or(|t| t.elapsed() > Duration::from_millis(1500)) {
            self.peak_db = db;
            self.peak_at = Some(Instant::now());
        }
    }
}

#[derive(Clone)]
pub struct Device {
    pub id: String,
    pub name: String,
}

/// Everything the UI shows and edits.
pub struct State {
    pub settings: Settings,
    pub recording: Option<Session>,
    pub preview: Option<Session>,
    /// A running camera between sessions (preview -> recording -> preview),
    /// handed over so it never restarts dark (see `Session::take_camera`).
    spare_cam: Option<crate::camera::Camera>,
    /// Settings the running preview session was started with.
    preview_settings: Option<Settings>,
    pub finishing: bool,
    pub last_file: Option<PathBuf>,
    pub error: Option<String>,
    pub meters: [Meter; 2],
    pub displays: Vec<String>,
    pub windows: Vec<(u32, String)>,
    /// Apps with windows on screen: (bundle ID, name).
    pub apps: Vec<(String, String)>,
    pub cameras: Vec<Device>,
    pub mics: Vec<Device>,
    /// Shortcuts that could not be registered (another app owns them).
    pub hotkey_failed: Vec<crate::hotkey::Action>,
    /// The shortcut being recorded in Settings (global shortcuts are off
    /// meanwhile, so the keys reach the window).
    pub capturing_shortcut: Option<crate::hotkey::Action>,
    /// The preview frame as a native (IOSurface-backed) egui texture.
    pub preview_tex: Option<egui::TextureId>,
    /// Its size in pixels.
    pub preview_px: [u32; 2],
    pub preview_aspect: f32,
    /// Preview frames shown (for the self-test).
    frames_shown: u64,
    /// UI requests handled after the frame.
    pub cmds: Vec<Cmd>,
    pub refresh_devices: bool,
    /// Overlay last sent to the running session.
    pip_applied: Option<crate::compose::PipSpec>,
    /// Scene (which one, and what it shows) and look (background,
    /// transition) last sent to it.
    scene_applied: Option<((crate::compose::Scene, Option<usize>), crate::layout::SceneSpec)>,
    look_applied: Option<(crate::layout::Background, crate::layout::Transition, u32)>,
    filters_applied: Option<crate::filters::MicFilters>,
    /// Width / height of the screen source (for scene thumbnails).
    pub screen_aspect: f64,
    /// The custom scene layer being edited.
    pub selected_layer: Option<usize>,
    /// The last run ended with a crash (the error banner offers a report).
    pub crashed_last_time: bool,
    /// Studio mode: the preview shows a staged scene beside the program;
    /// Transition makes it the program.
    pub studio: bool,
    pub staged: Option<(Scene, Option<usize>)>,
    /// What the session was last told to stage.
    studio_applied: Option<Option<crate::layout::SceneSpec>>,
    /// Permissions as last read, and when (the welcome screen).
    pub perms: Option<(onboarding::Perms, Instant)>,
    /// Permissions as the app last acted on them (see `permissions_changed`).
    perms_seen: Option<onboarding::Perms>,
    /// The welcome screen is up.
    pub onboarding: bool,
    /// The Settings window's open tab.
    pub settings_tab: ui::SettingsTab,
    /// A layer drag in progress: (layer, grip, its box when the drag
    /// began, the pointer's total movement as fractions of the frame).
    pub layer_drag: Option<(usize, crate::layout::Grip, [f64; 4], [f64; 2])>,
    /// Size in points of each display (for regions).
    pub display_points: Vec<(f64, f64)>,
    /// Picking a region in the preview.
    pub region_select: Option<RegionSelect>,
    /// Camera aspect ratio from the running session (for drag hit-testing).
    pub cam_aspect: f64,
    pub permissions_missing: bool,
    /// The shared live encode (replay buffer, streams).
    pub live: Arc<crate::live::Live>,
    /// Replay buffer (seconds held, bytes) when on.
    pub replay_stats: Option<(f64, usize)>,
    replay_saves: Vec<std::thread::JoinHandle<Result<PathBuf>>>,
    /// Running stream outputs.
    pub stream_ids: Vec<u64>,
    pub stream_statuses: Vec<crate::live::OutputStatus>,
    /// Stream keys being typed, per destination (0 is the first); moved
    /// to the Keychain on save and cleared.
    pub stream_key_input: Vec<String>,
    /// Whether the Keychain has a key, per destination URL.
    pub key_saved: std::collections::HashMap<String, bool>,
    /// A short confirmation (replay saved) and when it was shown.
    pub notice: Option<(String, Instant)>,
    /// The mic the running session opened (None = system default), and the
    /// device fallback warning last shown, so it isn't repeated on restarts.
    mic_in_use: Option<Option<String>>,
    mic_warned: Option<String>,
    /// The mic is muted from the recording controls (this recording only).
    pub mic_muted: bool,
    /// A Restart or Delete in the recording controls waiting to be
    /// confirmed, and when it was asked.
    pub controls_confirm: Option<(controls::Confirm, Instant)>,
    /// The GIF export's choices, for the recording in `gif_source`.
    pub gif_opts: crate::gif::GifOptions,
    /// The recording the GIF choices are for, and its length in seconds.
    pub gif_source: Option<(PathBuf, f64)>,
    /// The running GIF export's progress, in thousandths.
    pub gif_progress: Option<Arc<std::sync::atomic::AtomicU32>>,
    gif_job: Option<std::thread::JoinHandle<Result<(PathBuf, crate::gif::GifStats)>>>,
}

impl State {
    /// Whether the Keychain holds a stream key for `url` (looked up once
    /// per URL).
    pub fn has_stream_key(&mut self, url: &str) -> bool {
        if url.is_empty() {
            return false;
        }
        *self.key_saved.entry(url.to_string()).or_insert_with(|| crate::live::keychain::get(url).is_some())
    }

    /// The key typed for destination `i`.
    pub fn key_input(&mut self, i: usize) -> &mut String {
        if self.stream_key_input.len() <= i {
            self.stream_key_input.resize(i + 1, String::new());
        }
        &mut self.stream_key_input[i]
    }

    /// Store the key typed for destination `i` (at `url`) in the Keychain
    /// and forget it.
    pub fn save_stream_key(&mut self, i: usize, url: &str) {
        let key = std::mem::take(self.key_input(i));
        let key = key.trim();
        if url.is_empty() || key.is_empty() {
            return;
        }
        match crate::live::keychain::set(url, key) {
            Ok(()) => {
                self.key_saved.insert(url.to_string(), true);
            }
            Err(e) => self.error = Some(format!("Could not save the stream key: {e:#}")),
        }
    }

    pub fn delete_stream_key(&mut self, url: &str) {
        let _ = crate::live::keychain::delete(url);
        self.key_saved.insert(url.to_string(), false);
    }

    pub fn streaming(&self) -> bool {
        !self.stream_ids.is_empty()
    }

    /// Permissions as last read (at most once a second).
    pub fn perms(&mut self) -> onboarding::Perms {
        onboarding::refresh(self)
    }

    /// Set the GIF choices up for the last recording (its whole length, or
    /// the first 30 s of a longer one), unless they already are.
    pub fn gif_prepare(&mut self) {
        let Some(file) = self.last_file.clone() else { return };
        if self.gif_source.as_ref().is_some_and(|(f, _)| *f == file) {
            return;
        }
        let len = crate::gif::duration(&file).unwrap_or(0.0);
        self.gif_opts = crate::gif::GifOptions { start: 0.0, duration: Some(len.min(30.0)), ..self.gif_opts.clone() };
        self.gif_source = Some((file, len));
    }

    pub fn is_paused(&self) -> bool {
        self.recording.as_ref().is_some_and(|s| s.is_paused())
    }

    /// Recorded time (excluding pauses) of the running recording.
    pub fn clock(&self) -> String {
        self.recording.as_ref().map(|s| record::clock(s.recorded_secs())).unwrap_or_default()
    }

    /// Send the overlay settings to the running session if they changed:
    /// moving or restyling the camera is live in the preview and the file.
    pub fn apply_pip(&mut self) {
        let Some(sess) = self.recording.as_ref().or(self.preview.as_ref()) else {
            self.pip_applied = None;
            return;
        };
        if let Some(a) = sess.cam_aspect() {
            self.cam_aspect = a;
        }
        let spec = self.settings.pip_spec();
        if self.pip_applied.as_ref() != Some(&spec) {
            sess.set_pip(spec.clone());
            self.pip_applied = Some(spec);
        }
    }

    /// Send the scene and its look to the running session if they changed:
    /// scene switches are live (with the transition), in the preview and
    /// the file.
    pub fn apply_scene(&mut self) {
        let Some(sess) = self.recording.as_ref().or(self.preview.as_ref()) else {
            self.scene_applied = None;
            self.look_applied = None;
            self.filters_applied = None;
            self.studio_applied = None;
            return;
        };
        let applied = (self.settings.scene_id(), self.settings.scene_spec());
        if self.scene_applied.as_ref() != Some(&applied) {
            // Another scene animates in; edits to the same one apply at once.
            let animate = self.scene_applied.as_ref().is_none_or(|(id, _)| *id != applied.0);
            if sess.set_scene(applied.1.clone(), animate) {
                self.scene_applied = Some(applied);
            }
        }
        let staged = self.studio.then(|| self.settings.spec_of(self.staged.unwrap_or(self.settings.scene_id())));
        if self.studio_applied.as_ref() != Some(&staged) {
            if sess.set_studio(staged.clone()) {
                self.studio_applied = Some(staged);
            } else {
                // The session doesn't compose: no studio mode.
                self.studio = false;
                self.studio_applied = None;
            }
        }
        let s = &self.settings;
        let look = (s.background.clone(), s.transition, s.transition_ms);
        if self.look_applied.as_ref() != Some(&look) {
            sess.set_look(look.0.clone(), look.1, look.2 as f64 / 1000.0);
            self.look_applied = Some(look);
        }
        let filters = self.mic_filters();
        if self.filters_applied != Some(filters) {
            sess.set_mic_filters(filters);
            self.filters_applied = Some(filters);
        }
    }

    /// The mic filters from Settings, with the controls' mute.
    fn mic_filters(&self) -> crate::filters::MicFilters {
        crate::filters::MicFilters { mute: self.mic_muted, ..self.settings.mic_filters }
    }

    /// The running session was started with the current scene and look.
    fn scene_started(&mut self) {
        let s = &self.settings;
        self.scene_applied = Some((s.scene_id(), s.scene_spec()));
        self.look_applied = Some((s.background.clone(), s.transition, s.transition_ms));
        self.filters_applied = Some(self.mic_filters());
        self.studio_applied = None;
    }

    /// Whether the running recording can switch to `scene` live: it needs
    /// the camera if the scene shows it, a capture of every other display
    /// and camera it shows, and the compositor unless it is the plain screen.
    pub fn can_switch_live(&self, scene: &crate::layout::SceneSpec) -> bool {
        let Some(r) = &self.recording else { return true };
        let plain = *scene == crate::layout::SceneSpec::Template(Scene::Screen);
        let sources = scene.sources().iter().all(|n| r.sources().contains(n));
        (!scene.uses_cam() || r.camera_name().is_some()) && sources && (plain || r.composed())
    }
}

/// A region being picked by dragging over the preview.
#[derive(Clone, Copy, Debug, Default)]
pub struct RegionSelect {
    /// The region before (restored on cancel).
    pub before: Option<[f64; 4]>,
    /// Where the drag started, and where it last was.
    pub start: Option<egui::Pos2>,
    pub end: Option<egui::Pos2>,
}

/// A finalized recording: (file, outcome).
type Finished = (PathBuf, Result<Stats, String>);

/// The recording controls' Restart and Delete: the take is finished, then
/// moved to the Trash (and for Restart, a new one starts).
#[derive(Clone, Copy, PartialEq, Eq)]
enum AfterFinish {
    Delete,
    Restart,
}

/// Which of the app's windows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Main,
    Settings,
    Controls,
}

struct App {
    gpu: Option<Gpu>,
    main: Option<Win>,
    settings_win: Option<Win>,
    /// The floating recording controls (see `controls.rs`).
    controls: Option<Win>,
    /// Where the controls were last left, to reopen them there.
    controls_pos: Option<winit::dpi::PhysicalPosition<i32>>,
    /// What to do once the recording being finished is written.
    after_finish: Option<AfterFinish>,
    main_visible: bool,
    /// The main window was hidden when the recording started: bring it
    /// back once the recording is done.
    main_hidden_for_recording: bool,
    st: State,
    menubar: Option<menubar::MenuBar>,
    /// Global shortcuts registered now.
    hotkeys: Vec<(crate::hotkey::Action, crate::hotkey::Combo)>,
    _main_menu: Option<objc2::rc::Retained<menubar::Target>>,
    scaler: preview::Scaler,
    /// Textures of the last preview frames (egui may still draw them).
    preview_textures: std::collections::VecDeque<wgpu::Texture>,
    last_tick: Instant,
    last_meter: Instant,
    start_recording: bool,
    launched: Instant,
    menu_shown: bool,
    selftest: Option<SelfTest>,
    finished: Arc<Mutex<Option<Finished>>>,
}

fn load_devices(st: &mut State) {
    let ds = crate::screen::displays();
    if let Some(d) = ds.as_ref().ok().and_then(|ds| ds.get(st.settings.display.max(1) - 1).or(ds.first())) {
        st.screen_aspect = d.pixels.0 as f64 / d.pixels.1.max(1) as f64;
    }
    if let Ok(ds) = &ds {
        st.display_points = ds.iter().map(|d| (d.points.0 as f64, d.points.1 as f64)).collect();
    }
    st.displays = ds
        .map(|ds| {
            ds.iter()
                .enumerate()
                .map(|(i, d)| format!("Display {} — {}×{}", i + 1, d.pixels.0, d.pixels.1))
                .collect()
        })
        .unwrap_or_default();
    st.windows = crate::screen::windows()
        .map(|ws| {
            ws.into_iter()
                .filter(|w| !w.title.is_empty())
                .map(|w| (w.id, format!("{} — {}", w.app, w.title)))
                .collect()
        })
        .unwrap_or_default();
    st.apps = crate::screen::applications().map(|a| a.into_iter().map(|a| (a.bundle_id, a.name)).collect()).unwrap_or_default();
    st.cameras = crate::camera::cameras().into_iter().map(|d| Device { id: d.id, name: d.name }).collect();
    st.mics = crate::camera::microphones().into_iter().map(|d| Device { id: d.id, name: d.name }).collect();
}

impl App {
    fn new(start_recording: bool) -> App {
        let mut st = State {
            settings: Settings::load(),
            recording: None,
            preview: None,
            spare_cam: None,
            preview_settings: None,
            finishing: false,
            last_file: None,
            error: None,
            meters: Default::default(),
            displays: vec![],
            windows: vec![],
            apps: vec![],
            cameras: vec![],
            mics: vec![],
            hotkey_failed: vec![],
            capturing_shortcut: None,
            preview_tex: None,
            preview_px: [0, 0],
            preview_aspect: 16.0 / 9.0,
            frames_shown: 0,
            cmds: vec![],
            refresh_devices: false,
            permissions_missing: false,
            pip_applied: None,
            scene_applied: None,
            look_applied: None,
            filters_applied: None,
            screen_aspect: 16.0 / 10.0,
            selected_layer: None,
            crashed_last_time: false,
            studio: false,
            staged: None,
            studio_applied: None,
            perms: None,
            perms_seen: None,
            onboarding: false,
            settings_tab: Default::default(),
            layer_drag: None,
            display_points: vec![],
            region_select: None,
            cam_aspect: 16.0 / 9.0,
            live: crate::live::Live::new(),
            replay_stats: None,
            replay_saves: vec![],
            stream_ids: vec![],
            stream_statuses: vec![],
            stream_key_input: vec![],
            key_saved: Default::default(),
            notice: None,
            mic_in_use: None,
            mic_warned: None,
            mic_muted: false,
            controls_confirm: None,
            gif_opts: Default::default(),
            gif_source: None,
            gif_progress: None,
            gif_job: None,
        };
        st.live.set_config(st.settings.live_config());
        load_devices(&mut st);
        crate::camera::on_device_change(|| {
            DEVICES_CHANGED.store(true, Ordering::SeqCst);
            wake();
        });
        App {
            gpu: None,
            main: None,
            settings_win: None,
            controls: None,
            controls_pos: None,
            after_finish: None,
            main_visible: true,
            main_hidden_for_recording: false,
            st,
            menubar: None,
            hotkeys: vec![],
            _main_menu: None,
            scaler: preview::Scaler::new(),
            preview_textures: Default::default(),
            last_tick: Instant::now(),
            last_meter: Instant::now(),
            start_recording,
            launched: Instant::now(),
            menu_shown: false,
            selftest: std::env::var_os("REC_GUI_SELFTEST").map(|_| SelfTest::default()),
            finished: Arc::new(Mutex::new(None)),
        }
    }

    fn gpu(&mut self, surface: &wgpu::Surface<'static>) -> &Gpu {
        if self.gpu.is_none() {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
            let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(surface),
                ..Default::default()
            }))
            .expect("no compatible GPU adapter");
            let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("simple-record"),
                required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
                ..Default::default()
            }))
            .expect("create device");
            self.gpu = Some(Gpu { instance, adapter, device, queue });
        }
        self.gpu.as_ref().unwrap()
    }

    fn open_window(&mut self, el: &ActiveEventLoop, title: &str, size: (f64, f64), min: (f64, f64)) -> Win {
        use winit::platform::macos::WindowAttributesExtMacOS;
        let attrs = Window::default_attributes()
            .with_title(title)
            .with_inner_size(winit::dpi::LogicalSize::new(size.0, size.1))
            .with_min_inner_size(winit::dpi::LogicalSize::new(min.0, min.1))
            .with_titlebar_transparent(true)
            .with_fullsize_content_view(true)
            .with_title_hidden(true);
        let window = Arc::new(el.create_window(attrs).expect("create window"));
        self.wrap_window(window, false)
    }

    /// Render `window` with egui; `transparent`: what egui doesn't draw
    /// shows what is behind the window.
    fn wrap_window(&mut self, window: Arc<Window>, transparent: bool) -> Win {
        // A throwaway instance creates the first surface; the shared one
        // after that (surfaces must come from the adapter's instance).
        let surface = match &self.gpu {
            Some(g) => g.instance.create_surface(window.clone()).expect("surface"),
            None => {
                let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
                let s = instance.create_surface(window.clone()).expect("surface");
                let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    compatible_surface: Some(&s),
                    ..Default::default()
                }))
                .expect("no compatible GPU adapter");
                let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                    label: Some("simple-record"),
                    required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
                    ..Default::default()
                }))
                .expect("create device");
                self.gpu = Some(Gpu { instance, adapter, device, queue });
                s
            }
        };
        let g = self.gpu(&surface);
        let caps = surface.get_capabilities(&g.adapter);
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        let px = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: px.width.max(1),
            height: px.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: if transparent {
                [wgpu::CompositeAlphaMode::PreMultiplied, wgpu::CompositeAlphaMode::PostMultiplied]
                    .into_iter()
                    .find(|m| caps.alpha_modes.contains(m))
                    .unwrap_or(caps.alpha_modes[0])
            } else {
                caps.alpha_modes[0]
            },
            view_formats: vec![],
            color_space: Default::default(),
        };
        surface.configure(&g.device, &config);
        let renderer = egui_wgpu::Renderer::new(&g.device, format, egui_wgpu::RendererOptions::default());
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let egui = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(g.device.limits().max_texture_dimension_2d as usize),
        );
        let (x, y) = theme::TRAFFIC_LIGHTS;
        macos::inset_traffic_lights(&window, x, y);
        // egui and the preview are sRGB: let macOS convert them for the
        // display (wide-gamut panels otherwise show them oversaturated).
        macos::tag_layers_srgb(&window);
        window.request_redraw();
        Win { window, surface, config, egui, ctx, renderer, repaint_at: None, occluded: false }
    }

    /// Show the recording controls while recording (if wanted), and close
    /// them otherwise.
    fn sync_controls(&mut self, el: &ActiveEventLoop) {
        if self.st.recording.is_some() && self.st.settings.recording_controls {
            self.open_controls(el);
        } else {
            if let Some(w) = self.controls.take() {
                self.controls_pos = w.window.outer_position().ok();
            }
            self.st.controls_confirm = None;
        }
    }

    fn open_controls(&mut self, el: &ActiveEventLoop) {
        if self.controls.is_some() {
            return;
        }
        use winit::platform::macos::WindowAttributesExtMacOS;
        let (w, h) = controls::SIZE;
        let mut attrs = Window::default_attributes()
            .with_title("SimpleRecord Controls")
            .with_inner_size(winit::dpi::LogicalSize::new(w, h))
            .with_resizable(false)
            .with_decorations(false)
            .with_transparent(true)
            .with_window_level(winit::window::WindowLevel::AlwaysOnTop)
            .with_active(false)
            .with_has_shadow(false)
            .with_accepts_first_mouse(true);
        // Where it was left, else centered near the bottom of the screen
        // SimpleRecord's window is on (above the Dock).
        let pos = self.controls_pos.or_else(|| {
            let m = self.main.as_ref().and_then(|w| w.window.current_monitor()).or_else(|| el.primary_monitor())?;
            let (mp, ms, k) = (m.position(), m.size(), m.scale_factor());
            let (pw, ph) = ((w * k) as i32, (h * k) as i32);
            Some(winit::dpi::PhysicalPosition::new(mp.x + (ms.width as i32 - pw) / 2, mp.y + ms.height as i32 - ph - (96.0 * k) as i32))
        });
        if let Some(p) = pos {
            attrs = attrs.with_position(p);
        }
        let Ok(window) = el.create_window(attrs) else { return };
        let window = Arc::new(window);
        macos::make_floating_panel(&window);
        let win = self.wrap_window(window, true);
        self.controls = Some(win);
    }

    fn show_main(&mut self, el: &ActiveEventLoop) {
        if self.main.is_none() {
            // REC_MAIN_SIZE=WxH or full: open at this size or fullscreen (layout checks).
            let size = std::env::var("REC_MAIN_SIZE")
                .ok()
                .and_then(|v| v.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))))
                .unwrap_or((1040.0, 660.0));
            let w = self.open_window(el, "SimpleRecord", size, (860.0, 560.0));
            if std::env::var("REC_MAIN_SIZE").as_deref() == Ok("full") {
                w.window.set_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
            }
            self.main = Some(w);
        }
        if let Some(w) = &self.main {
            w.window.set_visible(true);
            if !background() {
                w.window.focus_window();
            }
            w.window.request_redraw();
        }
        self.main_visible = true;
        preview::set_wanted(true);
        if !background() {
            macos::activate();
        }
        self.sync_preview();
    }

    fn hide_main(&mut self) {
        if let Some(w) = &self.main {
            w.window.set_visible(false);
        }
        self.main_visible = false;
        preview::set_wanted(false);
        self.sync_preview();
    }

    fn show_settings(&mut self, el: &ActiveEventLoop) {
        if self.settings_win.is_none() {
            let w = self.open_window(el, "SimpleRecord Settings", (520.0, 600.0), (460.0, 420.0));
            self.settings_win = Some(w);
        }
        if let Some(w) = &self.settings_win {
            if !background() {
                w.window.focus_window();
            }
            w.window.request_redraw();
        }
        if !background() {
            macos::activate();
        }
    }

    fn options(&mut self, output: Option<&std::path::Path>) -> Result<(record::Options, Option<crate::screen::Target>)> {
        // A display that was unplugged falls back to the main one.
        let mut settings = self.st.settings.clone();
        let mut warning = None;
        let count = crate::screen::displays().map(|d| d.len()).unwrap_or(0);
        if count > 0 && settings.window.is_none() && settings.display > count {
            warning = Some(format!("Display {} not connected; using display 1", settings.display));
            settings.display = 1;
            eprintln!("rec: warning: {}", warning.as_deref().unwrap_or_default());
        }
        let args = settings.record_args(output);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let (mut o, t) = crate::prepare(crate::parse_record_args(&refs)?)?;
        o.live = Some(self.st.live.clone());
        o.scene = self.st.settings.scene_spec();
        o.mic_filters = self.st.settings.mic_filters;
        o.pip.effects = self.st.settings.camera_effects;
        // The command line has the colours only; the screen's inset too.
        o.background = self.st.settings.background.clone();
        // The mic is optional: without permission, record without it (the
        // Audio card says so) rather than refuse to start.
        if o.mic.is_some() && crate::tcc::mic_status() != crate::tcc::Status::Granted {
            o.mic = None;
            o.mic_warning = Some("Microphone not allowed: recording without it".into());
        }
        // Fallbacks never block: say so once.
        let warning = [warning, o.mic_warning.clone()].into_iter().flatten().collect::<Vec<_>>().join(" · ");
        let warning = Some(warning).filter(|w| !w.is_empty());
        if warning != self.st.mic_warned {
            if let Some(w) = &warning {
                self.st.notice = Some((w.clone(), Instant::now()));
            }
            self.st.mic_warned = warning;
        }
        self.st.mic_in_use = o.mic.clone();
        Ok((o, t))
    }

    /// After a device was plugged in or removed: refresh the lists, and
    /// restart the preview if the saved mic now resolves differently.
    fn devices_changed(&mut self) {
        load_devices(&mut self.st);
        let s = &self.st.settings;
        let Some(sel) = s.mic.as_deref().filter(|_| s.mic_enabled) else { return };
        if self.st.recording.is_some() || self.st.preview.is_none() {
            return;
        }
        let want = Some(crate::camera::pick_mic(sel, s.mic_name.as_deref()).id);
        if self.st.mic_in_use != want {
            self.st.preview_settings = None;
        }
    }

    /// Whether the live encode needs a capture session (replay or stream).
    fn live_wanted(&self) -> bool {
        self.st.settings.replay_on() || self.st.streaming()
    }

    /// Apply the replay and live-encode settings.
    fn sync_live(&mut self) {
        let cfg = self.st.settings.live_config();
        if self.st.live.config() != cfg {
            self.st.live.set_config(cfg);
        }
        // HDR sessions don't feed the (SDR) live encode.
        let want = self.st.settings.replay_on().then(|| self.st.settings.replay_secs.clamp(15, 300) as f64);
        let have = self.st.live.replay_stats().is_some();
        if want.is_some() || have {
            self.st.live.set_replay(want);
        }
        self.st.replay_stats = self.st.live.replay_stats();
        let statuses = self.st.live.statuses();
        self.st.stream_statuses = self.st.stream_ids.iter().filter_map(|id| statuses.iter().find(|(i, _)| i == id).map(|(_, s)| s.clone())).collect();
    }

    /// Register the global shortcuts the settings ask for. Save Replay is
    /// only held while the replay buffer is on (⇧⌘S is "Save As" in most
    /// apps), and none while a shortcut is being recorded.
    fn sync_hotkeys(&mut self) {
        use crate::hotkey::Action;
        let s = &self.st.settings;
        let want: Vec<(Action, crate::hotkey::Combo)> = Action::ALL
            .into_iter()
            .filter(|a| self.st.capturing_shortcut.is_none() && (*a != Action::SaveReplay || s.replay_on()))
            .filter_map(|a| s.hotkey(a).map(|c| (a, c)))
            .collect();
        if want == self.hotkeys {
            return;
        }
        for (a, _) in self.hotkeys.drain(..) {
            crate::hotkey::unregister(a.id());
        }
        self.st.hotkey_failed.clear();
        for (a, c) in &want {
            let cmd = Cmd::for_action(*a);
            if let Err(code) = crate::hotkey::register(a.id(), c.key, c.mods, move || push(cmd)) {
                eprintln!("rec: could not register {} for {} (error {code}); another app may own it", c.label(), a.label());
                self.st.hotkey_failed.push(*a);
            }
        }
        self.hotkeys = want;
        if let Some(m) = &self.menubar {
            m.set_shortcuts(s.hotkey(Action::Record), s.hotkey(Action::SaveReplay), !self.st.hotkey_failed.contains(&Action::Record));
        }
    }

    /// Show the next (`step` 1) or previous (-1) scene of the gallery that
    /// the running session can show.
    /// Studio mode: the staged scene becomes the program.
    fn transition(&mut self) {
        let Some(staged) = self.st.staged.filter(|_| self.st.studio) else { return };
        if staged != self.st.settings.scene_id() && self.st.can_switch_live(&self.st.settings.spec_of(staged)) {
            self.st.settings.show(staged);
            let _ = self.st.settings.save();
            self.st.apply_scene();
        }
    }

    /// The next or previous scene the session can show (in studio mode,
    /// staged rather than shown).
    fn cycle_scene(&mut self, step: i32) {
        let s = &self.st.settings;
        let mut all: Vec<(Scene, Option<usize>)> = Scene::ALL.iter().map(|x| (*x, None)).collect();
        all.extend((0..s.custom_scenes.len()).map(|i| (Scene::Screen, Some(i))));
        let n = all.len() as i32;
        let current = if self.st.studio { self.st.staged.unwrap_or(s.scene_id()) } else { s.scene_id() };
        let cur = all.iter().position(|x| *x == current).unwrap_or(0) as i32;
        for k in 1..n {
            let (scene, custom) = all[(cur + step * k).rem_euclid(n) as usize];
            let mut next = s.clone();
            next.scene = scene;
            next.custom_scene = custom;
            if self.st.can_switch_live(&next.scene_spec()) {
                let name = match custom {
                    Some(i) => next.custom_scenes[i].name.clone(),
                    None => scene.label().to_string(),
                };
                if self.st.studio {
                    self.st.staged = Some((scene, custom));
                    self.st.apply_scene();
                    self.st.notice = Some((format!("Staged: {name}"), Instant::now()));
                    return;
                }
                self.st.settings = next;
                let _ = self.st.settings.save();
                self.st.apply_scene();
                self.st.notice = Some((format!("Scene: {name}"), Instant::now()));
                return;
            }
        }
    }

    /// A folder on the Desktop with the logs, a description of this Mac and
    /// its devices, the settings without secrets, and recent crash
    /// reports, shown in Finder: ready to attach to a report.
    fn report_problem(&mut self) {
        let st = &self.st;
        let run = |cmd: &str, args: &[&str]| {
            std::process::Command::new(cmd).args(args).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
        };
        let (screen, mic, cam) = crate::tcc::report();
        let mut sys = format!(
            "SimpleRecord {}\nmacOS {} ({})\nModel {}\nPermissions: screen {screen}, microphone {mic}, camera {cam}\n",
            env!("CARGO_PKG_VERSION"),
            run("sw_vers", &["-productVersion"]),
            run("sw_vers", &["-buildVersion"]),
            run("sysctl", &["-n", "hw.model"]),
        );
        if let Some(l) = crate::outlatency::output_latency() {
            sys += &format!("Output latency {:.1} ms\n", l * 1000.0);
        }
        let list = |title: &str, items: Vec<String>| format!("{title}:\n{}\n", items.iter().map(|i| format!("  {i}\n")).collect::<String>());
        sys += &list("Displays", st.displays.clone());
        sys += &list("Cameras", st.cameras.iter().map(|d| d.name.clone()).collect());
        sys += &list("Microphones", st.mics.iter().map(|d| d.name.clone()).collect());
        sys += &format!("Recording: {}  Streaming: {}  Last error: {:?}\n", st.recording.is_some(), st.streaming(), st.error);
        let settings = std::fs::read_to_string(Settings::path()).ok();
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let desktop = Some(home.join("Desktop")).filter(|d| d.is_dir()).unwrap_or_else(|| st.settings.output_dir.clone());
        match crate::logging::diagnostics(&desktop, &sys, settings.as_deref(), &home.join("Library/Logs/DiagnosticReports"), 7) {
            Ok(dir) => {
                eprintln!("rec: diagnostics in {}", dir.display());
                macos::reveal(&dir);
                self.st.notice = Some(("Diagnostics saved to the Desktop: attach that folder to your report".into(), Instant::now()));
            }
            Err(e) => self.st.error = Some(format!("Could not gather diagnostics: {e}")),
        }
    }

    fn save_replay(&mut self) {
        if self.st.replay_stats.is_none() {
            return;
        }
        let path = self.st.settings.output_dir.join(crate::settings::timestamped("replay", "mp4"));
        let r = std::fs::create_dir_all(&self.st.settings.output_dir)
            .map_err(anyhow::Error::from)
            .and_then(|_| self.st.live.save_replay(path));
        match r {
            Ok(h) => self.st.replay_saves.push(h),
            Err(e) => self.st.error = Some(format!("Replay: {e:#}")),
        }
    }

    /// Go live on every destination (one shared encode), or stop them all.
    fn toggle_stream(&mut self) {
        if self.st.streaming() {
            for id in self.st.stream_ids.drain(..) {
                self.st.live.remove_output(id);
            }
            self.st.stream_statuses.clear();
            return;
        }
        if self.st.settings.hdr_on() {
            self.st.error = Some("Streaming is off while HDR recording is on (Settings > Video)".into());
            return;
        }
        // The encode is set for all destinations before the first starts.
        self.sync_live();
        let mut errors = vec![];
        for (url, own) in self.st.settings.stream_targets() {
            match crate::live::resolve_destination(&url).and_then(|(url, key)| crate::live::start_output(&url, &key)) {
                Ok(o) => self.st.stream_ids.push(self.st.live.add_output_with(own, o)),
                Err(e) => errors.push(format!("{e:#}")),
            }
        }
        self.st.error = (!errors.is_empty()).then(|| format!("Could not go live: {}", errors.join("; ")));
        self.sync_preview();
    }

    /// A permission was answered (a prompt, System Settings) since the last
    /// look: the error it caused is stale, and the preview, which was refused
    /// or opened without the mic or camera, starts again to use it.
    fn permissions_changed(&mut self) {
        let now = self.st.perms();
        let before = self.st.perms_seen.replace(now);
        if before.is_none_or(|b| b == now) {
            return;
        }
        eprintln!("rec: permissions now: screen {}, microphone {}, camera {}", now.screen, now.mic, now.camera);
        if self.st.permissions_missing {
            self.st.permissions_missing = false;
            self.st.error = None;
        }
        // Stale for `sync_preview`: a running preview restarts.
        self.st.preview_settings = None;
    }

    /// Run a preview session while the window is visible and nothing is
    /// recording; restart it when the settings change.
    fn sync_preview(&mut self) {
        // The replay buffer and streams keep a session running while the
        // window is closed.
        let want = (self.main_visible || self.live_wanted()) && self.st.recording.is_none() && !self.st.finishing;
        // Overlay changes are applied live (`apply_pip`), not by restarting.
        let stale = self.st.preview_settings.as_ref().map(Settings::capture_key) != Some(self.st.settings.capture_key());
        if self.st.preview.is_some() && (!want || stale) {
            if let Some(mut s) = self.st.preview.take() {
                if want {
                    self.st.spare_cam = s.take_camera();
                }
                let _ = s.finish();
            }
            self.st.preview_settings = None;
        }
        // Not under the welcome screen: without permission it can only fail.
        if want && self.st.preview.is_none() && self.st.error.is_none() && !self.st.onboarding {
            let cb = preview::callback(Arc::new(wake));
            let cam = self.st.spare_cam.take();
            match self.options(None).and_then(|(o, t)| Session::start_with_camera(&o, t, false, Some(cb), cam)) {
                Ok(s) => {
                    self.st.pip_applied = Some(self.st.settings.pip_spec());
                    self.st.scene_started();
                    self.st.preview = Some(s);
                    self.st.permissions_missing = false;
                }
                Err(e) => {
                    let msg = format!("{e:#}");
                    self.st.permissions_missing = msg.contains("missing permission");
                    self.st.error = Some(permission_hint(&msg).unwrap_or_else(|| format!("Preview unavailable: {msg}")));
                }
            }
            self.st.preview_settings = Some(self.st.settings.clone());
        }
        // Nothing took the camera (window hidden, or a scene without it):
        // turn it off.
        if !self.st.finishing
            && let Some(c) = self.st.spare_cam.take()
        {
            c.stop();
        }
    }

    fn toggle_recording(&mut self, el: &ActiveEventLoop) {
        if self.st.finishing {
            return;
        }
        if self.st.recording.is_some() {
            self.stop_recording();
        } else {
            self.start_recording(el);
        }
    }

    fn start_recording(&mut self, el: &ActiveEventLoop) {
        self.st.mic_muted = false;
        self.st.controls_confirm = None;
        // One capture session at a time: the recording feeds the preview,
        // and takes over its camera (already exposed for the room).
        let mut cam = self.st.spare_cam.take();
        if let Some(mut s) = self.st.preview.take() {
            cam = cam.or_else(|| s.take_camera());
            let _ = s.finish();
        }
        self.st.preview_settings = None;
        self.st.error = None;
        let file = self.st.settings.new_output_file();
        // The controls open before the capture starts: it leaves out every
        // window of SimpleRecord, found by the windows it has on screen.
        if self.st.settings.recording_controls {
            self.open_controls(el);
        }
        let cb = preview::callback(Arc::new(wake));
        let r = std::fs::create_dir_all(&self.st.settings.output_dir)
            .map_err(anyhow::Error::from)
            .and_then(|_| self.options(Some(&file)))
            .and_then(|(o, t)| Session::start_with_camera(&o, t, true, Some(cb), cam));
        match r {
            Ok(sess) => {
                let _ = std::fs::write(record::pidfile_path(), format!("{}\n{}\n", std::process::id(), file.display()));
                self.st.pip_applied = Some(self.st.settings.pip_spec());
                self.st.scene_started();
                self.st.recording = Some(sess);
                // The controls are on screen: get the window out of the way
                // (the gear in the controls opens it again).
                if self.st.settings.recording_controls && self.main_visible {
                    self.main_hidden_for_recording = true;
                    self.hide_main();
                }
                record::PAUSE_REQ.store(false, Ordering::SeqCst);
                record::RESUME_REQ.store(false, Ordering::SeqCst);
            }
            Err(e) => {
                eprintln!("rec: could not start recording: {e:#}");
                let msg = format!("{e:#}");
                self.st.permissions_missing = msg.contains("missing permission");
                self.st.error = Some(permission_hint(&msg).unwrap_or_else(|| format!("Could not start recording: {msg}")));
                self.sync_preview();
            }
        }
    }

    fn stop_recording(&mut self) {
        let Some(mut sess) = self.st.recording.take() else { return };
        // The camera goes back to the preview once the file is finished.
        self.st.spare_cam = sess.take_camera();
        let file = sess.output.clone().unwrap_or_default();
        self.st.finishing = true;
        let done = self.finished.clone();
        // Finalizing can take a moment; keep the UI responsive.
        std::thread::spawn(move || {
            let r = objc2::rc::autoreleasepool(|_| sess.finish()).map_err(|e| format!("{e:#}"));
            *done.lock().unwrap() = Some((file, r));
            let _ = std::fs::remove_file(record::pidfile_path());
            wake();
        });
    }

    fn export_gif(&mut self) {
        let Some((file, _)) = self.st.gif_source.clone() else { return };
        if self.st.gif_job.is_some() {
            return;
        }
        let out = crate::gif::gif_path(&file);
        let opts = self.st.gif_opts.clone();
        let progress = Arc::new(std::sync::atomic::AtomicU32::new(0));
        self.st.gif_progress = Some(progress.clone());
        self.st.gif_job = Some(std::thread::spawn(move || {
            let r = objc2::rc::autoreleasepool(|_| {
                crate::gif::export(&file, &out, &opts, &|p| {
                    progress.store((p * 1000.0) as u32, Ordering::Relaxed);
                    wake();
                })
            });
            wake();
            r.map(|s| (out, s))
        }));
    }

    fn toggle_pause(&mut self) {
        if let Some(s) = &self.st.recording {
            if s.is_paused() {
                s.resume();
            } else {
                s.pause();
            }
        }
    }

    fn quit(&mut self, el: &ActiveEventLoop) {
        if let Some(s) = self.st.recording.take() {
            let file = s.output.clone();
            let r = objc2::rc::autoreleasepool(|_| s.finish());
            if self.selftest.is_some() {
                println!("selftest: saved on quit={file:?} ok={}", r.is_ok());
            }
            let _ = std::fs::remove_file(record::pidfile_path());
        }
        while self.st.finishing && self.finished.lock().unwrap().is_none() {
            std::thread::sleep(Duration::from_millis(20));
        }
        if let Some(s) = self.st.preview.take() {
            let _ = s.finish();
        }
        self.st.live.stop_all();
        for h in self.st.replay_saves.drain(..) {
            let _ = h.join();
        }
        dock::allow_terminate();
        el.exit();
    }

    fn handle(&mut self, el: &ActiveEventLoop, c: Cmd) {
        match c {
            Cmd::ToggleRecording => self.toggle_recording(el),
            Cmd::RestartRecording | Cmd::DeleteRecording if self.st.recording.is_some() => {
                self.after_finish = Some(if c == Cmd::RestartRecording { AfterFinish::Restart } else { AfterFinish::Delete });
                self.stop_recording();
            }
            Cmd::RestartRecording | Cmd::DeleteRecording => {}
            Cmd::ExportGif => self.export_gif(),
            Cmd::TogglePause => self.toggle_pause(),
            Cmd::ShowWindow => {
                self.main_hidden_for_recording = false;
                self.show_main(el)
            }
            Cmd::ShowSettings => self.show_settings(el),
            Cmd::RevealLast => {
                if let Some(f) = &self.st.last_file {
                    macos::reveal(f);
                }
            }
            Cmd::OpenFolder => macos::open_folder(&self.st.settings.output_dir),
            Cmd::SaveReplay => self.save_replay(),
            Cmd::ToggleStream => self.toggle_stream(),
            Cmd::ReportProblem => self.report_problem(),
            Cmd::OpenLog => macos::reveal(&crate::logging::log_path()),
            Cmd::NextScene => self.cycle_scene(1),
            Cmd::PrevScene => self.cycle_scene(-1),
            Cmd::Transition => self.transition(),
            Cmd::Quit => self.quit(el),
        }
    }

    /// Periodic work: commands, signals, finished recordings, failures,
    /// menu bar, meters.
    fn tick(&mut self, el: &ActiveEventLoop) {
        // REC_SHOW_MENU: open the menu bar menu once, 3 s after launch
        // (for screenshots). It tracks modally until dismissed.
        if !self.menu_shown && std::env::var_os("REC_SHOW_MENU").is_some() && self.launched.elapsed() > Duration::from_secs(3) {
            self.menu_shown = true;
            if let Some(m) = &self.menubar {
                m.pop_up();
            }
        }
        if let Some(mut t) = self.selftest.take() {
            t.step(self);
            self.selftest = Some(t);
        }
        if SIG_QUIT.swap(false, Ordering::SeqCst) {
            self.quit(el);
            return;
        }
        if SIG_STOP.swap(false, Ordering::SeqCst) && self.st.recording.is_some() {
            self.stop_recording();
        }
        if self.st.recording.as_ref().is_some_and(|s| s.has_frames()) {
            if record::PAUSE_REQ.swap(false, Ordering::SeqCst) {
                self.st.recording.as_ref().unwrap().pause();
            }
            if record::RESUME_REQ.swap(false, Ordering::SeqCst) {
                self.st.recording.as_ref().unwrap().resume();
            }
        }
        // `simple-record save-replay` (SIGINFO).
        if record::REPLAY_REQ.swap(false, Ordering::SeqCst) {
            push(Cmd::SaveReplay);
        }
        self.sync_live();
        self.sync_hotkeys();
        let cmds: Vec<Cmd> = std::mem::take(&mut *QUEUE.lock().unwrap());
        let ui_cmds = std::mem::take(&mut self.st.cmds);
        for c in cmds.into_iter().chain(ui_cmds) {
            self.handle(el, c);
        }
        let (finished, running): (Vec<_>, Vec<_>) = self.st.replay_saves.drain(..).partition(|h| h.is_finished());
        self.st.replay_saves = running;
        for h in finished {
            match h.join() {
                Ok(Ok(p)) => {
                    eprintln!("rec: replay saved {}", p.display());
                    self.st.notice = Some((format!("Replay saved: {}", p.file_name().unwrap_or_default().to_string_lossy()), Instant::now()));
                    self.st.last_file = Some(p);
                }
                Ok(Err(e)) => self.st.error = Some(format!("Replay: {e:#}")),
                Err(_) => self.st.error = Some("Replay: the writer crashed".into()),
            }
        }
        if self.st.gif_job.as_ref().is_some_and(|h| h.is_finished()) {
            let h = self.st.gif_job.take().unwrap();
            self.st.gif_progress = None;
            match h.join() {
                Ok(Ok((out, s))) => {
                    eprintln!("rec: GIF saved {} ({} frames, {} bytes)", out.display(), s.frames, s.bytes);
                    let name = out.file_name().unwrap_or_default().to_string_lossy().into_owned();
                    self.st.notice = Some((format!("GIF saved: {name} ({:.1} MB)", s.bytes as f64 / 1e6), Instant::now()));
                    macos::reveal(&out);
                }
                Ok(Err(e)) => self.st.error = Some(format!("GIF export: {e:#}")),
                Err(_) => self.st.error = Some("GIF export: the exporter crashed".into()),
            }
        }
        let done = self.finished.lock().unwrap().take();
        if let Some((file, r)) = done {
            self.st.finishing = false;
            let restarting = self.after_finish == Some(AfterFinish::Restart);
            if self.main_hidden_for_recording && !restarting {
                self.main_hidden_for_recording = false;
                self.show_main(el);
            }
            match (r, self.after_finish.take()) {
                (r, Some(then)) => {
                    // Thrown away from the controls: to the Trash (not
                    // deleted, in case it was a slip), with its camera file.
                    let camera = r.ok().and_then(|s| s.camera_file);
                    let mut trashed = false;
                    for f in std::iter::once(&file).chain(camera.as_ref()).filter(|f| f.exists()) {
                        match macos::trash(f) {
                            Ok(()) => {
                                eprintln!("rec: moved to the Trash {}", f.display());
                                trashed = true;
                            }
                            Err(e) => self.st.error = Some(format!("Could not move {} to the Trash: {e}", f.display())),
                        }
                    }
                    if trashed {
                        self.st.notice = Some(("Recording moved to the Trash".into(), Instant::now()));
                    }
                    if then == AfterFinish::Restart {
                        self.start_recording(el);
                    }
                }
                (Ok(stats), None) => {
                    if let Some(c) = &stats.camera_file {
                        eprintln!("rec: camera saved {}", c.display());
                    }
                    self.st.last_file = Some(file);
                }
                (Err(e), None) => self.st.error = Some(format!("Recording failed: {e}")),
            }
        }
        let failed = self.st.recording.as_ref().is_some_and(|s| s.failed());
        if failed {
            let err = self.st.recording.as_ref().and_then(|s| s.error()).unwrap_or_default();
            self.stop_recording();
            self.st.error = Some(format!("Recording stopped: {err}"));
        }
        if self.st.preview.as_ref().is_some_and(|s| s.failed()) {
            let err = self.st.preview.as_ref().and_then(|s| s.error()).unwrap_or_default();
            if let Some(s) = self.st.preview.take() {
                let _ = s.finish();
            }
            self.st.error = Some(format!("Preview stopped: {err}"));
        }
        if self.st.refresh_devices {
            self.st.refresh_devices = false;
            load_devices(&mut self.st);
        }
        if DEVICES_CHANGED.swap(false, Ordering::SeqCst) {
            self.devices_changed();
        }
        // Meters.
        let dt = self.last_meter.elapsed().as_secs_f32();
        self.last_meter = Instant::now();
        let sess = self.st.recording.as_ref().or(self.st.preview.as_ref());
        let lv = sess.map(|s| s.poll().1).unwrap_or([0.0; 2]);
        for (m, l) in self.st.meters.iter_mut().zip(lv) {
            m.feed(l, dt);
        }
        self.permissions_changed();
        self.st.onboarding = onboarding::wanted(&mut self.st);
        self.sync_preview();
        self.sync_controls(el);
        self.st.apply_pip();
        self.st.apply_scene();
        let clock = self.st.clock();
        let view = menubar::View {
            recording: self.st.recording.is_some(),
            paused: self.st.is_paused(),
            finishing: self.st.finishing,
            clock: &clock,
            has_last: self.st.last_file.is_some(),
            replay: self.st.replay_stats.is_some(),
            live: self.st.streaming(),
        };
        if let Some(m) = &mut self.menubar {
            m.update(&view);
        }
    }

    fn kind(&self, id: WindowId) -> Option<Kind> {
        let is = |w: &Option<Win>| w.as_ref().is_some_and(|w| w.window.id() == id);
        if is(&self.main) {
            Some(Kind::Main)
        } else if is(&self.settings_win) {
            Some(Kind::Settings)
        } else if is(&self.controls) {
            Some(Kind::Controls)
        } else {
            None
        }
    }

    fn redraw(&mut self, which: WindowId) {
        let Some(kind) = self.kind(which) else { return };
        let is_main = kind == Kind::Main;
        let Some(gpu) = self.gpu.as_ref() else { return };
        preview::set_low_power(self.st.settings.preview_low_power);
        if is_main
            && let Some(f) = self.scaler.take(&gpu.device)
            && let Some(w) = self.main.as_mut()
        {
            self.st.preview_aspect = f.size[0] as f32 / f.size[1] as f32;
            self.st.preview_px = f.size;
            let view = f.texture.create_view(&Default::default());
            // Drawn pixel-aligned at its own size: nearest is exact, and
            // avoids a blur when the area is mid-resize.
            let filter = wgpu::FilterMode::Nearest;
            match self.st.preview_tex {
                Some(id) => w.renderer.update_egui_texture_from_wgpu_texture(&gpu.device, &view, filter, id),
                None => self.st.preview_tex = Some(w.renderer.register_native_texture(&gpu.device, &view, filter)),
            }
            self.preview_textures.push_back(f.texture);
            while self.preview_textures.len() > 2 {
                self.preview_textures.pop_front();
            }
            self.st.frames_shown += 1;
        }
        let win = match kind {
            Kind::Main => self.main.as_mut(),
            Kind::Settings => self.settings_win.as_mut(),
            Kind::Controls => self.controls.as_mut(),
        };
        let Some(win) = win else { return };
        let input = win.egui.take_egui_input(&win.window);
        let st = &mut self.st;
        let mut drag_controls = false;
        let out = win.ctx.run_ui(input, |ui| match kind {
            Kind::Main => ui::main_window(ui, st),
            Kind::Settings => ui::settings_window(ui, st),
            Kind::Controls => drag_controls = controls::bar(ui, st),
        });
        if kind == Kind::Controls {
            let solid = win.ctx.input(|i| i.pointer.has_pointer()) || st.controls_confirm.is_some();
            macos::set_alpha(&win.window, if solid { 1.0 } else { controls::IDLE_ALPHA });
        }
        win.egui.handle_platform_output(&win.window, out.platform_output);
        let delay = out.viewport_output.get(&egui::ViewportId::ROOT).map_or(Duration::MAX, |v| v.repaint_delay);
        win.repaint_at = if delay < Duration::from_secs(60) { Some(Instant::now() + delay) } else { None };
        let ppp = out.pixels_per_point;
        let prims = win.ctx.tessellate(out.shapes, ppp);
        // Upload every frame, even when the surface frame is skipped, or a
        // later partial update targets a texture that was never allocated.
        for (id, deltas) in &out.textures_delta.set {
            for d in deltas {
                win.renderer.update_texture(&gpu.device, &gpu.queue, *id, d);
            }
        }
        let frame = match win.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            _ => {
                win.surface.configure(&gpu.device, &win.config);
                for id in &out.textures_delta.free {
                    win.renderer.free_texture(id);
                }
                return;
            }
        };
        let (x, y) = theme::TRAFFIC_LIGHTS;
        macos::inset_traffic_lights(&win.window, x, y);
        let view = frame.texture.create_view(&Default::default());
        let sd = egui_wgpu::ScreenDescriptor { size_in_pixels: [win.config.width, win.config.height], pixels_per_point: ppp };
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        let extra = win.renderer.update_buffers(&gpu.device, &gpu.queue, &mut enc, &prims, &sd);
        {
            let bg = theme::palette(win.ctx.theme() == egui::Theme::Dark).window;
            let [r, g, b, _] = bg.to_normalized_gamma_f32();
            // The controls are a floating bar: clear around it.
            let a = if kind == Kind::Controls { 0.0 } else { 1.0 };
            let (r, g, b) = (r * a, g * a, b * a);
            let mut pass = enc
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color { r: r as f64, g: g as f64, b: b as f64, a: a as f64 }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            win.renderer.render(&mut pass, &prims, &sd);
        }
        gpu.queue.submit(extra.into_iter().chain([enc.finish()]));
        win.window.pre_present_notify();
        gpu.queue.present(frame);
        for id in &out.textures_delta.free {
            win.renderer.free_texture(id);
        }
        // Drag the window by the toolbar (the titlebar is transparent), or
        // the controls by their grip.
        if (kind != Kind::Controls && ui::take_drag_request()) || drag_controls {
            let _ = win.window.drag_window();
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.menubar.is_some() {
            return;
        }
        let mtm = objc2::MainThreadMarker::new().expect("main thread");
        self.menubar = Some(menubar::MenuBar::new(mtm));
        self.sync_hotkeys();
        self._main_menu = Some(menubar::install_main_menu(mtm));
        macos::set_dock_icon();
        if std::env::var_os("REC_GUI_HIDDEN").is_some() {
            self.main_visible = false;
        } else {
            self.show_main(el);
        }
        if self.start_recording {
            push(Cmd::ToggleRecording);
        }
        match self.st.settings.hotkey(crate::hotkey::Action::Record) {
            Some(c) => eprintln!("rec: app running ({} toggles recording)", c.label()),
            None => eprintln!("rec: app running"),
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, _: ()) {
        self.tick(el);
        if let Some(w) = &self.main
            && self.main_visible
            && !w.occluded
        {
            w.window.request_redraw();
        }
        for w in [&self.settings_win, &self.controls].into_iter().flatten() {
            w.window.request_redraw();
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(kind) = self.kind(id) else { return };
        let is_main = kind == Kind::Main;
        let win = match kind {
            Kind::Main => self.main.as_mut(),
            Kind::Settings => self.settings_win.as_mut(),
            Kind::Controls => self.controls.as_mut(),
        };
        let Some(win) = win else { return };
        let resp = win.egui.on_window_event(&win.window, &event);
        match event {
            WindowEvent::CloseRequested => match kind {
                Kind::Main => self.hide_main(),
                Kind::Settings => self.settings_win = None,
                // Not closable (no title bar); Stop or Settings hide it.
                Kind::Controls => {}
            },
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                let s = win.window.inner_size();
                win.config.width = s.width.max(1);
                win.config.height = s.height.max(1);
                if let Some(g) = &self.gpu {
                    win.surface.configure(&g.device, &win.config);
                }
                win.window.request_redraw();
            }
            WindowEvent::Occluded(o) => {
                win.occluded = o;
                if !o {
                    win.window.request_redraw();
                }
                if is_main {
                    preview::set_wanted(self.main_visible && !o);
                }
            }
            WindowEvent::ThemeChanged(_) => {
                theme::set_accent(theme::system_accent());
                theme::apply(&win.ctx);
                win.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                self.redraw(id);
                if !self.st.cmds.is_empty() {
                    self.tick(el);
                }
            }
            _ => {
                if resp.repaint {
                    win.window.request_redraw();
                }
            }
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if self.last_tick.elapsed() >= Duration::from_millis(100) {
            self.last_tick = Instant::now();
            self.tick(el);
        }
        // Animate while capturing (preview, meters, timer); otherwise idle
        // with a slow tick for signals.
        let live = self.st.recording.is_some() || self.st.preview.is_some() || self.st.finishing || self.st.streaming();
        let now = Instant::now();
        let mut next = now + Duration::from_millis(if live { 100 } else { 250 });
        // The occlusion event only reports changes (not a window created
        // while the screen is locked): ask AppKit each pass.
        for (w, main) in [(self.main.as_mut(), true), (self.settings_win.as_mut(), false), (self.controls.as_mut(), false)] {
            if let Some(w) = w {
                let o = macos::is_occluded(&w.window);
                if o != w.occluded {
                    w.occluded = o;
                    if main {
                        preview::set_wanted(self.main_visible && !o);
                    }
                }
            }
        }
        for w in [self.main.as_mut(), self.settings_win.as_mut(), self.controls.as_mut()].into_iter().flatten() {
            let visible = w.window.is_visible().unwrap_or(true) && !w.occluded;
            if live && visible {
                // Timer, meters: ~15 fps; preview frames wake us on arrival.
                w.window.request_redraw();
            }
            if let Some(t) = w.repaint_at.filter(|_| !w.occluded) {
                if t <= now {
                    w.repaint_at = None;
                    w.window.request_redraw();
                } else {
                    next = next.min(t);
                }
            }
        }
        el.set_control_flow(ControlFlow::WaitUntil(next));
    }
}

/// REC_GUI_SELFTEST=1: drive the real app paths on a timer (preview,
/// record via the hotkey's command, pause/resume, stop, settings window,
/// close) and print what happened.
#[derive(Default)]
struct SelfTest {
    t0: Option<Instant>,
    step: u32,
    /// Start time of the preview session before an overlay change.
    session_before: Option<Instant>,
}

impl SelfTest {
    fn step(&mut self, app: &mut App) {
        let t = self.t0.get_or_insert_with(Instant::now).elapsed().as_secs_f64();
        // REC_GUI_SELFTEST=terminate: quit the way the Dock does
        // (-[NSApp terminate:]) mid-recording; the file must be finalized.
        if std::env::var("REC_GUI_SELFTEST").as_deref() == Ok("terminate") {
            match self.step {
                0 if t > 2.0 => {
                    push(Cmd::ToggleRecording);
                    self.step = 1;
                }
                1 if t > 5.0 => {
                    println!("selftest: recording={} -> NSApp terminate:", app.st.recording.is_some());
                    let mtm = objc2::MainThreadMarker::new().unwrap();
                    objc2_app_kit::NSApplication::sharedApplication(mtm).terminate(None);
                    self.step = 2;
                }
                _ => {}
            }
            return;
        }
        // REC_GUI_SELFTEST=live: replay buffer and (if the settings name a
        // destination) a stream keep running across window close and a
        // recording; then the replay is saved with the hotkey's command.
        if std::env::var("REC_GUI_SELFTEST").as_deref() == Ok("live") {
            let streaming = !app.st.settings.stream_url().is_empty();
            match self.step {
                0 if t > 2.0 => {
                    println!("selftest: replay on={}", app.st.replay_stats.is_some());
                    if streaming {
                        push(Cmd::ToggleStream);
                    }
                    self.step = 1;
                }
                1 if t > 4.0 => {
                    app.hide_main();
                    println!("selftest: window hidden, live session={}", app.st.preview.is_some());
                    push(Cmd::ToggleRecording);
                    self.step = 2;
                }
                2 if t > 7.0 => {
                    push(Cmd::ToggleRecording);
                    self.step = 3;
                }
                3 if t > 9.0 && !app.st.finishing => {
                    println!("selftest: after recording live session={} saved={:?}", app.st.preview.is_some(), app.st.last_file);
                    push(Cmd::SaveReplay);
                    self.step = 4;
                }
                4 if app.st.notice.is_some() || app.st.error.is_some() || t > 25.0 => {
                    let held = app.st.replay_stats.map_or(0.0, |r| r.0);
                    println!("selftest: replay saved={:?} held={held:.1}s error={:?}", app.st.last_file, app.st.error);
                    for s in &app.st.stream_statuses {
                        println!(
                            "selftest: stream live={} state={} bitrate={:.1}Mb/s dropped={} reconnects={}",
                            s.live,
                            s.state,
                            s.bitrate / 1e6,
                            s.dropped_frames,
                            s.reconnects
                        );
                    }
                    push(Cmd::Quit);
                    self.step = 5;
                }
                _ => {}
            }
            return;
        }
        match self.step {
            0 if t > 3.0 => {
                println!("selftest: idle preview session={} frame_shown={}", app.st.preview.is_some(), app.st.frames_shown > 0);
                app.st.frames_shown = 0;
                push(Cmd::ToggleRecording);
                self.step = 1;
            }
            1 if t > 4.0 => {
                println!("selftest: recording={} controls={} error={:?}", app.st.recording.is_some(), app.controls.is_some(), app.st.error);
                push(Cmd::TogglePause);
                self.step = 2;
            }
            2 if t > 5.5 => {
                println!("selftest: paused={}", app.st.is_paused());
                push(Cmd::TogglePause);
                self.step = 3;
            }
            3 if t > 8.0 => {
                println!("selftest: preview during recording frame_shown={}", app.st.frames_shown > 0);
                println!(
                    "selftest: status title={:?} recorded={:.2}s wall={:.2}s",
                    app.menubar.as_ref().and_then(|m| m.title()),
                    app.st.recording.as_ref().map_or(0.0, |s| s.recorded_secs()),
                    app.st.recording.as_ref().map_or(0.0, |s| s.started.elapsed().as_secs_f64())
                );
                push(Cmd::ToggleRecording);
                self.step = 4;
            }
            4 if !app.st.finishing && app.st.recording.is_none() => {
                println!("selftest: saved={:?} controls={} error={:?}", app.st.last_file, app.controls.is_some(), app.st.error);
                push(Cmd::ShowSettings);
                self.step = 5;
            }
            5 if t > 9.0 && app.st.preview.is_some() => {
                // Move and restyle the overlay: must apply live, not restart.
                self.session_before = app.st.preview.as_ref().map(|s| s.started);
                app.st.settings.pip_place = crate::compose::Place::At([0.3, 0.4]);
                app.st.settings.pip_shape = crate::compose::Shape::Squircle;
                self.step = 6;
            }
            6 if t > 10.0 => {
                let now = app.st.preview.as_ref().map(|s| s.started);
                println!(
                    "selftest: overlay change live={} applied={}",
                    now.is_some() && now == self.session_before,
                    app.st.pip_applied.as_ref() == Some(&app.st.settings.pip_spec())
                );
                println!(
                    "selftest: settings window={} preview_restarted={}",
                    app.settings_win.is_some(),
                    app.st.preview.is_some()
                );
                app.hide_main();
                println!("selftest: after close preview_session={}", app.st.preview.is_some());
                push(Cmd::Quit);
                self.step = 7;
            }
            _ => {}
        }
    }
}

pub fn run(start_recording: bool, settings: bool, window: Option<u32>) -> Result<()> {
    use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
    // Opened from Finder, stderr goes nowhere: keep it in the log too.
    crate::logging::start();
    crate::logging::install_panic_hook();
    let crashed = crate::logging::take_last_crash();
    unsafe {
        libc::signal(libc::SIGINT, on_sigint as *const () as usize);
        libc::signal(libc::SIGTERM, on_sigterm as *const () as usize);
        libc::signal(libc::SIGHUP, on_sigterm as *const () as usize);
    }
    record::install_pause_signals();
    let el = EventLoop::<()>::with_user_event()
        // A regular app: Dock icon (support/AppIcon.icns) and a menu bar.
        .with_activation_policy(ActivationPolicy::Regular)
        .with_default_menu(false)
        .build()?;
    let _ = PROXY.set(Mutex::new(el.create_proxy()));
    dock::install();
    let mut app = App::new(start_recording);
    if let Some(c) = crashed {
        eprintln!("rec: the last run ended with a crash: {}", c.lines().nth(1).unwrap_or_default());
        // Its own banner: an error would also hold back the preview.
        app.st.crashed_last_time = true;
    }
    app.st.settings.window = window;
    if settings {
        push(Cmd::ShowSettings);
    }
    let r = el.run_app(&mut app);
    let _ = std::fs::remove_file(record::pidfile_path());
    eprintln!("rec: app quit");
    crate::logging::finish();
    r?;
    Ok(())
}

/// The GUI wording for a "missing permission(s)" error from the capture
/// layer, whose own text names a command-line tool.
fn permission_hint(msg: &str) -> Option<String> {
    let list = msg.split("missing permission(s): ").nth(1)?.lines().next()?.trim();
    Some(format!("SimpleRecord needs your permission for: {list}. Click Request Permissions, then turn SimpleRecord on in System Settings > Privacy & Security."))
}
