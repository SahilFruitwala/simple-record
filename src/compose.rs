//! GPU (CoreImage/Metal) compositing of scenes: the screen, the webcam,
//! images and text, placed by `crate::layout`, with outlines from
//! `crate::shapes`.

use std::collections::HashMap;

use objc2::rc::Retained;
use objc2_core_foundation::{CFRetained, CGAffineTransform, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGColorSpace, kCGColorSpaceSRGB};
use objc2_core_image::{CIColor, CIContext, CIFilter, CIImage, CIVector};
use objc2_core_video::CVPixelBuffer;
use objc2_foundation::NSData;

use crate::effects::{self, ChromaKey, Effects};
use crate::layout::{self, Background, Content, Source, Env, Frame, Placed, SceneSpec, Style, TextSpec, Transition};
use crate::util::*;

pub use crate::layout::Scene;
pub use crate::shapes::*;

/// Premultiplied BGRA image of `color` with per-pixel `alpha`.
fn alpha_image(alpha: &[u8], w: usize, h: usize, color: [u8; 3]) -> Retained<CIImage> {
    let mut px = Vec::with_capacity(w * h * 4);
    for &a in alpha {
        let m = |c: u8| ((c as u32 * a as u32 + 127) / 255) as u8;
        px.extend_from_slice(&[m(color[2]), m(color[1]), m(color[0]), a]);
    }
    unsafe {
        let data = objc2_foundation::NSData::with_bytes(&px);
        let cs = CGColorSpace::with_name(Some(kCGColorSpaceSRGB));
        CIImage::imageWithBitmapData_bytesPerRow_size_format_colorSpace(
            &data,
            w * 4,
            CGSize::new(w as f64, h as f64),
            objc2_core_image::kCIFormatBGRA8,
            cs.as_deref(),
        )
    }
}

type DecorKey = (usize, usize, Shape, u64, u64, [u8; 3], bool, u64);

/// Static layers for one layer size and style: where the picture shows,
/// its border and its shadow. They are built once on the CPU and reused
/// every frame; moving a layer only changes a transform, so dragging costs
/// nothing.
#[derive(Clone)]
struct Decor {
    key: DecorKey,
    /// Where the picture shows (inside the border, if any).
    mask: Retained<CIImage>,
    border: Option<Retained<CIImage>>,
    /// Shadow image and its padding in output pixels.
    shadow: Option<(Retained<CIImage>, f64)>,
}

impl Decor {
    fn key(style: &Style, w: usize, h: usize, out_h: f64, k: f64) -> DecorKey {
        let bw = border_px(style.border, out_h);
        (w, h, style.shape, style.roundness.to_bits(), bw.to_bits(), style.border_color, style.shadow, k.to_bits())
    }

    /// Built at `k` times the size and scaled up: below 1 while a
    /// transition animates the size every frame (cheap, and the softer
    /// edges don't show in motion).
    fn build(style: &Style, w: usize, h: usize, out_h: f64, k: f64) -> Decor {
        let (mw, mh) = (((w as f64 * k).round() as usize).max(1), ((h as f64 * k).round() as usize).max(1));
        let (sx, sy) = (w as f64 / mw as f64, h as f64 / mh as f64);
        let up = |img: Retained<CIImage>| -> Retained<CIImage> {
            if mw == w && mh == h {
                img
            } else {
                unsafe { img.imageByApplyingTransform(CGAffineTransform { a: sx, b: 0.0, c: 0.0, d: sy, tx: 0.0, ty: 0.0 }) }
            }
        };
        let bw = border_px(style.border, out_h) * mw as f64 / w as f64;
        let outer = mask(style.shape, mw, mh, style.roundness, 0.0);
        let inner = if bw > 0.0 { mask(style.shape, mw, mh, style.roundness, bw) } else { outer.clone() };
        let shadow = style.shadow.then(|| {
            let pad = ((out_h * 0.03).round().max(3.0) * mw as f64 / w as f64).round().max(1.0) as usize;
            let a = shadow(&outer, mw, mh, pad, 0.45);
            (up(alpha_image(&a, mw + 2 * pad, mh + 2 * pad, [0, 0, 0])), pad as f64 * sx)
        });
        Decor {
            key: Self::key(style, w, h, out_h, k),
            mask: up(alpha_image(&inner, mw, mh, [255, 255, 255])),
            border: (bw > 0.0).then(|| up(alpha_image(&outer, mw, mh, style.border_color))),
            shadow,
        }
    }
}

