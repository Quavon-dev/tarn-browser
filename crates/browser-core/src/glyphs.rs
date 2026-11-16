//! Glyphs (icon file names in `crates/ui/icons`) for things every client draws the same way:
//! page prompts, by the kind names the Native Shell clients use (`"camera"`, `"location"`, …;
//! the Mac maps its `PromptKind`), and downloaded files.

/// The prompt's glyph.
pub fn glyph(kind: &str) -> &'static str {
    match kind {
        "camera" | "camera and microphone" | "camera+microphone" => "camera",
        "microphone" => "mic",
        "location" | "geolocation" => "map-pin",
        "notifications" => "bell",
        "credentials" => "key-round",
        "extension" => "shield",
        "download" | "malicious download" => "triangle-alert",
        "external" => "external-link",
        "confirm" => "triangle-alert",
        _ => "info",
    }
}

/// The glyph for a downloaded file, by its name's extension.
pub fn file_glyph(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "zip" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar" | "tar" | "dmg" | "pkg" | "deb" | "rpm" | "msi" | "exe" | "appimage" | "apk" | "ipa" => {
            "file-archive"
        }
        "pdf" | "txt" | "md" | "doc" | "docx" | "odt" | "rtf" | "csv" | "json" | "xml" | "html" | "epub" => "file-text",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "heic" | "avif" | "bmp" | "tiff" => "file-image",
        "mp4" | "mov" | "mkv" | "webm" | "avi" | "m4v" | "mp3" | "m4a" | "wav" | "flac" | "ogg" => "file-video",
        _ => "file",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_and_files_get_a_glyph() {
        assert_eq!((glyph("location"), glyph("camera"), glyph("alert")), ("map-pin", "camera", "info"));
        assert_eq!(glyph("confirm"), "triangle-alert");
        assert_eq!(file_glyph("tarn-1.4.DMG"), "file-archive");
        assert_eq!(file_glyph("threat-model.pdf"), "file-text");
        assert_eq!(file_glyph("photo.HEIC"), "file-image");
        assert_eq!(file_glyph("README"), "file");
    }
}
