//! A chat tab: the message list, permission prompts, the task list, the
//! composer and the status bar.

use super::tools;
use material_3_ui::keys::take_key;
use material_3_ui::markdown;
use material_3_ui::widgets::{self, ButtonKind};
use super::{Action, Actions};
use crate::chat::Chat;
use crate::claude::conversation::{Body, Item};
use crate::claude::protocol::{self, Decision, PermissionRequest};
use material_3_ui::theme::M3Palette;
use egui::{CornerRadius, Frame, Margin, RichText, Ui};
use material_3_ui::icons::icons::*;
use std::collections::HashSet;

/// Widest the conversation column gets.
const COLUMN: f32 = 860.0;

pub struct Env<'a> {
    pub font_size: f32,
    pub program: Option<&'a std::path::Path>,
    pub config: &'a crate::config::Config,
}

pub fn show(ui: &mut Ui, chat: &mut Chat, env: &Env<'_>, actions: &mut Actions) {
    let p = widgets::palette(ui);
    let max_prompt = ui.available_height() * 0.45;
    egui::Panel::bottom(("chat-bottom", chat.uid))
        .resizable(false)
        .show_separator_line(false)
        .frame(Frame::NONE)
        .show(ui, |ui| {
            if let Some(err) = chat.exited.clone().filter(|e| !e.is_empty()) {
                ui.add_space(6.0);
                centered(ui, |ui| {
                    material_3_ui::common::banner(ui, ICON_ERROR, &format!("Claude Code exited: {err}"), true, |ui| {
                        if widgets::button(ui, ButtonKind::Text, None, "Dismiss").clicked() {
                            chat.exited = None;
                        }
                    })
                });
            }
            if let Some(b) = chat.conv.banner.clone() {
                ui.add_space(6.0);
                centered(ui, |ui| material_3_ui::common::banner(ui, ICON_SYNC, &b, false, |_| {}));
            }
            if !chat.conv.permissions.is_empty() {
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .id_salt(("permissions", chat.uid))
                    .max_height(max_prompt)
                    .auto_shrink([false, true])
                    .show(ui, |ui| centered(ui, |ui| permissions(ui, chat, &p)));
            }
            ui.add_space(6.0);
            centered(ui, |ui| super::composer::show(ui, chat, env, actions));
            ui.add_space(4.0);
            status_bar(ui, chat, &p);
            ui.add_space(4.0);
        });
    if !chat.conv.todos.is_empty() {
        ui.add_space(8.0);
        centered(ui, |ui| todo_panel(ui, chat, &p));
    }
    messages(ui, chat, env, actions, &p);
}

fn centered(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let w = ui.available_width();
    let col = w.min(COLUMN);
    let pad = ((w - col) / 2.0).max(0.0);
    ui.horizontal(|ui| {
        ui.add_space(pad);
        ui.vertical(|ui| {
            ui.set_width(col);
            add(ui);
        });
    });
}

