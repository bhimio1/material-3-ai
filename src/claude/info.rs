//! Facts about the installed `claude`: version and login state. Only
//! non-model commands are run; credentials are never read.

use std::path::Path;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Info {
    pub version: Option<String>,
    pub logged_in: bool,
    /// `oauth_token`, `api_key`, … as reported by `claude auth status`.
    pub auth_method: Option<String>,
    pub projects_dir: Option<String>,
    pub error: Option<String>,
}

pub fn parse_auth_status(json: &str) -> (bool, Option<String>, Option<String>) {
    let v: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    (
        v.get("loggedIn").and_then(|x| x.as_bool()).unwrap_or(false),
        v.get("authMethod").and_then(|x| x.as_str()).map(str::to_string),
        v.get("projectsDirectory").and_then(|x| x.as_str()).map(str::to_string),
    )
}

pub fn load(program: &Path) -> Info {
    let program = program.to_string_lossy();
    let mut info = Info::default();
    match material_3_ui::sys::cmd::run(&program, &["--version"]) {
        Ok(v) => info.version = v.split_whitespace().next().map(str::to_string),
        Err(e) => {
            info.error = Some(e);
            return info;
        }
    }
    match material_3_ui::sys::cmd::run(&program, &["auth", "status", "--json"]) {
        Ok(out) | Err(out) => {
            let (logged_in, method, projects) = parse_auth_status(&out);
            info.logged_in = logged_in;
            info.auth_method = method;
            info.projects_dir = projects;
        }
    }
    info
}

/// Human name for an auth method.
pub fn describe_method(method: &str) -> &str {
    match method {
        "oauth_token" | "claude.ai" | "oauth" => "Claude account",
        "api_key" | "apiKey" => "API key",
        "bedrock" => "Amazon Bedrock",
        "vertex" => "Google Vertex AI",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_status() {
        let (ok, method, projects) = parse_auth_status(
            r#"{"loggedIn": true, "authMethod": "oauth_token", "apiProvider": "firstParty", "projectsDirectory": "/root/.claude/projects"}"#,
        );
        assert!(ok);
        assert_eq!(method.as_deref(), Some("oauth_token"));
        assert_eq!(projects.as_deref(), Some("/root/.claude/projects"));
        assert_eq!(parse_auth_status("nope"), (false, None, None));
        assert_eq!(describe_method("oauth_token"), "Claude account");
    }
}
