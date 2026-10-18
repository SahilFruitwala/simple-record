//! Sidebar building blocks in the style of macOS inspectors: section headers
//! with an icon, soft rounded cards, label/control rows, switches and a
//! segmented control. Spacing follows an 8 pt grid.

use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Color32, CornerRadius, Layout, Margin, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, pos2, vec2};

use super::theme::{self, Palette, SECONDARY};

/// Height of one row inside a card.
pub const ROW_H: f32 = 28.0;
/// Card corner radius and inner padding.
const CARD_R: u8 = 10;
const CARD_PAD: i8 = 12;

#[derive(Clone, Copy)]
pub enum Icon {
    Scene,
    Display,
    Camera,
    Overlay,
    Audio,
    Output,
    Replay,
    Stream,
}

/// Card fill and hairline border for the sidebar.
fn card_colors(p: &Palette, dark: bool) -> (Color32, Color32) {
    if dark {
        (theme::blend(p.sidebar, Color32::from_white_alpha(10)), Color32::from_white_alpha(14))
    } else {
        (Color32::WHITE, Color32::from_black_alpha(16))
    }
}

/// Section header: a small icon and an uppercase, letter-spaced label.
pub fn header(ui: &mut Ui, icon: Icon, title: &str) {
    let p = theme::pal(ui);
    ui.add_space(16.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.add_space(2.0);
        let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
        paint_icon(ui, rect, icon, p.secondary);
        let mut job = LayoutJob::default();
        job.append(
            &title.to_uppercase(),
            0.0,
            TextFormat { font_id: theme::semibold(10.5), color: p.secondary, extra_letter_spacing: 0.6, ..Default::default() },
        );
        ui.label(job);
    });
    ui.add_space(8.0);
}

/// A rounded card holding grouped controls.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = theme::pal(ui);
    let (fill, border) = card_colors(&p, ui.visuals().dark_mode);
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(CARD_R))
        .inner_margin(Margin::symmetric(CARD_PAD, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            add(ui)
        })
        .inner
}

/// Hairline between rows of a card, spanning the card's content width.
pub fn divider(ui: &mut Ui) {
    let p = theme::pal(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.center().y, Stroke::new(1.0, p.separator.gamma_multiply(0.7)));
}

/// A label on the left and right-aligned controls.
pub fn row<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = theme::pal(ui);
    let w = ui.available_width();
    ui.allocate_ui_with_layout(vec2(w, ROW_H), Layout::left_to_right(Align::Center), |ui| {
        ui.set_min_size(vec2(w, ROW_H));
        ui.label(egui::RichText::new(label).color(p.text));
        ui.with_layout(Layout::right_to_left(Align::Center), add).inner
    })
    .inner
}

/// Like [`row`] but with a secondary label column of fixed width and the
/// control filling the rest (sliders).
pub fn field<R>(ui: &mut Ui, label: &str, label_w: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = theme::pal(ui);
    let w = ui.available_width();
    ui.allocate_ui_with_layout(vec2(w, ROW_H), Layout::left_to_right(Align::Center), |ui| {
        ui.set_min_size(vec2(w, ROW_H));
        slot(ui, label_w, Align::Min, egui::RichText::new(label).size(SECONDARY + 1.0).color(p.secondary));
        add(ui)
    })
    .inner
}

/// Text in a fixed-width slot, aligned left or right (columns line up).
pub fn slot(ui: &mut Ui, width: f32, align: Align, text: egui::RichText) {
    let layout = if align == Align::Max { Layout::right_to_left(Align::Center) } else { Layout::left_to_right(Align::Center) };
    ui.allocate_ui_with_layout(vec2(width, ROW_H), layout, |ui| {
        ui.set_min_size(vec2(width, ROW_H));
        ui.add(egui::Label::new(text).truncate());
    });
}

