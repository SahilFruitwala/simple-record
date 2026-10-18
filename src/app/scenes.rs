//! Scenes in the app: the gallery of templates and custom scenes, the layer
//! editor for custom scenes, and moving and resizing their layers in the
//! preview. Thumbnails are drawn from the same layouts the compositor uses,
//! so they show exactly what gets recorded.

use egui::{Align, Color32, CornerRadius, Rect, RichText, Sense, Stroke, Ui, vec2};

use super::State;
use super::theme::{self, SECONDARY};
use super::widgets as w;
use crate::compose::{Scene, Shape};
use crate::layout::{self, Background, Canvas, Content, Env, Fit, Frame, Grip, Layer, Layout, Style, TextSpec, drag_box};
use crate::settings::Settings;

/// Width of the nominal frame thumbnails and the preview editor work in
/// (any size works: custom layers are fractions).
const NOMINAL_W: f64 = 1920.0;

/// The layouts' environment, as the preview shows the frame.
fn env(st: &State, s: &Settings) -> Env {
    let aspect = (st.preview_aspect as f64).clamp(0.3, 4.0);
    Env {
        w: NOMINAL_W,
        h: (NOMINAL_W / aspect).round(),
        screen_aspect: if s.canvas == Canvas::Source { aspect } else { st.screen_aspect },
        cam_aspect: st.cam_aspect,
        pip: s.pip_spec(),
        background: s.background,
    }
}

/// Where the screen shows in the preview (the scene's first screen
/// layer), if it does.
pub fn screen_rect(st: &State, preview: Rect) -> Option<Rect> {
    let env = env(st, &st.settings);
    let f = st.settings.scene_spec().resolve(&env);
    let l = f.layers.iter().find(|l| l.content == Content::Screen)?;
    let (r, _, _) = layout::place(env.screen_aspect * 1000.0, 1000.0, l.rect, l.fit);
    let k = preview.width() as f64 / env.w;
    let r = Rect::from_min_size(preview.min + vec2((r[0] * k) as f32, (r[1] * k) as f32), vec2((r[2] * k) as f32, (r[3] * k) as f32));
    Some(r.intersect(preview))
}