/// Border width in output pixels for a width given at 1080p.
fn border_px(border: f64, out_h: f64) -> f64 {
    (border.max(0.0) * out_h / 1080.0).round()
}
/// Fills a window capture's transparent rounded corners. ScreenCaptureKit
/// delivers a single window with its corners (and any other uncovered
/// pixels) transparent, which a video frame shows as black. Behind the
/// window this draws its own edge pixels stretched outward (a clamped inset
/// of the frame), so each corner takes the color of the window chrome next
/// to it: the frame stays edge to edge with no black and nothing cropped.
pub struct CornerFill {
    ctx: Retained<CIContext>,
    cs: CFRetained<CGColorSpace>,
    pool: Option<((usize, usize), CFRetained<objc2_core_video::CVPixelBufferPool>)>,
}


impl CornerFill {
    pub fn new() -> Self {
        unsafe {
            let opts = dict(vec![(&*ns("kCIContextCacheIntermediates"), num_b(false))]);
            let ctx = CIContext::contextWithOptions(Some(&opts));
            let cs = CGColorSpace::with_name(Some(kCGColorSpaceSRGB)).expect("sRGB colorspace");
            CornerFill { ctx, cs, pool: None }
        }
    }

    /// `pb` (BGRA) with its transparent pixels filled; None if it has no
    /// alpha to fill or a buffer could not be made.
    pub fn fill(&mut self, pb: &CVPixelBuffer) -> Option<CFRetained<CVPixelBuffer>> {
        use objc2_core_video::{CVPixelBufferGetHeight, CVPixelBufferGetPixelFormatType, CVPixelBufferGetWidth, CVPixelBufferPool};
        if CVPixelBufferGetPixelFormatType(pb) != PIX_BGRA {
            return None;
        }
        let (w, h) = (CVPixelBufferGetWidth(pb), CVPixelBufferGetHeight(pb));
        if self.pool.as_ref().is_none_or(|(sz, _)| *sz != (w, h)) {
            self.pool = Some(((w, h), make_pool(w, h, PIX_BGRA)));
        }
        let pool = &self.pool.as_ref()?.1;
        let mut out = std::ptr::null_mut();
        unsafe { CVPixelBufferPool::create_pixel_buffer(None, pool, std::ptr::NonNull::from(&mut out)) };
        let out: CFRetained<CVPixelBuffer> = unsafe { CFRetained::from_raw(std::ptr::NonNull::new(out)?) };
        unsafe {
            let img = CIImage::imageWithCVPixelBuffer(pb);
            let (fw, fh) = (w as f64, h as f64);
            let i = corner_inset(fw, fh);
            let canvas = rect(0.0, 0.0, fw, fh);
            let back = img.imageByCroppingToRect(rect(i, i, fw - 2.0 * i, fh - 2.0 * i)).imageByClampingToExtent().imageByCroppingToRect(canvas);
            let filled = img.imageByCompositingOverImage(&back);
            self.ctx.render_toCVPixelBuffer_bounds_colorSpace(&filled, &out, canvas, Some(&self.cs));
        }
        Some(out)
    }
}

impl Default for CornerFill {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Compositor {
    ctx: Retained<CIContext>,
    cs: CFRetained<CGColorSpace>,
    w: f64,
    h: f64,
    pip: PipSpec,
    scene: SceneSpec,
    background: Background,
    transition: Transition,
    transition_secs: f64,
    /// The scene being left and when (on the frame clock, seconds) the
    /// change began.
    leaving: Option<(SceneSpec, f64)>,
    /// Time of the last frame rendered.
    now: f64,
    /// Recently used layer decorations, oldest first.
    decor: Vec<Decor>,
    /// Image files (None: could not be read).
    images: HashMap<String, Option<Retained<CIImage>>>,
    texts: Vec<((TextSpec, u64), Retained<CIImage>)>,
    /// Aspect ratio of the last camera frame (16:9 until one arrives).
    cam_aspect: f64,
    /// Aspect ratio of the last screen frame (the frame's until one arrives).
    screen_aspect: f64,
    /// The main camera's unique ID (a `Content::Webcam` layer naming it
    /// shows the main camera's frames).
    main_cam: Option<String>,
    /// Chroma key lookup tables, most recent last.
    cubes: Vec<(ChromaKey, Retained<NSData>)>,
    /// sRGB, which the tables are indexed in (whatever the output is in).
    srgb: CFRetained<CGColorSpace>,
}

/// Chroma keys kept (a few layers keying different colors).
const CUBE_CACHE: usize = 4;

/// Decorations kept for reuse (a few layers, plus a transition's sizes).
const DECOR_CACHE: usize = 12;

fn rect(x: f64, y: f64, w: f64, h: f64) -> CGRect {
    CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))
}

fn translate(img: &CIImage, x: f64, y: f64) -> Retained<CIImage> {
    unsafe { img.imageByApplyingTransform(CGAffineTransform { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: x, ty: y }) }
}

