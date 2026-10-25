//! Smart zoom: a recording made with `--smart-zoom` logs where the cursor
//! is and when it clicks (`<name>.events.json`), and keeps the screen on its
//! own (`<name>-screen.mov`) when the frame is composed. `export` then
//! writes `<name>-zoomed.mov`: the screen zoomed in around clicks, with the
//! rest of the scene (camera overlay, background) rebuilt as recorded and
//! the sound copied. The recording itself is the unzoomed version.
//!
//! Zooms are found from clicks (`segments`) and saved as
//! `<name>.zoom.json`, which a later export reuses, so it can be edited by
//! hand.

#![allow(deprecated)]

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail, ensure};
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2_av_foundation::*;
use objc2_core_foundation::{CFRetained, CGAffineTransform, CGPoint, CGRect, CGSize};
use objc2_core_image::{CIContext, CIImage};
use objc2_core_media::{CMFormatDescription, CMTime};
use objc2_core_video::{CVPixelBuffer, CVPixelBufferGetHeight, CVPixelBufferGetWidth, CVPixelBufferPool};
use objc2_foundation::NSURL;
use serde::{Deserialize, Serialize};

use crate::compose::{Compositor, PipSpec};
use crate::layout::{Background, SceneSpec, Transition};
use crate::util::*;
use crate::writer::{Codec, VideoSpec};

/// `rec-….mov` -> `rec-….events.json`.
pub fn events_path(rec: &Path) -> PathBuf {
    rec.with_extension("events.json")
}

/// `rec-….mov` -> `rec-….zoom.json` (the zooms an export uses).
pub fn segments_path(rec: &Path) -> PathBuf {
    rec.with_extension("zoom.json")
}

/// `rec-….mov` -> `rec-…-screen.mov`.
pub fn screen_file_path(rec: &Path) -> PathBuf {
    sibling(rec, "screen")
}

/// `rec-….mov` -> `rec-…-zoomed.mov`.
pub fn zoomed_path(rec: &Path) -> PathBuf {
    sibling(rec, "zoomed")
}

fn sibling(rec: &Path, suffix: &str) -> PathBuf {
    let stem = rec.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = rec.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "mov".into());
    rec.with_file_name(format!("{stem}-{suffix}.{ext}"))
}

/// The cursor at one output frame: where it is in the captured picture
/// (0-1 from the top left; outside that range when it is off the picture),
/// and whether the left button is down.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    /// The frame's time on the host clock (drives scene transitions when the
    /// scene is rebuilt).
    pub t: f64,
    pub x: f32,
    pub y: f32,
    #[serde(rename = "d")]
    pub down: bool,
    /// A key was pressed since the last frame (which one isn't kept).
    #[serde(rename = "k", default)]
    pub key: bool,
}

/// A change to the scene during the recording, applied at host time `t`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Op {
    Scene { scene: SceneSpec, animate: bool },
    Look { background: Background, transition: Transition, secs: f64 },
    Pip(PipSpec),
    MainCamera(Option<String>),
}

/// What a smart-zoom recording logged (`<name>.events.json`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventLog {
    pub version: u32,
    pub fps: u32,
    /// Output size.
    pub size: (usize, usize),
    pub codec: Codec,
    pub bitrate: u64,
    /// The frame was composed: the screen is in `<name>-screen.mov` and the
    /// camera in `<name>-camera.mov`; the scene is rebuilt from `scene`,
    /// `background`, `pip` and `ops`.
    pub composed: bool,
    pub scene: SceneSpec,
    pub background: Background,
    pub transition: Transition,
    pub transition_secs: f64,
    pub pip: PipSpec,
    pub ops: Vec<(f64, Op)>,
    /// The scene showed other displays, cameras, videos or web pages, which
    /// aren't kept: the scene can't be rebuilt.
    pub extras: bool,
    /// One per output frame.
    pub frames: Vec<Sample>,
}

/// Where the captured picture is on screen, in global points from the top
/// left of the main display.
pub enum Area {
    Fixed(CGRect),
    /// A window, followed as it moves.
    Window { id: u32, rect: CGRect, polled: u32 },
}

/// How often (in frames) a window's place is read again.
const WINDOW_POLL_FRAMES: u32 = 6;

