//! The message box: text, attachments, `/` command and `@` file
//! completion, model and permission-mode pickers, send and stop.

use super::chat::Env;
use super::widgets::{self, ButtonKind};
use super::{toast, Actions, ToastKind};
use crate::chat::Chat;
use crate::config::PERMISSION_MODES;
use egui::{CornerRadius, Frame, Key, Margin, Modifiers, RichText, Ui};
use egui_material_icons::icons::*;
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;

/// What the completion popup offers.
#[derive(Clone, Debug, PartialEq)]
enum Completion {
    Command { name: String, description: String, hint: String },
    File(String),
}

/// The `@token` being typed at the end of the text, if any.
pub fn mention_query(text: &str) -> Option<&str> {
    let last = text.rsplit(|c: char| c.is_whitespace()).next()?;
    last.strip_prefix('@')
}

/// The `/command` being typed (only at the start, before any space).
pub fn command_query(text: &str) -> Option<&str> {
    let rest = text.strip_prefix('/')?;
    (!rest.contains(char::is_whitespace)).then_some(rest)
}

fn completions(chat: &Chat) -> Vec<Completion> {
    let text = &chat.composer.text;
    let matcher = SkimMatcherV2::default();
    if let Some(q) = command_query(text) {
        let mut scored: Vec<(i64, Completion)> = chat
            .command_list()
            .into_iter()
            .filter_map(|c| {
                let score = if q.is_empty() { Some(0) } else { matcher.fuzzy_match(&c.name, q) }?;
                Some((score, Completion::Command { name: c.name, description: c.description, hint: c.hint }))
            })
            .collect();
        scored.sort_by_key(|s| std::cmp::Reverse(s.0));
        return scored.into_iter().take(8).map(|(_, c)| c).collect();
    }
    if let Some(q) = mention_query(text) {
        let Some(files) = &chat.files.value else { return Vec::new() };
        let mut scored: Vec<(i64, &String)> = files
            .iter()
            .filter_map(|f| {
                let s = if q.is_empty() { Some(0) } else { matcher.fuzzy_match(f, q) }?;
                Some((s, f))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.len().cmp(&b.1.len())));
        return scored.into_iter().take(8).map(|(_, f)| Completion::File(f.clone())).collect();
    }
    Vec::new()
}

fn apply_completion(text: &mut String, c: &Completion) {
    match c {
        Completion::Command { name, .. } => *text = format!("/{name} "),
        Completion::File(f) => {
            let cut = text.rfind('@').unwrap_or(text.len());
            text.truncate(cut);
            text.push('@');
            text.push_str(f);
            text.push(' ');
        }
    }
}

