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
- `load_file`: a file the user picked with Open File; `load` refuses `file:` URLs, and the gate
  lets a main-frame `file:` navigation through only for this load or the user's Back, Forward or
  Reload onto a file the view opened this way (`nav_policy::file_open`, `Source::FileOpen`). NSH components take it as the
  `load` prop with `file: true`;
- frame, visibility, focus and raise: on NSH, layout and focus come from the shell; Linux's
  `focus` command gives the page the keyboard (F6);
- `url`, `title`, `zoom` and `set_zoom`, `is_muted` and `set_muted` (NSH components: the `muted`
  prop, which the component keeps across a rebuild of its view; Linux has it, iOS and Android not
  yet);
- `set_dark`: the colour scheme pages see (`prefers-color-scheme`), set from the shell's theme
  when a view is made and whenever the theme changes, not from the system's appearance (macOS
  WebKit: the view's `NSAppearance`; Chromium: `Emulation.setEmulatedMedia`; NSH components: the
  `dark` prop, iOS the view's `overrideUserInterfaceStyle`, Android the theme of the context the
  view is made with; Linux: the app's NSH colour scheme, which WebKitGTK takes from GTK's settings
  for every view at once, left on the system's while the theme follows the system);
- `clear_find`: removes the highlight a `find` left, when the find bar closes or its text is
  emptied, and keeps the last match selected, as in Safari (on macOS and iOS WebKit hides its match
  marker with WKWebView's private `_hideFindUI`, called only where the view responds to it;
  Chromium stops finding and keeps the selection; Android's `clearMatches`, the only public way to
  end a find, drops it, `docs/mobile-plan.md`); `find` reports the match count and, where the
  engine says (Chromium), the highlighted match (NSH components: the `clearFind` command,
  WebKitGTK `search_finish` on Linux, and `current` in `found` on
  Android);
- `find`, `print`, `snapshot_png`, `pdf`, `picture_in_picture`; Linux has `print` (the GTK print
  dialog) and `pdf` with the absolute `path` the user picked in the save dialog, printed there by
  WebKitGTK and reported on the download route as a finished download; iOS and Android have
  neither yet (`docs/port-backlog.md`); Linux's `snapshot` command answers with a `snapshot` event carrying a PNG
  `data:` URI at most 752 px wide (the tab hover card), which iOS and Android do not take yet; NSH components: `screenshot` `{seq, name}`, which saves the visible page, or with
  `fullPage` or `x, y, width, height` (`browser_core::capture::area_args`) that area, as a new PNG
  under the sanitised `name` in Downloads (`browser_core::capture::save_new`), copies it to the
  clipboard (not on Android, `docs/mobile-plan.md`) and answers with the event `screenshot`
  `{seq, path}` or `{seq, message}`;
- `pick_rect`: the Zap picker (`page_scripts::pick_rect_script`) answering the picked element's
  rectangle in the visible page instead of a selector, for Capture Element
  (`browser_core::capture::element_area` cuts it to the page); empty when cancelled or under
  Lockdown. NSH components: `pick` with `rect: true`, answered by `picked` `{seq, url, rect}`.
  Capture Area draws its rectangle in the shell over a still snapshot of the page
  (`capture::drag_area`), with no script on the page, and so does the developer Measure tool
  (`browser_core::measure`, offered with `palette::State::developer`, where Web Inspector is);
- `insert_text`: Insert Snippet types a `store::Snippet`'s text into the page's focused text field
  (`page_scripts::insert_text_script`, isolated world, Android's page world, nothing under
  Lockdown) and answers whether it did (`browser_core::snippets::inserted`). Every shell remembers
  the tab and committed address the list opened on and types nothing once the tab shows another
  (`snippets::PAGE_CHANGED`). NSH components: `insertText` `{seq, text}`, answered by
  `textInserted` `{seq, inserted}`. Typing `;name` in a shell field expands there
  (`snippets::expand`); nothing on the page listens for keys. The profile's snippets are read once
  into `Core::snippets`, shared by every window, and forgotten by `Core::snippets_changed` after an
  edit, `reset_profile_data` or a deleted profile;
