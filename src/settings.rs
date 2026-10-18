//! App settings, persisted as JSON in ~/Library/Application Support/rec-rs.
//! The app turns them into the same `record` arguments the CLI takes, so
//! both share one code path (`crate::prepare`).

use std::collections::BTreeMap;
use std::path::PathBuf;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::compose::{PipSpec, Place, Scene, Shape};
use crate::hotkey::{Action, Combo};
use crate::layout::{Background, Canvas, Layout, SceneSpec, Transition};
use crate::writer::{AudioLayout, Codec};

pub const RESOLUTIONS: [(&str, &str); 5] =
    [("720p", "720p"), ("1080p", "1080p"), ("1440p", "1440p"), ("4K", "4k"), ("Native", "native")];
pub const FPS: [u32; 5] = [24, 25, 30, 50, 60];

/// An extra stream destination (the first is `stream_dest`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StreamDest {
    /// "youtube", "twitch" or "custom".
    pub dest: String,
    pub custom_url: String,
    pub enabled: bool,
}

impl Default for StreamDest {
    fn default() -> Self {
        StreamDest { dest: "twitch".into(), custom_url: String::new(), enabled: true }
    }
}

/// The ingest URL for a destination choice.
pub fn dest_url(dest: &str, custom_url: &str) -> String {
    match dest {
        "custom" => custom_url.trim().to_string(),
        d => crate::live::destination_url(d),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub output_dir: PathBuf,
    /// "mov" or "mp4".
    pub container: String,
    /// None = automatic (H.264 up to 1080p, HEVC above).
    pub codec: Option<Codec>,
    pub resolution: String,
    pub fps: u32,
    /// Mb/s; None = automatic.
    pub bitrate_mbps: Option<f64>,
    /// 1-based display number.
    pub display: usize,
    /// Record only this part of the display: [x, y, width, height] in
    /// points from its top-left corner.
    pub region: Option<[f64; 4]>,
    /// Record every window of this app (bundle ID) on the display, and
    /// nothing else.
    pub app: Option<String>,
    pub scene: Scene,
    /// Shape of the output frame.
    pub canvas: Canvas,
    /// Behind scene layers that don't cover the frame.
    pub background: Background,
    pub transition: Transition,
    pub transition_ms: u32,
    /// The user's own scenes.
    pub custom_scenes: Vec<Layout>,
    /// The custom scene shown instead of the template, if any.
    pub custom_scene: Option<usize>,
    /// Camera unique ID; None = system default.
    pub camera: Option<String>,
    pub camera_resolution: crate::camera::CamRes,
    /// Also save the webcam alone next to each recording.
    pub camera_file: bool,
    pub mic_enabled: bool,
    /// Microphone unique ID; None = system default.
    pub mic: Option<String>,
    /// The mic's display name, to find it again if its unique ID changes.
    pub mic_name: Option<String>,
    pub system_audio: bool,
    pub audio_tracks: AudioLayout,
    pub system_gain_db: f32,
    pub mic_gain_db: f32,
    /// Noise suppression, gate, compressor, low-cut (applied live).
    pub mic_filters: crate::filters::MicFilters,
    pub pip_size: f64,
    pub pip_place: Place,
    pub pip_shape: Shape,
    pub pip_roundness: f64,
    pub pip_border: f64,
    pub pip_border_color: [u8; 3],
    pub pip_shadow: bool,
    pub mirror: bool,
    pub cursor: bool,
    pub clicks: bool,
    /// Preview at half resolution and 15 fps.
    pub preview_low_power: bool,
    /// Keep the last `replay_secs` of the live encode in memory.
    pub replay_enabled: bool,
    pub replay_secs: u32,
    /// "youtube", "twitch", or an rtmp(s):// or WHIP http(s):// URL. The
    /// stream key lives in the Keychain, never here.
    pub stream_dest: String,
    pub stream_custom_url: String,
    /// More destinations streamed at the same time, from the same encode.
    pub stream_extra: Vec<StreamDest>,
    /// Live encode (replay buffer and streams).
    pub live_resolution: String,
    pub stream_bitrate_kbps: u32,
    /// Use the platform's recommended bitrate (`live::preset`) instead of
    /// `stream_bitrate_kbps`.
    pub stream_bitrate_auto: bool,
    /// "auto" (HEVC for YouTube, else H.264), "h264" or "hevc".
    pub stream_codec: String,
    pub keyframe_secs: f64,
    pub stream_cbr: bool,
    pub stream_audio_kbps: u32,
    /// Global shortcuts; an action missing here has its default, None
    /// means none.
    pub hotkeys: BTreeMap<Action, Option<Combo>>,
    /// Window to record (ID from `rec-rs devices`); not persisted because
    /// window IDs change between launches.
    #[serde(skip)]
    pub window: Option<u32>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            output_dir: default_output_dir(),
            container: "mov".into(),
            codec: None,
            resolution: "1080p".into(),
            fps: 60,
            bitrate_mbps: None,
            display: 1,
            region: None,
            app: None,
            scene: Scene::Screen,
            canvas: Canvas::Source,
            background: Background::default(),
            transition: Transition::Move,
            transition_ms: 400,
            custom_scenes: vec![],
            custom_scene: None,
            camera: None,
            camera_resolution: Default::default(),
            camera_file: false,
            mic_enabled: true,
            mic: None,
            mic_name: None,
            system_audio: true,
            audio_tracks: AudioLayout::Both,
            system_gain_db: 0.0,
            mic_gain_db: 0.0,
            mic_filters: Default::default(),
            pip_size: PipSpec::default().size,
            pip_place: PipSpec::default().place,
            pip_shape: PipSpec::default().shape,
            pip_roundness: PipSpec::default().roundness,
            pip_border: 0.0,
            pip_border_color: [255, 255, 255],
            pip_shadow: false,
            mirror: true,
            cursor: true,
            clicks: false,
            preview_low_power: false,
            replay_enabled: false,
            replay_secs: 60,
            stream_dest: "youtube".into(),
            stream_custom_url: String::new(),
            stream_extra: vec![],
            live_resolution: "1080p".into(),
            stream_bitrate_kbps: 6000,
            stream_bitrate_auto: true,
            stream_codec: "auto".into(),
            keyframe_secs: 2.0,
            stream_cbr: true,
            stream_audio_kbps: 192,
            hotkeys: BTreeMap::new(),
            window: None,
        }
    }
}

