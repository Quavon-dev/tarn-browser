/// How a toast reads, which sets its icon and tint on every client.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    #[default]
    Info,
    Success,
    Warning,
}

/// The toast after saving a page to the Reading List.
pub fn reading_list(saved: bool) -> (Tone, &'static str) {
    if saved { (Tone::Success, "Saved to Reading List") } else { (Tone::Warning, "Could not save to Reading List") }
}

/// The toast after closing tabs: a whole group, or `count` tabs.
pub fn closed(group: bool, count: usize) -> String {
    match (group, count) {
        (true, _) => "Group closed".to_owned(),
        (false, 1) => "Tab closed".to_owned(),
        (false, n) => format!("{n} tabs closed"),
    }
}

/// The toast for a download that ended: `Ok((quarantined, executable))` or the failure message.
/// `mark` names what flags a downloaded file on the platform.
pub fn download(name: &str, result: Result<(bool, bool), &str>, mark: &str) -> (Tone, String) {
    match result {
        Ok((false, _)) => (Tone::Warning, format!("Do not open it unless you trust it: {mark} could not be set on “{name}”.")),
        Ok((true, true)) => (Tone::Warning, format!("This file type can run code; open it only if you trust the site: “{name}”.")),
        Ok((true, false)) => (Tone::Success, format!("Downloaded “{name}”")),
        Err(message) => (Tone::Warning, format!("Download failed: {message}")),
    }
}

/// Adds a notice to those waiting, the first of which shows: a warning waits until dismissed,
/// while a harmless notice gives way to the next one.
pub fn push_notice<T>(queue: &mut Vec<T>, notice: T, warns: impl Fn(&T) -> bool) {
    queue.retain(&warns);
    queue.push(notice);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_harmless_notice_never_replaces_a_warning() {
        let mut queue = Vec::new();
        let warns = |n: &&str| n.starts_with('!');
        push_notice(&mut queue, "saved a", warns);
        push_notice(&mut queue, "saved b", warns);
        assert_eq!(queue, ["saved b"]);
        push_notice(&mut queue, "!unquarantined", warns);
        push_notice(&mut queue, "saved c", warns);
        assert_eq!(queue, ["!unquarantined", "saved c"]);
        push_notice(&mut queue, "!failed", warns);
        assert_eq!(queue, ["!unquarantined", "!failed"]);
    }

    #[test]
    fn a_download_warns_when_unmarked_runnable_or_failed() {
        assert_eq!(download("a.pdf", Ok((true, false)), "macOS quarantine"), (Tone::Success, "Downloaded “a.pdf”".into()));
        assert_eq!(download("a.pdf", Ok((false, false)), "macOS quarantine").0, Tone::Warning);
        assert!(download("a.pdf", Ok((false, false)), "the mark of the web").1.contains("the mark of the web could not be set"));
        assert!(download("a.app", Ok((true, true)), "macOS quarantine").1.contains("can run code"));
        // The warning comes before the name, so a long name cannot push it past the toast's last line.
        let long = "x".repeat(120);
        for warning in [download(&long, Ok((false, false)), "macOS quarantine").1, download(&long, Ok((true, true)), "macOS quarantine").1] {
            assert!(warning.find("trust").unwrap() < warning.find(&long).unwrap(), "{warning}");
        }
        assert_eq!(download("a.pdf", Err("disk full"), "macOS quarantine"), (Tone::Warning, "Download failed: disk full".into()));
    }

    #[test]
    fn closing_names_the_group_or_counts_the_tabs() {
        assert_eq!((closed(true, 3), closed(false, 1), closed(false, 2)), ("Group closed".into(), "Tab closed".into(), "2 tabs closed".into()));
    }

    #[test]
    fn a_failed_save_warns() {
        assert_eq!(reading_list(true), (Tone::Success, "Saved to Reading List"));
        assert_eq!(reading_list(false), (Tone::Warning, "Could not save to Reading List"));
    }
}
