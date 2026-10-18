//! ScreenCaptureKit: display video, system audio, and microphone.

use std::sync::{Arc, mpsc};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_core_graphics::{CGDisplayCopyDisplayMode, CGDisplayMode, kCGColorSpaceSRGB, kCGDisplayStreamYCbCrMatrix_ITU_R_709_2};
use objc2_core_media::{CMSampleBuffer, CMTime};
use objc2_core_video::CVPixelBuffer;
use objc2_foundation::{NSArray, NSDictionary, NSError, NSNumber, NSObject, NSObjectProtocol, NSString};
use objc2_screen_capture_kit::*;
use dispatch2::DispatchQueue;

use crate::appaudio::{AppAudio, Pick};
use crate::record::Shared;
use crate::util::*;
use crate::writer::AudioKind;

#[derive(Clone)]
pub struct DisplayInfo {
    pub id: u32,
    pub points: (i64, i64),
    pub pixels: (usize, usize),
    pub display: Retained<SCDisplay>,
}

fn shareable_content() -> Result<Retained<SCShareableContent>> {
    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(move |content: *mut SCShareableContent, err: *mut NSError| {
        let r = if let Some(c) = unsafe { Retained::retain(content) } {
            Ok(SendBox(c))
        } else {
            let msg = unsafe { err.as_ref() }
                .map(|e| e.localizedDescription().to_string())
                .unwrap_or_else(|| "unknown error".into());
            Err(msg)
        };
        let _ = tx.send(r);
    });
    unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&block) };
    match rx.recv_timeout(Duration::from_secs(15)) {
        Ok(Ok(c)) => Ok(c.0),
        Ok(Err(e)) => bail!(
            "ScreenCaptureKit refused: {e}\n  -> grant Screen & System Audio Recording to Rec (see `rec permissions`)"
        ),
        Err(_) => bail!("timed out waiting for ScreenCaptureKit shareable content"),
    }
}

pub fn displays() -> Result<Vec<DisplayInfo>> {
    let content = shareable_content()?;
    let mut out = vec![];
    for d in unsafe { content.displays() }.iter() {
        let id = unsafe { d.displayID() };
        let mode = CGDisplayCopyDisplayMode(id);
        let pixels = (
            CGDisplayMode::pixel_width(mode.as_deref()),
            CGDisplayMode::pixel_height(mode.as_deref()),
        );
        out.push(DisplayInfo { id, points: unsafe { (d.width() as i64, d.height() as i64) }, pixels, display: d });
    }
    Ok(out)
}

pub struct WindowInfo {
    pub id: u32,
    pub app: String,
    pub title: String,
    /// Size in points.
    pub size: (f64, f64),
    pub window: Retained<SCWindow>,
}

/// On-screen, normal-layer windows that have a title or an owning app.
pub fn windows() -> Result<Vec<WindowInfo>> {
    let content = shareable_content()?;
    let mut out = vec![];
    for w in unsafe { content.windows() }.iter() {
        unsafe {
            if !w.isOnScreen() || w.windowLayer() != 0 {
                continue;
            }
            let app = w.owningApplication().map(|a| a.applicationName().to_string()).unwrap_or_default();
            let title = w.title().map(|t| t.to_string()).unwrap_or_default();
            let f = w.frame();
            if (app.is_empty() && title.is_empty()) || f.size.width < 40.0 || f.size.height < 40.0 {
                continue;
            }
            out.push(WindowInfo { id: w.windowID(), app, title, size: (f.size.width, f.size.height), window: w });
        }
    }
    Ok(out)
}

/// Find a window by numeric ID, or case-insensitive substring of
/// "<app> <title>". Prefers the largest match.
pub fn find_window(sel: &str) -> Result<WindowInfo> {
    let ws = windows()?;
    if let Ok(id) = sel.parse::<u32>() {
        if let Some(w) = ws.into_iter().find(|w| w.id == id) {
            return Ok(w);
        }
        bail!("no on-screen window with id {id} (see `rec-rs devices`)");
    }
    let lower = sel.to_lowercase();
    ws.into_iter()
        .filter(|w| format!("{} {}", w.app, w.title).to_lowercase().contains(&lower))
        .max_by(|a, b| (a.size.0 * a.size.1).total_cmp(&(b.size.0 * b.size.1)))
        .ok_or_else(|| anyhow!("no on-screen window matches {sel:?} (see `rec-rs devices`)"))
}

