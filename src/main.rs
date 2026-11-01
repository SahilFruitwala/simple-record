mod app;
mod appaudio;
mod browser;
mod camera;
mod compose;
mod effects;
mod filters;
mod gif;
mod hotkey;
mod layout;
mod live;
mod logging;
mod media;
mod mixer;
mod outlatency;
mod play;
mod probe;
mod record;
mod scale;
mod screen;
mod settings;
mod shapes;
mod shortcut;
mod synth;
mod tcc;
mod util;
mod writer;
mod zoom;

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Args, Parser, Subcommand};

use compose::{Corner, PipSpec, Shape};
use writer::Codec;

#[derive(Parser)]
#[command(name = "simple-record", version, about = "Native macOS screen recorder (ScreenCaptureKit + VideoToolbox + AVAssetWriter)")]
struct Cli {
    /// Without a subcommand, rec starts the menu bar app.
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

// RecordArgs is large but parsed once; boxing it buys nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
enum Cmd {
    /// Record the screen (plus system audio, mic, and optional webcam overlay).
    Record(RecordArgs),
    /// The app: window with live preview and sources, settings, menu bar
    /// item and a global Cmd+Shift+R hotkey (what `open dist/SimpleRecord.app` starts).
    Gui {
        /// Start recording immediately (e.g. from a Login Item or script).
        #[arg(long)]
        start_recording: bool,
        /// Open the settings window at launch.
        #[arg(long)]
        settings: bool,
        /// Capture this window (ID from `simple-record devices`) instead of the display.
        #[arg(long, value_name = "ID")]
        window: Option<u32>,
    },
    /// List displays, windows, cameras and microphones.
    Devices,
    /// Stop a running recording (CLI or app) and print its file.
    Stop,
    /// Save the replay buffer of the running recorder (CLI or app) to MP4.
    SaveReplay,
    /// Store a stream key in the macOS Keychain for a destination (reads the
    /// key from stdin; never echoed or logged).
    StreamKey {
        /// Stream URL (rtmp://, rtmps://, https://.../whip) or a service:
        /// youtube, twitch.
        dest: String,
        /// Remove the stored key instead.
        #[arg(long)]
        delete: bool,
    },
    /// Pause the running recording (resume with `simple-record resume`).
    Pause,
    /// Resume a paused recording.
    Resume,
    /// Show (and optionally request) screen, microphone and camera permissions.
    Permissions {
        /// Trigger the system permission prompts for anything not yet granted.
        #[arg(long)]
        request: bool,
        /// Seconds to wait for prompt answers when requesting.
        #[arg(long, default_value_t = 60)]
        wait: u64,
    },
    /// Ask macOS for Screen Recording from this process (used by `permissions --request`).
    #[command(hide = true)]
    RequestScreen,
    /// Check a recording with AVFoundation (the engine QuickTime uses).
    Probe { file: PathBuf },
    /// Turn a recording (or part of it) into a looping GIF.
    Gif {
        file: PathBuf,
        /// Output file. Default: the recording's name with .gif.
        #[arg(short, long, visible_alias = "out")]
        output: Option<PathBuf>,
        /// Frames per second (1-50).
        #[arg(short, long, default_value_t = 15)]
        fps: u32,
        /// Largest width in pixels (never upscaled).
        #[arg(short, long, default_value_t = 720)]
        width: u32,
        /// Start, in seconds into the recording.
        #[arg(long, default_value_t = 0.0)]
        start: f64,
        /// Seconds to export (default: to the end).
        #[arg(short = 't', long)]
        duration: Option<f64>,
    },
    /// Write a smart zoom version of a recording made with --smart-zoom:
    /// zoomed in around clicks, as <name>-zoomed.mov. The zooms are saved to
    /// <name>.zoom.json, which later exports reuse (edit it to adjust them).
    Zoom {
        file: PathBuf,
        /// Output file. Default: <name>-zoomed.mov.
        #[arg(short, long, visible_alias = "out")]
        output: Option<PathBuf>,
        /// Magnification while zoomed in (1.25-3).
        #[arg(short, long, default_value_t = 2.0)]
        level: f64,
        /// Find the zooms from the clicks again, replacing the saved ones.
        #[arg(long)]
        fresh: bool,
        /// Move the smart zoom data (.<name>.smartzoom) to the Trash after
        /// exporting.
        #[arg(long)]
        clean: bool,
    },
    /// Play a file in an AVPlayer window (reference player for sync tests).
    #[command(hide = true)]
    Play {
        file: PathBuf,
        #[arg(long, default_value_t = 15.0)]
        secs: f64,
        /// Window content size in points, WIDTHxHEIGHT.
        #[arg(long, default_value = "640x360")]
        size: String,
    },
}

#[derive(Args)]
pub struct RecordArgs {
    /// Output file (.mov or .mp4). Default: ~/Movies/rec-<timestamp>.mov
    #[arg(short, long, visible_alias = "out")]
    output: Option<PathBuf>,
    /// Display number from `simple-record devices` (1 = main display).
    #[arg(short, long, default_value_t = 1)]
    display: usize,
    /// Record only this region of the display: X,Y,WIDTH,HEIGHT in points
    /// from the display's top-left corner.
    #[arg(long, value_name = "X,Y,W,H", conflicts_with = "window")]
    region: Option<String>,
    /// Record a single window: window ID or app/title substring (`rec devices`).
    #[arg(long, value_name = "WINDOW")]
    window: Option<String>,
    /// Record every window of an app (bundle ID or name substring) on the
    /// display, and nothing else on it.
    #[arg(long, value_name = "APP", conflicts_with_all = ["window", "region"])]
    app: Option<String>,
    /// Output size: 720p, 1080p, 1440p, 4k, native, or WIDTHxHEIGHT. The
    /// frame keeps the display's (or window's) aspect ratio and fits inside
    /// this size (1080p of a 16:10 display is 1728x1080), so there are never
    /// black bars; it is never larger than the source.
    #[arg(short, long, default_value = "native")]
    resolution: String,
    /// Accepted for compatibility: the frame always keeps the source's
    /// aspect ratio now.
    #[arg(long, hide = true)]
    keep_aspect: bool,
    /// Frames per second.
    #[arg(short, long, default_value_t = 60)]
    fps: u32,
    /// Video codec. Default: h264 up to 1080p, hevc above.
    #[arg(long, value_enum)]
    codec: Option<Codec>,
    /// Video bitrate in Mb/s. Default scales with resolution and fps.
    #[arg(long)]
    bitrate: Option<f64>,
    /// Quality of the automatic bitrate (ignored with --bitrate).
    #[arg(long, value_enum, default_value_t = writer::Quality::High)]
    quality: writer::Quality,
    /// Record HDR (experimental): 10-bit HEVC (or ProRes) tagged BT.2020
    /// PQ, from ScreenCaptureKit's HDR capture. Recording only: no replay
    /// buffer or streaming.
    #[arg(long)]
    hdr: bool,
    /// Record system audio (the default; see --no-system-audio).
    #[arg(long, overrides_with = "no_system_audio")]
    system_audio: bool,
    /// Do not record system audio.
    #[arg(long)]
    no_system_audio: bool,
    /// Microphone: `default`, a unique ID or a name substring
    /// (default: the system default mic).
    #[arg(long, value_name = "DEVICE")]
    mic: Option<String>,
    /// Display name of the --mic device, used to find it again when its
    /// unique ID changed.
    #[arg(long, hide = true, value_name = "NAME")]
    mic_name: Option<String>,
    /// Do not record the microphone.
    #[arg(long, conflicts_with = "mic")]
    no_mic: bool,
    /// Overlay a webcam (optionally: unique ID or name substring).
    #[arg(short, long, visible_alias = "cam", value_name = "DEVICE", num_args = 0..=1, default_missing_value = "")]
    webcam: Option<String>,
    /// Scene template: screen, screen-cam (picture-in-picture), cam
    /// (webcam full frame), presenter, side-by-side, cam-screen, split,
    /// reaction or vertical. Default: screen-cam with a webcam, else
    /// screen. A camera scene turns the webcam on.
    #[arg(long, value_enum)]
    scene: Option<compose::Scene>,
    /// A custom scene: a JSON layout (the app's Scenes > Custom export).
    #[arg(long, value_name = "FILE", conflicts_with = "scene")]
    scene_file: Option<PathBuf>,
    /// Shape of the output frame: source (the screen's own shape), 16:9,
    /// 9:16 (Shorts/Reels), 1:1 or 4:5. A preset resolution names the
    /// short side (1080p of 9:16 is 1080x1920).
    #[arg(long, value_enum, default_value_t = layout::Canvas::Source)]
    canvas: layout::Canvas,
    /// Background behind scene layers: a hex color, or two (top,bottom)
    /// for a gradient.
    #[arg(long, value_name = "RRGGBB[,RRGGBB]")]
    background: Option<String>,
    /// An image filling the background (cropped to the frame's shape),
    /// over the colors.
    #[arg(long, value_name = "FILE")]
    background_image: Option<PathBuf>,
    /// Blur the background image, in percent of the frame's shorter side
    /// (0-10).
    #[arg(long, value_name = "PCT", default_value_t = 0.0)]
    background_blur: f64,
    /// How live scene switches animate.
    #[arg(long, value_enum, default_value_t = layout::Transition::Move)]
    transition: layout::Transition,
    /// Scene transition length in milliseconds.
    #[arg(long, default_value_t = 400)]
    transition_ms: u32,
    /// Webcam capture resolution (the largest the camera supports up to it).
    #[arg(long, value_enum, default_value_t = camera::CamRes::P720)]
    camera_resolution: camera::CamRes,
    /// Also write the webcam alone to <output>-camera.mov, on the same
    /// timeline (for editing the camera separately).
    #[arg(long)]
    camera_file: bool,
    /// Also write the screen alone (before the camera and scene are added)
    /// to <output>-screen.mov, on the same timeline.
    #[arg(long)]
    screen_file: bool,
    /// Record for smart zoom: log the cursor and clicks to
    /// <output>.events.json (and keep the screen and webcam apart when the
    /// frame is composed), for `rec zoom`.
    #[arg(long)]
    smart_zoom: bool,
    /// Key out the webcam's background color (green screen): a hex color,
    /// green when none is given.
    #[arg(long, value_name = "COLOR", num_args = 0..=1, default_missing_value = "00ff00")]
    chroma_key: Option<String>,
    /// Chroma key: how far from the key color is removed, 0 to 1.
    #[arg(long, default_value_t = 0.4)]
    key_similarity: f64,
    /// Webcam brightness, -1 to 1.
    #[arg(long, default_value_t = 0.0, allow_negative_numbers = true)]
    camera_brightness: f64,
    /// Webcam contrast, 0 to 2 (1: unchanged).
    #[arg(long, default_value_t = 1.0)]
    camera_contrast: f64,
    /// Webcam saturation, 0 (gray) to 2 (1: unchanged).
    #[arg(long, default_value_t = 1.0)]
    camera_saturation: f64,
    /// Webcam overlay width as a fraction of the frame width.
    #[arg(long, default_value_t = 0.22)]
    pip_size: f64,
    /// Snap the webcam overlay to a corner.
    #[arg(long, value_enum, default_value_t = Corner::BottomRight)]
    pip_corner: Corner,
    /// Place the overlay's center anywhere: X,Y as fractions of the frame
    /// from the top-left (e.g. 0.5,0.5). Overrides --pip-corner.
    #[arg(long, value_name = "X,Y")]
    pip_pos: Option<String>,
    #[arg(long, value_enum, default_value_t = Shape::Rounded)]
    pip_shape: Shape,
    /// Corner roundness 0..1 (rounded, square and squircle shapes).
    #[arg(long, default_value_t = 0.2)]
    pip_roundness: f64,
    /// Overlay border width in pixels at 1080p (0 = none).
    #[arg(long, default_value_t = 0.0)]
    pip_border: f64,
    /// Border color as hex RGB.
    #[arg(long, default_value = "ffffff", value_name = "RRGGBB")]
    pip_border_color: String,
    /// Soft drop shadow under the overlay.
    #[arg(long)]
    pip_shadow: bool,
    /// Do not mirror the webcam.
    #[arg(long)]
    no_mirror: bool,
    /// Hide the mouse cursor.
    #[arg(long)]
    no_cursor: bool,
    /// Highlight mouse clicks.
    #[arg(long)]
    clicks: bool,
    /// Keep SimpleRecord's own windows in display captures (they are left
    /// out by default), e.g. to demo the app itself.
    #[arg(long)]
    show_self: bool,
    /// Stop automatically after this many seconds.
    #[arg(short = 't', long)]
    duration: Option<f64>,
    /// Do not print live status.
    #[arg(short, long)]
    quiet: bool,
    /// Audio tracks when recording both system audio and mic.
    #[arg(long, value_enum, default_value_t = writer::AudioLayout::Both)]
    audio_tracks: writer::AudioLayout,
    /// Shift system audio later by this many ms. Default: the output
    /// device's latency (system audio is captured before it is heard).
    #[arg(long, value_name = "MS", allow_hyphen_values = true)]
    system_audio_offset_ms: Option<f64>,
    /// Shift the microphone later by this many ms (e.g. a Bluetooth mic).
    #[arg(long, value_name = "MS", default_value_t = 0.0, allow_hyphen_values = true)]
    mic_offset_ms: f64,
    /// Keep the last SECS seconds (15-300) of encoded video and audio in
    /// memory; `simple-record save-replay` writes them to an MP4.
    #[arg(long, value_name = "SECS")]
    replay: Option<f64>,
    /// Stream to an RTMP(S) URL, an SRT URL
    /// (srt://host:port?streamid=...&passphrase=...&latency=MS) or a WHIP
    /// endpoint (http(s)://.../whip). youtube and twitch are shortcuts. The
    /// key comes from $REC_STREAM_KEY or the Keychain (`simple-record stream-key
    /// URL`); for SRT it is the stream ID if the URL has none.
    #[arg(long, value_name = "URL")]
    stream: Vec<String>,
    /// Do not write a recording file (replay buffer / streaming only).
    #[arg(long)]
    no_record: bool,
    /// Live encode (replay and streams) size.
    #[arg(long, default_value = "1080p")]
    live_resolution: String,
    /// Live video bitrate in kb/s. Default: the platform's recommendation
    /// for the size and frame rate (YouTube 4K60 45 Mb/s H.264 or 35 HEVC,
    /// Twitch at most 6 Mb/s), else YouTube's.
    #[arg(long)]
    stream_bitrate: Option<u32>,
    /// Live video codec: auto (HEVC when every destination is YouTube),
    /// h264 or hevc (enhanced RTMP / WHIP H.265; falls back to H.264 for
    /// servers that refuse it).
    #[arg(long, default_value = "auto", value_parser = ["auto", "h264", "hevc"])]
    stream_codec: String,
    /// Live keyframe interval in seconds.
    #[arg(long, default_value_t = 2.0)]
    keyframe: f64,
    /// Variable bitrate for the live encode (default: constant).
    #[arg(long)]
    vbr: bool,
    /// Live audio bitrate in kb/s (AAC; Opus for WHIP).
    #[arg(long, default_value_t = 192)]
    audio_bitrate: u32,
    /// System audio gain in the mix track (dB).
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    system_gain: f32,
    /// Microphone gain in the mix track (dB).
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    mic_gain: f32,
    /// System audio of these apps only (comma-separated bundle IDs, e.g.
    /// com.spotify.client,com.google.Chrome).
    #[arg(long, value_name = "IDS", conflicts_with = "audio_except")]
    audio_apps: Option<String>,
    /// System audio of every app but these (comma-separated bundle IDs).
    #[arg(long, value_name = "IDS")]
    audio_except: Option<String>,
    /// Microphone: cut rumble below 80 Hz (see --low-cut-hz).
    #[arg(long)]
    low_cut: bool,
    /// Microphone: the low cut's frequency (40-300 Hz).
    #[arg(long, value_name = "HZ", default_value_t = 80.0, requires = "low_cut")]
    low_cut_hz: f32,
    /// Microphone: voice EQ as LOW,PRESENCE,AIR in dB (-12 to 12): shelves
    /// at 150 Hz and 10 kHz, a peak at 3.5 kHz (e.g. 2,3,1).
    #[arg(long, value_name = "DB,DB,DB", allow_negative_numbers = true, value_parser = parse_eq)]
    eq: Option<[f32; 3]>,
    /// Microphone: de-esser, taming "s" sounds louder than this level
    /// (dBFS, e.g. -30).
    #[arg(long, value_name = "DB", allow_negative_numbers = true, num_args = 0..=1, default_missing_value = "-30")]
    deess: Option<f32>,
    /// Microphone: noise suppression (RNNoise; 48 kHz mics).
    #[arg(long)]
    denoise: bool,
    /// Microphone: hold speech at a steady loudness, with a limiter.
    #[arg(long)]
    level: bool,
    /// Microphone: noise gate opening above this level (dBFS, e.g. -45).
    // allow_negative_numbers, not allow_hyphen_values: with an optional
    // value, the latter takes the next flag as the value (`--gate --low-cut`).
    #[arg(long, value_name = "DB", allow_negative_numbers = true, num_args = 0..=1, default_missing_value = "-45")]
    gate: Option<f32>,
    /// Microphone: compressor starting at this level (dBFS, e.g. -20),
    /// 3:1 with 6 dB makeup gain.
    #[arg(long, value_name = "DB", allow_negative_numbers = true, num_args = 0..=1, default_missing_value = "-20")]
    compressor: Option<f32>,
    /// Microphone capture backend.
    #[arg(long, value_enum, default_value_t = record::MicBackend::Sck)]
    mic_backend: record::MicBackend,
    /// Record a generated test pattern instead of the screen (self-test;
    /// needs no Screen Recording permission). The frame flashes white once a
    /// second and the "system audio" beeps at the same instant, for sync
    /// checks. Implies --mic-backend avcapture.
    #[arg(long)]
    test_pattern: bool,
    /// With --test-pattern: add random noise blocks to stress the encoder.
    #[arg(long, requires = "test_pattern")]
    noise: bool,
}

fn even(x: f64) -> usize {
    ((x / 2.0).round() as usize * 2).max(2)
}

/// Output size for `s` (a preset or WxH, the bounding box) and a source of
/// `native` pixels: the source's aspect ratio fitted inside the box, never
/// larger than the source, even dimensions. A frame with the source's shape
/// fills edge to edge: no letterbox or pillarbox bars, nothing cropped.
fn parse_size(s: &str, native: (usize, usize)) -> Result<(usize, usize)> {
    let lower = s.to_lowercase();
    // The capture's own size is the native size rounded down to even.
    let (nw, nh) = ((native.0 & !1).max(2) as f64, (native.1 & !1).max(2) as f64);
    let (bw, bh) = match lower.as_str() {
        "720p" => (1280.0, 720.0),
        "1080p" => (1920.0, 1080.0),
        "1440p" => (2560.0, 1440.0),
        "4k" | "2160p" | "uhd" => (3840.0, 2160.0),
        "native" => (nw, nh),
        _ => {
            let Some((w, h)) = lower.split_once('x') else {
                bail!("bad --resolution {s:?}: use 720p/1080p/1440p/4k/native/WxH")
            };
            (w.trim().parse::<usize>()? as f64, h.trim().parse::<usize>()? as f64)
        }
    };
    // Presets name the frame's height for landscape sources and its width
    // for portrait ones (a 1080p portrait window is 1080 wide).
    let (bw, bh) = if nh > nw && matches!(lower.as_str(), "720p" | "1080p" | "1440p" | "4k" | "2160p" | "uhd") { (bh, bw) } else { (bw, bh) };
    let k = (bw / nw).min(bh / nh).min(1.0);
    Ok((even(nw * k), even(nh * k)))
}

/// Output size for a canvas of a fixed shape (`aspect` = width / height):
/// a preset names the short side (1080p of 9:16 is 1080x1920), WxH is the
/// largest frame of that shape inside it.
fn canvas_size(s: &str, aspect: f64, native: (usize, usize)) -> Result<(usize, usize)> {
    let lower = s.to_lowercase();
    let short = match lower.as_str() {
        "720p" => 720.0,
        "1080p" => 1080.0,
        "1440p" => 1440.0,
        "4k" | "2160p" | "uhd" => 2160.0,
        "native" => native.0.min(native.1).max(2) as f64,
        _ => {
            let Some((w, h)) = lower.split_once('x') else {
                bail!("bad --resolution {s:?}: use 720p/1080p/1440p/4k/native/WxH")
            };
            let (bw, bh) = (w.trim().parse::<usize>()? as f64, h.trim().parse::<usize>()? as f64);
            let (w, h) = if bw / bh > aspect { (bh * aspect, bh) } else { (bw, bw / aspect) };
            return Ok((even(w), even(h)));
        }
    };
    Ok(if aspect >= 1.0 { (even(short * aspect), even(short)) } else { (even(short), even(short / aspect)) })
}

/// The largest frame with the source's shape inside a `w`x`h` canvas,
/// never larger than the source (even sizes).
fn fit_inside(native: (usize, usize), w: usize, h: usize) -> (usize, usize) {
    let (nw, nh) = ((native.0 & !1).max(2) as f64, (native.1 & !1).max(2) as f64);
    let k = (w as f64 / nw).min(h as f64 / nh).min(1.0);
    (even(nw * k).min(w), even(nh * k).min(h))
}

fn parse_background(s: &str) -> Result<layout::Background> {
    let mut parts = s.split(',').map(str::trim);
    let top = parse_hex(parts.next().unwrap_or_default())?;
    let bottom = match parts.next() {
        Some(b) => parse_hex(b)?,
        None => top,
    };
    Ok(layout::Background::gradient(top, bottom))
}

/// The live encode's bounding box for a preset or WxH (the encoder fits the
/// frame inside it, keeping its aspect ratio).
fn live_box(s: &str) -> Result<(usize, usize)> {
    Ok(match s.to_lowercase().as_str() {
        "720p" => (1280, 720),
        "1080p" => (1920, 1080),
        "1440p" => (2560, 1440),
        "4k" | "2160p" | "uhd" => (3840, 2160),
        other => {
            let Some((w, h)) = other.split_once('x') else {
                bail!("bad --live-resolution {s:?}: use 720p/1080p/1440p/4k/WxH")
            };
            (even(w.trim().parse::<f64>()?), even(h.trim().parse::<f64>()?))
        }
    })
}

/// `simple-record stream-key DEST`: store (or delete) a stream key in the Keychain.
fn stream_key(dest: &str, delete: bool) -> Result<()> {
    let url = live::destination_url(dest);
    if delete {
        live::keychain::delete(&url)?;
        println!("removed the stream key for {url}");
        return Ok(());
    }
    let tty = unsafe { libc::isatty(0) } == 1;
    let mut old: libc::termios = unsafe { std::mem::zeroed() };
    if tty {
        eprint!("stream key for {url}: ");
        unsafe {
            libc::tcgetattr(0, &mut old);
            let mut t = old;
            t.c_lflag &= !libc::ECHO;
            libc::tcsetattr(0, libc::TCSANOW, &t);
        }
    }
    let mut key = String::new();
    let r = std::io::stdin().read_line(&mut key);
    if tty {
        unsafe { libc::tcsetattr(0, libc::TCSANOW, &old) };
        eprintln!();
    }
    r?;
    let key = key.trim();
    if key.is_empty() {
        bail!("no key given");
    }
    live::keychain::set(&url, key)?;
    println!("saved the stream key for {url} in the Keychain");
    Ok(())
}

fn parse_pos(s: &str) -> Result<[f64; 2]> {
    let v: Vec<f64> = s.split(',').map(|x| x.trim().parse::<f64>()).collect::<Result<_, _>>()?;
    match v[..] {
        [x, y] if (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y) => Ok([x, y]),
        _ => bail!("--pip-pos must be X,Y with both between 0 and 1"),
    }
}

fn parse_hex(s: &str) -> Result<[u8; 3]> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        bail!("color must be RRGGBB hex");
    }
    let c = |i: usize| u8::from_str_radix(&s[i..i + 2], 16);
    Ok([c(0)?, c(2)?, c(4)?])
}

