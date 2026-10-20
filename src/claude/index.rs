//! The sessions this app has created (Claude Code hides sessions started
//! through `-p`/stream-json from its own picker), plus a best-effort list of
//! other Claude Code sessions for a folder.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct SessionMeta {
    pub id: String,
    pub cwd: String,
    pub title: String,
    pub model: String,
    pub created: i64,
    pub updated: i64,
    pub cost_usd: f64,
    /// Found in Claude Code's own folder rather than created here.
    #[serde(skip)]
    pub external: bool,
}

#[derive(Default, Deserialize, Serialize)]
pub struct Index {
    pub sessions: Vec<SessionMeta>,
}

pub fn path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| crate::sys::paths::home().join(".local/share"))
        .join("m3-ai/sessions.json")
}

impl Index {
    pub fn load() -> Self {
        std::fs::read_to_string(path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let p = path();
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self).unwrap_or_default())?;
        std::fs::rename(tmp, p)
    }

    pub fn upsert(&mut self, meta: SessionMeta) {
        match self.sessions.iter_mut().find(|s| s.id == meta.id) {
            Some(s) => {
                let created = s.created;
                *s = meta;
                if created != 0 {
                    s.created = created;
                }
            }
            None => self.sessions.push(meta),
        }
        self.sessions.sort_by_key(|s| std::cmp::Reverse(s.updated));
    }

    pub fn remove(&mut self, id: &str) {
        self.sessions.retain(|s| s.id != id);
    }

    pub fn for_project(&self, cwd: &str) -> Vec<&SessionMeta> {
        self.sessions.iter().filter(|s| s.cwd == cwd).collect()
    }
}

/// Claude Code's folder name for a project: the path with every
/// non-alphanumeric character replaced by `-`.
pub fn encode_project(cwd: &str) -> String {
    cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

pub fn projects_dir(reported: Option<&str>) -> PathBuf {
    if let Some(p) = reported.filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::sys::paths::home().join(".claude"))
        .join("projects")
}

pub fn transcript_path(projects: &Path, cwd: &str, id: &str) -> PathBuf {
    projects.join(encode_project(cwd)).join(format!("{id}.jsonl"))
}

/// Other sessions for `cwd` in Claude Code's own folder. The transcript
/// format is internal, so only the first prompt and file time are used.
pub fn external_sessions(projects: &Path, cwd: &str, known: &[&str]) -> Vec<SessionMeta> {
    let dir = projects.join(encode_project(cwd));
    let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out: Vec<SessionMeta> = rd
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.extension()? != "jsonl" {
                return None;
            }
            let id = p.file_stem()?.to_string_lossy().into_owned();
            if known.contains(&id.as_str()) {
                return None;
            }
            let updated = e
                .metadata()
                .ok()?
                .modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_secs() as i64;
            let title = first_prompt(&p).unwrap_or_else(|| "Untitled session".into());
            Some(SessionMeta { id, cwd: cwd.into(), title, updated, created: updated, external: true, ..Default::default() })
        })
        .collect();
    out.sort_by_key(|s| std::cmp::Reverse(s.updated));
    out.truncate(30);
    out
}

fn first_prompt(path: &Path) -> Option<String> {
    use std::io::BufRead;
    let f = std::fs::File::open(path).ok()?;
    for line in std::io::BufReader::new(f).lines().take(200).map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        if v.get("type").and_then(|t| t.as_str()) != Some("user") || v.get("isMeta").and_then(|m| m.as_bool()) == Some(true) {
            continue;
        }
        let text = match v.pointer("/message/content") {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Array(a)) => a
                .iter()
                .find_map(|b| (b.get("type")?.as_str()? == "text").then(|| b.get("text")?.as_str().map(str::to_string))?)
                .unwrap_or_default(),
            _ => continue,
        };
        if !text.trim().is_empty() && !text.starts_with('<') {
            return Some(title_from(&text));
        }
    }
    None
}

/// A short title from a prompt.
pub fn title_from(prompt: &str) -> String {
    let line = prompt.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    let mut t: String = line.chars().take(60).collect();
    if line.chars().count() > 60 {
        t.push('…');
    }
    if t.is_empty() {
        "New chat".into()
    } else {
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_encoding_and_titles() {
        assert_eq!(encode_project("/home/me/my.app"), "-home-me-my-app");
        assert_eq!(title_from("\n  Fix the login bug please\nmore"), "Fix the login bug please");
        assert!(title_from(&"x".repeat(100)).ends_with('…'));
        assert_eq!(title_from(""), "New chat");
    }

    #[test]
    fn external_listing_and_upsert() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join(encode_project("/w/p"));
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(
            proj.join("abc.jsonl"),
            "{\"type\":\"summary\"}\n{\"type\":\"user\",\"isMeta\":true,\"message\":{\"content\":\"meta\"}}\n{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"Add tests\"}}\n",
        )
        .unwrap();
        std::fs::write(proj.join("known.jsonl"), "").unwrap();
        let found = external_sessions(dir.path(), "/w/p", &["known"]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "Add tests");
        assert!(found[0].external);
        assert_eq!(transcript_path(dir.path(), "/w/p", "abc"), proj.join("abc.jsonl"));

        let mut idx = Index::default();
        idx.upsert(SessionMeta { id: "a".into(), cwd: "/w/p".into(), created: 1, updated: 1, ..Default::default() });
        idx.upsert(SessionMeta { id: "b".into(), cwd: "/w/p".into(), created: 2, updated: 5, ..Default::default() });
        idx.upsert(SessionMeta { id: "a".into(), cwd: "/w/p".into(), created: 0, updated: 9, title: "t".into(), ..Default::default() });
        assert_eq!(idx.sessions[0].id, "a");
        assert_eq!(idx.sessions[0].created, 1);
        assert_eq!(idx.for_project("/w/p").len(), 2);
        idx.remove("a");
        assert_eq!(idx.sessions.len(), 1);
    }
}