fn messages(ui: &mut Ui, chat: &mut Chat, env: &Env<'_>, actions: &mut Actions, p: &M3Palette) {
    let stick = std::mem::take(&mut chat.scroll_to_bottom);
    let area = egui::ScrollArea::vertical().id_salt(("chat", chat.uid)).auto_shrink([false, false]).stick_to_bottom(true);
    area.show(ui, |ui| {
        ui.add_space(16.0);
        if chat.conv.items.is_empty() {
            empty_state(ui, chat, p);
        }
        let running = chat.conv.running;
        let items = &chat.conv.items;
        let expanded = &mut chat.expanded;
        let size = env.font_size;
        centered(ui, |ui| {
            for (i, item) in items.iter().enumerate() {
                if item.parent.is_none() {
                    show_item(ui, items, i, item, expanded, running, size, actions, p);
                }
            }
            if running && !items.last().is_some_and(|i| i.streaming) {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(16.0).color(p.primary));
                    ui.label(RichText::new("Working…").size(13.0).color(p.on_surface_variant));
                });
            }
        });
        ui.add_space(24.0);
        if stick {
            ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn show_item(
    ui: &mut Ui,
    items: &[Item],
    index: usize,
    item: &Item,
    expanded: &mut HashSet<String>,
    running: bool,
    size: f32,
    actions: &mut Actions,
    p: &M3Palette,
) {
    match &item.body {
        Body::User { text, images } => {
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                Frame::new()
                    .fill(p.primary_container)
                    .corner_radius(CornerRadius { nw: 20, ne: 6, sw: 20, se: 20 })
                    .inner_margin(Margin::symmetric(16, 10))
                    .show(ui, |ui| {
                        ui.set_max_width(ui.available_width() * 0.8);
                        ui.vertical(|ui| {
                            if *images > 0 {
                                ui.label(
                                    RichText::new(format!("🖼 {images} image{}", if *images == 1 { "" } else { "s" }))
                                        .size(12.0)
                                        .color(p.on_primary_container),
                                );
                            }
                            ui.add(egui::Label::new(RichText::new(text).size(size).color(p.on_primary_container)).wrap());
                        });
                    });
            });
            ui.add_space(6.0);
        }
        Body::Text(text) => {
            ui.add_space(2.0);
            let mut opened = None;
            markdown::show_with(ui, text, size, &mut |u| opened = Some(u), Some(super::syntax::highlighter()));
            if let Some(u) = opened {
                actions.push(Action::OpenUrl(u));
            }
        }
        Body::Thinking(text) => {
            let key = format!("think-{index}");
            let open = expanded.contains(&key);
            let r = ui.horizontal(|ui| {
                widgets::icon(ui, ICON_PSYCHOLOGY, 16.0, p.on_surface_variant);
                ui.label(RichText::new(if item.streaming { "Thinking…" } else { "Thought" }).size(12.5).italics().color(p.on_surface_variant));
                widgets::icon(ui, if open { ICON_EXPAND_LESS } else { ICON_EXPAND_MORE }, 16.0, p.on_surface_variant);
            });
            let click = ui.interact(r.response.rect, ui.scope_id().with(&key), egui::Sense::click());
            if click.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                if open {
                    expanded.remove(&key);
                } else {
                    expanded.insert(key);
                }
            }
            if open {
                Frame::new().inner_margin(Margin { left: 22, right: 0, top: 0, bottom: 4 }).show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(text).size(size * 0.9).italics().color(p.on_surface_variant)).wrap());
                });
            }
        }
        Body::Tool(t) => {
            ui.add_space(4.0);
            let id = t.id.clone();
            let mut children = |ui: &mut Ui, expanded: &mut HashSet<String>, actions: &mut Actions| {
                for (i, child) in items.iter().enumerate() {
                    if child.parent.as_deref() == Some(id.as_str()) {
                        show_item(ui, items, i, child, expanded, running, size * 0.95, actions, p);
                    }
                }
            };
            tools::card(ui, t, expanded, running, actions, &mut children);
        }
        Body::Notice { text, error } => {
            ui.add_space(4.0);
            Frame::new()
                .fill(if *error { p.error_container } else { p.surface_container })
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        widgets::icon(ui, if *error { ICON_ERROR } else { ICON_INFO }, 16.0, if *error { p.on_error_container } else { p.on_surface_variant });
                        ui.add(egui::Label::new(RichText::new(text).size(13.0).color(if *error { p.on_error_container } else { p.on_surface_variant })).wrap());
                    });
                });
        }
        Body::TurnEnd { cost_usd, duration_ms, error } => {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                let mut parts = Vec::new();
                if *duration_ms > 0 {
                    parts.push(format_duration(*duration_ms));
                }
                if *cost_usd > 0.0 {
                    parts.push(format!("${cost_usd:.3}"));
                }
                if *error {
                    parts.push("stopped".into());
                }
                if !parts.is_empty() {
                    ui.label(RichText::new(parts.join(" · ")).size(11.5).color(p.outline));
                }
            });
            ui.add_space(8.0);
        }
    }
}

fn format_duration(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 60 {
        format!("{}m {}s", s / 60, s % 60)
    } else {
        format!("{:.1}s", ms as f32 / 1000.0)
    }
}

