//! What a chat shows, built from protocol events: messages, thinking, tool
//! calls with their results, todos, cost and context usage. Pure logic, no
//! UI, so it can be tested on recorded event lines.

use super::protocol::{Incoming, Init, PermissionRequest, TurnResult, Usage};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
    /// Raw JSON while the input is still streaming.
    pub partial_input: String,
    pub result: Option<ToolResult>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolResult {
    pub text: String,
    pub is_error: bool,
    /// The CLI's structured `tool_use_result` (Bash stdout/stderr, Edit patch…).
    pub structured: Option<Value>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    User { text: String, images: usize },
    Text(String),
    Thinking(String),
    Tool(ToolCall),
    Notice { text: String, error: bool },
    TurnEnd { cost_usd: f64, duration_ms: u64, error: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub body: Body,
    /// Set for messages from a subagent (`parent_tool_use_id`).
    pub parent: Option<String>,
    /// Still receiving deltas.
    pub streaming: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Todo {
    pub content: String,
    pub status: String,
}

#[derive(Default)]
pub struct Conversation {
    pub items: Vec<Item>,
    pub session_id: Option<String>,
    pub init: Option<Init>,
    pub running: bool,
    pub permissions: Vec<PermissionRequest>,
    pub todos: Vec<Todo>,
    /// Cost of the whole session so far.
    pub cost_usd: f64,
    /// Cost recorded before this process started (resumed sessions).
    pub cost_base: f64,
    /// Usage reported by the latest turn.
    pub usage: Usage,
    /// Prompt size of the latest model call.
    pub context_tokens: u64,
    pub context_window: Option<u64>,
    /// Shown above the composer (retries, errors).
    pub banner: Option<String>,
    /// Streaming block positions: (parent, message id, block index) → item.
    streams: HashMap<(Option<String>, String, u64), usize>,
    current_message: HashMap<Option<String>, String>,
    tool_index: HashMap<String, usize>,
}

impl Conversation {
    pub fn push_user(&mut self, text: &str, images: usize) {
        self.items.push(Item {
            body: Body::User { text: text.to_string(), images },
            parent: None,
            streaming: false,
        });
        self.running = true;
        self.banner = None;
    }

    pub fn notice(&mut self, text: impl Into<String>, error: bool) {
        self.items.push(Item { body: Body::Notice { text: text.into(), error }, parent: None, streaming: false });
    }

    #[cfg(test)]
    pub fn tool(&self, id: &str) -> Option<&ToolCall> {
        match self.tool_index.get(id).and_then(|&i| self.items.get(i)) {
            Some(Item { body: Body::Tool(t), .. }) => Some(t),
            _ => None,
        }
    }

    pub fn context_fraction(&self) -> Option<f32> {
        let window = self.context_window.unwrap_or(200_000);
        (self.context_tokens > 0).then(|| (self.context_tokens as f32 / window as f32).min(1.0))
    }

    /// Apply one event. Returns true when something visible changed.
    pub fn apply(&mut self, ev: Incoming) -> bool {
        match ev {
            Incoming::Init(init) => {
                self.session_id = Some(init.session_id.clone()).filter(|s| !s.is_empty()).or(self.session_id.take());
                self.init = Some(init);
            }
            Incoming::System { subtype, raw } => match subtype.as_str() {
                "compact_boundary" => self.notice("Conversation compacted to save context.", false),
                "api_retry" => {
                    let attempt = raw.get("attempt").and_then(Value::as_u64).unwrap_or(0);
                    let max = raw.get("max_retries").and_then(Value::as_u64).unwrap_or(0);
                    let err = raw.get("error").and_then(Value::as_str).unwrap_or("API error");
                    self.banner = Some(if max > 0 {
                        format!("{err} — retrying ({attempt}/{max})…")
                    } else {
                        format!("{err} — retrying…")
                    });
                }
                "status" => {
                    let status = raw.get("status").and_then(Value::as_str).unwrap_or_default();
                    self.banner = (status == "compacting").then(|| "Compacting the conversation…".to_string());
                }
                _ => return false,
            },
            Incoming::Stream { event, parent } => return self.apply_stream(&event, parent),
            Incoming::Assistant { message, parent } => self.apply_assistant(&message, parent),
            Incoming::User { message, parent, tool_use_result } => self.apply_user(&message, parent, tool_use_result),
            Incoming::Result(r) => self.apply_result(r),
            Incoming::PermissionRequest(req) => {
                if !self.permissions.iter().any(|p| p.request_id == req.request_id) {
                    self.permissions.push(req);
                }
            }
            Incoming::CancelRequest { request_id } => {
                self.permissions.retain(|p| p.request_id != request_id);
            }
            Incoming::ContextWindow(w) => self.context_window = Some(w),
            Incoming::ControlResponse { .. } | Incoming::UnsupportedRequest { .. } | Incoming::Other(_) => return false,
        }
        true
    }

    fn apply_stream(&mut self, event: &Value, parent: Option<String>) -> bool {
        let ty = event.get("type").and_then(Value::as_str).unwrap_or_default();
        match ty {
            "message_start" => {
                let id = event.pointer("/message/id").and_then(Value::as_str).unwrap_or_default();
                self.current_message.insert(parent, id.to_string());
                if let Some(u) = event.pointer("/message/usage") {
                    let usage = Usage::from_value(u);
                    if usage.context() > 0 && self_is_main(&self.current_message) {
                        self.context_tokens = usage.context();
                    }
                }
                false
            }
            "content_block_start" => {
                let index = event.get("index").and_then(Value::as_u64).unwrap_or(0);
                let block = event.get("content_block").cloned().unwrap_or(Value::Null);
                let body = match block.get("type").and_then(Value::as_str).unwrap_or_default() {
                    "text" => Body::Text(str_of(&block, "text")),
                    "thinking" => Body::Thinking(str_of(&block, "thinking")),
                    "tool_use" | "server_tool_use" | "mcp_tool_use" => Body::Tool(ToolCall {
                        id: str_of(&block, "id"),
                        name: str_of(&block, "name"),
                        input: Value::Null,
                        partial_input: String::new(),
                        result: None,
                    }),
                    _ => return false,
                };
                let msg = self.current_message.get(&parent).cloned().unwrap_or_default();
                let idx = self.items.len();
                if let Body::Tool(t) = &body {
                    self.tool_index.insert(t.id.clone(), idx);
                }
                self.items.push(Item { body, parent: parent.clone(), streaming: true });
                self.streams.insert((parent, msg, index), idx);
                true
            }
            "content_block_delta" => {
                let index = event.get("index").and_then(Value::as_u64).unwrap_or(0);
                let msg = self.current_message.get(&parent).cloned().unwrap_or_default();
                let Some(&idx) = self.streams.get(&(parent, msg, index)) else { return false };
                let delta = event.get("delta").cloned().unwrap_or(Value::Null);
                let Some(item) = self.items.get_mut(idx) else { return false };
                match (&mut item.body, delta.get("type").and_then(Value::as_str).unwrap_or_default()) {
                    (Body::Text(t), "text_delta") => t.push_str(&str_of(&delta, "text")),
                    (Body::Thinking(t), "thinking_delta") => t.push_str(&str_of(&delta, "thinking")),
                    (Body::Tool(tool), "input_json_delta") => tool.partial_input.push_str(&str_of(&delta, "partial_json")),
                    _ => return false,
                }
                true
            }
            "content_block_stop" => {
                let index = event.get("index").and_then(Value::as_u64).unwrap_or(0);
                let msg = self.current_message.get(&parent).cloned().unwrap_or_default();
                if let Some(&idx) = self.streams.get(&(parent, msg, index)) {
                    if let Some(item) = self.items.get_mut(idx) {
                        if let Body::Tool(t) = &mut item.body {
                            if t.input.is_null() {
                                t.input = serde_json::from_str(&t.partial_input).unwrap_or(Value::Null);
                            }
                        }
                    }
                }
                false
            }
            _ => false,
        }
    }

    /// A completed assistant message. Blocks we streamed are replaced by the
    /// authoritative final version; unseen blocks are appended.
    fn apply_assistant(&mut self, message: &Value, parent: Option<String>) {
        let msg_id = message.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
        if parent.is_none() {
            if let Some(u) = message.get("usage") {
                let usage = Usage::from_value(u);
                if usage.context() > 0 {
                    self.context_tokens = usage.context();
                }
            }
        }
        let Some(blocks) = message.get("content").and_then(Value::as_array) else { return };
        for block in blocks {
            let kind = block.get("type").and_then(Value::as_str).unwrap_or_default();
            let body = match kind {
                "text" => Body::Text(str_of(block, "text")),
                "thinking" => Body::Thinking(str_of(block, "thinking")),
                "redacted_thinking" => Body::Thinking("(Redacted thinking)".into()),
                "tool_use" | "server_tool_use" | "mcp_tool_use" => Body::Tool(ToolCall {
                    id: str_of(block, "id"),
                    name: str_of(block, "name"),
                    input: block.get("input").cloned().unwrap_or(Value::Null),
                    partial_input: String::new(),
                    result: None,
                }),
                _ => continue,
            };
            if let Body::Tool(t) = &body {
                self.update_todos(t);
            }
            // Find the streamed item for this block: same message, same kind,
            // still streaming (tool calls match by id).
            let found = self
                .streams
                .iter()
                .filter(|((p, m, _), _)| *p == parent && *m == msg_id)
                .map(|(_, &i)| i)
                .filter(|&i| {
                    self.items.get(i).is_some_and(|it| {
                        it.streaming
                            && match (&it.body, &body) {
                                (Body::Tool(a), Body::Tool(b)) => a.id == b.id,
                                (Body::Text(_), Body::Text(_)) | (Body::Thinking(_), Body::Thinking(_)) => true,
                                _ => false,
                            }
                    })
                })
                .min();
            match found {
                Some(i) => {
                    let item = &mut self.items[i];
                    if let (Body::Tool(old), Body::Tool(new)) = (&item.body, &body) {
                        // Keep a result that may already have arrived.
                        let mut new = new.clone();
                        new.result = old.result.clone();
                        item.body = Body::Tool(new);
                    } else {
                        item.body = body;
                    }
                    item.streaming = false;
                }
                None => {
                    let idx = self.items.len();
                    if let Body::Tool(t) = &body {
                        if self.tool_index.contains_key(&t.id) {
                            continue; // already shown
                        }
                        self.tool_index.insert(t.id.clone(), idx);
                    }
                    self.items.push(Item { body, parent: parent.clone(), streaming: false });
                }
            }
        }
    }

    fn apply_user(&mut self, message: &Value, parent: Option<String>, structured: Option<Value>) {
        match message.get("content") {
            Some(Value::Array(blocks)) => {
                for b in blocks {
                    if b.get("type").and_then(Value::as_str) != Some("tool_result") {
                        continue;
                    }
                    let id = str_of(b, "tool_use_id");
                    let result = ToolResult {
                        text: tool_result_text(b.get("content")),
                        is_error: b.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                        structured: structured.clone(),
                    };
                    match self.tool_index.get(&id).copied() {
                        Some(i) => {
                            if let Some(Item { body: Body::Tool(t), .. }) = self.items.get_mut(i) {
                                t.result = Some(result);
                            }
                        }
                        None => self.notice(result.text, result.is_error),
                    }
                }
            }
            // Text the CLI shows itself, e.g. slash command output.
            Some(Value::String(text)) => {
                let cleaned = strip_command_tags(text);
                if !cleaned.trim().is_empty() && parent.is_none() {
                    self.notice(cleaned, false);
                }
            }
            _ => {}
        }
    }

    fn apply_result(&mut self, r: TurnResult) {
        self.running = false;
        self.banner = None;
        for item in &mut self.items {
            item.streaming = false;
        }
        self.streams.clear();
        if !r.session_id.is_empty() {
            self.session_id = Some(r.session_id.clone());
        }
        // The CLI reports the process's running total; turn cost is the step.
        let before = self.cost_usd;
        self.cost_usd = (self.cost_base + r.total_cost_usd).max(before);
        let turn_cost = self.cost_usd - before;
        self.usage = r.usage;
        let error = r.is_error;
        if error {
            let msg = if r.errors.is_empty() {
                match r.subtype.as_str() {
                    "error_max_turns" => "Stopped: reached the turn limit.".to_string(),
                    "error_max_budget_usd" => "Stopped: reached the budget limit.".to_string(),
                    _ => r.result.clone().unwrap_or_else(|| "The turn ended with an error.".into()),
                }
            } else {
                r.errors.join("\n")
            };
            self.notice(msg, true);
        }
        self.items.push(Item {
            body: Body::TurnEnd { cost_usd: turn_cost, duration_ms: r.duration_ms, error },
            parent: None,
            streaming: false,
        });
        self.permissions.clear();
    }

    fn update_todos(&mut self, t: &ToolCall) {
        if t.name != "TodoWrite" {
            return;
        }
        if let Some(list) = t.input.get("todos").and_then(Value::as_array) {
            self.todos = list
                .iter()
                .map(|x| Todo { content: str_of(x, "content"), status: str_of(x, "status") })
                .collect();
        }
    }

    /// Load a past session's transcript (`~/.claude/projects/…/<id>.jsonl`).
    /// The format is internal to Claude Code, so this is best effort: lines
    /// that look like protocol messages are applied, the rest skipped.
    pub fn load_transcript(&mut self, text: &str) {
        for line in text.lines() {
            let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
            match v.get("type").and_then(Value::as_str) {
                Some("user") => {
                    let msg = v.get("message").cloned().unwrap_or(Value::Null);
                    let is_meta = v.get("isMeta").and_then(Value::as_bool).unwrap_or(false);
                    match msg.get("content") {
                        Some(Value::String(s)) if !is_meta && !s.starts_with('<') => self.push_user(s, 0),
                        Some(Value::Array(blocks)) if blocks.iter().any(|b| b.get("type").and_then(Value::as_str) == Some("text")) && !is_meta => {
                            let text: Vec<String> = blocks
                                .iter()
                                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                                .map(|b| str_of(b, "text"))
                                .collect();
                            let images = blocks.iter().filter(|b| b.get("type").and_then(Value::as_str) == Some("image")).count();
                            self.push_user(&text.join("\n"), images);
                        }
                        _ => self.apply_user(&msg, None, v.get("toolUseResult").cloned()),
                    }
                }
                Some("assistant") => {
                    let msg = v.get("message").cloned().unwrap_or(Value::Null);
                    self.apply_assistant(&msg, None);
                }
                _ => {}
            }
        }
        self.running = false;
        for item in &mut self.items {
            item.streaming = false;
        }
    }
}

fn self_is_main(current: &HashMap<Option<String>, String>) -> bool {
    current.contains_key(&None)
}

fn str_of(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn tool_result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| match p.get("type").and_then(Value::as_str) {
                Some("text") => p.get("text").and_then(Value::as_str).map(str::to_string),
                Some("image") => Some("[image]".into()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// `<local-command-stdout>…</local-command-stdout>` → inner text.
fn strip_command_tags(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        match rest[start..].find('>') {
            Some(end) => rest = &rest[start + end + 1..],
            None => {
                rest = &rest[start..];
                break;
            }
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::protocol::parse_line;

    fn feed(c: &mut Conversation, lines: &[&str]) {
        for l in lines {
            if let Some(ev) = parse_line(l) {
                c.apply(ev);
            }
        }
    }

    #[test]
    fn streamed_text_then_final_message() {
        let mut c = Conversation::default();
        c.push_user("hi", 0);
        feed(
            &mut c,
            &[
                r#"{"type":"stream_event","event":{"type":"message_start","message":{"id":"m1","usage":{"input_tokens":5,"cache_read_input_tokens":1000}}},"parent_tool_use_id":null}"#,
                r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}},"parent_tool_use_id":null}"#,
                r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me "}},"parent_tool_use_id":null}"#,
                r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"think"}},"parent_tool_use_id":null}"#,
                r#"{"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}},"parent_tool_use_id":null}"#,
                r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Hel"}},"parent_tool_use_id":null}"#,
                r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"lo"}},"parent_tool_use_id":null}"#,
            ],
        );
        assert_eq!(c.items[1].body, Body::Thinking("Let me think".into()));
        assert_eq!(c.items[2].body, Body::Text("Hello".into()));
        assert!(c.items[2].streaming);
        assert_eq!(c.context_tokens, 1005);
        feed(
            &mut c,
            &[
                r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"thinking","thinking":"Let me think."}]},"parent_tool_use_id":null}"#,
                r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"Hello!"}]},"parent_tool_use_id":null}"#,
            ],
        );
        assert_eq!(c.items.len(), 3, "final blocks replace streamed ones");
        assert_eq!(c.items[2].body, Body::Text("Hello!".into()));
        assert!(!c.items[2].streaming);
        feed(&mut c, &[r#"{"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.02,"duration_ms":900,"session_id":"s9","usage":{"input_tokens":5,"output_tokens":7}}"#]);
        assert!(!c.running);
        assert_eq!(c.session_id.as_deref(), Some("s9"));
        assert!((c.cost_usd - 0.02).abs() < 1e-9);
        assert!(matches!(c.items.last().unwrap().body, Body::TurnEnd { .. }));
    }

    #[test]
    fn tool_calls_results_todos_and_subagents() {
        let mut c = Conversation::default();
        c.push_user("go", 0);
        feed(
            &mut c,
            &[
                r#"{"type":"stream_event","event":{"type":"message_start","message":{"id":"m2"}},"parent_tool_use_id":null}"#,
                r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t1","name":"Bash","input":{}}},"parent_tool_use_id":null}"#,
                r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"command\":\"ls"}},"parent_tool_use_id":null}"#,
                r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"\"}"}},"parent_tool_use_id":null}"#,
                r#"{"type":"stream_event","event":{"type":"content_block_stop","index":0},"parent_tool_use_id":null}"#,
            ],
        );
        assert_eq!(c.tool("t1").unwrap().input["command"], "ls");
        feed(
            &mut c,
            &[
                r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]},"parent_tool_use_id":null}"#,
                r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"a.txt\nb.txt","is_error":false}]},"parent_tool_use_id":null,"tool_use_result":{"stdout":"a.txt\nb.txt","stderr":"","interrupted":false}}"#,
                r#"{"type":"assistant","message":{"id":"m3","content":[{"type":"tool_use","id":"t2","name":"TodoWrite","input":{"todos":[{"content":"Read files","status":"completed"},{"content":"Fix bug","status":"in_progress"}]}}]},"parent_tool_use_id":null}"#,
                r#"{"type":"assistant","message":{"id":"m4","content":[{"type":"tool_use","id":"t3","name":"Task","input":{"description":"Explore","prompt":"look"}}]},"parent_tool_use_id":null}"#,
                r#"{"type":"assistant","message":{"id":"m5","content":[{"type":"text","text":"Found it"}]},"parent_tool_use_id":"t3"}"#,
            ],
        );
        let t1 = c.tool("t1").unwrap();
        assert_eq!(t1.result.as_ref().unwrap().text, "a.txt\nb.txt");
        assert_eq!(t1.result.as_ref().unwrap().structured.as_ref().unwrap()["stdout"], "a.txt\nb.txt");
        assert_eq!(c.items.iter().filter(|i| matches!(i.body, Body::Tool(_))).count(), 3);
        assert_eq!(c.todos.len(), 2);
        assert_eq!(c.todos[1].status, "in_progress");
        assert_eq!(c.items.last().unwrap().parent.as_deref(), Some("t3"));
    }

    #[test]
    fn permissions_and_errors() {
        let mut c = Conversation::default();
        feed(&mut c, &[r#"{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Write","input":{"file_path":"/x","content":"y"}}}"#]);
        assert_eq!(c.permissions.len(), 1);
        feed(&mut c, &[r#"{"type":"control_cancel_request","request_id":"r1"}"#]);
        assert!(c.permissions.is_empty());
        feed(&mut c, &[r#"{"type":"system","subtype":"api_retry","attempt":2,"max_retries":10,"error":"overloaded"}"#]);
        assert!(c.banner.as_deref().unwrap().contains("2/10"));
        feed(&mut c, &[r#"{"type":"result","subtype":"error_max_turns","is_error":true,"session_id":"s"}"#]);
        assert!(c.banner.is_none());
        assert!(c.items.iter().any(|i| matches!(&i.body, Body::Notice { error: true, text } if text.contains("turn limit"))));
    }

    #[test]
    fn slash_command_output_and_transcripts() {
        let mut c = Conversation::default();
        feed(&mut c, &[r#"{"type":"user","message":{"role":"user","content":"<local-command-stdout>Total cost: $0.10</local-command-stdout>"},"parent_tool_use_id":null}"#]);
        assert!(matches!(&c.items[0].body, Body::Notice { text, .. } if text == "Total cost: $0.10"));

        let mut t = Conversation::default();
        t.load_transcript(concat!(
            r#"{"type":"user","message":{"role":"user","content":"Fix the build"},"uuid":"a"}"#, "\n",
            "garbage\n",
            r#"{"type":"assistant","message":{"id":"m1","role":"assistant","content":[{"type":"text","text":"Sure"},{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/a"}}]}}"#, "\n",
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"fn main(){}"}]}}"#, "\n",
        ));
        assert_eq!(t.items.len(), 3);
        assert!(matches!(&t.items[0].body, Body::User { text, .. } if text == "Fix the build"));
        assert_eq!(t.tool("t1").unwrap().result.as_ref().unwrap().text, "fn main(){}");
        assert!(!t.running);
    }
}