pub fn show(ui: &mut Ui, chat: &mut Chat, env: &Env<'_>, actions: &mut Actions) {
    let p = widgets::palette(ui);
    let ctx = ui.ctx().clone();
    let input_id = ui.scope_id().with(("composer", chat.uid));
    let focused = ui.memory(|m| m.has_focus(input_id));

    if mention_query(&chat.composer.text).is_some() {
        chat.ensure_files(&ctx);
    }
    chat.files.poll();

    // Picked images arrive from the portal task.
    if let Some(result) = chat.composer.pick.take() {
        match result {
            Ok(paths) => {
                for path in paths {
                    match crate::images::from_file(&path, &ctx) {
                        Ok(a) => chat.composer.attachments.push(a),
                        Err(e) => toast(actions, e, ToastKind::Error),
                    }
                }
            }
            Err(e) => toast(actions, format!("Couldn't open the file picker: {e}"), ToastKind::Error),
        }
    }

    // Dropped files: images attach, anything else becomes an @mention.
    let dropped: Vec<std::path::PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
    for path in dropped {
        if crate::images::is_image(&path) {
            match crate::images::from_file(&path, &ctx) {
                Ok(a) => chat.composer.attachments.push(a),
                Err(e) => toast(actions, e, ToastKind::Error),
            }
        } else {
            let shown = path.strip_prefix(&chat.cwd).map(|r| r.to_path_buf()).unwrap_or(path);
            if !chat.composer.text.is_empty() && !chat.composer.text.ends_with(' ') {
                chat.composer.text.push(' ');
            }
            chat.composer.text.push_str(&format!("@{} ", shown.display()));
        }
    }

    // Pasting an image (egui only forwards pasted text).
    if focused && ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_)) || matches!(e, egui::Event::Key { key: Key::V, pressed: true, modifiers, .. } if modifiers.command))) {
        if let Some(r) = crate::images::from_clipboard(&ctx) {
            match r {
                Ok(a) => chat.composer.attachments.push(a),
                Err(e) => toast(actions, e, ToastKind::Error),
            }
        }
    }

    let items = completions(chat);
    if chat.composer.selected >= items.len() {
        chat.composer.selected = 0;
    }
    let mut send = false;
    if focused {
        if !items.is_empty() {
            ui.input_mut(|i| {
                if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                    chat.composer.selected = (chat.composer.selected + 1) % items.len();
                }
                if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                    chat.composer.selected = (chat.composer.selected + items.len() - 1) % items.len();
                }
                if i.consume_key(Modifiers::NONE, Key::Tab) || i.consume_key(Modifiers::NONE, Key::Enter) {
                    apply_completion(&mut chat.composer.text, &items[chat.composer.selected]);
                }
            });
        } else if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
            send = true;
        }
        if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) && chat.running() {
            chat.stop();
        }
    }

    // Completion popup above the box.
    if focused && !items.is_empty() {
        Frame::new()
            .fill(p.surface_container_high)
            .corner_radius(CornerRadius::same(16))
            .inner_margin(Margin::same(6))
            .shadow(ui.visuals().popup_shadow)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                for (i, c) in items.iter().enumerate() {
                    let selected = i == chat.composer.selected;
                    let (icon, title, sub) = match c {
                        Completion::Command { name, description, hint } => {
                            (ICON_TERMINAL, format!("/{name} {hint}"), description.clone())
                        }
                        Completion::File(f) => (ICON_DESCRIPTION, f.clone(), String::new()),
                    };
                    let r = Frame::new()
                        .fill(if selected { p.secondary_container } else { egui::Color32::TRANSPARENT })
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin::symmetric(10, 4))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                widgets::icon(ui, icon, 16.0, p.on_surface_variant);
                                ui.label(RichText::new(title).size(13.0).color(p.on_surface));
                                if !sub.is_empty() {
                                    ui.add(egui::Label::new(RichText::new(sub).size(12.0).color(p.on_surface_variant)).truncate());
                                }
                            });
                        })
                        .response;
                    if ui.interact(r.rect, ui.scope_id().with(("completion", i)), egui::Sense::click()).clicked() {
                        apply_completion(&mut chat.composer.text, c);
                        ui.memory_mut(|m| m.request_focus(input_id));
                    }
                }
            });
        ui.add_space(4.0);
    }

    // Queued messages.
    if !chat.queue.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Queued:").size(12.0).color(p.on_surface_variant));
            let mut remove = None;
            for (i, (text, _)) in chat.queue.iter().enumerate() {
                let label = crate::claude::index::title_from(text);
                if widgets::filter_chip(ui, &label, false, Some(ICON_SCHEDULE)).on_hover_text("Click to remove").clicked() {
                    remove = Some(i);
                }
            }
            if let Some(i) = remove {
                chat.queue.remove(i);
            }
        });
        ui.add_space(4.0);
    }

    Frame::new()
        .fill(p.surface_container_high)
        .corner_radius(CornerRadius::same(24))
        .inner_margin(Margin { left: 14, right: 10, top: 10, bottom: 8 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !chat.composer.attachments.is_empty() {
                ui.horizontal(|ui| {
                    let mut remove = None;
                    for (i, a) in chat.composer.attachments.iter().enumerate() {
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(56.0, 56.0), egui::Sense::hover());
                        if let Some(t) = &a.texture {
                            egui::Image::new(t).corner_radius(CornerRadius::same(10)).paint_at(ui, rect);
                        }
                        let x = egui::Rect::from_center_size(rect.right_top() + egui::vec2(-8.0, 8.0), egui::vec2(18.0, 18.0));
                        let resp = ui.interact(x, ui.scope_id().with(("rm", i)), egui::Sense::click()).on_hover_text(&a.name);
                        ui.painter().circle_filled(x.center(), 9.0, p.inverse_surface);
                        widgets::paint_icon(ui, ICON_CLOSE, x.center(), 13.0, p.inverse_on_surface);
                        if resp.clicked() {
                            remove = Some(i);
                        }
                    }
                    if let Some(i) = remove {
                        chat.composer.attachments.remove(i);
                    }
                });
                ui.add_space(6.0);
            }
            let hint = if chat.running() { "Queue a follow-up…" } else { "Ask Claude… (/ for commands, @ for files)" };
            ui.add(
                egui::TextEdit::multiline(&mut chat.composer.text)
                    .id(input_id)
                    .hint_text(hint)
                    .frame(egui::Frame::NONE)
                    .font(egui::FontId::proportional(env.font_size))
                    .desired_rows(1)
                    .desired_width(f32::INFINITY)
                    .lock_focus(true),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if widgets::icon_button_sized(ui, ICON_ADD_PHOTO_ALTERNATE, "Attach images", !chat.composer.pick.running(), 32.0, false).clicked() {
                    chat.composer.pick.spawn(&ctx, crate::sys::portal::pick_images);
                }
                // Model.
                let current = chat.models.iter().find(|m| m.value == chat.model).map(|m| m.label.clone()).unwrap_or_else(|| chat.model.clone());
                let mut new_model = None;
                egui::ComboBox::from_id_salt(("model", chat.uid))
                    .selected_text(RichText::new(current).size(12.5))
                    .width(150.0)
                    .show_ui(ui, |ui| {
                        for m in &chat.models {
                            let r = ui.selectable_label(m.value == chat.model, &m.label);
                            let r = if m.description.is_empty() { r } else { r.on_hover_text(&m.description) };
                            if r.clicked() {
                                new_model = Some(m.value.clone());
                            }
                        }
                    });
                if let Some(m) = new_model {
                    chat.set_model(&m);
                }
                // Permission mode.
                let mut mode = chat.permission_mode.clone();
                egui::ComboBox::from_id_salt(("mode", chat.uid))
                    .selected_text(RichText::new(crate::config::mode_label(&mode).to_string()).size(12.5))
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        for (key, label) in PERMISSION_MODES {
                            ui.selectable_value(&mut mode, key.to_string(), label);
                        }
                    });
                if mode != chat.permission_mode {
                    if mode == "bypassPermissions" {
                        toast(actions, "Bypass: Claude runs every tool without asking. Only use this in a sandbox.", ToastKind::Info);
                    }
                    chat.set_permission_mode(&mode);
                }
                if chat.permission_mode == "bypassPermissions" {
                    widgets::icon(ui, ICON_WARNING, 18.0, p.error).on_hover_text("Bypass permissions is on");
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let has_content = !chat.composer.text.trim().is_empty() || !chat.composer.attachments.is_empty();
                    if chat.running() && widgets::button(ui, ButtonKind::Tonal, Some(ICON_STOP), "Stop").on_hover_text("Esc").clicked() {
                        chat.stop();
                    }
                    if widgets::button_enabled(ui, ButtonKind::Filled, Some(ICON_ARROW_UPWARD), if chat.running() { "Queue" } else { "Send" }, has_content)
                        .clicked()
                    {
                        send = true;
                    }
                });
            });
        });

    if send {
        let text = chat.composer.text.trim().to_string();
        let images: Vec<_> = chat.composer.attachments.drain(..).map(|a| a.image).collect();
        if text.is_empty() && images.is_empty() {
            return;
        }
        match env.program {
            None => toast(actions, "Claude Code isn't installed. See Settings.", ToastKind::Error),
            Some(program) => match chat.send(text, images, program, env.config, &ctx) {
                Ok(()) => chat.composer.text.clear(),
                Err(e) => toast(actions, e, ToastKind::Error),
            },
        }
        ui.memory_mut(|m| m.request_focus(input_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries() {
        assert_eq!(command_query("/rev"), Some("rev"));
        assert_eq!(command_query("/review 12"), None);
        assert_eq!(command_query("hi /x"), None);
        assert_eq!(mention_query("look at @src/ma"), Some("src/ma"));
        assert_eq!(mention_query("email me@x"), None);
        assert_eq!(mention_query("@"), Some(""));
        let mut t = "fix @ma".to_string();
        apply_completion(&mut t, &Completion::File("src/main.rs".into()));
        assert_eq!(t, "fix @src/main.rs ");
        let mut t = "/re".to_string();
        apply_completion(&mut t, &Completion::Command { name: "review".into(), description: String::new(), hint: String::new() });
        assert_eq!(t, "/review ");
    }
}
