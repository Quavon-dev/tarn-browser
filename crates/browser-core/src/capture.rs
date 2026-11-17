use std::io::Write;
use std::path::{Path, PathBuf};

use url::Url;

pub fn file_name(title: &str, url: Option<&Url>, ext: &str) -> String {
    let title = crate::text::bounded_title(title);
    let base = if title.is_empty() { url.and_then(Url::host_str).unwrap_or("Page") } else { &title };
    format!("{}.{ext}", engine::downloads::sanitize_filename(&base.replace(['/', '\\'], "-")))
}

pub fn save_new(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let path = engine::downloads::unique_path(dir, name, |p| p.symlink_metadata().is_ok());
    std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?.write_all(bytes)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_names_are_one_safe_component() {
        let url = Url::parse("https://news.example/a").ok();
        assert_eq!(file_name("Q3 / Q4 report", url.as_ref(), "pdf"), "Q3 - Q4 report.pdf");
        assert_eq!(file_name("  \u{202E}", url.as_ref(), "png"), "news.example.png");
        assert_eq!(file_name("", None, "png"), "Page.png");
        assert_eq!(file_name("../../etc", None, "pdf"), "-..-etc.pdf");
        assert_eq!(file_name("..", None, "pdf"), "download.pdf");
        assert_eq!(file_name("a:b\nc", None, "png"), "a_b c.png");
    }

    #[test]
    fn saves_never_replace_a_file_or_follow_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let first = save_new(dir.path(), "shot.png", b"one").unwrap();
        assert_eq!(first, dir.path().join("shot.png"));
        let second = save_new(dir.path(), "shot.png", b"two").unwrap();
        assert_ne!(second, first);
        assert_eq!(std::fs::read(&first).unwrap(), b"one");
        assert_eq!(std::fs::read(&second).unwrap(), b"two");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.path().join("elsewhere"), dir.path().join("link.png")).unwrap();
            assert_ne!(save_new(dir.path(), "link.png", b"three").unwrap(), dir.path().join("link.png"));
            assert!(!dir.path().join("elsewhere").exists());
        }
    }
}