- `selected_text`, `media_state`, `media_command` and `snapshot_rect`: the selection, the first
  audible media element (read, and play, pause or seek), and a PNG of a rectangle of the visible
  page or of the whole document. The reads are isolated-world scripts from `engine::page_scripts`
  that answer empty under Lockdown; `browser_core::page_ops` checks every answer and command
  (`selection`, `MediaState::parse`, `media_command`, `snapshot_area`); components emit only
  `selection_answer` and `media_answer` of a script's answer. NSH components:
  `readSelection` (`selection`), `ping` (`pong`), `readMedia` (`mediaState`), `media` and `snapshotRect`
  (`snapshotRect`), `docs/mobile-plan.md` §2. `set_assistant_menu(Option<Vec<String>>)` (NSH prop
  `assistantMenu`, a list of command names or null; at most `view_policy::MAX_MENU_COMMANDS`) gives
  a selection's menu `view_policy::ASK_SELECTION` and an `ASSISTANT_COMMANDS` submenu with the
  names; null shows neither. A choice is `Event::AssistantSelection(command)`, after which the shell
  reads `selected_text`; NSH components read the selection themselves and emit
  `assistantSelection` `{url, text, command}` with `selection_answer` of it. Shells set the list
  from `tabs::Model::assistant_menu` for the page shown (WebKit's menu on macOS, Chromium's on
  macOS and Windows, WebKitGTK's `context-menu` on Linux, the edit menu of a `WKWebView` subclass
  on iOS, the text selection action mode of `PageView` on Android, where the floating toolbar has no
  submenus and lists the commands in its overflow). A full page is the document cut to
  `view_policy::SNAPSHOT_MAX_SIDE` and its height to `SNAPSHOT_MAX_PIXELS` at the device scale
  (`view_policy::snapshot_height`, also for rectangles) on macOS and Windows; Linux takes it only
  when the document fits those uncut (WebKitGTK cannot clip a full-document snapshot), answers
  none otherwise and drops any larger texture; iOS and
  Android draw only the visible page. The window's mini-player is `browser_core::media::Player`:
  shells ask `media_state` of a tab when the engine says it plays (Linux's `playing` event, the
  macOS and Windows background probe), when the user leaves it and after a button press, and
  again every `media::POLL` only while the shown media plays; its buttons send
  `media::Control::command` (`args` for NSH components). Its title is `text::bounded_title` of the
  page's media title or the tab's title on every client;
- isolated-world reads for the shell: `icon_links`, `fetch_icon` (NSH components: `readIconLinks`
  and `fetchIcon` with the requesting `page`, `docs/mobile-plan.md` §2), `json_text`; Android
  has no isolated world and runs those scripts in the page world, so a shell must not send it
  `readIconLinks` or `fetchIcon`, and Android shows letter tiles (`docs/mobile-plan.md`);
- `interaction_state` and `restore_interaction_state`, where the engine has them;
- Web Inspector availability, fixed when the view is created (macOS and Windows: `inspectable`
  at creation; NSH: the `inspectable` prop, which Linux applies outside Lockdown views only), and
  `clear_cache` on the profile's data store (NSH: the `clearCache` command on a view of that
  store). Linux's component implements `reloadFromOrigin`, `clearCache` and `readJson` (answered
  by `jsonText`); iOS (WebKit's `reloadFromOrigin`) and Android (one reload with
  `WebSettings.LOAD_NO_CACHE`, back to `LOAD_DEFAULT` when the page finishes, fails or stops) have
  `reloadFromOrigin`; they do not have the others yet;
- `filtered`: whether the view applies Shield's rules;
- start-of-document page-world scripts: `engine::page_scripts::ALWAYS_VISIBLE` in every view but
  Lockdown's, and `shield::youtube::script` where blocking is not paused and Block YouTube ads is
  on (macOS `DataStore::set_page_script`; NSH: the `youtube` prop, Linux so far, iOS and Android
  per `docs/port-backlog.md`).

**Events:**
- `Committed`, `Finished`, `TitleChanged`, `UrlChanged`, `Failed`;
- `Loading(bool)` on macOS and Windows, `started` and `stopped` on Linux, iOS and Android: a
  main-frame load began before it commits, or ended without `Finished` or `Failed` (a stop, a
  cancel, a load that turned into a download);
