//! GPU (CoreImage/Metal) compositing of scenes: the screen, the webcam,
//! images and text, placed by `crate::layout`.

use std::collections::HashMap;

use objc2::rc::Retained;
use objc2_core_foundation::{CFRetained, CGAffineTransform, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGColorSpace, kCGColorSpaceSRGB};
use objc2_core_image::{CIColor, CIContext, CIFilter, CIImage, CIVector};
use objc2_core_video::CVPixelBuffer;

use crate::layout::{self, Background, Content, Env, Frame, Placed, SceneSpec, Style, TextSpec, Transition};
use crate::util::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Corner {
    BottomRight,
    BottomLeft,
    TopRight,
    TopLeft,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Shape {
    /// Rounded rectangle, camera aspect ratio (corner radius from roundness).
    Rounded,
    /// Circle (center square crop).
    Circle,
    /// Square, rounded by roundness (center square crop).
    Square,
    /// Capsule: camera aspect ratio, fully round ends.
    Pill,
    /// Superellipse square (continuous corners); roundness sets the curve.
    Squircle,
    /// Plain rectangle (same as rounded with roundness 0).
    Rect,
}

impl Shape {
    /// The shapes offered in the app.
    pub const PICKER: [Shape; 5] = [Shape::Circle, Shape::Rounded, Shape::Square, Shape::Pill, Shape::Squircle];

    pub fn label(self) -> &'static str {
        match self {
            Shape::Rounded => "Rounded",
            Shape::Circle => "Circle",
            Shape::Square => "Square",
            Shape::Pill => "Pill",
            Shape::Squircle => "Squircle",
            Shape::Rect => "Rectangle",
        }
    }

    /// Crops the camera to a centered square.
    pub fn is_square(self) -> bool {
        matches!(self, Shape::Circle | Shape::Square | Shape::Squircle)
    }

    /// The roundness slider applies.
    pub fn uses_roundness(self) -> bool {
        matches!(self, Shape::Rounded | Shape::Square | Shape::Squircle)
    }
}

pub use crate::layout::Scene;

/// Uniform scale and offset that fits (`cover` false) or fills (`cover`
/// true) a `sw`x`sh` source into a `dw`x`dh` canvas, centred.
pub fn fit(sw: f64, sh: f64, dw: f64, dh: f64, cover: bool) -> (f64, f64, f64) {
    let (a, b) = (dw / sw, dh / sh);
    let s = if cover { a.max(b) } else { a.min(b) };
    (s, (dw - sw * s) / 2.0, (dh - sh * s) / 2.0)
}

/// Aspect ratios this close (relative) count as the same: output sizes are
/// rounded to even pixels, so a frame sized from its source differs from it
/// by a fraction of a pixel.
const SAME_ASPECT: f64 = 0.01;

pub fn same_aspect(sw: f64, sh: f64, dw: f64, dh: f64) -> bool {
    let (a, b) = (sw / sh, dw / dh);
    ((a - b) / b).abs() <= SAME_ASPECT
}

/// Scale (x, y) and offset that put a `sw`x`sh` source into a `dw`x`dh`
/// canvas edge to edge when their aspect ratios match up to rounding (a
/// sub-pixel stretch instead of a hairline bar), else fitted and centred.
pub fn fill(sw: f64, sh: f64, dw: f64, dh: f64) -> (f64, f64, f64, f64) {
    if same_aspect(sw, sh, dw, dh) {
        return (dw / sw, dh / sh, 0.0, 0.0);
    }
    let (s, tx, ty) = fit(sw, sh, dw, dh, false);
    (s, s, tx, ty)
}

/// Where the overlay sits: snapped to a corner (stays there when the size
/// changes) or centered at a point given as fractions of the frame from its
/// top-left corner.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Place {
    Corner(Corner),
    At([f64; 2]),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PipSpec {
    /// Overlay width as a fraction of the output width.
    pub size: f64,
    pub place: Place,
    pub shape: Shape,
    /// Corner rounding, 0 (sharp) to 1 (as round as the shape allows).
    pub roundness: f64,
    /// Border width in pixels at 1080p (scaled with the output height).
    pub border: f64,
    pub border_color: [u8; 3],
    pub shadow: bool,
    pub mirror: bool,
}

impl Default for PipSpec {
    fn default() -> Self {
        PipSpec {
            size: 0.22,
            place: Place::Corner(Corner::BottomRight),
            shape: Shape::Rounded,
            roundness: 0.2,
            border: 0.0,
            border_color: [255, 255, 255],
            shadow: false,
            mirror: true,
        }
    }
}