fn ci_color(c: [u8; 3], alpha: f64) -> Retained<CIColor> {
    let f = |v: u8| v as f64 / 255.0;
    unsafe { CIColor::colorWithRed_green_blue_alpha(f(c[0]), f(c[1]), f(c[2]), alpha) }
}

/// `img` with its alpha multiplied by `alpha`.
fn faded(img: &CIImage, alpha: f64) -> Retained<CIImage> {
    unsafe {
        let v = CIVector::vectorWithX_Y_Z_W(0.0, 0.0, 0.0, alpha);
        let params = dict(vec![(&*ns("inputAVector"), obj(&*v))]);
        img.imageByApplyingFilter_withInputParameters(&ns("CIColorMatrix"), &params)
    }
}

/// A generator filter's output (no input image).
fn generate(name: &str, params: Vec<(&objc2_foundation::NSString, Retained<objc2::runtime::AnyObject>)>) -> Option<Retained<CIImage>> {
    unsafe { CIFilter::filterWithName_withInputParameters(&ns(name), Some(&dict(params)))?.outputImage() }
}

impl Compositor {
    pub fn new(width: usize, height: usize, pip: PipSpec) -> Self {
        unsafe {
            let opts = dict(vec![(&*ns("kCIContextCacheIntermediates"), num_b(false))]);
            let ctx = CIContext::contextWithOptions(Some(&opts));
            let cs = CGColorSpace::with_name(Some(kCGColorSpaceSRGB)).expect("sRGB colorspace");
            Compositor {
                ctx,
                w: width as f64,
                h: height as f64,
                pip,
                scene: SceneSpec::default(),
                background: Background::default(),
                transition: Transition::default(),
                transition_secs: 0.4,
                leaving: None,
                now: 0.0,
                decor: vec![],
                images: HashMap::new(),
                texts: vec![],
                cam_aspect: 16.0 / 9.0,
                screen_aspect: width as f64 / height.max(1) as f64,
                main_cam: None,
                cubes: vec![],
                srgb: cs.clone(),
                cs,
            }
        }
    }

    /// Change the overlay live (position, shape, size, style).
    pub fn set_pip(&mut self, pip: PipSpec) {
        self.pip = pip;
    }

    /// Render in this color space (HDR capture's), not sRGB.
    pub fn set_color_space(&mut self, name: &str) {
        let cf = objc2_core_foundation::CFString::from_str(name);
        if let Some(cs) = CGColorSpace::with_name(Some(&cf)) {
            self.cs = cs;
        }
    }

    pub fn set_main_camera(&mut self, id: Option<String>) {
        self.main_cam = id;
    }

    pub fn set_background(&mut self, bg: Background) {
        self.background = bg;
    }

    pub fn set_transition(&mut self, kind: Transition, secs: f64) {
        self.transition = kind;
        self.transition_secs = secs.clamp(0.0, 3.0);
    }

    /// Show `scene`, animated with the transition when `animate` (from the
    /// last frame rendered).
    pub fn set_scene(&mut self, scene: SceneSpec, animate: bool) {
        if scene == self.scene {
            return;
        }
        let old = std::mem::replace(&mut self.scene, scene);
        self.leaving = (animate && self.transition != Transition::Cut && self.transition_secs > 0.0).then_some((old, self.now));
    }

    pub fn cam_aspect(&self) -> f64 {
        self.cam_aspect
    }

    fn env(&self) -> Env {
        Env { w: self.w, h: self.h, screen_aspect: self.screen_aspect, cam_aspect: self.cam_aspect, pip: self.pip.clone(), background: self.background }
    }

    /// Whether the scene (or the one being left) shows the screen: without
    /// a screen frame there is nothing to emit then.
    pub fn needs_screen(&self) -> bool {
        let env = self.env();
        self.scene.resolve(&env).uses_screen() || self.leaving.as_ref().is_some_and(|(s, _)| s.resolve(&env).uses_screen())
    }

    /// The size to scale a `sw`x`sh` screen frame to before compositing:
    /// the largest size the scene draws it at, when smaller than the frame.
    /// The compositor then draws it 1:1, so the high-quality scaler does
    /// all the resampling.
    pub fn screen_target(&self, sw: f64, sh: f64) -> Option<(usize, usize)> {
        let env = Env { screen_aspect: sw / sh.max(1.0), ..self.env() };
        let f = self.scene.resolve(&env);
        let (tw, th) = f
            .layers
            .iter()
            .filter(|l| l.content == Content::Screen)
            .map(|l| {
                let (_, sx, sy) = layout::place(sw, sh, l.rect, l.fit);
                ((sw * sx).round().max(2.0), (sh * sy).round().max(2.0))
            })
            .max_by(|a, b| (a.0 * a.1).total_cmp(&(b.0 * b.1)))?;
        (tw < sw && th < sh).then_some((tw as usize, th as usize))
    }