impl Area {
    pub fn for_target(t: &crate::screen::Target) -> Area {
        use crate::screen::Target;
        match t {
            Target::Display(d, crop) => {
                let b = objc2_core_graphics::CGDisplayBounds(d.id);
                Area::Fixed(match crop {
                    Some(c) => CGRect::new(CGPoint::new(b.origin.x + c.origin.x, b.origin.y + c.origin.y), c.size),
                    None => b,
                })
            }
            Target::App(d, _) => Area::Fixed(objc2_core_graphics::CGDisplayBounds(d.id)),
            Target::Window(w) => {
                let rect = window_bounds(w.id).unwrap_or(CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(w.size.0, w.size.1)));
                Area::Window { id: w.id, rect, polled: 0 }
            }
        }
    }

    fn rect(&mut self) -> CGRect {
        match self {
            Area::Fixed(r) => *r,
            Area::Window { id, rect, polled } => {
                *polled += 1;
                if *polled >= WINDOW_POLL_FRAMES {
                    *polled = 0;
                    if let Some(r) = window_bounds(*id) {
                        *rect = r;
                    }
                }
                *rect
            }
        }
    }
}

/// A window's frame in global points (top-left origin).
fn window_bounds(id: u32) -> Option<CGRect> {
    use objc2_core_graphics::*;
    unsafe {
        let list = CGWindowListCopyWindowInfo(CGWindowListOption::OptionIncludingWindow, id)?;
        // SAFETY: CFArray and NSArray are toll-free bridged.
        let arr = &*(&*list as *const objc2_core_foundation::CFArray as *const objc2_foundation::NSArray<objc2_foundation::NSDictionary<objc2_foundation::NSString, objc2::runtime::AnyObject>>);
        let info = arr.firstObject()?;
        let bounds = info.objectForKey(cf_ns(kCGWindowBounds))?;
        let bounds = &*(Retained::as_ptr(&bounds) as *const objc2_core_foundation::CFDictionary);
        let mut r = CGRect::default();
        CGRectMakeWithDictionaryRepresentation(Some(bounds), &mut r).then_some(r)
    }
}

/// Key presses since login, system-wide: only a count (no permission needed;
/// which keys isn't known).
fn key_count() -> u32 {
    use objc2_core_graphics::*;
    CGEventSource::counter_for_event_type(CGEventSourceStateID::CombinedSessionState, CGEventType::KeyDown)
}

/// Logs a recording's cursor and scene changes (owned by the writer's
/// sink, which calls it for every output frame).
pub struct Recorder {
    area: Area,
    /// The system's key-down count at the last frame.
    keys: u32,
    log: EventLog,
    path: PathBuf,
}

impl Recorder {
    #[allow(clippy::too_many_arguments)]
    pub fn new(path: PathBuf, area: Area, spec: &VideoSpec, composed: bool, scene: SceneSpec, background: Background, transition: Transition, transition_secs: f64, pip: PipSpec) -> Recorder {
        let log = EventLog {
            version: 1,
            fps: spec.fps,
            size: (spec.width, spec.height),
            codec: spec.codec,
            bitrate: spec.bitrate,
            composed,
            scene,
            background,
            transition,
            transition_secs,
            pip,
            ops: vec![],
            extras: false,
            frames: vec![],
        };
        Recorder { area, keys: key_count(), log, path }
    }

    /// Output frame `index` shows host time `t`: note the cursor now.
    pub fn frame(&mut self, index: i64, t: f64) {
        let r = self.area.rect();
        let (p, down) = {
            use objc2_core_graphics::*;
            let e = CGEvent::new(None);
            let p = CGEvent::location(e.as_deref());
            (p, CGEventSource::button_state(CGEventSourceStateID::CombinedSessionState, CGMouseButton::Left))
        };
        let keys = key_count();
        let key = keys != self.keys;
        self.keys = keys;
        let s = Sample {
            t,
            x: ((p.x - r.origin.x) / r.size.width.max(1.0)) as f32,
            y: ((p.y - r.origin.y) / r.size.height.max(1.0)) as f32,
            down,
            key,
        };
        // Frames the encoder dropped repeat the last sample.
        while (self.log.frames.len() as i64) < index {
            let last = self.log.frames.last().copied().unwrap_or(s);
            self.log.frames.push(last);
        }
        if self.log.frames.len() as i64 == index {
            self.log.frames.push(s);
        }
    }

    pub fn op(&mut self, t: f64, op: Op) {
        self.log.ops.push((t, op));
    }

    pub fn note_extras(&mut self) {
        self.log.extras = true;
    }

    /// Write the log for a recording of `frames` frames.
    pub fn save(mut self, frames: i64) -> Result<PathBuf> {
        self.log.frames.truncate(frames.max(0) as usize);
        let f = std::fs::File::create(&self.path).with_context(|| format!("can't write {}", self.path.display()))?;
        serde_json::to_writer(std::io::BufWriter::new(f), &self.log)?;
        Ok(self.path)
    }
}

