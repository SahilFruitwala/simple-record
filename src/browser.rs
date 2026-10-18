//! A web page as a scene source (alerts, chat, a countdown...): an
//! offscreen WKWebView, snapshotted about 15 times a second and handed to
//! the sink as an extra source (`Source::Browser`). Transparent pages
//! stay transparent.
//!
//! WebKit lives on the main thread, so these run from its run loop (the
//! app's, or the command line's while it waits): a timer takes the
//! snapshots. The session holds only a stop flag; a page also stops once
//! its session's sink is gone.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use anyhow::{Result, anyhow};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSBackingStoreType, NSImage, NSWindow, NSWindowStyleMask};
use objc2_core_foundation::{CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGBitmapContextCreate, CGColorSpace, CGContext, kCGColorSpaceSRGB};
use objc2_core_video::*;
use objc2_foundation::{NSError, NSObjectNSKeyValueCoding, NSTimer, NSURL, NSURLRequest};
use objc2_web_kit::{WKSnapshotConfiguration, WKWebView, WKWebViewConfiguration};

use crate::layout::Source;
use crate::record::Shared;
use crate::util::*;

/// Snapshots a second.
const FPS: f64 = 15.0;

struct Page {
    view: Retained<WKWebView>,
    window: Retained<NSWindow>,
    timer: RefCell<Option<Retained<NSTimer>>>,
    src: Source,
    shared: Weak<Shared>,
    stop: Arc<AtomicBool>,
    size: (usize, usize),
    pool: CFRetained<CVPixelBufferPool>,
    /// A snapshot is being taken.
    busy: Cell<bool>,
}

thread_local! {
    /// The pages running (on the main thread).
    static PAGES: RefCell<Vec<Rc<Page>>> = const { RefCell::new(Vec::new()) };
}

/// `url`, or a local file's path, as a URL.
pub fn parse_url(url: &str) -> Option<Retained<NSURL>> {
    let path = std::path::Path::new(url);
    if !url.contains("://") && path.exists() {
        let abs = std::path::absolute(path).ok()?;
        return Some(NSURL::fileURLWithPath(&ns(abs.to_str()?)));
    }
    let u = NSURL::URLWithString(&ns(url))?;
    matches!(u.scheme()?.to_string().as_str(), "http" | "https" | "file").then_some(u)
}

/// Load `url` at `w`x`h` and send its picture to `shared` as `src` until
/// `stop` is set. Main thread only.
pub fn start(url: &str, w: usize, h: usize, src: Source, shared: &Arc<Shared>, stop: Arc<AtomicBool>) -> Result<()> {
    let mtm = MainThreadMarker::new().ok_or_else(|| anyhow!("browser layers need the main thread"))?;
    let u = parse_url(url).ok_or_else(|| anyhow!("not a web address: {url}"))?;
    let (w, h) = (w.max(16), h.max(16));
    unsafe {
        // WebKit needs the application object, even without a window.
        let _ = NSApplication::sharedApplication(mtm);
        let frame = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(w as f64, h as f64));
        let config = WKWebViewConfiguration::new(mtm);
        let view = WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), frame, &config);
        // Pages without a background of their own are transparent.
        view.setValue_forKey(Some(&*num_b(false)), &ns("drawsBackground"));
        // An offscreen window: WebKit renders only views in one.
        let at = CGRect::new(CGPoint::new(-20000.0, -20000.0), frame.size);
        let window = NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm), at, NSWindowStyleMask::Borderless, NSBackingStoreType::Buffered, false);
        window.setReleasedWhenClosed(false);
        window.setIgnoresMouseEvents(true);
        window.setContentView(Some(&view));
        window.orderBack(None);
        view.loadRequest(&NSURLRequest::requestWithURL(&u));
        let page = Rc::new(Page {
            view,
            window,
            timer: RefCell::new(None),
            src,
            shared: Arc::downgrade(shared),
            stop,
            size: (w, h),
            pool: make_pool(w, h, PIX_BGRA),
            busy: Cell::new(false),
        });
        let p = page.clone();
        let tick = RcBlock::new(move |_: std::ptr::NonNull<NSTimer>| tick(&p));
        *page.timer.borrow_mut() = Some(NSTimer::scheduledTimerWithTimeInterval_repeats_block(1.0 / FPS, true, &tick));
        PAGES.with(|v| {
            let mut v = v.borrow_mut();
            v.retain(|p| p.timer.borrow().is_some());
            v.push(page);
        });
    }
    Ok(())
}

fn close(p: &Page) {
    if let Some(t) = p.timer.borrow_mut().take() {
        t.invalidate();
    }
    p.window.close();
}

fn tick(p: &Rc<Page>) {
    if p.stop.load(Ordering::SeqCst) || p.shared.strong_count() == 0 {
        close(p);
        return;
    }
    if p.busy.get() {
        return;
    }
    p.busy.set(true);
    let mtm = MainThreadMarker::from(&*p.view);
    let page = p.clone();
    let done = RcBlock::new(move |img: *mut NSImage, _err: *mut NSError| {
        page.busy.set(false);
        if let Some(img) = unsafe { img.as_ref() }
            && let Some(pb) = frame(&page, img)
            && let Some(shared) = page.shared.upgrade()
        {
            shared.with(|s| s.push_extra(&page.src, host_now(), pb));
        }
    });
    unsafe {
        let cfg = WKSnapshotConfiguration::new(mtm);
        cfg.setAfterScreenUpdates(false);
        p.view.takeSnapshotWithConfiguration_completionHandler(Some(&cfg), &done);
    }
}

/// The snapshot drawn into a BGRA frame of the page's size.
fn frame(p: &Page, img: &NSImage) -> Option<CFRetained<CVPixelBuffer>> {
    unsafe {
        let cg = img.CGImageForProposedRect_context_hints(std::ptr::null_mut(), None, None)?;
        let mut buf = std::ptr::null_mut();
        CVPixelBufferPool::create_pixel_buffer(None, &p.pool, std::ptr::NonNull::from(&mut buf));
        let pb: CFRetained<CVPixelBuffer> = CFRetained::from_raw(std::ptr::NonNull::new(buf)?);
        let (w, h) = p.size;
        CVPixelBufferLockBaseAddress(&pb, CVPixelBufferLockFlags(0));
        let base = CVPixelBufferGetBaseAddress(&pb);
        let row = CVPixelBufferGetBytesPerRow(&pb);
        std::ptr::write_bytes(base as *mut u8, 0, row * h);
        let cs = CGColorSpace::with_name(Some(kCGColorSpaceSRGB));
        // Premultiplied BGRA: alpha first, 32-bit little-endian.
        let ctx = CGBitmapContextCreate(base, w, h, 8, row, cs.as_deref(), 2 | (2 << 12));
        if let Some(ctx) = &ctx {
            CGContext::draw_image(Some(ctx), CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(w as f64, h as f64)), Some(&cg));
        }
        CVPixelBufferUnlockBaseAddress(&pb, CVPixelBufferLockFlags(0));
        ctx.map(|_| pb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert!(parse_url("https://example.com/overlay?x=1").is_some());
        assert!(parse_url("http://localhost:8080").is_some());
        assert!(parse_url("javascript:alert(1)").is_none());
        assert!(parse_url("not a url").is_none());
        // A local file by path.
        let f = std::env::temp_dir().join(format!("rec-page-{}.html", std::process::id()));
        std::fs::write(&f, "<p>hi</p>").unwrap();
        let u = parse_url(f.to_str().unwrap()).unwrap();
        assert_eq!(u.scheme().unwrap().to_string(), "file");
        let _ = std::fs::remove_file(&f);
    }
}
