//! Thin wrappers around `std::process::Command`. Nothing here goes through a
//! shell unless the caller asks for `sh -c` explicitly.

use std::path::Path;
use std::process::{Command, Stdio};

/// Run a program and return its stdout. A non-zero exit becomes an error
/// carrying stderr (or stdout when stderr is empty).
pub fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if output.status.success() {
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            if stdout.trim().is_empty() {
                format!("{program} exited with {}", output.status)
            } else {
                stdout.trim().to_string()
            }
        } else {
            stderr
        })
    }
}

/// Start a program without waiting for it. It gets its own process group
/// so it survives this app closing.
pub fn spawn_detached(program: &str, args: &[&str]) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("{program}: {e}"))
}

/// Is `program` on `$PATH` (or an existing absolute path)?
pub fn exists(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_reports_failures() {
        assert_eq!(run("echo", &["hi"]).unwrap().trim(), "hi");
        assert!(run("false", &[]).is_err());
        assert!(run("definitely-not-a-real-program-m3s", &[]).is_err());
    }

    #[test]
    fn path_lookup() {
        assert!(exists("sh"));
        assert!(!exists("definitely-not-a-real-program-m3s"));
    }
}
