//! Open File: a local file the user picked, shown in a tab (S6: `file:` only by explicit user action).

use std::io;
use std::path::Path;

use url::Url;

/// The URL to give `load_file` for a file the user picked; `None` for a folder or a missing file.
pub fn url_for(path: &Path) -> Option<Url> {
    let path = path.canonicalize().ok().filter(|p| p.is_file())?;
    Url::from_file_path(path).ok()
}

/// Copies a picked file that came as bytes (iOS, Android) into a folder of its own under `dir`, so
/// the engine is granted only that folder, and returns its URL.
pub fn stage(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<Url> {
    std::fs::create_dir_all(dir)?;
    let folder = (0..u32::MAX)
        .map(|n| dir.join(n.to_string()))
        .find_map(|folder| match std::fs::create_dir(&folder) {
            Ok(()) => Some(Ok(folder)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => None,
            Err(e) => Some(Err(e)),
        })
        .unwrap_or_else(|| Err(io::ErrorKind::StorageFull.into()))?;
    let path = folder.join(engine::downloads::sanitize_filename(name));
    std::fs::write(&path, bytes)?;
    url_for(&path).ok_or_else(|| io::ErrorKind::InvalidInput.into())
}

/// Deletes every staged copy; the shell calls it at launch, when no tab can show one.
pub fn clear(dir: &Path) {
    std::fs::remove_dir_all(dir).ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_each_file_in_its_own_folder_with_a_safe_name() {
        let dir = tempfile::tempdir().unwrap();
        let staged = dir.path().join("opened");
        let a = stage(&staged, "../../evil\u{202E}.html", b"<p>a</p>").unwrap();
        let b = stage(&staged, "page.html", b"<p>b</p>").unwrap();
        let (a, b) = (a.to_file_path().unwrap(), b.to_file_path().unwrap());
        assert_eq!(std::fs::read(&a).unwrap(), b"<p>a</p>");
        assert!(a.starts_with(staged.canonicalize().unwrap()) && a.file_name().unwrap() == "evil.html");
        assert_ne!(a.parent(), b.parent());
        assert_eq!(engine::nav_policy::file_read_access(&url_for(&b).unwrap()).as_deref(), b.parent());
        clear(&staged);
        assert!(!staged.exists());
    }

    #[test]
    fn only_existing_files_get_a_url() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(url_for(dir.path()), None);
        assert_eq!(url_for(&dir.path().join("missing.html")), None);
        let file = dir.path().join("a b.pdf");
        std::fs::write(&file, b"%PDF").unwrap();
        assert!(url_for(&file).is_some_and(|u| u.scheme() == "file" && u.path().ends_with("/a%20b.pdf")));
    }
}
