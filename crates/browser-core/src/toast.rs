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

#[cfg(test)]
mod tests {
    use super::*;

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