impl PipSpec {
    /// The overlay's outline.
    pub fn style(&self) -> Style {
        Style { shape: self.shape, roundness: self.roundness, border: self.border, border_color: self.border_color, shadow: self.shadow }
    }
}

/// Gap between a corner-snapped overlay and the frame edges.
pub fn margin(w: f64) -> f64 {
    (w * 0.02).round()
}

/// Overlay size in output pixels for a `w`x`h` output.
pub fn overlay_size(pip: &PipSpec, w: f64, h: f64, cam_aspect: f64) -> (f64, f64) {
    let aspect = if pip.shape.is_square() { 1.0 } else { cam_aspect.max(0.1) };
    let ow = (w * pip.size).round().clamp(16.0, w);
    let oh = (ow / aspect).round().clamp(16.0, h);
    (ow, oh)
}

/// Overlay rectangle `[x, y, width, height]` in output pixels, measured
/// from the top-left corner, always inside the frame.
pub fn overlay_rect(pip: &PipSpec, w: f64, h: f64, cam_aspect: f64) -> [f64; 4] {
    let (ow, oh) = overlay_size(pip, w, h, cam_aspect);
    let (x, y) = match pip.place {
        Place::Corner(c) => {
            let m = margin(w);
            let x = if matches!(c, Corner::TopLeft | Corner::BottomLeft) { m } else { w - m - ow };
            let y = if matches!(c, Corner::TopLeft | Corner::TopRight) { m } else { h - m - oh };
            (x, y)
        }
        Place::At([cx, cy]) => ((cx * w - ow / 2.0).round(), (cy * h - oh / 2.0).round()),
    };
    [x.clamp(0.0, w - ow), y.clamp(0.0, h - oh), ow, oh]
}

/// The corner whose snapped position is within `radius` (a fraction of
/// the output width) of the overlay's current center, if any.
pub fn snap_corner(pip: &PipSpec, w: f64, h: f64, cam_aspect: f64, radius: f64) -> Option<Corner> {
    let center = |p: &PipSpec| {
        let r = overlay_rect(p, w, h, cam_aspect);
        (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0)
    };
    let c = center(pip);
    [Corner::TopLeft, Corner::TopRight, Corner::BottomLeft, Corner::BottomRight]
        .into_iter()
        .map(|k| {
            let q = center(&PipSpec { place: Place::Corner(k), ..pip.clone() });
            (k, ((q.0 - c.0).powi(2) + (q.1 - c.1).powi(2)).sqrt())
        })
        .filter(|(_, d)| *d <= radius * w)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(k, _)| k)
}

/// Superellipse exponent for a squircle: 2 (a circle) at roundness 1, 4
/// (the classic squircle) at 0.5, 10 (nearly square) at 0.
pub fn squircle_exponent(roundness: f64) -> f64 {
    2.0 + 8.0 * (1.0 - roundness.clamp(0.0, 1.0)).powi(2)
}

