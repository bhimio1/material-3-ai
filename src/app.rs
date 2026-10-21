//! App state, theme, layout, tabs and action dispatch.

use crate::chat::{Chat, ChatEvent};
use crate::claude::index::{self, Index, SessionMeta};
use crate::claude::info::{self, Info};
use crate::claude::process;
use crate::config::Config;
use material_3_ui::sys::task::Task;
use material_3_ui::sys::{cmd, paths, term};
use material_3_ui::widgets::{self, ButtonKind};
use crate::ui::{self, Action, Actions, ToastKind};
use material_3_ui::keys::take_key;
use material_3_ui::notify::{Notification, Notifier, Outcome};
use material_3_ui::theme::{M3Palette, ThemeManager};
use material_3_ui::toasts::{Toast, Toasts};
use egui::{CornerRadius, Frame, Margin, RichText};
use material_3_ui::icons::icons::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub struct Startup {
    pub dir: Option<PathBuf>,
    pub resume: Option<String>,
}

pub struct App {
    pub ctx: egui::Context,
    /// The theme's current palette.
    pub palette: M3Palette,
    pub theme: ThemeManager,
    pub config: Config,
    pub toasts: Toasts<Action>,
    pub actions: Actions,
    pub info: Task<Info>,
    pub program: Option<PathBuf>,
    pub index: Index,
    pub tabs: Vec<Chat>,
    pub active: usize,
    pub show_settings: bool,
    pub settings: ui::settings::State,
    pub sidebar_query: String,
    pub expanded_projects: HashSet<String>,
    external: HashMap<String, Task<Vec<SessionMeta>>>,
    folder_pick: Task<Result<Option<PathBuf>, String>>,
    /// Typed-path fallback when no file chooser portal answers.
    typed_folder: Option<String>,
    focused: bool,
    /// Development: a prompt sent to the first chat (`M3_AI_DEMO_PROMPT`).
    demo_prompt: Option<String>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, startup: Startup) -> Self {
        let ctx = cc.egui_ctx.clone();
        let config = Config::load();
        let theme = ThemeManager::new(&ctx, &config.appearance);
        let mut app = Self {
            palette: theme.palette().clone(),
            theme,
            program: process::resolve_program(&config.claude_path),
            config,
            toasts: Toasts::default(),
            actions: Vec::new(),
            info: Task::default(),
            index: Index::load(),
            tabs: Vec::new(),
            active: 0,
            show_settings: false,
            settings: Default::default(),
            sidebar_query: String::new(),
            expanded_projects: HashSet::new(),
            external: HashMap::new(),
            folder_pick: Task::default(),
            typed_folder: None,
            focused: true,
            demo_prompt: std::env::var("M3_AI_DEMO_PROMPT").ok().filter(|p| !p.is_empty()),
            ctx,
        };
        app.apply_theme();
        app.refresh_info();
        app.show_settings = std::env::var("M3_AI_PAGE").is_ok_and(|p| p == "settings");
        if let Some(first) = app.config.projects.first().cloned() {
            app.expanded_projects.insert(first);
        }
        if let Some(id) = startup.resume {
            match app.index.sessions.iter().find(|s| s.id == id).cloned() {
                Some(meta) => app.actions.push(Action::OpenSession(meta)),
                None => {
                    let cwd = startup.dir.clone().unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| paths::home()));
                    app.actions.push(Action::OpenSession(SessionMeta {
                        id,
                        cwd: cwd.to_string_lossy().into_owned(),
                        title: "Resumed chat".into(),
                        ..Default::default()
                    }));
                }
            }
        } else if let Some(dir) = startup.dir {
            app.actions.push(Action::NewChat(dir));
        }
        app
    }

    pub fn toast(&mut self, text: impl Into<String>, kind: ToastKind) {
        self.toasts.push(Toast::new(text, kind));
    }

    // ------------------------------------------------------------------ theme

    /// Apply the appearance settings after they changed.
    pub fn apply_theme(&mut self) {
        self.theme.apply(&self.config.appearance);
        self.theme_applied();
    }

    fn theme_applied(&mut self) {
        if let Some(e) = self.theme.take_error() {
            self.toast(e, ToastKind::Error);
        }
        self.palette = self.theme.palette().clone();
    }

    // ------------------------------------------------------------------ state

    pub fn refresh_info(&mut self) {
        self.program = process::resolve_program(&self.config.claude_path);
        match self.program.clone() {
            Some(p) => self.info.spawn(&self.ctx, move || info::load(&p)),
            None => self.info.value = Some(Info { error: Some("not found".into()), ..Default::default() }),
        }
    }

    pub fn current_project(&self) -> Option<PathBuf> {
        if !self.show_settings {
            if let Some(c) = self.tabs.get(self.active) {
                return Some(c.cwd.clone());
            }
        }
        self.config.projects.first().map(PathBuf::from)
    }

    pub fn active_session_id(&self) -> Option<String> {
        if self.show_settings {
            return None;
        }
        self.tabs.get(self.active).and_then(|c| c.conv.session_id.clone())
    }

    fn projects_dir(&self) -> PathBuf {
        index::projects_dir(self.info.value.as_ref().and_then(|i| i.projects_dir.as_deref()))
    }

    /// Claude Code sessions for `dir` that this app didn't create (loaded in
    /// the background on first use).
    pub fn external_sessions(&mut self, dir: &str) -> Vec<SessionMeta> {
        let projects = self.projects_dir();
        let known: Vec<String> = self.index.sessions.iter().map(|s| s.id.clone()).collect();
        let task = self.external.entry(dir.to_string()).or_default();
        let d = dir.to_string();
        task.ensure(&self.ctx, move || {
            let refs: Vec<&str> = known.iter().map(String::as_str).collect();
            index::external_sessions(&projects, &d, &refs)
        });
        task.value.clone().unwrap_or_default()
    }

    fn save_meta(&mut self, i: usize) {
        if let Some(meta) = self.tabs.get(i).and_then(|c| c.meta()) {
            self.index.upsert(meta);
            if let Err(e) = self.index.save() {
                self.toast(format!("Couldn't save the chat list: {e}"), ToastKind::Error);
            }
        }
    }

    fn notify(&self, title: &str, body: &str) {
        if self.focused || !self.config.notifications {
            return;
        }
        let n = Notification {
            body: body.into(),
            icon: "dialog-information".into(),
            desktop_entry: Some("material_3_ai".into()),
            ..Notification::new("Material 3 AI", title)
        }
        .action("default", "Show");
        // Clicking the notification brings the window back.
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let sent = Notifier::connect().and_then(|n2| n2.send(&n).map(|id| (n2, id)));
            match sent {
                Ok((notifier, id)) => {
                    if let Outcome::Action(_) = notifier.wait(id, Duration::from_secs(30 * 60)) {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                        ctx.request_repaint();
                    }
                }
                Err(e) => log::warn!("couldn't show a notification: {e}"),
            }
        });
    }

    fn poll(&mut self) {
        // Palette file changes and system light/dark switches.
        if self.theme.poll(&self.config.appearance) {
            self.theme_applied();
        }
        self.focused = self.ctx.input(|i| i.focused);
        self.info.poll();
        for t in self.external.values_mut() {
            t.poll();
        }
        // Chats.
        let Some(program) = self.program.clone() else { return };
        if let (Some(prompt), Some(chat)) = (self.demo_prompt.clone(), self.tabs.get_mut(self.active)) {
            self.demo_prompt = None;
            if let Err(e) = chat.send(prompt, Vec::new(), &program, &self.config, &self.ctx) {
                self.toast(e, ToastKind::Error);
            }
        }
        for i in 0..self.tabs.len() {
            let events = self.tabs[i].poll(&program, &self.config, &self.ctx);
            for ev in events {
                match ev {
                    ChatEvent::MetaChanged => self.save_meta(i),
                    ChatEvent::TurnDone { error } => {
                        let title = self.tabs[i].title.clone();
                        self.notify(if error { "Claude stopped" } else { "Claude finished" }, &title);
                    }
                    ChatEvent::NeedsPermission(tool) => {
                        let title = self.tabs[i].title.clone();
                        self.notify("Claude needs permission", &format!("{tool} · {title}"));
                        if !self.focused {
                            self.ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
                        }
                    }
                    ChatEvent::Error(e) => self.toast(e, ToastKind::Error),
                }
            }
        }
        // Folder picker result.
        if let Some(result) = self.folder_pick.take() {
            match result {
                Ok(Some(dir)) => self.actions.push(Action::AddProject(dir)),
                Ok(None) => {}
                Err(_) => self.typed_folder = Some(paths::tilde(&paths::home())),
            }
        }
        if self.tabs.iter().any(|t| t.running()) {
            self.ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    // ---------------------------------------------------------------- actions

    fn open_chat(&mut self, chat: Chat) {
        let dir = chat.cwd.to_string_lossy().into_owned();
        self.config.touch_project(&dir);
        self.expanded_projects.insert(dir);
        let _ = self.config.save();
        self.tabs.push(chat);
        self.active = self.tabs.len() - 1;
        self.show_settings = false;
    }

    fn process(&mut self, action: Action) {
        match action {
            Action::Toast(t, k) => self.toast(t, k),
            Action::NewChat(dir) => {
                if !dir.is_dir() {
                    self.toast(format!("{} isn't a folder", dir.display()), ToastKind::Error);
                    return;
                }
                let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
                let chat = Chat::new(dir, &self.config);
                self.open_chat(chat);
            }
            Action::OpenSession(meta) => {
                if let Some(i) = self.tabs.iter().position(|t| t.conv.session_id.as_deref() == Some(meta.id.as_str())) {
                    self.active = i;
                    self.show_settings = false;
                    return;
                }
                let chat = Chat::resume(&meta, &self.config, &self.projects_dir());
                self.open_chat(chat);
            }
            Action::SelectTab(i) => {
                if i < self.tabs.len() {
                    self.active = i;
                    self.show_settings = false;
                }
            }
            Action::CloseTab(i) => {
                if i < self.tabs.len() {
                    self.save_meta(i);
                    self.tabs.remove(i);
                    if self.active >= self.tabs.len() {
                        self.active = self.tabs.len().saturating_sub(1);
                    }
                }
            }
            Action::ShowSettings => self.show_settings = true,
            Action::PickFolder => {
                if !self.folder_pick.running() {
                    self.folder_pick.spawn(&self.ctx, || crate::sys::portal::pick_folder("Choose a project folder"));
                }
            }
            Action::AddProject(dir) => self.actions.push(Action::NewChat(dir)),
            Action::RemoveProject(dir) => {
                self.config.projects.retain(|p| *p != dir);
                let _ = self.config.save();
            }
            Action::ForgetSession(id) => {
                self.index.remove(&id);
                let _ = self.index.save();
            }
            Action::OpenPath(path) => {
                if let Err(e) = cmd::spawn_detached("xdg-open", &[&path.to_string_lossy()]) {
                    self.toast(e, ToastKind::Error);
                }
            }
            Action::OpenUrl(url) => {
                if let Err(e) = cmd::spawn_detached("xdg-open", &[&url]) {
                    self.toast(e, ToastKind::Error);
                }
            }
            Action::SignIn => match &self.program {
                Some(p) => {
                    // Claude Code's own login flow runs in the terminal.
                    let script = format!("{} auth login", term::shell_quote(&p.to_string_lossy()));
                    match term::run(&self.config.terminal, &script) {
                        Ok(t) => self.toast(format!("Finish signing in in {t}, then come back."), ToastKind::Info),
                        Err(e) => self.toast(e, ToastKind::Error),
                    }
                }
                None => self.toast("Install Claude Code first.", ToastKind::Error),
            },
            Action::RefreshInfo => self.refresh_info(),
            Action::ConfigChanged => {
                if let Err(e) = self.config.save() {
                    self.toast(format!("Couldn't save settings: {e}"), ToastKind::Error);
                }
                self.apply_theme();
            }
        }
    }

    fn process_actions(&mut self) {
        while !self.actions.is_empty() {
            for a in std::mem::take(&mut self.actions) {
                self.process(a);
            }
        }
    }

    // ----------------------------------------------------------------- layout

    fn layout(&mut self, ui: &mut egui::Ui) {
        let p = self.palette.clone();
        let surface = Frame::new().fill(p.surface);
        egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(272.0)
            .size_range(220.0..=380.0)
            .show_separator_line(false)
            .frame(surface.inner_margin(Margin { left: 12, right: 4, top: 12, bottom: 12 }))
            .show(ui, |ui| ui::sidebar::show(ui, self));

        egui::CentralPanel::default()
            .frame(surface.inner_margin(Margin { left: 8, right: 12, top: 10, bottom: 12 }))
            .show(ui, |ui| {
                if !self.tabs.is_empty() && !self.show_settings {
                    self.tab_bar(ui);
                    ui.add_space(6.0);
                }
                Frame::new()
                    .fill(p.surface_container_low)
                    .corner_radius(CornerRadius::same(24))
                    .inner_margin(Margin::same(4))
                    .show(ui, |ui| {
                        ui.set_min_size(ui.available_size());
                        if self.show_settings {
                            ui::settings::show(ui, self);
                        } else if self.tabs.is_empty() {
                            self.welcome(ui);
                        } else {
                            let env_program = self.program.clone();
                            let config = self.config.clone();
                            let env = ui::chat::Env { font_size: config.chat_font_size, program: env_program.as_deref(), config: &config };
                            let mut actions = Vec::new();
                            if let Some(chat) = self.tabs.get_mut(self.active) {
                                ui::chat::show(ui, chat, &env, &mut actions);
                            }
                            self.actions.extend(actions);
                        }
                    });
            });
    }

    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        let p = self.palette.clone();
        let mut actions = Vec::new();
        egui::ScrollArea::horizontal().id_salt("tabs").show(ui, |ui| {
            ui.horizontal(|ui| {
                for (i, chat) in self.tabs.iter().enumerate() {
                    let selected = i == self.active;
                    let fill = if selected { p.secondary_container } else { p.surface_container };
                    let fg = if selected { p.on_secondary_container } else { p.on_surface_variant };
                    let r = Frame::new()
                        .fill(fill)
                        .corner_radius(CornerRadius::same(18))
                        .inner_margin(Margin { left: 14, right: 6, top: 6, bottom: 6 })
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                if !chat.conv.permissions.is_empty() {
                                    widgets::icon(ui, ICON_SHIELD, 15.0, p.tertiary);
                                } else if chat.running() {
                                    ui.add(egui::Spinner::new().size(13.0).color(p.primary));
                                }
                                let project = chat.cwd.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                                ui.add(
                                    egui::Label::new(widgets::title_text(chat.title.clone(), 13.0).color(fg))
                                        .truncate()
                                        .selectable(false),
                                )
                                .on_hover_text(format!("{} · {}", chat.title, project));
                                if widgets::icon_button_sized(ui, ICON_CLOSE, "Close", true, 24.0, false).clicked() {
                                    actions.push(Action::CloseTab(i));
                                }
                            });
                        })
                        .response;
                    let click = ui.interact(r.rect, ui.scope_id().with(("tab", chat.uid)), egui::Sense::click());
                    if click.clicked() {
                        actions.push(Action::SelectTab(i));
                    }
                    if click.middle_clicked() {
                        actions.push(Action::CloseTab(i));
                    }
                }
                if widgets::icon_button(ui, ICON_ADD, "New chat", true).clicked() {
                    match self.current_project() {
                        Some(d) => actions.push(Action::NewChat(d)),
                        None => actions.push(Action::PickFolder),
                    }
                }
            });
        });
        self.actions.extend(actions);
    }

    fn welcome(&mut self, ui: &mut egui::Ui) {
        let p = self.palette.clone();
        let mut actions = Vec::new();
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.22);
            widgets::icon(ui, ICON_AUTO_AWESOME, 56.0, p.primary);
            ui.add_space(10.0);
            ui.label(RichText::new("Material 3 AI").size(30.0).color(p.on_surface));
            ui.label(RichText::new("A desktop app for Claude Code. Pick a project folder to start.").size(15.0).color(p.on_surface_variant));
            ui.add_space(18.0);
            if self.program.is_none() {
                ui.label(RichText::new("Claude Code isn't installed yet.").color(p.error));
                ui.add_space(8.0);
                if widgets::button(ui, ButtonKind::Tonal, Some(ICON_OPEN_IN_NEW), "How to install").clicked() {
                    actions.push(Action::OpenUrl(ui::settings::INSTALL_DOCS.into()));
                }
            } else {
                if widgets::button(ui, ButtonKind::Filled, Some(ICON_CREATE_NEW_FOLDER), "Open a project").clicked() {
                    actions.push(Action::PickFolder);
                }
                ui.add_space(14.0);
                for dir in self.config.projects.iter().take(5) {
                    let label = paths::tilde(Path::new(dir));
                    if widgets::button(ui, ButtonKind::Text, Some(ICON_FOLDER), &label).clicked() {
                        actions.push(Action::NewChat(dir.into()));
                    }
                }
            }
        });
        self.actions.extend(actions);
    }

    fn typed_folder_dialog(&mut self) {
        let Some(text) = self.typed_folder.as_mut() else { return };
        let p = self.palette.clone();
        let mut close = false;
        let mut open = None;
        material_3_ui::common::modal(&self.ctx, &p, "typed-folder", ICON_FOLDER_OPEN, "Open a project folder", |ui| {
            ui.label(RichText::new("No file chooser is available. Type the folder's path.").color(p.on_surface_variant));
            ui.add_space(8.0);
            let r = ui.add(egui::TextEdit::singleline(text).desired_width(380.0));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            material_3_ui::common::dialog_buttons(ui, |ui| {
                if widgets::button(ui, ButtonKind::Filled, None, "Open").clicked() || enter {
                    open = Some(paths::expand(text.trim()));
                }
                if widgets::button(ui, ButtonKind::Text, None, "Cancel").clicked() {
                    close = true;
                }
            });
        });
        if let Some(dir) = open {
            self.typed_folder = None;
            self.actions.push(Action::AddProject(dir));
        } else if close {
            self.typed_folder = None;
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let new_chat = take_key(ui.ctx(), egui::Modifiers::COMMAND, egui::Key::N);
        let settings = take_key(ui.ctx(), egui::Modifiers::COMMAND, egui::Key::Comma);
        if new_chat {
            match self.current_project() {
                Some(d) => self.actions.push(Action::NewChat(d)),
                None => self.actions.push(Action::PickFolder),
            }
        }
        if settings {
            self.actions.push(Action::ShowSettings);
        }
        self.layout(ui);
        self.typed_folder_dialog();
        let fired = self.toasts.show(&self.ctx.clone(), &self.palette, 20.0);
        self.actions.extend(fired);
        self.process_actions();
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        self.palette.surface.to_normalized_gamma_f32()
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        for i in 0..self.tabs.len() {
            self.save_meta(i);
        }
        let _ = self.config.save();
    }
}
