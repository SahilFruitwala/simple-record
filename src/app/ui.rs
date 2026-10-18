//! egui layout of the main window and the settings window.

use std::sync::atomic::{AtomicBool, Ordering};

use egui::{Align, Color32, CornerRadius, Layout, Margin, RichText, Sense, Stroke, Ui, vec2};

use super::theme::{self, BODY, SECONDARY, TOOLBAR_H, TRAFFIC_LIGHTS_W};
use super::widgets::{self as w, Icon};
use super::{Cmd, Meter, State};
use crate::compose::{self, Corner, Place, Shape};
use crate::hotkey::{Action, Combo};
use crate::layout::{Background, Canvas, Transition};
use crate::settings::Settings;
use crate::settings::{FPS, RESOLUTIONS};
use crate::writer::{AudioLayout, Codec};

const RED: Color32 = Color32::from_rgb(0xff, 0x3b, 0x30);
const SIDEBAR_W: f32 = 304.0;

static DRAG: AtomicBool = AtomicBool::new(false);

/// Whether the toolbar asked to drag the window this frame.
pub fn take_drag_request() -> bool {
    DRAG.swap(false, Ordering::Relaxed)
}

/// A path with the home folder shown as `~`.
fn tilde(p: &std::path::Path) -> String {
    match std::env::var_os("HOME").map(std::path::PathBuf::from) {
        Some(h) if p.starts_with(&h) => format!("~/{}", p.strip_prefix(&h).unwrap().display()),
        _ => p.display().to_string(),
    }
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(title).font(theme::semibold(SECONDARY)).color(theme::pal(ui).secondary));
    ui.add_space(2.0);
}

/// Toolbar strip: empty space drags the window.
fn toolbar(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let rect = ui.max_rect();
    let bg = ui.interact(rect, ui.id().with("drag"), Sense::click_and_drag());
    if bg.drag_started() {
        DRAG.store(true, Ordering::Relaxed);
    }
    ui.horizontal_centered(|ui| {
        ui.add_space(TRAFFIC_LIGHTS_W);
        add(ui);
    });
    let p = theme::pal(ui);
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.separator));
}

fn pill(ui: &mut Ui, text: &str, fill: Color32, fg: Color32) {
    let galley = ui.painter().layout_no_wrap(text.to_string(), theme::semibold(SECONDARY), fg);
    let (rect, _) = ui.allocate_exact_size(galley.size() + vec2(16.0, 6.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(9), fill);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
}

pub fn main_window(ui: &mut Ui, st: &mut State) {
    let p = theme::pal(ui);
    let recording = st.recording.is_some();
    egui::Panel::top("toolbar")
        .exact_size(TOOLBAR_H)
        .frame(egui::Frame::NONE.fill(p.window).inner_margin(Margin::symmetric(12, 0)))
        .show(ui, |ui| {
            toolbar(ui, |ui| {
                ui.label(RichText::new("Rec").font(theme::semibold(BODY)));
                ui.add_space(6.0);
                if st.finishing {
                    pill(ui, "SAVING…", p.control, p.secondary);
                } else if st.is_paused() {
                    pill(ui, &format!("PAUSED  {}", st.clock()), Color32::from_rgb(0xff, 0x95, 0x00), Color32::WHITE);
                } else if recording {
                    pill(ui, &format!("● REC  {}", st.clock()), RED, Color32::WHITE);
                } else if st.preview.is_some() {
                    pill(ui, "PREVIEW", p.control, p.secondary);
                }
                if st.stream_statuses.iter().any(|s| s.live) {
                    ui.add_space(4.0);
                    pill(ui, "LIVE", RED, Color32::WHITE);
                }
                if let Some((text, at)) = &st.notice
                    && at.elapsed().as_secs_f32() < 4.0
                {
                    ui.add_space(6.0);
                    // Cut to fit, leaving room for the buttons on the right.
                    let w = (ui.available_width() - 160.0).max(40.0);
                    ui.allocate_ui(vec2(w, ui.available_height()), |ui| {
                        ui.add(egui::Label::new(RichText::new(text).size(SECONDARY).color(p.secondary)).truncate()).on_hover_text(text);
                    });
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Settings…").on_hover_text("Output, codec, resolution, audio (⌘,)").clicked() {
                        st.cmds.push(Cmd::ShowSettings);
                    }
                    if st.hotkey_failed.contains(&Action::Record)
                        && let Some(c) = st.settings.hotkey(Action::Record)
                    {
                        ui.label(RichText::new(format!("{} unavailable", c.label())).color(p.secondary));
                    }
                });
            });
        });
    egui::Panel::bottom("controls")
        .exact_size(68.0)
        .frame(egui::Frame::NONE.fill(p.window).inner_margin(Margin::symmetric(16, 10)))
        .show(ui, |ui| controls(ui, st));
    egui::Panel::right("sources")
        .exact_size(SIDEBAR_W)
        .frame(egui::Frame::NONE.fill(p.sidebar).stroke(Stroke::new(1.0, p.separator)))
        .show(ui, |ui| {
            // Lay the contents out in a child of exactly the panel's
            // size: a too-wide widget (a long device name) would otherwise
            // widen the panel past the window edge, under the preview.
            let rect = ui.max_rect();
            let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)));
            inner.set_clip_rect(rect.intersect(ui.clip_rect()));
            egui::ScrollArea::vertical().auto_shrink(false).show(&mut inner, |ui| {
                // 16 pt gutters; the floating scroll bar sits in the right one.
                egui::Frame::NONE.inner_margin(Margin { left: 16, right: 16, top: 0, bottom: 16 }).show(ui, |ui| {
                    ui.set_max_width(SIDEBAR_W - 32.0);
                    ui.set_min_width(SIDEBAR_W - 32.0);
                    ui.spacing_mut().interact_size.y = 24.0;
                    ui.spacing_mut().button_padding = vec2(10.0, 4.0);
                    sources(ui, st)
                });
            });
        });
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(p.window).inner_margin(Margin::same(16)))
        .show(ui, |ui| preview(ui, st));
    // Keyboard: ⌘, settings, ⌘W close (handled as hide by the app), space
    // does nothing (avoid accidental recordings).
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Comma)) {
        st.cmds.push(Cmd::ShowSettings);
    }
}

