//! Cards for Claude's tool calls: commands with output, file edits as
//! diffs, searches, web requests, subagents and generic MCP calls.

use super::widgets;
use super::{Action, Actions};
use crate::claude::conversation::ToolCall;
use crate::theme::M3Palette;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, CornerRadius, FontId, Frame, Margin, RichText, Ui};
use egui_material_icons::{icons::*, MaterialIcon};
use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or_default()
}

pub fn icon_for(name: &str) -> MaterialIcon {
    match name {
        "Bash" | "BashOutput" | "KillShell" | "KillBash" => ICON_TERMINAL,
        "Read" => ICON_DESCRIPTION,
        "Write" => ICON_NOTE_ADD,
        "Edit" | "MultiEdit" | "NotebookEdit" => ICON_EDIT_DOCUMENT,
        "Grep" => ICON_SEARCH,
        "Glob" | "LS" => ICON_FOLDER_OPEN,
        "WebFetch" => ICON_LANGUAGE,
        "WebSearch" => ICON_TRAVEL_EXPLORE,
        "TodoWrite" => ICON_CHECKLIST,
        "Task" | "Agent" => ICON_SMART_TOY,
        "ExitPlanMode" => ICON_ASSIGNMENT,
        n if n.starts_with("mcp__") => ICON_EXTENSION,
        _ => ICON_BUILD,
    }
}

/// One-line summary of a call (also used in permission prompts).
pub fn summary(name: &str, input: &Value) -> String {
    let short = |p: &str| -> String {
        let path = std::path::Path::new(p);
        path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string())
    };
    match name {
        "Bash" => {
            let d = s(input, "description");
            if d.is_empty() { first_line(s(input, "command")) } else { d.to_string() }
        }
        "Read" => format!("Read {}", short(s(input, "file_path"))),
        "Write" => format!("Write {}", short(s(input, "file_path"))),
        "Edit" | "MultiEdit" => format!("Edit {}", short(s(input, "file_path"))),
        "NotebookEdit" => format!("Edit notebook {}", short(s(input, "notebook_path"))),
        "Grep" => format!("Search for “{}”", s(input, "pattern")),
        "Glob" => format!("Find files {}", s(input, "pattern")),
        "WebFetch" => format!("Fetch {}", s(input, "url")),
        "WebSearch" => format!("Search the web for “{}”", s(input, "query")),
        "TodoWrite" => "Update the task list".into(),
        "Task" | "Agent" => {
            let d = s(input, "description");
            if d.is_empty() { "Run a subagent".into() } else { d.to_string() }
        }
        "ExitPlanMode" => "Finish planning".into(),
        n if n.starts_with("mcp__") => {
            let parts: Vec<&str> = n.splitn(3, "__").collect();
            format!("{} › {}", parts.get(1).unwrap_or(&""), parts.get(2).unwrap_or(&""))
        }
        other => other.to_string(),
    }
}

fn first_line(t: &str) -> String {
    let l = t.lines().next().unwrap_or("");
    if t.lines().count() > 1 { format!("{l} …") } else { l.to_string() }
}

pub fn file_path(t: &ToolCall) -> Option<PathBuf> {
    let p = s(&t.input, "file_path");
    let p = if p.is_empty() { s(&t.input, "notebook_path") } else { p };
    (!p.is_empty()).then(|| PathBuf::from(p))
}

