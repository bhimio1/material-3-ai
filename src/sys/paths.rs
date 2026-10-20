//! Path helpers: `~` expansion and display.

use std::path::{Path, PathBuf};

pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

pub fn config_home() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| home().join(".config"))
}

/// Expand a leading `~` and `$XDG_CONFIG_HOME` / `$HOME`.
pub fn expand(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        return home().join(rest);
    }
    if path == "~" {
        return home();
    }
    if let Some(rest) = path.strip_prefix("$XDG_CONFIG_HOME/") {
        return config_home().join(rest);
    }
    if let Some(rest) = path.strip_prefix("$HOME/") {
        return home().join(rest);
    }
    PathBuf::from(path)
}

/// Show a path with the home directory as `~`.
pub fn tilde(path: &Path) -> String {
    let home = home();
    match path.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expansion_round_trips() {
        let p = expand("~/.config/kitty/kitty.conf");
        assert!(p.is_absolute());
        assert_eq!(tilde(&p), "~/.config/kitty/kitty.conf");
        assert_eq!(expand("/etc/pacman.conf"), PathBuf::from("/etc/pacman.conf"));
    }
}