fn preview(ui: &mut Ui, st: &mut State) {
    let p = theme::pal(ui);
    if let Some(err) = st.error.clone() {
        egui::Frame::NONE
            .fill(Color32::from_rgba_unmultiplied(0xff, 0x3b, 0x30, 30))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::same(10))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(err).color(p.text));
                    if st.permissions_missing && ui.button("Request Permissions").clicked() {
                        crate::tcc::request_all(true, true, true, std::time::Duration::from_secs(0));
                    }
                    if ui.button("Dismiss").clicked() {
                        st.error = None;
                    }
                });
            });
        ui.add_space(10.0);
    }
    let avail = ui.available_size();
    let ppp = ui.ctx().pixels_per_point();
    // The scaler renders frames at exactly this many pixels (aspect kept).
    super::preview::set_target((avail.x * ppp).floor() as u32, (avail.y * ppp).floor() as u32);
    let aspect = st.preview_aspect.max(0.1);
    let mut size = vec2(avail.x, avail.x / aspect);
    if size.y > avail.y {
        size = vec2(avail.y * aspect, avail.y);
    }
    // Show the frame at its own pixel size when it fits (1:1, no second
    // scaling), e.g. always on Retina, except in low-power mode.
    let px = vec2(st.preview_px[0] as f32, st.preview_px[1] as f32) / ppp;
    if px.x > 0.0 && px.x <= avail.x + 0.5 && px.y <= avail.y + 0.5 && !st.settings.preview_low_power {
        size = px;
    }
    ui.vertical_centered(|ui| {
        ui.add_space(((avail.y - size.y) / 2.0).max(0.0));
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        // Snap to the pixel grid so texels land on pixels.
        let rect = egui::Rect::from_min_size(
            egui::pos2((rect.min.x * ppp).round() / ppp, (rect.min.y * ppp).round() / ppp),
            rect.size(),
        );
        // REC_DEBUG_PREVIEW: where the preview is, in window pixels (for
        // comparing it against a screenshot of the screen).
        if std::env::var_os("REC_DEBUG_PREVIEW").is_some() {
            eprintln!(
                "rec: preview rect {} {} {} {}",
                (rect.min.x * ppp).round(),
                (rect.min.y * ppp).round(),
                (rect.width() * ppp).round(),
                (rect.height() * ppp).round()
            );
        }
        // Square and edge to edge: rounded corners would show the backdrop
        // (black corners), and the frame already has the source's shape.
        let r = CornerRadius::ZERO;
        let live = st.recording.is_some() || st.preview.is_some();
        match &st.preview_tex {
            Some(id) if live => {
                egui::Image::new((*id, size)).paint_at(ui, rect);
            }
            _ => {
                ui.painter().rect_filled(rect, CornerRadius::same(10), theme::pal(ui).control);
                let msg = if st.finishing { "Saving…" } else { "No preview" };
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, msg, egui::FontId::proportional(BODY), Color32::GRAY);
            }
        }
        if st.recording.is_some() {
            let border = if st.is_paused() { Color32::from_rgb(0xff, 0x95, 0x00) } else { RED };
            // Around the picture, not over its edges.
            ui.painter().rect_stroke(rect, r, Stroke::new(2.0, border), egui::StrokeKind::Outside);
        }
        let custom = st.settings.scene_id().1.is_some();
        if live && st.region_select.is_some() {
            select_region(ui, st, rect);
        } else if live && custom {
            super::scenes::edit_in_preview(ui, st, rect);
        } else if live && st.settings.scene.has_pip() {
            drag_overlay(ui, st, rect);
        }
    });
}

/// Pick the part of the display to record by dragging over the preview of
/// the whole display. Esc cancels.
fn select_region(ui: &mut Ui, st: &mut State, preview: egui::Rect) {
    let accent = theme::accent();
    let Some(screen) = super::scenes::screen_rect(st, preview) else { return };
    let painter = ui.painter_at(preview);
    let shade = Color32::from_black_alpha(110);
    let resp = ui.interact(screen, ui.id().with("region"), Sense::drag());
    ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    let start = st.region_select.as_ref().and_then(|r| r.1);
    let current = resp.interact_pointer_pos().map(|p| p.clamp(screen.min, screen.max));
    let sel = match (start, current) {
        (Some(a), Some(b)) => Some(egui::Rect::from_two_pos(a, b)),
        _ => None,
    };
    match sel {
        Some(r) => {
            // Shade around the selection.
            for band in [
                egui::Rect::from_min_max(preview.min, egui::pos2(preview.max.x, r.min.y)),
                egui::Rect::from_min_max(egui::pos2(preview.min.x, r.max.y), preview.max),
                egui::Rect::from_min_max(egui::pos2(preview.min.x, r.min.y), egui::pos2(r.min.x, r.max.y)),
                egui::Rect::from_min_max(egui::pos2(r.max.x, r.min.y), egui::pos2(preview.max.x, r.max.y)),
            ] {
                painter.rect_filled(band, CornerRadius::ZERO, shade);
            }
            painter.rect_stroke(r, CornerRadius::ZERO, Stroke::new(1.5, accent), egui::StrokeKind::Outside);
            let (pw, ph) = st.display_points.get(st.settings.display.max(1) - 1).copied().unwrap_or((screen.width() as f64, screen.height() as f64));
            let label = format!("{:.0} × {:.0} pt", r.width() as f64 / screen.width() as f64 * pw, r.height() as f64 / screen.height() as f64 * ph);
            painter.text(r.center(), egui::Align2::CENTER_CENTER, label, theme::semibold(BODY), Color32::WHITE);
        }
        None => {
            painter.rect_filled(preview, CornerRadius::ZERO, Color32::from_black_alpha(60));
            let msg = "Drag over the screen to choose the area to record  ·  Esc cancels";
            let galley = painter.layout_no_wrap(msg.into(), theme::semibold(BODY), Color32::WHITE);
            let at = egui::pos2(preview.center().x - galley.size().x / 2.0, preview.top() + 14.0);
            painter.rect_filled(egui::Rect::from_min_size(at - vec2(10.0, 6.0), galley.size() + vec2(20.0, 12.0)), CornerRadius::same(8), Color32::from_black_alpha(160));
            painter.galley(at, galley, Color32::WHITE);
        }
    }
    if resp.drag_started() {
        if let Some(r) = st.region_select.as_mut() {
            r.1 = current;
        }
    }
    if resp.drag_stopped() {
        if let Some(r) = sel.filter(|r| r.width() >= 8.0 && r.height() >= 8.0) {
            let (pw, ph) = st.display_points.get(st.settings.display.max(1) - 1).copied().unwrap_or((1920.0, 1080.0));
            let fx = |x: f32| ((x - screen.min.x) / screen.width()) as f64;
            let fy = |y: f32| ((y - screen.min.y) / screen.height()) as f64;
            st.settings.region = Some([(fx(r.min.x) * pw).round(), (fy(r.min.y) * ph).round(), ((fx(r.max.x) - fx(r.min.x)) * pw).round(), ((fy(r.max.y) - fy(r.min.y)) * ph).round()]);
            st.region_select = None;
            let _ = st.settings.save();
        } else if let Some(r) = st.region_select.as_mut() {
            r.1 = None;
        }
    }
    if ui.input(|i| i.key_pressed(egui::Key::Escape))
        && let Some((before, _)) = st.region_select.take()
    {
        st.settings.region = before;
    }
}

/// Output-pixel frame the overlay geometry is computed in (any size with
/// the preview's aspect ratio; positions are fractions of it).
fn frame_size(st: &State) -> (f64, f64) {
    (3840.0, (3840.0 / st.preview_aspect.max(0.1) as f64).round())
}

fn overlay_ui_rect(st: &State, spec: &compose::PipSpec, preview: egui::Rect) -> egui::Rect {
    let (w, h) = frame_size(st);
    let [x, y, ow, oh] = compose::overlay_rect(spec, w, h, st.cam_aspect);
    let (sx, sy) = (preview.width() as f64 / w, preview.height() as f64 / h);
    egui::Rect::from_min_size(
        preview.min + vec2((x * sx) as f32, (y * sy) as f32),
        vec2((ow * sx) as f32, (oh * sy) as f32),
    )
}