- `Blocked`, `PopupBlocked`, `Popup`, `ShieldLevelChanged`;
- `Prompt`, `PeekLink`, `ExternalScheme`, `ContentProcessTerminated`; `PeekLink` is `peek {url}`
  on Linux, sent only for the user's own Shift-click under the pop-up rules; iOS and Android have
  no Shift-click and send none;
- `OpenTab`: Chromium views only, for a link the user opens in a new tab (Ctrl- or middle-click,
  the page menu); WebKit views report it as a `Popup`. Open Link in Split View in a link's menu
  sets `split`, which puts the tab beside its opener (macOS WebKit also as `OpenTab`; NSH
  components as `popup` with `split: true`: Linux from WebKitGTK's link menu, iOS and Android from
  the long-press menu while the `splitLinks` prop is set); each takes the pop-up checks;
- `FullScreen`: Chromium views on Windows and WebKitGTK on Linux (`fullscreen`
  `{on, windowFullScreen}`, the latter whether the window was already in full screen), whose
  element full screen fills the browser's view or, on WebKitGTK, puts the shell's own window in
  full screen; the shell hides itself while `browser_core::fullscreen` lets the page fill the
  window. WebKit on macOS and iOS and Android WebView open their own full-screen window;
- `DownloadFinished`, `DownloadFailed`, `DownloadsActive`;
- `pressed`: Linux, iOS and Android, a pointer or touch press on the page, which still reaches the
  page, so a split pane takes the focus. macOS and Windows see the same press through the shell's
  mouse monitor (`engine::menu::EventMonitor::on_mouse_down`);
- `LinkHovered`: the link under the pointer as the page wrote it, `None` once the pointer leaves it,
  shown through `browser_core::link_status` at the page's bottom-left corner (macOS WebKit:
  WKUIDelegate's private `_webView:mouseDidMoveOverElement:withFlags:userInfo:`; Chromium:
  `on_status_message`; Linux: `hover {url}` from WebKitGTK's `mouse-target-changed`). iOS and
  Android send none (`docs/mobile-plan.md`);
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

Store install link: every implementation cancels a main-frame navigation the user started
(`nav_policy::store_link_asks`: a shell load that is not a link or pop-up the shell loads for a
page, or a link click with a user gesture)
whose URL `tarn_ext::store_install` maps to a catalog id (exactly
`https://extensions.tarnbrowser.com/install/<id>`, with or without a final `/`), before any
request, and reports `Event::StoreInstall(id)`; NSH engines emit `tarn_ext::STORE_EVENT`
(`storeInstall`, `{id}`). Redirects, subframes and script navigations without a gesture load
normally. WebKit on macOS reads the gesture from the navigation action's private
`_isUserInitiated` and redirects from `_isRedirect`; without them only shell loads count. iOS uses
only public signals: a shell load or a main-frame `linkActivated` navigation, script-started ones
and redirects after a link click included. The shell takes at most one store link per `install::STORE_LINK_INTERVAL` (10 s,
`Core::admit_store_link`) and ignores the others without a sheet. It makes no request on the
link: `extensions::install::store_step` (with `Core::store_link_allowed` for the tab and the
catalog copy the shell already holds) returns the entry, which the shell keeps in an
`install::StoreSheet` and Settings › Extensions shows with `store_sheet`, or the notice the shell
shows on the tab (no fresh catalog: load it in Settings › Extensions). A new store link replaces
the sheet under a new `StoreToken` (sequence number, id and SHA-256); closing the Extensions pane
or Settings drops it. The Install button carries the token of the sheet it was drawn for, and
`StoreSheet::take` hands the offer over only for that token and only `STORE_CONFIRM_DELAY` (700 ms)
after the sheet appeared (on macOS and Windows also only while `Browser::layout_stable`); then
`store_confirm` checks everything again (and the same entry), then `Install::Official(entry)`; when the unpacked package's source is that entry
(`store_confirmed`) the shell confirms it without a second prompt. Settings › Extensions links
`extensions::STORE_URL` on every client.

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
`browser_core::schedule`, Find in Tabs rows `browser_core::find`, the Suggestions switch and the
tab sleeping choices (keys, options, defaults, the idle limit under memory pressure)
`browser_core::settings`, and bookmarks (Bookmark Tab, Bookmark All Tabs, New Folder, Save for
Later, moves, recent and search) `browser_core::bookmarks`.