fn parse_eq(s: &str) -> Result<[f32; 3], String> {
    let v: Vec<f32> = s.split(',').map(|x| x.trim().parse::<f32>().map_err(|e| e.to_string())).collect::<Result<_, _>>()?;
    <[f32; 3]>::try_from(v).map_err(|_| "needs three values: LOW,PRESENCE,AIR".to_string())
}

fn default_output() -> PathBuf {
    let mut buf = [0u8; 64];
    let n = unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        libc::strftime(buf.as_mut_ptr() as *mut _, buf.len(), c"rec-%Y-%m-%d-%H%M%S.mov".as_ptr(), &tm)
    };
    let name = String::from_utf8_lossy(&buf[..n]).to_string();
    let dir = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Movies")).unwrap_or_default();
    dir.join(name)
}

/// Turn record arguments into session options and a capture target
/// (shared by the CLI and the GUI).
pub fn prepare(a: RecordArgs) -> Result<(record::Options, Option<screen::Target>)> {
    prepare_with(a, false)
}

/// `prepare` for the app, which records without a mic or camera it isn't
/// allowed to use.
pub fn prepare_lenient(a: RecordArgs) -> Result<(record::Options, Option<screen::Target>)> {
    prepare_with(a, true)
}

fn prepare_with(a: RecordArgs, lenient: bool) -> Result<(record::Options, Option<screen::Target>)> {
    // A missing mic falls back to the default; audio never blocks a session.
    let mut mic_warning = None;
    let mic = if a.no_mic {
        None
    } else {
        Some(a.mic.as_deref().filter(|m| *m != "default").and_then(|sel| {
            let p = camera::pick_mic(sel, a.mic_name.as_deref());
            if let Some(w) = p.warning {
                eprintln!("rec: warning: {w}");
                mic_warning = Some(w);
            }
            p.id
        }))
    };
    let mut webcam = a.webcam.map(|s| if s.is_empty() || s == "default" { None } else { Some(s) });
    let scene = match &a.scene_file {
        Some(f) => {
            let text = std::fs::read_to_string(f).map_err(|e| anyhow::anyhow!("--scene-file {}: {e}", f.display()))?;
            layout::SceneSpec::Custom(serde_json::from_str(&text).map_err(|e| anyhow::anyhow!("--scene-file {}: {e}", f.display()))?)
        }
        None => layout::SceneSpec::Template(a.scene.unwrap_or(if webcam.is_some() { compose::Scene::ScreenCam } else { compose::Scene::Screen })),
    };
    if scene.uses_cam() && webcam.is_none() {
        webcam = Some(None);
    }
    let mut opts = record::Options {
        output: a.output.unwrap_or_else(default_output),
        size: (0, 0),
        screen_size: (0, 0),
        native: (0, 0),
        fps: a.fps,
        codec: Codec::H264,
        bitrate: 0,
        hdr: a.hdr,
        system_audio: !a.no_system_audio,
        mic,
        mic_warning,
        webcam,
        cam_res: a.camera_resolution,
        camera_file: a.camera_file,
        smart_zoom: a.smart_zoom,
        screen_file: a.screen_file,
        pip: PipSpec {
            size: a.pip_size.clamp(0.05, 0.9),
            place: match &a.pip_pos {
                Some(p) => compose::Place::At(parse_pos(p)?),
                None => compose::Place::Corner(a.pip_corner),
            },
            shape: a.pip_shape,
            roundness: a.pip_roundness.clamp(0.0, 1.0),
            border: a.pip_border.clamp(0.0, 64.0),
            border_color: parse_hex(&a.pip_border_color)?,
            shadow: a.pip_shadow,
            mirror: !a.no_mirror,
            effects: effects::Effects {
                brightness: a.camera_brightness.clamp(-1.0, 1.0),
                contrast: a.camera_contrast.clamp(0.0, 2.0),
                saturation: a.camera_saturation.clamp(0.0, 2.0),
                chroma_key: match &a.chroma_key {
                    Some(c) => Some(effects::ChromaKey { color: parse_hex(c)?, similarity: a.key_similarity.clamp(0.0, 1.0), ..Default::default() }),
                    None => None,
                },
            },
        },
        cursor: !a.no_cursor,
        clicks: a.clicks,
        show_self: a.show_self,
        duration: a.duration,
        quiet: a.quiet,
        test_pattern: a.test_pattern.then_some(a.noise),
        audio_layout: a.audio_tracks,
        gains_db: [a.system_gain, a.mic_gain],
        mic_backend: if a.test_pattern { record::MicBackend::Avcapture } else { a.mic_backend },
        scene,
        background: {
            let mut bg = match &a.background {
                Some(b) => parse_background(b)?,
                None => layout::Background::default(),
            };
            if let Some(f) = &a.background_image {
                if !f.is_file() {
                    bail!("--background-image: {} not found", f.display());
                }
                bg.image = Some(f.to_string_lossy().into_owned());
            }
            bg.blur = (a.background_blur / 100.0).clamp(0.0, layout::Background::MAX_BLUR);
            bg
        },
        transition: a.transition,
        transition_secs: a.transition_ms.min(3000) as f64 / 1000.0,
        app_audio: appaudio::AppAudio::from_args(a.audio_apps.as_deref(), a.audio_except.as_deref()),
        mic_filters: filters::MicFilters {
            low_cut: a.low_cut,
            low_cut_hz: a.low_cut_hz.clamp(40.0, 300.0),
            denoise: a.denoise,
            level: a.level,
            gate: a.gate.is_some(),
            gate_db: a.gate.unwrap_or(-45.0).clamp(-80.0, 0.0),
            compressor: a.compressor.is_some(),
            comp_threshold_db: a.compressor.unwrap_or(-20.0).clamp(-60.0, 0.0),
            eq: a.eq.is_some(),
            eq_low_db: a.eq.as_ref().map_or(0.0, |e| e[0].clamp(-12.0, 12.0)),
            eq_mid_db: a.eq.as_ref().map_or(0.0, |e| e[1].clamp(-12.0, 12.0)),
            eq_high_db: a.eq.as_ref().map_or(0.0, |e| e[2].clamp(-12.0, 12.0)),
            deess: a.deess.is_some(),
            deess_db: a.deess.unwrap_or(-30.0).clamp(-60.0, 0.0),
            ..Default::default()
        },
        system_offset_ms: a.system_audio_offset_ms,
        mic_offset_ms: a.mic_offset_ms,
        write: !a.no_record,
        live: None,
        streams: vec![],
    };
    if lenient {
        record::drop_unallowed(&mut opts);
    }
    record::check_permissions(&opts)?;
    if !(1..=240).contains(&a.fps) || 60000 % a.fps != 0 {
        bail!("--fps must divide 60000 (e.g. 24, 25, 30, 50, 60, 120)");
    }
    let (target, native) = if a.test_pattern {
        (None, (3840, 2160))
    } else {
        let mut displays = screen::displays()?;
        if a.display == 0 || a.display > displays.len() {
            bail!("display {} not found: displays are numbered 1..={} (see `simple-record devices`)", a.display, displays.len());
        }
        let d = displays.swap_remove(a.display - 1);
        let scale = d.pixels.0 as f64 / d.points.0.max(1) as f64;
        if let Some(sel) = &a.window {
            let w = screen::find_window(sel)?;
            let native = ((w.size.0 * scale) as usize, (w.size.1 * scale) as usize);
            (Some(screen::Target::Window(w)), native)
        } else if let Some(sel) = &a.app {
            let app = screen::find_app(sel)?;
            let native = d.pixels;
            (Some(screen::Target::App(d, app)), native)
        } else if let Some(r) = &a.region {
            let v: Vec<f64> = r.split(',').map(|x| x.trim().parse::<f64>()).collect::<Result<_, _>>()?;
            if v.len() != 4 || v[2] <= 0.0 || v[3] <= 0.0 {
                bail!("--region must be X,Y,WIDTH,HEIGHT (points)");
            }
            let rect = objc2_core_foundation::CGRect::new(
                objc2_core_foundation::CGPoint::new(v[0], v[1]),
                objc2_core_foundation::CGSize::new(v[2], v[3]),
            );
            let native = ((v[2] * scale) as usize, (v[3] * scale) as usize);
            (Some(screen::Target::Display(d, Some(rect))), native)
        } else {
            let native = d.pixels;
            (Some(screen::Target::Display(d, None)), native)
        }
    };
    let (w, h) = match a.canvas.aspect() {
        Some(aspect) => canvas_size(&a.resolution, aspect, native)?,
        None => parse_size(&a.resolution, native)?,
    };
    let codec = a.codec.unwrap_or(if w * h > 1920 * 1080 || a.hdr { Codec::Hevc } else { Codec::H264 });
    if a.hdr {
        if codec == Codec::H264 {
            bail!("HDR needs HEVC or ProRes (--codec hevc or prores)");
        }
        if a.test_pattern {
            bail!("HDR can't be used with the test pattern");
        }
        if a.replay.is_some() || !a.stream.is_empty() || a.no_record {
            bail!("HDR is for recordings: the replay buffer and streams are SDR (drop --hdr, or --replay/--stream)");
        }
    }
    // Bits per pixel per frame tuned for screen content (sharp text).
    let bpp = match codec {
        Codec::H264 => 0.16,
        Codec::Hevc => 0.10,
        Codec::Prores => 0.0, // fixed quality; bitrate unused
    };
    let bitrate = a
        .bitrate
        .map(|m| (m * 1e6) as u64)
        .unwrap_or_else(|| ((w * h) as f64 * a.fps as f64 * bpp * a.quality.factor()) as u64)
        .clamp(1_000_000, 400_000_000);
    if codec == Codec::Prores && opts.output.extension().is_some_and(|e| e == "mp4") {
        bail!("ProRes needs a .mov output");
    }
    opts.size = (w, h);
    opts.screen_size = if a.canvas.aspect().is_some() { fit_inside(native, w, h) } else { (w, h) };
    opts.native = (native.0 & !1, native.1 & !1);
    if a.no_record && a.replay.is_none() && a.stream.is_empty() {
        bail!("--no-record needs --replay or --stream");
    }
    if a.replay.is_some() || !a.stream.is_empty() {
        let live = live::Live::new();
        let (lw, lh) = live_box(&a.live_resolution)?;
        let urls: Vec<String> = a.stream.iter().map(|d| live::destination_url(d)).collect();
        let platforms: Vec<live::Platform> = urls.iter().map(|u| live::platform(u)).collect();
        let want_hevc = match a.stream_codec.as_str() {
            "hevc" => true,
            "h264" => false,
            _ => !platforms.is_empty() && platforms.iter().all(|p| *p == live::Platform::YouTube),
        };
        // Every destination's limits: the smallest bitrate and size, HEVC
        // only if all of them take it. Sizes are of the frame's short side
        // (a 9:16 frame streams as 1080x1920 at "1080p").
        let short = w.min(h);
        let presets: Vec<(u32, usize, bool)> = if platforms.is_empty() { vec![live::preset(live::Platform::Other, lh.min(short), a.fps.min(60), want_hevc)] } else {
            platforms.iter().map(|p| live::preset(*p, lh.min(short), a.fps.min(60), want_hevc)).collect()
        };
        let kbps = a.stream_bitrate.unwrap_or_else(|| presets.iter().map(|p| p.0).min().unwrap_or(6000));
        let max_h = presets.iter().map(|p| p.1).min().unwrap_or(lh);
        let hevc = presets.iter().all(|p| p.2);
        let box_h = lh.min(short).min(max_h);
        let (lw, lh) = (even(lw as f64 * box_h as f64 / lh as f64), box_h);
        live.set_config(live::LiveConfig {
            width: lw,
            height: lh,
            bitrate: kbps.clamp(500, 80_000) as u64 * 1000,
            keyframe_secs: a.keyframe.clamp(0.5, 4.0),
            cbr: !a.vbr,
            audio_bitrate: a.audio_bitrate.clamp(64, 320) * 1000,
            hevc,
        });
        if !a.quiet {
            eprintln!(
                "rec: live encode {lw}x{lh} {} {:.1} Mb/s, keyframe every {} s",
                if hevc { "HEVC" } else { "H.264" },
                kbps as f64 / 1000.0,
                a.keyframe
            );
        }
        if let Some(secs) = a.replay {
            if !(15.0..=300.0).contains(&secs) {
                bail!("--replay must be 15-300 seconds");
            }
            live.set_replay(Some(secs));
        }
        for dest in &a.stream {
            opts.streams.push(live::resolve_destination(dest)?);
        }
        opts.live = Some(live);
    }
    opts.codec = codec;
    opts.bitrate = bitrate;
    if let Some(parent) = opts.output.parent()
        && !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    Ok((opts, target))
}

