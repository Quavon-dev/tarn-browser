# Shared core

Status: `crates/browser-core` exists and every window shares one `Core` (see §3). The macOS and Windows
shells use GPUI, and Linux, iOS and Android get a Native Shell (NSH) shell (plan
`docs/nsh-plan.md`). All shells should run the same browser logic. This document fixes the interface between that logic
and each platform's web view, and when the logic moves out of `crates/app`.

## 1. Contract: the web view

Every shell's web view offers the same operations and reports the same events. The reference is
`engine::WebView` (`crates/engine/src/view.rs`) and `engine::Event`
(`crates/engine/src/events.rs`). The NSH component `tarn.webview` follows the same contract.

**Operations:**
- create for a profile's data store at a Shield level;
- adopt a popup and move to another host (`adopt`, `rehost`);
- `load`, `back`, `forward`, `reload`, `reload_from_origin`, `stop`;
- frame, visibility, focus and raise: on NSH, layout and focus come from the shell;
- `url`, `title`, `zoom` and `set_zoom`, `is_muted` and `set_muted`;
- `find`, `print`, `snapshot_png`, `pdf`, `picture_in_picture`;
- isolated-world reads for the shell: `icon_links`, `fetch_icon` (NSH components: `readIconLinks`
  and `fetchIcon` with the requesting `page`, `docs/mobile-plan.md` §2), `json_text`; Android
  has no isolated world and runs those scripts in the page world, so a shell must not send it
  `readIconLinks` or `fetchIcon`: Android reports the page's icon with the `pageIcon` event;
- `interaction_state` and `restore_interaction_state`, where the engine has them;
- `filtered`: whether the view applies Shield's rules.

**Events:**
- `Committed`, `Finished`, `TitleChanged`, `UrlChanged`, `Failed`;
- `Loading(bool)` on macOS and Windows, `started` and `stopped` on Linux, iOS and Android: a
  main-frame load began before it commits, or ended without `Finished` or `Failed` (a stop, a
  cancel, a load that turned into a download);
- `Blocked`, `PopupBlocked`, `Popup`, `ShieldLevelChanged`;
- `Prompt`, `PeekLink`, `ExternalScheme`, `ContentProcessTerminated`;
- `OpenTab`: Chromium views only, for a link the user opens in a new tab (Ctrl- or middle-click,
  the page menu); WebKit views report it as a `Popup`;
- `FullScreen`: Chromium views on Windows only, whose element full screen fills the browser's
  view; WebKit and the other engines open their own full-screen window;
- `DownloadFinished`, `DownloadFailed`, `DownloadsActive`;
- `pressed`: Linux, iOS and Android, a pointer or touch press on the page, which still reaches the
  page, so a split pane takes the focus. macOS and Windows see the same press through the shell's
  mouse monitor (`engine::menu::EventMonitor::on_mouse_down`);
- `history`: iOS and Android, whether the view can go back and forward after a same-document
  navigation (`pushState`, a fragment), which commits nothing: what `UrlChanged` carries in
  `can_go_back` and `can_go_forward` on macOS and Windows. Linux sends `finished` with
  `canGoBack` and `canGoForward` when the URL changes without a load, which covers the same case.

**Rules for every implementation:**
- **Decisions inside the web view:** policy decisions are made inside the web view, synchronously,
  by calling `engine::nav_policy`. Events only report them.
- **Page isolation:** no page-to-shell bridge, and shell scripts run in an isolated world.
- **Additions:** a new operation or event is added to every implementation, or documented as a gap
  for the platforms that lack it.

**Tarn extensions** (`engine::tarn_ext`, runtime in `browser_core::tarn_ext`). Every implementation
adds:
- `ExtensionView::set_extension_gate(Gate)`: the engine asks the gate synchronously for each frame
  navigation (`FrameNav`: frame URL, top URL, main frame or not) which of a world's scripts may run
  there, and injects nothing else. The shell's gate calls `tarn_ext::frame_decision`. Where the
  shell runs on its own thread (NSH: Linux, iOS, Android), the engine asks it through a round trip
  instead and holds each document until the answer arrives; nothing runs before it.
- `add_extension_scripts(Rc<WorldScripts>)` / `remove_extension_scripts(&World)`: the bootstrap
  first, then each approved `Script`'s CSS and JS at its `RunAt`, main frame or all frames, in the
  isolated world `World::name()` (`tarn-ext-<install id>`), never the page's main world. All
  frames include cross-site frames in another process; an engine that sees one as a target of its
  own (Chromium) attaches it, with the same scripts, before its first document.