    fn decor(&mut self, style: &Style, w: usize, h: usize, k: f64) -> Decor {
        let key = Decor::key(style, w, h, self.h, k);
        if let Some(i) = self.decor.iter().position(|d| d.key == key) {
            let d = self.decor.remove(i);
            self.decor.push(d.clone());
            return d;
        }
        let d = Decor::build(style, w, h, self.h, k);
        if self.decor.len() >= DECOR_CACHE {
            self.decor.remove(0);
        }
        self.decor.push(d.clone());
        d
    }

    fn image(&mut self, path: &str) -> Option<Retained<CIImage>> {
        self.images
            .entry(path.to_string())
            .or_insert_with(|| unsafe {
                let url = objc2_foundation::NSURL::fileURLWithPath(&ns(path));
                let img = CIImage::imageWithContentsOfURL(&url);
                if img.is_none() {
                    eprintln!("rec: warning: could not read image {path}");
                }
                img
            })
            .clone()
    }

    /// `t` rendered at `px` pixels tall (cached by size, in 4 px steps so
    /// an animated box doesn't render text every frame).
    fn text(&mut self, t: &TextSpec, px: f64) -> Option<Retained<CIImage>> {
        if t.text.trim().is_empty() {
            return None;
        }
        let size = ((px * 0.8 / 4.0).round() * 4.0).max(8.0);
        let key = (t.clone(), size.to_bits());
        if let Some((_, img)) = self.texts.iter().find(|(k, _)| *k == key) {
            return Some(img.clone());
        }
        let raw = generate(
            "CITextImageGenerator",
            vec![
                (&*ns("inputText"), obj(&*ns(&t.text))),
                (&*ns("inputFontName"), obj(&*ns("HelveticaNeue-Bold"))),
                (&*ns("inputFontSize"), num_f(size)),
                (&*ns("inputScaleFactor"), num_f(1.0)),
            ],
        )?;
        // Recolor: the glyphs' coverage with the text color.
        let img = unsafe {
            let zero = CIVector::vectorWithX_Y_Z_W(0.0, 0.0, 0.0, 0.0);
            let f = |v: u8| v as f64 / 255.0;
            let bias = CIVector::vectorWithX_Y_Z_W(f(t.color[0]), f(t.color[1]), f(t.color[2]), 0.0);
            let params = dict(vec![
                (&*ns("inputRVector"), obj(&*zero)),
                (&*ns("inputGVector"), obj(&*zero)),
                (&*ns("inputBVector"), obj(&*zero)),
                (&*ns("inputBiasVector"), obj(&*bias)),
            ]);
            raw.imageByApplyingFilter_withInputParameters(&ns("CIColorMatrix"), &params)
        };
        if self.texts.len() >= 8 {
            self.texts.remove(0);
        }
        self.texts.push((key, img.clone()));
        Some(img)
    }

    fn background_image(&self, bg: Background) -> Retained<CIImage> {
        let canvas = rect(0.0, 0.0, self.w, self.h);
        unsafe {
            let img = if bg.top == bg.bottom {
                Some(CIImage::imageWithColor(&ci_color(bg.top, 1.0)))
            } else {
                generate(
                    "CILinearGradient",
                    vec![
                        (&*ns("inputPoint0"), obj(&*CIVector::vectorWithX_Y(0.0, self.h))),
                        (&*ns("inputPoint1"), obj(&*CIVector::vectorWithX_Y(0.0, 0.0))),
                        (&*ns("inputColor0"), obj(&*ci_color(bg.top, 1.0))),
                        (&*ns("inputColor1"), obj(&*ci_color(bg.bottom, 1.0))),
                    ],
                )
            };
            img.unwrap_or_else(|| CIImage::imageWithColor(&CIColor::colorWithRed_green_blue(0.0, 0.0, 0.0))).imageByCroppingToRect(canvas)
        }
    }

    fn cube(&mut self, k: &ChromaKey) -> Retained<NSData> {
        if let Some(i) = self.cubes.iter().position(|(c, _)| c == k) {
            let hit = self.cubes.remove(i);
            self.cubes.push(hit);
        } else {
            let floats = effects::chroma_cube(k, effects::CUBE_SIZE);
            let bytes: Vec<u8> = floats.iter().flat_map(|f| f.to_ne_bytes()).collect();
            if self.cubes.len() >= CUBE_CACHE {
                self.cubes.remove(0);
            }
            self.cubes.push((*k, NSData::with_bytes(&bytes)));
        }
        self.cubes.last().unwrap().1.clone()
    }