/// Draw a tool call card. `children` draws nested subagent messages.
pub fn card(
    ui: &mut Ui,
    t: &ToolCall,
    expanded: &mut HashSet<String>,
    running: bool,
    actions: &mut Actions,
    children: &mut dyn FnMut(&mut Ui, &mut HashSet<String>, &mut Actions),
) {
    let p = widgets::palette(ui);
    let open = expanded.contains(&t.id);
    let status_err = t.result.as_ref().is_some_and(|r| r.is_error);
    Frame::new()
        .fill(p.surface_container)
        .corner_radius(CornerRadius::same(16))
        .inner_margin(Margin { left: 12, right: 8, top: 6, bottom: 6 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let header = ui.horizontal(|ui| {
                widgets::icon(ui, icon_for(&t.name), 18.0, if status_err { p.error } else { p.primary });
                ui.label(widgets::title_text(display_name(&t.name), 13.5).color(p.on_surface));
                let text = if t.input.is_null() && !t.partial_input.is_empty() { "…".to_string() } else { summary(&t.name, &t.input) };
                ui.add(egui::Label::new(RichText::new(text).size(13.0).color(p.on_surface_variant)).truncate());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let icon = if open { ICON_EXPAND_LESS } else { ICON_EXPAND_MORE };
                    widgets::icon(ui, icon, 20.0, p.on_surface_variant);
                    match (&t.result, running) {
                        (Some(r), _) if r.is_error => {
                            widgets::icon(ui, ICON_ERROR, 16.0, p.error);
                        }
                        (Some(_), _) => {
                            widgets::icon(ui, ICON_CHECK_CIRCLE, 16.0, p.primary);
                        }
                        (None, true) => {
                            ui.add(egui::Spinner::new().size(14.0).color(p.primary));
                        }
                        (None, false) => {}
                    }
                    if let Some(path) = file_path(t) {
                        if widgets::icon_button_sized(ui, ICON_OPEN_IN_NEW, "Open file", true, 26.0, false).clicked() {
                            actions.push(Action::OpenPath(path));
                        }
                    }
                });
            });
            let click = ui.interact(header.response.rect, ui.scope_id().with(("tool", &t.id)), egui::Sense::click());
            if click.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                if open {
                    expanded.remove(&t.id);
                } else {
                    expanded.insert(t.id.clone());
                }
            }
            // Edits and commands show their substance even when collapsed.
            let always = matches!(t.name.as_str(), "Edit" | "MultiEdit" | "Bash");
            if open || always {
                ui.add_space(4.0);
                body(ui, t, &p, open);
            }
            if matches!(t.name.as_str(), "Task" | "Agent") {
                ui.add_space(4.0);
                Frame::new()
                    .inner_margin(Margin { left: 10, right: 0, top: 0, bottom: 0 })
                    .show(ui, |ui| children(ui, expanded, actions));
            }
        });
}

fn display_name(name: &str) -> &str {
    match name {
        "Task" | "Agent" => "Agent",
        "TodoWrite" => "Tasks",
        n if n.starts_with("mcp__") => "MCP",
        n => n,
    }
}

fn body(ui: &mut Ui, t: &ToolCall, p: &M3Palette, open: bool) {
    let input = &t.input;
    match t.name.as_str() {
        "Bash" => {
            mono_box(ui, p, &format!("$ {}", s(input, "command")), 6);
            if let Some(r) = &t.result {
                let (out, err) = match &r.structured {
                    Some(v) => (s(v, "stdout").to_string(), s(v, "stderr").to_string()),
                    None => (r.text.clone(), String::new()),
                };
                let text = if err.is_empty() { out } else if out.is_empty() { err } else { format!("{out}\n{err}") };
                if !text.trim().is_empty() {
                    mono_box(ui, p, text.trim_end(), if open { 400 } else { 8 });
                }
            }
        }
        "Edit" => diff(ui, p, s(input, "old_string"), s(input, "new_string"), if open { 400 } else { 14 }),
        "MultiEdit" => {
            if let Some(edits) = input.get("edits").and_then(Value::as_array) {
                for e in edits.iter().take(if open { 50 } else { 2 }) {
                    diff(ui, p, s(e, "old_string"), s(e, "new_string"), if open { 200 } else { 10 });
                }
            }
        }
        "Write" => {
            let content = s(input, "content");
            let ext = std::path::Path::new(s(input, "file_path")).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
            code(ui, p, content, &ext, 30);
        }
        "TodoWrite" => todo_list(ui, p, input),
        "WebFetch" => {
            ui.label(RichText::new(s(input, "prompt")).size(12.5).color(p.on_surface_variant));
            result_text(ui, t, p, 20);
        }
        _ => {
            if !input.is_null() && !matches!(t.name.as_str(), "Read" | "Grep" | "Glob" | "WebSearch" | "Task" | "Agent") {
                mono_box(ui, p, &serde_json::to_string_pretty(input).unwrap_or_default(), 20);
            }
            if matches!(t.name.as_str(), "Task" | "Agent") {
                ui.label(RichText::new(s(input, "prompt")).size(12.5).color(p.on_surface_variant));
            }
            result_text(ui, t, p, 30);
        }
    }
}

fn result_text(ui: &mut Ui, t: &ToolCall, p: &M3Palette, lines: usize) {
    if let Some(r) = &t.result {
        if !r.text.trim().is_empty() {
            let color = if r.is_error { p.error } else { p.on_surface };
            mono_box_colored(ui, p, r.text.trim_end(), lines, color);
        }
    }
}

pub fn mono_box(ui: &mut Ui, p: &M3Palette, text: &str, max_lines: usize) {
    mono_box_colored(ui, p, text, max_lines, p.on_surface);
}