/// Secondary helper text under a control.
pub fn hint(ui: &mut Ui, text: &str) {
    let p = theme::pal(ui);
    ui.add(egui::Label::new(egui::RichText::new(text).size(SECONDARY).color(p.secondary)).wrap());
}

/// An iOS/macOS style switch.
pub fn switch(ui: &mut Ui, on: &mut bool, enabled: bool) -> Response {
    let size = vec2(30.0, 18.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    if enabled && resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let p = theme::pal(ui);
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let off = if ui.visuals().dark_mode { Color32::from_white_alpha(38) } else { Color32::from_black_alpha(30) };
    let track = lerp_color(off, theme::accent(), t);
    let track = if enabled { track } else { track.gamma_multiply(0.45) };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(9), track);
    let r = rect.height() / 2.0 - 2.0;
    let cx = egui::lerp(rect.left() + r + 2.0..=rect.right() - r - 2.0, t);
    let c = pos2(cx, rect.center().y);
    painter.circle_filled(c + vec2(0.0, 0.5), r + 0.5, Color32::from_black_alpha(40));
    painter.circle_filled(c, r, if enabled { Color32::WHITE } else { theme::blend(p.control, Color32::from_white_alpha(120)) });
    if resp.hovered() && enabled {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// A macOS check box: a small rounded square, filled with the accent and
/// ticked when on (egui's own reads as a radio button with this theme's
/// corner radius). Use with `ui.add` / `ui.add_enabled`.
pub struct Check<'a> {
    on: &'a mut bool,
    text: egui::WidgetText,
}

impl<'a> Check<'a> {
    pub fn new(on: &'a mut bool, text: impl Into<egui::WidgetText>) -> Self {
        Check { on, text: text.into() }
    }
}

impl egui::Widget for Check<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        const BOX: f32 = 14.0;
        let p = theme::pal(ui);
        let galley = (!self.text.is_empty()).then(|| self.text.into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Body));
        let text_w = galley.as_ref().map_or(0.0, |g| g.size().x + 6.0);
        let h = ui.spacing().interact_size.y;
        let (rect, mut resp) = ui.allocate_exact_size(vec2(BOX + text_w, h), Sense::click());
        if resp.clicked() {
            *self.on = !*self.on;
            resp.mark_changed();
        }
        let enabled = ui.is_enabled();
        let b = Rect::from_center_size(pos2(rect.left() + BOX / 2.0, rect.center().y), vec2(BOX, BOX));
        let painter = ui.painter();
        if *self.on {
            painter.rect_filled(b, CornerRadius::same(4), theme::accent());
            let s = Stroke::new(1.8, Color32::WHITE);
            painter.line(vec![b.left_top() + vec2(3.5, 7.2), b.left_top() + vec2(6.0, 9.8), b.left_top() + vec2(10.6, 4.4)], s);
        } else {
            let fill = if resp.hovered() && enabled { theme::blend(p.field, p.hover) } else { p.field };
            painter.rect_filled(b, CornerRadius::same(4), fill);
            painter.rect_stroke(b, CornerRadius::same(4), Stroke::new(1.0, p.secondary.gamma_multiply(0.6)), egui::StrokeKind::Inside);
        }
        if let Some(g) = galley {
            let color = if enabled { p.text } else { p.secondary };
            painter.galley(pos2(b.right() + 6.0, rect.center().y - g.size().y / 2.0), g, color);
        }
        resp
    }
}

/// A button that asks before acting: the first click turns it into the
/// question, then Cancel and <confirm>; true once confirmed. For destructive
/// actions (delete, restore defaults). [`ask_confirm`] opens the question
/// from elsewhere.
pub fn confirm_button(ui: &mut Ui, id: &str, label: &str, question: &str, confirm: &str, enabled: bool) -> bool {
    // A global id, so another control (a context menu) can open the question.
    let id = egui::Id::new(("confirm", id));
    let asking = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    let mut done = false;
    if asking {
        // The question on its own line: a long name would push the buttons
        // out of a narrow card.
        ui.add(egui::Label::new(egui::RichText::new(question).size(SECONDARY + 1.0)).wrap());
        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                ui.data_mut(|d| d.remove::<bool>(id));
            }
            let b = egui::Button::new(egui::RichText::new(confirm).color(Color32::WHITE)).fill(Color32::from_rgb(0xff, 0x3b, 0x30));
            if ui.add(b).clicked() {
                ui.data_mut(|d| d.remove::<bool>(id));
                done = true;
            }
        });
    } else if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
        ui.data_mut(|d| d.insert_temp(id, true));
    }
    done
}