/// Fill `rect` with a vertical gradient.
fn gradient(ui: &Ui, rect: Rect, bg: Background) {
    let c = |v: [u8; 3]| Color32::from_rgb(v[0], v[1], v[2]);
    let mut mesh = egui::Mesh::default();
    for (pos, col) in [(rect.left_top(), bg.top), (rect.right_top(), bg.top), (rect.right_bottom(), bg.bottom), (rect.left_bottom(), bg.bottom)] {
        mesh.vertices.push(egui::epaint::Vertex { pos, uv: egui::epaint::WHITE_UV, color: c(col) });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    ui.painter().add(egui::Shape::mesh(mesh));
}

/// A miniature of a resolved frame `fw` pixels wide: the screen as a window,
/// the camera as a person, images and text as placeholders.
fn thumb(ui: &Ui, rect: Rect, f: &Frame, fw: f64, screen_aspect: f64) {
    let k = rect.width() as f64 / fw;
    gradient(ui, rect, f.background);
    let painter = ui.painter_at(rect);
    for l in &f.layers {
        let src = match l.content {
            Content::Screen => screen_aspect,
            Content::Display(_) => 16.0 / 10.0,
            Content::Text(_) => l.rect[2] / l.rect[3].max(1.0),
            _ => 16.0 / 9.0,
        };
        let (r, _, _) = layout::place(src * 100.0, 100.0, l.rect, l.fit);
        let r = if l.fit == Fit::Cover { l.rect } else { r };
        let er = Rect::from_min_size(rect.min + vec2((r[0] * k) as f32, (r[1] * k) as f32), vec2((r[2] * k) as f32, (r[3] * k) as f32));
        let radius = match l.style.shape {
            Shape::Circle | Shape::Pill => er.width().min(er.height()) / 2.0,
            Shape::Rect => 0.0,
            _ => (l.style.roundness as f32 * er.width().min(er.height()) / 2.0).max(0.0),
        };
        let cr = CornerRadius::same(radius.round().min(255.0) as u8);
        let alpha = l.opacity.clamp(0.0, 1.0) as f32;
        match &l.content {
            Content::Screen | Content::Display(_) => {
                let fill = if l.content == Content::Screen { Color32::from_rgb(0xe8, 0xec, 0xf3) } else { Color32::from_rgb(0xd8, 0xe6, 0xdc) };
                painter.rect_filled(er, cr, fill.gamma_multiply(alpha));
                // A title bar and a few lines of text.
                let bar = Rect::from_min_size(er.min, vec2(er.width(), (er.height() * 0.12).max(1.0)));
                painter.rect_filled(bar, CornerRadius { nw: cr.nw, ne: cr.ne, sw: 0, se: 0 }, Color32::from_rgb(0xc5, 0xcc, 0xd8).gamma_multiply(alpha));
                for i in 0..3 {
                    let y = er.top() + er.height() * (0.3 + 0.18 * i as f32);
                    let x1 = er.left() + er.width() * (0.62 - 0.12 * i as f32);
                    let stroke = Stroke::new((er.height() * 0.06).clamp(0.5, 2.0), Color32::from_rgb(0x9a, 0xa5, 0xb8).gamma_multiply(alpha));
                    painter.hline((er.left() + er.width() * 0.12)..=x1, y, stroke);
                }
            }
            Content::Camera => {
                painter.rect_filled(er, cr, Color32::from_rgb(0x4a, 0x55, 0x6a).gamma_multiply(alpha));
                // Head and shoulders.
                let u = er.width().min(er.height());
                let c = er.center();
                painter.circle_filled(c - vec2(0.0, u * 0.12), u * 0.17, Color32::from_rgb(0xf0, 0xc0, 0x98).gamma_multiply(alpha));
                let body = Rect::from_center_size(c + vec2(0.0, u * 0.33), vec2(u * 0.6, u * 0.4));
                painter.rect_filled(body.intersect(er), CornerRadius::same((u * 0.2).min(255.0) as u8), Color32::from_rgb(0x3d, 0x8b, 0xff).gamma_multiply(alpha));
            }
            Content::Image(_) => {
                painter.rect_filled(er, cr, Color32::from_rgb(0x6c, 0x5c, 0x9c).gamma_multiply(alpha));
                // A mountain and a sun.
                let u = er.width().min(er.height());
                let base = er.bottom() - er.height() * 0.2;
                let pts = vec![egui::pos2(er.left() + er.width() * 0.15, base), egui::pos2(er.center().x, base - u * 0.4), egui::pos2(er.right() - er.width() * 0.15, base)];
                painter.add(egui::Shape::convex_polygon(pts, Color32::from_rgb(0xd9, 0xd2, 0xf0).gamma_multiply(alpha), Stroke::NONE));
                painter.circle_filled(er.right_top() + vec2(-u * 0.22, u * 0.25), u * 0.1, Color32::from_rgb(0xff, 0xd6, 0x6b).gamma_multiply(alpha));
            }
            Content::Text(t) => {
                let c = Color32::from_rgb(t.color[0], t.color[1], t.color[2]).gamma_multiply(alpha);
                let size = (er.height() * 0.8).clamp(4.0, 28.0);
                painter.text(er.center(), egui::Align2::CENTER_CENTER, &t.text, egui::FontId::proportional(size), c);
            }
            Content::Color(c) => {
                painter.rect_filled(er, cr, Color32::from_rgb(c[0], c[1], c[2]).gamma_multiply(alpha));
            }
        }
        if l.style.border > 0.0 {
            let c = l.style.border_color;
            painter.rect_stroke(er, cr, Stroke::new(1.0, Color32::from_rgb(c[0], c[1], c[2])), egui::StrokeKind::Inside);
        }
    }
}

/// What a gallery tile does when clicked.
enum Pick {
    Template(Scene),
    Custom(usize),
    New,
}

/// The templates, the custom scenes and a "new scene" tile, as a grid of
/// miniatures. Returns true when the shown scene changed.
pub fn gallery(ui: &mut Ui, st: &mut State, s: &mut Settings, idle: bool) -> bool {
    let p = theme::pal(ui);
    let cols = 3;
    let gap = 6.0;
    let tile_w = ((ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32).floor();
    let env = env(st, s);
    let aspect = env.w / env.h;
    let thumb_h = (tile_w / aspect as f32).min(tile_w * 0.9);
    let thumb_w = (thumb_h * aspect as f32).min(tile_w);
    let shown = s.scene_id();
    let mut tiles: Vec<(Pick, String, Option<Frame>, bool, bool, String)> = vec![];
    for scene in Scene::ALL {
        let spec = layout::SceneSpec::Template(scene);
        let ok = idle || st.can_switch_live(&spec);
        tiles.push((Pick::Template(scene), scene.label().into(), Some(scene.resolve(&env)), shown == (scene, None), ok, scene.description().into()));
    }
    for (i, l) in s.custom_scenes.iter().enumerate() {
        let ok = idle || st.can_switch_live(&layout::SceneSpec::Custom(l.clone()));
        tiles.push((Pick::Custom(i), l.name.clone(), Some(l.resolve(&env)), shown.1 == Some(i), ok, format!("{} (custom scene)", l.name)));
    }
    tiles.push((Pick::New, "New Scene".into(), None, false, true, "A custom scene, starting from the one shown".into()));
    let mut picked = None;
    for row in tiles.chunks(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (i, (pick, label, frame, sel, ok, tip)) in row.iter().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(vec2(tile_w, thumb_h + 18.0), if *ok { Sense::click() } else { Sense::hover() });
                let t = Rect::from_center_size(egui::pos2(rect.center().x, rect.top() + thumb_h / 2.0), vec2(thumb_w, thumb_h));
                match frame {
                    Some(f) => thumb(ui, t, f, env.w, env.screen_aspect),
                    None => {
                        let c = if resp.hovered() { p.text } else { p.secondary };
                        ui.painter().rect_filled(t, CornerRadius::same(4), p.control);
                        ui.painter().text(t.center(), egui::Align2::CENTER_CENTER, "+", egui::FontId::proportional(20.0), c);
                    }
                }
                if !ok {
                    ui.painter().rect_filled(t, CornerRadius::same(4), p.window.gamma_multiply(0.6));
                }
                let stroke = if *sel {
                    Stroke::new(2.0, theme::accent())
                } else if resp.hovered() && *ok {
                    Stroke::new(1.0, p.secondary)
                } else {
                    Stroke::new(1.0, p.control_border)
                };
                ui.painter().rect_stroke(t, CornerRadius::same(4), stroke, egui::StrokeKind::Outside);
                let color = if !ok { p.tertiary } else if *sel { p.text } else { p.secondary };
                let font = if *sel { theme::semibold(SECONDARY) } else { egui::FontId::proportional(SECONDARY) };
                let galley = ui.painter().layout_no_wrap(label.clone(), font, color);
                // Long custom names are cut to the tile.
                let text_rect = Rect::from_center_size(egui::pos2(rect.center().x, t.bottom() + 9.0), vec2(tile_w, 14.0));
                ui.painter().with_clip_rect(text_rect).galley(egui::pos2(text_rect.center().x - galley.size().x.min(tile_w) / 2.0, text_rect.top()), galley, color);
                let resp = if *ok { resp.on_hover_text(tip.as_str()) } else { resp.on_hover_text("Start the recording with this scene's sources to switch to it live") };
                if resp.clicked() && !sel {
                    picked = Some(match pick {
                        Pick::Template(sc) => Pick::Template(*sc),
                        Pick::Custom(i) => Pick::Custom(*i),
                        Pick::New => Pick::New,
                    });
                }
                if let Pick::Custom(ci) = pick {
                    let ci = *ci;
                    resp.context_menu(|ui| {
                        if ui.button("Duplicate").clicked() {
                            let mut l = s.custom_scenes[ci].clone();
                            l.name = format!("{} copy", l.name);
                            s.custom_scenes.push(l);
                            ui.close();
                        }
                        if ui.add_enabled(idle, egui::Button::new("Delete")).clicked() {
                            delete_scene(st, s, ci);
                            ui.close();
                        }
                    });
                }
                let _ = i;
            }
        });
        ui.add_space(4.0);
    }
    match picked {
        Some(Pick::Template(sc)) => {
            s.scene = sc;
            s.custom_scene = None;
            true
        }
        Some(Pick::Custom(i)) => {
            s.custom_scene = Some(i);
            st.selected_layer = None;
            true
        }
        Some(Pick::New) => {
            let mut l = match s.scene_spec() {
                layout::SceneSpec::Custom(l) => Layout { name: format!("{} copy", l.name), ..l },
                layout::SceneSpec::Template(sc) => Layout::from_template(sc, &env),
            };
            l.name = unique_name(s, &l.name);
            s.custom_scenes.push(l);
            s.custom_scene = Some(s.custom_scenes.len() - 1);
            st.selected_layer = None;
            true
        }
        None => false,
    }
}

