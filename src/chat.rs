//! One chat tab: a conversation, the `claude` process behind it, the
//! composer state and what the CLI told us at startup.

use crate::claude::conversation::Conversation;
use crate::claude::index::{self, SessionMeta};
use crate::claude::process::{Event, Launch, Process};
use crate::claude::protocol::{Decision, ImageBlock, Incoming, PermissionRequest, SpawnOptions};
use crate::config::Config;
use material_3_ui::sys::task::Task;
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq)]
pub struct ModelOption {
    pub value: String,
    pub label: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    pub name: String,
    pub description: String,
    pub hint: String,
}

#[derive(Clone)]
pub struct Attachment {
    pub name: String,
    pub image: ImageBlock,
    pub texture: Option<egui::TextureHandle>,
}

#[derive(Default)]
pub struct Composer {
    pub text: String,
    pub attachments: Vec<Attachment>,
    /// Index into the autocomplete list.
    pub selected: usize,
    pub deny_reason: String,
    /// Images picked through the portal.
    pub pick: Task<Result<Vec<PathBuf>, String>>,
}

/// Things a chat wants the app to do after polling.
#[derive(Debug, PartialEq)]
pub enum ChatEvent {
    /// A turn finished (notify if unfocused).
    TurnDone { error: bool },
    /// Claude is waiting for permission.
    NeedsPermission(String),
    /// Session id/cost/title changed: update the index.
    MetaChanged,
    Error(String),
}

pub struct Chat {
    pub uid: u64,
    pub cwd: PathBuf,
    pub conv: Conversation,
    pub process: Option<Process>,
    pub title: String,
    pub model: String,
    pub permission_mode: String,
    pub composer: Composer,
    pub queue: Vec<(String, Vec<ImageBlock>)>,
    pub models: Vec<ModelOption>,
    pub commands: Vec<Command>,
    pub version: Option<String>,
    /// The process died; shown as a banner with a restart button.
    pub exited: Option<String>,
    pub created: i64,
    pub expanded: HashSet<String>,
    /// Project files for `@` mentions.
    pub files: Task<Vec<String>>,
    pub scroll_to_bottom: bool,
    /// When an interrupt was sent (for the SIGINT fallback).
    interrupt_sent: Option<std::time::Instant>,
}

impl Chat {
    pub fn new(cwd: PathBuf, config: &Config) -> Self {
        Chat {
            uid: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            cwd,
            conv: Conversation::default(),
            process: None,
            title: "New chat".into(),
            model: config.default_model.clone(),
            permission_mode: config.default_permission_mode.clone(),
            composer: Composer::default(),
            queue: Vec::new(),
            models: default_models(),
            commands: Vec::new(),
            version: None,
            exited: None,
            created: chrono::Utc::now().timestamp(),
            expanded: HashSet::new(),
            files: Task::default(),
            scroll_to_bottom: true,
            interrupt_sent: None,
        }
    }

    /// A chat for a past session; its transcript is loaded when available.
    pub fn resume(meta: &SessionMeta, config: &Config, projects_dir: &Path) -> Self {
        let mut c = Chat::new(PathBuf::from(&meta.cwd), config);
        c.conv.session_id = Some(meta.id.clone());
        c.conv.cost_usd = meta.cost_usd;
        c.conv.cost_base = meta.cost_usd;
        c.title = meta.title.clone();
        c.created = meta.created;
        if !meta.model.is_empty() {
            c.model = meta.model.clone();
        }
        let path = index::transcript_path(projects_dir, &meta.cwd, &meta.id);
        if let Ok(text) = std::fs::read_to_string(path) {
            c.conv.load_transcript(&text);
        }
        c
    }

    pub fn running(&self) -> bool {
        self.conv.running
    }

    pub fn meta(&self) -> Option<SessionMeta> {
        Some(SessionMeta {
            id: self.conv.session_id.clone()?,
            cwd: self.cwd.to_string_lossy().into_owned(),
            title: self.title.clone(),
            model: self.model.clone(),
            created: self.created,
            updated: chrono::Utc::now().timestamp(),
            cost_usd: self.conv.cost_usd,
            external: false,
        })
    }