    /// `img` through a layer's filters: the key, then color correction.
    fn filtered(&mut self, img: Retained<CIImage>, e: &Effects) -> Retained<CIImage> {
        let mut img = img;
        unsafe {
            if let Some(k) = &e.chroma_key {
                let data = self.cube(k);
                let cs: Retained<objc2::runtime::AnyObject> = Retained::retain(&*self.srgb as *const CGColorSpace as *mut objc2::runtime::AnyObject).unwrap();
                let params = dict(vec![
                    (&*ns("inputCubeDimension"), num_i(effects::CUBE_SIZE as i64)),
                    (&*ns("inputCubeData"), obj(&*data)),
                    (&*ns("inputColorSpace"), cs),
                ]);
                img = img.imageByApplyingFilter_withInputParameters(&ns("CIColorCubeWithColorSpace"), &params);
            }
            if e.has_color() {
                let params = dict(vec![
                    (&*ns("inputBrightness"), num_f(e.brightness.clamp(-1.0, 1.0))),
                    (&*ns("inputContrast"), num_f(e.contrast.clamp(0.0, 4.0))),
                    (&*ns("inputSaturation"), num_f(e.saturation.clamp(0.0, 4.0))),
                ]);
                img = img.imageByApplyingFilter_withInputParameters(&ns("CIColorControls"), &params);
            }
        }
        img
    }

    /// Draw `l` showing `src` over `under`.
    fn place(&mut self, l: &Placed, src: &CIImage, under: Retained<CIImage>, k: f64) -> Retained<CIImage> {
        unsafe {
            let e = src.extent();
            let (ew, eh) = (e.size.width, e.size.height);
            if !(ew > 0.0 && eh > 0.0 && ew.is_finite() && eh.is_finite()) {
                return under;
            }
            let ([dx, top, dw, dh], sx, sy) = layout::place(ew, eh, l.rect, l.fit);
            // CoreImage's origin is bottom-left.
            let dy = self.h - top - dh;
            let (cx, cy) = (dx + dw / 2.0, dy + dh / 2.0);
            let a = if l.mirror { -sx } else { sx };
            let t = CGAffineTransform { a, b: 0.0, c: 0.0, d: sy, tx: cx - a * (e.origin.x + ew / 2.0), ty: cy - sy * (e.origin.y + eh / 2.0) };
            let mut placed = src.imageByApplyingTransform(t).imageByCroppingToRect(rect(dx, dy, dw, dh));
            let alpha = l.opacity.clamp(0.0, 1.0);
            if alpha < 1.0 {
                placed = faded(&placed, alpha);
            }
            if l.style.is_plain() {
                return placed.imageByCompositingOverImage(&under);
            }
            // While a transition animates sizes, outlines are rebuilt every
            // frame: at most 400 px across keeps that to a millisecond or two.
            let k = if k < 1.0 { (400.0 / dw.max(dh)).min(1.0) } else { 1.0 };
            let d = self.decor(&l.style, dw.round().max(1.0) as usize, dh.round().max(1.0) as usize, k);
            let fade = |img: Retained<CIImage>| if alpha < 1.0 { faded(&img, alpha) } else { img };
            let mut img = under;
            if let Some((sh, pad)) = &d.shadow {
                // Drop the shadow slightly below the layer.
                let drop = (self.h * 0.006).round();
                img = fade(translate(sh, dx - pad, dy - pad - drop)).imageByCompositingOverImage(&img);
            }
            if let Some(b) = &d.border {
                img = fade(translate(b, dx, dy)).imageByCompositingOverImage(&img);
            }
            let params = dict(vec![(&*ns("inputBackgroundImage"), obj(&*translate(&d.mask, dx, dy)))]);
            let masked = placed.imageByApplyingFilter_withInputParameters(&ns("CISourceInCompositing"), &params);
            masked.imageByCompositingOverImage(&img)
        }
    }

    /// Draw a resolved frame.
    fn draw(&mut self, f: &Frame, screen: Option<&Retained<CIImage>>, cam: Option<&Retained<CIImage>>, extras: &[(&Source, Retained<CIImage>)], k: f64) -> Retained<CIImage> {
        let mut img = self.background_image(f.background);
        for l in &f.layers {
            if l.opacity <= 0.0 {
                continue;
            }
            let src = match &l.content {
                Content::Screen => screen.cloned(),
                Content::Camera => cam.cloned(),
                Content::Webcam { id, .. } if self.main_cam.as_ref() == Some(id) => cam.cloned(),
                Content::Display(_) | Content::Webcam { .. } => {
                    let want = l.content.source();
                    extras.iter().find(|(s, _)| Some(*s) == want.as_ref()).map(|(_, i)| i.clone())
                }
                Content::Image(p) => self.image(p),
                Content::Text(t) => self.text(t, l.rect[3]),
                Content::Color(c) => {
                    let r = rect(0.0, 0.0, l.rect[2].max(1.0), l.rect[3].max(1.0));
                    Some(unsafe { CIImage::imageWithColor(&ci_color(*c, 1.0)).imageByCroppingToRect(r) })
                }
            };
            if let Some(src) = src {
                let src = if l.effects.is_none() { src } else { self.filtered(src, &l.effects) };
                img = self.place(l, &src, img, k);
            }
        }
        img
    }

