//! Scene layouts: where the screen, the webcam and other layers sit in the
//! output frame. Built-in templates (`Scene`) and custom scenes (`Layout`)
//! both resolve to a `Frame`: a background and a list of layers placed in
//! output pixels, which the compositor draws (`compose::Compositor`).
//! Everything here is plain geometry, so it is unit-tested without a GPU.

use serde::{Deserialize, Serialize};

use crate::compose::{PipSpec, Shape, overlay_rect, same_aspect};

/// Built-in scene templates. Switchable live while the webcam runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, clap::ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scene {
    /// The screen only.
    Screen,
    /// The screen with the webcam picture-in-picture.
    #[default]
    ScreenCam,
    /// The webcam full frame.
    Cam,
    /// The screen inset on a background, with the webcam over a corner.
    Presenter,
    /// The screen and the webcam next to each other.
    SideBySide,
    /// The webcam and the screen next to each other, the webcam first.
    CamScreen,
    /// Two halves: the screen and the webcam.
    Split,
    /// The webcam full frame with the screen in a corner.
    Reaction,
    /// For Shorts and Reels: the screen on top, the webcam below.
    Vertical,
}

impl Scene {
    pub const ALL: [Scene; 9] = [
        Scene::Screen,
        Scene::ScreenCam,
        Scene::Cam,
        Scene::Presenter,
        Scene::SideBySide,
        Scene::CamScreen,
        Scene::Split,
        Scene::Reaction,
        Scene::Vertical,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Scene::Screen => "Screen",
            Scene::ScreenCam => "Screen + Camera",
            Scene::Cam => "Camera",
            Scene::Presenter => "Presenter",
            Scene::SideBySide => "Side by Side",
            Scene::CamScreen => "Camera + Screen",
            Scene::Split => "Split",
            Scene::Reaction => "Reaction",
            Scene::Vertical => "Vertical",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Scene::Screen => "Just the screen",
            Scene::ScreenCam => "The screen with your camera in a corner (drag it anywhere)",
            Scene::Cam => "Your camera, full frame",
            Scene::Presenter => "The screen on a background with rounded corners, your camera over it",
            Scene::SideBySide => "The screen and your camera next to each other",
            Scene::CamScreen => "Your camera first, then the screen",
            Scene::Split => "Half screen, half camera",
            Scene::Reaction => "Your camera full frame, the screen in a corner",
            Scene::Vertical => "For Shorts and Reels: the screen on top, your camera below (9:16)",
        }
    }

    pub fn uses_cam(self) -> bool {
        self != Scene::Screen
    }

    pub fn uses_screen(self) -> bool {
        self != Scene::Cam
    }

    /// The webcam is a draggable overlay styled by the overlay settings.
    pub fn has_pip(self) -> bool {
        matches!(self, Scene::ScreenCam | Scene::Presenter)
    }

    /// The canvas this template is designed for, when it isn't the
    /// source's shape.
    pub fn suggested_canvas(self) -> Option<Canvas> {
        match self {
            Scene::Vertical => Some(Canvas::Portrait),
            _ => None,
        }
    }
}

/// Shape of the output frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, clap::ValueEnum, Serialize, Deserialize)]
pub enum Canvas {
    /// The screen's (or window's) own shape: no bars, nothing cropped.
    #[default]
    #[serde(rename = "source")]
    #[value(name = "source")]
    Source,
    /// 16:9 landscape (YouTube).
    #[serde(rename = "16:9")]
    #[value(name = "16:9")]
    Landscape,
    /// 9:16 portrait (Shorts, Reels, TikTok).
    #[serde(rename = "9:16")]
    #[value(name = "9:16")]
    Portrait,
    /// 1:1 square.
    #[serde(rename = "1:1")]
    #[value(name = "1:1")]
    Square,
    /// 4:5 portrait (Instagram feed).
    #[serde(rename = "4:5")]
    #[value(name = "4:5")]
    Tall,
}

impl Canvas {
    pub const ALL: [Canvas; 5] = [Canvas::Source, Canvas::Landscape, Canvas::Portrait, Canvas::Square, Canvas::Tall];

