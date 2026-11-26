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
    /// The user chose Ask Assistant about Selection (`None`) or a command of the selection menu
    /// `WebView::set_assistant_menu` set; the shell reads the selection with `selected_text`.
    AssistantSelection(Option<String>),
    /// The page started (`true`) or stopped (`false`) loading, before it commits or after it ends;
    /// WebKit says only when a main-frame load starts, `Finished` and `Failed` end it.
    Loading(bool),
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
    /// The main frame's load failed on its certificate; nothing of the site was shown. `chain`
    /// is the server's certificates, leaf first, bounded by `cert::bounded_chain`.
    CertificateError {
        url: Url,
        problem: crate::cert::Problem,
        chain: Vec<Vec<u8>>,
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
    /// The user's own load needs a different Shield level than this view was built with; the
    /// shell must load it in a view built for that level. Page-initiated navigations are never
    /// reported this way: they continue in place with the new site's JavaScript setting.
    ShieldLevelChanged(Url),
    /// A load that is the page's needs a view built for another level: on macOS a link load the
    /// shell made for a page, on Windows a page-started navigation across the Lockdown boundary.
    /// The shell rebuilds the view for the URL's level and loads it as the page's link.
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
    /// A Chromium link the user opened in a new tab (Ctrl- or middle-click, the page's menu),
    /// already checked by `nav_policy` as a pop-up, and the page it was opened from; `background`
    /// keeps the opener in front, and `split` puts the tab beside it in split view.
    OpenTab {
        url: Url,
        from: Option<Url>,
        background: bool,
        split: bool,
    },
    StoreInstall(String),
    /// A page-started navigation of an agent's tab to a site the agent may not use yet, cancelled
    /// before any request (`WebView::set_agent_gate`).
    AgentHeld(Url),
    /// A link to another app's scheme; the shell must ask before handing it off. `origin` is the
    /// frame that started it, `None` for the shell's own loads and opaque origins.
    ExternalScheme {
        url: Url,
        origin: Option<String>,
    },
    ContentProcessTerminated,
    LinkHovered(Option<String>),
    /// A page entered (`true`) or left full screen for an element (a video). A Chromium page on
    /// Windows fills the shell's window, which the shell shows alone, filling the screen; WebKit
    /// shows it in a window of its own (`WebView::owns_fullscreen_window`).
    FullScreen(bool),
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
    /// A Tarn extension's world, or its own page's main world, called `__tarnExtPost`; `world`,
    /// `frame` and `origin` (the subframe document's web origin) are where the call arrived, never
    /// read from `json`.
    ExtensionMessage {
        world: crate::tarn_ext::World,
        frame: crate::tarn_ext::FrameId,
        origin: Option<url::Url>,
        json: String,
    },
}