/// How zooms are found and how they move.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoomOptions {
    /// Magnification while zoomed in (1.25-3).
    pub level: f64,
    /// Find the zooms again even when `<name>.zoom.json` exists.
    pub fresh: bool,
}

impl Default for ZoomOptions {
    fn default() -> Self {
        ZoomOptions { level: 2.0, fresh: false }
    }
}

pub const LEVELS: std::ops::RangeInclusive<f64> = 1.25..=3.0;

/// Zoom in this long before a click.
pub const LEAD_SECS: f64 = 0.3;
/// Stay in this long after the last click.
pub const HOLD_SECS: f64 = 1.5;
/// Clicks closer than this share a zoom.
pub const GAP_SECS: f64 = 3.0;
/// A click this close to the edge of the zoomed view (a share of it) moves
/// the view.
const MARGIN: f64 = 0.1;
/// Spring stiffness: settles in about 0.4 s.
const OMEGA: f64 = 10.0;

/// Where a zoom looks from `t` on (0-1 of the picture).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Focus {
    pub t: f64,
    pub x: f64,
    pub y: f64,
}

/// One zoom: in from `start` to `end` (seconds into the recording), looking
/// at each focus in turn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub level: f64,
    pub focus: Vec<Focus>,
}

/// Left-button presses on the picture: (seconds into the recording, x, y).
pub fn clicks(frames: &[Sample], fps: u32) -> Vec<(f64, f64, f64)> {
    let mut out = vec![];
    let mut was = false;
    for (i, s) in frames.iter().enumerate() {
        let inside = (0.0..=1.0).contains(&s.x) && (0.0..=1.0).contains(&s.y);
        if s.down && !was && inside {
            out.push((i as f64 / fps as f64, s.x as f64, s.y as f64));
        }
        was = s.down;
    }
    out
}

/// Seconds into the recording of frames where a key was pressed.
pub fn typing(frames: &[Sample], fps: u32) -> Vec<f64> {
    frames.iter().enumerate().filter(|(_, s)| s.key).map(|(i, _)| i as f64 / fps as f64).collect()
}

/// The center a `level`x view at (x, y) actually has: kept inside the picture.
fn clamp_center(x: f64, y: f64, level: f64) -> (f64, f64) {
    let half = 0.5 / level.max(1.0);
    (x.clamp(half, 1.0 - half), y.clamp(half, 1.0 - half))
}

/// The zooms for `clicks` in a recording `duration` seconds long. Typing
/// (`keys`) keeps a zoom in, where it looks, but doesn't start one.
pub fn segments(clicks: &[(f64, f64, f64)], keys: &[f64], level: f64, duration: f64) -> Vec<Segment> {
    let mut out: Vec<Segment> = vec![];
    let mut last_click = f64::NEG_INFINITY;
    let mut keys = keys.iter().copied().peekable();
    for &(t, x, y) in clicks {
        // Typing before this click stretches the zoom it follows.
        while let Some(k) = keys.next_if(|&k| k < t) {
            if k - last_click <= GAP_SECS && let Some(seg) = out.last_mut() {
                last_click = k;
                seg.end = (k + HOLD_SECS).min(duration);
            }
        }
        let joins = t - last_click <= GAP_SECS && !out.is_empty();
        last_click = t;
        if joins {
            let seg = out.last_mut().unwrap();
            seg.end = (t + HOLD_SECS).min(duration);
            // Move only when the click is near or past the view's edge.
            let f = seg.focus.last().unwrap();
            let (cx, cy) = clamp_center(f.x, f.y, level);
            let reach = 0.5 / level * (1.0 - 2.0 * MARGIN);
            if (x - cx).abs() > reach || (y - cy).abs() > reach {
                seg.focus.push(Focus { t, x, y });
            }
        } else {
            out.push(Segment { start: (t - LEAD_SECS).max(0.0), end: (t + HOLD_SECS).min(duration), level, focus: vec![Focus { t: 0.0, x, y }] });
        }
    }
    for k in keys {
        if k - last_click <= GAP_SECS && let Some(seg) = out.last_mut() {
            last_click = k;
            seg.end = (k + HOLD_SECS).min(duration);
        }
    }
    out
}

