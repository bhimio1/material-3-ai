//! Preferences (`~/.config/m3-ai/config.toml`).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ColorScheme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ThemeSource {
    /// end4-pC's colors, else the file manager's theme.json, else the seed.
    #[default]
    Auto,
    Seed,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub theme_source: ThemeSource,
    pub color_scheme: ColorScheme,
    pub seed_color: String,
    pub filled_icons: bool,
    pub ui_scale: f32,
    /// Path to `claude`; empty = find it on PATH.
    pub claude_path: String,
    /// Terminal for signing in; empty = automatic.
    pub terminal: String,
    pub default_model: String,
    pub default_permission_mode: String,
    pub extra_dirs: Vec<String>,
    pub append_system_prompt: String,
    pub mcp_config: String,
    pub chat_font_size: f32,
    pub notifications: bool,
    /// Project folders, most recent first.
    pub projects: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme_source: ThemeSource::Auto,
            color_scheme: ColorScheme::System,
            seed_color: "#D97757".into(),
            filled_icons: true,
            ui_scale: 1.0,
            claude_path: String::new(),
            terminal: String::new(),
            default_model: "default".into(),
            default_permission_mode: "default".into(),
            extra_dirs: Vec::new(),
            append_system_prompt: String::new(),
            mcp_config: String::new(),
            chat_font_size: 14.5,
            notifications: true,
            projects: Vec::new(),
        }
    }
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("m3-ai/config.toml"))
    }

    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::path().ok_or_else(|| anyhow::anyhow!("no config directory"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }

    /// Move a project to the top of the recent list.
    pub fn touch_project(&mut self, dir: &str) {
        self.projects.retain(|p| p != dir);
        self.projects.insert(0, dir.to_string());
        self.projects.truncate(30);
    }
}

/// Permission modes as Claude Code names them, with labels.
pub const PERMISSION_MODES: [(&str, &str); 5] = [
    ("default", "Ask"),
    ("acceptEdits", "Accept edits"),
    ("plan", "Plan"),
    ("auto", "Auto"),
    ("bypassPermissions", "Bypass"),
];

pub fn mode_label(mode: &str) -> &str {
    PERMISSION_MODES.iter().find(|(k, _)| *k == mode).map(|(_, l)| *l).unwrap_or(mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_recent_projects() {
        let c: Config = toml::from_str("chat_font_size = 16.0\n").unwrap();
        assert_eq!(c.chat_font_size, 16.0);
        assert_eq!(c.default_permission_mode, "default");
        let mut c = Config::default();
        c.touch_project("/a");
        c.touch_project("/b");
        c.touch_project("/a");
        assert_eq!(c.projects, vec!["/a", "/b"]);
        assert_eq!(mode_label("acceptEdits"), "Accept edits");
    }
}