fn unique_name(s: &Settings, base: &str) -> String {
    let mut name = base.to_string();
    let mut n = 2;
    while s.custom_scenes.iter().any(|l| l.name == name) {
        name = format!("{base} {n}");
        n += 1;
    }
    name
}

fn delete_scene(st: &mut State, s: &mut Settings, i: usize) {
    if i >= s.custom_scenes.len() {
        return;
    }
    s.custom_scenes.remove(i);
    s.custom_scene = match s.custom_scene {
        Some(c) if c == i => None,
        Some(c) if c > i => Some(c - 1),
        c => c,
    };
    st.selected_layer = None;
}

/// A new layer of `content`, centered.
fn new_layer(content: Content, aspect: f64) -> Layer {
    let (w, h): (f64, f64) = match &content {
        Content::Text(_) => (0.32, 0.09 * aspect.min(1.8)),
        Content::Camera => (0.3, 0.3 * aspect / (16.0 / 9.0)),
        _ => (0.4, 0.4),
    };
    let (w, h) = (w.min(1.0), h.min(1.0));
    let fit = match content {
        Content::Camera | Content::Color(_) => Fit::Cover,
        _ => Fit::Contain,
    };
    Layer { content, rect: [(1.0 - w) / 2.0, (1.0 - h) / 2.0, w, h], fit, ..Default::default() }
}

