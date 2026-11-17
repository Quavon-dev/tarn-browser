use std::path::{Path, PathBuf};

/// Below NAME_MAX (255) so `unique_path` can still add a " (n)" suffix.
const MAX_NAME_BYTES: usize = 240;

/// Extensions macOS will execute or install on open.
const DANGEROUS: &[&str] = &[
    "app",
    "command",
    "tool",
    "sh",
    "zsh",
    "bash",
    "csh",
    "pkg",
    "mpkg",
    "dmg",
    "terminal",
    "scpt",
    "scptd",
    "applescript",
    "workflow",
    "action",
    "jar",
    "py",
    "pl",
    "rb",
    "dylib",
    "kext",
    "prefpane",
    "webloc",
    "inetloc",
    "fileloc",
    "url",
    "exe",
    "msi",
    "bat",
    "cmd",
    "ps1",
    "vbs",
    "mobileconfig",
    "configprofile",
    "shortcut",
    "iso",
    "img",
    "sparseimage",
    "sparsebundle",
    "bundle",
    "plugin",
    "osax",
    "saver",
    "jnlp",
    "html",
    "htm",
    "svg",
    "xhtml",
    "xht",
    "shtml",
    "mht",
    "mhtml",
    "xml",
    "xsl",
    "webarchive",
    "xpc",
    "appex",
    "framework",
    "systemextension",
    "qlgenerator",
    "mdimporter",
    "service",
    "wdgt",
    "xip",
    "mobileprovision",
    "provisionprofile",
    "definition",
    "inputplugin",
    "afploc",
    "ftploc",
    "smbloc",
    "vncloc",
    "atloc",
    "mailloc",
    "nfsloc",
    "lnk",
    "scr",
    "hta",
    "js",
    "jse",
    "vbe",
    "wsf",
    "wsh",
    "com",
    "pif",
    "reg",
    "cpl",
    "msc",
    "msix",
    "msixbundle",
    "appx",
    "appxbundle",
    "appinstaller",
    "application",
    "gadget",
    "inf",
    "psm1",
    "chm",
    "msp",
    "mst",
    "dll",
    "vb",
    "ws",
    "wsc",
    "ps1xml",
    "psd1",
    "settingcontent-ms",
    "library-ms",
    "search-ms",
    "appref-ms",
    "xbap",
    "website",
    "diagcab",
    "xll",
    "vhd",
    "vhdx",
];

/// Names Windows maps to devices whatever their extension.
#[cfg_attr(not(windows), allow(dead_code))]
const DEVICES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7",
    "LPT8", "LPT9", "COM¹", "COM²", "COM³", "LPT¹", "LPT²", "LPT³", "CONIN$", "CONOUT$",
];

fn is_bidi_or_control(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{061C}' | '\u{115F}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{3164}' | '\u{FEFF}' | '\u{FFA0}' | '\u{E0000}'..='\u{E007F}')
}

/// Reduces a server-suggested name to a single safe path component.
pub fn sanitize_filename(suggested: &str) -> String {
    let base = suggested.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String =
        base.chars().filter(|c| !is_bidi_or_control(*c)).map(|c| if c == ':' || (cfg!(windows) && "<>\"|?*".contains(c)) { '_' } else { c }).collect();
    let trimmed = cleaned.trim_matches(|c: char| c == '.' || c.is_whitespace());
    let name = if trimmed.is_empty() { "download" } else { trimmed };
    let device = cfg!(windows) && DEVICES.iter().any(|d| name.split('.').next().is_some_and(|stem| stem.trim_end().eq_ignore_ascii_case(d)));
    truncate_keeping_extension(&if device { format!("_{name}") } else { name.to_owned() }).trim_end_matches(['.', ' ']).to_owned()
}

fn truncate_keeping_extension(name: &str) -> String {
    if name.len() <= MAX_NAME_BYTES {
        return name.to_owned();
    }
    let ext = name.rsplit_once('.').map(|(_, e)| e).filter(|e| e.len() < 16).unwrap_or("");
    let budget = MAX_NAME_BYTES - if ext.is_empty() { 0 } else { ext.len() + 1 };
    let mut stem_end = budget.min(name.len());
    while !name.is_char_boundary(stem_end) {
        stem_end -= 1;
    }
    if ext.is_empty() { name[..stem_end].to_owned() } else { format!("{}.{ext}", &name[..stem_end]) }
}

