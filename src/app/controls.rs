//! The recording controls: a small floating bar (stop and timer, mic mute,
//! pause, restart, delete, open SimpleRecord, and a grip to move it) shown while a
//! recording runs. It belongs to SimpleRecord, whose windows are left out of every
//! capture, so it never appears in the recording, the preview or a stream.

use std::time::{Duration, Instant};

use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};

use super::{Cmd, State, theme};

/// Room around the bar for its shadow (points).
pub const MARGIN: f32 = 10.0;
const BAR_H: f32 = 44.0;
const PAD: f32 = 7.0;
const BTN: f32 = 32.0;
const GAP: f32 = 2.0;
/// Stop button and timer.
const STOP_W: f32 = 30.0 + 6.0 + 62.0;
const GRIP_W: f32 = 20.0;
const BAR_W: f32 = PAD + STOP_W + 10.0 + 5.0 * BTN + 4.0 * GAP + 9.0 + 1.0 + 3.0 + GRIP_W + PAD;
/// The window: the bar and its margin.
pub const SIZE: (f64, f64) = ((BAR_W + 2.0 * MARGIN) as f64, (BAR_H + 2.0 * MARGIN) as f64);
/// How long a Restart or Delete waits for its confirmation.
const CONFIRM_FOR: Duration = Duration::from_secs(5);

const RED: Color32 = Color32::from_rgb(0xff, 0x3b, 0x30);
const ORANGE: Color32 = Color32::from_rgb(0xff, 0x95, 0x00);

/// A destructive action waiting for its second click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confirm {
    Restart,
    Delete,
}

#[derive(Clone, Copy)]
enum Glyph {
    Mic { muted: bool },
    Pause { paused: bool },
    Restart,
    Delete,
    Settings,
}

