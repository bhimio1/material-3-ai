//! Running commands in a terminal window, for anything interactive (package
//! installs need a sudo password and show progress).

use super::cmd;

/// Known terminals and how each one takes a command to run.
const TERMINALS: &[(&str, &[&str])] = &[
    ("kitty", &["--"]),
    ("foot", &[]),
    ("alacritty", &["-e"]),
    ("ghostty", &["-e"]),
    ("wezterm", &["start", "--"]),
    ("konsole", &["-e"]),
    ("gnome-terminal", &["--"]),
    ("kgx", &["--"]),
    ("xfce4-terminal", &["-x"]),
    ("xterm", &["-e"]),
];

pub fn known() -> impl Iterator<Item = &'static str> {
    TERMINALS.iter().map(|(name, _)| *name)
}

/// The terminal that would be used: the configured one, `$TERMINAL`, then
/// the first installed known terminal.
pub fn pick(configured: &str) -> Option<String> {
    let configured = configured.trim();
    if !configured.is_empty() && cmd::exists(configured.split_whitespace().next()?) {
        return Some(configured.to_string());
    }
    if let Ok(t) = std::env::var("TERMINAL") {
        if cmd::exists(t.split_whitespace().next().unwrap_or("")) {
            return Some(t);
        }
    }
    known().find(|t| cmd::exists(t)).map(str::to_string)
}

/// Open `script` (a shell command line) in a terminal. The window stays open
/// afterwards so the output can be read.
pub fn run(configured: &str, script: &str) -> Result<String, String> {
    let terminal = pick(configured).ok_or_else(|| {
        "No terminal found. Set one in Preferences → Terminal.".to_string()
    })?;
    let mut parts = terminal.split_whitespace();
    let program = parts.next().unwrap_or_default().to_string();
    let mut args: Vec<String> = parts.map(str::to_string).collect();
    let flags = TERMINALS
        .iter()
        .find(|(name, _)| *name == program)
        .map(|(_, f)| *f)
        .unwrap_or(&["-e"]);
    if !args.iter().any(|a| flags.contains(&a.as_str())) {
        args.extend(flags.iter().map(|s| s.to_string()));
    }
    let wrapped = format!(
        "{script}; status=$?; echo; if [ $status -eq 0 ]; then echo 'Done.'; else echo \"Failed (exit $status).\"; fi; printf 'Press Enter to close…'; read -r _"
    );
    args.extend(["sh".into(), "-c".into(), wrapped]);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    cmd::spawn_detached(&program, &refs)?;
    Ok(program)
}

/// Quote a word for `sh`.
pub fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=+@%,".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(shell_quote("firefox"), "firefox");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote(""), "''");
    }
}
