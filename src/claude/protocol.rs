//! Claude Code's stream-json protocol, as spoken by the official Agent SDK:
//! newline-delimited JSON on the CLI's stdin and stdout.
//!
//! Incoming lines are parsed defensively into [`Incoming`]; anything we
//! don't model is kept as [`Incoming::Other`] and ignored by callers.
//! Outgoing messages are built by the functions at the bottom. Field names
//! follow `@anthropic-ai/claude-agent-sdk` (sdk.d.ts / sdk.mjs).

use serde_json::{json, Value};

/// Arguments for a long-lived session, in the order the SDK builds them
/// (`--flag=value` form). `-p` is not needed: stream-json input implies it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpawnOptions {
    pub model: Option<String>,
    pub resume: Option<String>,
    pub permission_mode: Option<String>,
    pub add_dirs: Vec<String>,
    pub mcp_config: Option<String>,
}

pub fn spawn_args(o: &SpawnOptions) -> Vec<String> {
    let mut a: Vec<String> = ["--output-format", "stream-json", "--verbose", "--input-format", "stream-json"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if let Some(m) = o.model.as_deref().filter(|m| !m.is_empty() && *m != "default") {
        a.push(format!("--model={m}"));
    }
    a.push("--permission-prompt-tool=stdio".into());
    if let Some(r) = o.resume.as_deref().filter(|r| !r.is_empty()) {
        a.push(format!("--resume={r}"));
    }
    if let Some(cfg) = o.mcp_config.as_deref().filter(|c| !c.is_empty()) {
        a.push(format!("--mcp-config={cfg}"));
    }
    if let Some(mode) = o.permission_mode.as_deref().filter(|m| !m.is_empty()) {
        a.push(format!("--permission-mode={mode}"));
        if mode == "bypassPermissions" {
            a.push("--allow-dangerously-skip-permissions".into());
        }
    }
    a.push("--include-partial-messages".into());
    for d in &o.add_dirs {
        a.push(format!("--add-dir={d}"));
    }
    a
}

/// A `can_use_tool` request: Claude wants to run a tool and asks us.
#[derive(Clone, Debug, PartialEq)]
pub struct PermissionRequest {
    pub request_id: String,
    pub tool_name: String,
    pub input: Value,
    /// `PermissionUpdate[]` Claude suggests for "always allow".
    pub suggestions: Vec<Value>,
    pub tool_use_id: Option<String>,
    pub title: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub decision_reason: Option<String>,
    pub blocked_path: Option<String>,
    pub agent_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    /// `system` / `init`.
    Init(Init),
    /// Other `system` messages (`compact_boundary`, `api_retry`, `status`, …).
    System { subtype: String, raw: Value },
    /// Raw Anthropic streaming event (with `--include-partial-messages`).
    Stream { event: Value, parent: Option<String> },
    /// A completed assistant message (one or more content blocks).
    Assistant { message: Value, parent: Option<String> },
    /// A user-role message: tool results, or text echoed by the CLI.
    User { message: Value, parent: Option<String>, tool_use_result: Option<Value> },
    /// End of a turn.
    Result(TurnResult),
    /// The CLI asks us something (`can_use_tool`, hook callbacks, …).
    PermissionRequest(PermissionRequest),
    /// Another control request we don't handle; must still be answered.
    UnsupportedRequest { request_id: String, subtype: String },
    /// The CLI withdrew one of its requests.
    CancelRequest { request_id: String },
    /// Answer to one of our control requests.
    ControlResponse { request_id: String, ok: bool, response: Value, error: Option<String> },
    /// `autocompact_state`: the effective context window.
    ContextWindow(u64),
    Other(Value),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Init {
    pub session_id: String,
    pub model: String,
    pub cwd: String,
    pub tools: Vec<String>,
    pub slash_commands: Vec<String>,
    pub mcp_servers: Vec<(String, String)>,
    pub permission_mode: String,
    pub version: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TurnResult {
    pub session_id: String,
    pub subtype: String,
    pub is_error: bool,
    pub result: Option<String>,
    pub errors: Vec<String>,
    pub total_cost_usd: f64,
    pub duration_ms: u64,
    pub num_turns: u64,
    pub usage: Usage,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_creation: u64,
}

impl Usage {
    pub fn from_value(v: &Value) -> Self {
        let n = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
        Usage {
            input: n("input_tokens"),
            output: n("output_tokens"),
            cache_read: n("cache_read_input_tokens"),
            cache_creation: n("cache_creation_input_tokens"),
        }
    }

    /// Tokens in the prompt: what fills the context window.
    pub fn context(&self) -> u64 {
        self.input + self.cache_read + self.cache_creation
    }
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn opt_s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

fn parent(v: &Value) -> Option<String> {
    opt_s(v, "parent_tool_use_id")
}

/// Parse one stdout line. Blank and non-JSON lines give `None`.
pub fn parse_line(line: &str) -> Option<Incoming> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    Some(parse_value(v))
}

pub fn parse_value(v: Value) -> Incoming {
    match v.get("type").and_then(Value::as_str).unwrap_or_default() {
        "system" => {
            let subtype = s(&v, "subtype");
            if subtype == "init" {
                Incoming::Init(Init {
                    session_id: s(&v, "session_id"),
                    model: s(&v, "model"),
                    cwd: s(&v, "cwd"),
                    tools: string_list(v.get("tools")),
                    slash_commands: string_list(v.get("slash_commands")),
                    mcp_servers: v
                        .get("mcp_servers")
                        .and_then(Value::as_array)
                        .map(|a| a.iter().map(|m| (s(m, "name"), s(m, "status"))).collect())
                        .unwrap_or_default(),
                    permission_mode: s(&v, "permissionMode"),
                    version: s(&v, "claude_code_version"),
                })
            } else {
                Incoming::System { subtype, raw: v }
            }
        }
        "stream_event" => Incoming::Stream {
            parent: parent(&v),
            event: v.get("event").cloned().unwrap_or(Value::Null),
        },
        "assistant" => Incoming::Assistant {
            parent: parent(&v),
            message: v.get("message").cloned().unwrap_or(Value::Null),
        },
        "user" => Incoming::User {
            parent: parent(&v),
            tool_use_result: v.get("tool_use_result").cloned(),
            message: v.get("message").cloned().unwrap_or(Value::Null),
        },
        "result" => Incoming::Result(TurnResult {
            session_id: s(&v, "session_id"),
            subtype: s(&v, "subtype"),
            is_error: v.get("is_error").and_then(Value::as_bool).unwrap_or(false),
            result: opt_s(&v, "result"),
            errors: string_list(v.get("errors")),
            total_cost_usd: v.get("total_cost_usd").and_then(Value::as_f64).unwrap_or(0.0),
            duration_ms: v.get("duration_ms").and_then(Value::as_u64).unwrap_or(0),
            num_turns: v.get("num_turns").and_then(Value::as_u64).unwrap_or(0),
            usage: v.get("usage").map(Usage::from_value).unwrap_or_default(),
        }),
        "control_request" => {
            let request_id = s(&v, "request_id");
            let req = v.get("request").cloned().unwrap_or(Value::Null);
            let subtype = s(&req, "subtype");
            if subtype == "can_use_tool" {
                Incoming::PermissionRequest(PermissionRequest {
                    request_id,
                    tool_name: s(&req, "tool_name"),
                    input: req.get("input").cloned().unwrap_or(Value::Null),
                    suggestions: req
                        .get("permission_suggestions")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default(),
                    tool_use_id: opt_s(&req, "tool_use_id"),
                    title: opt_s(&req, "title"),
                    display_name: opt_s(&req, "display_name"),
                    description: opt_s(&req, "description"),
                    decision_reason: req.get("decision_reason").and_then(|d| match d {
                        Value::String(s) => Some(s.clone()),
                        Value::Null => None,
                        other => Some(other.to_string()),
                    }),
                    blocked_path: opt_s(&req, "blocked_path"),
                    agent_id: opt_s(&req, "agent_id"),
                })
            } else {
                Incoming::UnsupportedRequest { request_id, subtype }
            }
        }
        "control_cancel_request" => Incoming::CancelRequest { request_id: s(&v, "request_id") },
        "control_response" => {
            let r = v.get("response").cloned().unwrap_or(Value::Null);
            let ok = s(&r, "subtype") == "success";
            Incoming::ControlResponse {
                request_id: s(&r, "request_id"),
                ok,
                error: opt_s(&r, "error"),
                response: r.get("response").cloned().unwrap_or(Value::Null),
            }
        }
        "autocompact_state" => match v.pointer("/value/effective_window").and_then(Value::as_u64) {
            Some(w) => Incoming::ContextWindow(w),
            None => Incoming::Other(v),
        },
        _ => Incoming::Other(v),
    }
}

fn string_list(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| match x {
                    Value::String(s) => Some(s.clone()),
                    Value::Object(_) => x.get("name").and_then(Value::as_str).map(str::to_string),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

// ----------------------------------------------------------------- outgoing

/// An image attached to a message.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageBlock {
    pub media_type: String,
    pub base64: String,
}

pub fn user_message(text: &str, images: &[ImageBlock]) -> Value {
    let mut content: Vec<Value> = images
        .iter()
        .map(|i| {
            json!({"type": "image", "source": {"type": "base64", "media_type": i.media_type, "data": i.base64}})
        })
        .collect();
    if !text.is_empty() || content.is_empty() {
        content.push(json!({"type": "text", "text": text}));
    }
    json!({
        "type": "user",
        "message": {"role": "user", "content": content},
        "parent_tool_use_id": null,
    })
}

pub fn control_request(request_id: &str, request: Value) -> Value {
    json!({"type": "control_request", "request_id": request_id, "request": request})
}

/// The SDK always sends `initialize` first; `appendSystemPrompt` goes here
/// rather than on the command line.
pub fn initialize(append_system_prompt: Option<&str>) -> Value {
    let mut r = json!({"subtype": "initialize"});
    if let Some(p) = append_system_prompt.filter(|p| !p.trim().is_empty()) {
        r["appendSystemPrompt"] = json!(p);
    }
    r
}

pub fn interrupt() -> Value {
    json!({"subtype": "interrupt"})
}

pub fn set_model(model: &str) -> Value {
    json!({"subtype": "set_model", "model": model})
}

pub fn set_permission_mode(mode: &str) -> Value {
    json!({"subtype": "set_permission_mode", "mode": mode})
}

/// Our answer to a `can_use_tool` request.
#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    Allow { updated_permissions: Vec<Value> },
    Deny { message: String },
}

pub fn permission_response(req: &PermissionRequest, decision: &Decision) -> Value {
    let mut result = match decision {
        Decision::Allow { updated_permissions } => {
            let mut r = json!({"behavior": "allow", "updatedInput": req.input});
            if !updated_permissions.is_empty() {
                r["updatedPermissions"] = json!(updated_permissions);
            }
            r
        }
        Decision::Deny { message } => json!({
            "behavior": "deny",
            "message": if message.is_empty() { "The user denied this action." } else { message.as_str() },
        }),
    };
    if let Some(id) = &req.tool_use_id {
        result["toolUseID"] = json!(id);
    }
    json!({
        "type": "control_response",
        "response": {"subtype": "success", "request_id": req.request_id, "response": result},
    })
}

/// "Always allow": Claude's own suggestions when it sent some, otherwise a
/// session rule for the tool.
pub fn always_allow_updates(req: &PermissionRequest) -> Vec<Value> {
    if !req.suggestions.is_empty() {
        return req.suggestions.clone();
    }
    vec![json!({
        "type": "addRules",
        "rules": [{"toolName": req.tool_name}],
        "behavior": "allow",
        "destination": "session",
    })]
}

/// Answer for control requests we can't handle, so the CLI doesn't wait.
pub fn error_response(request_id: &str, error: &str) -> Value {
    json!({
        "type": "control_response",
        "response": {"subtype": "error", "request_id": request_id, "error": error},
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_follow_the_sdk() {
        let a = spawn_args(&SpawnOptions {
            model: Some("opus".into()),
            resume: Some("abc".into()),
            permission_mode: Some("bypassPermissions".into()),
            add_dirs: vec!["/tmp/x".into()],
            mcp_config: None,
        });
        assert_eq!(
            a,
            vec![
                "--output-format", "stream-json", "--verbose", "--input-format", "stream-json",
                "--model=opus", "--permission-prompt-tool=stdio", "--resume=abc",
                "--permission-mode=bypassPermissions", "--allow-dangerously-skip-permissions",
                "--include-partial-messages", "--add-dir=/tmp/x",
            ]
        );
        let plain = spawn_args(&SpawnOptions { model: Some("default".into()), ..Default::default() });
        assert!(!plain.iter().any(|x| x.starts_with("--model")));
        assert!(!plain.iter().any(|x| x == "-p" || x == "--bare"));
    }

    #[test]
    fn init_message() {
        let line = r#"{"type":"system","subtype":"init","uuid":"u","session_id":"s1","cwd":"/p","model":"sonnet","tools":["Bash","Read"],"mcp_servers":[{"name":"gh","status":"connected"}],"permissionMode":"default","claude_code_version":"2.1.304","slash_commands":["compact","review"],"capabilities":[]}"#;
        let Some(Incoming::Init(i)) = parse_line(line) else { panic!() };
        assert_eq!(i.session_id, "s1");
        assert_eq!(i.tools, vec!["Bash", "Read"]);
        assert_eq!(i.mcp_servers, vec![("gh".to_string(), "connected".to_string())]);
        assert_eq!(i.slash_commands, vec!["compact", "review"]);
        assert_eq!(i.version, "2.1.304");
    }

    #[test]
    fn permission_round_trip() {
        let line = r#"{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"ls"},"permission_suggestions":[{"type":"addRules","rules":[{"toolName":"Bash","ruleContent":"ls:*"}],"behavior":"allow","destination":"localSettings"}],"tool_use_id":"toolu_1","decision_reason":"no rule"}}"#;
        let Some(Incoming::PermissionRequest(req)) = parse_line(line) else { panic!() };
        assert_eq!(req.tool_name, "Bash");
        assert_eq!(req.suggestions.len(), 1);
        let allow = permission_response(&req, &Decision::Allow { updated_permissions: always_allow_updates(&req) });
        assert_eq!(allow["type"], "control_response");
        assert_eq!(allow["response"]["subtype"], "success");
        assert_eq!(allow["response"]["request_id"], "r1");
        assert_eq!(allow["response"]["response"]["behavior"], "allow");
        assert_eq!(allow["response"]["response"]["updatedInput"]["command"], "ls");
        assert_eq!(allow["response"]["response"]["toolUseID"], "toolu_1");
        assert_eq!(allow["response"]["response"]["updatedPermissions"][0]["destination"], "localSettings");
        let deny = permission_response(&req, &Decision::Deny { message: String::new() });
        assert_eq!(deny["response"]["response"]["behavior"], "deny");
        assert!(deny["response"]["response"]["message"].as_str().unwrap().contains("denied"));
        assert!(deny["response"]["response"].get("updatedInput").is_none());
    }

    #[test]
    fn fallback_always_allow_rule() {
        let req = PermissionRequest {
            request_id: "r".into(),
            tool_name: "WebFetch".into(),
            input: json!({}),
            suggestions: vec![],
            tool_use_id: None,
            title: None,
            display_name: None,
            description: None,
            decision_reason: None,
            blocked_path: None,
            agent_id: None,
        };
        let u = always_allow_updates(&req);
        assert_eq!(u[0]["rules"][0]["toolName"], "WebFetch");
        assert_eq!(u[0]["destination"], "session");
    }

    #[test]
    fn other_control_traffic() {
        assert_eq!(
            parse_line(r#"{"type":"control_request","request_id":"h","request":{"subtype":"hook_callback"}}"#),
            Some(Incoming::UnsupportedRequest { request_id: "h".into(), subtype: "hook_callback".into() })
        );
        assert_eq!(
            parse_line(r#"{"type":"control_cancel_request","request_id":"r1"}"#),
            Some(Incoming::CancelRequest { request_id: "r1".into() })
        );
        let Some(Incoming::ControlResponse { request_id, ok, response, .. }) =
            parse_line(r#"{"type":"control_response","response":{"subtype":"success","request_id":"i1","response":{"models":[]}}}"#)
        else {
            panic!()
        };
        assert!(ok && request_id == "i1" && response["models"].is_array());
        let Some(Incoming::ControlResponse { ok, error, .. }) =
            parse_line(r#"{"type":"control_response","response":{"subtype":"error","request_id":"x","error":"nope"}}"#)
        else {
            panic!()
        };
        assert!(!ok && error.as_deref() == Some("nope"));
        let e = error_response("h", "unsupported");
        assert_eq!(e["response"]["subtype"], "error");
    }

    #[test]
    fn result_and_misc() {
        let line = r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":1200,"num_turns":2,"result":"Done","session_id":"s1","total_cost_usd":0.0123,"usage":{"input_tokens":10,"output_tokens":20,"cache_read_input_tokens":300,"cache_creation_input_tokens":5}}"#;
        let Some(Incoming::Result(r)) = parse_line(line) else { panic!() };
        assert_eq!(r.result.as_deref(), Some("Done"));
        assert!((r.total_cost_usd - 0.0123).abs() < 1e-9);
        assert_eq!(r.usage.context(), 315);
        let err = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["boom"],"session_id":"s1"}"#;
        let Some(Incoming::Result(r)) = parse_line(err) else { panic!() };
        assert!(r.is_error && r.errors == vec!["boom"]);
        assert_eq!(
            parse_line(r#"{"type":"autocompact_state","value":{"enabled":true,"effective_window":980000}}"#),
            Some(Incoming::ContextWindow(980000))
        );
        assert!(matches!(parse_line(r#"{"type":"active_goal","value":null}"#), Some(Incoming::Other(_))));
        assert_eq!(parse_line("not json"), None);
        assert_eq!(parse_line("   "), None);
        assert!(matches!(
            parse_line(r#"{"type":"system","subtype":"compact_boundary"}"#),
            Some(Incoming::System { .. })
        ));
    }

    #[test]
    fn outgoing_messages() {
        let m = user_message("hi", &[ImageBlock { media_type: "image/png".into(), base64: "AAA".into() }]);
        assert_eq!(m["type"], "user");
        assert_eq!(m["message"]["role"], "user");
        assert_eq!(m["message"]["content"][0]["type"], "image");
        assert_eq!(m["message"]["content"][0]["source"]["data"], "AAA");
        assert_eq!(m["message"]["content"][1]["text"], "hi");
        assert!(m["parent_tool_use_id"].is_null());
        let c = control_request("q1", set_model("opus"));
        assert_eq!(c["request"]["subtype"], "set_model");
        assert_eq!(c["request"]["model"], "opus");
        assert_eq!(set_permission_mode("plan")["mode"], "plan");
        assert_eq!(initialize(Some("Be brief"))["appendSystemPrompt"], "Be brief");
        assert!(initialize(None).get("appendSystemPrompt").is_none());
        assert_eq!(interrupt()["subtype"], "interrupt");
    }
}