/// `PREFIX-YYYY-MM-DD-HHMMSS.EXT` in local time.
pub fn timestamped(prefix: &str, ext: &str) -> String {
    let mut buf = [0u8; 64];
    let n = unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        libc::strftime(buf.as_mut_ptr() as *mut _, buf.len(), c"%Y-%m-%d-%H%M%S".as_ptr(), &tm)
    };
    format!("{prefix}-{}.{ext}", String::from_utf8_lossy(&buf[..n]))
}

pub fn default_output_dir() -> PathBuf {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Movies")).unwrap_or_default()
}

fn value<T: ValueEnum>(v: T) -> String {
    v.to_possible_value().map(|p| p.get_name().to_string()).unwrap_or_default()
}

impl Settings {
    /// $REC_SETTINGS, else ~/Library/Application Support/rec-rs/settings.json.
    pub fn path() -> PathBuf {
        if let Some(p) = std::env::var_os("REC_SETTINGS") {
            return PathBuf::from(p);
        }
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        home.join("Library/Application Support/rec-rs/settings.json")
    }

    pub fn load() -> Self {
        let mut s: Settings = std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        // 160 kb/s was the default live audio bitrate; 192 is now.
        if s.stream_audio_kbps == 160 {
            s.stream_audio_kbps = 192;
        }
        // REC_OUTPUT_DIR overrides the folder (self-tests keep files out of ~/Movies).
        if let Some(d) = std::env::var_os("REC_OUTPUT_DIR") {
            s.output_dir = PathBuf::from(d);
        }
        s
    }

    pub fn save(&self) -> std::io::Result<()> {
        let p = Self::path();
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(p, serde_json::to_string_pretty(self).unwrap_or_default())
    }

    /// The camera overlay these settings describe.
    pub fn pip_spec(&self) -> PipSpec {
        PipSpec {
            size: self.pip_size,
            place: self.pip_place,
            shape: self.pip_shape,
            roundness: self.pip_roundness,
            border: self.pip_border,
            border_color: self.pip_border_color,
            shadow: self.pip_shadow,
            mirror: self.mirror,
        }
    }

    /// The global shortcut for `a`, if any.
    pub fn hotkey(&self, a: Action) -> Option<Combo> {
        self.hotkeys.get(&a).copied().unwrap_or_else(|| a.default_combo())
    }

