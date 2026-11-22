//! Announcements: a banner drawn above whichever scene is live (and above a
//! scene change in progress), for a few seconds or until hidden. It belongs
//! to the session, not to a scene.
//!
//! Everything here is plain geometry and timing, so it is unit-tested without
//! a GPU; `compose::Compositor` draws the layers.

use serde::{Deserialize, Serialize};

use crate::effects::Effects;
use crate::layout::{Content, Fit, Placed, Style, TextSpec, smoothstep};

/// How long it takes to slide in (and out), in seconds.
pub const ANIM_SECS: f64 = 0.45;

/// Where and how an announcement sits in the frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, clap::ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnnounceStyle {
    /// A bar across the lower left, sliding in from the left.
    #[default]
    LowerThird,
    /// The whole frame dimmed, the content centered on it, fading in.
    CenterStage,
}

impl AnnounceStyle {
    pub const ALL: [AnnounceStyle; 2] = [AnnounceStyle::LowerThird, AnnounceStyle::CenterStage];

    pub fn label(self) -> &'static str {
        match self {
            AnnounceStyle::LowerThird => "Lower third",
            AnnounceStyle::CenterStage => "Full screen",
        }
    }
}

/// What an announcement shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, clap::ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnnounceKind {
    /// A title and an optional second line.
    #[default]
    Text,
    /// A picture (a logo, a sticker), with the text beside or under it.
    Image,
    /// The time left (of `secs`) in big digits, with the title as its label.
    Countdown,
}

impl AnnounceKind {
    pub const ALL: [AnnounceKind; 3] = [AnnounceKind::Text, AnnounceKind::Image, AnnounceKind::Countdown];