pub struct AppInfo {
    pub name: String,
    pub bundle_id: String,
    pub app: Retained<SCRunningApplication>,
}

/// Apps with a window on screen (for capturing all of an app's windows),
/// by name.
pub fn applications() -> Result<Vec<AppInfo>> {
    let content = shareable_content()?;
    let me = std::process::id() as i32;
    let mut pids = std::collections::HashSet::new();
    for w in unsafe { content.windows() }.iter() {
        unsafe {
            if w.isOnScreen() && w.windowLayer() == 0 && let Some(a) = w.owningApplication() {
                pids.insert(a.processID());
            }
        }
    }
    let mut out: Vec<AppInfo> = unsafe { content.applications() }
        .iter()
        .filter(|a| unsafe { a.processID() != me && pids.contains(&a.processID()) })
        .map(|a| unsafe { AppInfo { name: a.applicationName().to_string(), bundle_id: a.bundleIdentifier().to_string(), app: a } })
        .filter(|a| !a.name.is_empty())
        .collect();
    out.sort_by_key(|a| a.name.to_lowercase());
    Ok(out)
}

/// An app by bundle ID or (case-insensitive) name substring.
pub fn find_app(sel: &str) -> Result<AppInfo> {
    let lower = sel.to_lowercase();
    let apps = applications()?;
    if let Some(i) = apps.iter().position(|a| a.bundle_id == sel) {
        return Ok(apps.into_iter().nth(i).unwrap());
    }
    apps.into_iter()
        .find(|a| a.name.to_lowercase().contains(&lower))
        .ok_or_else(|| anyhow!("no app with a window on screen matches {sel:?} (see `rec-rs devices`)"))
}

pub enum Target {
    /// A display, optionally cropped to a region (points, top-left origin).
    Display(DisplayInfo, Option<objc2_core_foundation::CGRect>),
    Window(WindowInfo),
    /// Every window of an app on a display (the rest of the screen is
    /// left out).
    App(DisplayInfo, AppInfo),
}

impl Target {
    pub fn describe(&self) -> String {
        match self {
            Target::Display(d, None) => format!("display {}", d.id),
            Target::Display(d, Some(r)) => format!(
                "display {} region {}x{}+{}+{}",
                d.id, r.size.width, r.size.height, r.origin.x, r.origin.y
            ),
            Target::Window(w) => format!("window \"{} {}\" ({})", w.app, w.title, w.id),
            Target::App(d, a) => format!("app \"{}\" on display {}", a.name, d.id),
        }
    }
}

/// ScreenCaptureKit's HDR capture format: (pixel format, color space
/// name), from its HDR stream preset for the canonical display (rendered
/// for a reference HDR display, the same on any Mac).
pub fn hdr_format() -> (u32, String) {
    unsafe {
        let c = SCStreamConfiguration::streamConfigurationWithPreset(SCStreamConfigurationPreset::CaptureHDRStreamCanonicalDisplay);
        (c.pixelFormat(), c.colorSpaceName().to_string())
    }
}

pub struct ScreenConfig {
    pub target: Target,
    pub width: usize,
    pub height: usize,
    pub fps: u32,
    pub pixel_format: u32,
    pub cursor: bool,
    pub clicks: bool,
    pub system_audio: bool,
    /// None = no mic; Some(None) = default mic; Some(Some(id)) = specific device.
    pub mic: Option<Option<String>>,
    /// An extra screen source for scene layers (`Content::Display(n)`):
    /// its frames go to the sink as this source.
    pub extra: Option<crate::layout::Source>,
    /// Capture HDR (`hdr_format`); the pixel format is then the preset's.
    pub hdr: bool,
}