    /// Bind `combo` to `a`, taking it from any other action.
    pub fn set_hotkey(&mut self, a: Action, combo: Option<Combo>) {
        if let Some(c) = combo {
            for other in Action::ALL {
                if other != a && self.hotkey(other) == Some(c) {
                    self.hotkeys.insert(other, None);
                }
            }
        }
        self.hotkeys.insert(a, combo);
    }

    /// The scene these settings show.
    pub fn scene_spec(&self) -> SceneSpec {
        match self.custom_scene.and_then(|i| self.custom_scenes.get(i)) {
            Some(l) => SceneSpec::Custom(l.clone()),
            None => SceneSpec::Template(self.scene),
        }
    }

    /// Which scene is shown (a template, or a custom scene by index):
    /// switching between these animates, editing one doesn't.
    pub fn scene_id(&self) -> (Scene, Option<usize>) {
        match self.custom_scene.filter(|i| *i < self.custom_scenes.len()) {
            Some(i) => (Scene::Screen, Some(i)),
            None => (self.scene, None),
        }
    }

    /// The settings that need a new capture session when they change. The
    /// overlay's look and position, and switches between scenes that the
    /// session can show (with the camera on, or composed), are applied live
    /// instead.
    pub fn capture_key(&self) -> (Settings, bool, Vec<usize>) {
        let d = Settings::default();
        let spec = self.scene_spec();
        // A scene without the camera that still needs the compositor.
        let composed = !spec.uses_cam() && spec != SceneSpec::Template(Scene::Screen);
        let key = Settings {
            pip_size: d.pip_size,
            pip_place: d.pip_place,
            pip_shape: d.pip_shape,
            pip_roundness: d.pip_roundness,
            pip_border: d.pip_border,
            pip_border_color: d.pip_border_color,
            pip_shadow: d.pip_shadow,
            mirror: d.mirror,
            // Scenes switch live (their look too), as long as the camera is
            // on for all of them.
            scene: if spec.uses_cam() { Scene::ScreenCam } else { Scene::Screen },
            custom_scenes: vec![],
            custom_scene: None,
            background: d.background,
            transition: d.transition,
            transition_ms: d.transition_ms,
            preview_low_power: d.preview_low_power,
            replay_enabled: d.replay_enabled,
            replay_secs: d.replay_secs,
            stream_dest: d.stream_dest.clone(),
            stream_custom_url: d.stream_custom_url.clone(),
            stream_extra: d.stream_extra.clone(),
            live_resolution: d.live_resolution.clone(),
            stream_bitrate_kbps: d.stream_bitrate_kbps,
            stream_bitrate_auto: d.stream_bitrate_auto,
            stream_codec: d.stream_codec.clone(),
            keyframe_secs: d.keyframe_secs,
            stream_cbr: d.stream_cbr,
            stream_audio_kbps: d.stream_audio_kbps,
            hotkeys: d.hotkeys.clone(),
            mic_filters: d.mic_filters,
            ..self.clone()
        };
        // Other displays shown need capture streams.
        (key, composed, spec.extra_displays())
    }

    /// The stream destination URL (the YouTube and Twitch ingest URLs for
    /// those choices).
    pub fn stream_url(&self) -> String {
        dest_url(&self.stream_dest, &self.stream_custom_url)
    }

    /// Every destination Go Live starts: the first one and the enabled
    /// extras, without blanks or repeats.
    pub fn stream_urls(&self) -> Vec<String> {
        let mut urls = vec![self.stream_url()];
        urls.extend(self.stream_extra.iter().filter(|d| d.enabled).map(|d| dest_url(&d.dest, &d.custom_url)));
        let mut seen = std::collections::HashSet::new();
        urls.into_iter().filter(|u| !u.is_empty() && seen.insert(u.clone())).collect()
    }

