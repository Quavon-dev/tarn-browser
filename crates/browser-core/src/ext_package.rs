//! Extension packages: unpack a `.zip` or `.crx`, hash the tree, sweep leftovers.

use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;
use zip::{CompressionMethod, ZipArchive};

pub const MAX_ARCHIVE_BYTES: u64 = 64 << 20;
const MAX_ENTRIES: usize = 4_000;
const MAX_DEPTH: usize = 16;
const MAX_WRITTEN: u64 = 64 << 20;
const MAX_MANIFEST: u64 = 1 << 20;
const MAX_CRX_HEADER: usize = 64 << 10;
const MAX_COMPONENT: usize = 255;
pub const TEMP_PREFIX: &str = ".tmp-";

pub type Result<T> = std::result::Result<T, String>;

fn refuse<T>(why: &str) -> Result<T> {
    Err(why.to_owned())
}

/// The zip inside a CRX3 file; the header is skipped, never trusted, and its signature unchecked.
fn skip_crx(bytes: &[u8]) -> Result<&[u8]> {
    if !bytes.starts_with(b"Cr24") {
        return Ok(bytes);
    }
    let word = |at: usize| bytes.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    match (word(4), word(8)) {
        (Some(3), Some(len)) if len <= MAX_CRX_HEADER => bytes.get(12 + len..).map_or_else(|| refuse("the CRX file is truncated"), Ok),
        (Some(3), Some(_)) => refuse("the CRX header is too large"),
        (Some(_), Some(_)) => refuse("only CRX version 3 is supported"),
        _ => refuse("the CRX file is truncated"),
    }
}

/// The checked relative path of an entry, or `None` for the wrapper directory itself.
fn entry_path(name: &str, wrapped: Option<&mut Option<String>>) -> Result<Option<PathBuf>> {
    let mut parts: Vec<&str> = name.split('/').collect();
    if name.ends_with('/') {
        parts.pop();
    }
    if let Some(top) = wrapped {
        if parts.len() < 2 && !name.ends_with('/') {
            return refuse("the archive has files outside its top directory");
        }
        let first = parts.remove(0).to_owned();
        if top.get_or_insert_with(|| first.clone()) != &first {
            return refuse("the archive has more than one top directory");
        }
        if parts.is_empty() {
            return Ok(None);
        }
    }
    if parts.len() > MAX_DEPTH {
        return refuse("the archive is nested too deeply");
    }
    for part in &parts {
        let bad = part.is_empty()
            || *part == "."
            || *part == ".."
            || part.len() > MAX_COMPONENT
            || part.chars().any(|c| c.is_control() || c == '\\' || c == '\0' || c == ':')
            || part.ends_with(['.', ' '])
            || windows_reserved(part);
        if bad {
            return refuse("the archive holds an unsafe file name");
        }
    }
    let path: PathBuf = parts.iter().collect();
    if !path.components().all(|c| matches!(c, Component::Normal(_))) {
        return refuse("the archive holds an unsafe file name");
    }
    Ok(Some(path))
}

fn windows_reserved(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or_default().trim_end_matches(' ').to_ascii_uppercase();
    match stem.as_bytes() {
        b"CON" | b"PRN" | b"AUX" | b"NUL" => true,
        [b'C', b'O', b'M', n] | [b'L', b'P', b'T', n] => (b'1'..=b'9').contains(n),
        _ => false,
    }
}

/// How the file system may see a name: case and Unicode form do not tell entries apart on APFS.
fn fold(path: &Path) -> String {
    path.to_string_lossy().nfd().collect::<String>().to_lowercase().nfc().collect()
}

const S_IFMT: u32 = 0o170_000;
const S_IFREG: u32 = 0o100_000;
const S_IFDIR: u32 = 0o040_000;