/// Parse a `rec record ...` argument list (used by the GUI).
pub fn parse_record_args(args: &[&str]) -> Result<RecordArgs> {
    let mut v = vec!["simple-record", "record"];
    v.extend_from_slice(args);
    match Cli::try_parse_from(v)?.cmd {
        Some(Cmd::Record(a)) => Ok(a),
        _ => bail!("internal: not a record command"),
    }
}

fn record(a: RecordArgs) -> Result<()> {
    let (opts, target) = prepare(a)?;
    let out = opts.output.clone();
    let quiet = opts.quiet;
    let wrote = opts.write;
    let stats = record::run(opts, target)?;
    if !wrote {
        if !quiet {
            eprintln!("rec: stopped ({} video frames, {} dropped)", stats.video_frames_out, stats.video_drops);
        }
        return Ok(());
    }
    if !quiet {
        eprintln!(
            "rec: saved {} ({} video frames, {} duplicated, {} dropped; audio buffers sys {} mic {} mix {}, {} dropped, {} clipped samples)",
            out.display(),
            stats.video_frames_out,
            stats.video_dups,
            stats.video_drops,
            stats.audio_sys_buffers,
            stats.audio_mic_buffers,
            stats.audio_mix_buffers,
            stats.audio_drops,
            stats.audio_clipped_samples
        );
    }
    if std::env::var_os("REC_DEBUG").is_some() {
        let avg = |sum: f64, n: u64| if n == 0 { 0.0 } else { sum / n as f64 * 1000.0 };
        eprintln!(
            "rec: debug: encoder busy retries {}, audio gaps filled {}",
            stats.video_retries,
            stats.audio_gaps_filled
        );
        eprintln!(
            "rec: debug: frame arrival latency  screen avg {:.1} ms max {:.1} ms ({} frames)  camera avg {:.1} ms max {:.1} ms ({} frames)",
            avg(stats.screen_latency_sum, stats.screen_frames_in),
            stats.screen_latency_max * 1000.0,
            stats.screen_frames_in,
            avg(stats.cam_latency_sum, stats.cam_frames_in),
            stats.cam_latency_max * 1000.0,
            stats.cam_frames_in
        );
    }
    if let Some(c) = &stats.camera_file {
        if !quiet {
            eprintln!("rec: camera saved {} ({} frames)", c.display(), stats.camera_frames);
        }
        println!("{}", c.display());
    }
    // A screen file kept only for smart zoom stays in its hidden folder.
    if let Some(s) = &stats.screen_file
        && stats.zoom_data.as_ref().is_none_or(|d| !s.starts_with(d))
    {
        if !quiet {
            eprintln!("rec: screen saved {}", s.display());
        }
        println!("{}", s.display());
    }
    if stats.zoom_data.is_some() && !quiet {
        eprintln!("rec: recorded for smart zoom (`rec zoom {}` writes the zoomed version)", out.display());
    }
    println!("{}", out.display());
    Ok(())
}

