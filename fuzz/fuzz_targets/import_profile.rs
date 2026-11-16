#![no_main]

use std::path::PathBuf;
use std::sync::OnceLock;

use import::{Browser, Source};
use libfuzzer_sys::fuzz_target;

fn dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("tarn-fuzz-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temporary directory");
        dir
    })
}

fuzz_target!(|data: &[u8]| {
    let Some((&which, rest)) = data.split_first() else { return };
    let (browser, file) = match which % 5 {
        0 => (Browser::Chrome, "Bookmarks"),
        1 => (Browser::Chrome, "History"),
        2 => (Browser::Firefox, "places.sqlite"),
        3 => (Browser::Firefox, "places.sqlite"),
        _ => (Browser::Firefox, "cookies.sqlite"),
    };
    let dir = dir();
    let path = dir.join(file);
    std::fs::write(&path, rest).expect("the source file is written");
    let source = Source { browser, label: String::new(), dir: dir.clone() };
    match which % 5 {
        0 | 3 => {
            let _ = source.bookmarks();
        }
        1 | 2 => {
            let _ = source.history();
        }
        _ => {
            let _ = source.cookies();
        }
    }
    let _ = std::fs::remove_file(&path);
});