/// Drag the camera overlay in the preview. It moves live (in the preview
/// and the recording) and snaps to a corner when released near one.
fn drag_overlay(ui: &mut Ui, st: &mut State, preview: egui::Rect) {
    const SNAP: f64 = 0.05;
    let spec = st.settings.pip_spec();
    let ov = overlay_ui_rect(st, &spec, preview);
    let resp = ui.interact(ov, ui.id().with("pip-drag"), Sense::drag());
    let accent = theme::accent();
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(if resp.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
        ui.painter().rect_stroke(ov, CornerRadius::same(4), Stroke::new(1.5, accent), egui::StrokeKind::Outside);
    }
    if resp.dragged() {
        let d = resp.drag_delta();
        let (cx, cy) = ((ov.center().x - preview.min.x) / preview.width(), (ov.center().y - preview.min.y) / preview.height());
        let at = [(cx + d.x / preview.width()).clamp(0.0, 1.0) as f64, (cy + d.y / preview.height()).clamp(0.0, 1.0) as f64];
        st.settings.pip_place = Place::At(at);
        // Show where it would snap.
        let (w, h) = frame_size(st);
        if let Some(c) = compose::snap_corner(&st.settings.pip_spec(), w, h, st.cam_aspect, SNAP) {
            let ghost = overlay_ui_rect(st, &compose::PipSpec { place: Place::Corner(c), ..st.settings.pip_spec() }, preview);
            ui.painter().rect_stroke(ghost, CornerRadius::same(4), Stroke::new(1.0, accent.gamma_multiply(0.7)), egui::StrokeKind::Outside);
        }
        st.apply_pip();
    }
    if resp.drag_stopped() {
        let (w, h) = frame_size(st);
        if let Some(c) = compose::snap_corner(&st.settings.pip_spec(), w, h, st.cam_aspect, SNAP) {
            st.settings.pip_place = Place::Corner(c);
        }
        st.apply_pip();
        let _ = st.settings.save();
    }
}

/// A small icon of `shape` (with the current roundness), `scale` times the
/// usual size.
pub(super) fn shape_icon(ui: &Ui, rect: egui::Rect, shape: Shape, roundness: f64, color: Color32, scale: f32) {
    let c = rect.center();
    let (w, h) = if shape.is_square() { (16.0 * scale, 16.0 * scale) } else { (22.0 * scale, 13.0 * scale) };
    let r = egui::Rect::from_center_size(c, vec2(w, h));
    let painter = ui.painter();
    let stroke = Stroke::new(1.5, color);
    match shape {
        Shape::Circle => {
            painter.circle_stroke(c, 8.0 * scale, stroke);
        }
        Shape::Squircle => {
            let n = compose::squircle_exponent(roundness);
            let pts: Vec<egui::Pos2> = (0..64)
                .map(|i| {
                    let t = i as f64 / 64.0 * std::f64::consts::TAU;
                    let (ct, s) = (t.cos(), t.sin());
                    let x = ct.signum() * ct.abs().powf(2.0 / n) * 8.0 * scale as f64;
                    let y = s.signum() * s.abs().powf(2.0 / n) * 8.0 * scale as f64;
                    c + vec2(x as f32, y as f32)
                })
                .collect();
            painter.add(egui::Shape::closed_line(pts, stroke));
        }
        _ => {
            let rad = match shape {
                Shape::Pill => h / 2.0,
                Shape::Rect => 0.0,
                _ => roundness as f32 * h / 2.0,
            };
            painter.rect_stroke(r, CornerRadius::same(rad.round() as u8), stroke, egui::StrokeKind::Middle);
        }
    }
}

/// Camera overlay look and position. Live: also while recording.
fn overlay_controls(ui: &mut Ui, s: &mut Settings) {
    let p = theme::pal(ui);
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let n = Shape::PICKER.len() as f32;
        let bw = ((ui.available_width() - 6.0 * (n - 1.0)) / n).max(28.0);
        for shape in Shape::PICKER {
            let sel = s.pip_shape == shape;
            let (rect, resp) = ui.allocate_exact_size(vec2(bw, 32.0), Sense::click());
            let fill = if sel { theme::accent().gamma_multiply(0.18) } else if resp.hovered() { p.hover } else { Color32::TRANSPARENT };
            ui.painter().rect_filled(rect, CornerRadius::same(7), fill);
            let stroke = if sel { Stroke::new(1.5, theme::accent()) } else { Stroke::new(1.0, p.control_border) };
            ui.painter().rect_stroke(rect, CornerRadius::same(7), stroke, egui::StrokeKind::Inside);
            shape_icon(ui, rect.shrink(2.0), shape, s.pip_roundness, if sel { theme::accent() } else { p.secondary }, 1.0);
            if resp.on_hover_text(shape.label()).clicked() {
                s.pip_shape = shape;
            }
        }
    });
    ui.add_space(4.0);
    // Label column, slider, then a fixed-width value so sliders line up.
    const LABEL_W: f32 = 72.0;
    const VALUE_W: f32 = 40.0;
    let slider = |ui: &mut Ui, value: &mut f64, range: std::ops::RangeInclusive<f64>, fmt: &dyn Fn(f64) -> String, enabled: bool, extra: f32| -> bool {
        ui.spacing_mut().slider_width = (ui.available_width() - VALUE_W - extra - 8.0).max(40.0);
        let changed = ui.add_enabled(enabled, egui::Slider::new(value, range).show_value(false)).changed();
        w::slot(ui, VALUE_W, Align::Max, RichText::new(fmt(*value)).size(SECONDARY + 1.0).color(p.secondary));
        changed
    };
    w::field(ui, "Size", LABEL_W, |ui| {
        let mut pct = s.pip_size * 100.0;
        if slider(ui, &mut pct, 10.0..=50.0, &|v| format!("{v:.0}%"), true, 0.0) {
            s.pip_size = pct.round() / 100.0;
        }
    });
    w::field(ui, "Roundness", LABEL_W, |ui| {
        let mut pct = s.pip_roundness * 100.0;
        let on = s.pip_shape.uses_roundness();
        if slider(ui, &mut pct, 0.0..=100.0, &|v| format!("{v:.0}%"), on, 0.0) {
            s.pip_roundness = pct.round() / 100.0;
        }
    });
    w::field(ui, "Border", LABEL_W, |ui| {
        // The color swatch shares the row.
        let mut b = s.pip_border;
        if slider(ui, &mut b, 0.0..=20.0, &|v| format!("{v:.0} px"), true, 30.0) {
            s.pip_border = b.round();
        }
        ui.add_enabled_ui(s.pip_border > 0.0, |ui| {
            ui.spacing_mut().interact_size = vec2(22.0, 18.0);
            ui.color_edit_button_srgb(&mut s.pip_border_color);
        });
    });
    w::field(ui, "Position", LABEL_W, |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (c, label, tip) in [
            (Corner::TopLeft, "↖", "Top left"),
            (Corner::TopRight, "↗", "Top right"),
            (Corner::BottomLeft, "↙", "Bottom left"),
            (Corner::BottomRight, "↘", "Bottom right"),
        ] {
            let sel = s.pip_place == Place::Corner(c);
            if ui.add(egui::Button::selectable(sel, label).min_size(vec2(28.0, 24.0)).corner_radius(6)).on_hover_text(tip).clicked() {
                s.pip_place = Place::Corner(c);
            }
        }
        if matches!(s.pip_place, Place::At(_)) {
            ui.label(RichText::new("custom").size(SECONDARY).color(p.secondary));
        }
    });
    w::divider(ui);
    w::row(ui, "Shadow", |ui| w::switch(ui, &mut s.pip_shadow, true));
    w::row(ui, "Mirror", |ui| w::switch(ui, &mut s.mirror, true));
    w::divider(ui);
    w::hint(ui, "Drag the camera in the preview to move it; it snaps to corners.");
    ui.add_space(2.0);
}