/// Anti-aliased coverage (0..1) of the pixel centered at (`px`, `py`) by
/// `shape` filling a `w`x`h` box.
pub fn coverage(shape: Shape, w: f64, h: f64, roundness: f64, px: f64, py: f64) -> f32 {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let (x, y) = ((px - hw).abs(), (py - hh).abs());
    // Approximate signed distance to the edge in pixels (negative inside).
    let d = match shape {
        Shape::Squircle => {
            let n = squircle_exponent(roundness);
            (((x / hw).powf(n) + (y / hh).powf(n)).powf(1.0 / n) - 1.0) * hw.min(hh)
        }
        Shape::Circle => (((x / hw).powi(2) + (y / hh).powi(2)).sqrt() - 1.0) * hw.min(hh),
        _ => {
            let r = match shape {
                Shape::Pill => hw.min(hh),
                Shape::Rect => 0.0,
                _ => roundness.clamp(0.0, 1.0) * hw.min(hh),
            };
            let (qx, qy) = (x - (hw - r), y - (hh - r));
            (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r
        }
    };
    (0.5 - d).clamp(0.0, 1.0) as f32
}

/// Alpha mask (row-major, `w`x`h`) of the shape inset by `inset` pixels.
pub fn mask(shape: Shape, w: usize, h: usize, roundness: f64, inset: f64) -> Vec<u8> {
    let (iw, ih) = (w as f64 - 2.0 * inset, h as f64 - 2.0 * inset);
    let mut out = vec![0u8; w * h];
    if iw <= 0.0 || ih <= 0.0 {
        return out;
    }
    for y in 0..h {
        for x in 0..w {
            let c = coverage(shape, iw, ih, roundness, x as f64 + 0.5 - inset, y as f64 + 0.5 - inset);
            out[y * w + x] = (c * 255.0).round() as u8;
        }
    }
    out
}

/// Soft drop shadow: `alpha` padded by `pad` pixels on every side, blurred
/// with three box passes (close to a Gaussian) and scaled by `opacity`.
pub fn shadow(alpha: &[u8], w: usize, h: usize, pad: usize, opacity: f32) -> Vec<u8> {
    let (sw, sh) = (w + 2 * pad, h + 2 * pad);
    let mut a = vec![0f32; sw * sh];
    for y in 0..h {
        for x in 0..w {
            a[(y + pad) * sw + x + pad] = alpha[y * w + x] as f32 / 255.0;
        }
    }
    let r = (pad / 3).max(1);
    let mut tmp = vec![0f32; sw * sh];
    for _ in 0..3 {
        box_blur(&a, &mut tmp, sw, sh, r, true);
        box_blur(&tmp, &mut a, sw, sh, r, false);
    }
    a.iter().map(|v| (v * opacity * 255.0).round().clamp(0.0, 255.0) as u8).collect()
}

fn box_blur(src: &[f32], dst: &mut [f32], w: usize, h: usize, r: usize, horizontal: bool) {
    let (n, lines) = if horizontal { (w, h) } else { (h, w) };
    let at = |line: usize, i: usize| if horizontal { line * w + i } else { i * w + line };
    let k = (2 * r + 1) as f32;
    for line in 0..lines {
        let mut sum: f32 = (0..=r.min(n - 1)).map(|i| src[at(line, i)]).sum();
        for i in 0..n {
            dst[at(line, i)] = sum / k;
            if i + r + 1 < n {
                sum += src[at(line, i + r + 1)];
            }
            if i >= r {
                sum -= src[at(line, i - r)];
            }
        }
    }
}

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

/// How far in from each edge the fill samples: past the largest window
/// corner radius (about 26 points at 2x).
pub fn corner_inset(w: f64, h: f64) -> f64 {
    80.0f64.min(w / 4.0).min(h / 4.0).floor()
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
}

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
                cs,
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
            }
        }
    }

    /// Change the overlay live (position, shape, size, style).
    pub fn set_pip(&mut self, pip: PipSpec) {
        self.pip = pip;
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
    fn draw(&mut self, f: &Frame, screen: Option<&Retained<CIImage>>, cam: Option<&Retained<CIImage>>, k: f64) -> Retained<CIImage> {
        let mut img = self.background_image(f.background);
        for l in &f.layers {
            if l.opacity <= 0.0 {
                continue;
            }
            let src = match &l.content {
                Content::Screen => screen.cloned(),
                Content::Camera => cam.cloned(),
                Content::Image(p) => self.image(p),
                Content::Text(t) => self.text(t, l.rect[3]),
                Content::Color(c) => {
                    let r = rect(0.0, 0.0, l.rect[2].max(1.0), l.rect[3].max(1.0));
                    Some(unsafe { CIImage::imageWithColor(&ci_color(*c, 1.0)).imageByCroppingToRect(r) })
                }
            };
            if let Some(src) = src {
                img = self.place(l, &src, img, k);
            }
        }
        img
    }

    /// Render the scene at frame time `t` (seconds; drives transitions).
    pub fn render(&mut self, t: f64, screen: Option<&CVPixelBuffer>, cam: Option<&CVPixelBuffer>, out: &CVPixelBuffer) {
        self.now = t;
        let canvas = rect(0.0, 0.0, self.w, self.h);
        let screen = screen.map(|s| unsafe { CIImage::imageWithCVPixelBuffer(s) });
        let cam = cam.map(|c| unsafe { CIImage::imageWithCVPixelBuffer(c) });
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
        if progress.is_some_and(|p| p >= 1.0 || p < 0.0) {
            self.leaving = None;
        }
        let img = match (self.leaving.clone(), progress) {
            (Some((old, _)), Some(p)) => {
                let prev = old.resolve(&env);
                match self.transition {
                    Transition::Fade => {
                        let a = self.draw(&prev, screen.as_ref(), cam.as_ref(), 1.0);
                        let b = self.draw(&cur, screen.as_ref(), cam.as_ref(), 1.0);
                        unsafe {
                            let params = dict(vec![
                                (&*ns("inputTargetImage"), obj(&*b)),
                                (&*ns("inputTime"), num_f(layout::smoothstep(p))),
                            ]);
                            a.imageByApplyingFilter_withInputParameters(&ns("CIDissolveTransition"), &params)
                        }
                    }
                    _ => self.draw(&layout::blend(&prev, &cur, p), screen.as_ref(), cam.as_ref(), 0.5),
                }
            }
            _ => self.draw(&cur, screen.as_ref(), cam.as_ref(), 1.0),
        };
        unsafe { self.ctx.render_toCVPixelBuffer_bounds_colorSpace(&img.imageByCroppingToRect(canvas), out, canvas, Some(&self.cs)) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_letterboxes_and_cover_crops() {
        // 4:3 camera into a 16:9 canvas.
        let (s, tx, ty) = fit(1280.0, 960.0, 1920.0, 1080.0, false);
        assert_eq!((s, tx, ty), (1.125, 240.0, 0.0));
        let (s, tx, ty) = fit(1280.0, 960.0, 1920.0, 1080.0, true);
        assert_eq!((s, tx, ty), (1.5, 0.0, -180.0));
    }

    #[test]
    fn fill_is_edge_to_edge_for_the_same_aspect() {
        // A 4112x2658 display into its 1670x1080 output: a sub-pixel stretch,
        // no offset, so no hairline bar on any edge.
        let (sx, sy, tx, ty) = fill(4112.0, 2658.0, 1670.0, 1080.0);
        assert_eq!((tx, ty), (0.0, 0.0));
        assert!((4112.0 * sx - 1670.0).abs() < 1e-9 && (2658.0 * sy - 1080.0).abs() < 1e-9);
        // A different shape (a window resized mid-recording) is fitted.
        let (sx, sy, tx, ty) = fill(1280.0, 960.0, 1920.0, 1080.0);
        assert_eq!((sx, sy, tx, ty), (1.125, 1.125, 240.0, 0.0));
        assert_eq!(corner_inset(4086.0, 2464.0), 80.0);
        assert_eq!(corner_inset(200.0, 100.0), 25.0);
    }

    fn spec(shape: Shape, place: Place) -> PipSpec {
        PipSpec { shape, place, ..Default::default() }
    }

    #[test]
    fn corners_and_free_positions() {
        let a = 16.0 / 9.0;
        let p = spec(Shape::Rounded, Place::Corner(Corner::BottomRight));
        // 22% of 1920 = 422 wide, 237 tall, 38 px margin.
        assert_eq!(overlay_rect(&p, 1920.0, 1080.0, a), [1460.0, 805.0, 422.0, 237.0]);
        let p = spec(Shape::Circle, Place::Corner(Corner::TopLeft));
        assert_eq!(overlay_rect(&p, 1920.0, 1080.0, a), [38.0, 38.0, 422.0, 422.0]);
        let p = spec(Shape::Rounded, Place::At([0.5, 0.5]));
        assert_eq!(overlay_rect(&p, 1920.0, 1080.0, a), [749.0, 422.0, 422.0, 237.0]);
        // Dragged past the edge: kept inside the frame.
        let p = spec(Shape::Square, Place::At([1.2, -0.3]));
        assert_eq!(overlay_rect(&p, 1920.0, 1080.0, a), [1498.0, 0.0, 422.0, 422.0]);
    }

    #[test]
    fn corner_placement_survives_resize() {
        let mut p = spec(Shape::Rounded, Place::Corner(Corner::BottomRight));
        p.size = 0.4;
        let r = overlay_rect(&p, 1920.0, 1080.0, 16.0 / 9.0);
        assert_eq!((r[0] + r[2], r[1] + r[3]), (1920.0 - 38.0, 1080.0 - 38.0));
    }

    #[test]
    fn snapping() {
        let a = 16.0 / 9.0;
        let near = spec(Shape::Rounded, Place::At([0.85, 0.82]));
        assert_eq!(snap_corner(&near, 1920.0, 1080.0, a, 0.06), Some(Corner::BottomRight));
        let middle = spec(Shape::Rounded, Place::At([0.5, 0.5]));
        assert_eq!(snap_corner(&middle, 1920.0, 1080.0, a, 0.06), None);
        let top_left = spec(Shape::Circle, Place::At([0.1, 0.18]));
        assert_eq!(snap_corner(&top_left, 1920.0, 1080.0, a, 0.06), Some(Corner::TopLeft));
    }

    #[test]
    fn shape_coverage() {
        let cov = |s, r, x, y| coverage(s, 100.0, 100.0, r, x, y);
        for s in Shape::PICKER {
            assert_eq!(cov(s, 0.5, 50.0, 50.0), 1.0, "{s:?} center");
            assert_eq!(cov(s, 0.5, 50.0, 0.5), 1.0, "{s:?} top edge middle");
        }
        // The corner pixel: covered only by sharp shapes.
        assert_eq!(cov(Shape::Rect, 0.0, 0.5, 0.5), 1.0);
        assert_eq!(cov(Shape::Rounded, 0.0, 0.5, 0.5), 1.0, "roundness 0 is a rectangle");
        assert_eq!(cov(Shape::Rounded, 0.5, 0.5, 0.5), 0.0);
        assert_eq!(cov(Shape::Circle, 0.0, 0.5, 0.5), 0.0);
        assert_eq!(cov(Shape::Squircle, 0.5, 0.5, 0.5), 0.0);
        // The squircle is fuller than a circle at the diagonal (0.8, 0.8).
        assert_eq!(cov(Shape::Circle, 0.5, 90.0, 90.0), 0.0);
        assert_eq!(cov(Shape::Squircle, 0.5, 90.0, 90.0), 1.0);
        // Roundness 1 on a square equals a circle.
        assert_eq!(cov(Shape::Square, 1.0, 90.0, 90.0), 0.0);
        // Anti-aliased edge.
        let edge = cov(Shape::Circle, 0.5, 100.0, 50.0);
        assert!(edge > 0.0 && edge < 1.0, "edge {edge}");
    }

    #[test]
    fn pill_has_round_ends() {
        let cov = |x, y| coverage(Shape::Pill, 200.0, 100.0, 0.0, x, y);
        assert_eq!(cov(5.0, 50.0), 1.0);
        assert_eq!(cov(5.0, 5.0), 0.0);
        assert_eq!(cov(100.0, 1.0), 1.0);
        assert_eq!(cov(195.0, 95.0), 0.0);
    }

    #[test]
    fn border_inset_mask() {
        let outer = mask(Shape::Square, 40, 40, 0.0, 0.0);
        let inner = mask(Shape::Square, 40, 40, 0.0, 4.0);
        assert_eq!((outer[2 * 40 + 20], inner[2 * 40 + 20]), (255, 0), "border band");
        assert_eq!((outer[20 * 40 + 20], inner[20 * 40 + 20]), (255, 255));
    }

    #[test]
    fn shadow_spreads_beyond_the_shape() {
        let a = mask(Shape::Square, 20, 20, 0.0, 0.0);
        let s = shadow(&a, 20, 20, 9, 0.5);
        let w = 38;
        assert!(s[19 * w + 19] > 100, "under the shape");
        assert!(s[19 * w + 4] > 0, "outside the shape");
        assert_eq!(s[0], 0, "far corner");
        assert!(s.iter().all(|&v| v <= 128));
    }

    /// Bake an overlay with CoreImage and read pixels back.
    #[test]
    fn renders_overlay_with_border_into_the_frame() {
        use objc2_core_video::*;
        let solid = |w: usize, h: usize, bgra: [u8; 4]| {
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
        };
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
        c.render(0.0, Some(&screen), Some(&cam), &out);
        let [x, y, w, h] = overlay_rect(&pip, 640.0, 360.0, c.cam_aspect());
        assert_eq!([x, y, w, h], [240.0, 10.0, 160.0, 160.0]);
        let px = |px: usize, py: usize| unsafe {
            CVPixelBufferLockBaseAddress(&out, CVPixelBufferLockFlags::ReadOnly);
            let base = CVPixelBufferGetBaseAddress(&out) as *const u8;
            let p = base.add(py * CVPixelBufferGetBytesPerRow(&out) + px * 4);
            let v = [*p, *p.add(1), *p.add(2)];
            CVPixelBufferUnlockBaseAddress(&out, CVPixelBufferLockFlags::ReadOnly);
            v
        };
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
        c.render(1.0 / 30.0, Some(&screen), Some(&cam), &out);
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
                    c.render(0.0, None, Some(&cam), &out);
                    center(&out)
                }).collect();
                println!("{name}: in [32, 64, 128, 192] -> {vals:?}");
            }
        }
    }
}