    fn ensure_process(&mut self, program: &Path, config: &Config, ctx: &egui::Context) -> Result<(), String> {
        if self.process.is_some() {
            return Ok(());
        }
        let ctx = ctx.clone();
        let p = Process::spawn(
            Launch {
                program,
                cwd: &self.cwd,
                options: SpawnOptions {
                    model: Some(self.model.clone()),
                    resume: self.conv.session_id.clone(),
                    permission_mode: Some(self.permission_mode.clone()),
                    add_dirs: config.extra_dirs.iter().filter(|d| !d.trim().is_empty()).cloned().collect(),
                    mcp_config: Some(config.mcp_config.clone()).filter(|c| !c.trim().is_empty()),
                },
                append_system_prompt: Some(config.append_system_prompt.clone()),
            },
            move || ctx.request_repaint(),
        )?;
        self.process = Some(p);
        self.exited = None;
        self.conv.cost_base = self.conv.cost_usd;
        Ok(())
    }

    /// Send a message, or queue it while Claude is busy.
    pub fn send(&mut self, text: String, images: Vec<ImageBlock>, program: &Path, config: &Config, ctx: &egui::Context) -> Result<(), String> {
        if self.conv.running {
            self.queue.push((text, images));
            return Ok(());
        }
        self.ensure_process(program, config, ctx)?;
        if self.conv.items.is_empty() || self.title == "New chat" {
            self.title = index::title_from(&text);
        }
        self.conv.push_user(&text, images.len());
        if let Some(p) = &self.process {
            p.send_user(&text, &images);
        }
        self.scroll_to_bottom = true;
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(p) = &self.process {
            p.interrupt();
            self.interrupt_sent = Some(std::time::Instant::now());
        }
        self.queue.clear();
    }

    pub fn answer(&mut self, request_id: &str, decision: Decision) {
        let Some(pos) = self.conv.permissions.iter().position(|r| r.request_id == request_id) else { return };
        let req: PermissionRequest = self.conv.permissions.remove(pos);
        if let Some(p) = &self.process {
            p.answer(&req, &decision);
        }
    }

    pub fn set_model(&mut self, model: &str) {
        self.model = model.to_string();
        if let Some(p) = &self.process {
            p.set_model(model);
        }
    }

    pub fn set_permission_mode(&mut self, mode: &str) {
        self.permission_mode = mode.to_string();
        if let Some(p) = &self.process {
            p.set_permission_mode(mode);
        }
    }

    /// Drain events from the process. Returns things the app should act on.
    pub fn poll(&mut self, program: &Path, config: &Config, ctx: &egui::Context) -> Vec<ChatEvent> {
        let mut out = Vec::new();
        let Some(proc_) = &self.process else { return out };
        let events: Vec<Event> = proc_.events.try_iter().collect();
        let mut died = None;
        for ev in events {
            match ev {
                Event::Message(Incoming::ControlResponse { request_id, ok, response, error }) => {
                    let what = self.process.as_ref().and_then(|p| p.take_pending(&request_id));
                    if !ok {
                        if what != Some("interrupt") {
                            out.push(ChatEvent::Error(error.unwrap_or_else(|| "Claude Code refused a request.".into())));
                        }
                        continue;
                    }
                    if what == Some("initialize") {
                        self.read_initialize(&response);
                    }
                    if what == Some("interrupt") {
                        self.interrupt_sent = None;
                    }
                }
                Event::Message(msg) => {
                    let permission = matches!(&msg, Incoming::PermissionRequest(_));
                    let tool_name = if let Incoming::PermissionRequest(r) = &msg { r.tool_name.clone() } else { String::new() };
                    let result = if let Incoming::Result(r) = &msg { Some(r.is_error) } else { None };
                    let init = matches!(&msg, Incoming::Init(_));
                    if let Incoming::Init(i) = &msg {
                        self.version = Some(i.version.clone()).filter(|v| !v.is_empty()).or(self.version.take());
                        if !i.permission_mode.is_empty() {
                            self.permission_mode = i.permission_mode.clone();
                        }
                    }
                    let had_session = self.conv.session_id.clone();
                    if self.conv.apply(msg) {
                        self.scroll_to_bottom = true;
                    }
                    if permission {
                        out.push(ChatEvent::NeedsPermission(tool_name));
                    }
                    if init && self.conv.session_id != had_session {
                        out.push(ChatEvent::MetaChanged);
                    }
                    if let Some(error) = result {
                        out.push(ChatEvent::TurnDone { error });
                        out.push(ChatEvent::MetaChanged);
                    }
                }
                Event::Exited { stderr } => died = Some(stderr),
            }
        }
        if let Some(stderr) = died {
            let msg = last_lines(&stderr, 6);
            self.process = None;
            if self.conv.running {
                self.conv.running = false;
                self.conv.permissions.clear();
                self.conv.notice(
                    if msg.is_empty() { "Claude Code stopped unexpectedly.".to_string() } else { format!("Claude Code stopped: {msg}") },
                    true,
                );
            }
            self.exited = Some(msg);
        }
        // SIGINT if the interrupt request went unanswered.
        if let Some(t) = self.interrupt_sent {
            if t.elapsed() > std::time::Duration::from_secs(4) {
                if let Some(p) = &self.process {
                    p.sigint();
                }
                self.interrupt_sent = None;
            }
        }
        // Send the next queued message once idle.
        if !self.conv.running && !self.queue.is_empty() && self.process.is_some() {
            let (text, images) = self.queue.remove(0);
            if let Err(e) = self.send(text, images, program, config, ctx) {
                out.push(ChatEvent::Error(e));
            }
        }
        out
    }

