//! The Preview window: plays a recording, and for one made with Record for
//! smart zoom shows its zooms on a timeline to move, resize, add and delete
//! before exporting the zoomed copy.
//!
//! The zooms are saved to `<name>.zoom.json` as they change (the file the
//! CLI's `simple-record zoom` reads too), and the export uses them as they
//! are. The picture is the screen as the export zooms it: for a composed
//! recording, the screen on its own (the export adds the camera and the
//! background as recorded).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use egui::{Align, Color32, CornerRadius, Layout, Margin, RichText, Sense, Stroke, Ui, pos2, vec2};

use super::player::Player;
use super::theme::{self, BODY, SECONDARY, TOOLBAR_H};
use super::widgets as w;
use super::{Cmd, State};
use crate::zoom::{self, EventLog, Focus, Segment, ZoomOptions};

/// Zoom levels offered for a zoom.
const LEVELS: [(f64, &str); 4] = [(1.5, "1.5×"), (2.0, "2×"), (2.5, "2.5×"), (3.0, "3×")];
/// A zoom is at least this long (seconds).
const MIN_LEN: f64 = 0.5;
/// A new zoom's length.
const NEW_LEN: f64 = 3.0;
/// Width of the grab zone at a zoom's ends (points).
const EDGE: f32 = 7.0;

pub struct Editor {
    pub file: PathBuf,
    pub player: Player,
    /// The newest frame as a texture (registered with the window's renderer).
    pub tex: Option<egui::TextureId>,
    pub px: [u32; 2],
    pub zoom: Option<Zooms>,
    /// Why zooms can't be edited for this recording.
    pub note: Option<String>,
    drag: Option<Drag>,
    /// Playing when a scrub began (resumes after it).
    resume: bool,
}

/// The recording's zooms being edited.
pub struct Zooms {
    pub segs: Vec<Segment>,
    fps: u32,
    /// Cursor position per logged frame (0-1; outside when off the picture).
    cursor: Vec<(f32, f32)>,
    clicks: Vec<f64>,
    /// The view at each logged frame (from `zoom::camera_path`).
    path: Vec<(f64, f64, f64)>,
    pub selected: Option<usize>,
    /// The export can't rebuild this recording's scene.
    pub blocked: Option<String>,
    composed: bool,
    duration: f64,
}

#[derive(Clone)]
enum Drag {
    Scrub,
    /// Moving zoom `i`: the pointer's time when it was grabbed, and the zoom then.
    Move(usize, f64, Segment),
    Start(usize),
    End(usize),
}

fn first_existing(paths: [PathBuf; 2]) -> Option<PathBuf> {
    paths.into_iter().find(|p| p.exists())
}

impl Editor {
    pub fn open(file: &Path) -> Result<Editor> {
        let mut video = file.to_path_buf();
        let mut zoom = None;
        let mut note = None;
        if zoom::available(file) {
            match Zooms::load(file) {
                Ok((z, src)) => {
                    if let Some(s) = src {
                        video = s;
                    }
                    zoom = Some(z);
                }
                Err(e) => note = Some(format!("This recording's click log can't be read: {e:#}")),
            }
        } else {
            note = Some("Turn on Settings › Record for smart zoom to edit zooms for the next recording.".into());
        }
        let player = Player::open(&video, file).with_context(|| format!("can't play {}", file.display()))?;
        Ok(Editor { file: file.to_path_buf(), player, tex: None, px: [0, 0], zoom, note, drag: None, resume: false })
    }
}

impl Zooms {
    /// The zooms for `rec`, and the file the export zooms (when it isn't
    /// the recording itself).
    fn load(rec: &Path) -> Result<(Zooms, Option<PathBuf>)> {
        let ev = zoom::events_path(rec);
        let log: EventLog = serde_json::from_str(&std::fs::read_to_string(&ev)?).context("not a click log")?;
        anyhow::ensure!(!log.frames.is_empty() && log.fps > 0, "nothing was logged");
        let src = match log.composed {
            true => Some(
                first_existing([zoom::work_screen_path(rec), crate::record::screen_file_path(rec)]).context("the screen wasn't kept apart")?,
            ),
            false => None,
        };
        let segs = zoom::load_segments(rec, &log, &ZoomOptions { level: 2.0, fresh: false })?;
        let blocked = (log.composed && log.extras)
            .then(|| "The scene showed other displays, cameras, videos or web pages, which smart zoom can't rebuild: the zooms can be edited but not exported.".to_string());
        let mut z = Zooms {
            segs,
            fps: log.fps,
            cursor: log.frames.iter().map(|s| (s.x, s.y)).collect(),
            clicks: zoom::clicks(&log.frames, log.fps).into_iter().map(|c| c.0).collect(),
            path: vec![],
            selected: None,
            blocked,
            composed: log.composed,
            duration: log.frames.len() as f64 / log.fps as f64,
        };
        z.segs.sort_by(|a, b| a.start.total_cmp(&b.start));
        z.path = zoom::camera_path(&z.segs, z.cursor.len(), z.fps);
        Ok((z, src))
    }