/// The custom scene's name, layers (top first) and the selected layer's
/// settings. Live, also while recording.
pub fn layers_panel(ui: &mut Ui, st: &mut State, s: &mut Settings, idle: bool) {
    let p = theme::pal(ui);
    let Some(ci) = s.custom_scene.filter(|i| *i < s.custom_scenes.len()) else { return };
    let aspect = (st.preview_aspect as f64).clamp(0.3, 4.0);
    w::field(ui, "Name", 52.0, |ui| {
        ui.add(egui::TextEdit::singleline(&mut s.custom_scenes[ci].name).margin(vec2(8.0, 4.0)).desired_width(f32::INFINITY));
    });
    w::divider(ui);
    let n = s.custom_scenes[ci].layers.len();
    if st.selected_layer.is_some_and(|i| i >= n) {
        st.selected_layer = None;
    }
    // Top layer first, like a stack.
    let mut action: Option<(usize, i32)> = None;
    for i in (0..n).rev() {
        let l = &mut s.custom_scenes[ci].layers[i];
        let sel = st.selected_layer == Some(i);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.spacing_mut().button_padding.x = 4.0;
            let eye = if l.visible { "●" } else { "○" };
            if ui.add(egui::Button::new(eye).frame(false).min_size(vec2(18.0, 22.0))).on_hover_text(if l.visible { "Hide" } else { "Show" }).clicked() {
                l.visible = !l.visible;
            }
            let label = l.content.label();
            let lw = (ui.available_width() - 3.0 * 26.0).max(40.0);
            let resp = ui.add_sized(vec2(lw, 22.0), egui::Button::selectable(sel, RichText::new(label).size(SECONDARY + 1.0)).truncate());
            if resp.clicked() {
                st.selected_layer = if sel { None } else { Some(i) };
            }
            if ui.add_enabled(i + 1 < n, egui::Button::new("↑").min_size(vec2(22.0, 22.0))).on_hover_text("Move up").clicked() {
                action = Some((i, 1));
            }
            if ui.add_enabled(i > 0, egui::Button::new("↓").min_size(vec2(22.0, 22.0))).on_hover_text("Move down").clicked() {
                action = Some((i, -1));
            }
            if ui.add(egui::Button::new("✕").min_size(vec2(22.0, 22.0))).on_hover_text("Remove").clicked() {
                action = Some((i, 0));
            }
        });
    }
    if n == 0 {
        w::hint(ui, "No layers yet: add some below.");
    }
    let layers = &mut s.custom_scenes[ci].layers;
    match action {
        Some((i, 0)) => {
            layers.remove(i);
            st.selected_layer = None;
        }
        Some((i, d)) => {
            let j = (i as i32 + d) as usize;
            layers.swap(i, j);
            if st.selected_layer == Some(i) {
                st.selected_layer = Some(j);
            }
        }
        None => {}
    }
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        ui.spacing_mut().button_padding.x = 6.0;
        let mut add = None;
        ui.label(RichText::new("Add").size(SECONDARY).color(p.secondary));
        if ui.small_button("Screen").clicked() {
            add = Some(Content::Screen);
        }
        // The camera needs a session started with it (switching is live).
        if ui.add_enabled(idle || st.recording.as_ref().is_some_and(|r| r.camera_name().is_some()), egui::Button::new("Camera").small()).clicked() {
            add = Some(Content::Camera);
        }
        if ui.small_button("Image…").clicked()
            && let Some(f) = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "heic", "tiff", "gif", "bmp", "webp"]).pick_file()
        {
            add = Some(Content::Image(f.to_string_lossy().into_owned()));
        }
        if ui.small_button("Text").clicked() {
            add = Some(Content::Text(TextSpec::default()));
        }
        if ui.small_button("Color").clicked() {
            add = Some(Content::Color([0x30, 0x30, 0x3a]));
        }
        // Other displays, whole (each a capture stream of its own).
        if st.displays.len() > 1 {
            for n in 1..=st.displays.len() {
                if n != s.display && ui.add_enabled(idle, egui::Button::new(format!("Display {n}")).small()).on_hover_text("Another display, whole").clicked() {
                    add = Some(Content::Display(n));
                }
            }
        }
        if let Some(c) = add {
            let layers = &mut s.custom_scenes[ci].layers;
            // Screens and colors go under everything else, the rest on top.
            let under = matches!(c, Content::Screen | Content::Color(_));
            let mut l = new_layer(c, aspect);
            if under {
                l.rect = [0.0, 0.0, 1.0, 1.0];
                layers.insert(0, l);
                st.selected_layer = Some(0);
            } else {
                layers.push(l);
                st.selected_layer = Some(layers.len() - 1);
            }
        }
    });
    if let Some(i) = st.selected_layer {
        w::divider(ui);
        layer_settings(ui, &mut s.custom_scenes[ci].layers[i]);
    }
    w::divider(ui);
    ui.horizontal(|ui| {
        ui.spacing_mut().button_padding.x = 8.0;
        if ui.button("Export…").on_hover_text("Save as JSON for `rec-rs record --scene-file`").clicked()
            && let Some(f) = rfd::FileDialog::new().add_filter("Scene", &["json"]).set_file_name(format!("{}.json", s.custom_scenes[ci].name)).save_file()
        {
            let json = serde_json::to_string_pretty(&s.custom_scenes[ci]).unwrap_or_default();
            if let Err(e) = std::fs::write(&f, json) {
                st.error = Some(format!("Could not export the scene: {e}"));
            }
        }
        if ui.button("Import…").clicked()
            && let Some(f) = rfd::FileDialog::new().add_filter("Scene", &["json"]).pick_file()
        {
            match std::fs::read_to_string(&f).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<Layout>(&t).map_err(|e| e.to_string())) {
                Ok(mut l) => {
                    l.name = unique_name(s, &l.name);
                    s.custom_scenes.push(l);
                    s.custom_scene = Some(s.custom_scenes.len() - 1);
                    st.selected_layer = None;
                }
                Err(e) => st.error = Some(format!("Could not import {}: {e}", f.display())),
            }
        }
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if ui.add_enabled(idle, egui::Button::new("Delete")).on_hover_text("Delete this custom scene").clicked() {
                delete_scene(st, s, ci);
            }
        });
    });
    w::hint(ui, "Drag layers in the preview to move them; drag a corner to resize.");
}