    pub fn label(self) -> &'static str {
        match self {
            Canvas::Source => "Source shape",
            Canvas::Landscape => "16:9",
            Canvas::Portrait => "9:16",
            Canvas::Square => "1:1",
            Canvas::Tall => "4:5",
        }
    }

    /// Width / height, None for the source's own shape.
    pub fn aspect(self) -> Option<f64> {
        match self {
            Canvas::Source => None,
            Canvas::Landscape => Some(16.0 / 9.0),
            Canvas::Portrait => Some(9.0 / 16.0),
            Canvas::Square => Some(1.0),
            Canvas::Tall => Some(4.0 / 5.0),
        }
    }
}

/// How a scene change is animated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, clap::ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Transition {
    /// Instant.
    Cut,
    /// Cross-dissolve between the two scenes.
    Fade,
    /// Layers glide to their new places; others fade in or out.
    #[default]
    Move,
}

impl Transition {
    pub const ALL: [Transition; 3] = [Transition::Cut, Transition::Fade, Transition::Move];

    pub fn label(self) -> &'static str {
        match self {
            Transition::Cut => "Cut",
            Transition::Fade => "Fade",
            Transition::Move => "Move",
        }
    }
}

/// How a layer's picture fills its box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, clap::ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Fit {
    /// Fill the box, cropping the picture's edges.
    Cover,
    /// The whole picture, centered in the box.
    #[default]
    Contain,
}

/// A layer's outline.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    pub shape: Shape,
    /// Corner rounding, 0 (sharp) to 1 (as round as the shape allows).
    pub roundness: f64,
    /// Border width in pixels at 1080p (scaled with the output height).
    pub border: f64,
    pub border_color: [u8; 3],
    pub shadow: bool,
}

impl Default for Style {
    fn default() -> Self {
        Style { shape: Shape::Rect, roundness: 0.0, border: 0.0, border_color: [255, 255, 255], shadow: false }
    }
}

impl Style {
    /// A plain rectangle: drawn without a mask.
    pub fn is_plain(&self) -> bool {
        let sharp = match self.shape {
            Shape::Rect => true,
            Shape::Rounded | Shape::Square => self.roundness <= 0.0,
            _ => false,
        };
        sharp && self.border <= 0.0 && !self.shadow
    }

    /// Rounded corners of `radius` pixels on a `w`x`h` box, with a shadow.
    pub fn rounded(radius: f64, w: f64, h: f64) -> Style {
        let half = (w.min(h) / 2.0).max(1.0);
        Style { shape: Shape::Rounded, roundness: (radius / half).clamp(0.0, 1.0), shadow: true, ..Default::default() }
    }
}

/// Text drawn in a layer's box.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextSpec {
    pub text: String,
    pub color: [u8; 3],
    /// Box fill behind the text (RGBA), if any.
    pub background: Option<[u8; 4]>,
}

impl Default for TextSpec {
    fn default() -> Self {
        TextSpec { text: "Your Name".into(), color: [255, 255, 255], background: Some([20, 20, 28, 200]) }
    }
}

/// What a layer shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Content {
    Screen,
    Camera,
    /// An image file (PNG, JPEG, HEIC...): a logo or a frame.
    Image(String),
    Text(TextSpec),
    /// A solid color.
    Color([u8; 3]),
}

impl Content {
    pub fn label(&self) -> String {
        match self {
            Content::Screen => "Screen".into(),
            Content::Camera => "Camera".into(),
            Content::Image(p) => {
                let name = std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned());
                format!("Image {}", name.unwrap_or_default())
            }
            Content::Text(t) => format!("Text “{}”", t.text),
            Content::Color(_) => "Color".into(),
        }
    }
}

/// A layer placed in output pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub content: Content,
    /// `[x, y, width, height]` in output pixels from the top-left corner.
    pub rect: [f64; 4],
    pub fit: Fit,
    pub style: Style,
    /// Flip horizontally (a webcam, like a mirror).
    pub mirror: bool,
    pub opacity: f64,
}

/// Vertical gradient (a solid color when both ends are equal).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Background {
    pub top: [u8; 3],
    pub bottom: [u8; 3],
}

impl Default for Background {
    fn default() -> Self {
        Background { top: [0x1d, 0x1f, 0x2b], bottom: [0x3b, 0x2f, 0x5e] }
    }
}