pub struct Ivars {
    shared: Arc<Shared>,
    extra: Option<crate::layout::Source>,
    /// Only the sound is wanted (a per-app audio stream).
    audio_only: bool,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "RecScreenOutput"]
    #[ivars = Ivars]
    pub struct ScreenOutput;

    unsafe impl NSObjectProtocol for ScreenOutput {}

    unsafe impl SCStreamOutput for ScreenOutput {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        unsafe fn did_output(&self, _stream: &SCStream, sb: &CMSampleBuffer, kind: SCStreamOutputType) {
            self.handle(sb, kind);
        }
    }

    unsafe impl SCStreamDelegate for ScreenOutput {
        #[unsafe(method(stream:didStopWithError:))]
        unsafe fn did_stop(&self, _stream: &SCStream, error: &NSError) {
            let msg = format!("screen capture stopped: {} [{} {}]", error.localizedDescription(), error.domain(), error.code());
            self.ivars().shared.fail(msg);
        }
    }
);

impl ScreenOutput {
    fn create(shared: Arc<Shared>, extra: Option<crate::layout::Source>, audio_only: bool) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars { shared, extra, audio_only });
        unsafe { msg_send![super(this), init] }
    }

    fn handle(&self, sb: &CMSampleBuffer, kind: SCStreamOutputType) {
        unsafe {
            if !sb.is_valid() {
                return;
            }
        }
        let shared = &self.ivars().shared;
        if kind == SCStreamOutputType::Screen {
            if self.ivars().audio_only || !frame_complete(sb) {
                return;
            }
            let Some(pb) = (unsafe { sb.image_buffer() }) else { return };
            let pts: CMTime = unsafe { sb.presentation_time_stamp() };
            let pb: objc2_core_foundation::CFRetained<CVPixelBuffer> = pb;
            match &self.ivars().extra {
                Some(src) => shared.with(|s| s.push_extra(src, pts, pb)),
                None => shared.with(|s| s.push_screen(pts, pb)),
            }
        } else if kind == SCStreamOutputType::Audio {
            shared.with(|s| s.push_audio(AudioKind::System, sb));
        } else if kind == SCStreamOutputType::Microphone {
            shared.with(|s| s.push_audio(AudioKind::Mic, sb));
        }
    }
}

/// Only `Complete` frames carry new pixels; idle/blank/suspended do not.
fn frame_complete(sb: &CMSampleBuffer) -> bool {
    unsafe {
        let Some(arr) = sb.sample_attachments_array(false) else { return false };
        let arr: &NSArray<NSDictionary<NSString, AnyObject>> =
            &*(&*arr as *const objc2_core_foundation::CFArray as *const NSArray<NSDictionary<NSString, AnyObject>>);
        let Some(info) = arr.firstObject() else { return false };
        let Some(status) = info.objectForKey(SCStreamFrameInfoStatus) else { return false };
        let Ok(n) = status.downcast::<NSNumber>() else { return false };
        n.integerValue() == SCFrameStatus::Complete.0
    }
}

pub struct ScreenCapture {
    stream: Retained<SCStream>,
    _output: Retained<ScreenOutput>,
    _queues: Vec<dispatch2::DispatchRetained<DispatchQueue>>,
}

unsafe impl Send for ScreenCapture {}

