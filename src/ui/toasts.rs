//! M3 snackbars at the bottom of the window.

use super::{widgets, Action, ToastKind};
use crate::app::App;
use egui::{Align2, CornerRadius, Frame, Id, Margin, RichText};
use egui_material_icons::icons::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const MAX_VISIBLE: usize = 4;
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub struct Toast {
    id: u64,
    pub text: String,
    pub kind: ToastKind,
    pub action: Option<(String, Action)>,
    shown: Instant,
    /// Hovering pushes this back so the toast can be read.
    expires: Instant,
}

impl Toast {
    pub fn new(text: impl Into<String>, kind: ToastKind) -> Self {
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            text: text.into(),
            kind,
            action: None,
            shown: Instant::now(),
            expires: Instant::now()
                + Duration::from_secs(if kind == ToastKind::Error { 7 } else { 4 }),
        }
    }
}

#[derive(Default)]
pub struct Toasts {
    list: Vec<Toast>,
    }

impl Toasts {
    pub fn push(&mut self, mut toast: Toast) {
        if toast.action.is_some() {
            toast.expires = toast.expires.max(Instant::now() + Duration::from_secs(7));
        }
        // Collapse exact repeats instead of stacking them.
        self.list.retain(|t| t.text != toast.text);
        self.list.push(toast);
        if self.list.len() > MAX_VISIBLE {
            self.list.remove(0);
        }
    }
}

pub fn show(ctx: &egui::Context, app: &mut App) {
    let p = app.palette.clone();
    let now = Instant::now();
    let mut dismiss = Vec::new();
    let mut fire = Vec::new();

    let bottom = 20.0;
    egui::Area::new(Id::unique("toasts"))
        .anchor(Align2::CENTER_BOTTOM, egui::vec2(0.0, -bottom))
        .order(egui::Order::Foreground)
        .interactable(true)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            for toast in &mut app.toasts.list {
                let appear = ((now - toast.shown).as_secs_f32() / 0.15).min(1.0);
                let resp = Frame::new()
                    .fill(p.inverse_surface)
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin {
                        left: 16,
                        right: 8,
                        top: 6,
                        bottom: 6,
                    })
                    .shadow(ui.visuals().popup_shadow)
                    .multiply_with_opacity(appear)
                    .show(ui, |ui| {
                        ui.set_min_width(320.0);
                        ui.set_max_width(560.0);
                        ui.horizontal(|ui| {
                            let (icon, color) = match toast.kind {
                                ToastKind::Info => (ICON_INFO, p.inverse_on_surface),
                                ToastKind::Success => (ICON_CHECK_CIRCLE, p.inverse_primary),
                                ToastKind::Error => (ICON_ERROR, p.error_container),
                            };
                            widgets::icon(ui, icon, 20.0, color);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&toast.text).color(p.inverse_on_surface),
                                )
                                .wrap(),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let close = ui.add(
                                        egui::Button::new(
                                            widgets::icon_text(ICON_CLOSE, 18.0)
                                                .color(p.inverse_on_surface),
                                        )
                                        .frame(false),
                                    );
                                    if close.clicked() {
                                        dismiss.push(toast.id);
                                    }
                                    if let Some((label, action)) = &toast.action {
                                        let b = ui.add(
                                            egui::Button::new(
                                                RichText::new(label)
                                                    .color(p.inverse_primary)
                                                    .size(14.0),
                                            )
                                            .frame(false),
                                        );
                                        if b.clicked() {
                                            fire.push(action.clone());
                                            dismiss.push(toast.id);
                                        }
                                    }
                                },
                            );
                        });
                    });
                if resp.response.contains_pointer() {
                    toast.expires = toast.expires.max(now + Duration::from_secs(2));
                } else if now >= toast.expires {
                    dismiss.push(toast.id);
                }
            }
        });

    app.toasts.list.retain(|t| !dismiss.contains(&t.id));
    app.actions.extend(fire);
    if !app.toasts.list.is_empty() {
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}