    fn read_initialize(&mut self, r: &Value) {
        if let Some(models) = r.get("models").and_then(Value::as_array) {
            let list: Vec<ModelOption> = models
                .iter()
                .filter_map(|m| {
                    Some(ModelOption {
                        value: m.get("value")?.as_str()?.to_string(),
                        label: m.get("displayName").and_then(Value::as_str).unwrap_or_default().to_string(),
                        description: m.get("description").and_then(Value::as_str).unwrap_or_default().to_string(),
                    })
                })
                .collect();
            if !list.is_empty() {
                self.models = list;
            }
        }
        if let Some(cmds) = r.get("commands").and_then(Value::as_array) {
            self.commands = cmds
                .iter()
                .filter_map(|c| {
                    Some(Command {
                        name: c.get("name")?.as_str()?.to_string(),
                        description: c.get("description").and_then(Value::as_str).unwrap_or_default().to_string(),
                        hint: c.get("argumentHint").and_then(Value::as_str).unwrap_or_default().to_string(),
                    })
                })
                .collect();
        }
        if let Some(v) = r.get("claude_code_version").and_then(Value::as_str) {
            self.version = Some(v.to_string());
        }
        if let Some(mode) = r.get("current_permission_mode").and_then(Value::as_str) {
            self.permission_mode = mode.to_string();
        }
    }

    /// Slash commands to offer: from initialize, else from init.
    pub fn command_list(&self) -> Vec<Command> {
        if !self.commands.is_empty() {
            return self.commands.clone();
        }
        self.conv
            .init
            .as_ref()
            .map(|i| i.slash_commands.iter().map(|n| Command { name: n.clone(), description: String::new(), hint: String::new() }).collect())
            .unwrap_or_default()
    }

    /// Start listing project files for `@` mentions (respects .gitignore).
    pub fn ensure_files(&mut self, ctx: &egui::Context) {
        let cwd = self.cwd.clone();
        self.files.ensure(ctx, move || list_files(&cwd, 20_000));
    }
}

pub fn list_files(root: &Path, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    for entry in ignore::WalkBuilder::new(root).hidden(true).git_ignore(true).build().flatten() {
        if entry.file_type().is_some_and(|t| t.is_file()) {
            if let Ok(rel) = entry.path().strip_prefix(root) {
                out.push(rel.to_string_lossy().into_owned());
            }
        }
        if out.len() >= limit {
            break;
        }
    }
    out.sort();
    out
}

fn last_lines(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// Shown until `initialize` reports the account's real list.
pub fn default_models() -> Vec<ModelOption> {
    [("default", "Default"), ("sonnet", "Sonnet"), ("opus", "Opus"), ("haiku", "Haiku")]
        .iter()
        .map(|(v, l)| ModelOption { value: v.to_string(), label: l.to_string(), description: String::new() })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_listing_respects_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::create_dir_all(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "").unwrap();
        std::fs::write(dir.path().join("target/out"), "").unwrap();
        // The ignore crate only applies .gitignore inside a git repo.
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        let files = list_files(dir.path(), 100);
        assert!(files.contains(&"src/main.rs".to_string()));
        assert!(!files.iter().any(|f| f.starts_with("target")));
    }

    #[test]
    fn initialize_response_fills_models_and_commands() {
        let mut c = Chat::new(PathBuf::from("/tmp"), &Config::default());
        c.read_initialize(&serde_json::json!({
            "commands": [{"name": "review", "description": "Review a PR", "argumentHint": "[pr]"}],
            "models": [{"value": "default", "displayName": "Default (recommended)", "description": "Sonnet"}],
            "claude_code_version": "2.1.304",
            "current_permission_mode": "acceptEdits"
        }));
        assert_eq!(c.commands[0].hint, "[pr]");
        assert_eq!(c.models[0].label, "Default (recommended)");
        assert_eq!(c.version.as_deref(), Some("2.1.304"));
        assert_eq!(c.permission_mode, "acceptEdits");
        assert_eq!(last_lines("a\n\nb\nc\n", 2), "b\nc");
    }
}