fn controls(ui: &mut Ui, st: &mut State) {
    let p = theme::pal(ui);
    let recording = st.recording.is_some();
    ui.horizontal_centered(|ui| {
        // Record / stop: a round button like a camera's.
        let (rect, resp) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::click());
        let c = rect.center();
        let painter = ui.painter();
        painter.circle_stroke(c, 21.0, Stroke::new(3.0, if resp.hovered() { p.text } else { p.secondary }));
        if st.finishing {
            painter.circle_filled(c, 15.0, p.tertiary);
        } else if recording {
            painter.rect_filled(egui::Rect::from_center_size(c, vec2(18.0, 18.0)), CornerRadius::same(4), RED);
        } else {
            painter.circle_filled(c, 16.0, RED);
        }
        let key = st.settings.hotkey(Action::Record).map(|c| format!(" ({})", c.label())).unwrap_or_default();
        let tip = format!("{} recording{key}", if recording { "Stop" } else { "Start" });
        if resp.on_hover_text(tip).clicked() && !st.finishing {
            st.cmds.push(Cmd::ToggleRecording);
        }
        ui.add_space(6.0);
        let paused = st.is_paused();
        if ui
            .add_enabled(recording, egui::Button::new(if paused { "Resume" } else { "Pause" }).min_size(vec2(72.0, 26.0)))
            .clicked()
        {
            st.cmds.push(Cmd::TogglePause);
        }
        ui.add_space(10.0);
        let clock = if recording { st.clock() } else { "00:00".into() };
        let color = if paused { p.secondary } else if recording { RED } else { p.tertiary };
        ui.label(RichText::new(clock).font(egui::FontId::monospace(22.0)).color(color));
        ui.add_space(16.0);
        ui.vertical(|ui| {
            ui.add_space(4.0);
            meter(ui, "System", st.meters[0], st.settings.system_audio);
            meter(ui, "Mic", st.meters[1], st.settings.mic_enabled);
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("Open Folder").clicked() {
                st.cmds.push(Cmd::OpenFolder);
            }
            if st.last_file.is_some() && ui.button("Reveal Last").clicked() {
                st.cmds.push(Cmd::RevealLast);
            }
        });
    });
}

/// Horizontal peak meter, -60..0 dBFS, green/yellow/red with a peak tick.
fn meter(ui: &mut Ui, label: &str, m: Meter, on: bool) {
    let p = theme::pal(ui);
    ui.horizontal(|ui| {
        ui.add_sized(vec2(44.0, 14.0), egui::Label::new(RichText::new(label).size(SECONDARY).color(p.secondary)));
        let (rect, _) = ui.allocate_exact_size(vec2(220.0, 8.0), Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(3), p.control);
        if !on {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, "off", egui::FontId::proportional(9.0), p.tertiary);
            return;
        }
        let frac = |db: f32| ((db + 60.0) / 60.0).clamp(0.0, 1.0);
        let x = |f: f32| rect.left() + rect.width() * f;
        let level = frac(m.db);
        for (lo, hi, col) in [
            (0.0, frac(-18.0), Color32::from_rgb(0x34, 0xc7, 0x59)),
            (frac(-18.0), frac(-6.0), Color32::from_rgb(0xff, 0xcc, 0x00)),
            (frac(-6.0), 1.0, RED),
        ] {
            let hi = hi.min(level);
            if hi > lo {
                let r = egui::Rect::from_x_y_ranges(x(lo)..=x(hi), rect.y_range());
                painter.rect_filled(r, CornerRadius::same(2), col);
            }
        }
        if m.peak_db > -60.0 {
            let px = x(frac(m.peak_db));
            painter.vline(px, rect.y_range(), Stroke::new(2.0, p.text));
        }
        ui.label(
            RichText::new(if m.db <= -89.0 { "-∞".to_string() } else { format!("{:.0}", m.db) })
                .size(SECONDARY)
                .color(p.secondary),
        );
    });
}

/// Mic choices; a saved mic that isn't connected stays listed as such.
fn mic_options(st: &State, s: &Settings) -> Vec<(Option<String>, String)> {
    let mut mics: Vec<(Option<String>, String)> = vec![(None, "Default microphone".into())];
    mics.extend(st.mics.iter().map(|d| (Some(d.id.clone()), d.name.clone())));
    if let Some(id) = &s.mic
        && !st.mics.iter().any(|d| &d.id == id)
    {
        let name = s.mic_name.clone().unwrap_or_else(|| id.clone());
        mics.push((Some(id.clone()), format!("{name} (not connected)")));
    }
    mics
}

/// Display name of the chosen mic (None for the system default).
fn mic_label(mics: &[(Option<String>, String)], id: &Option<String>) -> Option<String> {
    id.as_ref().and(mics.iter().find(|(v, _)| v == id).map(|(_, l)| l.trim_end_matches(" (not connected)").to_string()))
}

fn combo<T: Clone + PartialEq>(ui: &mut Ui, id: &str, value: &mut T, options: &[(T, String)], enabled: bool) -> bool {
    let current = options.iter().find(|(v, _)| v == value).map(|(_, l)| l.clone()).unwrap_or_default();
    let mut changed = false;
    // An explicit full-width slot: laid out bare, a combo with a short
    // label can come out narrower than the row.
    let full = ui.available_width();
    ui.allocate_ui_with_layout(vec2(full, 24.0), Layout::top_down(Align::Min), |ui| {
        ui.add_enabled_ui(enabled, |ui| {
            let resp = egui::ComboBox::from_id_salt(id)
                .width(full)
                .truncate()
                .icon(w::popup_chevrons)
                .selected_text(current.clone())
                .show_ui(ui, |ui| {
                    for (v, label) in options {
                        if ui.selectable_label(v == value, label).clicked() && v != value {
                            *value = v.clone();
                            changed = true;
                        }
                    }
                });
            // Long names are cut to fit; show the whole one on hover.
            resp.response.on_hover_text(current);
        });
    });
    changed
}

