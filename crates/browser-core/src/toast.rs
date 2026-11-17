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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_save_warns() {
        assert_eq!(reading_list(true), (Tone::Success, "Saved to Reading List"));
        assert_eq!(reading_list(false), (Tone::Warning, "Could not save to Reading List"));
    }
}