/// The selected layer's box, fit, outline and content.
fn layer_settings(ui: &mut Ui, l: &mut Layer) {
    const LABEL_W: f32 = 72.0;
    let mut pct = l.rect.map(|v| v * 100.0);
    w::field(ui, "Position", LABEL_W, |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.add(egui::DragValue::new(&mut pct[0]).range(-50.0..=100.0).speed(0.2).max_decimals(1).prefix("x "));
        ui.add(egui::DragValue::new(&mut pct[1]).range(-50.0..=100.0).speed(0.2).max_decimals(1).prefix("y "));
    });
    w::field(ui, "Size", LABEL_W, |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.add(egui::DragValue::new(&mut pct[2]).range(1.0..=200.0).speed(0.2).max_decimals(1).prefix("w "));
        ui.add(egui::DragValue::new(&mut pct[3]).range(1.0..=200.0).speed(0.2).max_decimals(1).prefix("h "));
    });
    let rect = pct.map(|v| v / 100.0);
    if rect != l.rect {
        l.rect = rect;
    }
    if ui.small_button("Fill the frame").clicked() {
        l.rect = [0.0, 0.0, 1.0, 1.0];
    }
    match &mut l.content {
        Content::Text(t) => {
            w::field(ui, "Text", LABEL_W, |ui| {
                ui.add(egui::TextEdit::singleline(&mut t.text).margin(vec2(8.0, 4.0)).desired_width(f32::INFINITY));
            });
            w::field(ui, "Color", LABEL_W, |ui| {
                ui.color_edit_button_srgb(&mut t.color);
                let mut boxed = t.background.is_some();
                if ui.checkbox(&mut boxed, "Box").changed() {
                    t.background = boxed.then_some([20, 20, 28, 200]);
                }
                if let Some(bg) = &mut t.background {
                    ui.color_edit_button_srgba_unmultiplied(bg);
                }
            });
        }
        Content::Image(path) => {
            w::field(ui, "File", LABEL_W, |ui| {
                let name = std::path::Path::new(path.as_str()).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if ui.small_button("Choose…").clicked()
                    && let Some(f) = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "heic", "tiff", "gif", "bmp", "webp"]).pick_file()
                {
                    *path = f.to_string_lossy().into_owned();
                }
                ui.add(egui::Label::new(RichText::new(name).size(SECONDARY)).truncate()).on_hover_text(path.as_str());
            });
        }
        Content::Color(c) => {
            w::field(ui, "Color", LABEL_W, |ui| {
                ui.color_edit_button_srgb(c);
            });
        }
        Content::Camera => {
            w::row(ui, "Mirror", |ui| w::switch(ui, &mut l.mirror, true));
        }
        Content::Screen | Content::Display(_) => {}
    }
    if matches!(l.content, Content::Screen | Content::Display(_) | Content::Camera | Content::Image(_)) {
        w::field(ui, "Fit", LABEL_W, |ui| {
            w::segmented(ui, &mut l.fit, &[(Fit::Contain, "Whole picture", true, ""), (Fit::Cover, "Fill the box", true, "")]);
        });
    }
    shape_controls(ui, &mut l.style, LABEL_W);
}