fn sources(ui: &mut Ui, st: &mut State) {
    let p = theme::pal(ui);
    let recording = st.recording.is_some() || st.finishing;
    let idle = !recording;
    let mut s = st.settings.clone();

    w::header(ui, Icon::Scene, "Scene");
    w::card(ui, |ui| {
        if super::scenes::gallery(ui, st, &mut s, idle)
            && idle
            && s.scene_id().1.is_none()
            && let Some(c) = s.scene.suggested_canvas()
            && s.canvas == Canvas::Source
        {
            // A template made for another frame shape picks it.
            s.canvas = c;
        }
        if s.scene_id().1.is_none() {
            w::hint(ui, s.scene.description());
        }
        w::divider(ui);
        w::row(ui, "Frame", |ui| {
            let opts: Vec<(Canvas, String)> = Canvas::ALL.iter().map(|c| (*c, c.label().to_string())).collect();
            ui.allocate_ui(vec2(120.0, 24.0), |ui| combo(ui, "canvas", &mut s.canvas, &opts, idle));
        });
    });

    w::header(ui, Icon::Display, "Screen");
    w::card(ui, |ui| {
        let displays: Vec<(usize, String)> = st.displays.iter().enumerate().map(|(i, d)| (i + 1, d.clone())).collect();
        combo(ui, "display", &mut s.display, &displays, idle);
        // What to record on the display: all of it, one app's windows, or
        // one window.
        #[derive(Clone, PartialEq)]
        enum Pick {
            Display,
            App(String),
            Window(u32),
        }
        let mut targets: Vec<(Pick, String)> = vec![(Pick::Display, "Entire display".into())];
        targets.extend(st.apps.iter().map(|(id, name)| (Pick::App(id.clone()), format!("App: {name} (all windows)"))));
        targets.extend(st.windows.iter().map(|(id, l)| (Pick::Window(*id), l.clone())));
        if let Some(a) = &s.app
            && !st.apps.iter().any(|(id, _)| id == a)
        {
            targets.push((Pick::App(a.clone()), format!("App: {a} (not running)")));
        }
        let mut pick = match (&s.window, &s.app) {
            (Some(w), _) => Pick::Window(*w),
            (None, Some(a)) => Pick::App(a.clone()),
            _ => Pick::Display,
        };
        ui.horizontal(|ui| {
            let bw = 26.0;
            ui.allocate_ui(vec2(ui.available_width() - bw - ui.spacing().item_spacing.x, 24.0), |ui| {
                if combo(ui, "window", &mut pick, &targets, idle) {
                    (s.window, s.app) = match &pick {
                        Pick::Display => (None, None),
                        Pick::App(a) => (None, Some(a.clone())),
                        Pick::Window(w) => (Some(*w), None),
                    };
                }
            });
            // Tight padding so the button is exactly `bw` wide: a wider row
            // would widen every card below it.
            ui.spacing_mut().button_padding.x = 4.0;
            if ui.add_enabled(idle, egui::Button::new("↻").min_size(vec2(bw, 24.0))).on_hover_text("Refresh windows and devices").clicked() {
                st.refresh_devices = true;
            }
        });
        if s.window.is_none() && s.app.is_none() {
            w::row(ui, "Area", |ui| {
                if st.region_select.is_some() {
                    if ui.button("Cancel").clicked()
                        && let Some((before, _)) = st.region_select.take()
                    {
                        s.region = before;
                    }
                    ui.label(RichText::new("Drag in the preview").size(SECONDARY).color(p.secondary));
                } else {
                    if s.region.is_some() && ui.add_enabled(idle, egui::Button::new("Whole")).on_hover_text("Record the entire display").clicked() {
                        s.region = None;
                    }
                    if ui.add_enabled(idle, egui::Button::new(if s.region.is_some() { "Change…" } else { "Select…" })).on_hover_text("Record part of the display: drag over it in the preview").clicked() {
                        st.region_select = Some((s.region, None));
                        s.region = None;
                    }
                    let text = match s.region {
                        Some([_, _, w, h]) => format!("{w:.0} × {h:.0} pt"),
                        None => "Entire display".into(),
                    };
                    ui.label(RichText::new(text).size(SECONDARY).color(p.secondary));
                }
            });
        }
    });

    w::header(ui, Icon::Camera, "Camera");
    w::card(ui, |ui| {
        let mut cams: Vec<(Option<String>, String)> = vec![(None, "Default camera".into())];
        cams.extend(st.cameras.iter().map(|d| (Some(d.id.clone()), d.name.clone())));
        let uses_cam = s.scene_spec().uses_cam();
        combo(ui, "camera", &mut s.camera, &cams, idle && uses_cam);
        if uses_cam {
            w::row(ui, "Resolution", |ui| {
                let opts: Vec<(crate::camera::CamRes, String)> = crate::camera::CamRes::ALL.iter().map(|r| (*r, r.label().to_string())).collect();
                ui.allocate_ui(vec2(90.0, 24.0), |ui| combo(ui, "cam-res", &mut s.camera_resolution, &opts, idle));
            });
            w::row(ui, "Save camera separately", |ui| w::switch(ui, &mut s.camera_file, idle))
                .on_hover_text("Also write the webcam alone to …-camera.mov, on the recording's timeline");
        }
        if !uses_cam {
            w::hint(ui, "Pick a camera scene to use the webcam.");
        }
    });
    if s.scene_id().1.is_some() {
        w::header(ui, Icon::Overlay, "Layers");
        w::card(ui, |ui| super::scenes::layers_panel(ui, st, &mut s, idle));
    } else if s.scene.has_pip() {
        w::header(ui, Icon::Overlay, "Camera Overlay");
        w::card(ui, |ui| overlay_controls(ui, &mut s));
    }

    w::header(ui, Icon::Audio, "Audio");
    w::card(ui, |ui| {
        w::row(ui, "System audio", |ui| w::switch(ui, &mut s.system_audio, idle));
        w::divider(ui);
        w::row(ui, "Microphone", |ui| w::switch(ui, &mut s.mic_enabled, idle));
        let mics = mic_options(st, &s);
        if combo(ui, "mic", &mut s.mic, &mics, idle && s.mic_enabled) {
            s.mic_name = mic_label(&mics, &s.mic);
        }
        if s.mic_enabled {
            // Live, also while recording.
            w::divider(ui);
            let f = &mut s.mic_filters;
            w::row(ui, "Noise suppression", |ui| w::switch(ui, &mut f.denoise, true)).on_hover_text("Removes fans, hum and keyboard noise (RNNoise)");
            w::row(ui, "Noise gate", |ui| w::switch(ui, &mut f.gate, true)).on_hover_text("Silences the mic between words");
            w::row(ui, "Compressor", |ui| w::switch(ui, &mut f.compressor, true)).on_hover_text("Evens out loud and quiet speech");
            w::row(ui, "Low cut (80 Hz)", |ui| w::switch(ui, &mut f.low_cut, true)).on_hover_text("Removes rumble and desk knocks");
        }
    });

    w::header(ui, Icon::Output, "Output");
    w::card(ui, |ui| {
        let codec = s.codec.map_or("Auto".to_string(), |c| format!("{c:?}").replace("H264", "H.264").replace("Hevc", "HEVC").replace("Prores", "ProRes 422"));
        let value = |ui: &mut Ui, text: String| {
            ui.add(egui::Label::new(RichText::new(&text).color(p.secondary)).truncate()).on_hover_text(text);
        };
        w::row(ui, "Format", |ui| value(ui, format!("{} · {} fps", s.resolution.to_uppercase(), s.fps)));
        w::divider(ui);
        w::row(ui, "Codec", |ui| value(ui, format!("{codec} · .{}", s.container)));
        w::divider(ui);
        w::row(ui, "Folder", |ui| value(ui, tilde(&s.output_dir)));
    });

    live_controls(ui, st, &mut s);

    if s != st.settings {
        st.settings = s;
        st.apply_scene();
        let _ = st.settings.save();
    }
}

fn bytes_text(b: usize) -> String {
    if b >= 1 << 30 { format!("{:.1} GB", b as f64 / (1u64 << 30) as f64) } else { format!("{:.0} MB", b as f64 / (1u64 << 20) as f64) }
}

pub const STREAM_DESTS: [(&str, &str); 3] = [("youtube", "YouTube"), ("twitch", "Twitch"), ("custom", "Custom (RTMP, SRT or WHIP URL)")];

