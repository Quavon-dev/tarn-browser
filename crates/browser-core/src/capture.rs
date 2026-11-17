use std::io::Write;
use std::path::{Path, PathBuf};

pub fn save_new(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let path = engine::downloads::free_path(dir, name, |_| false);
    std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?.write_all(bytes)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

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