/// The view at each output frame: (magnification, center x, center y),
/// eased with a critically damped spring.
pub fn camera_path(segs: &[Segment], frames: usize, fps: u32) -> Vec<(f64, f64, f64)> {
    let dt = 1.0 / fps as f64;
    let mut state = [1.0, 0.5, 0.5];
    let mut vel = [0.0; 3];
    let mut target = [1.0, 0.5, 0.5];
    let mut out = Vec::with_capacity(frames);
    for i in 0..frames {
        let t = i as f64 * dt;
        match segs.iter().find(|s| t >= s.start && t < s.end) {
            Some(s) => {
                let f = s.focus.iter().rev().find(|f| f.t <= t).unwrap_or(&s.focus[0]);
                let (x, y) = clamp_center(f.x, f.y, s.level);
                target = [s.level.max(1.0), x, y];
            }
            // Zooming out keeps looking where it was.
            None => target[0] = 1.0,
        }
        // A few substeps keep the spring stable at low frame rates.
        let steps = 4;
        let h = dt / steps as f64;
        for _ in 0..steps {
            for k in 0..3 {
                let a = OMEGA * OMEGA * (target[k] - state[k]) - 2.0 * OMEGA * vel[k];
                vel[k] += a * h;
                state[k] += vel[k] * h;
            }
        }
        let z = state[0].max(1.0);
        let (x, y) = clamp_center(state[1], state[2], z);
        out.push((z, x, y));
    }
    out
}

/// The zooms an export of `rec` uses: `<name>.zoom.json` if there is one
/// (unless `fresh`), else found from the clicks and saved there.
pub fn load_segments(rec: &Path, log: &EventLog, o: &ZoomOptions) -> Result<Vec<Segment>> {
    let path = segments_path(rec);
    if !o.fresh && path.exists() {
        let s = std::fs::read_to_string(&path)?;
        let mut segs: Vec<Segment> = serde_json::from_str(&s).with_context(|| format!("{} isn't a list of zooms", path.display()))?;
        segs.retain(|s| s.end > s.start && !s.focus.is_empty());
        return Ok(segs);
    }
    let duration = log.frames.len() as f64 / log.fps as f64;
    let segs = segments(&clicks(&log.frames, log.fps), &typing(&log.frames, log.fps), o.level.clamp(*LEVELS.start(), *LEVELS.end()), duration);
    std::fs::write(&path, serde_json::to_string_pretty(&segs)?)?;
    Ok(segs)
}

#[derive(Debug)]
pub struct ZoomStats {
    pub frames: usize,
    pub zooms: usize,
}

/// Whether `rec` was recorded for smart zoom.
pub fn available(rec: &Path) -> bool {
    events_path(rec).exists()
}

/// Write the zoomed version of `rec` to `output`. `progress` gets the share
/// done (0-1) now and then.
pub fn export(rec: &Path, output: &Path, o: &ZoomOptions, progress: &dyn Fn(f64)) -> Result<ZoomStats> {
    let ev = events_path(rec);
    let log: EventLog = serde_json::from_str(
        &std::fs::read_to_string(&ev).with_context(|| format!("{} wasn't recorded for smart zoom (no {})", rec.display(), ev.display()))?,
    )
    .with_context(|| format!("{} can't be read", ev.display()))?;
    ensure!(!log.frames.is_empty(), "{}: no frames logged", ev.display());
    ensure!(!(log.composed && log.extras), "the scene showed other displays, cameras, videos or web pages, which smart zoom can't rebuild");
    let segs = load_segments(rec, &log, o)?;
    if output.exists() {
        std::fs::remove_file(output)?;
    }
    let r = write(rec, output, &log, &segs, progress);
    if r.is_err() {
        let _ = std::fs::remove_file(output);
    }
    r.map(|frames| ZoomStats { frames, zooms: segs.len() })
}

/// A video track decoded to BGRA, read forward: the frame showing at a time.
struct Frames {
    _reader: Retained<AVAssetReader>,
    out: Retained<AVAssetReaderTrackOutput>,
    cur: Option<SendBox<CFRetained<CVPixelBuffer>>>,
    next: Option<(f64, SendBox<CFRetained<CVPixelBuffer>>)>,
    done: bool,
}

fn asset(path: &Path) -> Retained<AVURLAsset> {
    unsafe {
        let url = NSURL::fileURLWithPath(&ns(&path.to_string_lossy()));
        AVURLAsset::URLAssetWithURL_options(&url, None)
    }
}

fn reader_error(r: &AVAssetReader) -> String {
    unsafe { r.error().map(|e| e.localizedDescription().to_string()).unwrap_or_default() }
}