impl ScreenCapture {
    pub fn start(cfg: &ScreenConfig, shared: Arc<Shared>) -> Result<Self> {
        unsafe {
            let conf = if cfg.hdr {
                SCStreamConfiguration::streamConfigurationWithPreset(SCStreamConfigurationPreset::CaptureHDRStreamCanonicalDisplay)
            } else {
                SCStreamConfiguration::new()
            };
            let filter = match &cfg.target {
                Target::Display(d, region) => {
                    if let Some(r) = region {
                        conf.setSourceRect(*r);
                    }
                    // Exclude rec's own windows (the GUI preview), which
                    // would otherwise be recorded as a hall of mirrors.
                    let me = std::process::id() as i32;
                    let own = shareable_content()
                        .ok()
                        .and_then(|c| c.applications().iter().find(|a| a.processID() == me));
                    if std::env::var_os("REC_DEBUG").is_some() {
                        eprintln!("rec: debug: excluding own app from capture: {}", own.is_some());
                    }
                    match own {
                        Some(app) => SCContentFilter::initWithDisplay_excludingApplications_exceptingWindows(
                            SCContentFilter::alloc(),
                            &d.display,
                            &NSArray::from_retained_slice(&[app]),
                            &NSArray::new(),
                        ),
                        None => SCContentFilter::initWithDisplay_excludingWindows(
                            SCContentFilter::alloc(),
                            &d.display,
                            &NSArray::new(),
                        ),
                    }
                }
                Target::Window(w) => {
                    SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &w.window)
                }
                Target::App(d, a) => SCContentFilter::initWithDisplay_includingApplications_exceptingWindows(
                    SCContentFilter::alloc(),
                    &d.display,
                    &NSArray::from_retained_slice(std::slice::from_ref(&a.app)),
                    &NSArray::new(),
                ),
            };
            conf.setWidth(cfg.width);
            conf.setHeight(cfg.height);
            // Slightly under the frame period, so capture jitter never makes
            // ScreenCaptureKit skip a frame the pacer needs (as OBS does).
            conf.setMinimumFrameInterval(CMTime::new(9, cfg.fps as i32 * 10));
            if !cfg.hdr {
                conf.setPixelFormat(cfg.pixel_format);
            }
            conf.setQueueDepth(8);
            conf.setShowsCursor(cfg.cursor);
            conf.setShowMouseClicks(cfg.clicks);
            conf.setScalesToFit(true);
            conf.setPreservesAspectRatio(true);
            conf.setCaptureResolution(SCCaptureResolutionType::Best);
            if !cfg.hdr {
                conf.setColorSpaceName(kCGColorSpaceSRGB);
                if cfg.pixel_format == PIX_420V {
                    conf.setColorMatrix(kCGDisplayStreamYCbCrMatrix_ITU_R_709_2);
                }
            }
            conf.setCapturesAudio(cfg.system_audio);
            conf.setSampleRate(48000);
            conf.setChannelCount(2);
            conf.setExcludesCurrentProcessAudio(true);
            if let Some(dev) = &cfg.mic {
                conf.setCaptureMicrophone(true);
                if let Some(id) = dev {
                    conf.setMicrophoneCaptureDeviceID(Some(&NSString::from_str(id)));
                }
            }

            let output = ScreenOutput::create(shared, cfg.extra.clone(), false);
            let delegate = ProtocolObject::from_ref(&*output);
            let stream = SCStream::initWithFilter_configuration_delegate(SCStream::alloc(), &filter, &conf, Some(delegate));
            let out_proto: &ProtocolObject<dyn SCStreamOutput> = ProtocolObject::from_ref(&*output);
            let vq = DispatchQueue::new("rec.screen.video", None);
            let aq = DispatchQueue::new("rec.screen.audio", None);
            let mq = DispatchQueue::new("rec.screen.mic", None);
            stream
                .addStreamOutput_type_sampleHandlerQueue_error(out_proto, SCStreamOutputType::Screen, Some(&vq))
                .map_err(|e| anyhow!("add screen output: {}", e.localizedDescription()))?;
            if cfg.system_audio {
                stream
                    .addStreamOutput_type_sampleHandlerQueue_error(out_proto, SCStreamOutputType::Audio, Some(&aq))
                    .map_err(|e| anyhow!("add audio output: {}", e.localizedDescription()))?;
            }
            if cfg.mic.is_some() {
                stream
                    .addStreamOutput_type_sampleHandlerQueue_error(out_proto, SCStreamOutputType::Microphone, Some(&mq))
                    .map_err(|e| anyhow!("add microphone output: {}", e.localizedDescription()))?;
            }
            wait_err(|b| stream.startCaptureWithCompletionHandler(Some(b)))
                .context("starting screen capture")?;
            Ok(ScreenCapture { stream, _output: output, _queues: vec![vq, aq, mq] })
        }
    }

    /// System audio of the apps `sel` picks, as a stream of its own (the
    /// picture's filter stays as it is). `display`: the one captured, else
    /// the main one. None when `sel` picks no running app (silence).
    pub fn start_audio(display: Option<&SCDisplay>, sel: &AppAudio, shared: Arc<Shared>) -> Result<Option<Self>> {
        unsafe {
            let content = shareable_content()?;
            let display = match display {
                Some(d) => Retained::retain(d as *const SCDisplay as *mut SCDisplay).unwrap(),
                None => content.displays().firstObject().ok_or_else(|| anyhow!("no display"))?,
            };
            let apps: Vec<Retained<SCRunningApplication>> = content.applications().iter().collect();
            let ids: Vec<String> = apps.iter().map(|a| a.bundleIdentifier().to_string()).collect();
            let me = std::process::id() as i32;
            let own = apps.iter().position(|a| a.processID() == me);
            let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
            let chosen = |v: &[usize]| NSArray::from_retained_slice(&v.iter().map(|&i| apps[i].clone()).collect::<Vec<_>>());
            let filter = match sel.pick(&ids, own) {
                Pick::Include(v) if v.is_empty() => return Ok(None),
                Pick::Include(v) => SCContentFilter::initWithDisplay_includingApplications_exceptingWindows(SCContentFilter::alloc(), &display, &chosen(&v), &NSArray::new()),
                Pick::Exclude(v) => SCContentFilter::initWithDisplay_excludingApplications_exceptingWindows(SCContentFilter::alloc(), &display, &chosen(&v), &NSArray::new()),
                Pick::Everything => SCContentFilter::initWithDisplay_excludingWindows(SCContentFilter::alloc(), &display, &NSArray::new()),
            };
            // The smallest picture, once a second: only the sound is used.
            let conf = SCStreamConfiguration::new();
            conf.setWidth(2);
            conf.setHeight(2);
            conf.setMinimumFrameInterval(CMTime::new(1, 1));
            conf.setQueueDepth(3);
            conf.setCapturesAudio(true);
            conf.setSampleRate(48000);
            conf.setChannelCount(2);
            conf.setExcludesCurrentProcessAudio(true);
            let output = ScreenOutput::create(shared, None, true);
            let delegate = ProtocolObject::from_ref(&*output);
            let stream = SCStream::initWithFilter_configuration_delegate(SCStream::alloc(), &filter, &conf, Some(delegate));
            let out_proto: &ProtocolObject<dyn SCStreamOutput> = ProtocolObject::from_ref(&*output);
            let vq = DispatchQueue::new("rec.appaudio.video", None);
            let aq = DispatchQueue::new("rec.appaudio", None);
            stream
                .addStreamOutput_type_sampleHandlerQueue_error(out_proto, SCStreamOutputType::Screen, Some(&vq))
                .map_err(|e| anyhow!("add screen output: {}", e.localizedDescription()))?;
            stream
                .addStreamOutput_type_sampleHandlerQueue_error(out_proto, SCStreamOutputType::Audio, Some(&aq))
                .map_err(|e| anyhow!("add audio output: {}", e.localizedDescription()))?;
            wait_err(|b| stream.startCaptureWithCompletionHandler(Some(b))).context("starting app audio capture")?;
            Ok(Some(ScreenCapture { stream, _output: output, _queues: vec![vq, aq] }))
        }
    }

    pub fn stop(&self) -> Result<()> {
        unsafe { wait_err(|b| self.stream.stopCaptureWithCompletionHandler(Some(b))) }
    }
}

fn wait_err(f: impl FnOnce(&block2::DynBlock<dyn Fn(*mut NSError)>)) -> Result<()> {
    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(move |err: *mut NSError| {
        let r = match unsafe { err.as_ref() } {
            None => Ok(()),
            Some(e) => Err(format!("{} [{} {}]", e.localizedDescription(), e.domain(), e.code())),
        };
        let _ = tx.send(r);
    });
    f(&block);
    match rx.recv_timeout(Duration::from_secs(15)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => bail!("{e}"),
        Err(_) => bail!("timed out"),
    }
}
