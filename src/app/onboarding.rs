//! First run: what SimpleRecord needs permission for, each permission's state (live),
//! and the way to grant it: the system prompt, or the right pane of System
//! Settings when it was denied. Screen Recording takes effect only after a
//! relaunch, which a button does.

use std::time::{Duration, Instant};

use egui::{Align, Color32, CornerRadius, Layout, Margin, RichText, Ui, vec2};

use super::theme::{self, BODY, SECONDARY};
use super::widgets as w;
use super::{Cmd, State};
use crate::tcc::Status;

/// System Settings panes (Privacy & Security).
pub(super) const SCREEN_PANE: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";
pub(super) const MIC_PANE: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone";
pub(super) const CAMERA_PANE: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_Camera";

/// The three permissions, read at most once a second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Perms {
    pub screen: Status,
    pub mic: Status,
    pub camera: Status,
}

impl Perms {
    pub fn read() -> Perms {
        let (screen, mic, camera) = crate::tcc::report();
        Perms { screen, mic, camera }
    }

    pub fn all_granted(&self) -> bool {
        [self.screen, self.mic, self.camera].iter().all(|s| *s == Status::Granted)
    }
}

/// What a permission's row offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offer {
    /// Granted: nothing to do.
    Done,
    /// Never asked: the system prompt.
    Ask,
    /// Denied or restricted: only System Settings can change it.
    OpenSettings,
}

pub fn offer(s: Status) -> Offer {
    match s {
        Status::Granted => Offer::Done,
        Status::NotDetermined => Offer::Ask,
        Status::Denied | Status::Restricted => Offer::OpenSettings,
    }
}

/// Whether to show the welcome screen: until it's been through once, and at
/// launch whenever Screen Recording is missing. Settings outlive the app, so
/// `onboarded` can be left over from a build whose grants don't apply to this one.
pub fn wanted(st: &mut State) -> bool {
    // Scripted runs (the GUI self-test) drive the app without it.
    if st.welcome_dismissed || st.recording.is_some() || std::env::var_os("REC_GUI_SELFTEST").is_some() {
        return false;
    }
    let p = refresh(st);
    if !st.settings.onboarded && p.all_granted() {
        st.settings.onboarded = true;
        let _ = st.settings.save();
        return false;
    }
    needs_welcome(st.settings.onboarded, p.screen)
}

fn needs_welcome(onboarded: bool, screen: Status) -> bool {
    !onboarded || screen != Status::Granted
}

pub(super) fn refresh(st: &mut State) -> Perms {
    match st.perms {
        Some((p, at)) if at.elapsed() < Duration::from_secs(1) => p,
        _ => {
            let p = Perms::read();
            st.perms = Some((p, Instant::now()));
            p
        }
    }
}

fn status_pill(ui: &mut Ui, s: Status) {
    let (text, fill) = match s {
        Status::Granted => ("Allowed", Color32::from_rgb(0x34, 0xc7, 0x59)),
        Status::NotDetermined => ("Not asked yet", Color32::from_rgb(0x8e, 0x8e, 0x93)),
        Status::Denied => ("Not allowed", Color32::from_rgb(0xff, 0x3b, 0x30)),
        Status::Restricted => ("Restricted", Color32::from_rgb(0xff, 0x95, 0x00)),
    };
    let galley = ui.painter().layout_no_wrap(text.into(), theme::semibold(SECONDARY), Color32::WHITE);
    let (rect, _) = ui.allocate_exact_size(galley.size() + vec2(16.0, 6.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(9), fill);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, Color32::WHITE);
}