/// Outline: shape, roundness, border and shadow.
fn shape_controls(ui: &mut Ui, style: &mut Style, label_w: f32) {
    let p = theme::pal(ui);
    w::field(ui, "Shape", label_w, |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for shape in [Shape::Rect, Shape::Rounded, Shape::Circle, Shape::Pill, Shape::Squircle] {
            let sel = style.shape == shape;
            let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 24.0), Sense::click());
            let fill = if sel { theme::accent().gamma_multiply(0.18) } else if resp.hovered() { p.hover } else { Color32::TRANSPARENT };
            ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
            super::ui::shape_icon(ui, rect.shrink(4.0), shape, style.roundness.max(0.3), if sel { theme::accent() } else { p.secondary }, 0.7);
            if resp.on_hover_text(shape.label()).clicked() {
                style.shape = shape;
            }
        }
    });
    if style.shape.uses_roundness() {
        w::field(ui, "Roundness", label_w, |ui| {
            let mut pct = style.roundness * 100.0;
            ui.spacing_mut().slider_width = (ui.available_width() - 8.0).max(40.0);
            if ui.add(egui::Slider::new(&mut pct, 0.0..=100.0).show_value(false)).changed() {
                style.roundness = pct.round() / 100.0;
            }
        });
    }
    w::field(ui, "Border", label_w, |ui| {
        ui.spacing_mut().slider_width = (ui.available_width() - 40.0).max(40.0);
        ui.add(egui::Slider::new(&mut style.border, 0.0..=20.0).step_by(1.0).show_value(false));
        ui.add_enabled_ui(style.border > 0.0, |ui| {
            ui.spacing_mut().interact_size = vec2(22.0, 18.0);
            ui.color_edit_button_srgb(&mut style.border_color);
        });
    });
    w::row(ui, "Shadow", |ui| w::switch(ui, &mut style.shadow, true));
}