/// A resolved scene: what the compositor draws.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub background: Background,
    pub layers: Vec<Placed>,
}

impl Frame {
    pub fn uses_screen(&self) -> bool {
        self.layers.iter().any(|l| l.content == Content::Screen)
    }
}

/// What a layout is resolved against.
#[derive(Clone, Debug, PartialEq)]
pub struct Env {
    /// Output size in pixels.
    pub w: f64,
    pub h: f64,
    /// Width / height of the screen capture and of the webcam.
    pub screen_aspect: f64,
    pub cam_aspect: f64,
    pub pip: PipSpec,
    pub background: Background,
}

/// The largest `aspect`-shaped rectangle centered in `b`.
pub fn contain(aspect: f64, b: [f64; 4]) -> [f64; 4] {
    let [x, y, w, h] = b;
    let aspect = aspect.max(0.01);
    let (cw, ch) = if w / h > aspect { (h * aspect, h) } else { (w, w / aspect) };
    [(x + (w - cw) / 2.0).round(), (y + (h - ch) / 2.0).round(), cw.round(), ch.round()]
}

/// A layer custom scenes store: its box as fractions of the frame, so the
/// scene works at any output size.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layer {
    pub content: Content,
    /// `[x, y, width, height]` as fractions of the frame from the top-left.
    pub rect: [f64; 4],
    pub fit: Fit,
    pub style: Style,
    /// Mirror the webcam (camera layers; follows the overlay setting).
    pub mirror: bool,
    pub visible: bool,
}

impl Default for Layer {
    fn default() -> Self {
        Layer {
            content: Content::Screen,
            rect: [0.0, 0.0, 1.0, 1.0],
            fit: Fit::Contain,
            style: Style::default(),
            mirror: true,
            visible: true,
        }
    }
}

/// A custom scene: layers bottom to top, over a background.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub name: String,
    pub layers: Vec<Layer>,
    /// None = the background from the settings.
    pub background: Option<Background>,
}

impl Default for Layout {
    fn default() -> Self {
        Layout { name: "Custom".into(), layers: vec![], background: None }
    }
}

/// A scene to show: a template or a custom layout.
#[derive(Clone, Debug, PartialEq)]
pub enum SceneSpec {
    Template(Scene),
    Custom(Layout),
}

impl Default for SceneSpec {
    fn default() -> Self {
        SceneSpec::Template(Scene::default())
    }
}

impl SceneSpec {
    pub fn uses_cam(&self) -> bool {
        match self {
            SceneSpec::Template(s) => s.uses_cam(),
            SceneSpec::Custom(l) => l.layers.iter().any(|x| x.visible && x.content == Content::Camera),
        }
    }

    pub fn resolve(&self, env: &Env) -> Frame {
        match self {
            SceneSpec::Template(s) => s.resolve(env),
            SceneSpec::Custom(l) => l.resolve(env),
        }
    }
}

impl Layout {
    pub fn resolve(&self, env: &Env) -> Frame {
        let mut layers = vec![];
        for l in self.layers.iter().filter(|l| l.visible) {
            let [fx, fy, fw, fh] = l.rect;
            let rect = [(fx * env.w).round(), (fy * env.h).round(), (fw * env.w).round().max(2.0), (fh * env.h).round().max(2.0)];
            let mirror = l.content == Content::Camera && l.mirror;
            // A text box's fill is a color layer under the text.
            if let Content::Text(t) = &l.content
                && let Some([r, g, b, a]) = t.background
            {
                layers.push(Placed {
                    content: Content::Color([r, g, b]),
                    rect,
                    fit: Fit::Cover,
                    style: l.style,
                    mirror: false,
                    opacity: a as f64 / 255.0,
                });
                let pad = (rect[3] * 0.18).round();
                let inner = [rect[0] + pad, rect[1] + pad, (rect[2] - 2.0 * pad).max(2.0), (rect[3] - 2.0 * pad).max(2.0)];
                layers.push(Placed { content: l.content.clone(), rect: inner, fit: Fit::Contain, style: Style::default(), mirror: false, opacity: 1.0 });
                continue;
            }
            layers.push(Placed { content: l.content.clone(), rect, fit: l.fit, style: l.style, mirror, opacity: 1.0 });
        }
        Frame { background: self.background.unwrap_or(env.background), layers }
    }