/// One permission: what it's for, its state, and how to grant it.
/// `ask_again`: forgets an earlier decision and asks from scratch, for one that
/// System Settings can't change (it came from another build of the app).
/// A required permission that is missing gets the window's primary button.
#[allow(clippy::too_many_arguments)]
fn row(ui: &mut Ui, title: &str, why: &str, s: Status, required: bool, ask: &mut dyn FnMut(), ask_again: &mut dyn FnMut(), pane: &str) {
    let p = theme::pal(ui);
    egui::Frame::NONE.fill(p.sidebar).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(14)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.vertical(|ui| {
            // Title and status on one line, the pill pinned to the right.
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).font(theme::semibold(BODY)));
                ui.label(RichText::new(if required { "required" } else { "optional" }).size(SECONDARY).color(p.secondary));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| status_pill(ui, s));
            });
            ui.add(egui::Label::new(RichText::new(why).size(SECONDARY + 1.0).color(p.secondary)).wrap());
            if !matches!(offer(s), Offer::Done) {
                ui.add_space(8.0);
                let button = |ui: &mut Ui, label: &str| if required { w::primary_button(ui, label) } else { ui.button(label) };
                ui.horizontal(|ui| match offer(s) {
                    Offer::Done => {}
                    Offer::Ask => {
                        if button(ui, "Allow…").clicked() {
                            ask();
                        }
                    }
                    Offer::OpenSettings => {
                        if button(ui, "Open System Settings").on_hover_text("Turn SimpleRecord on in this list").clicked() {
                            // Screen Recording's list fills in by requesting.
                            if required {
                                crate::tcc::request_screen_here();
                            }
                            super::macos::open_url(pane);
                        }
                        if s == Status::Denied && ui.button("Ask Again").on_hover_text("Forget the earlier answer and show the macOS prompt. Use this if SimpleRecord is missing from the list, or is on but still not allowed").clicked() {
                            ask_again();
                        }
                    }
                });
            }
        });
    });
    ui.add_space(8.0);
}

pub fn show(ui: &mut Ui, st: &mut State) {
    let p = theme::pal(ui);
    let perms = refresh(st);
    // The status changes while System Settings is in front.
    ui.ctx().request_repaint_after(Duration::from_secs(1));
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.set_max_width(560.0);
            ui.add_space(24.0);
            ui.label(RichText::new("Welcome to SimpleRecord").size(26.0).strong());
            ui.add_space(6.0);
            ui.add(
                egui::Label::new(
                    RichText::new("macOS asks you to allow what SimpleRecord records. Only Screen Recording is required; the microphone and camera are for when you use them.")
                        .color(p.secondary),
                )
                .wrap(),
            );
            ui.add_space(18.0);
        });
        ui.vertical(|ui| {
            ui.set_max_width(560.0);
            row(
                ui,
                "Screen & System Audio Recording",
                "To record your screen, windows and the sound your Mac plays. After you turn SimpleRecord on in System Settings, SimpleRecord needs to restart.",
                perms.screen,
                true,
                &mut crate::tcc::request_screen,
                &mut crate::tcc::request_screen,
                SCREEN_PANE,
            );
            row(ui, "Microphone", "To record your voice.", perms.mic, false, &mut || crate::tcc::request_all(false, true, false, Duration::ZERO), &mut || crate::tcc::ask_again(false), MIC_PANE);
            row(ui, "Camera", "To show your webcam in scenes.", perms.camera, false, &mut || crate::tcc::request_all(false, false, true, Duration::ZERO), &mut || crate::tcc::ask_again(true), CAMERA_PANE);
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                // The primary action is granting Screen Recording (above);
                // going on without it is the secondary one.
                let granted = perms.screen == Status::Granted;
                let resp = if granted { w::primary_button(ui, "Continue") } else { ui.add(egui::Button::new("Continue without Screen Recording").min_size(vec2(0.0, 30.0))) };
                if resp.clicked() {
                    st.welcome_dismissed = true;
                    st.settings.onboarded = true;
                    let _ = st.settings.save();
                    // Errors from before (no permission) are stale now.
                    st.error = None;
                }
                if perms.screen != Status::Granted {
                    ui.add_space(8.0);
                    if ui.add(egui::Button::new("Restart SimpleRecord").min_size(vec2(0.0, 30.0))).on_hover_text("Screen Recording takes effect after a restart: use this once SimpleRecord is on in System Settings").clicked() {
                        st.settings.onboarded = true;
                        let _ = st.settings.save();
                        super::macos::relaunch();
                        st.cmds.push(Cmd::Quit);
                    }
                }
            });
            ui.add_space(8.0);
            ui.label(RichText::new("You can change these any time in System Settings > Privacy & Security.").size(SECONDARY).color(p.tertiary));
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn welcome_returns_when_screen_recording_is_missing() {
        assert!(needs_welcome(false, Status::Granted));
        assert!(needs_welcome(true, Status::Denied));
        assert!(!needs_welcome(true, Status::Granted));
    }

    #[test]
    fn offers() {
        assert_eq!(offer(Status::Granted), Offer::Done);
        assert_eq!(offer(Status::NotDetermined), Offer::Ask);
        assert_eq!(offer(Status::Denied), Offer::OpenSettings);
        assert_eq!(offer(Status::Restricted), Offer::OpenSettings);
    }
}