fn mono_box_colored(ui: &mut Ui, p: &M3Palette, text: &str, max_lines: usize, color: Color32) {
    let total = text.lines().count();
    let shown: String = if total > max_lines {
        let mut t: String = text.lines().take(max_lines).collect::<Vec<_>>().join("\n");
        t.push_str(&format!("\n… {} more lines", total - max_lines));
        t
    } else {
        text.to_string()
    };
    Frame::new()
        .fill(p.surface_container_highest)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::horizontal().id_salt(ui.next_auto_id()).show(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(shown).monospace().size(12.0).color(color)).extend());
            });
        });
}

fn code(ui: &mut Ui, p: &M3Palette, text: &str, lang: &str, max_lines: usize) {
    let shown: String = text.lines().take(max_lines).collect::<Vec<_>>().join("\n");
    Frame::new()
        .fill(p.surface_container_highest)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::horizontal().id_salt(ui.next_auto_id()).show(ui, |ui| {
                ui.add(egui::Label::new(super::markdown::highlight(&shown, lang, p, 12.0)).extend());
            });
            let more = text.lines().count().saturating_sub(max_lines);
            if more > 0 {
                ui.label(RichText::new(format!("… {more} more lines")).size(11.5).color(p.on_surface_variant));
            }
        });
}

/// Unified diff of two strings with added/removed lines tinted.
pub fn diff(ui: &mut Ui, p: &M3Palette, old: &str, new: &str, max_lines: usize) {
    let d = similar::TextDiff::from_lines(old, new);
    let added = blend_green(p);
    let font = FontId::monospace(12.0);
    let mut job = LayoutJob::default();
    let mut lines = 0;
    let mut hidden = 0;
    for change in d.iter_all_changes() {
        if lines >= max_lines {
            hidden += 1;
            continue;
        }
        lines += 1;
        let (sign, fg, bg) = match change.tag() {
            similar::ChangeTag::Delete => ("-", p.on_error_container, p.error_container),
            similar::ChangeTag::Insert => ("+", p.on_surface, added),
            similar::ChangeTag::Equal => (" ", p.on_surface_variant, Color32::TRANSPARENT),
        };
        let mut f = TextFormat::simple(font.clone(), fg);
        f.background = bg;
        let mut line = format!("{sign} {}", change.value());
        if !line.ends_with('\n') {
            line.push('\n');
        }
        job.append(&line, 0.0, f);
    }
    Frame::new()
        .fill(p.surface_container_highest)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::horizontal().id_salt(ui.next_auto_id()).show(ui, |ui| {
                ui.add(egui::Label::new(job).extend());
            });
            if hidden > 0 {
                ui.label(RichText::new(format!("… {hidden} more lines")).size(11.5).color(p.on_surface_variant));
            }
        });
}

fn blend_green(p: &M3Palette) -> Color32 {
    let base = Color32::from_rgb(0x2E, 0x7D, 0x32);
    crate::theme::palette::blend(p.surface_container_highest, base, if p.is_dark() { 0.35 } else { 0.22 })
}

pub fn todo_list(ui: &mut Ui, p: &M3Palette, input: &Value) {
    let Some(list) = input.get("todos").and_then(Value::as_array) else { return };
    for t in list {
        let status = s(t, "status");
        let (icon, color) = match status {
            "completed" => (ICON_CHECK_CIRCLE, p.primary),
            "in_progress" => (ICON_PENDING, p.tertiary),
            _ => (ICON_RADIO_BUTTON_UNCHECKED, p.on_surface_variant),
        };
        ui.horizontal(|ui| {
            widgets::icon(ui, icon, 16.0, color);
            let mut text = RichText::new(s(t, "content")).size(13.0).color(if status == "completed" { p.on_surface_variant } else { p.on_surface });
            if status == "completed" {
                text = text.strikethrough();
            }
            ui.label(text);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn summaries() {
        assert_eq!(summary("Bash", &json!({"command": "cargo test", "description": "Run tests"})), "Run tests");
        assert_eq!(summary("Bash", &json!({"command": "ls\npwd"})), "ls …");
        assert_eq!(summary("Edit", &json!({"file_path": "/a/b/main.rs"})), "Edit main.rs");
        assert_eq!(summary("mcp__github__create_issue", &json!({})), "github › create_issue");
        assert_eq!(summary("WebSearch", &json!({"query": "egui"})), "Search the web for “egui”");
    }
}
