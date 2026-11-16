use std::path::PathBuf;
use std::time::SystemTime;

use shield::Category;
use url::Url;

#[cfg(windows)]
use crate::host_windows::Popup;
#[cfg(target_os = "macos")]
use crate::webview::Popup;

/// A protection Tarn applied to a page load itself, counted on the profile's Shield card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protection {
    /// A plain-HTTP page loaded over HTTPS by the Strict rules.
    HttpsUpgrade,
    /// A page or window refused because a public page sent it to the local network.
    LocalNetwork,
}

#[derive(Debug)]
pub enum Event {
    /// The main frame committed to a new URL.
    Committed {
        url: Url,
        title: String,
        can_go_back: bool,
        can_go_forward: bool,
    },
    Finished {
        url: Url,
        title: String,
        can_go_back: bool,
        can_go_forward: bool,
    },
    TitleChanged(String),
    /// The main frame's URL changed without a load, as with `history.pushState`.
    UrlChanged {
        url: Url,
        can_go_back: bool,
        can_go_forward: bool,
    },
    Failed {
        url: Option<Url>,
        code: isize,
        message: String,
    },
    /// A navigation was stopped by Shield; the current page is unchanged. With a category, a
    /// domain set named the host: `Category::warns` ones get the warning page, others a notice.
    Blocked {
        url: Url,
        message: String,
        category: Option<Category>,
    },
    /// A pop-up the site's rule or a domain set refused; `None` when WebKit gave no URL.
    PopupBlocked(Option<Url>),
    /// A pop-up a click asked for that a Chromium tab does not open itself; the notice offers it.
    PopupOffered(Url),
    /// A user-gesture popup or `target=_blank` link, already checked by `nav_policy`; adopt it
    /// with `WebView::adopt` or drop it to discard.
    #[cfg(any(target_os = "macos", windows))]
    Popup(Popup),
    /// A load the shell started needs a different Shield level than this view was built with;
    /// the shell must load it in a view built for that level. Page-initiated navigations are
    /// never reported this way: they continue in place with the new site's JavaScript setting.
    ShieldLevelChanged(Url),
    /// Windows: a page-started main-frame navigation crossed the Lockdown boundary, which a
    /// Chromium view cannot do in place; the shell rebuilds the view for the URL's level and loads
    /// it as the page's link.
    RebuildForLink(Url),
    /// A page-started main-frame navigation in a Chromium view to a site at `lockdown`, cancelled
    /// before any request; only the user may load it, in WebKit.
    WebEngineRequired(Url),
    /// A main-frame navigation in a Chromium view, cancelled because the bundled Chromium is too
    /// old (`chromium::close_in`); the shell closes its Chromium views.
    WebEngineOutdated,
    /// A dialog, permission or credentials request; dropping it answers "no".
    #[cfg(any(target_os = "macos", windows))]
    Prompt(crate::Prompt),
    /// A ⇧-click on a link, already checked by `nav_policy`, and the origin it was judged from; the
    /// page stays where it is.
    PeekLink(Url, Option<Url>),
    /// A link to another app's scheme; the shell must ask before handing it off. `origin` is the
    /// frame that started it, `None` for the shell's own loads and opaque origins.
    ExternalScheme {
        url: Url,
        origin: Option<String>,
    },
    ContentProcessTerminated,
    /// With the upgraded page's host for an HTTPS upgrade; the shell names a refusal by the tab's page.
    Protected(Protection, Option<String>),
    /// Saved and never opened automatically. `quarantined` is false if the quarantine
    /// attribute could not be set.
    DownloadFinished {
        id: u64,
        url: Option<Url>,
        path: PathBuf,
        started: SystemTime,
        quarantined: bool,
    },
    /// `id` names a download that had started saving.
    DownloadFailed {
        id: Option<u64>,
        message: String,
    },
    /// A download saving or paused: sent when it starts, while it saves, and when it pauses or
    /// resumes. `total` is `None` while the size is unknown.
    DownloadProgress {
        id: u64,
        path: PathBuf,
        done: u64,
        total: Option<u64>,
        paused: bool,
    },
    /// A download that had started saving and the user declined; its saved part is removed.
    DownloadCancelled(u64),
    /// How many downloads of this data store are saving or paused; sent whenever it may change.
    DownloadsActive(usize),
}