fn empty_state(ui: &mut Ui, chat: &Chat, p: &M3Palette) {
    ui.add_space(60.0);
    ui.vertical_centered(|ui| {
        widgets::icon(ui, ICON_AUTO_AWESOME, 44.0, p.primary);
        ui.add_space(8.0);
        ui.label(RichText::new("What should we work on?").size(26.0).color(p.on_surface));
        ui.label(
            RichText::new(format!("Claude Code works in {}", material_3_ui::sys::paths::tilde(&chat.cwd)))
                .size(14.0)
                .color(p.on_surface_variant),
        );
        ui.add_space(6.0);
        ui.label(RichText::new("Type / for commands and @ to mention a file.").size(13.0).color(p.outline));
    });
    ui.add_space(30.0);
}

fn todo_panel(ui: &mut Ui, chat: &Chat, p: &M3Palette) {
    let done = chat.conv.todos.iter().filter(|t| t.status == "completed").count();
    Frame::new()
        .fill(p.surface_container)
        .corner_radius(CornerRadius::same(16))
        .inner_margin(Margin::symmetric(14, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::CollapsingHeader::new(
                RichText::new(format!("Tasks · {done}/{}", chat.conv.todos.len())).size(13.0).color(p.on_surface),
            )
            .id_salt(("todos", chat.uid))
            .default_open(true)
            .show(ui, |ui| {
                let v = serde_json::json!({
                    "todos": chat.conv.todos.iter().map(|t| serde_json::json!({"content": t.content, "status": t.status})).collect::<Vec<_>>()
                });
                tools::todo_list(ui, p, &v);
            });
        });
    ui.add_space(6.0);
}

fn permissions(ui: &mut Ui, chat: &mut Chat, p: &M3Palette) {
    let reqs: Vec<PermissionRequest> = chat.conv.permissions.clone();
    let mut decided: Option<(String, Decision)> = None;
    // Keyboard shortcuts while no text field has focus.
    let no_focus = ui.memory(|m| m.focused().is_none());
    for (n, req) in reqs.iter().enumerate() {
        let first = n == 0;
        Frame::new()
            .fill(p.surface_container_high)
            .stroke(egui::Stroke::new(1.5, p.primary))
            .corner_radius(CornerRadius::same(20))
            .inner_margin(Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    widgets::icon(ui, ICON_SHIELD, 20.0, p.primary);
                    let title = req.title.clone().unwrap_or_else(|| format!("Allow {}?", req.display_name.clone().unwrap_or_else(|| req.tool_name.clone())));
                    ui.label(widgets::title_text(title, 15.0).color(p.on_surface));
                });
                let summary = tools::summary(&req.tool_name, &req.input);
                ui.label(RichText::new(req.description.clone().unwrap_or(summary)).size(13.0).color(p.on_surface_variant));
                ui.add_space(4.0);
                let s = |k: &str| req.input.get(k).and_then(|v| v.as_str()).unwrap_or_default();
                match req.tool_name.as_str() {
                    "Bash" => tools::mono_box(ui, p, &format!("$ {}", s("command")), 12),
                    "Edit" => tools::diff(ui, p, s("old_string"), s("new_string"), 30),
                    "Write" => tools::mono_box(ui, p, s("content"), 16),
                    "MultiEdit" => {
                        if let Some(edits) = req.input.get("edits").and_then(|e| e.as_array()) {
                            for e in edits.iter().take(4) {
                                let g = |k: &str| e.get(k).and_then(|v| v.as_str()).unwrap_or_default();
                                tools::diff(ui, p, g("old_string"), g("new_string"), 12);
                            }
                        }
                    }
                    _ => tools::mono_box(ui, p, &serde_json::to_string_pretty(&req.input).unwrap_or_default(), 12),
                }
                if let Some(reason) = &req.decision_reason {
                    ui.label(RichText::new(reason).size(12.0).color(p.outline));
                }
                if let Some(path) = &req.blocked_path {
                    ui.label(RichText::new(format!("Outside the project: {path}")).size(12.0).color(p.tertiary));
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut chat.composer.deny_reason)
                            .hint_text("Reason (optional, sent with Deny)")
                            .desired_width((ui.available_width() - 360.0).max(140.0)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let keys = first && no_focus;
                        let allow = widgets::button(ui, ButtonKind::Filled, Some(ICON_CHECK), "Allow").on_hover_text("Y");
                        let always = widgets::button(ui, ButtonKind::Tonal, Some(ICON_DONE_ALL), "Always").on_hover_text(always_label(req));
                        let deny = widgets::button(ui, ButtonKind::Text, Some(ICON_BLOCK), "Deny").on_hover_text("N");
                        let (ky, ka, kn) = if keys {
                            let key = |k| take_key(ui.ctx(), egui::Modifiers::NONE, k);
                            (key(egui::Key::Y), key(egui::Key::A), key(egui::Key::N))
                        } else {
                            (false, false, false)
                        };
                        if allow.clicked() || ky {
                            decided = Some((req.request_id.clone(), Decision::Allow { updated_permissions: vec![] }));
                        } else if always.clicked() || ka {
                            decided = Some((req.request_id.clone(), Decision::Allow { updated_permissions: protocol::always_allow_updates(req) }));
                        } else if deny.clicked() || kn {
                            decided = Some((req.request_id.clone(), Decision::Deny { message: chat.composer.deny_reason.trim().to_string() }));
                        }
                    });
                });
            });
        ui.add_space(6.0);
    }
    if let Some((id, d)) = decided {
        chat.composer.deny_reason.clear();
        chat.answer(&id, d);
    }
}