/// Unpacks `bytes` into `dest`, which must not exist yet. With `wrapped`, the archive's single
/// top directory (as in a GitHub source archive) is stripped.
pub fn unpack(bytes: &[u8], dest: &Path, wrapped: bool) -> Result<()> {
    let zip = skip_crx(bytes)?;
    let parent = dest.parent().ok_or("no parent directory")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temp = parent.join(format!("{TEMP_PREFIX}{}", uuid::Uuid::new_v4()));
    let unpacked = extract(zip, &temp, wrapped).and_then(|()| check_manifest(&temp));
    // macOS 15 refuses to rename a directory without write permission, so the top is sealed after the move.
    let done = unpacked.and_then(|()| {
        seal(&temp).and_then(|()| set_mode(&temp, 0o700)).map_err(|e| e.to_string())?;
        fs::rename(&temp, dest).map_err(|e| e.to_string())?;
        set_mode(dest, 0o555).map_err(|e| {
            remove_tree(dest).ok();
            e.to_string()
        })
    });
    if done.is_err() {
        remove_tree(&temp).ok();
    }
    done
}

fn extract(zip: &[u8], temp: &Path, wrapped: bool) -> Result<()> {
    let mut archive = ZipArchive::new(Cursor::new(zip)).map_err(|_| "not a valid zip archive".to_owned())?;
    if archive.len() > MAX_ENTRIES {
        return refuse("the archive has too many files");
    }
    archive.validate_header_consistency().map_err(|_| "the archive's headers disagree".to_owned())?;
    fs::create_dir(temp).map_err(|e| e.to_string())?;
    let mut seen = HashSet::new();
    let mut written = 0u64;
    let mut top = None;
    for i in 0..archive.len() {
        let entry = archive.by_index_raw(i).map_err(|_| "not a valid zip archive".to_owned())?;
        let name = std::str::from_utf8(entry.name_raw()).map_err(|_| "the archive holds a file name that is not UTF-8".to_owned())?.to_owned();
        if entry.encrypted() {
            return refuse("the archive is encrypted");
        }
        let kind = entry.unix_mode().map(|m| m & S_IFMT).filter(|k| *k != 0);
        let dir = name.ends_with('/');
        if entry.is_symlink() || kind.is_some_and(|k| k != if dir { S_IFDIR } else { S_IFREG }) {
            return refuse("the archive holds a link or special file");
        }
        let Some(path) = entry_path(&name, wrapped.then_some(&mut top))? else { continue };
        if !seen.insert(fold(&path)) {
            return refuse("the archive holds two files with the same name");
        }
        let target = temp.join(&path);
        if dir {
            fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            continue;
        }
        if !matches!(entry.compression(), CompressionMethod::Stored | CompressionMethod::Deflated) {
            return refuse("the archive uses an unsupported compression method");
        }
        drop(entry);
        let mut entry = archive.by_index(i).map_err(|_| "not a valid zip archive".to_owned())?;
        if let Some(dir) = target.parent() {
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let mut options = OpenOptions::new();
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.write(true).create_new(true).open(&target).map_err(|e| e.to_string())?;
        let room = MAX_WRITTEN - written;
        let copied = std::io::copy(&mut (&mut entry).take(room + 1), &mut file).map_err(|_| "the archive is damaged".to_owned())?;
        if copied > room {
            return refuse("the archive unpacks to more than 64 MB");
        }
        written += copied;
        file.flush().map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn check_manifest(dir: &Path) -> Result<()> {
    let path = dir.join("manifest.json");
    let meta = fs::symlink_metadata(&path).map_err(|_| "no manifest.json at the top of the archive".to_owned())?;
    if !meta.is_file() || meta.len() > MAX_MANIFEST {
        return refuse("manifest.json is missing or too large");
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).map_err(|e| e.to_string())?).map_err(|_| "manifest.json is not valid JSON".to_owned())?;
    match manifest.get("manifest_version").and_then(|v| v.as_u64()) {
        Some(2 | 3) => Ok(()),
        _ => refuse("manifest.json states no supported manifest_version"),
    }
}

/// Files `0444`, directories `0555`, children before their parent.
fn seal(dir: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if fs::symlink_metadata(&path)?.is_dir() {
            seal(&path)?;
        } else {
            set_mode(&path, 0o444)?;
        }
    }
    set_mode(dir, 0o555)
}

/// Windows keeps only a read-only flag, which it honours on files.
pub(crate) fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    return fs::set_permissions(path, fs::Permissions::from_mode(mode));
    #[cfg(not(unix))]
    {
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_readonly(mode & 0o200 == 0);
        fs::set_permissions(path, permissions)
    }
}