/// Replay buffer and streaming: both use the shared live encode and run
/// with or without a recording.
fn live_controls(ui: &mut Ui, st: &mut State, s: &mut Settings) {
    let p = theme::pal(ui);
    if s.hdr {
        w::header(ui, Icon::Stream, "Replay & Stream");
        w::card(ui, |ui| w::hint(ui, "Off while recording HDR: the live encode is SDR. Turn HDR off in Settings to use them."));
        return;
    }
    w::header(ui, Icon::Replay, "Replay Buffer");
    w::card(ui, |ui| {
        w::row(ui, "Keep recent footage", |ui| w::switch(ui, &mut s.replay_enabled, true));
        w::field(ui, "Length", 72.0, |ui| {
            ui.spacing_mut().slider_width = (ui.available_width() - 48.0).max(40.0);
            ui.add(egui::Slider::new(&mut s.replay_secs, 15..=300).step_by(5.0).show_value(false));
            w::slot(ui, 40.0, Align::Max, RichText::new(format!("{} s", s.replay_secs)).size(SECONDARY + 1.0).color(p.secondary));
        });
        if let Some((held, bytes)) = st.replay_stats {
            w::divider(ui);
            ui.horizontal(|ui| {
                let tip = match st.settings.hotkey(Action::SaveReplay) {
                    Some(c) if st.hotkey_failed.contains(&Action::SaveReplay) => format!("Save the buffer to an MP4 without stopping ({} unavailable)", c.label()),
                    Some(c) => format!("Save the buffer to an MP4 without stopping ({})", c.label()),
                    None => "Save the buffer to an MP4 without stopping".to_string(),
                };
                if ui.add_enabled(held > 0.0, egui::Button::new("Save Replay")).on_hover_text(tip).clicked() {
                    st.cmds.push(Cmd::SaveReplay);
                }
                ui.label(RichText::new(format!("{held:.0} s · {} in memory", bytes_text(bytes))).size(SECONDARY).color(p.secondary));
            });
        }
    });

    w::header(ui, Icon::Stream, "Stream");
    w::card(ui, |ui| {
        let live = st.streaming();
        let mut first = (s.stream_dest.clone(), s.stream_custom_url.clone());
        destination(ui, st, 0, &mut first.0, &mut first.1, live);
        (s.stream_dest, s.stream_custom_url) = first;
        let mut remove = None;
        for i in 0..s.stream_extra.len() {
            w::divider(ui);
            let d = &mut s.stream_extra[i];
            ui.horizontal(|ui| {
                w::switch(ui, &mut d.enabled, !live).on_hover_text("Stream here too");
                ui.label(RichText::new(format!("Also stream to ({})", i + 2)).size(SECONDARY).color(p.secondary));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add_enabled(!live, egui::Button::new("✕").small()).on_hover_text("Remove this destination").clicked() {
                        remove = Some(i);
                    }
                });
            });
            if d.enabled {
                destination(ui, st, i + 1, &mut d.dest, &mut d.custom_url, live);
            }
        }
        if let Some(i) = remove {
            s.stream_extra.remove(i);
            if st.stream_key_input.len() > i + 1 {
                st.stream_key_input.remove(i + 1);
            }
        }
        if !live && ui.small_button("+ Add destination").on_hover_text("Stream to several places at once from one encode").clicked() {
            s.stream_extra.push(Default::default());
        }
        w::divider(ui);
        let urls = s.stream_urls();
        let ready = !urls.is_empty() && urls.iter().all(|u| u.starts_with("http") || u.starts_with("srt://") || st.has_stream_key(u));
        let label = match (live, urls.len()) {
            (true, _) => "Stop Stream".to_string(),
            (false, n) if n > 1 => format!("Go Live ({n} destinations)"),
            _ => "Go Live".to_string(),
        };
        let (fill, fg) = if live {
            (RED, Color32::WHITE)
        } else if ready {
            (theme::accent(), Color32::WHITE)
        } else {
            (p.control, p.secondary)
        };
        // Growable atoms on both sides center the label.
        let b = egui::Button::new((egui::Atom::grow(), RichText::new(label).font(theme::semibold(BODY)).color(fg), egui::Atom::grow())).fill(fill).corner_radius(7).min_size(vec2(ui.available_width(), 28.0));
        let resp = ui.add_enabled(live || ready, b);
        let resp = if ready || live { resp } else { resp.on_disabled_hover_text("Save a stream key for every destination first") };
        if resp.clicked() {
            st.cmds.push(Cmd::ToggleStream);
        }
        for status in &st.stream_statuses {
            let text = if status.live {
                format!("{}  {}  {:.1} Mb/s  {} dropped", status.name, crate::record::clock(status.duration), status.bitrate / 1e6, status.dropped_frames)
            } else {
                format!("{}  {}", status.name, status.state)
            };
            ui.add(egui::Label::new(RichText::new(text).size(SECONDARY).color(if status.live { p.text } else { p.secondary })).truncate());
            if let Some(e) = (!status.live).then_some(status.error.as_ref()).flatten() {
                ui.add(egui::Label::new(RichText::new(e).size(SECONDARY).color(RED)).wrap());
            }
        }
    });
}

/// One stream destination: service, URL (custom) and its key in the
/// Keychain (per URL, never in the settings file).
fn destination(ui: &mut Ui, st: &mut State, i: usize, dest: &mut String, custom_url: &mut String, live: bool) {
    let p = theme::pal(ui);
    let dests: Vec<(String, String)> = STREAM_DESTS.iter().map(|(v, l)| (v.to_string(), l.to_string())).collect();
    combo(ui, &format!("stream-dest-{i}"), dest, &dests, !live);
    if dest == "custom" {
        ui.add_enabled(
            !live,
            egui::TextEdit::singleline(custom_url)
                .hint_text("rtmp://…, srt://… or https://…/whip")
                .margin(vec2(8.0, 4.0))
                .desired_width(f32::INFINITY),
        );
    }
    let url = crate::settings::dest_url(dest, custom_url);
    let has_key = st.has_stream_key(&url);
    if has_key && st.key_input(i).is_empty() {
        w::row(ui, "Stream key", |ui| {
            if ui.add_enabled(!live, egui::Button::new("Remove")).clicked() {
                st.delete_stream_key(&url);
            }
            ui.label(RichText::new("In Keychain").size(SECONDARY).color(p.secondary));
        });
    } else {
        ui.horizontal(|ui| {
            let bw = 52.0;
            let hint = if url.starts_with("srt") {
                "Stream ID (optional)"
            } else if url.starts_with("http") {
                "Bearer token (optional)"
            } else {
                "Stream key"
            };
            ui.add_sized(
                vec2(ui.available_width() - bw - ui.spacing().item_spacing.x, 24.0),
                egui::TextEdit::singleline(st.key_input(i)).password(true).margin(vec2(8.0, 4.0)).hint_text(hint),
            );
            if ui.add_enabled(!st.key_input(i).trim().is_empty(), egui::Button::new("Save").min_size(vec2(bw, 24.0))).clicked() {
                st.save_stream_key(i, &url);
            }
        });
    }
}