- `post_extension_message(&World, Option<FrameId>, &str)`: calls `__tarnExtReceive(json)` in that
  world of one frame, or of every frame where the world runs (`None`).
- Event `ExtensionMessage { world, frame, origin, json }`: the world called `__tarnExtPost(json)`.
  The engine defines `__tarnExtPost` only in that world (or in an extension page's main world) and
  fills `world`, `frame` and, for a subframe, `origin` (the frame document's web origin) from where
  the call arrived, never from `json`; the router drops a subframe's message without `origin`.
  Messages over `MAX_MESSAGE_BYTES` may be dropped by the engine.
- A `tarn-extension://` handler per profile (`Resources`, from `tarn_ext::Resolver::handler`),
  registered as a standard, secure scheme, serving only what the handler returns, with its MIME
  type and `Content-Security-Policy` header, and only to the extension page view of that origin.
  Web pages cannot load the scheme.
- Extension page views (`PageKind::Background` hidden and owned by the shell, `PageKind::Popup`)
  that load `tarn-extension://` URLs with `Package::bootstrap(true)` and `__tarnExtPost` in their
  main world, with the profile's Shield, private-network and HTTPS rules. Their messages arrive as
  `ExtensionMessage` with the install's world and `FrameId::MAIN`.

Engines that can only register scripts ahead of a document (WebKit's user scripts, Chromium's
new-document scripts) carry the gate's answers in the scripts themselves: `tarn_ext::Decisions`
records the answer per frame key (`frame_key`: main frame and URL without fragment, or sub frame with the top page's origin and its URL without fragment) when the
gate is asked, and `tarn_ext::injections` builds the world's scripts from it, a gate script first
and then each content script file guarded by it, so a document whose answer is missing runs
nothing. A view of the profile without a gate shares those scripts, so it records an empty answer
for each of its documents and holds it until it gets a gate or closes; a tab's answer for the same
frame key is empty meanwhile. An engine's own setup messages start with `HELLO` and never reach the shell. Extension
page views are made from a `PageSpec`; on macOS and Windows `engine::ExtensionPage` is that view.
Android WebView differs in three ways: its gate is asked at document start through a round trip to
the shell thread (about 40 ms), so `document_start` scripts arrive slightly late; documents of the
custom scheme get an opaque origin, so extension pages are served with their CSP's `'self'`
replaced by `tarn-extension:` and requests to any other `tarn-extension://` origin get 403; and
WebView takes no origin rule for the custom scheme, so the page bridge is registered for every
origin, only on the dedicated extension page view, which keeps its top-level navigations on its
own origin and accepts messages only from its top document there.

Shells share the runtime around the router: `tarn_ext::runtime::Runtime` keeps each profile's
checked installs (`replace`, `insert`, `retain`), the session's grants (`gate`, `answer`,
`tab_gone`, `tick`, `prompt_dropped` for a prompt the shell drops unshown), the rule lists (`rule_lists`, `update_rules`), the router (`handle`,
`clicked`) and the shell's page views, and returns `runtime::Effect`s for the shell to carry out.
Every shell reads the "every site" switch with `extensions::every_site_setting`, and installs go
through `extensions::install` (`slot`, `vet`, `carry_over`, the signed catalog).

The shim (`tarn_ext_shim.js`) defines `chrome.*` and `browser.*` and talks only through these two
functions: it sends `{"id": <n or null>, "call": "<api>", "args": [...]}` and receives
`{"id": <n>, "result": ...}`, `{"id": <n>, "error": "..."}` or an event
(`{"event": "runtime.onMessage" | "storage.onChanged" | "action.onClicked", ...}`). Every decision
on a message (who may receive it, grants, quotas, rate, `tabs.*` filtering) is made by
`tarn_ext::router::Router`; the shell carries out the runtime's `Effect`s. Network rules compile per
install with `tarn_ext::rule_list` into one content rule list that WebKit engines load and other
engines evaluate per request with `engine::content_rules` (`ResourceKind::Document` for a main
frame, `Subdocument` for a sub frame, so `load-context` holds); neither is attached to a
Lockdown-level view. Shells build the runtime's `Policy` with `Core::extension_policy` and call
`Runtime::level_changed` after a profile's Shield level changes, which closes its extension pages
when it moved into Lockdown.

## 2. Where new logic goes

Logic without UI belongs in a shared crate, not in `crates/app`:
- `store` for data;
- `shield` for blocking;
- `engine` for platform-neutral engine logic: `nav_policy`, download names, `cookies`,
  `content_rules` and site icons (`favicon`: which URLs to try, decoding, and `favicon::Tracker`, the
  NSH shells' icons in memory with their fetch, retry and refresh bookkeeping).

Page scripts the shell runs in the web view's isolated world (Find in Tabs, page text, icons, the
element picker, per-site styles and forced dark mode) are in `engine::page_scripts`, the reader's in
`engine::reader`, and a platform-neutral QR encoder in `engine::qr` (contract v3 in
`docs/mobile-plan.md` §2); clean links are `shield::url_clean::clean_link`, space schedules
`browser_core::schedule` and Find in Tabs rows `browser_core::find`.

The engine choice already follows this: engine resolution is in `store` (`WebEngine::resolve`),
and `engine::cookies` and `shield::matcher` are in shared crates.

## 3. `browser-core`

**What it holds:** a new crate with the browser model and no UI types. That means the app-level state
of profiles, data stores, spaces, tabs, lifecycle, engine resolution, Shield levels, prompts,
downloads, history and suggestions, filter lists and the archive timer. Shells reach the web view only
through the contract in §1.

**Why:** this state used to live in the per-window `Browser`. Keeping the app-level state in
`browser-core` instead of inside `crates/app` lets every shell share it.

**Built so far:** `browser_core::Core` with the stores, profile runtimes and data stores, the
compiled rule lists, domain sets, YouTube script, Chromium's matcher and Shield profile and the
default web engine, and `browser_core::site` with levels, overrides, site rules and the gate's
verdict.

**Platform-neutral:** `browser-core` builds for macOS, Linux, iOS and
Android. `Core<D>` and `ProfileRuntime<D>` are generic over `browser_core::WebsiteData` (open a
persistent or ephemeral store, empty a persistent one, import cookies, and the engine's compiled
rule-list type); `engine::DataStore` implements it on macOS and the GPUI shell names
`Core<DataStore>`. Each NSH shell implements it for its own web view's data store. Its ephemeral
store holds the private space and Lockdown data and must never write website data to disk (threat
model S8); each implementation needs a test that checks this on its platform. `data_dir(base)`
is `TARN_DATA_DIR`, else the host's `base`, else `~/Library/Application Support/Tarn` on macOS and
`$XDG_DATA_HOME/tarn` or `~/.local/share/tarn` on Linux; iOS and Android have no default, so their
host passes `base`. `container` and `staleness` are macOS-only.

**Window model:** `browser_core::tabs::Model` holds one window's spaces, the current
space's tabs, folders and groups, the open folders, the active tab, the tab lifecycle and the
reopen stack, with the sidebar's tab order, number-key slots, cycling, the tab a close selects,
the tab to show when none is active, new tabs, pin, folder, group and move-to-space commands,
which space a window reopens on and the private space. `browser_core::closed` has the reopen stack
and the archive of unused tabs; `Core::{level, has_override, web_engine}` resolve per tab. The
GPUI `Browser` keeps the views and what only a shell has (pages, prompts, find, favicons,
rendering, gestures, menus, text inputs, windows) and calls the model; it passes in what the
model cannot see, such as whether another window shows a tab and a live view's current URL.

**Shared by every window:** the GPUI shell opens one `Core` per app and every window
holds it (`crates/app/src/shared.rs`, which adds the shell's switches, filter-list status and
extensions). Fields change through `Cell`/`RefCell`, so a borrow lasts one statement and never
spans a call into another window. Extension discovery, the filter task and the archive timer run
once per app on GPUI's executor and reach each window through `shared::each`; moving that loop
into `browser-core` waits for a shell-neutral executor. Window placement is in `browser_core::windows`
(frame onto a screen, the place a saved window reopens on, what it saves), with the `windows`
table in `store`.

**NSH shells:** the Linux, iOS and Android shells share `browser_core::command` (input
parsing, `engine_key`, the address label: `address`, which the macOS toolbar uses, and its text form `display_host`) and
`browser_core::prompts` (prompt events, which permission answers are remembered); like macOS they reload
a crashed page once and then stop until a page finishes loading; their web views share `engine::view_policy`
(site levels with `"*"`, bursts, the NSH zoom range `MIN_PAGE_ZOOM`..`MAX_PAGE_ZOOM`, 0.5 to 3.0; the macOS app keeps its own `engine::MIN_ZOOM` of 0.3) and the filter-list groups in `shield::groups`.

The NSH shells for Linux, iOS and Android (`docs/nsh-plan.md` P2–P4) sit on `browser-core`.
