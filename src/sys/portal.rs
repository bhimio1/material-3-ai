//! File and folder pickers through xdg-desktop-portal (the Material 3 File
//! Manager can serve these with `m3fm --portal`). Blocking: call from a
//! background task.

use std::path::PathBuf;

fn run<T>(fut: impl std::future::Future<Output = T>) -> Option<T> {
    tokio::runtime::Builder::new_current_thread().enable_all().build().ok().map(|rt| rt.block_on(fut))
}

fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    Some(PathBuf::from(percent_decode(rest)))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Ask for a folder. `Err` means the portal isn't available.
pub fn pick_folder(title: &str) -> Result<Option<PathBuf>, String> {
    use ashpd::desktop::file_chooser::SelectedFiles;
    let title = title.to_string();
    run(async move {
        let req = SelectedFiles::open_file().title(title.as_str()).directory(true).modal(true).send().await.map_err(|e| e.to_string())?;
        match req.response() {
            Ok(files) => Ok(files.uris().iter().find_map(|u| uri_to_path(u.as_str()))),
            Err(_) => Ok(None), // cancelled
        }
    })
    .unwrap_or_else(|| Err("couldn't start the portal client".into()))
}

/// Ask for image files.
pub fn pick_images() -> Result<Vec<PathBuf>, String> {
    use ashpd::desktop::file_chooser::{FileFilter, SelectedFiles};
    run(async move {
        let filter = FileFilter::new("Images")
            .mimetype("image/png")
            .mimetype("image/jpeg")
            .mimetype("image/gif")
            .mimetype("image/webp");
        let req = SelectedFiles::open_file()
            .title("Attach images")
            .multiple(true)
            .filter(filter)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        match req.response() {
            Ok(files) => Ok(files.uris().iter().filter_map(|u| uri_to_path(u.as_str())).collect()),
            Err(_) => Ok(Vec::new()),
        }
    })
    .unwrap_or_else(|| Err("couldn't start the portal client".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris() {
        assert_eq!(uri_to_path("file:///home/me/My%20Project"), Some(PathBuf::from("/home/me/My Project")));
        assert_eq!(uri_to_path("https://x"), None);
        assert_eq!(percent_decode("a%2"), "a%2");
    }
}
