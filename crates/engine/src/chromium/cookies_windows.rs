//! On Windows Chromium's cookie jar is each profile's own; there is no WebKit store to bridge.

use std::cell::Cell;
use std::rc::Rc;

use cef::{Callback, ReturnValue};
use url::Url;

pub(crate) fn hold_for(_url: &str) -> Option<String> {
    None
}

pub(crate) fn hold(_site: &str, _callback: Option<&mut Callback>) -> ReturnValue {
    ReturnValue::CONTINUE
}

pub(crate) fn bridge_navigation(_url: &Url, _tab: u64) {}

pub(crate) fn changed_in_chromium() {}

pub(crate) fn finish() -> Rc<Cell<bool>> {
    Rc::new(Cell::new(true))
}
