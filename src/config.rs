//! Preferences (`~/.config/m3-ai/config.toml`).

use material_3_ui::theme::{ColorScheme, ThemeSettings, ThemeSource};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The seed color when there's no desktop palette.
pub const DEFAULT_SEED: &str = "#D97757";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Config {
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
    /// Color source, light/dark, seed color, icons and scale, shared with
    /// the other Material 3 apps. Last, as a TOML table.
    pub appearance: ThemeSettings,
}

/// The appearance fields as 0.1 stored them, at the top level.
#[derive(Deserialize)]
struct Legacy {
    theme_source: Option<ThemeSource>,
    color_scheme: Option<ColorScheme>,
    seed_color: Option<String>,
    filled_icons: Option<bool>,
    ui_scale: Option<f32>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
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
            appearance: ThemeSettings { seed_color: DEFAULT_SEED.into(), ..Default::default() },
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
            .and_then(|s| Self::parse(&s))
            .unwrap_or_default()
    }

    /// Reads the file, moving 0.1's top-level appearance fields into
    /// `appearance`.
    fn parse(text: &str) -> Option<Self> {
        let mut config: Config = toml::from_str(text).ok()?;
        let table: toml::Table = toml::from_str(text).ok()?;
        if !table.contains_key("appearance") {
            if let Ok(old) = toml::from_str::<Legacy>(text) {
                let a = &mut config.appearance;
                a.source = old.theme_source.unwrap_or(a.source);
                a.scheme = old.color_scheme.unwrap_or(a.scheme);
                a.seed_color = old.seed_color.unwrap_or(std::mem::take(&mut a.seed_color));
                a.filled_icons = old.filled_icons.unwrap_or(a.filled_icons);
                a.ui_scale = old.ui_scale.unwrap_or(a.ui_scale);
            }
        }
        Some(config)
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
        let c = Config::parse("chat_font_size = 16.0\n").unwrap();
        assert_eq!(c.chat_font_size, 16.0);
        assert_eq!(c.appearance.seed_color, DEFAULT_SEED);
        assert_eq!(c.default_permission_mode, "default");
        let mut c = Config::default();
        c.touch_project("/a");
        c.touch_project("/b");
        c.touch_project("/a");
        assert_eq!(c.projects, vec!["/a", "/b"]);
        assert_eq!(mode_label("acceptEdits"), "Accept edits");
    }

    #[test]
    fn old_appearance_fields_carry_over() {
        let old = "theme_source = \"seed\"\ncolor_scheme = \"light\"\nseed_color = \"#1E8E3E\"\nfilled_icons = false\nui_scale = 1.5\nchat_font_size = 13.0\n";
        let c = Config::parse(old).unwrap();
        assert_eq!(c.appearance.source, ThemeSource::Seed);
        assert_eq!(c.appearance.scheme, ColorScheme::Light);
        assert_eq!(c.appearance.seed_color, "#1E8E3E");
        assert!(!c.appearance.filled_icons);
        assert_eq!(c.appearance.ui_scale, 1.5);
        assert_eq!(c.chat_font_size, 13.0);
        // Saved in the new layout, it reads back the same.
        let text = toml::to_string_pretty(&c).unwrap();
        assert!(text.contains("[appearance]"));
        assert_eq!(Config::parse(&text).unwrap(), c);
        // Stray old keys don't override the new table.
        let c = Config::parse("color_scheme = \"dark\"\n[appearance]\nscheme = \"light\"\n").unwrap();
        assert_eq!(c.appearance.scheme, ColorScheme::Light);
    }
}