/// What "Always" will add, for the tooltip.
fn always_label(req: &PermissionRequest) -> String {
    let rules: Vec<String> = protocol::always_allow_updates(req)
        .iter()
        .flat_map(|u| {
            u.get("rules")
                .and_then(|r| r.as_array())
                .map(|rules| {
                    rules
                        .iter()
                        .map(|r| {
                            let tool = r.get("toolName").and_then(|v| v.as_str()).unwrap_or_default();
                            match r.get("ruleContent").and_then(|v| v.as_str()) {
                                Some(c) => format!("{tool}({c})"),
                                None => tool.to_string(),
                            }
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .collect();
    if rules.is_empty() {
        "Always allow (A)".into()
    } else {
        format!("Always allow {} (A)", rules.join(", "))
    }
}

fn status_bar(ui: &mut Ui, chat: &Chat, p: &M3Palette) {
    let w = ui.available_width();
    ui.allocate_ui_with_layout(egui::vec2(w, 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.add_space(16.0);
        let small = |t: String| RichText::new(t).size(12.0).color(p.on_surface_variant);
        let model = chat.conv.init.as_ref().map(|i| i.model.clone()).filter(|m| !m.is_empty()).unwrap_or_else(|| chat.model.clone());
        widgets::icon(ui, ICON_MEMORY, 14.0, p.on_surface_variant);
        ui.label(small(model));
        if let Some(f) = chat.conv.context_fraction() {
            ui.add_space(8.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(60.0, 6.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, CornerRadius::same(3), p.surface_container_highest);
            let mut fill = rect;
            fill.set_width(rect.width() * f);
            ui.painter().rect_filled(fill, CornerRadius::same(3), if f > 0.8 { p.error } else { p.primary });
            ui.label(small(format!("{:.0}% context", f * 100.0)));
        }
        if chat.conv.cost_usd > 0.0 {
            ui.add_space(8.0);
            ui.label(small(format!("${:.3}", chat.conv.cost_usd)));
        }
        let u = chat.conv.usage;
        if u.output > 0 {
            ui.label(small(format!("· {} in / {} out", fmt_tokens(u.context()), fmt_tokens(u.output))));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(16.0);
            if let Some(v) = &chat.version {
                ui.label(small(format!("Claude Code {v}")));
            }
            if let Some(init) = &chat.conv.init {
                for (name, status) in &init.mcp_servers {
                    let color = if status == "connected" { p.primary } else { p.error };
                    let r = ui.label(RichText::new(format!("● {name}")).size(12.0).color(color));
                    r.on_hover_text(format!("MCP server {name}: {status}"));
                }
            }
            if chat.process.is_some() {
                ui.label(small("●".into())).on_hover_text("Claude Code is running for this chat");
            }
        });
    });
}

fn fmt_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1e6)
    } else if n >= 1000 {
        format!("{:.1}k", n as f64 / 1e3)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting() {
        assert_eq!(format_duration(1500), "1.5s");
        assert_eq!(format_duration(125_000), "2m 5s");
        assert_eq!(fmt_tokens(1234), "1.2k");
        assert_eq!(fmt_tokens(12), "12");
    }
}
