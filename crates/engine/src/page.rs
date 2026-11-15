//! Limits and wording both engines apply to what a page does, over `view_policy`.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::Instant;

use shield::Category;
use url::Url;

pub(crate) use crate::view_policy::{DOWNLOAD_BURST, POPUP_BURST, PROMPT_BURST};

/// True if another event fits into the burst limit; records it when it does.
pub(crate) fn within_burst(recent: &RefCell<VecDeque<Instant>>, max: usize) -> bool {
    crate::view_policy::admit(&mut recent.borrow_mut(), max, Instant::now())
}

pub(crate) fn serialized_origin(url: &Url) -> Option<String> {
    crate::view_policy::origin(url)
}

pub(crate) fn bounded(title: &str) -> String {
    crate::view_policy::title(title)
}

pub(crate) fn blocked_message(category: Category, url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    match category {
        Category::Malware => format!("{host} is listed as malicious."),
        Category::Adult => format!("{host} is listed as adult content."),
        Category::Popups => format!("Stopped a redirect to {host}, a known pop-up ad site."),
        Category::Ads | Category::Trackers | Category::Annoyances => format!("{host} is blocked."),
    }
}

/// Whether `host` is the domain `record` or below it.
pub(crate) fn site_matches(host: &str, record: &str) -> bool {
    let record = record.to_ascii_lowercase();
    !record.is_empty() && (host == record || host.ends_with(&format!(".{record}")))
}
