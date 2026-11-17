//! Limits and wording both engines apply to what a page does, over `view_policy`.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::Instant;

use url::Url;

pub(crate) use crate::nav_policy::blocked_message;
pub(crate) use crate::view_policy::{DOWNLOAD_BURST, POPUP_BURST, PROMPT_BURST, site_matches};

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