/// Shows the question of the [`confirm_button`] with this id.
pub fn ask_confirm(ctx: &egui::Context, id: &str) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(("confirm", id)), true));
}

/// The window's one prominent action: accent fill, white semibold text.
pub fn primary_button(ui: &mut Ui, label: &str) -> Response {
    let b = egui::Button::new(egui::RichText::new(label).font(theme::semibold(theme::BODY)).color(Color32::WHITE)).fill(theme::accent()).corner_radius(7).min_size(vec2(0.0, 30.0));
    ui.add(b)
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let [ar, ag, ab, aa] = a.to_srgba_unmultiplied();
    let [br, bg, bb, ba] = b.to_srgba_unmultiplied();
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(m(ar, br), m(ag, bg), m(ab, bb), m(aa, ba))
}

/// Segmented control: a recessed track with the chosen segment raised.
/// `options` are (value, label, enabled, disabled hint).
pub fn segmented<T: Copy + PartialEq>(ui: &mut Ui, value: &mut T, options: &[(T, &str, bool, &str)]) -> bool {
    let p = theme::pal(ui);
    let dark = ui.visuals().dark_mode;
    // The track's own (auto) id keys the segments: two controls in one Ui
    // must not share ids, or a click lands in both.
    let (track_rect, track_resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
    let track = if dark { Color32::from_black_alpha(70) } else { Color32::from_black_alpha(14) };
    ui.painter().rect_filled(track_rect, CornerRadius::same(8), track);
    let inner = track_rect.shrink(2.0);
    let w = inner.width() / options.len() as f32;
    let mut changed = false;
    for (i, (v, label, ok, why)) in options.iter().enumerate() {
        let r = Rect::from_min_size(pos2(inner.left() + w * i as f32, inner.top()), vec2(w, inner.height()));
        let resp = ui.interact(r, track_resp.id.with(("seg", i)), if *ok { Sense::click() } else { Sense::hover() });
        let sel = *value == *v;
        if sel {
            let raised = if dark { Color32::from_rgb(0x5a, 0x5a, 0x5e) } else { Color32::WHITE };
            ui.painter().rect_filled(r.translate(vec2(0.0, 0.5)), CornerRadius::same(6), Color32::from_black_alpha(if dark { 60 } else { 22 }));
            ui.painter().rect_filled(r, CornerRadius::same(6), raised);
        } else if resp.hovered() && *ok {
            ui.painter().rect_filled(r, CornerRadius::same(6), p.hover);
        }
        let color = if !ok { p.tertiary } else if sel { p.text } else { p.secondary };
        let font = if sel { theme::semibold(SECONDARY + 1.0) } else { egui::FontId::new(SECONDARY + 1.0, egui::FontFamily::Name("medium".into())) };
        let galley = ui.painter().layout_no_wrap(label.to_string(), font, color);
        ui.painter().galley(r.center() - galley.size() / 2.0, galley, color);
        let resp = if *ok { resp } else { resp.on_hover_text(*why) };
        if resp.clicked() && !sel {
            *value = *v;
            changed = true;
        }
    }
    changed
}

/// Up/down chevrons, as on a macOS pop-up button (for combo boxes).
pub fn popup_chevrons(ui: &Ui, rect: Rect, visuals: &egui::style::WidgetVisuals, _open: bool) {
    let c = rect.center();
    let stroke = Stroke::new(1.3, visuals.fg_stroke.color.gamma_multiply(0.8));
    let (w, h, gap) = (3.0, 2.2, 1.6);
    let painter = ui.painter();
    painter.line(vec![pos2(c.x - w, c.y - gap), pos2(c.x, c.y - gap - h), pos2(c.x + w, c.y - gap)], stroke);
    painter.line(vec![pos2(c.x - w, c.y + gap), pos2(c.x, c.y + gap + h), pos2(c.x + w, c.y + gap)], stroke);
}

/// Small line icons drawn with the painter (14×14 pt).
pub fn paint_icon(ui: &Ui, rect: Rect, icon: Icon, color: Color32) {
    let painter = ui.painter();
    let s = Stroke::new(1.3, color);
    let c = rect.center();
    let at = |x: f32, y: f32| -> Pos2 { pos2(rect.left() + x, rect.top() + y) };
    let r2 = CornerRadius::same(2);
    let outline = |r: Rect, cr: CornerRadius| painter.rect_stroke(r, cr, s, egui::StrokeKind::Middle);
    match icon {
        Icon::Scene => {
            // Two stacked layers.
            outline(Rect::from_min_max(at(1.5, 4.5), at(10.5, 12.5)), r2);
            painter.line(vec![at(4.0, 2.0), at(12.5, 2.0), at(12.5, 9.5)], s);
        }
        Icon::Display => {
            outline(Rect::from_min_max(at(1.0, 2.0), at(13.0, 10.0)), r2);
            painter.line_segment([at(7.0, 10.0), at(7.0, 12.5)], s);
            painter.line_segment([at(4.5, 12.5), at(9.5, 12.5)], s);
        }
        Icon::Camera => {
            outline(Rect::from_min_max(at(1.0, 4.0), at(9.5, 11.0)), r2);
            painter.add(Shape::convex_polygon(vec![at(10.0, 7.5), at(13.0, 5.0), at(13.0, 10.0)], Color32::TRANSPARENT, s));
        }
        Icon::Overlay => {
            outline(Rect::from_min_max(at(1.0, 2.0), at(13.0, 12.0)), r2);
            painter.circle_filled(at(9.5, 8.5), 2.0, color);
        }
        Icon::Audio => {
            // Waveform bars.
            for (x, h) in [(2.0, 3.0), (4.75, 7.0), (7.5, 10.0), (10.25, 6.0), (13.0, 2.5)] {
                painter.line_segment([pos2(rect.left() + x - 0.5, c.y - h / 2.0), pos2(rect.left() + x - 0.5, c.y + h / 2.0)], s);
            }
        }
        Icon::Output => {
            // Folder.
            painter.add(Shape::line(vec![at(1.0, 11.5), at(1.0, 3.0), at(5.0, 3.0), at(6.5, 4.5), at(13.0, 4.5), at(13.0, 11.5), at(1.0, 11.5)], s));
        }
        Icon::Replay => {
            // Circular arrow.
            let n = 20;
            let pts: Vec<Pos2> = (0..=n)
                .map(|i| {
                    let a = -2.4 + (i as f32 / n as f32) * 5.2;
                    c + vec2(a.cos(), a.sin()) * 5.0
                })
                .collect();
            let start = pts[0];
            painter.add(Shape::line(pts, s));
            painter.line(vec![start + vec2(-2.6, 0.2), start, start + vec2(0.4, -2.8)], s);
        }
        Icon::Stream => {
            painter.circle_filled(c, 1.6, color);
            for (r, span) in [(3.8, 0.75), (6.2, 0.8)] {
                for side in [0.0, std::f32::consts::PI] {
                    let pts: Vec<Pos2> = (0..=10)
                        .map(|i| {
                            let a = side - span + (i as f32 / 10.0) * span * 2.0;
                            c + vec2(a.cos(), a.sin()) * r
                        })
                        .collect();
                    painter.add(Shape::line(pts, s));
                }
            }
        }
    }
}