    /// The zooms found from the clicks again, at `level`.
    fn redetect(&mut self, rec: &Path, level: f64) {
        if let Ok(a) = zoom::activity(rec) {
            self.segs = a.segments(level);
            self.selected = None;
        }
    }

    /// Recompute the view and save the zooms (the export reads them).
    fn changed(&mut self, rec: &Path) {
        self.segs.sort_by(|a, b| a.start.total_cmp(&b.start));
        self.path = zoom::camera_path(&self.segs, self.cursor.len(), self.fps);
        if let Ok(json) = serde_json::to_string_pretty(&self.segs)
            && let Err(e) = std::fs::write(zoom::segments_path(rec), json)
        {
            eprintln!("rec: warning: couldn't save the zooms: {e}");
        }
    }

    /// The view at `t` seconds: (magnification, center x, center y).
    fn view(&self, t: f64) -> (f64, f64, f64) {
        let i = ((t * self.fps as f64).round() as usize).min(self.path.len().saturating_sub(1));
        self.path.get(i).copied().unwrap_or((1.0, 0.5, 0.5))
    }

    /// Where the cursor was at `t`, kept on the picture.
    fn cursor_at(&self, t: f64) -> (f64, f64) {
        let i = ((t * self.fps as f64) as usize).min(self.cursor.len().saturating_sub(1));
        match self.cursor.get(i) {
            Some(&(x, y)) if (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y) => (x as f64, y as f64),
            _ => (0.5, 0.5),
        }
    }

    /// Room for a zoom of `i` (or a new one at `t` when `i` is None):
    /// between the zooms around it.
    fn room(&self, i: Option<usize>, t: f64) -> (f64, f64) {
        let lo = self.segs.iter().enumerate().filter(|(j, s)| Some(*j) != i && s.end <= t + 1e-9).map(|(_, s)| s.end).fold(0.0, f64::max);
        let hi = self.segs.iter().enumerate().filter(|(j, s)| Some(*j) != i && s.start >= t - 1e-9).map(|(_, s)| s.start).fold(self.duration, f64::min);
        (lo, hi)
    }

    /// Add a zoom at `t`, looking where the cursor was. Returns false when
    /// there's no room.
    fn add(&mut self, t: f64, level: f64) -> bool {
        if self.segs.iter().any(|s| t >= s.start && t < s.end) {
            return false;
        }
        let (lo, hi) = self.room(None, t);
        if hi - lo < MIN_LEN {
            return false;
        }
        let start = (t - 0.3).max(lo);
        let end = (start + NEW_LEN).min(hi);
        let start = start.min(end - MIN_LEN).max(lo);
        let (x, y) = self.cursor_at(t);
        self.segs.push(Segment { start, end, level, focus: vec![Focus { t: 0.0, x, y }] });
        self.segs.sort_by(|a, b| a.start.total_cmp(&b.start));
        self.selected = self.segs.iter().position(|s| s.start == start);
        true
    }
}

fn fmt_time(t: f64) -> String {
    let t = t.max(0.0);
    format!("{}:{:04.1}", (t / 60.0) as u64, t % 60.0)
}