/// Draw the bar. Returns true when the grip (or the bar's empty space)
/// starts a drag, to move the window.
pub fn bar(ui: &mut Ui, st: &mut State) -> bool {
    let dark = ui.ctx().theme() == egui::Theme::Dark;
    let p = theme::palette(dark);
    let fg = if dark { Color32::from_rgb(0xe5, 0xe5, 0xe7) } else { Color32::from_rgb(0x3a, 0x3a, 0x3c) };
    let fill = if dark { Color32::from_rgb(0x2b, 0x2b, 0x2d) } else { Color32::WHITE };
    let hover = if dark { Color32::from_white_alpha(22) } else { Color32::from_black_alpha(14) };

    let rect = Rect::from_min_size(pos2(MARGIN, MARGIN), vec2(BAR_W, BAR_H));
    let radius = CornerRadius::same((BAR_H / 2.0) as u8);
    let shadow = egui::Shadow { offset: [0, 2], blur: 10, spread: 0, color: Color32::from_black_alpha(if dark { 90 } else { 45 }) };
    ui.painter().add(shadow.as_shape(rect, radius));
    ui.painter().rect(rect, radius, fill, Stroke::new(1.0, p.separator), egui::StrokeKind::Inside);

    // Empty space drags the window; the controls drawn after it take
    // their own clicks.
    let mut drag = ui.interact(rect, ui.id().with("bar"), Sense::click_and_drag()).drag_started();

    if st.controls_confirm.is_some_and(|(_, at)| at.elapsed() > CONFIRM_FOR) {
        st.controls_confirm = None;
    }
    let paused = st.is_paused();
    let mut x = rect.left() + PAD;
    let cy = rect.center().y;

    // Stop, with the recorded time.
    let stop = Rect::from_min_size(pos2(x, cy - 15.0), vec2(STOP_W, 30.0));
    let r = ui.interact(stop, ui.id().with("stop"), Sense::click());
    if r.hovered() {
        ui.painter().rect_filled(stop, CornerRadius::same(15), hover);
    }
    let accent = if paused { ORANGE } else { RED };
    let c = pos2(x + 15.0, cy);
    ui.painter().circle_stroke(c, 9.5, Stroke::new(1.8, accent));
    ui.painter().rect_filled(Rect::from_center_size(c, vec2(7.0, 7.0)), CornerRadius::same(2), accent);
    // Blink while paused.
    let blink = paused && (ui.input(|i| i.time) * 2.0) as u64 % 2 == 1;
    let time_color = if blink { accent.gamma_multiply(0.35) } else { accent };
    ui.painter().text(pos2(x + 36.0, cy), egui::Align2::LEFT_CENTER, st.clock(), theme::semibold(15.0), time_color);
    if r.clicked() {
        st.controls_confirm = None;
        st.cmds.push(Cmd::ToggleRecording);
    }
    x += STOP_W + 10.0;

    match st.controls_confirm {
        Some((what, _)) => {
            // The question replaces the buttons until answered.
            let area = Rect::from_min_max(pos2(x, rect.top()), pos2(rect.right() - PAD - GRIP_W - 13.0, rect.bottom()));
            let (question, yes) = match what {
                Confirm::Restart => ("Restart?", "Restart"),
                Confirm::Delete => ("Delete take?", "Delete"),
            };
            ui.painter().text(pos2(area.left(), cy), egui::Align2::LEFT_CENTER, question, theme::semibold(12.5), fg);
            let cancel = Rect::from_min_size(pos2(area.right() - 56.0, cy - 13.0), vec2(56.0, 26.0));
            let ok = Rect::from_min_size(pos2(cancel.left() - 64.0, cy - 13.0), vec2(60.0, 26.0));
            if text_button(ui, ok, "confirm-yes", yes, Color32::WHITE, RED, RED.gamma_multiply(0.85)) {
                st.controls_confirm = None;
                st.cmds.push(match what {
                    Confirm::Restart => Cmd::RestartRecording,
                    Confirm::Delete => Cmd::DeleteRecording,
                });
            }
            if text_button(ui, cancel, "confirm-no", "Cancel", fg, Color32::TRANSPARENT, hover) {
                st.controls_confirm = None;
            }
            x = area.right();
        }
        None => {
            let mic_on = st.settings.mic_enabled;
            let level = ((st.meters[1].db + 60.0) / 60.0).clamp(0.0, 1.0);
            let buttons = [
                (Glyph::Mic { muted: st.mic_muted }, mic_on),
                (Glyph::Pause { paused }, true),
                (Glyph::Restart, true),
                (Glyph::Delete, true),
                (Glyph::Settings, true),
            ];
            for (i, (g, enabled)) in buttons.into_iter().enumerate() {
                let b = Rect::from_min_size(pos2(x, cy - BTN / 2.0), vec2(BTN, BTN));
                let r = ui.interact(b, ui.id().with(("btn", i)), if enabled { Sense::click() } else { Sense::hover() });
                if r.hovered() && enabled {
                    ui.painter().circle_filled(b.center(), BTN / 2.0, hover);
                }
                let color = match g {
                    Glyph::Mic { muted: true } => RED,
                    _ if !enabled => fg.gamma_multiply(0.35),
                    _ => fg,
                };
                paint(ui, b, g, color);
                if let Glyph::Mic { muted: false } = g
                    && enabled
                {
                    // The mic's level, under it.
                    let track = Rect::from_center_size(pos2(b.center().x, b.bottom() - 3.0), vec2(14.0, 2.0));
                    ui.painter().rect_filled(track, CornerRadius::same(1), fg.gamma_multiply(0.15));
                    let mut lit = track;
                    lit.set_width(track.width() * level);
                    ui.painter().rect_filled(lit, CornerRadius::same(1), theme::MAC_BLUE);
                }
                if r.clicked() {
                    match g {
                        Glyph::Mic { .. } => st.mic_muted = !st.mic_muted,
                        Glyph::Pause { .. } => st.cmds.push(Cmd::TogglePause),
                        Glyph::Restart => st.controls_confirm = Some((Confirm::Restart, Instant::now())),
                        Glyph::Delete => st.controls_confirm = Some((Confirm::Delete, Instant::now())),
                        Glyph::Settings => st.cmds.push(Cmd::ShowWindow),
                    }
                }
                x += BTN + GAP;
            }
            x -= GAP;
        }
    }

    // Separator and grip.
    x += 9.0;
    ui.painter().vline(x, (cy - 13.0)..=(cy + 13.0), Stroke::new(1.0, p.separator));
    x += 4.0;
    let grip = Rect::from_min_size(pos2(x, cy - 15.0), vec2(GRIP_W, 30.0));
    let r = ui.interact(grip, ui.id().with("grip"), Sense::click_and_drag());
    if r.hovered() {
        ui.painter().rect_filled(grip, CornerRadius::same(6), hover);
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    for dy in [-5.0, 0.0, 5.0] {
        ui.painter().circle_filled(pos2(grip.center().x, cy + dy), 1.6, fg);
    }
    drag |= r.drag_started();
    drag
}

fn text_button(ui: &mut Ui, rect: Rect, id: &str, label: &str, fg: Color32, fill: Color32, hover: Color32) -> bool {
    let r = ui.interact(rect, ui.id().with(id), Sense::click());
    ui.painter().rect_filled(rect, CornerRadius::same(13), if r.hovered() { hover } else { fill });
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, label, theme::semibold(12.0), fg);
    r.clicked()
}

