//! One running `claude` process for one chat. A reader thread parses stdout
//! into events for the UI; writes go straight to stdin (they're small).

use super::protocol::{self, Decision, ImageBlock, Incoming, PermissionRequest, SpawnOptions};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// What the reader thread delivers.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum Event {
    Message(Incoming),
    /// The process ended. `stderr` holds its last lines.
    Exited { stderr: String },
}

pub struct Process {
    child: Child,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    pub events: flume::Receiver<Event>,
    next_id: AtomicU64,
    /// Request ids of our pending control requests → what they were.
    pending: Mutex<Vec<(String, &'static str)>>,
}

pub struct Launch<'a> {
    pub program: &'a Path,
    pub cwd: &'a Path,
    pub options: SpawnOptions,
    pub append_system_prompt: Option<String>,
}

impl Process {
    /// Start `claude` and send `initialize`. `wake` is called after every
    /// event so the UI can repaint.
    pub fn spawn(launch: Launch<'_>, wake: impl Fn() + Send + 'static) -> Result<Self, String> {
        let mut cmd = Command::new(launch.program);
        cmd.args(protocol::spawn_args(&launch.options))
            .current_dir(launch.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Don't inherit the session of a Claude Code we might run inside.
        cmd.env_remove("CLAUDE_CODE_SESSION_ID");
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Couldn't start {}: {e}", launch.program.display()))?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let stderr = child.stderr.take().ok_or("no stderr")?;
        let stdin = Arc::new(Mutex::new(child.stdin.take()));

        let stderr_tail = Arc::new(Mutex::new(String::new()));
        {
            let tail = stderr_tail.clone();
            std::thread::spawn(move || {
                let mut r = BufReader::new(stderr);
                let mut buf = String::new();
                while r.read_line(&mut buf).unwrap_or(0) > 0 {
                    if let Ok(mut t) = tail.lock() {
                        t.push_str(&buf);
                        if t.len() > 8000 {
                            let cut = t.len() - 4000;
                            let cut = (cut..t.len()).find(|&i| t.is_char_boundary(i)).unwrap_or(0);
                            t.drain(..cut);
                        }
                    }
                    buf.clear();
                }
            });
        }

        let (tx, rx) = flume::unbounded();
        {
            let stdin = stdin.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stdout);
                let mut line = Vec::new();
                loop {
                    line.clear();
                    match reader.by_ref().take(64 << 20).read_until(b'\n', &mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                    let text = String::from_utf8_lossy(&line);
                    let Some(msg) = protocol::parse_line(&text) else { continue };
                    // Requests we don't support must still be answered.
                    if let Incoming::UnsupportedRequest { request_id, subtype } = &msg {
                        let reply = protocol::error_response(request_id, &format!("{subtype} isn't supported by Material 3 AI"));
                        write_json(&stdin, &reply);
                    }
                    if tx.send(Event::Message(msg)).is_err() {
                        break;
                    }
                    wake();
                }
                let stderr = stderr_tail.lock().map(|t| t.clone()).unwrap_or_default();
                let _ = tx.send(Event::Exited { stderr });
                wake();
            });
        }

        let p = Process {
            child,
            stdin,
            events: rx,
            next_id: AtomicU64::new(1),
            pending: Mutex::new(Vec::new()),
        };
        p.request("initialize", protocol::initialize(launch.append_system_prompt.as_deref()));
        Ok(p)
    }

    fn request(&self, what: &'static str, body: Value) -> String {
        let id = format!("m3ai-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        if let Ok(mut p) = self.pending.lock() {
            p.push((id.clone(), what));
        }
        self.write(&protocol::control_request(&id, body));
        id
    }

    /// Which of our requests a response belongs to (and forget it).
    pub fn take_pending(&self, request_id: &str) -> Option<&'static str> {
        let mut p = self.pending.lock().ok()?;
        let pos = p.iter().position(|(id, _)| id == request_id)?;
        Some(p.remove(pos).1)
    }

    fn write(&self, v: &Value) {
        write_json(&self.stdin, v);
    }

    pub fn send_user(&self, text: &str, images: &[ImageBlock]) {
        self.write(&protocol::user_message(text, images));
    }

    pub fn interrupt(&self) {
        self.request("interrupt", protocol::interrupt());
    }

    pub fn set_model(&self, model: &str) {
        self.request("set_model", protocol::set_model(model));
    }

    pub fn set_permission_mode(&self, mode: &str) {
        self.request("set_permission_mode", protocol::set_permission_mode(mode));
    }

    pub fn answer(&self, req: &PermissionRequest, decision: &Decision) {
        self.write(&protocol::permission_response(req, decision));
    }

    /// Fallback interrupt when the control request isn't answered.
    pub fn sigint(&self) {
        let _ = std::process::Command::new("kill").args(["-INT", &self.child.id().to_string()]).status();
    }
}