pub fn settings_window(ui: &mut Ui, st: &mut State) {
    let p = theme::pal(ui);
    let idle = st.recording.is_none() && !st.finishing;
    egui::Panel::top("settings-toolbar")
        .exact_size(TOOLBAR_H)
        .frame(egui::Frame::NONE.fill(p.window).inner_margin(Margin::symmetric(12, 0)))
        .show(ui, |ui| {
            toolbar(ui, |ui| {
                ui.label(RichText::new("Settings").font(theme::semibold(BODY)));
            });
        });
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(p.window).inner_margin(Margin::symmetric(20, 12)))
        .show(ui, |ui| {
            if !idle {
                ui.label(RichText::new("Settings are locked while recording.").color(p.secondary));
            }
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                ui.add_enabled_ui(idle, |ui| settings_form(ui, st));
                // The overlay stays editable while recording (applied live).
                section(ui, "CAMERA OVERLAY");
                let mut s = st.settings.clone();
                overlay_controls(ui, &mut s);
                if s != st.settings {
                    st.settings = s;
                    st.apply_pip();
                    let _ = st.settings.save();
                }
            });
        });
}

/// Global shortcuts: click one, then press the keys (Esc cancels, ⌫ clears).
fn shortcuts(ui: &mut Ui, st: &mut State, s: &mut Settings) {
    let p = theme::pal(ui);
    if let Some(a) = st.capturing_shortcut {
        let events = ui.input(|i| i.events.clone());
        for e in events {
            let egui::Event::Key { key, physical_key, pressed: true, modifiers, .. } = e else { continue };
            let k = physical_key.unwrap_or(key);
            let bare = !(modifiers.ctrl || modifiers.alt || modifiers.mac_cmd || modifiers.command || modifiers.shift);
            if k == egui::Key::Escape && bare {
                st.capturing_shortcut = None;
                break;
            }
            if matches!(k, egui::Key::Backspace | egui::Key::Delete) && bare {
                s.set_hotkey(a, None);
                st.capturing_shortcut = None;
                break;
            }
            if let Some(c) = Combo::from_key(k.name(), modifiers.ctrl, modifiers.alt, modifiers.shift, modifiers.mac_cmd || modifiers.command) {
                s.set_hotkey(a, Some(c));
                st.capturing_shortcut = None;
                break;
            }
        }
    }
    egui::Grid::new("shortcuts").num_columns(2).spacing(vec2(12.0, 6.0)).min_col_width(110.0).show(ui, |ui| {
        for a in Action::ALL {
            ui.label(a.label());
            ui.horizontal(|ui| {
                let capturing = st.capturing_shortcut == Some(a);
                let text = if capturing {
                    "Press keys…".to_string()
                } else {
                    s.hotkey(a).map(|c| c.label()).unwrap_or_else(|| "None".into())
                };
                let b = egui::Button::selectable(capturing, RichText::new(text).monospace()).min_size(vec2(110.0, 22.0));
                if ui.add(b).on_hover_text("Click, then press the shortcut (Esc cancels, ⌫ clears)").clicked() {
                    st.capturing_shortcut = if capturing { None } else { Some(a) };
                }
                if s.hotkey(a).is_some() && ui.small_button("✕").on_hover_text("No shortcut").clicked() {
                    s.set_hotkey(a, None);
                }
                if st.hotkey_failed.contains(&a) {
                    ui.label(RichText::new("in use by another app").size(SECONDARY).color(RED));
                } else if a == Action::SaveReplay && s.hotkey(a).is_some() {
                    ui.label(RichText::new("while the replay buffer is on").size(SECONDARY).color(p.secondary));
                }
            });
            ui.end_row();
        }
    });
    if s.hotkeys.values().any(|_| true) && ui.small_button("Restore default shortcuts").clicked() {
        s.hotkeys.clear();
    }
}