/// Select, move and resize the custom scene's layers in the preview.
pub fn edit_in_preview(ui: &mut Ui, st: &mut State, preview: Rect) {
    let Some(ci) = st.settings.custom_scene.filter(|i| *i < st.settings.custom_scenes.len()) else { return };
    let accent = theme::accent();
    let to_ui = |r: [f64; 4]| {
        Rect::from_min_size(
            preview.min + vec2(r[0] as f32 * preview.width(), r[1] as f32 * preview.height()),
            vec2(r[2] as f32 * preview.width(), r[3] as f32 * preview.height()),
        )
    };
    // Clicking the empty frame clears the selection.
    let bg = ui.interact(preview, ui.id().with("scene-bg"), Sense::click());
    if bg.clicked() {
        st.selected_layer = None;
    }
    let layers = st.settings.custom_scenes[ci].layers.clone();
    let mut changed = None;
    let mut done = false;
    // Bodies, bottom first: the top layer under the pointer wins.
    for (i, l) in layers.iter().enumerate() {
        if !l.visible {
            continue;
        }
        let r = to_ui(l.rect).intersect(preview);
        if r.width() <= 0.0 || r.height() <= 0.0 {
            continue;
        }
        let resp = ui.interact(r, ui.id().with(("layer", i)), Sense::click_and_drag());
        if resp.hovered() && !resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            ui.painter().rect_stroke(r, CornerRadius::ZERO, Stroke::new(1.0, accent.gamma_multiply(0.6)), egui::StrokeKind::Inside);
        }
        if resp.clicked() || resp.drag_started() {
            st.selected_layer = Some(i);
        }
        if resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            let d = resp.drag_delta();
            changed = Some((i, Grip::Body, [(d.x / preview.width()) as f64, (d.y / preview.height()) as f64]));
        }
        done |= resp.drag_stopped();
    }
    if let Some(i) = st.selected_layer.filter(|i| *i < layers.len() && layers[*i].visible) {
        let r = to_ui(layers[i].rect);
        ui.painter().rect_stroke(r, CornerRadius::ZERO, Stroke::new(1.5, accent), egui::StrokeKind::Outside);
        // Corner handles, on top of the bodies.
        for (right, bottom) in [(false, false), (true, false), (false, true), (true, true)] {
            let c = egui::pos2(if right { r.right() } else { r.left() }, if bottom { r.bottom() } else { r.top() });
            let h = Rect::from_center_size(c, vec2(9.0, 9.0));
            let resp = ui.interact(h.expand(3.0), ui.id().with(("grip", i, right, bottom)), Sense::drag());
            ui.painter().rect_filled(h, CornerRadius::same(2), Color32::WHITE);
            ui.painter().rect_stroke(h, CornerRadius::same(2), Stroke::new(1.0, accent), egui::StrokeKind::Outside);
            if resp.hovered() || resp.dragged() {
                let icon = if right == bottom { egui::CursorIcon::ResizeNwSe } else { egui::CursorIcon::ResizeNeSw };
                ui.ctx().set_cursor_icon(icon);
            }
            if resp.dragged() {
                let d = resp.drag_delta();
                changed = Some((i, Grip::Corner(right, bottom), [(d.x / preview.width()) as f64, (d.y / preview.height()) as f64]));
            }
            done |= resp.drag_stopped();
        }
    }
    if let Some((i, grip, d)) = changed {
        let l = &mut st.settings.custom_scenes[ci].layers[i];
        // Snap within 6 points of the preview.
        let snap = 6.0 / preview.width().max(preview.height()) as f64;
        l.rect = drag_box(l.rect, grip, d, 0.02, snap);
        st.apply_scene();
    }
    if done {
        let _ = st.settings.save();
    }
}