    /// This template as an editable custom scene.
    pub fn from_template(scene: Scene, env: &Env) -> Layout {
        let f = scene.resolve(env);
        let layers = f
            .layers
            .into_iter()
            .map(|p| {
                let [x, y, w, h] = p.rect;
                Layer {
                    mirror: p.content != Content::Camera || p.mirror,
                    content: p.content,
                    rect: [x / env.w, y / env.h, w / env.w, h / env.h],
                    fit: p.fit,
                    style: p.style,
                    visible: true,
                }
            })
            .collect();
        Layout { name: format!("{} (custom)", scene.label()), layers, background: None }
    }
}

impl Scene {
    pub fn resolve(self, env: &Env) -> Frame {
        let (w, h) = (env.w, env.h);
        let min = w.min(h);
        // Gaps between the pieces and the frame edges, and the corner radius
        // of inset pieces.
        let m = (min * 0.035).round();
        let radius = (min * 0.016).round();
        let pip = &env.pip;
        let cam_style = |r: [f64; 4]| Style {
            border: pip.border,
            border_color: pip.border_color,
            ..Style::rounded(radius, r[2], r[3])
        };
        let screen = |r: [f64; 4], style: Style| Placed { content: Content::Screen, rect: r, fit: Fit::Contain, style, mirror: false, opacity: 1.0 };
        let cam = |r: [f64; 4], fit: Fit, style: Style| Placed { content: Content::Camera, rect: r, fit, style, mirror: pip.mirror, opacity: 1.0 };
        let overlay = || {
            let r = overlay_rect(pip, w, h, env.cam_aspect);
            cam(r, Fit::Cover, pip.style())
        };
        let full = [0.0, 0.0, w, h];
        let sa = env.screen_aspect.max(0.01);
        let layers = match self {
            Scene::Screen => vec![screen(full, Style::default())],
            Scene::Cam => vec![cam(full, Fit::Cover, Style::default())],
            Scene::ScreenCam => vec![screen(full, Style::default()), overlay()],
            Scene::Presenter => {
                let inset = (m * 1.5).round();
                let r = contain(sa, [inset, inset, w - 2.0 * inset, h - 2.0 * inset]);
                vec![screen(r, Style::rounded(radius, r[2], r[3])), overlay()]
            }
            Scene::SideBySide | Scene::CamScreen | Scene::Vertical => {
                let (s, c) = side_by_side(w, h, m, sa, self == Scene::Vertical);
                let pieces = vec![screen(s, Style::rounded(radius, s[2], s[3])), cam(c, Fit::Cover, cam_style(c))];
                if self == Scene::CamScreen { swap_places(pieces) } else { pieces }
            }
            Scene::Split => {
                let (s, c) = if w >= h {
                    ([0.0, 0.0, (w / 2.0).round(), h], [(w / 2.0).round(), 0.0, w - (w / 2.0).round(), h])
                } else {
                    ([0.0, 0.0, w, (h / 2.0).round()], [0.0, (h / 2.0).round(), w, h - (h / 2.0).round()])
                };
                vec![screen(contain(sa, s), Style::default()), cam(c, Fit::Cover, Style::default())]
            }
            Scene::Reaction => {
                let bw = (w * if w >= h { 0.42 } else { 0.8 }).round();
                let r = contain(sa, [w - m - bw, m, bw, h * 0.5]);
                let r = [w - m - r[2], m, r[2], r[3]];
                let style = Style { border: pip.border, border_color: pip.border_color, ..Style::rounded(radius, r[2], r[3]) };
                vec![cam(full, Fit::Cover, Style::default()), screen(r, style)]
            }
        };
        Frame { background: env.background, layers }
    }
}