    pub fn label(self) -> &'static str {
        match self {
            AnnounceKind::Text => "Text",
            AnnounceKind::Image => "Image",
            AnnounceKind::Countdown => "Countdown",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Announcement {
    pub style: AnnounceStyle,
    pub kind: AnnounceKind,
    pub title: String,
    /// A smaller second line (may be empty); not used by a countdown.
    pub subtitle: String,
    /// The picture's file, for `Image`.
    pub image: String,
    /// The accent color of the bar's stripe and the full screen's underline.
    pub accent: [u8; 3],
    /// Seconds to stay up, counted from when it starts sliding in; none:
    /// until hidden. A countdown counts these down.
    pub secs: Option<f64>,
}

impl Default for Announcement {
    fn default() -> Self {
        Announcement {
            style: AnnounceStyle::default(),
            kind: AnnounceKind::default(),
            title: String::new(),
            subtitle: String::new(),
            image: String::new(),
            accent: [255, 59, 92],
            secs: Some(5.0),
        }
    }
}

/// How far in (0 to 1) an announcement is at frame time `t`, given when it
/// `start`ed and when it was told to `hide_at` (both frame time); none once
/// it has slid out. It also ends by itself after `secs`.
pub fn progress(start: f64, hide_at: Option<f64>, secs: Option<f64>, t: f64) -> Option<f64> {
    let end = match (hide_at, secs) {
        (Some(h), Some(s)) => Some(h.min(start + s)),
        (Some(h), None) => Some(h),
        (None, Some(s)) => Some(start + s),
        (None, None) => None,
    };
    let entering = ((t - start) / ANIM_SECS).clamp(0.0, 1.0);
    let leaving = match end {
        Some(e) => 1.0 - ((t - e) / ANIM_SECS).clamp(0.0, 1.0),
        None => 1.0,
    };
    let p = entering.min(leaving);
    if end.is_some_and(|e| t >= e + ANIM_SECS) { None } else { Some(p) }
}

/// Seconds left as a clock: `0:05`, `12:00`.
pub fn clock(remaining: f64) -> String {
    let s = remaining.max(0.0).ceil() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// A rough width of `s` set in the bold face at `font` pixels. Text layers
/// are centered in their box, so a box that is about as wide as its text
/// reads as left-aligned.
fn text_w(s: &str, font: f64) -> f64 {
    s.chars().count() as f64 * font * 0.6
}

fn color(rect: [f64; 4], c: [u8; 3], opacity: f64) -> Placed {
    Placed { content: Content::Color(c), rect, fit: Fit::Cover, style: Style::default(), mirror: false, opacity, effects: Effects::default() }
}

fn image(rect: [f64; 4], path: &str, opacity: f64) -> Placed {
    Placed { content: Content::Image(path.to_string()), rect, fit: Fit::Contain, style: Style::default(), mirror: false, opacity, effects: Effects::default() }
}

fn text(rect: [f64; 4], s: &str, c: [u8; 3], opacity: f64) -> Placed {
    let spec = TextSpec { text: s.to_string(), color: c, background: None };
    Placed { content: Content::Text(spec), rect, fit: Fit::Contain, style: Style::default(), mirror: false, opacity, effects: Effects::default() }
}

/// One line of text: what it says, its font size (at 1080p) and color.
struct Line {
    s: String,
    font: f64,
    color: [u8; 3],
}

const WHITE: [u8; 3] = [255, 255, 255];
const GRAY: [u8; 3] = [185, 188, 200];

impl Announcement {
    fn has_image(&self) -> bool {
        self.kind == AnnounceKind::Image && !self.image.trim().is_empty()
    }

    /// The lines to set, top to bottom, with font sizes for a 1080-pixel
    /// frame: `big` is the full screen's scale.
    fn lines(&self, remaining: Option<f64>, big: bool) -> Vec<Line> {
        let (title_f, sub_f, label_f, digits_f) = if big { (110.0, 44.0, 48.0, 260.0) } else { (52.0, 30.0, 30.0, 84.0) };
        let mut v = vec![];
        let mut add = |s: &str, font: f64, color: [u8; 3]| {
            if !s.trim().is_empty() {
                v.push(Line { s: s.trim().to_string(), font, color });
            }
        };
        if self.kind == AnnounceKind::Countdown {
            add(&self.title, label_f, GRAY);
            if let Some(r) = remaining {
                add(&clock(r), digits_f, WHITE);
            }
        } else {
            add(&self.title, title_f, WHITE);
            add(&self.subtitle, sub_f, GRAY);
        }
        v
    }

    /// Whether there is anything to show.
    pub fn is_blank(&self) -> bool {
        match self.kind {
            AnnounceKind::Text => self.title.trim().is_empty(),
            AnnounceKind::Image => !self.has_image() && self.title.trim().is_empty(),
            AnnounceKind::Countdown => self.secs.is_none(),
        }
    }

    /// The layers to draw for a `w` x `h` frame when `p` (0 to 1) of the way
    /// in, from the bottom up, with `remaining` seconds left (for a
    /// countdown). Sizes are for a 1080-pixel-tall frame, scaled.
    pub fn layers(&self, w: f64, h: f64, p: f64, remaining: Option<f64>) -> Vec<Placed> {
        let e = smoothstep(p.clamp(0.0, 1.0));
        if e <= 0.0 || self.is_blank() {
            return vec![];
        }
        let u = h / 1080.0;
        let img = self.has_image();
        let mut v = vec![];
        match self.style {
            AnnounceStyle::LowerThird => {
                let lines = self.lines(remaining, false);
                let (pad, gap, stripe) = (22.0 * u, 6.0 * u, 22.0 * u);
                let heights: Vec<f64> = lines.iter().map(|l| l.font / 0.8 * u).collect();
                let content_h = heights.iter().sum::<f64>() + gap * heights.len().saturating_sub(1) as f64;
                let bar_h = (content_h + 2.0 * pad).max(if img { 120.0 * u } else { 0.0 });
                let tag_w = if img { bar_h } else { stripe };
                let text_width = lines.iter().map(|l| text_w(&l.s, l.font * u)).fold(0.0, f64::max);
                let body_w = if lines.is_empty() { 0.0 } else { (text_width + 80.0 * u).min(w * 0.8 - tag_w) };
                let total = tag_w + body_w;
                let x = (e - 1.0) * total;
                let y = h * 0.93 - bar_h;
                if body_w > 0.0 {
                    v.push(color([x, y, total, bar_h], [16, 18, 22], 0.93 * e));
                }
                v.push(color([x, y, tag_w, bar_h], self.accent, e));
                if img {
                    let m = 10.0 * u;
                    v.push(image([x + m, y + m, tag_w - 2.0 * m, bar_h - 2.0 * m], &self.image, e));
                }
                let mut ty = y + (bar_h - content_h) / 2.0;
                for (l, hh) in lines.iter().zip(&heights) {
                    v.push(text([x + tag_w + 40.0 * u, ty, body_w - 80.0 * u, *hh], &l.s, l.color, e));
                    ty += hh + gap;
                }
            }
            AnnounceStyle::CenterStage => {
                let lines = self.lines(remaining, true);
                let (gap, pic, bar_w, bar_t) = (14.0 * u, 280.0 * u, 220.0 * u, 8.0 * u);
                let heights: Vec<f64> = lines.iter().map(|l| l.font / 0.8 * u).collect();
                let mut content_h = heights.iter().sum::<f64>() + gap * heights.len().saturating_sub(1) as f64;
                if img {
                    content_h += pic + if lines.is_empty() { 0.0 } else { 3.0 * gap };
                }
                if !lines.is_empty() || img {
                    content_h += 3.0 * gap + bar_t;
                }
                // Rises into place while it fades in.
                let mut y = (h - content_h) / 2.0 + (1.0 - e) * 60.0 * u;
                v.push(color([0.0, 0.0, w, h], [6, 7, 11], 0.9 * e));
                if img {
                    v.push(image([(w - pic) / 2.0, y, pic, pic], &self.image, e));
                    y += pic + if lines.is_empty() { 0.0 } else { 3.0 * gap };
                }
                for (l, hh) in lines.iter().zip(&heights) {
                    let tw = text_w(&l.s, l.font * u).min(w * 0.9);
                    v.push(text([(w - tw) / 2.0, y, tw, *hh], &l.s, l.color, e));
                    y += hh + gap;
                }
                if !lines.is_empty() || img {
                    v.push(color([(w - bar_w) / 2.0, y + 2.0 * gap, bar_w, bar_t], self.accent, e));
                }
            }
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(style: AnnounceStyle, sub: &str) -> Announcement {
        Announcement { style, title: "New video is out now".into(), subtitle: sub.into(), ..Announcement::default() }
    }

    fn inside(r: [f64; 4], w: f64, h: f64) -> bool {
        r[0] >= -0.5 && r[1] >= -0.5 && r[0] + r[2] <= w + 0.5 && r[1] + r[3] <= h + 0.5
    }

    fn texts(l: &[Placed]) -> Vec<String> {
        l.iter().filter_map(|p| if let Content::Text(t) = &p.content { Some(t.text.clone()) } else { None }).collect()
    }

    #[test]
    fn slides_in_holds_and_slides_out() {
        // Up for 2 seconds from t = 10.
        let at = |t| progress(10.0, None, Some(2.0), t);
        assert_eq!(at(9.0), Some(0.0));
        assert!((at(10.0 + ANIM_SECS / 2.0).unwrap() - 0.5).abs() < 1e-9);
        assert_eq!(at(11.0), Some(1.0));
        assert!(at(12.0 + ANIM_SECS / 2.0).unwrap() < 0.6);
        assert_eq!(at(12.0 + ANIM_SECS), None);
    }

    #[test]
    fn stays_until_hidden() {
        assert_eq!(progress(0.0, None, None, 3600.0), Some(1.0));
        // Hidden at 5 s: gone after the slide out.
        assert_eq!(progress(0.0, Some(5.0), None, 5.0), Some(1.0));
        assert_eq!(progress(0.0, Some(5.0), None, 5.0 + ANIM_SECS), None);
        // Hidden before its time is up wins; so does the timer when it is sooner.
        assert_eq!(progress(0.0, Some(1.0), Some(5.0), 1.0 + ANIM_SECS), None);
        assert_eq!(progress(0.0, Some(9.0), Some(2.0), 2.0 + ANIM_SECS), None);
    }

    #[test]
    fn hiding_while_sliding_in_slides_out_from_there() {
        let p = progress(0.0, Some(ANIM_SECS / 2.0), None, ANIM_SECS / 2.0).unwrap();
        assert!((p - 0.5).abs() < 1e-9);
    }

    #[test]
    fn clock_rounds_up_to_whole_seconds() {
        assert_eq!(clock(5.0), "0:05");
        assert_eq!(clock(4.2), "0:05");
        assert_eq!(clock(0.0), "0:00");
        assert_eq!(clock(61.0), "1:01");
        assert_eq!(clock(-3.0), "0:00");
    }

    #[test]
    fn nothing_to_draw_when_out_or_blank() {
        assert!(a(AnnounceStyle::LowerThird, "").layers(1920.0, 1080.0, 0.0, None).is_empty());
        let empty = Announcement { title: "  ".into(), ..Announcement::default() };
        assert!(empty.layers(1920.0, 1080.0, 1.0, None).is_empty());
        // An image with no file and no title; a countdown with no time to count.
        let img = Announcement { kind: AnnounceKind::Image, ..Announcement::default() };
        assert!(img.layers(1920.0, 1080.0, 1.0, None).is_empty());
        let cd = Announcement { kind: AnnounceKind::Countdown, secs: None, ..Announcement::default() };
        assert!(cd.layers(1920.0, 1080.0, 1.0, None).is_empty());
    }

    #[test]
    fn every_kind_and_style_fits_the_frame() {
        let kinds = [
            Announcement { title: "New video is out now".into(), subtitle: "youtube.com/@simple-record".into(), ..Default::default() },
            Announcement { title: "A very long title that goes on and on across the frame and then some more".into(), ..Default::default() },
            Announcement { kind: AnnounceKind::Image, image: "/tmp/logo.png".into(), title: "Sponsored by Acme".into(), ..Default::default() },
            Announcement { kind: AnnounceKind::Image, image: "/tmp/logo.png".into(), ..Default::default() },
            Announcement { kind: AnnounceKind::Countdown, title: "Starting in".into(), secs: Some(300.0), ..Default::default() },
        ];
        for base in kinds {
            for style in AnnounceStyle::ALL {
                for (w, h) in [(1920.0, 1080.0), (1280.0, 720.0), (1080.0, 1920.0)] {
                    let n = Announcement { style, ..base.clone() };
                    let l = n.layers(w, h, 1.0, Some(299.0));
                    assert!(!l.is_empty(), "{:?} {style:?}", n.kind);
                    for p in l {
                        assert!(inside(p.rect, w, h), "{:?} {style:?} {w}x{h}: {:?} {:?}", n.kind, p.content.label(), p.rect);
                    }
                }
            }
        }
    }

    #[test]
    fn bar_slides_in_from_the_left_and_fades() {
        let bar = a(AnnounceStyle::LowerThird, "x").layers(1920.0, 1080.0, 0.5, None);
        assert!(bar.iter().map(|p| p.rect[0]).fold(f64::MAX, f64::min) < 0.0);
        assert!(bar.iter().all(|p| p.opacity < 1.0));
    }

    #[test]
    fn full_screen_dims_the_whole_frame_and_centers_its_text() {
        let l = a(AnnounceStyle::CenterStage, "").layers(1920.0, 1080.0, 1.0, None);
        assert_eq!(l[0].rect, [0.0, 0.0, 1920.0, 1080.0]);
        assert!(l[0].opacity > 0.5 && l[0].opacity < 1.0);
        for p in l.iter().filter(|p| matches!(p.content, Content::Text(_))) {
            assert!(((p.rect[0] + p.rect[2] / 2.0) - 960.0).abs() < 1.0, "centered: {:?}", p.rect);
        }
        // Faded in with the slide.
        assert!(a(AnnounceStyle::CenterStage, "").layers(1920.0, 1080.0, 0.5, None).iter().all(|p| p.opacity < 1.0));
    }

    #[test]
    fn a_countdown_shows_the_time_left_under_its_label() {
        let cd = |style, r| Announcement { style, kind: AnnounceKind::Countdown, title: "Starting in".into(), secs: Some(300.0), ..Default::default() }.layers(1920.0, 1080.0, 1.0, Some(r));
        for style in AnnounceStyle::ALL {
            assert_eq!(texts(&cd(style, 125.5)), ["Starting in", "2:06"], "{style:?}");
            assert_eq!(texts(&cd(style, 3.0)), ["Starting in", "0:03"], "{style:?}");
        }
    }

    #[test]
    fn an_image_is_drawn_when_it_has_a_file() {
        for style in AnnounceStyle::ALL {
            let n = Announcement { style, kind: AnnounceKind::Image, image: "/tmp/logo.png".into(), title: "Acme".into(), ..Default::default() };
            let l = n.layers(1920.0, 1080.0, 1.0, None);
            assert!(l.iter().any(|p| p.content == Content::Image("/tmp/logo.png".into())), "{style:?}");
            // Text alone shows no image even when a file is set.
            let t = Announcement { kind: AnnounceKind::Text, ..n };
            assert!(!t.layers(1920.0, 1080.0, 1.0, None).iter().any(|p| matches!(p.content, Content::Image(_))), "{style:?}");
        }
    }

    #[test]
    fn a_subtitle_makes_the_bar_taller() {
        let h = |s: &str| a(AnnounceStyle::LowerThird, s).layers(1920.0, 1080.0, 1.0, None)[0].rect[3];
        assert!(h("more") > h(""));
    }
}