/// The Preview window.
pub fn window(ui: &mut Ui, st: &mut State) {
    let p = theme::pal(ui);
    let Some(mut ed) = st.editor.take() else { return };
    egui::Panel::top("editor-toolbar")
        .exact_size(TOOLBAR_H)
        .frame(egui::Frame::NONE.fill(p.window).inner_margin(Margin::symmetric(12, 0)))
        .show(ui, |ui| {
            super::ui::toolbar(ui, |ui| {
                ui.label(RichText::new("Preview").font(theme::semibold(BODY + 1.0)));
                ui.add_space(8.0);
                ui.label(RichText::new(ed.file.file_name().unwrap_or_default().to_string_lossy()).size(SECONDARY).color(p.secondary));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Reveal in Finder").clicked() {
                        super::macos::reveal(&ed.file);
                    }
                });
            });
        });
    egui::Panel::bottom("editor-timeline")
        .frame(egui::Frame::NONE.fill(p.window).inner_margin(Margin { left: 16, right: 16, top: 10, bottom: 14 }))
        .show(ui, |ui| bottom(ui, st, &mut ed));
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(p.window).inner_margin(Margin::symmetric(16, 12))).show(ui, |ui| picture(ui, &ed));
    keys(ui, &mut ed);
    if ed.player.playing() || ed.drag.is_some() {
        ui.ctx().request_repaint();
    } else {
        // Frames still arrive for a moment after a seek.
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    }
    st.editor = Some(ed);
}

fn keys(ui: &mut Ui, ed: &mut Editor) {
    if ui.ctx().memory(|m| m.focused().is_some()) {
        return;
    }
    let (space, del, left, right, shift) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::Space),
            i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
            i.key_pressed(egui::Key::ArrowLeft),
            i.key_pressed(egui::Key::ArrowRight),
            i.modifiers.shift,
        )
    });
    if space {
        ed.player.toggle();
    }
    if left || right {
        ed.player.pause();
        let step = if shift { 1.0 } else { 1.0 / ed.player.fps };
        let t = ed.player.time() + if right { step } else { -step };
        ed.player.seek_to(t);
    }
    if del
        && let Some(z) = ed.zoom.as_mut()
        && let Some(i) = z.selected.take()
    {
        z.segs.remove(i);
        z.changed(&ed.file);
    }
}

/// The frame, zoomed as the export zooms it.
fn picture(ui: &mut Ui, ed: &Editor) {
    let p = theme::pal(ui);
    let area = ui.available_rect_before_wrap();
    ui.painter().rect_filled(area, CornerRadius::same(8), p.field);
    let Some(tex) = ed.tex else {
        ui.painter().text(area.center(), egui::Align2::CENTER_CENTER, "Loading…", egui::FontId::proportional(BODY), p.tertiary);
        return;
    };
    let aspect = ed.px[0].max(1) as f32 / ed.px[1].max(1) as f32;
    let mut size = area.size();
    if size.x / size.y > aspect {
        size.x = size.y * aspect;
    } else {
        size.y = size.x / aspect;
    }
    let rect = egui::Rect::from_center_size(area.center(), size);
    let (z, x, y) = ed.zoom.as_ref().map_or((1.0, 0.5, 0.5), |zm| zm.view(ed.player.time()));
    let h = 0.5 / z.max(1.0);
    let uv = egui::Rect::from_min_max(pos2((x - h) as f32, (y - h) as f32), pos2((x + h) as f32, (y + h) as f32));
    egui::Image::new((tex, size)).uv(uv).corner_radius(6).paint_at(ui, rect);
    if z > 1.05 {
        let badge = format!("{z:.1}×");
        let pos = rect.right_top() + vec2(-10.0, 10.0);
        let galley = ui.painter().layout_no_wrap(badge, egui::FontId::proportional(SECONDARY), Color32::WHITE);
        let r = egui::Rect::from_min_size(pos - vec2(galley.size().x + 14.0, 0.0), galley.size() + vec2(14.0, 6.0));
        ui.painter().rect_filled(r, CornerRadius::same(255), Color32::from_black_alpha(150));
        ui.painter().galley(r.min + vec2(7.0, 3.0), galley, Color32::WHITE);
    }
}