fn devices() -> Result<()> {
    println!("Displays:");
    match screen::displays() {
        Ok(ds) => {
            for (i, d) in ds.iter().enumerate() {
                println!(
                    "  [{}] id {}  {}x{} pixels ({}x{} points)",
                    i + 1,
                    d.id,
                    d.pixels.0, d.pixels.1, d.points.0, d.points.1
                );
            }
        }
        Err(e) => println!("  unavailable: {e}"),
    }
    println!("Apps (for --app):");
    match screen::applications() {
        Ok(apps) => {
            for a in apps {
                println!("  {}  [{}]", a.name, a.bundle_id);
            }
        }
        Err(e) => println!("  unavailable: {e}"),
    }
    println!("Windows (for --window):");
    match screen::windows() {
        Ok(ws) => {
            for w in ws {
                println!("  {:>6}  {} — {}  ({}x{} pt)", w.id, w.app, w.title, w.size.0, w.size.1);
            }
        }
        Err(e) => println!("  unavailable: {e}"),
    }
    println!("Cameras:");
    for c in camera::cameras() {
        println!("  {}  [{}]", c.name, c.id);
    }
    println!("Microphones:");
    for m in camera::microphones() {
        println!("  {}  [{}]", m.name, m.id);
    }
    Ok(())
}