/// The two boxes of a two-piece layout: the screen (its own shape) and the
/// webcam, side by side on a landscape frame or stacked on a portrait one,
/// centered as a group. `vertical` puts the screen edge to edge on top
/// (the Shorts look) on a portrait frame.
fn side_by_side(w: f64, h: f64, m: f64, sa: f64, vertical: bool) -> ([f64; 4], [f64; 4]) {
    if w >= h {
        // The webcam box is 3:4 next to the screen at the same height.
        let cam_aspect = 0.75;
        let hs = ((w - 3.0 * m) / (sa + cam_aspect)).min(h - 2.0 * m).floor();
        let ws = (hs * sa).round();
        let wc = (w - 3.0 * m - ws).round().min(hs * 1.2).max(hs * 0.5).round();
        let x0 = ((w - ws - wc - m) / 2.0).round();
        let y = ((h - hs) / 2.0).round();
        ([x0, y, ws, hs], [x0 + ws + m, y, wc, hs])
    } else if vertical {
        let hs = (w / sa).round().min(h * 0.55);
        let s = contain(sa, [0.0, 0.0, w, hs]);
        (s, [0.0, s[1] + s[3], w, h - s[1] - s[3]])
    } else {
        let ws = w - 2.0 * m;
        let hs = (ws / sa).round().min((h - 3.0 * m) * 0.6);
        let s = contain(sa, [m, m, ws, hs]);
        let top = s[1] + s[3] + m;
        (s, [m, top, ws, h - top - m])
    }
}

/// The same two pieces in each other's place (the webcam first).
fn swap_places(mut pieces: Vec<Placed>) -> Vec<Placed> {
    let (a, b) = (pieces[0].rect, pieces[1].rect);
    // Keep the group's outer edges: the first box starts where the first
    // piece started.
    let landscape = (a[1] - b[1]).abs() < 1.0;
    if landscape {
        let gap = b[0] - (a[0] + a[2]);
        pieces[1].rect = [a[0], b[1], b[2], b[3]];
        pieces[0].rect = [a[0] + b[2] + gap, a[1], a[2], a[3]];
    } else {
        let gap = b[1] - (a[1] + a[3]);
        pieces[1].rect = [b[0], a[1], b[2], b[3]];
        pieces[0].rect = [a[0], a[1] + b[3] + gap, a[2], a[3]];
    }
    pieces
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn lerp_color(a: [u8; 3], b: [u8; 3], t: f64) -> [u8; 3] {
    [0, 1, 2].map(|i| lerp(a[i] as f64, b[i] as f64, t).round() as u8)
}

/// Ease in and out.
pub fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The in-between frame `t` (0..1) of a Move transition from `a` to `b`:
/// layers showing the same thing glide between their boxes and styles,
/// others fade out (under) or in (in `b`'s order).
pub fn blend(a: &Frame, b: &Frame, t: f64) -> Frame {
    let t = smoothstep(t);
    let mut used = vec![false; a.layers.len()];
    let mut matched = vec![];
    for l in &b.layers {
        let i = a.layers.iter().enumerate().position(|(i, x)| !used[i] && x.content == l.content);
        if let Some(i) = i {
            used[i] = true;
        }
        matched.push(i);
    }
    let mut layers: Vec<Placed> = a
        .layers
        .iter()
        .zip(&used)
        .filter(|(_, u)| !**u)
        .map(|(l, _)| Placed { opacity: l.opacity * (1.0 - t), ..l.clone() })
        .collect();
    for (l, i) in b.layers.iter().zip(matched) {
        match i {
            Some(i) => {
                let p = &a.layers[i];
                let rect = [0, 1, 2, 3].map(|k| lerp(p.rect[k], l.rect[k], t));
                let half = t < 0.5;
                let style = Style {
                    shape: if half { p.style.shape } else { l.style.shape },
                    roundness: lerp(p.style.roundness, l.style.roundness, t),
                    border: lerp(p.style.border, l.style.border, t),
                    border_color: lerp_color(p.style.border_color, l.style.border_color, t),
                    shadow: if half { p.style.shadow } else { l.style.shadow },
                };
                layers.push(Placed {
                    rect,
                    style,
                    fit: if half { p.fit } else { l.fit },
                    mirror: if half { p.mirror } else { l.mirror },
                    opacity: lerp(p.opacity, l.opacity, t),
                    content: l.content.clone(),
                });
            }
            None => layers.push(Placed { opacity: l.opacity * t, ..l.clone() }),
        }
    }
    let background = Background {
        top: lerp_color(a.background.top, b.background.top, t),
        bottom: lerp_color(a.background.bottom, b.background.bottom, t),
    };
    Frame { background, layers }
}

/// Where a `sw`x`sh` source lands inside layer box `r` with `fit`: the
/// drawn rectangle (output pixels, top-left origin) and the scale (x, y).
/// A source with the box's shape (up to rounding) fills it exactly.
pub fn place(sw: f64, sh: f64, r: [f64; 4], fit: Fit) -> ([f64; 4], f64, f64) {
    let [x, y, w, h] = r;
    if same_aspect(sw, sh, w, h) {
        return (r, w / sw, h / sh);
    }
    match fit {
        Fit::Cover => {
            let s = (w / sw).max(h / sh);
            (r, s, s)
        }
        Fit::Contain => {
            let s = (w / sw).min(h / sh);
            let (dw, dh) = ((sw * s).round(), (sh * s).round());
            ([(x + (w - dw) / 2.0).round(), (y + (h - dh) / 2.0).round(), dw, dh], s, s)
        }
    }
}

/// Which part of a layer box a drag holds.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Grip {
    Body,
    Corner(bool, bool),
}