/// Transport, timeline and actions.
fn bottom(ui: &mut Ui, st: &mut State, ed: &mut Editor) {
    let p = theme::pal(ui);
    ui.spacing_mut().item_spacing.y = 8.0;
    ui.horizontal(|ui| {
        if play_button(ui, ed.player.playing()).on_hover_text("Play or pause (Space)").clicked() {
            ed.player.toggle();
        }
        ui.add_space(4.0);
        ui.label(RichText::new(format!("{}  /  {}", fmt_time(ed.player.time()), fmt_time(ed.player.duration))).monospace().size(SECONDARY + 1.0).color(p.secondary));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().button_padding = vec2(14.0, 6.0);
            let Some(z) = ed.zoom.as_mut() else { return };
            if let Some(pr) = &st.zoom_progress {
                let done = pr.load(std::sync::atomic::Ordering::Relaxed) as f32 / 1000.0;
                ui.add(egui::ProgressBar::new(done).desired_width(140.0).text(format!("Exporting {:.0}%", done * 100.0)));
            } else {
                let ok = z.blocked.is_none() && !z.segs.is_empty();
                let r = ui.add_enabled(ok, egui::Button::new(RichText::new("Export Zoomed Copy").color(Color32::WHITE)).fill(theme::accent()));
                if r.on_hover_text("Write …-zoomed.mov next to the recording, with these zooms").clicked() {
                    st.cmds.push(Cmd::ExportEdited);
                }
            }
            if ui.button("Re-detect").on_hover_text("Replace these zooms with ones found from the clicks again").clicked() {
                z.redetect(&ed.file, st.zoom_opts.level);
                z.changed(&ed.file);
            }
        });
    });
    timeline(ui, ed);
    match ed.zoom.as_mut() {
        Some(z) => {
            ui.horizontal(|ui| match z.selected.filter(|&i| i < z.segs.len()) {
                Some(i) => {
                    let n = z.segs.len();
                    let s = &mut z.segs[i];
                    ui.label(RichText::new(format!("Zoom {} of {n}", i + 1)).font(theme::semibold(SECONDARY + 1.0)));
                    ui.label(RichText::new(format!("{} – {}", fmt_time(s.start), fmt_time(s.end))).size(SECONDARY).color(p.secondary));
                    ui.add_space(12.0);
                    let mut level = s.level;
                    let opts: Vec<(f64, &str, bool, &str)> = LEVELS.iter().map(|(l, n)| (*l, *n, true, "")).collect();
                    if w::segmented(ui, &mut level, &opts) && level != s.level {
                        s.level = level;
                        z.changed(&ed.file);
                    }
                    ui.add_space(8.0);
                    if ui.button("Delete").on_hover_text("Remove this zoom (Delete)").clicked() {
                        z.segs.remove(i);
                        z.selected = None;
                        z.changed(&ed.file);
                    }
                }
                None => {
                    let n = z.segs.len();
                    let what = match n {
                        0 => "No zooms.".to_string(),
                        1 => "1 zoom.".to_string(),
                        n => format!("{n} zooms."),
                    };
                    w::hint(ui, &format!("{what} Drag a zoom to move it, its ends to resize it; double-click the track to add one. Space plays, ← → step."));
                }
            });
            if let Some(b) = &z.blocked {
                w::hint(ui, b);
            } else if z.composed {
                w::hint(ui, "The preview shows the screen; the export adds the camera and background as recorded.");
            }
        }
        None => {
            if let Some(n) = &ed.note {
                w::hint(ui, n);
            }
        }
    }
}

fn play_button(ui: &mut Ui, playing: bool) -> egui::Response {
    let p = theme::pal(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::click());
    let fill = if resp.is_pointer_button_down_on() { theme::blend(p.control, p.pressed) } else if resp.hovered() { theme::blend(p.control, p.hover) } else { p.control };
    ui.painter().circle_filled(rect.center(), 17.0, fill);
    let c = rect.center();
    if playing {
        for dx in [-3.5, 3.5] {
            ui.painter().rect_filled(egui::Rect::from_center_size(c + vec2(dx, 0.0), vec2(3.5, 12.0)), CornerRadius::same(1), p.text);
        }
    } else {
        let pts = vec![c + vec2(-4.0, -7.0), c + vec2(7.0, 0.0), c + vec2(-4.0, 7.0)];
        ui.painter().add(egui::Shape::convex_polygon(pts, p.text, Stroke::NONE));
    }
    resp
}