impl Frames {
    fn open(path: &Path) -> Result<Frames> {
        let name = path.display();
        unsafe {
            let asset = asset(path);
            let video = AVMediaTypeVideo.ok_or_else(|| anyhow!("no video media type"))?;
            let track = asset.tracksWithMediaType(video).firstObject().ok_or_else(|| anyhow!("{name}: no video track"))?;
            let reader = AVAssetReader::assetReaderWithAsset_error(&asset).map_err(|e| anyhow!("{name}: {}", e.localizedDescription()))?;
            let settings = dict(vec![(cf_ns(objc2_core_video::kCVPixelBufferPixelFormatTypeKey), num_i(PIX_BGRA as i64))]);
            let out = AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(&track, Some(&settings));
            out.setAlwaysCopiesSampleData(false);
            reader.addOutput(&out);
            if !reader.startReading() {
                bail!("{name}: can't be read ({})", reader_error(&reader));
            }
            let mut f = Frames { _reader: reader, out, cur: None, next: None, done: false };
            f.advance();
            Ok(f)
        }
    }

    fn advance(&mut self) {
        self.next = None;
        while !self.done {
            let Some(sb) = (unsafe { self.out.copyNextSampleBuffer() }) else {
                self.done = true;
                break;
            };
            if let Some(pb) = unsafe { sb.image_buffer() } {
                self.next = Some((secs(unsafe { sb.presentation_time_stamp() }), SendBox(pb)));
                break;
            }
        }
    }

    /// The newest frame at or before `t` (the first one before it starts).
    fn at(&mut self, t: f64) -> Option<&CVPixelBuffer> {
        while let Some((pts, _)) = &self.next {
            if *pts > t + 1e-4 && self.cur.is_some() {
                break;
            }
            self.cur = self.next.take().map(|n| n.1);
            self.advance();
        }
        self.cur.as_ref().map(|b| &*b.0)
    }
}

/// An audio track copied as it is.
struct AudioCopy {
    _reader: Retained<AVAssetReader>,
    out: Retained<AVAssetReaderTrackOutput>,
    input: Retained<AVAssetWriterInput>,
    done: bool,
}

fn buffer(pool: &CVPixelBufferPool) -> Option<CFRetained<CVPixelBuffer>> {
    let mut buf = std::ptr::null_mut();
    unsafe { CVPixelBufferPool::create_pixel_buffer(None, pool, std::ptr::NonNull::from(&mut buf)) };
    std::ptr::NonNull::new(buf).map(|b| unsafe { CFRetained::from_raw(b) })
}

/// `src` magnified `z` times around (x, y) (0-1 from the top left), drawn
/// into `dst`.
fn zoom_into(ctx: &CIContext, src: &CVPixelBuffer, dst: &CVPixelBuffer, (z, x, y): (f64, f64, f64)) {
    let (sw, sh) = (CVPixelBufferGetWidth(src) as f64, CVPixelBufferGetHeight(src) as f64);
    let (dw, dh) = (CVPixelBufferGetWidth(dst) as f64, CVPixelBufferGetHeight(dst) as f64);
    unsafe {
        let img = CIImage::imageWithCVPixelBuffer(src).imageByClampingToExtent();
        // The part shown, in CoreImage's bottom-left coordinates.
        let (vw, vh) = (sw / z, sh / z);
        let x0 = x * sw - vw / 2.0;
        let y0 = sh - (y * sh + vh / 2.0);
        let (kx, ky) = (dw / vw, dh / vh);
        let t = CGAffineTransform { a: kx, b: 0.0, c: 0.0, d: ky, tx: -x0 * kx, ty: -y0 * ky };
        let canvas = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(dw, dh));
        let out = img.imageByApplyingTransform(t).imageByCroppingToRect(canvas);
        let cs = objc2_core_graphics::CGColorSpace::with_name(Some(objc2_core_graphics::kCGColorSpaceSRGB));
        ctx.render_toCVPixelBuffer_bounds_colorSpace(&out, dst, canvas, cs.as_deref());
    }
}