/// Move a box (fractions) by a drag of `d` fractions with `grip`, keeping
/// it at least `min` across, and snap its edges and center to the frame's
/// edges and center when within `snap`.
pub fn drag_box(r: [f64; 4], grip: Grip, d: [f64; 2], min: f64, snap: f64) -> [f64; 4] {
    let [mut x, mut y, mut w, mut h] = r;
    let snapped = |v: f64, targets: &[f64]| targets.iter().copied().find(|t| (v - t).abs() <= snap);
    match grip {
        Grip::Body => {
            x += d[0];
            y += d[1];
            // The nearest of the left edge, center and right edge to a target.
            for (pos, size) in [(&mut x, w), (&mut y, h)] {
                let candidates = [(*pos, 0.0), (*pos + size / 2.0, size / 2.0), (*pos + size, size)];
                if let Some((v, off)) = candidates.iter().find_map(|(v, off)| snapped(*v, &[0.0, 0.5, 1.0]).map(|t| (t, *off))) {
                    *pos = v - off;
                }
            }
        }
        Grip::Corner(right, bottom) => {
            let (mut x0, mut y0, mut x1, mut y1) = (x, y, x + w, y + h);
            if right { x1 += d[0] } else { x0 += d[0] }
            if bottom { y1 += d[1] } else { y0 += d[1] }
            for v in [&mut x0, &mut y0, &mut x1, &mut y1] {
                if let Some(t) = snapped(*v, &[0.0, 0.5, 1.0]) {
                    *v = t;
                }
            }
            if right { x1 = x1.max(x0 + min) } else { x0 = x0.min(x1 - min) }
            if bottom { y1 = y1.max(y0 + min) } else { y0 = y0.min(y1 - min) }
            (x, y, w, h) = (x0, y0, x1 - x0, y1 - y0);
        }
    }
    [x, y, w, h]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(w: f64, h: f64) -> Env {
        Env { w, h, screen_aspect: 16.0 / 10.0, cam_aspect: 16.0 / 9.0, pip: PipSpec::default(), background: Background::default() }
    }

    fn inside(r: [f64; 4], w: f64, h: f64) -> bool {
        r[0] >= 0.0 && r[1] >= 0.0 && r[0] + r[2] <= w + 0.5 && r[1] + r[3] <= h + 0.5 && r[2] > 0.0 && r[3] > 0.0
    }

    fn overlap(a: [f64; 4], b: [f64; 4]) -> bool {
        a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
    }

    #[test]
    fn every_template_fits_every_canvas() {
        for (w, h) in [(1920.0, 1080.0), (1728.0, 1080.0), (1080.0, 1920.0), (1080.0, 1080.0), (1080.0, 1350.0)] {
            for s in Scene::ALL {
                let f = s.resolve(&env(w, h));
                assert!(!f.layers.is_empty(), "{s:?}");
                for l in &f.layers {
                    assert!(inside(l.rect, w, h), "{s:?} {w}x{h}: {:?} {:?}", l.content, l.rect);
                }
                assert_eq!(f.uses_screen(), s.uses_screen(), "{s:?}");
                assert_eq!(f.layers.iter().any(|l| l.content == Content::Camera), s.uses_cam(), "{s:?}");
            }
        }
    }

    #[test]
    fn two_piece_layouts_do_not_overlap() {
        for (w, h) in [(1920.0, 1080.0), (1080.0, 1920.0), (1080.0, 1080.0)] {
            for s in [Scene::SideBySide, Scene::CamScreen, Scene::Split, Scene::Vertical] {
                let f = s.resolve(&env(w, h));
                assert!(!overlap(f.layers[0].rect, f.layers[1].rect), "{s:?} {w}x{h}: {:?}", f.layers);
            }
        }
    }

    #[test]
    fn side_by_side_keeps_the_screen_shape_and_order() {
        let e = env(1920.0, 1080.0);
        let f = Scene::SideBySide.resolve(&e);
        let s = f.layers.iter().find(|l| l.content == Content::Screen).unwrap().rect;
        let c = f.layers.iter().find(|l| l.content == Content::Camera).unwrap().rect;
        assert!((s[2] / s[3] - 1.6).abs() < 0.01, "screen keeps 16:10: {s:?}");
        assert!(s[0] < c[0], "screen first");
        assert_eq!(s[3], c[3], "same height");
        let g = Scene::CamScreen.resolve(&e);
        let s2 = g.layers.iter().find(|l| l.content == Content::Screen).unwrap().rect;
        let c2 = g.layers.iter().find(|l| l.content == Content::Camera).unwrap().rect;
        assert!(c2[0] < s2[0], "camera first");
        assert_eq!(c2[0], s[0], "same outer edge");
        assert_eq!(s2[0] + s2[2], c[0] + c[2]);
    }

    #[test]
    fn screen_cam_is_the_overlay() {
        let e = env(1920.0, 1080.0);
        let f = Scene::ScreenCam.resolve(&e);
        assert_eq!(f.layers[1].rect, overlay_rect(&e.pip, 1920.0, 1080.0, 16.0 / 9.0));
        assert_eq!(f.layers[0].rect, [0.0, 0.0, 1920.0, 1080.0]);
    }

    #[test]
    fn vertical_puts_the_screen_on_top_edge_to_edge() {
        let f = Scene::Vertical.resolve(&env(1080.0, 1920.0));
        let s = f.layers[0].rect;
        assert_eq!((s[0], s[1], s[2]), (0.0, 0.0, 1080.0));
        let c = f.layers[1].rect;
        assert_eq!(c[1], s[3]);
        assert_eq!(c[1] + c[3], 1920.0);
    }

    #[test]
    fn contain_and_place() {
        assert_eq!(contain(2.0, [0.0, 0.0, 100.0, 100.0]), [0.0, 25.0, 100.0, 50.0]);
        // Same shape up to rounding: stretched edge to edge.
        let (r, sx, sy) = place(4112.0, 2658.0, [10.0, 10.0, 1670.0, 1080.0], Fit::Contain);
        assert_eq!(r, [10.0, 10.0, 1670.0, 1080.0]);
        assert!((4112.0 * sx - 1670.0).abs() < 1e-9 && (2658.0 * sy - 1080.0).abs() < 1e-9);
        // Cover crops: the scale fills the box.
        let (r, s, _) = place(1600.0, 900.0, [0.0, 0.0, 900.0, 900.0], Fit::Cover);
        assert_eq!((r, s), ([0.0, 0.0, 900.0, 900.0], 1.0));
        let (r, s, _) = place(1600.0, 900.0, [0.0, 0.0, 800.0, 800.0], Fit::Contain);
        assert_eq!((r, s), ([0.0, 175.0, 800.0, 450.0], 0.5));
    }

    #[test]
    fn move_blends_matching_layers_and_fades_the_rest() {
        let e = env(1920.0, 1080.0);
        let a = Scene::ScreenCam.resolve(&e);
        let b = Scene::SideBySide.resolve(&e);
        assert_eq!(blend(&a, &b, 0.0).layers, a.layers);
        let end = blend(&a, &b, 1.0);
        assert_eq!(end.layers.iter().map(|l| l.rect).collect::<Vec<_>>(), b.layers.iter().map(|l| l.rect).collect::<Vec<_>>());
        let mid = blend(&a, &b, 0.5);
        assert_eq!(mid.layers.len(), 2);
        let s = mid.layers[0].rect;
        assert!(s[2] < 1920.0 && s[2] > b.layers[0].rect[2], "screen shrinking: {s:?}");
        // Screen -> Camera: the screen fades out under, the camera fades in.
        let c = Scene::Cam.resolve(&e);
        let s = Scene::Screen.resolve(&e);
        let mid = blend(&s, &c, 0.5);
        assert_eq!(mid.layers.len(), 2);
        assert_eq!(mid.layers[0].content, Content::Screen);
        assert!((mid.layers[0].opacity - 0.5).abs() < 1e-9 && (mid.layers[1].opacity - 0.5).abs() < 1e-9);
    }

    #[test]
    fn custom_layouts_round_trip_and_scale() {
        let e = env(1920.0, 1080.0);
        let l = Layout::from_template(Scene::SideBySide, &e);
        let f = l.resolve(&e);
        let t = Scene::SideBySide.resolve(&e);
        for (x, y) in f.layers.iter().zip(&t.layers) {
            for k in 0..4 {
                assert!((x.rect[k] - y.rect[k]).abs() <= 1.0, "{:?} vs {:?}", x.rect, y.rect);
            }
        }
        let json = serde_json::to_string(&l).unwrap();
        let back: Layout = serde_json::from_str(&json).unwrap();
        assert_eq!(back.resolve(&e), f);
        // At 4K the same layout scales.
        let big = l.resolve(&env(3840.0, 2160.0));
        assert!((big.layers[0].rect[2] - 2.0 * f.layers[0].rect[2]).abs() <= 2.0);
        // Text with a box: a color layer, then the text inside it.
        let text = Layout {
            layers: vec![Layer { content: Content::Text(TextSpec::default()), rect: [0.05, 0.8, 0.3, 0.1], ..Default::default() }],
            ..Default::default()
        };
        let f = text.resolve(&e);
        assert!(matches!(f.layers[0].content, Content::Color(_)));
        assert!(matches!(f.layers[1].content, Content::Text(_)));
        assert!(f.layers[1].rect[2] < f.layers[0].rect[2]);
    }

    #[test]
    fn plain_styles() {
        assert!(Style::default().is_plain());
        assert!(!Style::rounded(10.0, 100.0, 100.0).is_plain());
        assert!(!Style { shape: Shape::Circle, ..Default::default() }.is_plain());
        assert!(!Style { border: 2.0, ..Default::default() }.is_plain());
    }

    #[test]
    fn canvas_names_parse() {
        use clap::ValueEnum;
        for c in Canvas::ALL {
            let name = c.to_possible_value().unwrap().get_name().to_string();
            assert_eq!(Canvas::from_str(&name, false).unwrap(), c);
            let json = serde_json::to_string(&c).unwrap();
            assert_eq!(json, format!("\"{name}\""));
        }
    }

    #[test]
    fn dragging_moves_and_snaps() {
        let r = drag_box([0.1, 0.1, 0.2, 0.2], Grip::Body, [0.05, 0.0], 0.02, 0.01);
        assert!((r[0] - 0.15).abs() < 1e-9 && r[2] == 0.2);
        // The center lands near the middle: it snaps there.
        let r = drag_box([0.3, 0.1, 0.2, 0.2], Grip::Body, [0.095, 0.0], 0.02, 0.01);
        assert!((r[0] + r[2] / 2.0 - 0.5).abs() < 1e-9, "{r:?}");
        // The left edge snaps to the frame edge.
        let r = drag_box([0.02, 0.4, 0.2, 0.2], Grip::Body, [-0.015, 0.0], 0.02, 0.01);
        assert_eq!(r[0], 0.0);
    }

    #[test]
    fn corners_resize_with_a_minimum() {
        let r = drag_box([0.1, 0.1, 0.2, 0.2], Grip::Corner(true, true), [0.1, 0.05], 0.02, 0.0);
        assert!((r[2] - 0.3).abs() < 1e-9 && (r[3] - 0.25).abs() < 1e-9 && r[0] == 0.1);
        // Dragging the top-left past the bottom-right stops at the minimum.
        let r = drag_box([0.1, 0.1, 0.2, 0.2], Grip::Corner(false, false), [0.5, 0.5], 0.02, 0.0);
        assert!((r[2] - 0.02).abs() < 1e-9 && (r[0] + r[2] - 0.3).abs() < 1e-9, "{r:?}");
    }
}