fn settings_form(ui: &mut Ui, st: &mut State) {
    let mut s = st.settings.clone();
    let grid = |ui: &mut Ui, id: &str, add: &mut dyn FnMut(&mut Ui)| {
        egui::Grid::new(id).num_columns(2).spacing(vec2(12.0, 8.0)).min_col_width(110.0).show(ui, |ui| add(ui));
    };

    section(ui, "OUTPUT");
    grid(ui, "out", &mut |ui| {
        ui.label("Folder");
        ui.horizontal(|ui| {
            ui.label(RichText::new(tilde(&s.output_dir)).size(SECONDARY));
            if ui.button("Choose…").clicked()
                && let Some(d) = rfd::FileDialog::new().set_directory(&s.output_dir).pick_folder()
            {
                s.output_dir = d;
            }
        });
        ui.end_row();
        ui.label("Format");
        let opts = [("mov".to_string(), "QuickTime (.mov)".to_string()), ("mp4".to_string(), "MPEG-4 (.mp4)".to_string())];
        combo(ui, "container", &mut s.container, &opts, s.codec != Some(Codec::Prores));
        ui.end_row();
    });

    section(ui, "VIDEO");
    grid(ui, "video", &mut |ui| {
        ui.label("Resolution");
        let opts: Vec<(String, String)> = RESOLUTIONS.iter().map(|(l, v)| (v.to_string(), l.to_string())).collect();
        combo(ui, "res", &mut s.resolution, &opts, true);
        ui.end_row();
        ui.label("Frame rate");
        let opts: Vec<(u32, String)> = FPS.iter().map(|f| (*f, format!("{f} fps"))).collect();
        combo(ui, "fps", &mut s.fps, &opts, true);
        ui.end_row();
        ui.label("Codec");
        let opts = vec![
            (None, "Auto (H.264 ≤1080p, HEVC above)".to_string()),
            (Some(Codec::H264), "H.264".to_string()),
            (Some(Codec::Hevc), "HEVC (H.265)".to_string()),
            (Some(Codec::Prores), "ProRes 422 (.mov)".to_string()),
        ];
        combo(ui, "codec", &mut s.codec, &opts, true);
        ui.end_row();
        ui.label("HDR");
        ui.vertical(|ui| {
            ui.add_enabled(s.codec != Some(Codec::H264), egui::Checkbox::new(&mut s.hdr, "Record HDR (experimental)"))
                .on_hover_text("10-bit HEVC or ProRes, BT.2020 PQ, from ScreenCaptureKit's HDR capture");
            if s.hdr {
                ui.label(RichText::new("HDR is for recordings: the replay buffer and streams are off while it is on.").size(SECONDARY).color(theme::pal(ui).secondary));
            }
        });
        ui.end_row();
        ui.label("Bitrate");
        ui.horizontal(|ui| {
            let mut auto = s.bitrate_mbps.is_none();
            if ui.add_enabled(s.codec != Some(Codec::Prores), egui::Checkbox::new(&mut auto, "Auto")).changed() {
                s.bitrate_mbps = if auto { None } else { Some(30.0) };
            }
            if let Some(b) = &mut s.bitrate_mbps {
                ui.add(egui::Slider::new(b, 2.0..=200.0).suffix(" Mb/s").logarithmic(true).integer());
            }
        });
        ui.end_row();
        ui.label("Cursor");
        ui.horizontal(|ui| {
            ui.checkbox(&mut s.cursor, "Show");
            ui.add_enabled(s.cursor, egui::Checkbox::new(&mut s.clicks, "Highlight clicks"));
        });
        ui.end_row();
        ui.label("Preview");
        ui.checkbox(&mut s.preview_low_power, "Low power (half resolution, 15 fps)");
        ui.end_row();
    });

    section(ui, "SCENES");
    grid(ui, "scenes", &mut |ui| {
        ui.label("Frame shape");
        let opts: Vec<(Canvas, String)> = Canvas::ALL
            .iter()
            .map(|c| (*c, match c {
                Canvas::Source => "The screen's shape (no bars)".to_string(),
                Canvas::Landscape => "16:9 (YouTube)".to_string(),
                Canvas::Portrait => "9:16 (Shorts, Reels, TikTok)".to_string(),
                Canvas::Square => "1:1 (square)".to_string(),
                Canvas::Tall => "4:5 (Instagram feed)".to_string(),
            }))
            .collect();
        combo(ui, "canvas2", &mut s.canvas, &opts, true);
        ui.end_row();
        ui.label("Background");
        ui.horizontal(|ui| {
            ui.color_edit_button_srgb(&mut s.background.top);
            ui.color_edit_button_srgb(&mut s.background.bottom);
            ui.label(RichText::new("top, bottom").size(SECONDARY).color(theme::pal(ui).secondary));
            if ui.small_button("Reset").clicked() {
                s.background = Background::default();
            }
        });
        ui.end_row();
        ui.label("Transition");
        ui.horizontal(|ui| {
            let opts: Vec<(Transition, String)> = Transition::ALL.iter().map(|t| (*t, t.label().to_string())).collect();
            ui.allocate_ui(vec2(90.0, 24.0), |ui| combo(ui, "transition", &mut s.transition, &opts, true));
            ui.add_enabled(s.transition != Transition::Cut, egui::Slider::new(&mut s.transition_ms, 100..=1500).suffix(" ms").step_by(50.0));
        });
        ui.end_row();
    });

    section(ui, "AUDIO");
    grid(ui, "audio", &mut |ui| {
        ui.label("System audio");
        ui.checkbox(&mut s.system_audio, "Record");
        ui.end_row();
        ui.label("Microphone");
        let mics = mic_options(st, &s);
        ui.horizontal(|ui| {
            ui.checkbox(&mut s.mic_enabled, "");
            if combo(ui, "mic2", &mut s.mic, &mics, s.mic_enabled) {
                s.mic_name = mic_label(&mics, &s.mic);
            }
        });
        ui.end_row();
        ui.label("Tracks");
        let opts = vec![
            (AudioLayout::Both, "Mix + separate system & mic".to_string()),
            (AudioLayout::Mix, "Mix only".to_string()),
            (AudioLayout::Stems, "Separate only".to_string()),
        ];
        combo(ui, "tracks", &mut s.audio_tracks, &opts, true);
        ui.end_row();
        ui.label("System gain");
        ui.add(egui::Slider::new(&mut s.system_gain_db, -24.0..=12.0).suffix(" dB").step_by(0.5));
        ui.end_row();
        ui.label("Mic gain");
        ui.add(egui::Slider::new(&mut s.mic_gain_db, -24.0..=24.0).suffix(" dB").step_by(0.5));
        ui.end_row();
        let f = &mut s.mic_filters;
        ui.label("Mic filters");
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut f.low_cut, "Low cut");
            ui.checkbox(&mut f.denoise, "Noise suppression");
            ui.checkbox(&mut f.gate, "Gate");
            ui.checkbox(&mut f.compressor, "Compressor");
        });
        ui.end_row();
        ui.label("Gate opens at");
        ui.add_enabled(f.gate, egui::Slider::new(&mut f.gate_db, -70.0..=-20.0).suffix(" dBFS").step_by(1.0));
        ui.end_row();
        ui.label("Compressor");
        ui.add_enabled_ui(f.compressor, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::Slider::new(&mut f.comp_threshold_db, -40.0..=-5.0).suffix(" dB").step_by(1.0).text("threshold"));
                ui.add(egui::Slider::new(&mut f.comp_ratio, 1.5..=10.0).step_by(0.5).text(": 1"));
                ui.add(egui::Slider::new(&mut f.comp_makeup_db, 0.0..=18.0).suffix(" dB").step_by(1.0).text("makeup"));
            });
        });
        ui.end_row();
    });

    section(ui, "LIVE ENCODE (REPLAY & STREAMING)");
    let streaming = st.streaming();
    ui.add_enabled_ui(!streaming, |ui| {
        grid(ui, "live", &mut |ui| {
            ui.label("Resolution");
            let opts: Vec<(String, String)> =
                [("720p", "720p"), ("1080p", "1080p"), ("1440p", "1440p"), ("4k", "4K")].iter().map(|(v, l)| (v.to_string(), l.to_string())).collect();
            combo(ui, "live-res", &mut s.live_resolution, &opts, true);
            ui.end_row();
            ui.label("Codec");
            let opts: Vec<(String, String)> = [("auto", "Auto (HEVC for YouTube, else H.264)"), ("h264", "H.264"), ("hevc", "HEVC (falls back to H.264)")]
                .iter()
                .map(|(v, l)| (v.to_string(), l.to_string()))
                .collect();
            combo(ui, "live-codec", &mut s.stream_codec, &opts, true);
            ui.end_row();
            ui.label("Video bitrate");
            ui.horizontal(|ui| {
                ui.checkbox(&mut s.stream_bitrate_auto, "Platform preset");
                if !s.stream_bitrate_auto {
                    ui.add(egui::Slider::new(&mut s.stream_bitrate_kbps, 1000..=60_000).suffix(" kb/s").step_by(500.0).logarithmic(true));
                }
            });
            ui.end_row();
            ui.label("");
            let c = s.live_config();
            ui.label(
                RichText::new(format!(
                    "Up to {}x{} {} at {:.1} Mb/s (the frame keeps its shape, never above the recording size)",
                    c.width,
                    c.height,
                    if c.hevc { "HEVC" } else { "H.264" },
                    c.bitrate as f64 / 1e6
                ))
                .size(SECONDARY)
                .color(theme::pal(ui).secondary),
            );
            ui.end_row();
            ui.label("Rate control");
            ui.checkbox(&mut s.stream_cbr, "Constant bitrate (CBR)");
            ui.end_row();
            ui.label("Keyframe every");
            ui.add(egui::Slider::new(&mut s.keyframe_secs, 1.0..=4.0).suffix(" s").step_by(0.5));
            ui.end_row();
            ui.label("Audio bitrate");
            let opts: Vec<(u32, String)> = [96, 128, 160, 192, 256, 320].iter().map(|k| (*k, format!("{k} kb/s"))).collect();
            combo(ui, "live-audio", &mut s.stream_audio_kbps, &opts, true);
            ui.end_row();
        });
    });
    if streaming {
        ui.label(RichText::new("Stop the stream to change the live encode.").size(SECONDARY).color(theme::pal(ui).secondary));
    }

    section(ui, "SHORTCUTS");
    shortcuts(ui, st, &mut s);

    section(ui, "CAMERA");
    grid(ui, "cam", &mut |ui| {
        ui.label("Camera");
        let mut cams: Vec<(Option<String>, String)> = vec![(None, "Default camera".into())];
        cams.extend(st.cameras.iter().map(|d| (Some(d.id.clone()), d.name.clone())));
        combo(ui, "cam2", &mut s.camera, &cams, true);
        ui.end_row();
    });
    ui.add_space(12.0);
    if ui.button("Restore Defaults").clicked() {
        s = crate::settings::Settings { output_dir: s.output_dir.clone(), ..Default::default() };
    }
    if s != st.settings {
        st.settings = s;
        let _ = st.settings.save();
    }
}