fn write(rec: &Path, output: &Path, log: &EventLog, segs: &[Segment], progress: &dyn Fn(f64)) -> Result<usize> {
    let n = log.frames.len();
    let path = camera_path(segs, n, log.fps);
    let (w, h) = log.size;
    let spec = VideoSpec { width: w, height: h, fps: log.fps, codec: log.codec, bitrate: log.bitrate, pixel_format: PIX_BGRA, hdr_pq: None };
    let (writer, video, adaptor) = crate::writer::open_video(output, &spec)?;
    unsafe {
        video.setExpectsMediaDataInRealTime(false);
        // The sound, copied from the recording (same tracks, same order).
        let main = asset(rec);
        let audio_type = AVMediaTypeAudio.ok_or_else(|| anyhow!("no audio media type"))?;
        let mut audio = vec![];
        for track in main.tracksWithMediaType(audio_type).iter() {
            let reader = AVAssetReader::assetReaderWithAsset_error(&main).map_err(|e| anyhow!("{}: {}", rec.display(), e.localizedDescription()))?;
            let out = AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(&track, None);
            reader.addOutput(&out);
            let hint = track.formatDescriptions().firstObject().map(|f| Retained::cast_unchecked::<CMFormatDescription>(f));
            let input = AVAssetWriterInput::initWithMediaType_outputSettings_sourceFormatHint(AVAssetWriterInput::alloc(), audio_type, None, hint.as_deref());
            if !writer.canAddInput(&input) {
                bail!("can't copy the recording's sound");
            }
            writer.addInput(&input);
            if !reader.startReading() {
                bail!("{}: sound can't be read ({})", rec.display(), reader_error(&reader));
            }
            audio.push(AudioCopy { _reader: reader, out, input, done: false });
        }
        if audio.len() > 1 {
            // As recorded: players play the first track (the mix).
            let members: Vec<&AVAssetWriterInput> = audio.iter().map(|a| &*a.input).collect();
            let arr = objc2_foundation::NSArray::from_slice(&members);
            let group = AVAssetWriterInputGroup::initWithInputs_defaultInput(AVAssetWriterInputGroup::alloc(), &arr, Some(members[0]));
            if writer.canAddInputGroup(&group) {
                writer.addInputGroup(&group);
            }
        }
        if !writer.startWriting() {
            bail!("can't write {}: {}", output.display(), crate::writer::err_text(&writer));
        }
        writer.startSessionAtSourceTime(CMTime::new(0, log.fps as i32));

        let mut screen = Frames::open(&if log.composed { screen_file_path(rec) } else { rec.to_path_buf() })?;
        let cam_file = crate::record::camera_file_path(rec);
        let mut cam = if log.composed && cam_file.exists() { Some(Frames::open(&cam_file)?) } else { None };
        let mut comp = log.composed.then(|| {
            let mut c = Compositor::new(w, h, log.pip.clone());
            c.set_background(log.background.clone());
            c.set_transition(log.transition, log.transition_secs);
            c.set_scene(log.scene.clone(), false);
            c
        });
        let mut ops = log.ops.iter().peekable();
        let opts = dict(vec![(&*ns("kCIContextCacheIntermediates"), num_b(false))]);
        let ctx = CIContext::contextWithOptions(Some(&opts));
        let mut zoomed_pool: Option<((usize, usize), CFRetained<CVPixelBufferPool>)> = None;
        let pool = adaptor.pixelBufferPool().ok_or_else(|| anyhow!("no encoder buffers"))?;

        let mut i = 0;
        loop {
            let mut busy = false;
            if i < n && video.isReadyForMoreMediaData() {
                busy = true;
                let t = i as f64 / log.fps as f64;
                let host_t = log.frames[i].t;
                let src = screen.at(t).ok_or_else(|| anyhow!("{}: no screen frames", rec.display()))?;
                let dst = buffer(&pool).ok_or_else(|| anyhow!("no encoder buffer"))?;
                match comp.as_mut() {
                    None => zoom_into(&ctx, src, &dst, path[i]),
                    Some(c) => {
                        while let Some((_, op)) = ops.next_if(|(ot, _)| *ot < host_t) {
                            match op.clone() {
                                Op::Scene { scene, animate } => c.set_scene(scene, animate),
                                Op::Look { background, transition, secs } => {
                                    c.set_background(background);
                                    c.set_transition(transition, secs);
                                }
                                Op::Pip(p) => c.set_pip(p),
                                Op::MainCamera(id) => c.set_main_camera(id),
                            }
                        }
                        let size = (CVPixelBufferGetWidth(src), CVPixelBufferGetHeight(src));
                        if zoomed_pool.as_ref().is_none_or(|(s, _)| *s != size) {
                            zoomed_pool = Some((size, make_pool(size.0, size.1, PIX_BGRA)));
                        }
                        let z = buffer(&zoomed_pool.as_ref().unwrap().1).ok_or_else(|| anyhow!("no buffer"))?;
                        zoom_into(&ctx, src, &z, path[i]);
                        let cam_pb = cam.as_mut().and_then(|f| f.at(t));
                        c.render(host_t, Some(&z), cam_pb, &[], &dst);
                    }
                }
                if !adaptor.appendPixelBuffer_withPresentationTime(&dst, CMTime::new(i as i64, log.fps as i32)) {
                    bail!("writing failed: {}", crate::writer::err_text(&writer));
                }
                i += 1;
                if i % 30 == 0 {
                    progress(i as f64 / n as f64);
                }
                if i == n {
                    video.markAsFinished();
                }
            }
            for a in audio.iter_mut().filter(|a| !a.done) {
                if a.input.isReadyForMoreMediaData() {
                    busy = true;
                    match a.out.copyNextSampleBuffer() {
                        Some(sb) => {
                            if !a.input.appendSampleBuffer(&sb) {
                                bail!("writing sound failed: {}", crate::writer::err_text(&writer));
                            }
                        }
                        None => {
                            a.input.markAsFinished();
                            a.done = true;
                        }
                    }
                }
            }
            if i == n && audio.iter().all(|a| a.done) {
                break;
            }
            if writer.status() == AVAssetWriterStatus::Failed {
                bail!("writing failed: {}", crate::writer::err_text(&writer));
            }
            if !busy {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        writer.endSessionAtSourceTime(CMTime::new(n as i64, log.fps as i32));
        let (tx, rx) = std::sync::mpsc::channel();
        let block = block2::RcBlock::new(move || {
            let _ = tx.send(());
        });
        writer.finishWritingWithCompletionHandler(&block);
        rx.recv_timeout(std::time::Duration::from_secs(120)).context("timed out finalizing the movie")?;
        if writer.status() != AVAssetWriterStatus::Completed {
            bail!("writing failed: {}", crate::writer::err_text(&writer));
        }
        progress(1.0);
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: f32, y: f32, down: bool) -> Sample {
        Sample { t: 0.0, x, y, down, key: false }
    }

    #[test]
    fn clicks_are_presses_on_the_picture() {
        let f = [s(0.5, 0.5, false), s(0.5, 0.5, true), s(0.5, 0.5, true), s(0.5, 0.5, false), s(1.5, 0.5, true), s(0.2, 0.2, false), s(0.2, 0.3, true)];
        let c = clicks(&f, 10);
        assert_eq!(c.len(), 2);
        assert!((c[0].0 - 0.1).abs() < 1e-9);
        assert!((c[1].0 - 0.6).abs() < 1e-9 && (c[1].2 - 0.3).abs() < 1e-6);
    }

    #[test]
    fn close_clicks_share_a_zoom() {
        let segs = segments(&[(1.0, 0.5, 0.5), (2.0, 0.55, 0.5), (4.5, 0.5, 0.5), (10.0, 0.1, 0.1)], &[], 2.0, 11.0);
        assert_eq!(segs.len(), 2);
        assert!((segs[0].start - 0.7).abs() < 1e-9 && (segs[0].end - 6.0).abs() < 1e-9);
        // Near clicks don't move the view.
        assert_eq!(segs[0].focus.len(), 1);
        assert!((segs[1].end - 11.0).abs() < 1e-9, "clamped to the end");
    }

    #[test]
    fn typing_keeps_a_zoom_in() {
        // Click at 1 s, then type every second from 2 s to 8 s.
        let keys: Vec<f64> = (2..=8).map(|k| k as f64).collect();
        let segs = segments(&[(1.0, 0.3, 0.3)], &keys, 2.0, 20.0);
        assert_eq!(segs.len(), 1);
        assert!((segs[0].end - 9.5).abs() < 1e-9, "{}", segs[0].end);
        assert_eq!(segs[0].focus.len(), 1, "typing doesn't move the view");
        // Typing alone, or long after, starts nothing.
        assert!(segments(&[], &keys, 2.0, 20.0).is_empty());
        assert!((segments(&[(1.0, 0.3, 0.3)], &[10.0], 2.0, 20.0)[0].end - 2.5).abs() < 1e-9);
        // Typing then a click: one zoom.
        assert_eq!(segments(&[(1.0, 0.3, 0.3), (7.0, 0.3, 0.3)], &[3.0, 5.0], 2.0, 20.0).len(), 1);
    }

    #[test]
    fn far_clicks_pan() {
        let segs = segments(&[(1.0, 0.3, 0.3), (2.0, 0.9, 0.9)], &[], 2.0, 10.0);
        assert_eq!(segs[0].focus.len(), 2);
        assert_eq!(segs[0].focus[1].t, 2.0);
    }

    #[test]
    fn camera_eases_in_and_out_inside_the_picture() {
        let segs = segments(&[(1.0, 0.0, 0.0)], &[], 2.0, 5.0);
        let p = camera_path(&segs, 150, 30);
        assert_eq!(p[0], (1.0, 0.5, 0.5));
        // In by the click's hold, at the clamped corner.
        let (z, x, y) = p[60];
        assert!(z > 1.9, "zoomed in: {z}");
        assert!((x - 0.25).abs() < 0.02 && (y - 0.25).abs() < 0.02, "{x} {y}");
        assert!(p.iter().all(|&(z, x, y)| { let h = 0.5 / z; x >= h - 1e-9 && x <= 1.0 - h + 1e-9 && y >= h - 1e-9 && y <= 1.0 - h + 1e-9 }));
        assert!(p[149].0 < 1.05, "back out by the end: {}", p[149].0);
        // No jumps.
        assert!(p.windows(2).all(|w| (w[1].0 - w[0].0).abs() < 0.2));
    }

    #[test]
    fn names() {
        let r = Path::new("/m/rec-1.mov");
        assert_eq!(events_path(r), Path::new("/m/rec-1.events.json"));
        assert_eq!(segments_path(r), Path::new("/m/rec-1.zoom.json"));
        assert_eq!(screen_file_path(r), Path::new("/m/rec-1-screen.mov"));
        assert_eq!(zoomed_path(r), Path::new("/m/rec-1-zoomed.mov"));
    }

    fn ffmpeg(args: &[&str], out: &Path) {
        let ok = std::process::Command::new("ffmpeg").args(["-v", "error", "-y"]).args(args).arg(out).status().expect("ffmpeg (brew install ffmpeg)");
        assert!(ok.success());
    }

    fn log_for(composed: bool, n: usize) -> EventLog {
        // A click at 0.5 s near the top left, held 0.2 s.
        let frames = (0..n).map(|i| Sample { t: 100.0 + i as f64 / 30.0, x: 0.2, y: 0.2, down: (15..21).contains(&i), key: false }).collect();
        EventLog {
            version: 1,
            fps: 30,
            size: (640, 360),
            codec: Codec::H264,
            bitrate: 2_000_000,
            composed,
            scene: SceneSpec::Template(if composed { crate::layout::Scene::ScreenCam } else { crate::layout::Scene::Screen }),
            background: Background::default(),
            transition: Transition::Cut,
            transition_secs: 0.0,
            pip: PipSpec::default(),
            ops: vec![(100.9, Op::Pip(PipSpec { size: 0.3, ..PipSpec::default() }))],
            extras: false,
            frames,
        }
    }

    fn tracks(path: &Path) -> (f64, usize, usize) {
        unsafe {
            let a = asset(path);
            (secs(a.duration()), a.tracksWithMediaType(AVMediaTypeVideo.unwrap()).count(), a.tracksWithMediaType(AVMediaTypeAudio.unwrap()).count())
        }
    }

    /// 2 s clips, plain and composed with a camera: zoomed versions of the
    /// same length, with the sound, and the zooms saved for reuse.
    #[test]
    fn exports_zoomed_versions() {
        let dir = std::env::temp_dir().join(format!("rec-zoom-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = ["-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30", "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000", "-t", "2", "-pix_fmt", "yuv420p", "-c:a", "aac"];
        for composed in [false, true] {
            let rec = dir.join(format!("rec-{composed}.mov"));
            ffmpeg(&src, &rec);
            if composed {
                ffmpeg(&["-f", "lavfi", "-i", "testsrc=size=1280x720:rate=30", "-t", "2", "-pix_fmt", "yuv420p"], &screen_file_path(&rec));
                ffmpeg(&["-f", "lavfi", "-i", "smptebars=size=640x360:rate=30", "-t", "2", "-pix_fmt", "yuv420p"], &crate::record::camera_file_path(&rec));
            }
            std::fs::write(events_path(&rec), serde_json::to_string(&log_for(composed, 60)).unwrap()).unwrap();
            let out = zoomed_path(&rec);
            let stats = objc2::rc::autoreleasepool(|_| export(&rec, &out, &ZoomOptions::default(), &|p| assert!((0.0..=1.0).contains(&p)))).unwrap();
            assert_eq!((stats.frames, stats.zooms), (60, 1), "composed {composed}");
            let (d, v, a) = tracks(&out);
            assert!((d - 2.0).abs() < 0.1, "length {d}");
            assert_eq!((v, a), (1, 1));
            let segs: Vec<Segment> = serde_json::from_str(&std::fs::read_to_string(segments_path(&rec)).unwrap()).unwrap();
            assert_eq!(segs.len(), 1);
            // Again, over the existing output, from the saved zooms.
            objc2::rc::autoreleasepool(|_| export(&rec, &out, &ZoomOptions::default(), &|_| {})).unwrap();
        }
        // Not recorded for smart zoom: an error, no file.
        let plain = dir.join("plain.mov");
        ffmpeg(&src, &plain);
        assert!(export(&plain, &zoomed_path(&plain), &ZoomOptions::default(), &|_| {}).is_err());
        assert!(!zoomed_path(&plain).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
