//! Navigation drawer: new chat, projects with their sessions, settings and
//! the account status.

use super::widgets::{self, nav_item, section_header, ButtonKind};
use super::Action;
use crate::app::App;
use crate::claude::index::SessionMeta;
use egui::{pos2, vec2, CornerRadius, RichText, Sense, Ui};
use egui_material_icons::icons::*;

fn ago(ts: i64) -> String {
    let d = chrono::Utc::now().timestamp() - ts;
    match d {
        i64::MIN..=59 => "now".into(),
        60..=3599 => format!("{}m", d / 60),
        3600..=86_399 => format!("{}h", d / 3600),
        86_400..=2_591_999 => format!("{}d", d / 86_400),
        _ => chrono::DateTime::from_timestamp(ts, 0).map(|t| t.format("%b %e").to_string()).unwrap_or_default(),
    }
}

pub fn show(ui: &mut Ui, app: &mut App) {
    let p = app.palette.clone();
    let mut actions = Vec::new();

    ui.horizontal(|ui| {
        ui.add_space(12.0);
        widgets::icon(ui, ICON_AUTO_AWESOME, 26.0, p.primary);
        ui.label(widgets::title_text("Material 3 AI", 20.0).color(p.on_surface));
    });
    ui.add_space(14.0);

    // "New chat" extended FAB: in the current project, else ask for a folder.
    let fab_size = vec2(ui.available_width().min(150.0), 52.0);
    let (rect, fab) = ui.allocate_exact_size(fab_size, Sense::click());
    let fill = widgets::state_layer(p.primary_container, p.on_primary_container, &fab);
    ui.painter().rect_filled(rect, CornerRadius::same(16), fill);
    widgets::paint_icon(ui, ICON_EDIT_SQUARE, pos2(rect.left() + 28.0, rect.center().y), 22.0, p.on_primary_container);
    ui.painter().text(
        pos2(rect.left() + 50.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        "New chat",
        widgets::title_font(15.0),
        p.on_primary_container,
    );
    if fab.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        match app.current_project() {
            Some(dir) => actions.push(Action::NewChat(dir)),
            None => actions.push(Action::PickFolder),
        }
    }
    ui.add_space(12.0);
    let w = ui.available_width() - 4.0;
    super::common::search_field(ui, &mut app.sidebar_query, "Search chats", w);
    ui.add_space(6.0);

    let query = app.sidebar_query.trim().to_lowercase();
    let projects = app.config.projects.clone();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal(|ui| {
            section_header(ui, "Projects");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::icon_button_sized(ui, ICON_CREATE_NEW_FOLDER, "Add a project folder", true, 30.0, false).clicked() {
                    actions.push(Action::PickFolder);
                }
            });
        });
        if projects.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                ui.add(egui::Label::new(RichText::new("Add a folder to start chatting about its code.").size(12.5).color(p.on_surface_variant)).wrap());
            });
        }
        for dir in &projects {
            let name = std::path::Path::new(dir).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| dir.clone());
            let open = app.expanded_projects.contains(dir);
            let active = app.current_project().is_some_and(|c| c.to_string_lossy() == *dir);
            let resp = nav_item(ui, if open { ICON_FOLDER_OPEN } else { ICON_FOLDER }, &name, active && !open, None)
                .on_hover_text(crate::sys::paths::tilde(std::path::Path::new(dir)));
            if resp.clicked() {
                if open {
                    app.expanded_projects.remove(dir);
                } else {
                    app.expanded_projects.insert(dir.clone());
                }
            }
            resp.context_menu(|ui| {
                if ui.button("New chat here").clicked() {
                    actions.push(Action::NewChat(dir.into()));
                    ui.close();
                }
                if ui.button("Open folder").clicked() {
                    actions.push(Action::OpenPath(dir.into()));
                    ui.close();
                }
                if ui.button("Remove from list").clicked() {
                    actions.push(Action::RemoveProject(dir.clone()));
                    ui.close();
                }
            });
            if open || !query.is_empty() {
                let mut sessions: Vec<SessionMeta> = app.index.for_project(dir).into_iter().cloned().collect();
                sessions.extend(app.external_sessions(dir));
                sessions.retain(|s| query.is_empty() || s.title.to_lowercase().contains(&query));
                if !query.is_empty() && sessions.is_empty() {
                    continue;
                }
                ui.indent(("sessions", dir), |ui| {
                    if open && widgets::button(ui, ButtonKind::Text, Some(ICON_ADD), "New chat").clicked() {
                        actions.push(Action::NewChat(dir.into()));
                    }
                    for s in sessions.iter().take(if open { 40 } else { 8 }) {
                        let selected = app.active_session_id().as_deref() == Some(s.id.as_str());
                        let trailing = ago(s.updated);
                        let icon = if s.external { ICON_TERMINAL } else { ICON_CHAT_BUBBLE };
                        let r = nav_item(ui, icon, &s.title, selected, Some(&trailing));
                        let r = if s.external {
                            r.on_hover_text("Started in Claude Code")
                        } else if s.cost_usd > 0.0 {
                            r.on_hover_text(format!("${:.3}", s.cost_usd))
                        } else {
                            r
                        };
                        if r.clicked() {
                            actions.push(Action::OpenSession(s.clone()));
                        }
                        if !s.external {
                            r.context_menu(|ui| {
                                if ui.button("Remove from list").clicked() {
                                    actions.push(Action::ForgetSession(s.id.clone()));
                                    ui.close();
                                }
                            });
                        }
                    }
                });
            }
        }
        ui.add_space(8.0);
        if nav_item(ui, ICON_SETTINGS, "Settings", app.show_settings, None).clicked() {
            actions.push(Action::ShowSettings);
        }
    });

    // Account chip at the bottom.
    let info = app.info.value.clone();
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        let (text, color) = match &info {
            None => ("Checking Claude Code…".to_string(), p.on_surface_variant),
            Some(i) if i.version.is_none() => ("Claude Code not found".to_string(), p.error),
            Some(i) if !i.logged_in => ("Not signed in".to_string(), p.tertiary),
            Some(i) => (
                format!(
                    "{} · {}",
                    i.auth_method.as_deref().map(crate::claude::info::describe_method).unwrap_or("Signed in"),
                    i.version.clone().unwrap_or_default()
                ),
                p.on_surface_variant,
            ),
        };
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.label(RichText::new(text).size(12.0).color(color));
        });
    });
    app.actions.extend(actions);
}