    /// Render the scene at frame time `t` (seconds; drives transitions).
    /// `extras` are other displays' and cameras' frames.
    pub fn render(&mut self, t: f64, screen: Option<&CVPixelBuffer>, cam: Option<&CVPixelBuffer>, extras: &[(&Source, &CVPixelBuffer)], out: &CVPixelBuffer) {
        self.now = t;
        let canvas = rect(0.0, 0.0, self.w, self.h);
        let screen = screen.map(|s| unsafe { CIImage::imageWithCVPixelBuffer(s) });
        let cam = cam.map(|c| unsafe { CIImage::imageWithCVPixelBuffer(c) });
        let extras: Vec<(&Source, Retained<CIImage>)> = extras.iter().map(|(n, b)| (*n, unsafe { CIImage::imageWithCVPixelBuffer(b) })).collect();
        for (img, aspect) in [(&cam, &mut self.cam_aspect), (&screen, &mut self.screen_aspect)] {
            if let Some(i) = img {
                let e = unsafe { i.extent() };
                if e.size.height > 0.0 {
                    *aspect = e.size.width / e.size.height;
                }
            }
        }
        let env = self.env();
        let cur = self.scene.resolve(&env);
        let progress = self.leaving.as_ref().map(|(_, start)| (t - start) / self.transition_secs.max(1e-3));
        if progress.is_some_and(|p| !(0.0..1.0).contains(&p)) {
            self.leaving = None;
        }
        let img = match (self.leaving.clone(), progress) {
            (Some((old, _)), Some(p)) => {
                let prev = old.resolve(&env);
                match self.transition {
                    Transition::Fade => {
                        let a = self.draw(&prev, screen.as_ref(), cam.as_ref(), &extras, 1.0);
                        let b = self.draw(&cur, screen.as_ref(), cam.as_ref(), &extras, 1.0);
                        unsafe {
                            let params = dict(vec![
                                (&*ns("inputTargetImage"), obj(&*b)),
                                (&*ns("inputTime"), num_f(layout::smoothstep(p))),
                            ]);
                            a.imageByApplyingFilter_withInputParameters(&ns("CIDissolveTransition"), &params)
                        }
                    }
                    // Below 1: decorations at reduced size (see `place`).
                    _ => self.draw(&layout::blend(&prev, &cur, p), screen.as_ref(), cam.as_ref(), &extras, 0.5),
                }
            }
            _ => self.draw(&cur, screen.as_ref(), cam.as_ref(), &extras, 1.0),
        };
        unsafe { self.ctx.render_toCVPixelBuffer_bounds_colorSpace(&img.imageByCroppingToRect(canvas), out, canvas, Some(&self.cs)) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_core_video::*;

    /// A `w`x`h` frame of one BGRA color (and the pool it came from).
    fn solid(w: usize, h: usize, bgra: [u8; 4]) -> (CFRetained<CVPixelBuffer>, CFRetained<CVPixelBufferPool>) {
        let pb = crate::util::make_pool(w, h, PIX_BGRA);
        let mut buf = std::ptr::null_mut();
        unsafe {
            CVPixelBufferPool::create_pixel_buffer(None, &pb, std::ptr::NonNull::from(&mut buf));
            let b: CFRetained<CVPixelBuffer> = CFRetained::from_raw(std::ptr::NonNull::new(buf).unwrap());
            CVPixelBufferLockBaseAddress(&b, CVPixelBufferLockFlags(0));
            let base = CVPixelBufferGetBaseAddress(&b) as *mut u8;
            let stride = CVPixelBufferGetBytesPerRow(&b);
            for y in 0..h {
                for x in 0..w {
                    std::ptr::copy_nonoverlapping(bgra.as_ptr(), base.add(y * stride + x * 4), 4);
                }
            }
            CVPixelBufferUnlockBaseAddress(&b, CVPixelBufferLockFlags(0));
            (b, pb)
        }
    }

    /// The BGR of pixel (x, y).
    fn pixel(b: &CVPixelBuffer, x: usize, y: usize) -> [u8; 3] {
        unsafe {
            CVPixelBufferLockBaseAddress(b, CVPixelBufferLockFlags::ReadOnly);
            let base = CVPixelBufferGetBaseAddress(b) as *const u8;
            let p = base.add(y * CVPixelBufferGetBytesPerRow(b) + x * 4);
            let v = [*p, *p.add(1), *p.add(2)];
            CVPixelBufferUnlockBaseAddress(b, CVPixelBufferLockFlags::ReadOnly);
            v
        }
    }

    /// The camera keyed out over the background, and color correction.
    #[test]
    fn chroma_key_and_color_correction() {
        let (green, _p1) = solid(320, 180, [0, 255, 0, 255]);
        let (red, _p2) = solid(320, 180, [0, 0, 255, 255]);
        let (out, _p3) = solid(320, 180, [0, 0, 0, 255]);
        let bg = Background { top: [200, 100, 50], bottom: [200, 100, 50] };
        let keyed = PipSpec { effects: Effects { chroma_key: Some(ChromaKey::default()), ..Default::default() }, ..Default::default() };
        let mut c = Compositor::new(320, 180, keyed.clone());
        c.set_background(bg);
        c.set_scene(SceneSpec::Template(Scene::Cam), false);
        c.render(0.0, None, Some(&green), &[], &out);
        let [b, g, r] = pixel(&out, 160, 90);
        assert!(r > 180 && (80..=120).contains(&g) && b < 70, "the background shows through: {:?}", [b, g, r]);
        // The key leaves other colors alone.
        c.render(0.1, None, Some(&red), &[], &out);
        let [b, g, r] = pixel(&out, 160, 90);
        assert!(r > 220 && g < 30 && b < 30, "red kept: {:?}", [b, g, r]);
        // No saturation: red goes gray.
        c.set_pip(PipSpec { effects: Effects { saturation: 0.0, ..Default::default() }, ..keyed });
        c.render(0.2, None, Some(&red), &[], &out);
        let [b, g, r] = pixel(&out, 160, 90);
        assert!(r.abs_diff(g) < 8 && g.abs_diff(b) < 8 && r > 20, "gray: {:?}", [b, g, r]);
    }

    /// Layers of other cameras show their own frames; one naming the main
    /// camera shows the main camera's.
    #[test]
    fn other_cameras() {
        let (red, _p1) = solid(160, 90, [0, 0, 255, 255]);
        let (blue, _p2) = solid(160, 90, [255, 0, 0, 255]);
        let (out, _p3) = solid(320, 180, [0, 0, 0, 255]);
        let layer = |id: &str, x: f64| crate::layout::Layer {
            content: Content::Webcam { id: id.into(), name: id.into() },
            rect: [x, 0.0, 0.5, 1.0],
            fit: crate::layout::Fit::Cover,
            mirror: false,
            ..Default::default()
        };
        let scene = crate::layout::Layout { layers: vec![layer("a", 0.0), layer("b", 0.5)], ..Default::default() };
        let mut c = Compositor::new(320, 180, PipSpec::default());
        c.set_scene(SceneSpec::Custom(scene), false);
        let b = Source::Camera("b".into());
        c.render(0.0, None, Some(&red), &[(&b, &blue)], &out);
        let is_red = |v: [u8; 3]| v[2] > 200 && v[0] < 40;
        let is_blue = |v: [u8; 3]| v[0] > 200 && v[2] < 40;
        assert!(!is_red(pixel(&out, 80, 90)) && is_blue(pixel(&out, 240, 90)), "a has no frames yet: {:?}", pixel(&out, 80, 90));
        c.set_main_camera(Some("a".into()));
        c.render(0.1, None, Some(&red), &[(&b, &blue)], &out);
        assert!(is_red(pixel(&out, 80, 90)) && is_blue(pixel(&out, 240, 90)), "{:?}", pixel(&out, 80, 90));
    }

    /// Bake an overlay with CoreImage and read pixels back.
    #[test]
    fn renders_overlay_with_border_into_the_frame() {
        let (screen, _p1) = solid(640, 360, [0, 0, 0, 255]);
        let (cam, _p2) = solid(320, 180, [0, 0, 255, 255]); // red
        let (out, _p3) = solid(640, 360, [0, 255, 0, 255]);
        let pip = PipSpec {
            size: 0.25,
            place: Place::At([0.5, 0.25]),
            shape: Shape::Square,
            roundness: 0.3,
            border: 12.0,
            border_color: [255, 255, 255],
            ..Default::default()
        };
        let mut c = Compositor::new(640, 360, pip.clone());
        c.set_scene(SceneSpec::Template(Scene::ScreenCam), false);
        c.render(0.0, Some(&screen), Some(&cam), &[], &out);
        let [x, y, w, h] = overlay_rect(&pip, 640.0, 360.0, c.cam_aspect());
        assert_eq!([x, y, w, h], [240.0, 10.0, 160.0, 160.0]);
        let px = |x: usize, y: usize| pixel(&out, x, y);
        let red = |v: [u8; 3]| v[2] > 200 && v[1] < 40 && v[0] < 40;
        let white = |v: [u8; 3]| v.iter().all(|&c| c > 200);
        let black = |v: [u8; 3]| v.iter().all(|&c| c < 30);
        assert!(red(px(320, 90)), "camera at the overlay center: {:?}", px(320, 90));
        // 12 px border at 1080p is 4 px at 360p: just inside the top edge.
        assert!(white(px(320, 11)), "border at the top edge: {:?}", px(320, 11));
        assert!(black(px(241, 11)), "rounded corner shows the screen: {:?}", px(241, 11));
        assert!(black(px(320, 200)), "screen below the overlay: {:?}", px(320, 200));
        // Moving the overlay is live.
        c.set_pip(PipSpec { place: Place::Corner(Corner::BottomLeft), ..pip });
        c.render(1.0 / 30.0, Some(&screen), Some(&cam), &[], &out);
        assert!(black(px(320, 90)), "old position is screen again");
        assert!(red(px(13 + 80, 360 - 13 - 80)), "new position: {:?}", px(93, 267));
    }
}

#[cfg(test)]
pub(crate) mod color_probe {
    use super::*;
    use objc2_core_foundation::CFString;
    use objc2_core_video::*;

    pub(crate) fn gray(v: u8, tag: Option<(&CFString, &CFString)>) -> CFRetained<CVPixelBuffer> {
        stripes(64, 36, &[v], tag)
    }

    /// A BGRA frame of vertical gray stripes, optionally color-tagged.
    pub(crate) fn stripes(w: usize, h: usize, vals: &[u8], tag: Option<(&CFString, &CFString)>) -> CFRetained<CVPixelBuffer> {
        let pool = make_pool(w, h, PIX_BGRA);
        let mut b = std::ptr::null_mut();
        unsafe {
            CVPixelBufferPool::create_pixel_buffer(None, &pool, std::ptr::NonNull::from(&mut b));
            let b: CFRetained<CVPixelBuffer> = CFRetained::from_raw(std::ptr::NonNull::new(b).unwrap());
            CVPixelBufferLockBaseAddress(&b, CVPixelBufferLockFlags(0));
            let p = CVPixelBufferGetBaseAddress(&b) as *mut u8;
            let row = CVPixelBufferGetBytesPerRow(&b);
            for y in 0..h {
                for x in 0..w {
                    let v = vals[x * vals.len() / w];
                    for c in 0..4 {
                        *p.add(y * row + x * 4 + c) = if c == 3 { 255 } else { v };
                    }
                }
            }
            CVPixelBufferUnlockBaseAddress(&b, CVPixelBufferLockFlags(0));
            if let Some((prim, tf)) = tag {
                let m = CVAttachmentMode::ShouldPropagate;
                b.set_attachment(kCVImageBufferColorPrimariesKey, prim.as_ref(), m);
                b.set_attachment(kCVImageBufferTransferFunctionKey, tf.as_ref(), m);
            }
            b
        }
    }

    fn center(b: &CVPixelBuffer) -> u8 {
        unsafe {
            CVPixelBufferLockBaseAddress(b, CVPixelBufferLockFlags(1));
            let p = CVPixelBufferGetBaseAddress(b) as *const u8;
            let v = *p.add(CVPixelBufferGetBytesPerRow(b) * 18 + 32 * 4 + 1);
            CVPixelBufferUnlockBaseAddress(b, CVPixelBufferLockFlags(1));
            v
        }
    }

    /// Camera-style frames through the compositor (run with --ignored).
    #[test]
    #[ignore]
    fn camera_tags() {
        let out = gray(0, None);
        let mut c = Compositor::new(64, 36, PipSpec::default());
        c.set_scene(SceneSpec::Template(Scene::Cam), false);
        unsafe {
            for (name, tag) in [
                ("untagged", None),
                ("709/709", Some((kCVImageBufferColorPrimaries_ITU_R_709_2, kCVImageBufferTransferFunction_ITU_R_709_2))),
                ("709/sRGB", Some((kCVImageBufferColorPrimaries_ITU_R_709_2, kCVImageBufferTransferFunction_sRGB))),
            ] {
                let vals: Vec<u8> = [32u8, 64, 128, 192].iter().map(|&v| {
                    let cam = gray(v, tag);
                    c.render(0.0, None, Some(&cam), &[], &out);
                    center(&out)
                }).collect();
                println!("{name}: in [32, 64, 128, 192] -> {vals:?}");
            }
        }
    }
}