fn write_json(stdin: &Arc<Mutex<Option<ChildStdin>>>, v: &Value) {
    if let Ok(mut guard) = stdin.lock() {
        if let Some(w) = guard.as_mut() {
            let mut line = v.to_string();
            line.push('\n');
            if w.write_all(line.as_bytes()).and_then(|_| w.flush()).is_err() {
                *guard = None;
            }
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if let Ok(mut g) = self.stdin.lock() {
            g.take(); // EOF lets claude exit cleanly
        }
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_millis(300) {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The `claude` executable to use: the configured path, `M3_AI_CLAUDE`, or
/// `claude` on PATH (plus the usual install locations).
pub fn resolve_program(configured: &str) -> Option<PathBuf> {
    let configured = configured.trim();
    if !configured.is_empty() {
        let p = material_3_ui::sys::paths::expand(configured);
        return p.is_file().then_some(p);
    }
    if let Some(p) = std::env::var_os("M3_AI_CLAUDE").map(PathBuf::from) {
        if p.is_file() {
            return Some(p);
        }
    }
    let home = material_3_ui::sys::paths::home();
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain([home.join(".local/bin"), home.join(".claude/local"), home.join(".npm-global/bin")])
        .map(|d| d.join("claude"))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::conversation::{Body, Conversation};
    use std::time::{Duration, Instant};

    fn stub() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/dev/fake-claude")
    }

    /// Feed events into a conversation until `done` says stop.
    fn pump(p: &Process, conv: &mut Conversation, mut done: impl FnMut(&Incoming) -> bool) -> Vec<Incoming> {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut seen = Vec::new();
        while Instant::now() < deadline {
            match p.events.recv_timeout(Duration::from_millis(200)) {
                Ok(Event::Message(m)) => {
                    let stop = done(&m);
                    seen.push(m.clone());
                    conv.apply(m);
                    if stop {
                        return seen;
                    }
                }
                Ok(Event::Exited { stderr, .. }) => panic!("stub exited: {stderr}"),
                Err(_) => {}
            }
        }
        panic!("timed out; saw {seen:?}");
    }

    #[test]
    fn session_against_stub() {
        if !std::path::Path::new("/usr/bin/python3").exists() && !material_3_ui::sys::cmd::exists("python3") {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let p = Process::spawn(
            Launch { program: &stub(), cwd: dir.path(), options: SpawnOptions::default(), append_system_prompt: None },
            || {},
        )
        .unwrap();
        let mut conv = Conversation::default();

        // initialize is answered.
        let seen = pump(&p, &mut conv, |m| matches!(m, Incoming::ControlResponse { .. }));
        let Some(Incoming::ControlResponse { request_id, ok: true, response, .. }) = seen.last() else { panic!() };
        assert_eq!(p.take_pending(request_id), Some("initialize"));
        assert_eq!(response["models"][1]["value"], "opus");

        // Plain streamed answer.
        conv.push_user("hi", 0);
        p.send_user("hi", &[]);
        pump(&p, &mut conv, |m| matches!(m, Incoming::Result(_)));
        assert!(conv.items.iter().any(|i| matches!(&i.body, Body::Text(t) if t.starts_with("Hello!"))));
        assert_eq!(conv.session_id.as_deref(), Some("5f0c2d0e-8a51-4c4e-9b8e-6d0c1f2a3b4c"));
        assert_eq!(conv.init.as_ref().unwrap().mcp_servers[0].0, "github");

        // Permission: always allow sends Claude's suggested rule back.
        conv.push_user("needs permission", 0);
        p.send_user("needs permission", &[]);
        pump(&p, &mut conv, |m| matches!(m, Incoming::PermissionRequest(_)));
        let req = conv.permissions[0].clone();
        assert_eq!(req.tool_name, "Bash");
        p.answer(&req, &Decision::Allow { updated_permissions: protocol::always_allow_updates(&req) });
        conv.permissions.clear();
        let seen = pump(&p, &mut conv, |m| matches!(m, Incoming::Result(_)));
        let Some(Incoming::Result(r)) = seen.last() else { panic!() };
        assert!(r.result.as_deref().unwrap().contains("localSettings"), "{:?}", r.result);
        assert!(conv.items.iter().any(|i| matches!(&i.body, Body::Tool(t) if t.name == "Bash" && t.result.is_some())));

        // Permission: deny with a reason.
        p.send_user("permission again", &[]);
        pump(&p, &mut conv, |m| matches!(m, Incoming::PermissionRequest(_)));
        let req = conv.permissions[0].clone();
        p.answer(&req, &Decision::Deny { message: "not now".into() });
        conv.permissions.clear();
        pump(&p, &mut conv, |m| matches!(m, Incoming::Result(_)));
        assert!(conv.items.iter().any(|i| matches!(&i.body, Body::Text(t) if t.contains("not now"))));

        // Interrupt a running turn.
        conv.push_user("slow", 0);
        p.send_user("slow", &[]);
        pump(&p, &mut conv, |m| matches!(m, Incoming::Stream { .. }));
        p.interrupt();
        pump(&p, &mut conv, |m| matches!(m, Incoming::Result(_)));
        assert!(!conv.running);
        assert!((conv.cost_usd - 0.0412 * 4.0).abs() < 1e-6, "cost {}", conv.cost_usd);
    }

    /// Runs the real `claude` (only `initialize`, no prompt is sent, so no
    /// model call). `cargo test -- --ignored real_claude_initialize`
    #[test]
    #[ignore]
    fn real_claude_initialize() {
        let Some(program) = resolve_program("") else { return };
        let dir = tempfile::tempdir().unwrap();
        let p = Process::spawn(
            Launch { program: &program, cwd: dir.path(), options: SpawnOptions::default(), append_system_prompt: None },
            || {},
        )
        .unwrap();
        let mut conv = Conversation::default();
        let seen = pump(&p, &mut conv, |m| matches!(m, Incoming::ControlResponse { .. }));
        let Some(Incoming::ControlResponse { request_id, ok, response, error }) = seen.last() else { panic!() };
        assert!(ok, "{error:?}");
        assert_eq!(p.take_pending(request_id), Some("initialize"));
        assert!(response["commands"].is_array());
        assert!(response["models"].as_array().is_some_and(|m| !m.is_empty()));
        eprintln!("real claude {} with {} models", response["claude_code_version"], response["models"].as_array().unwrap().len());
    }
}