/// Deletes a tree whose directories may be read-only.
pub fn remove_tree(path: &Path) -> std::io::Result<()> {
    let Ok(meta) = fs::symlink_metadata(path) else { return Ok(()) };
    if meta.is_dir() {
        set_mode(path, 0o700)?;
        for entry in fs::read_dir(path)? {
            remove_tree(&entry?.path())?;
        }
        fs::remove_dir(path)
    } else {
        // Windows refuses to delete a read-only file.
        #[cfg(not(unix))]
        if meta.is_file() {
            set_mode(path, 0o600)?;
        }
        fs::remove_file(path)
    }
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let meta = fs::symlink_metadata(&path)?;
        if meta.is_dir() {
            walk(root, &path, out)?;
        } else if meta.is_file() {
            let rel = path.strip_prefix(root).map_err(std::io::Error::other)?;
            out.push(rel.to_str().ok_or_else(|| std::io::Error::other("a file name is not UTF-8"))?.to_owned());
        } else {
            return Err(std::io::Error::other("the extension holds a link or special file"));
        }
    }
    Ok(())
}

/// sha256 over `len(path) ‖ path ‖ len(data) ‖ data` for every file, by sorted relative path.
pub fn tree_sha256(dir: &Path) -> std::io::Result<String> {
    let mut paths = Vec::new();
    walk(dir, dir, &mut paths)?;
    paths.sort();
    let mut hash = Sha256::new();
    for rel in paths {
        let data = fs::read(dir.join(&rel))?;
        hash.update((rel.len() as u64).to_be_bytes());
        hash.update(rel.as_bytes());
        hash.update((data.len() as u64).to_be_bytes());
        hash.update(&data);
    }
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Deletes temporary directories and every profile, install or version directory under `root`
/// (`<root>/<profile>/<install>/<n>`) that no path in `keep` lies in.
pub fn sweep(root: &Path, keep: &[PathBuf]) {
    fn level(dir: &Path, depth: usize, keep: &[PathBuf]) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let temp = entry.file_name().to_string_lossy().starts_with(TEMP_PREFIX);
            if temp || !keep.iter().any(|k| k.starts_with(&path)) {
                remove_tree(&path).ok();
            } else if depth < 2 {
                level(&path, depth + 1, keep);
            }
        }
    }
    level(root, 0, keep);
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::{SimpleFileOptions, ZipWriter};

    fn archive(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, data) in files {
            if name.ends_with('/') {
                zip.add_directory(*name, SimpleFileOptions::default()).unwrap();
            } else {
                zip.start_file(*name, SimpleFileOptions::default().compression_method(CompressionMethod::Deflated)).unwrap();
                zip.write_all(data).unwrap();
            }
        }
        zip.finish().unwrap().into_inner()
    }

    const MANIFEST: (&str, &[u8]) = ("manifest.json", br#"{"manifest_version":3,"name":"x","version":"1"}"#);

    fn try_unpack(bytes: &[u8], wrapped: bool) -> (tempfile::TempDir, Result<()>) {
        let root = tempfile::tempdir().unwrap();
        let result = unpack(bytes, &root.path().join("out"), wrapped);
        (root, result)
    }

    fn refused(files: &[(&str, &[u8])]) {
        let (root, result) = try_unpack(&archive(files), false);
        assert!(result.is_err(), "{files:?} was accepted");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0, "a temporary directory was left behind");
    }

    #[test]
    fn unpacks_read_only_and_hashes_the_same_tree_alike() {
        let bytes = archive(&[MANIFEST, ("js/", b""), ("js/bg.js", b"1")]);
        let (root, result) = try_unpack(&bytes, false);
        result.unwrap();
        let out = root.path().join("out");
        #[cfg(unix)]
        assert_eq!(fs::metadata(out.join("js/bg.js")).unwrap().permissions().mode() & 0o777, 0o444);
        #[cfg(unix)]
        assert_eq!(fs::metadata(out.join("js")).unwrap().permissions().mode() & 0o777, 0o555);
        assert!(fs::metadata(out.join("js/bg.js")).unwrap().permissions().readonly());
        let (again, _) = try_unpack(&bytes, false);
        assert_eq!(tree_sha256(&out).unwrap(), tree_sha256(&again.path().join("out")).unwrap());
        remove_tree(&out).unwrap();
        assert!(!out.exists());
    }

    #[test]
    fn refuses_unsafe_names() {
        refused(&[MANIFEST, ("../evil.js", b"")]);
        refused(&[MANIFEST, ("a/../../evil.js", b"")]);
        refused(&[MANIFEST, ("a/./b.js", b"")]);
        refused(&[MANIFEST, ("/abs.js", b"")]);
        refused(&[MANIFEST, ("a//b.js", b"")]);
        refused(&[MANIFEST, ("a\\b.js", b"")]);
        refused(&[MANIFEST, ("C:evil.js", b"")]);
        refused(&[MANIFEST, ("a.js:stream", b"")]);
        refused(&[MANIFEST, ("a\u{1}b.js", b"")]);
        refused(&[MANIFEST, (&"x".repeat(256), b"")]);
        refused(&[MANIFEST, ("d/".repeat(17).trim_end_matches('/'), b"")]);
        for name in ["CON", "nul.js", "a/Com1.txt", "lpt9", "aux.tar.gz", "a./b.js", "b.js.", "c.js "] {
            refused(&[MANIFEST, (name, b"")]);
        }
        let (_, ok) = try_unpack(&archive(&[MANIFEST, ("console.js", b""), ("com10.js", b""), ("lpt0.js", b"")]), false);
        ok.unwrap();
    }

    #[test]
    fn refuses_duplicate_and_colliding_names() {
        refused(&[MANIFEST, ("A.js", b""), ("a.js", b"")]);
        refused(&[MANIFEST, ("caf\u{e9}.js", b""), ("cafe\u{301}.js", b"")]);
    }

    #[test]
    fn refuses_a_symlink() {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file(MANIFEST.0, SimpleFileOptions::default()).unwrap();
        zip.write_all(MANIFEST.1).unwrap();
        zip.add_symlink("link", "/etc/passwd", SimpleFileOptions::default()).unwrap();
        let (_root, result) = try_unpack(&zip.finish().unwrap().into_inner(), false);
        assert_eq!(result.unwrap_err(), "the archive holds a link or special file");
    }

    fn patch(bytes: &mut [u8], from: &[u8], to: &[u8], nth: usize) {
        let at = bytes.windows(from.len()).enumerate().filter(|(_, w)| *w == from).nth(nth).unwrap().0;
        bytes[at..at + to.len()].copy_from_slice(to);
    }

    #[test]
    fn refuses_a_name_that_is_not_utf8() {
        let mut bytes = archive(&[MANIFEST, ("bad.js", b"")]);
        patch(&mut bytes, b"bad.js", b"b\xffd.js", 0);
        patch(&mut bytes, b"bad.js", b"b\xffd.js", 0);
        assert_eq!(try_unpack(&bytes, false).1.unwrap_err(), "the archive holds a file name that is not UTF-8");
    }

    #[test]
    fn refuses_local_and_central_headers_that_disagree() {
        let mut bytes = archive(&[MANIFEST, ("one.js", b"")]);
        patch(&mut bytes, b"one.js", b"two.js", 0);
        assert_eq!(try_unpack(&bytes, false).1.unwrap_err(), "the archive's headers disagree");
    }

    #[test]
    fn refuses_an_encrypted_entry() {
        let mut bytes = archive(&[MANIFEST]);
        bytes[6] |= 1;
        let central = bytes.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        bytes[central + 8] |= 1;
        assert_eq!(try_unpack(&bytes, false).1.unwrap_err(), "the archive is encrypted");
    }

    #[test]
    fn counts_what_is_written_not_what_headers_claim() {
        let big = vec![0u8; (MAX_WRITTEN as usize) / 2 + 1];
        let bytes = archive(&[MANIFEST, ("a.bin", &big), ("b.bin", &big)]);
        assert!(try_unpack(&bytes, false).1.is_err());
        let mut small = archive(&[MANIFEST, ("a.bin", &[7u8; 4096])]);
        let size = 4096u32.to_le_bytes();
        patch(&mut small, &size, &10u32.to_le_bytes(), 0);
        patch(&mut small, &size, &10u32.to_le_bytes(), 0);
        assert!(try_unpack(&small, false).1.is_err());
    }

    fn crx(header_len: u32, header: &[u8], zip: &[u8]) -> Vec<u8> {
        [b"Cr24".as_slice(), &3u32.to_le_bytes(), &header_len.to_le_bytes(), header, zip].concat()
    }

    #[test]
    fn crx3_headers_are_skipped_and_bounded() {
        let zip = archive(&[MANIFEST]);
        assert!(try_unpack(&crx(4, &[0; 4], &zip), false).1.is_ok());
        assert!(try_unpack(&crx(1 << 20, &[], &zip), false).1.is_err());
        assert!(try_unpack(&crx(4000, &[0; 10], &[]), false).1.is_err());
        let crx2 = [b"Cr24".as_slice(), &2u32.to_le_bytes(), &0u32.to_le_bytes(), &zip].concat();
        assert!(try_unpack(&crx2, false).1.is_err());
    }

    #[test]
    fn the_manifest_must_be_at_the_root() {
        refused(&[("sub/manifest.json", MANIFEST.1)]);
        refused(&[("manifest.json", br#"{"manifest_version":1}"#)]);
        let wrapped = archive(&[("repo-1.0/", b""), ("repo-1.0/manifest.json", MANIFEST.1)]);
        assert!(try_unpack(&wrapped, true).1.is_ok());
        assert!(try_unpack(&wrapped, false).1.is_err());
        assert!(try_unpack(&archive(&[MANIFEST, ("repo/x.js", b"")]), true).1.is_err());
        let two = archive(&[("a/manifest.json", MANIFEST.1), ("b/x.js", b"")]);
        assert!(try_unpack(&two, true).1.is_err());
    }

    #[test]
    fn hashes_tell_apart_where_a_path_ends() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        fs::write(a.path().join("ab"), b"c").unwrap();
        fs::write(b.path().join("a"), b"bc").unwrap();
        assert_ne!(tree_sha256(a.path()).unwrap(), tree_sha256(b.path()).unwrap());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("ab", a.path().join("link")).unwrap();
            assert!(tree_sha256(a.path()).is_err());
        }
    }

    #[test]
    fn sweep_removes_orphans_although_read_only() {
        let root = tempfile::tempdir().unwrap();
        let kept = root.path().join("1/aaa/2");
        let bytes = archive(&[MANIFEST]);
        for dir in ["1/aaa/1", "1/aaa/2", "1/bbb/1", "9/ccc/1"] {
            unpack(&bytes, &root.path().join(dir), false).unwrap();
        }
        fs::create_dir_all(root.path().join(format!("1/{TEMP_PREFIX}x"))).unwrap();
        sweep(root.path(), std::slice::from_ref(&kept));
        let mut left: Vec<_> = ["1", "1/aaa", "1/aaa/1", "1/aaa/2", "1/bbb", "9", "1/.tmp-x"].into_iter().filter(|p| root.path().join(p).exists()).collect();
        left.sort();
        assert_eq!(left, ["1", "1/aaa", "1/aaa/2"]);
        assert!(kept.join("manifest.json").exists());
    }
}