fn main() {
    // Behave like a normal CLI when piped into `head` etc.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    tcc::disclaim_and_reexec();
    let cli = Cli::parse();
    let r = match cli.cmd.unwrap_or(Cmd::Gui { start_recording: false, settings: false, window: None }) {
        Cmd::Gui { start_recording, settings, window } => app::run(start_recording, settings, window),
        Cmd::Record(a) => record(a),
        Cmd::Devices => {
            let r = devices();
            if let Some(l) = outlatency::output_latency() {
                println!("Output latency (system audio is shifted by this): {:.1} ms", l * 1000.0);
            }
            r
        }
        Cmd::Stop => record::stop_running(),
        Cmd::Pause => record::signal_running(libc::SIGUSR1),
        Cmd::Resume => record::signal_running(libc::SIGUSR2),
        Cmd::SaveReplay => record::signal_running(libc::SIGINFO),
        Cmd::StreamKey { dest, delete } => stream_key(&dest, delete),
        Cmd::Permissions { request, wait } => {
            if request {
                tcc::request_all(true, true, true, std::time::Duration::from_secs(wait));
            }
            let (s, m, c) = tcc::report();
            println!("screen recording : {s}");
            println!("microphone       : {m}");
            println!("camera           : {c}");
            Ok(())
        }
        Cmd::RequestScreen => {
            tcc::request_screen_here();
            Ok(())
        }
        Cmd::Probe { file } => probe::probe(&file),
        Cmd::Gif { file, output, fps, width, start, duration } => {
            let out = output.unwrap_or_else(|| gif::gif_path(&file));
            let o = gif::GifOptions { fps, width, start, duration };
            gif::export(&file, &out, &o, &|p| eprint!("\rrec: GIF {:>3.0}%", p * 100.0)).map(|s| {
                eprintln!();
                println!("{} ({}x{}, {} frames, {:.1} MB)", out.display(), s.size.0, s.size.1, s.frames, s.bytes as f64 / 1e6);
            })
        }
        Cmd::Zoom { file, output, level, fresh, clean } => {
            let out = output.unwrap_or_else(|| zoom::zoomed_path(&file));
            let o = zoom::ZoomOptions { level, fresh };
            zoom::export(&file, &out, &o, &|p| eprint!("\rrec: zoom {:>3.0}%", p * 100.0)).map(|s| {
                eprintln!();
                println!("{} ({} frames, {} zooms)", out.display(), s.frames, s.zooms);
                if clean && let Err(e) = app::macos::trash(&zoom::data_dir(&file)) {
                    eprintln!("rec: warning: couldn't move the smart zoom data to the Trash: {e}");
                }
            })
        }
        Cmd::Play { file, secs, size } => play::play(&file, secs, &size),
    };
    if let Err(e) = r {
        eprintln!("rec: error: {e:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the app runs when the camera isn't allowed: the scene shows the
    /// camera, which isn't there. The layer stays empty; the rest records.
    #[test]
    fn camera_scene_records_without_the_camera() {
        let dir = std::env::temp_dir().join(format!("rec-nocam-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("nocam.mov");
        let a = parse_record_args(&["--test-pattern", "--no-mic", "--no-system-audio", "-o", file.to_str().unwrap()]).unwrap();
        let (mut o, t) = prepare(a).unwrap();
        o.scene = layout::SceneSpec::Template(compose::Scene::ScreenCam);
        assert!(o.webcam.is_none());
        let s = record::Session::start_with_camera(&o, t, true, None, None).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(1));
        s.finish().unwrap();
        assert!(std::fs::metadata(&file).unwrap().len() > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn goal_command_line_parses() {
        let a = parse_record_args(&[
            "--display", "1", "--mic", "default", "--system-audio", "--cam", "--out", "file.mp4", "--duration", "10",
        ])
        .unwrap();
        assert_eq!(a.display, 1);
        assert_eq!(a.mic.as_deref(), Some("default"));
        assert!(a.system_audio && !a.no_system_audio);
        assert_eq!(a.webcam.as_deref(), Some(""));
        assert_eq!(a.output, Some(PathBuf::from("file.mp4")));
        assert_eq!(a.duration, Some(10.0));
    }

    #[test]
    fn system_audio_flags_override_each_other() {
        let a = parse_record_args(&["--system-audio", "--no-system-audio"]).unwrap();
        assert!(a.no_system_audio);
        let a = parse_record_args(&["--no-system-audio", "--system-audio"]).unwrap();
        assert!(!a.no_system_audio);
    }

    #[test]
    fn sizes() {
        // Every size keeps the source's aspect ratio (no bars), even sizes.
        let native = (3456, 2234);
        assert_eq!(parse_size("1080p", native).unwrap(), (1670, 1080));
        assert_eq!(parse_size("4K", native).unwrap(), (3342, 2160));
        assert_eq!(parse_size("native", native).unwrap(), native);
        assert_eq!(parse_size("1280x720", native).unwrap(), (1114, 720));
        assert_eq!(parse_size("1080p", (2880, 1800)).unwrap(), (1728, 1080));
        assert_eq!(parse_size("1080p", (3840, 2160)).unwrap(), (1920, 1080));
        // Never upscaled; odd sources round down to even, like the capture.
        assert_eq!(parse_size("4k", (1201, 801)).unwrap(), (1200, 800));
        // Portrait: the preset names the width.
        assert_eq!(parse_size("1080p", (2160, 3840)).unwrap(), (1080, 1920));
        for (sz, n) in [("1080p", (3456, 2234)), ("720p", (4112, 2658)), ("4k", (5120, 2880)), ("1440p", (3144, 1588))] {
            let (w, h) = parse_size(sz, n).unwrap();
            assert!(w % 2 == 0 && h % 2 == 0);
            assert!(compose::same_aspect(n.0 as f64, n.1 as f64, w as f64, h as f64), "{sz} {n:?} -> {w}x{h}");
        }
        assert!(parse_size("huge", native).is_err());
    }

    #[test]
    fn overlay_options() {
        let a = parse_record_args(&["--cam", "--pip-pos", "0.25,0.75", "--pip-shape", "squircle", "--pip-roundness", "0.6",
            "--pip-border", "8", "--pip-border-color", "#ff8800", "--pip-shadow"]).unwrap();
        assert_eq!(parse_pos(a.pip_pos.as_deref().unwrap()).unwrap(), [0.25, 0.75]);
        assert_eq!(a.pip_shape, Shape::Squircle);
        assert_eq!(parse_hex(&a.pip_border_color).unwrap(), [255, 136, 0]);
        assert!(a.pip_shadow);
        assert!(parse_pos("1.5,0").is_err());
        assert!(parse_hex("fff").is_err());
        for s in ["circle", "rounded", "square", "pill", "squircle", "rect"] {
            assert!(parse_record_args(&["--pip-shape", s]).is_ok(), "{s}");
        }
    }

    #[test]
    fn scene_option() {
        let a = parse_record_args(&["--scene", "cam"]).unwrap();
        assert_eq!(a.scene, Some(compose::Scene::Cam));
        assert!(parse_record_args(&["--scene", "nope"]).is_err());
        for s in ["presenter", "side-by-side", "cam-screen", "split", "reaction", "vertical"] {
            assert!(parse_record_args(&["--scene", s]).is_ok(), "{s}");
        }
        let a = parse_record_args(&["--canvas", "9:16", "--transition", "fade", "--background", "101010,303030"]).unwrap();
        assert_eq!(a.canvas, layout::Canvas::Portrait);
        assert_eq!(a.transition, layout::Transition::Fade);
        assert_eq!(parse_background(a.background.as_deref().unwrap()).unwrap(), layout::Background::gradient([16; 3], [48; 3]));
        assert!(parse_record_args(&["--scene", "cam", "--scene-file", "x.json"]).is_err());
    }

    #[test]
    fn quality_option() {
        assert_eq!(parse_record_args(&[]).unwrap().quality, writer::Quality::High);
        assert_eq!(parse_record_args(&["--quality", "max"]).unwrap().quality, writer::Quality::Max);
        assert!(parse_record_args(&["--quality", "ultra"]).is_err());
    }

    #[test]
    fn hdr_option() {
        assert!(parse_record_args(&["--hdr", "--codec", "hevc"]).unwrap().hdr);
        assert!(!parse_record_args(&[]).unwrap().hdr);
    }

    #[test]
    fn mic_filter_options() {
        let a = parse_record_args(&["--background-image", "/tmp/w.png", "--background-blur", "4"]).unwrap();
        assert_eq!((a.background_image.as_deref(), a.background_blur), (Some(std::path::Path::new("/tmp/w.png")), 4.0));
        let a = parse_record_args(&["--denoise", "--gate", "--compressor", "-24", "--low-cut"]).unwrap();
        assert!(a.denoise && a.low_cut);
        assert_eq!(a.gate, Some(-45.0));
        assert_eq!(a.compressor, Some(-24.0));
        let a = parse_record_args(&[]).unwrap();
        assert!(a.gate.is_none() && a.compressor.is_none());
        // A value-less filter flag doesn't swallow the next flag.
        let a = parse_record_args(&["--gate", "--low-cut", "--compressor"]).unwrap();
        assert_eq!((a.gate, a.compressor, a.low_cut), (Some(-45.0), Some(-20.0), true));
        let a = parse_record_args(&["--low-cut", "--low-cut-hz", "120", "--eq", "2,-3,1.5", "--deess"]).unwrap();
        assert_eq!((a.low_cut_hz, a.eq.as_ref().map(|e| &e[..]), a.deess), (120.0, Some(&[2.0, -3.0, 1.5][..]), Some(-30.0)));
        assert!(parse_record_args(&["--eq", "2,3"]).is_err());
        assert!(parse_record_args(&["--low-cut-hz", "120"]).is_err());
        assert_eq!(parse_record_args(&["--gate", "-50"]).unwrap().gate, Some(-50.0));
    }

    #[test]
    fn app_audio_options() {
        let a = parse_record_args(&["--audio-apps", "com.spotify.client,com.google.Chrome"]).unwrap();
        assert_eq!(appaudio::AppAudio::from_args(a.audio_apps.as_deref(), a.audio_except.as_deref()).apps().len(), 2);
        assert!(parse_record_args(&["--audio-apps", "a", "--audio-except", "b"]).is_err());
    }

    #[test]
    fn camera_effect_options() {
        let a = parse_record_args(&["--chroma-key", "--camera-brightness", "-0.1"]).unwrap();
        assert_eq!(a.chroma_key.as_deref(), Some("00ff00"));
        assert_eq!(a.camera_brightness, -0.1);
        let a = parse_record_args(&["--chroma-key", "0000ff", "--key-similarity", "0.3"]).unwrap();
        assert_eq!(parse_hex(a.chroma_key.as_deref().unwrap()).unwrap(), [0, 0, 255]);
        assert!(parse_record_args(&[]).unwrap().chroma_key.is_none());
    }

    #[test]
    fn canvas_sizes() {
        let native = (3456, 2234);
        assert_eq!(canvas_size("1080p", 9.0 / 16.0, native).unwrap(), (1080, 1920));
        assert_eq!(canvas_size("1080p", 16.0 / 9.0, native).unwrap(), (1920, 1080));
        assert_eq!(canvas_size("4k", 1.0, native).unwrap(), (2160, 2160));
        assert_eq!(canvas_size("720p", 0.8, native).unwrap(), (720, 900));
        assert_eq!(canvas_size("1000x1000", 16.0 / 9.0, native).unwrap(), (1000, 562));
        // The screen inside a portrait canvas: full width, its own shape.
        assert_eq!(fit_inside(native, 1080, 1920), (1080, 698));
        // Never larger than the source.
        assert_eq!(fit_inside((1280, 800), 3840, 2160), (1280, 800));
    }
}