The engine choice already follows this: engine resolution is in `store` (`WebEngine::resolve`),
and `engine::cookies` and `shield::matcher` are in shared crates.

The AI providers (`ai::ClaudeApi`, `ai::Ollama`) send through `ai::Transport`: a POST with headers
and a JSON body whose streamed answer fails past a byte cap or once the cancel flag is set, with no
redirects. Before anything is sent, `ai::transport::allowed` admits only `https://api.anthropic.com`
and plain HTTP to `127.0.0.1` (Ollama). macOS, Windows, Linux and iOS use `ai::UreqTransport`
(rustls with the platform's certificate verifier). Android uses
`tarn_mobile_android::HostTransport`, which streams through the Java host's `Fetch.post`: that
keeps the host's TLS key policy (`StrongKeys`) and proxy hooks, where rustls's platform verifier
would need its Kotlin component bundled into the APK and initialised over JNI. `Fetch.post` accepts
HTTPS only, so Ollama is not reachable on Android.

Local models implement `ai::LocalModel`: `availability()` (`Available`, `NotSupported(reason)` or
`NeedsSetup(reason)`) and `respond`, which takes a `local::Checked` token only `ai::local::stream`
can make, so `stream` is the one way to run them: it asks the model only when it is available and
otherwise fails with `AiError::Unavailable` and that reason; nothing falls back to a cloud provider.
Both may block (Ollama asks its server), so shells call `stream` off the UI thread. `ai::local::route(choice, availability, on_device_task)` is the assistant's rule for
who answers: a local choice is answered locally or not at all, and a selection command or summary
stays on the device whenever a local model is available, even with the cloud chosen.
`ai::local::answer(chosen, on_device, on_device_task, …)` applies it for every client and names the
model that answers before it streams (`assistant::Event::AnsweredBy`, shown as "Answered by …");
`assistant::on_device_task` makes every command an on-device task. The cloud is the platform's
on-device model's only alternative when the cloud was chosen; Ollama, a local choice, keeps its
own model. Backends:
`engine::foundation_models::FoundationModel` on macOS and iOS (Apple's on-device model through the
Swift shim `crates/engine/src/foundation_models.swift`, built by `crates/engine/build.rs` with the
`foundation-models` feature, which needs the macOS or iOS 26 SDK; `app/foundation-models`,
`tarn-mobile/foundation-models` and `tarn-mobile-ios/foundation-models` forward it), and `ai::Ollama` on loopback, available once Ollama answers `/api/show` for a model that runs on
this machine: a `-cloud` or `:cloud` name or a reported remote host is refused (macOS, Windows,
Linux). Windows and Android have no
on-device model (`docs/windows-plan.md` §8, `docs/mobile-plan.md`).

The Claude API token lives behind `ai::Secrets` (`get`, `set`, `delete` by service and account;
`ai::secrets::{store_token, load_token, delete_token}`), never in a file or the database.
`ai::secrets::Keyring` is the Keychain on macOS and iOS, the Credential Manager on Windows and the
Secret Service over D-Bus on Linux (`oo7`, as `import` already uses). Android uses
`tarn_mobile_android::Keystore`: the host's `Secrets.java` seals each value with AES-GCM under a key
generated in the Android Keystore and keeps only the sealed bytes in private preferences, which the
app excludes from backups.

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
which space a window reopens on and the private space. Where a new tab goes is one rule,
`Model::insert_index` (`Model::open_tab` with `Opened::Typed` or `Opened::Link(opener)`): a typed or new
tab on top of the unpinned tabs, a tab a link or pop-up opened after its opener and the tabs opened
from it before, in opening order, in the opener's group; the openers are kept in memory only.
`browser_core::undo::Stack` (`Model::undo`) is the window's Undo: `Model::push_closed` puts a close on
the reopen stack and the undo stack, `sidebar::drop_tab` and `sidebar::clean_up` add the move or Clean
Up they made, and `Model::pop_undo` gives the newest step still undoable; each client runs a `Step`
(reopen that entry, `cleanup::undo`, `undo::Move::revert`) for ⌘Z or Ctrl+Z that no text field or
page took and for a toast's Undo, which first drops the step. Nothing of a private space or burner
tab is kept (threat model S8).
`browser_core::closed` has the reopen stack
and the archive of unused tabs with its Settings › Tabs choice (`ARCHIVE_AFTER`, `archive_after`,
`ARCHIVE_INTERVAL`); `browser_core::cleanup` is Clean Up (`plan`, `apply`, `cleanable`, `summary`; `cleanup::ai::request` and `sort_from` build an assistant grouping request from loose tabs' ids, hosts and titles, never in a private space, burner tab or Lockdown, and turn a checked proposal into the same undoable `Sort`); `Core::{level, has_override, web_engine}` resolve per tab. The
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
`browser_core::prompts` (prompt events, which permission answers are remembered and at which Shield level,
and the `answer` with `silence` that blocks a page's requests until its next navigation, `docs/mobile-plan.md` §2); like macOS they reload
a crashed page once and then stop until a page finishes loading; their web views share `engine::view_policy`
(site levels with `"*"`, bursts, the NSH zoom range `MIN_PAGE_ZOOM`..`MAX_PAGE_ZOOM`, 0.5 to 3.0; the macOS app keeps its own `engine::MIN_ZOOM` of 0.3) and the filter-list groups in `shield::groups`.
The tab, motion and gesture switches and the accent choice are stored under the macOS keys through
`browser_core::prefs`.

The NSH shells for Linux, iOS and Android (`docs/nsh-plan.md` P2–P4) sit on `browser-core`.

**Shortcut tips:** `browser_core::hints::Throttle` decides whether a toast names a command's
shortcut after it ran from the command bar (`Throttle::hint`, text `hints::text`): at most
`MAX_SHOWS` times per command, never in a profile's first day, at least `QUIET` (two days) after the
profile's last tip, never while the user is typing (on macOS and Windows: while a text field has the
keyboard), and never once Settings turns off
`prefs::FEATURE_HINTS` (Settings › Appearance › Tips on every client, explained by `hints::NOTE`).
Counters live in the profile's `feature_hints` rows, which go with the profile; a private space or
burner tab offers none. macOS and Windows offer one after a command bar command, Linux after a
command bar or panel menu command, the iPad keyboard layout after a command bar command; a command
that showed its own toast gets none.

**Hung pages:** `engine::lifecycle::Hang` decides when the shown page counts as hung: each shell
pings it every `PING_EVERY` with `engine::page_scripts::PING`, which reads nothing and answers
only once the page's main thread runs it (macOS and Windows `WebView::ping`: WebKit's client
world, Chromium one `Runtime.evaluate` of a constant in the page's world; NSH components `ping`,
answered by `pong`: the isolated world on Linux and iOS, the page world on Android, which has no
other). A page whose main thread is stuck never answers, and the shell shows a card (`HANG_NOTE`)
once a ping has gone unanswered for `HANG_AFTER`. Blind spot: under Lockdown no script of Tarn's
runs, so every engine answers a ping at once and a hung Lockdown page shows no card. iOS and
Android redraw only when the card appears or goes, never for a ping or its answer.
Wait hides it for another `HANG_AFTER`; Reload gives the page a fresh start: macOS and Windows
rebuild the tab's view, Linux's component takes `reloadFresh`, which ends the view's web process
(`webkit_web_view_terminate_web_process`) and reloads, and iOS and Android give the tab a new view
(gaps in `docs/mobile-plan.md` and `docs/windows-plan.md`).

**Tabs on top:** `browser_core::layout::TabPlacement` (`Side` or `Top`) is each window's own,
toggled by `shortcuts::Action::ToggleTabsOnTop` (`mod+alt+shift+t`) and the command bar's
`palette::Command::TabsOnTop`; `TabPlacement::toggled` stores the pick for new windows and `shown`
keeps phones and the touch layout at the side. `browser_core::sidebar::strip` lists the strip:
tiles and pinned tabs (as icons), then the other tabs in the sidebar's order, including every tab of
a collapsed folder or group, so none is out of reach while the sidebar hides. With tabs on top the
sidebar hides without forgetting its own setting and no floating sidebar appears at the edge;
ToggleSidebar brings the side back. macOS and Windows (`crates/app/src/tab_strip.rs`), Linux
(`tab_strip` in `linux/src/ui.rs`, rows in `Snapshot::strip`) and the iPad Mac layout
(`strip` in `mobile/app/src/ui/mac/sidebar.rs`) draw it under the toolbar.

**Space switching:** `browser_core::sidebar::slide_start` says where the next space's list starts
its spring, from where a running slide or drag stands, or `None` under Reduce Motion (macOS and
Windows). The spring is critically damped: `spring` in `crates/ui/src/motion.rs`, and
`SPRING_DAMPING` in `nsh-shared/src/motion.rs`, whose `space_switch` animates Linux, iOS and
Android's layout on that spring over `SLIDE` before a click, key or swipe switches the space.

**Density:** `prefs::COMPACT` (Settings › Appearance, explained by `prefs::COMPACT_NOTE`) picks
the second of each density token, `TAB_ROW` and `ROW_GAP`, defined once in `crates/ui/src/theme.rs`
and mirrored in `nsh-shared/src/theme.rs` (a test keeps them equal); every shell reads them through
`Theme::tab_row` and `Theme::row_gap`, set with `Theme::set_compact` when the pref is read.

**Command bar answers:** `browser_core::palette::answers` gives the rows the command bar answers
offline, listed first on every client, labels cut to `MAX_ANSWER_LABEL`; picking one (arrow keys
then Enter, a click or a tap) copies `Answer::copy`. An answer row is never the default:
`palette::default_row` gives the first row after them, so Enter alone runs what was typed (search
or navigate) on macOS, Windows, Linux, iPad and phone. `palette::calc` evaluates `+ - * / % ^`,
parentheses and decimals with at least one operator between two numbers (a signed number, or digits
joined only by `-` or `/` such as a date or "24/7", gets no answer unless typed after `=`; at most
`calc::MAX_INPUT` bytes, nesting bounded, finite results only, exponent notation outside 1e-9 to
1e15), `palette::units` converts length, mass, temperature, volume, data size and time ("5 km in mi",
"1,5 km in m", "12 in to cm"), and `palette::emoji` finds emoji by name after a `:` in a built-in
table. Nothing is fetched, so there is no currency conversion.

**Command bar sections:** while nothing is typed (`palette::sectioned`), every client lists
`palette::sections(state, offered, data)` instead of ranked rows, under `command::Section` headers:
Now (commands for the current tab), Favorites (pinned tabs, then top-level bookmarks), Recent Tabs
(the `SECTION_ROWS` most recently used other tabs), Frequent (history by `suggest::frecency`) and
Quick Actions; empty sections are left out and a page shows once. `palette::Data` carries the
space's tabs, the active tab, the profile's bookmarks and the history the bar already loaded. A
private space lists no Recent Tabs and no Frequent rows. Nothing is highlighted until the user
moves, so Enter on the empty field opens nothing.

**Command bar actions:** `palette::actions(item, spaces)` is the actions page of a row
(`palette::Item`: a command, a tab, a bookmark or a history page), named and in order; `spaces`
names the client's move targets for a tab, so `Action::MoveToSpace(ix)` indexes them. ⌘K (Ctrl+K)
already opens the command bar (`shortcuts::Action::CommandBar`), so the page opens with → when the
field's caret is at the end with nothing selected (desktop and the iPad Mac layout), with the row's
… button, or with a row's More button on phones; Esc (Back on phones) returns to the rows.

**Invalid address:** Enter on typed input that `command::classify_input` rejects
(`Input::Rejected`) opens nothing; the field shakes sideways once for `palette::SHAKE` (the
`SHAKE` token in `crates/ui/src/motion.rs` and `nsh-shared/src/motion.rs`) along `palette::shake`,
and each new Enter shakes it again. Under Reduce Motion it does not move and shows
`palette::INVALID_ADDRESS` beside it until the text changes.

**Notes:** `browser_core::notes` is the notes list and editor every client renders: `rows(notes,
query)` filters by title and text without case (title matches first, last edited first) and gives
each row its shown title (`notes::title`: the title, else the first line, else `UNTITLED`) and a
preview line; `Draft::save` checks a draft and saves it through `store::Store::save_note`, which
drops invisible characters (`store::is_invisible`), caps the title at `MAX_NOTE_TITLE`, the text at
`MAX_NOTE` and a profile at `MAX_NOTES` notes, and refuses a private profile. `notes::as_pill`
gives the `assistant::Pill::Note` that attaches a note to a question. Notes live in the profile
table `notes`, so export, Delete everything and deleting the profile cover them; they are never
synced.

**Assistant:** `browser_core::assistant` is the chat every client renders and the rules its
requests follow (threat model S2, S3, S9); it does not depend on the `ai` crate and sends nothing.
`Session::new` binds a chat to one profile and refuses a private space, a burner tab, a Lockdown
level or a profile whose `ai_enabled` is off (`store::Store::set_profile_ai`). Every `Origin` a
client passes carries the page's effective Shield level, including its per-site setting, and a
client drops the `Session` when its profile is deleted or reset or its AI switch is turned off.
`Session::apply` takes an `Event` (add or remove a pill, stream chunk, done, failed, cancel) and
`Session::send` a send; both return `Effect`s (start or stop a request by id, append a chunk to the
last row, render). One request runs at a time, at most one send per `MIN_SEND_INTERVAL_MS`, and
chunks of a stopped request are ignored. `send` takes the current `Origin`, the AI switch and the
effective level of each pill's page now (`site::effective_level`): from a private space, a burner
tab, a Lockdown page, another profile or with AI off it is refused and every pill dropped, and a
pill whose page went to Lockdown, or whose address does not parse, is dropped and the send refused;
a pill removed earlier is still checked. Either way every exchange made since a dropped pill was
attached is marked interrupted. An answer stops at
`MAX_PILL_BYTES`; a cancelled or failed exchange, or one whose answer stayed empty, loses its empty
answer, stays visible as interrupted and is never sent again. A `Pill` (page, tab, selection with
its page's address, note) is added only by an explicit event, only from the session's own profile
at a level `allowed` accepts, and gets its label (`Page 2`) once when added. `compose` builds the
request: `{question}`, `{page}` and `{selection}` are expanded in one pass over the template only,
each pill becomes one `Block` with URLs stripped of query, fragment and sign-in, control, bidi and
other invisible characters (`store::is_invisible`) dropped and `tarn-block` removed in any case, at
most `MAX_PILL_BYTES` a block and `MAX_TOTAL_BYTES` over pills and history. Each request delimits
its blocks with a tag holding a random `nonce` of its own (`Composed::wrapped`), named in its
system prompt (`Composed::system`); `Composed::user_message` is the user turn a provider sends.
`selection(url, raw)` makes a selection pill bounded by `page_ops::selection`, and
`commands::for_selection` the template a selection's menu command sends with no question, where
`{page}` names the selection. `tabs::Model::assistant_tabs` is the chat's tab picker: the current
space's other web tabs the assistant may read, each `awake` only while its view is live; a
sleeping tab is listed but never woken to be read.
`settings` keeps the provider (Claude API or Ollama on this computer) and the Ollama model name. `commands::all` lists the built-ins and the profile's own
(`store` table `ai_commands`, names at most `store::MAX_AI_NAME` bytes without control or invisible
characters, removed with the profile and by Delete everything). `links::classify` refuses every
scheme but `http`, `https` and `mailto`, any sign-in parts and local and home-network hosts
(`shield::rules::local_target`); the rest open only after a confirmation that shows the host and
the whole address, `mailto` with its headers removed and refused when its address encodes `?`,
`&`, a control or an invisible character. `links::shown` always shows the whole
address. `view` returns the `ChatView` a client draws: rows, pill chips, the blocks a send would
carry with their sizes, the notice and the send and cancel states. `browser_core::palette` has
`Command::Ask` and `Command::AskPage` (which also needs a page), offered only when the client sets
`State::assistant` from `assistant::allowed` and the AI switch (`tabs::Model::assistant_offered`,
with `assistant_origin` giving the active page's `Origin`). `sidebar::assistant_grouping` and
`group_as_proposed` are Clean Up's Group with Assistant for a window's current space. Linux, iOS
and Android share Settings › AI, the chat and the grouping run in `tarn_nsh_shared::assistant`
(`Host` for the shell's store, origin and page text; `Platform` for its secret store, transport
and whether Ollama works there): providers stream on a background thread and the chat takes what
arrived once a frame.
