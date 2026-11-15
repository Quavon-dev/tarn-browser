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
- isolated-world reads for the shell: `icon_links`, `fetch_icon`, `json_text`;
- `interaction_state` and `restore_interaction_state`, where the engine has them;
- `filtered`: whether the view applies Shield's rules.

**Events:**
- `Committed`, `Finished`, `TitleChanged`, `UrlChanged`, `Failed`;
- `Blocked`, `PopupBlocked`, `Popup`, `ShieldLevelChanged`;
- `Prompt`, `PeekLink`, `ExternalScheme`, `ContentProcessTerminated`;
- `DownloadFinished`, `DownloadFailed`, `DownloadsActive`.

**Rules for every implementation:**
- **Decisions inside the web view:** policy decisions are made inside the web view, synchronously,
  by calling `engine::nav_policy`. Events only report them.
- **Page isolation:** no page-to-shell bridge, and shell scripts run in an isolated world.
- **Additions:** a new operation or event is added to every implementation, or documented as a gap
  for the platforms that lack it.

## 2. Where new logic goes

Logic without UI belongs in a shared crate, not in `crates/app`:
- `store` for data;
- `shield` for blocking;
- `engine` for platform-neutral engine logic: `nav_policy`, download names, `cookies` and
  `content_rules`.

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