/// First path for `name` in `dir` that is neither on disk (a link counts, even a dangling one) nor
/// `in_use` by a download still being saved, which may not have created its file yet.
pub fn free_path(dir: &Path, name: &str, in_use: impl Fn(&Path) -> bool) -> PathBuf {
    unique_path(dir, name, |p| p.symlink_metadata().is_ok() || in_use(p))
}

/// True when opening the file could run code, including disguised names like `photo.jpg.app`.
pub fn is_dangerous(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, ext)| DANGEROUS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// First free path for `name` in `dir`, adding " (2)", " (3)" … before the extension.
pub fn unique_path(dir: &Path, name: &str, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let candidate = dir.join(name);
    if !exists(&candidate) {
        return candidate;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };
    (2..).map(|n| dir.join(format!("{stem} ({n}){ext}"))).find(|p| !exists(p)).expect("unbounded range")
}

/// The first of `urls` (request, response, then the page that started it) whose host is on the
/// switched-on malware set: such a download waits until the user chooses to keep it.
pub fn flagged_source(urls: impl IntoIterator<Item = Option<url::Url>>, site_for: impl Fn(&url::Url) -> crate::nav_policy::Site) -> Option<url::Url> {
    urls.into_iter().flatten().find(|u| site_for(u).malware)
}

/// What the prompt to keep a download from `from`, the URL `flagged_source` found, says.
pub fn malware_warning(from: &url::Url) -> String {
    let device = if cfg!(windows) { "PC" } else { "Mac" };
    format!("{} is on the malware list. The file may harm your {device}.", from.host_str().unwrap_or_default())
}