/// Line icons, 18 pt, centered in `rect`.
fn paint(ui: &Ui, rect: Rect, g: Glyph, color: Color32) {
    let painter = ui.painter();
    let s = Stroke::new(1.6, color);
    let c = rect.center();
    let arc = |r: f32, from: f32, to: f32| -> Vec<Pos2> {
        (0..=24).map(|i| c + vec2((from + (to - from) * i as f32 / 24.0).cos(), (from + (to - from) * i as f32 / 24.0).sin()) * r).collect()
    };
    match g {
        Glyph::Mic { muted } => {
            let body = Rect::from_center_size(c + vec2(0.0, -3.0), vec2(6.0, 11.0));
            painter.rect_stroke(body, CornerRadius::same(3), s, egui::StrokeKind::Middle);
            // The cradle, the stem and the base.
            let pi = std::f32::consts::PI;
            let cradle: Vec<Pos2> = (0..=16)
                .map(|i| {
                    let a = pi * i as f32 / 16.0;
                    c + vec2(-a.cos() * 5.5, -1.0 + a.sin() * 5.0)
                })
                .collect();
            painter.add(Shape::line(cradle, s));
            painter.line_segment([c + vec2(0.0, 4.0), c + vec2(0.0, 7.0)], s);
            if muted {
                painter.line_segment([c + vec2(-6.5, -8.0), c + vec2(6.5, 7.0)], s);
            }
        }
        Glyph::Pause { paused } => {
            painter.circle_stroke(c, 8.5, s);
            if paused {
                // Resume: a play triangle.
                painter.add(Shape::convex_polygon(vec![c + vec2(-2.5, -4.0), c + vec2(4.0, 0.0), c + vec2(-2.5, 4.0)], color, Stroke::NONE));
            } else {
                painter.line_segment([c + vec2(-2.2, -3.5), c + vec2(-2.2, 3.5)], s);
                painter.line_segment([c + vec2(2.2, -3.5), c + vec2(2.2, 3.5)], s);
            }
        }
        Glyph::Restart => {
            // Counterclockwise arrow, open at the top right.
            let pts = arc(7.0, -1.0, 4.4);
            let end = pts[0];
            painter.add(Shape::line(pts, s));
            painter.line(vec![end + vec2(-3.6, -0.6), end, end + vec2(0.4, 3.4)], s);
        }
        Glyph::Delete => {
            painter.line_segment([c + vec2(-7.0, -5.0), c + vec2(7.0, -5.0)], s);
            painter.line(vec![c + vec2(-2.5, -5.0), c + vec2(-2.5, -7.5), c + vec2(2.5, -7.5), c + vec2(2.5, -5.0)], s);
            painter.add(Shape::line(vec![c + vec2(-5.5, -5.0), c + vec2(-4.5, 8.0), c + vec2(4.5, 8.0), c + vec2(5.5, -5.0)], s));
            painter.line_segment([c + vec2(-1.5, -1.5), c + vec2(-1.3, 4.5)], s);
            painter.line_segment([c + vec2(1.5, -1.5), c + vec2(1.3, 4.5)], s);
        }
        Glyph::Settings => {
            painter.circle_stroke(c, 2.6, s);
            painter.circle_stroke(c, 6.0, s);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                let d = vec2(a.cos(), a.sin());
                painter.line_segment([c + d * 6.0, c + d * 8.5], Stroke::new(2.4, color));
            }
        }
    }
}