    /// The live encode these settings describe (sizes above the capture
    /// are clamped to it by the encoder): the platform's limits, codec and
    /// recommended bitrate unless set by hand.
    pub fn live_config(&self) -> crate::live::LiveConfig {
        use crate::live::{Platform, platform, preset};
        let height = match self.live_resolution.as_str() {
            "720p" => 720,
            "1440p" => 1440,
            "4k" => 2160,
            _ => 1080,
        };
        // Every destination's limits: the smallest bitrate and size, HEVC
        // only if all of them take it.
        let mut platforms: Vec<Platform> = self.stream_urls().iter().map(|u| platform(u)).collect();
        if platforms.is_empty() {
            platforms.push(Platform::Other);
        }
        let hevc = match self.stream_codec.as_str() {
            "hevc" => true,
            "h264" => false,
            _ => platforms.iter().all(|p| *p == Platform::YouTube),
        };
        let presets: Vec<(u32, usize, bool)> = platforms.iter().map(|p| preset(*p, height, self.fps.min(60), hevc)).collect();
        let kbps = presets.iter().map(|p| p.0).min().unwrap_or(6000);
        let height = presets.iter().map(|p| p.1).min().unwrap_or(height);
        let hevc = presets.iter().all(|p| p.2);
        let bitrate = if self.stream_bitrate_auto { kbps } else { self.stream_bitrate_kbps.clamp(500, 80_000) };
        crate::live::LiveConfig {
            width: height * 16 / 9,
            height,
            bitrate: bitrate as u64 * 1000,
            keyframe_secs: self.keyframe_secs.clamp(0.5, 4.0),
            cbr: self.stream_cbr,
            audio_bitrate: self.stream_audio_kbps.clamp(64, 320) * 1000,
            hevc,
        }
    }

    /// A fresh timestamped file in the output folder.
    pub fn new_output_file(&self) -> PathBuf {
        let ext = if self.container == "mp4" && self.codec != Some(Codec::Prores) { "mp4" } else { "mov" };
        self.output_dir.join(timestamped("rec", ext))
    }