/// Downloads subfolders by lowercase extension; other types stay at the top.
const FOLDERS: &[(&str, &[&str])] = &[
    ("Images", &["jpg", "jpeg", "png", "gif", "webp", "heic", "heif", "avif", "tif", "tiff", "bmp", "ico", "svg", "dng"]),
    ("Videos", &["mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv", "mpg", "mpeg"]),
    ("Audio", &["mp3", "m4a", "aac", "wav", "flac", "ogg", "oga", "opus", "aif", "aiff"]),
    ("Documents", &["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "key", "pages", "numbers", "odt", "ods", "odp", "rtf", "txt", "md", "csv", "epub"]),
    ("Archives", &["zip", "tar", "gz", "tgz", "bz2", "xz", "7z", "rar", "zst"]),
    ("Installers", &["dmg", "pkg", "mpkg", "iso", "xip", "exe", "msi", "msix", "appx"]),
];

/// The Downloads subfolder a file of this name is sorted into, `None` for other types.
pub fn folder_for(name: &str) -> Option<&'static str> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    FOLDERS.iter().find(|(_, exts)| exts.contains(&ext.as_str())).map(|(folder, _)| *folder)
}

/// `downloads/folder`, created when missing; `None` when the name is taken by anything but a real
/// folder, so a link there is never followed.
pub fn sorted_dir(downloads: &Path, folder: &str) -> Option<PathBuf> {
    let dir = downloads.join(folder);
    match dir.symlink_metadata() {
        Ok(meta) => meta.is_dir().then_some(dir),
        Err(_) => std::fs::create_dir(&dir).ok().map(|()| dir),
    }
}

/// Where a started download is. `D` is the engine's handle to it, `R` the data to continue from.
#[derive(Debug, PartialEq, Eq)]
pub enum Transfer<D, R> {
    Saving(D),
    /// Cancelled to pause, until the engine hands over the data to continue from.
    Pausing(D),
    Paused(D, R),
    /// Asked to continue, until the engine hands over the new handle.
    Resuming,
}

impl<D, R> Transfer<D, R> {
    /// The engine's handle, by which its later reports are matched.
    pub fn handle(&self) -> Option<&D> {
        match self {
            Transfer::Saving(d) | Transfer::Pausing(d) | Transfer::Paused(d, _) => Some(d),
            Transfer::Resuming => None,
        }
    }

    /// The engine is writing it and will report it finished or failed. A decline at the malware
    /// prompt cancels such a download and lets that report remove it, and quit keeps its part;
    /// any other is removed at once, as no report follows.
    pub fn running(&self) -> bool {
        matches!(self, Transfer::Saving(_))
    }

    pub fn is_saving(&self) -> bool {
        matches!(self, Transfer::Saving(_) | Transfer::Resuming)
    }

    /// Saving to pausing; true when the download must now be cancelled.
    pub fn pause(&mut self) -> bool {
        match std::mem::replace(self, Transfer::Resuming) {
            Transfer::Saving(d) => {
                *self = Transfer::Pausing(d);
                true
            }
            other => {
                *self = other;
                false
            }
        }
    }

    /// Pausing to paused once the data to continue from arrived; false if it was not pausing.
    pub fn paused(&mut self, data: R) -> bool {
        match std::mem::replace(self, Transfer::Resuming) {
            Transfer::Pausing(d) => {
                *self = Transfer::Paused(d, data);
                true
            }
            other => {
                *self = other;
                false
            }
        }
    }

    /// Paused to resuming; the data to continue from, `None` if it was not paused.
    pub fn resume(&mut self) -> Option<R> {
        match std::mem::replace(self, Transfer::Resuming) {
            Transfer::Paused(_, data) => Some(data),
            other => {
                *self = other;
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn windows_names_are_never_devices_or_invalid() {
        assert_eq!(sanitize_filename("CON"), "_CON");
        assert_eq!(sanitize_filename("nul.txt"), "_nul.txt");
        assert_eq!(sanitize_filename("com1 .tar.gz"), "_com1 .tar.gz");
        assert_eq!(sanitize_filename("a<b>c|d?.txt"), "a_b_c_d_.txt");
        assert_eq!(sanitize_filename("report. "), "report");
        assert_eq!(sanitize_filename("console.log"), "console.log");
        assert_eq!(sanitize_filename("COM¹.txt"), "_COM¹.txt");
        assert_eq!(sanitize_filename("conout$"), "_conout$");
        assert!(is_dangerous("manual.chm") && is_dangerous("x.settingcontent-ms"));
        assert!(is_dangerous("setup.lnk") && is_dangerous("x.HTA") && is_dangerous("app.msix"));
    }

    #[test]
    fn unique_path_numbers_before_extension() {
        let taken = ["/d/a.pdf", "/d/a (2).pdf", "/d/noext"];
        let exists = |p: &Path| taken.iter().any(|t| Path::new(t) == p);
        assert_eq!(unique_path(Path::new("/d"), "a.pdf", exists), Path::new("/d/a (3).pdf"));
        assert_eq!(unique_path(Path::new("/d"), "b.pdf", exists), Path::new("/d/b.pdf"));
        assert_eq!(unique_path(Path::new("/d"), "noext", exists), Path::new("/d/noext (2)"));
    }

    #[test]
    fn free_path_skips_files_links_and_downloads_in_progress() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.zip"), b"x").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.path().join("missing"), dir.path().join("a (2).zip")).unwrap();
        #[cfg(not(unix))]
        std::fs::write(dir.path().join("a (2).zip"), b"x").unwrap();
        let saving = dir.path().join("a (3).zip");
        assert_eq!(free_path(dir.path(), "a.zip", |p| p == saving), dir.path().join("a (4).zip"));
        assert_eq!(free_path(dir.path(), "b.zip", |_| false), dir.path().join("b.zip"));
    }

    #[test]
    fn strips_paths_and_traversal() {
        assert_eq!(sanitize_filename("../../x"), "x");
        assert_eq!(sanitize_filename("..\\..\\evil.txt"), "evil.txt");
        assert_eq!(sanitize_filename("/etc/passwd"), "passwd");
        assert_eq!(sanitize_filename(".."), "download");
        assert_eq!(sanitize_filename(""), "download");
        assert_eq!(sanitize_filename(".hidden"), "hidden");
        assert_eq!(sanitize_filename("a:b\0c\n.txt"), "a_bc.txt");
    }

    #[test]
    fn double_extensions_and_bidi_spoofing() {
        assert!(is_dangerous("photo.jpg.app"));
        assert!(is_dangerous("Invoice.PDF.Command"));
        assert!(is_dangerous("share.afploc") && is_dangerous("server.SMBLOC") && is_dangerous("mount.nfsloc"));
        assert!(!is_dangerous("report.pdf"));
        assert!(!is_dangerous("archive.tar.gz"));
        let spoofed = sanitize_filename("invoice\u{202E}fdp.app");
        assert_eq!(spoofed, "invoicefdp.app");
        assert!(is_dangerous(&spoofed));
        assert_eq!(sanitize_filename("a\u{3164}b\u{E0041}.txt"), "ab.txt");
    }

    #[test]
    fn only_a_saving_transfer_waits_for_the_engine() {
        let states: [Transfer<u8, &str>; 4] = [Transfer::Saving(1), Transfer::Pausing(1), Transfer::Paused(1, "data"), Transfer::Resuming];
        assert_eq!(states.map(|t| t.running()), [true, false, false, false]);
    }

    #[test]
    fn transfers_pause_and_resume_only_in_order() {
        let mut t: Transfer<u8, &str> = Transfer::Saving(1);
        assert!(t.is_saving() && !t.paused("early") && t.resume().is_none());
        assert!(t.pause() && !t.pause());
        assert_eq!(t, Transfer::Pausing(1));
        assert!(!t.is_saving() && t.resume().is_none() && t.handle() == Some(&1));
        assert!(t.paused("data") && !t.paused("again"));
        assert_eq!(t, Transfer::Paused(1, "data"));
        assert_eq!(t.handle(), Some(&1));
        assert_eq!(t.resume(), Some("data"));
        assert_eq!(t, Transfer::Resuming);
        assert!(t.is_saving() && t.handle().is_none() && !t.pause() && t.resume().is_none());
    }

    #[test]
    fn the_first_listed_url_flags_a_download() {
        let site = |malware| crate::nav_policy::Site { malware, ..crate::nav_policy::Site::new(shield::ShieldLevel::Standard) };
        let u = |s: &str| Some(url::Url::parse(s).unwrap());
        let listed = |url: &url::Url| site(url.host_str() == Some("bad.test"));
        assert_eq!(flagged_source([u("https://ok.test/f"), None, u("https://bad.test/p")], listed), u("https://bad.test/p"));
        assert_eq!(flagged_source([u("https://ok.test/f"), None], listed), None);
        let expected = if cfg!(windows) {
            "bad.test is on the malware list. The file may harm your PC."
        } else {
            "bad.test is on the malware list. The file may harm your Mac."
        };
        assert_eq!(malware_warning(&u("https://bad.test/p").unwrap()), expected);
    }

    #[test]
    fn types_map_to_folders() {
        assert_eq!(folder_for("Photo.JPG"), Some("Images"));
        assert_eq!(folder_for("clip.webm"), Some("Videos"));
        assert_eq!(folder_for("song.flac"), Some("Audio"));
        assert_eq!(folder_for("paper.pdf"), Some("Documents"));
        assert_eq!(folder_for("src.tar.gz"), Some("Archives"));
        assert_eq!(folder_for("Tarn.dmg"), Some("Installers"));
        assert_eq!(folder_for("photo.jpg.app"), None);
        assert_eq!(folder_for("README"), None);
        assert_eq!(folder_for("data.bin"), None);
    }

    #[cfg(unix)]
    #[test]
    fn sorting_never_follows_a_link() {
        let root = tempfile::tempdir().unwrap();
        let downloads = root.path().join("Downloads");
        std::fs::create_dir(&downloads).unwrap();
        assert_eq!(sorted_dir(&downloads, "Images"), Some(downloads.join("Images")));
        assert!(downloads.join("Images").is_dir());
        assert_eq!(sorted_dir(&downloads, "Images"), Some(downloads.join("Images")));
        std::fs::create_dir(root.path().join("elsewhere")).unwrap();
        std::os::unix::fs::symlink(root.path().join("elsewhere"), downloads.join("Videos")).unwrap();
        assert_eq!(sorted_dir(&downloads, "Videos"), None);
        std::fs::write(downloads.join("Audio"), b"").unwrap();
        assert_eq!(sorted_dir(&downloads, "Audio"), None);
    }

    #[test]
    fn long_names_keep_extension() {
        let name = sanitize_filename(&format!("{}.pdf", "é".repeat(300)));
        assert!(name.len() <= MAX_NAME_BYTES && name.ends_with(".pdf"));
    }
}
