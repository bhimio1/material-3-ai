//! Settings: the Claude Code installation and sign-in, defaults for new
//! chats, appearance and notifications.

use crate::app::App;
use crate::config::{ColorScheme, Config, ThemeSource, PERMISSION_MODES};
use crate::sys::paths;
use crate::theme::palette::{parse_hex, to_hex};
use crate::ui::common::{self, card, row};
use crate::ui::widgets::{self, segmented, switch, ButtonKind};
use crate::ui::Action;
use egui::{vec2, Color32, RichText, Ui};
use egui_material_icons::icons::*;

const SEED_PRESETS: &[&str] = &["#D97757", "#6750A4", "#4285F4", "#B3261E", "#1E8E3E", "#00897B", "#C2185B", "#607D8B"];

pub const INSTALL_DOCS: &str = "https://code.claude.com/docs/en/setup";

#[derive(Default)]
pub struct State {
    seed_text: Option<String>,
    dirs_text: Option<String>,
}

pub fn show(ui: &mut Ui, app: &mut App) {
    let p = app.palette.clone();
    let theme_file = app.theme_file();
    let info = app.info.value.clone();
    let checking = app.info.running();
    let program = app.program.clone();
    let config = &mut app.config;
    let st = &mut app.settings;
    let mut actions = Vec::new();
    let mut changed = false;

    common::page(ui, ICON_SETTINGS, "Settings", "Material 3 AI runs Claude Code from your computer, signed in with your own account.", |ui| {
        card(ui, "Claude Code", |ui| {
            match (&program, &info) {
                (None, _) => {
                    common::banner(ui, ICON_WARNING, "Claude Code isn't installed (no `claude` on your PATH). Install it with `npm install -g @anthropic-ai/claude-code` or the official installer, then check again.", true, |ui| {
                        if widgets::button(ui, ButtonKind::Text, Some(ICON_OPEN_IN_NEW), "Install guide").clicked() {
                            actions.push(Action::OpenUrl(INSTALL_DOCS.into()));
                        }
                    });
                }
                (Some(path), _) => {
                    let desc = match &info {
                        Some(i) if i.version.is_some() => format!("{} · version {}", paths::tilde(path), i.version.clone().unwrap_or_default()),
                        Some(i) => format!("{} · {}", paths::tilde(path), i.error.clone().unwrap_or_else(|| "not responding".into())),
                        None => paths::tilde(path),
                    };
                    row(ui, "Installation", &desc, |ui| {
                        if common::refresh_button(ui, checking) {
                            actions.push(Action::RefreshInfo);
                        }
                    });
                    ui.separator();
                    let (title, desc) = match &info {
                        Some(i) if i.logged_in => (
                            "Signed in",
                            format!(
                                "Using your {}. Material 3 AI never reads or stores your credentials.",
                                i.auth_method.as_deref().map(crate::claude::info::describe_method).unwrap_or("account")
                            ),
                        ),
                        Some(_) => ("Not signed in", "Sign in through Claude Code's own login in a terminal.".to_string()),
                        None => ("Checking sign-in…", String::new()),
                    };
                    row(ui, title, &desc, |ui| {
                        let label = if info.as_ref().is_some_and(|i| i.logged_in) { "Switch account" } else { "Sign in" };
                        if widgets::button(ui, ButtonKind::Tonal, Some(ICON_LOGIN), label).clicked() {
                            actions.push(Action::SignIn);
                        }
                    });
                }
            }
            ui.separator();
            row(ui, "Path to claude", "Leave empty to find it on your PATH.", |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut config.claude_path).hint_text("Automatic").desired_width(240.0));
                if r.lost_focus() {
                    changed = true;
                    actions.push(Action::RefreshInfo);
                }
            });
        });

        card(ui, "New chats", |ui| {
            row(ui, "Model", "default, sonnet, opus, haiku or a full model id. Each chat can change it.", |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut config.default_model).desired_width(180.0));
                if r.lost_focus() {
                    changed = true;
                }
            });
            ui.separator();
            row(ui, "Permissions", "Ask before running tools, accept file edits, plan only, let Claude decide (Auto) or never ask (Bypass).", |ui| {
                let mut mode = config.default_permission_mode.clone();
                egui::ComboBox::from_id_salt("default-mode")
                    .selected_text(crate::config::mode_label(&mode).to_string())
                    .width(140.0)
                    .show_ui(ui, |ui| {
                        for (k, l) in PERMISSION_MODES {
                            ui.selectable_value(&mut mode, k.to_string(), l);
                        }
                    });
                if mode != config.default_permission_mode {
                    config.default_permission_mode = mode;
                    changed = true;
                }
            });
            if config.default_permission_mode == "bypassPermissions" {
                common::banner(ui, ICON_WARNING, "Bypass lets Claude run any command and edit any file without asking. Use it only in a sandbox or container.", true, |_| {});
            }
            ui.separator();
            row(ui, "Extra folders", "Folders Claude may also read and edit, one per line (--add-dir).", |_| {});
            let text = st.dirs_text.get_or_insert_with(|| config.extra_dirs.join("\n"));
            let r = ui.add(egui::TextEdit::multiline(text).desired_rows(2).desired_width(f32::INFINITY).hint_text("~/notes"));
            if r.lost_focus() {
                config.extra_dirs = text.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
                changed = true;
            }
            ui.add_space(6.0);
            ui.separator();
            row(ui, "Extra instructions", "Appended to Claude Code's system prompt for every chat.", |_| {});
            let r = ui.add(egui::TextEdit::multiline(&mut config.append_system_prompt).desired_rows(3).desired_width(f32::INFINITY).hint_text("e.g. Answer briefly. Prefer small commits."));
            if r.lost_focus() {
                changed = true;
            }
            ui.add_space(6.0);
            ui.separator();
            row(ui, "MCP servers file", "A JSON file with extra MCP servers (--mcp-config). Servers from Claude Code's own settings load anyway.", |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut config.mcp_config).hint_text("none").desired_width(240.0));
                if r.lost_focus() {
                    changed = true;
                }
            });
        });

        card(ui, "Appearance", |ui| {
            let source_desc = match (&config.theme_source, &theme_file) {
                (ThemeSource::Auto, Some(f)) => format!("Following {} (updates live with your wallpaper).", paths::tilde(f)),
                (ThemeSource::Auto, None) => "No end-4 or file manager theme found, so the seed color below is used.".to_string(),
                (ThemeSource::Seed, _) => "Using a palette generated from the seed color below.".to_string(),
            };
            row(ui, "Color source", &source_desc, |ui| {
                changed |= segmented(ui, &mut config.theme_source, &[(ThemeSource::Auto, ICON_WALLPAPER, "Auto"), (ThemeSource::Seed, ICON_PALETTE, "Seed")]);
            });
            ui.separator();
            let seed_mode = theme_file.is_none();
            ui.add_enabled_ui(seed_mode, |ui| {
                row(ui, "Color scheme", if seed_mode { "Light, dark, or follow the system." } else { "The theme file decides light or dark." }, |ui| {
                    changed |= segmented(ui, &mut config.color_scheme, &[
                        (ColorScheme::System, ICON_BRIGHTNESS_AUTO, "System"),
                        (ColorScheme::Light, ICON_LIGHT_MODE, "Light"),
                        (ColorScheme::Dark, ICON_DARK_MODE, "Dark"),
                    ]);
                });
                ui.separator();
                row(ui, "Seed color", "Every color in the interface is derived from this one.", |ui| {
                    let text = st.seed_text.get_or_insert_with(|| config.seed_color.clone());
                    let resp = ui.add(egui::TextEdit::singleline(text).desired_width(92.0).char_limit(7));
                    if resp.changed() {
                        if let Some(c) = parse_hex(text) {
                            config.seed_color = to_hex(c);
                            changed = true;
                        }
                    }
                    let mut color = parse_hex(&config.seed_color).unwrap_or(Color32::from_rgb(0xD9, 0x77, 0x57));
                    if ui.color_edit_button_srgba(&mut color).changed() {
                        config.seed_color = to_hex(color);
                        *text = config.seed_color.clone();
                        changed = true;
                    }
                    for hex in SEED_PRESETS.iter().rev() {
                        let Some(c) = parse_hex(hex) else { continue };
                        let (rect, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), egui::Sense::click());
                        ui.painter().circle_filled(rect.center(), 11.0, c);
                        if config.seed_color.eq_ignore_ascii_case(hex) {
                            ui.painter().circle_stroke(rect.center(), 12.5, egui::Stroke::new(2.0, p.on_surface));
                        }
                        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            config.seed_color = hex.to_string();
                            *text = hex.to_string();
                            changed = true;
                        }
                    }
                });
            });
            ui.separator();
            row(ui, "Chat text size", "Size of messages and the message box.", |ui| {
                let r = ui.add(egui::Slider::new(&mut config.chat_font_size, 11.0..=22.0).step_by(0.5).suffix(" pt"));
                if r.drag_stopped() || (r.changed() && !r.dragged()) {
                    changed = true;
                }
            });
            ui.separator();
            row(ui, "Filled icons", "Use filled Material Symbols instead of outlined ones.", |ui| {
                changed |= switch(ui, &mut config.filled_icons).changed();
            });
            ui.separator();
            row(ui, "Interface scale", "Size of everything in the window.", |ui| {
                let r = ui.add(egui::Slider::new(&mut config.ui_scale, 0.75..=1.75).step_by(0.05).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
                if r.drag_stopped() || (r.changed() && !r.dragged()) {
                    changed = true;
                }
            });
        });

        card(ui, "Notifications", |ui| {
            row(ui, "Desktop notifications", "When a reply finishes or Claude needs permission while this window isn't focused.", |ui| {
                changed |= switch(ui, &mut config.notifications).changed();
            });
        });

        card(ui, "Data", |ui| {
            let index = crate::claude::index::path();
            row(ui, "Chat list", &format!("{} (transcripts stay in Claude Code's own folder).", paths::tilde(&index)), |ui| {
                if widgets::button(ui, ButtonKind::Text, Some(ICON_FOLDER_OPEN), "Show").clicked() {
                    if let Some(dir) = index.parent() {
                        let _ = std::fs::create_dir_all(dir);
                        actions.push(Action::OpenPath(dir.to_path_buf()));
                    }
                }
            });
            ui.separator();
            let path = Config::path().unwrap_or_default();
            row(ui, "Preferences file", &paths::tilde(&path), |ui| {
                if widgets::button(ui, ButtonKind::Text, Some(ICON_FOLDER_OPEN), "Show").clicked() {
                    if let Some(dir) = path.parent() {
                        actions.push(Action::OpenPath(dir.to_path_buf()));
                    }
                }
            });
        });

        ui.add_space(24.0);
        ui.label(
            RichText::new(format!("Material 3 AI {} — a desktop app for Claude Code. Not affiliated with Anthropic.", env!("CARGO_PKG_VERSION")))
                .size(12.0)
                .color(p.on_surface_variant),
        );
    });

    if changed {
        actions.push(Action::ConfigChanged);
    }
    app.actions.extend(actions);
}