    /// `rec-rs record` arguments for these settings (`output` None = preview).
    pub fn record_args(&self, output: Option<&std::path::Path>) -> Vec<String> {
        let mut a: Vec<String> = vec!["-q".into(), "-r".into(), self.resolution.clone()];
        a.extend(["--fps".into(), self.fps.to_string()]);
        a.extend(["--display".into(), self.display.to_string()]);
        if let Some(w) = self.window {
            a.extend(["--window".into(), w.to_string()]);
        } else if let Some(app) = &self.app {
            a.extend(["--app".into(), app.clone()]);
        } else if let Some([x, y, w, h]) = self.region {
            a.extend(["--region".into(), format!("{},{},{},{}", x.round(), y.round(), w.round().max(16.0), h.round().max(16.0))]);
        }
        if let Some(c) = self.codec {
            a.extend(["--codec".into(), value(c)]);
        }
        if let Some(b) = self.bitrate_mbps {
            a.extend(["--bitrate".into(), format!("{b}")]);
        }
        if let Some(o) = output {
            a.extend(["-o".into(), o.to_string_lossy().into_owned()]);
        }
        // A custom scene is set on the options by the app (`App::options`);
        // here it only turns the webcam on.
        let spec = self.scene_spec();
        a.extend(["--scene".into(), value(if self.scene_id().1.is_some() { Scene::Screen } else { self.scene })]);
        a.extend(["--canvas".into(), value(self.canvas)]);
        let hex = |c: [u8; 3]| format!("{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
        a.extend(["--background".into(), format!("{},{}", hex(self.background.top), hex(self.background.bottom))]);
        a.extend(["--transition".into(), value(self.transition)]);
        a.extend(["--transition-ms".into(), self.transition_ms.to_string()]);
        if spec.uses_cam() {
            a.extend(["--webcam".into(), self.camera.clone().unwrap_or_else(|| "default".into())]);
            a.extend(["--camera-resolution".into(), value(self.camera_resolution)]);
            if self.camera_file {
                a.push("--camera-file".into());
            }
        }
        if !self.mic_enabled {
            a.push("--no-mic".into());
        } else if let Some(m) = &self.mic {
            a.extend(["--mic".into(), m.clone()]);
            if let Some(n) = &self.mic_name {
                a.extend(["--mic-name".into(), n.clone()]);
            }
        }
        if !self.system_audio {
            a.push("--no-system-audio".into());
        }
        a.extend(["--audio-tracks".into(), value(self.audio_tracks)]);
        a.extend(["--system-gain".into(), format!("{}", self.system_gain_db)]);
        a.extend(["--mic-gain".into(), format!("{}", self.mic_gain_db)]);
        let f = &self.mic_filters;
        if f.low_cut {
            a.push("--low-cut".into());
        }
        if f.denoise {
            a.push("--denoise".into());
        }
        if f.gate {
            a.extend(["--gate".into(), format!("{}", f.gate_db)]);
        }
        if f.compressor {
            a.extend(["--compressor".into(), format!("{}", f.comp_threshold_db)]);
        }
        a.extend(["--pip-size".into(), format!("{}", self.pip_size)]);
        match self.pip_place {
            Place::Corner(c) => a.extend(["--pip-corner".into(), value(c)]),
            Place::At([x, y]) => a.extend(["--pip-pos".into(), format!("{},{}", x.clamp(0.0, 1.0), y.clamp(0.0, 1.0))]),
        }
        a.extend(["--pip-shape".into(), value(self.pip_shape)]);
        a.extend(["--pip-roundness".into(), format!("{}", self.pip_roundness)]);
        a.extend(["--pip-border".into(), format!("{}", self.pip_border)]);
        let [r, g, b] = self.pip_border_color;
        a.extend(["--pip-border-color".into(), format!("{r:02x}{g:02x}{b:02x}")]);
        if self.pip_shadow {
            a.push("--pip-shadow".into());
        }
        if !self.mirror {
            a.push("--no-mirror".into());
        }
        if !self.cursor {
            a.push("--no-cursor".into());
        }
        if self.clicks {
            a.push("--clicks".into());
        }
        a
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrip_and_defaults_for_missing_fields() {
        let mut s = Settings { codec: Some(Codec::Hevc), scene: Scene::Cam, bitrate_mbps: Some(40.0), ..Default::default() };
        s.window = Some(7);
        let t = serde_json::to_string(&s).unwrap();
        assert!(t.contains("\"hevc\"") && t.contains("\"cam\""));
        let back: Settings = serde_json::from_str(&t).unwrap();
        assert_eq!(back.window, None, "window IDs are not persisted");
        assert_eq!(Settings { window: None, ..s }, back);
        let partial: Settings = serde_json::from_str(r#"{"fps": 30}"#).unwrap();
        assert_eq!(partial.fps, 30);
        assert_eq!(partial.resolution, "1080p");
    }

    #[test]
    fn record_args_parse_with_the_cli() {
        let s = Settings {
            codec: Some(Codec::Prores),
            scene: Scene::ScreenCam,
            mic: Some("BuiltInMic".into()),
            system_gain_db: -6.0,
            window: Some(42),
            ..Default::default()
        };
        let args = s.record_args(Some(std::path::Path::new("/tmp/x.mov")));
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        crate::parse_record_args(&refs).expect("settings produce valid CLI arguments");
        assert!(args.windows(2).any(|w| w == ["--webcam", "default"]));
        assert!(args.windows(2).any(|w| w == ["--codec", "prores"]));
        assert!(args.windows(2).any(|w| w == ["--window", "42"]));
        // A region applies to the display, not a window.
        let r = Settings { region: Some([10.4, 20.0, 1280.0, 720.0]), ..Default::default() };
        let args = r.record_args(None);
        assert!(args.windows(2).any(|w| w == ["--region", "10,20,1280,720"]));
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        crate::parse_record_args(&refs).unwrap();
        let rw = Settings { window: Some(3), ..r.clone() };
        assert!(!rw.record_args(None).iter().any(|a| a == "--region"));
        // An app replaces the region; the CLI takes it.
        let ra = Settings { app: Some("com.apple.Safari".into()), ..r };
        let args = ra.record_args(None);
        assert!(args.windows(2).any(|w| w == ["--app", "com.apple.Safari"]));
        assert!(!args.iter().any(|a| a == "--region"));
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        crate::parse_record_args(&refs).unwrap();
    }

    #[test]
    fn overlay_settings_round_trip_through_the_cli() {
        let s = Settings {
            scene: Scene::ScreenCam,
            pip_place: Place::At([0.3, 0.6]),
            pip_shape: Shape::Pill,
            pip_roundness: 0.7,
            pip_border: 6.0,
            pip_border_color: [10, 200, 30],
            pip_shadow: true,
            mirror: false,
            ..Default::default()
        };
        let args = s.record_args(None);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let opts = crate::parse_record_args(&refs).unwrap();
        assert_eq!(opts.pip_pos.as_deref(), Some("0.3,0.6"));
        assert_eq!(opts.pip_border_color, "0ac81e");
        assert!(opts.pip_shadow && opts.no_mirror);
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""pip_place":{"at":[0.3,0.6]}"#), "{json}");
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.pip_spec(), s.pip_spec());
    }

    #[test]
    fn overlay_changes_do_not_restart_capture() {
        let a = Settings::default();
        let b = Settings { pip_place: Place::At([0.1, 0.1]), pip_shape: Shape::Circle, pip_shadow: true, ..a.clone() };
        assert_eq!(a.capture_key(), b.capture_key());
        let c = Settings { fps: 30, ..a.clone() };
        assert_ne!(a.capture_key(), c.capture_key());
        // Camera scenes switch live; turning the camera on restarts.
        let d = Settings { scene: Scene::SideBySide, ..a.clone() };
        let e = Settings { scene: Scene::Reaction, transition: Transition::Fade, ..a.clone() };
        assert_eq!(d.capture_key(), e.capture_key());
        assert_ne!(a.capture_key(), d.capture_key());
        let f = Settings { canvas: Canvas::Portrait, ..a.clone() };
        assert_ne!(a.capture_key(), f.capture_key());
    }

    #[test]
    fn custom_scenes() {
        use crate::layout::{Content, Layer, Layout};
        let cam = Layout { name: "Cam".into(), layers: vec![Layer { content: Content::Camera, ..Default::default() }], ..Default::default() };
        let logo = Layout { name: "Logo".into(), layers: vec![Layer::default(), Layer { content: Content::Color([1, 2, 3]), ..Default::default() }], ..Default::default() };
        let s = Settings { custom_scenes: vec![cam, logo], custom_scene: Some(0), ..Default::default() };
        assert!(matches!(s.scene_spec(), SceneSpec::Custom(ref l) if l.name == "Cam"));
        assert_eq!(s.scene_id(), (Scene::Screen, Some(0)));
        // A camera custom scene turns the webcam on.
        let args = s.record_args(None);
        assert!(args.iter().any(|a| a == "--webcam"));
        // Same sources as a camera template: switches live.
        let t = Settings { scene: Scene::SideBySide, custom_scene: None, ..s.clone() };
        assert_eq!(s.capture_key(), t.capture_key());
        // A composed scene without the camera needs another session than
        // the plain screen.
        let logo = Settings { custom_scene: Some(1), ..s.clone() };
        let plain = Settings { custom_scene: None, scene: Scene::Screen, ..s.clone() };
        assert_ne!(logo.capture_key(), plain.capture_key());
        // A missing index falls back to the template.
        let gone = Settings { custom_scene: Some(9), ..s };
        assert_eq!(gone.scene_spec(), SceneSpec::Template(gone.scene));
    }

    #[test]
    fn hotkeys_default_and_move() {
        let mut s = Settings::default();
        assert_eq!(s.hotkey(Action::Record).map(|c| c.label()).as_deref(), Some("⇧⌘R"));
        assert_eq!(s.hotkey(Action::Pause), None);
        // Giving ⇧⌘R to pause takes it from recording.
        let r = s.hotkey(Action::Record);
        s.set_hotkey(Action::Pause, r);
        assert_eq!(s.hotkey(Action::Pause), r);
        assert_eq!(s.hotkey(Action::Record), None);
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.hotkeys, s.hotkeys);
        assert_eq!(s.capture_key(), Settings::default().capture_key());
    }

    #[test]
    fn several_destinations() {
        let mut s = Settings { stream_dest: "youtube".into(), ..Default::default() };
        assert_eq!(s.stream_urls().len(), 1);
        assert!(s.live_config().hevc, "YouTube alone takes HEVC");
        s.stream_extra.push(StreamDest::default());
        s.stream_extra.push(StreamDest { dest: "custom".into(), custom_url: " ".into(), enabled: true });
        s.stream_extra.push(StreamDest { dest: "youtube".into(), ..Default::default() });
        let urls = s.stream_urls();
        assert_eq!(urls.len(), 2, "blank and repeated destinations are skipped: {urls:?}");
        let c = s.live_config();
        assert!(!c.hevc && c.height <= 1080 && c.bitrate <= 6_000_000, "Twitch limits the shared encode: {c:?}");
        s.stream_extra[0].enabled = false;
        assert_eq!(s.stream_urls().len(), 1);
    }

    #[test]
    fn prores_forces_mov() {
        let s = Settings { container: "mp4".into(), codec: Some(Codec::Prores), ..Default::default() };
        assert!(s.new_output_file().extension().unwrap() == "mov");
        let s = Settings { container: "mp4".into(), ..Default::default() };
        assert!(s.new_output_file().extension().unwrap() == "mp4");
    }
}