/// A ruler, the zooms track (when the recording has one) and the playhead.
fn timeline(ui: &mut Ui, ed: &mut Editor) {
    let p = theme::pal(ui);
    let dur = ed.player.duration.max(0.001);
    let track_h = if ed.zoom.is_some() { 36.0 } else { 10.0 };
    let (rect, bg) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0 + track_h), Sense::click_and_drag());
    let ruler = egui::Rect::from_min_size(rect.min, vec2(rect.width(), 20.0));
    let track = egui::Rect::from_min_max(pos2(rect.left(), ruler.bottom()), rect.max);
    let x_of = |t: f64| rect.left() + (t / dur).clamp(0.0, 1.0) as f32 * rect.width();
    let t_of = |x: f32| ((x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64 * dur;
    let painter = ui.painter_at(rect.expand(4.0));

    // Ruler: labels at least ~70 points apart, on a round step.
    let per_pt = dur / rect.width() as f64;
    let step = [0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0].into_iter().find(|s| s / per_pt >= 70.0).unwrap_or(1200.0);
    let mut t = 0.0;
    while t <= dur + 1e-9 {
        let x = x_of(t);
        painter.line_segment([pos2(x, ruler.bottom() - 5.0), pos2(x, ruler.bottom())], Stroke::new(1.0, p.tertiary));
        let label = if step < 1.0 { fmt_time(t) } else { crate::record::clock(t) };
        painter.text(pos2(x + 3.0, ruler.top() + 1.0), egui::Align2::LEFT_TOP, label, egui::FontId::proportional(SECONDARY - 1.0), p.tertiary);
        t += step;
    }
    painter.rect_filled(track, CornerRadius::same(6), p.field);

    // Scrub on the ruler or empty track; double-click the track to add a zoom.
    let mut changed = false;
    if (bg.drag_started() || bg.clicked())
        && let Some(z) = ed.zoom.as_mut()
    {
        z.selected = None;
    }
    if bg.drag_started() {
        ed.drag = Some(Drag::Scrub);
        ed.resume = ed.player.playing();
        ed.player.pause();
    }
    if let Some(pos) = bg.interact_pointer_pos()
        && (bg.clicked() || matches!(ed.drag, Some(Drag::Scrub)))
    {
        ed.player.seek_to(t_of(pos.x));
    }
    if bg.double_clicked()
        && let Some(pos) = bg.interact_pointer_pos()
        && track.contains(pos)
        && let Some(z) = ed.zoom.as_mut()
    {
        let level = z.selected.and_then(|i| z.segs.get(i)).map_or(2.0, |s| s.level);
        changed |= z.add(t_of(pos.x), level);
    }

    if let Some(z) = ed.zoom.as_mut() {
        for &c in &z.clicks {
            let x = x_of(c);
            painter.line_segment([pos2(x, track.top() + 3.0), pos2(x, track.top() + 8.0)], Stroke::new(1.5, p.tertiary));
        }
        let inner = track.shrink2(vec2(0.0, 6.0));
        for i in 0..z.segs.len() {
            let s = &z.segs[i];
            let r = egui::Rect::from_min_max(pos2(x_of(s.start), inner.top() + 4.0), pos2(x_of(s.end).max(x_of(s.start) + 4.0), inner.bottom()));
            let id = ui.id().with(("zoom", i));
            let body = ui.interact(r.shrink2(vec2(EDGE.min(r.width() / 3.0), 0.0)), id, Sense::click_and_drag());
            let left = ui.interact(egui::Rect::from_min_max(pos2(r.left() - 3.0, r.top()), pos2(r.left() + EDGE.min(r.width() / 3.0), r.bottom())), id.with("l"), Sense::drag());
            let right = ui.interact(egui::Rect::from_min_max(pos2(r.right() - EDGE.min(r.width() / 3.0), r.top()), pos2(r.right() + 3.0, r.bottom())), id.with("r"), Sense::drag());
            if body.hovered() {
                ui.ctx().set_cursor_icon(if body.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
            }
            if left.hovered() || right.hovered() || left.dragged() || right.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            if body.clicked() || body.drag_started() || left.drag_started() || right.drag_started() {
                z.selected = Some(i);
            }
            if body.double_clicked() {
                ed.player.seek_to(s.start);
            }
            let grab = body.interact_pointer_pos().map(|p| t_of(p.x));
            if body.drag_started() {
                ed.drag = Some(Drag::Move(i, grab.unwrap_or(s.start), s.clone()));
            }
            if left.drag_started() {
                ed.drag = Some(Drag::Start(i));
            }
            if right.drag_started() {
                ed.drag = Some(Drag::End(i));
            }
            let selected = z.selected == Some(i);
            let fill = if selected { theme::accent() } else { theme::accent().gamma_multiply(0.7) };
            painter.rect_filled(r, CornerRadius::same(5), fill);
            if selected {
                painter.rect_stroke(r, CornerRadius::same(5), Stroke::new(1.5, p.text), egui::StrokeKind::Outside);
            }
            for x in [r.left() + 3.0, r.right() - 3.0] {
                painter.line_segment([pos2(x, r.top() + 6.0), pos2(x, r.bottom() - 6.0)], Stroke::new(1.5, Color32::from_black_alpha(90)));
            }
            if r.width() > 34.0 {
                painter.text(r.center(), egui::Align2::CENTER_CENTER, format!("{:.1}×", s.level), egui::FontId::proportional(SECONDARY), Color32::from_black_alpha(200));
            }
        }
        // Apply the drag in progress.
        if let Some(pos) = ui.ctx().pointer_interact_pos() {
            let t = t_of(pos.x);
            match ed.drag.clone() {
                Some(Drag::Move(i, grab, orig)) if i < z.segs.len() => {
                    let (lo, hi) = z.room(Some(i), orig.start);
                    let len = orig.end - orig.start;
                    let start = (orig.start + t - grab).clamp(lo, (hi - len).max(lo));
                    let dt = start - orig.start;
                    let s = &mut z.segs[i];
                    s.start = start;
                    s.end = start + len;
                    s.focus = orig.focus.iter().map(|f| Focus { t: if f.t > 0.0 { f.t + dt } else { f.t }, ..*f }).collect();
                    changed = true;
                }
                Some(Drag::Start(i)) if i < z.segs.len() => {
                    let (lo, _) = z.room(Some(i), z.segs[i].start);
                    let s = &mut z.segs[i];
                    s.start = t.clamp(lo, s.end - MIN_LEN);
                    changed = true;
                }
                Some(Drag::End(i)) if i < z.segs.len() => {
                    let (_, hi) = z.room(Some(i), z.segs[i].end);
                    let s = &mut z.segs[i];
                    s.end = t.clamp(s.start + MIN_LEN, hi);
                    changed = true;
                }
                _ => {}
            }
        }
    }

    // The playhead.
    let x = x_of(ed.player.time());
    painter.line_segment([pos2(x, ruler.top() + 12.0), pos2(x, rect.bottom())], Stroke::new(1.5, p.text));
    painter.add(egui::Shape::convex_polygon(vec![pos2(x - 5.0, ruler.top() + 10.0), pos2(x + 5.0, ruler.top() + 10.0), pos2(x, ruler.top() + 16.0)], p.text, Stroke::NONE));

    // A drag ends on release: save once then.
    let released = ui.input(|i| i.pointer.any_released());
    if released && ed.drag.is_some() {
        if matches!(ed.drag, Some(Drag::Scrub)) && ed.resume {
            ed.player.play();
        }
        let edited = !matches!(ed.drag, Some(Drag::Scrub));
        ed.drag = None;
        if edited && let Some(z) = ed.zoom.as_mut() {
            z.changed(&ed.file);
        }
    } else if changed && let Some(z) = ed.zoom.as_mut() {
        // Live: the view follows the edit; saved on release.
        z.path = zoom::camera_path(&z.segs, z.cursor.len(), z.fps);
        if ed.drag.is_none() {
            z.changed(&ed.file);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zooms(segs: Vec<(f64, f64)>) -> Zooms {
        Zooms {
            segs: segs.into_iter().map(|(start, end)| Segment { start, end, level: 2.0, focus: vec![Focus { t: 0.0, x: 0.5, y: 0.5 }] }).collect(),
            fps: 10,
            cursor: vec![(0.25, 0.75); 200],
            clicks: vec![],
            path: vec![],
            selected: None,
            blocked: None,
            composed: false,
            duration: 20.0,
        }
    }

    #[test]
    fn a_new_zoom_fits_between_the_others() {
        let mut z = zooms(vec![(2.0, 5.0), (7.0, 9.0)]);
        assert!(!z.add(3.0, 2.0), "inside a zoom adds nothing");
        assert!(z.add(5.5, 2.0));
        let s = &z.segs[1];
        assert!(s.start >= 5.0 && s.end <= 7.0 + 1e-9, "{s:?}");
        assert_eq!((s.focus[0].x, s.focus[0].y), (0.25, 0.75), "looks where the cursor was");
        assert_eq!(z.selected, Some(1));
        // No room for another between 5 and 7 once that one fills it.
        z.segs[1].start = 5.0;
        z.segs[1].end = 7.0;
        assert!(!z.add(6.9, 2.0));
    }

    #[test]
    fn room_is_bounded_by_the_neighbours() {
        let z = zooms(vec![(2.0, 5.0), (7.0, 9.0), (12.0, 13.0)]);
        assert_eq!(z.room(Some(1), 7.0), (5.0, 12.0));
        assert_eq!(z.room(Some(0), 2.0), (0.0, 7.0));
        assert_eq!(z.room(Some(2), 12.0), (9.0, 20.0));
    }
}
