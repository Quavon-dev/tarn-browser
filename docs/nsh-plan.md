# Linux and mobile on Native Shell (NSH)

Status: P0 embedding checks done for items 1–3. P2 (Linux app) built, checked in Docker only (see P2). Still open: a real Linux desktop and HiDPI, an iOS device, IME, screen readers and the tab-list performance. GPUI stays the shell on macOS and Windows. Linux, iOS and
Android get one shared shell built on [Native Shell](https://nsh.dev/) (`longbridge/nsh`, Apache-2.0,
pre-1.0). Each platform uses the system web engine, so the vendor or the distribution patches it,
never us. This plan covers Linux and mobile only; macOS and Windows are out of scope except for the
shared core in §3. Issue #2 (Linux) resumes under this plan.

## 1. Why NSH

| Need | How NSH meets it |
|---|---|
| One UI codebase for Linux, iOS and Android, in Rust | NSH renders with the platform's own widgets: GTK 4 on Linux, UIKit on iOS, Android views. Layout runs in Rust (Taffy). Rust apps include no JavaScript engine. |
| The system web engine inside the shell | Web views are native widgets in the NSH tree. On Linux this lets WebKitGTK sit beside the shell UI, which GPUI could not do (issue #2 tests). |
| Native text input, IME and accessibility | "Text input, selection, input methods and accessibility come from the system." |
| Footprint | NSH's own hello world is 12–16 MB with about 14–19 MB of memory at rest. The P0 builds came out larger: a 35 MB Linux binary, a 48 MB iOS app and a 46 MB APK (of which `libnsh.so` is 39 MB). Tarn's current GPUI binary is 10.8 MB. Reducing this means trimming NSH features (P1). |

Accepted costs:
- **Pre-1.0:** the API changes between releases. NSH is pinned to an exact version and moved forward deliberately.
- **No drawing over the page:** web views are drawn "above everything NSH draws", so overlays hide or shrink the page, as on macOS today.
- **Thin built-in web view:** NSH's `WebView` is too thin for Tarn, so Tarn ships its own web-view component (§4).

## 2. Constraints that do not change

- **iOS:** browsers must use WebKit (`WKWebView`), except under BrowserEngineKit in the EU, which is out of scope.
- **Android:** the system WebView, updated by Google through the Play Store. No bundled Chromium or Gecko.
- **Linux:** the distribution's **WebKitGTK 6.0** (GTK 4 API), patched on Fedora, Debian, Ubuntu and Arch (checked 2026-11-13). Shipped only as distribution packages (`.deb`, `.rpm`, AUR); no Flatpak or AppImage, so we never ship WebKit ourselves.
- **Threat model:** every mitigation in `docs/threat-model.md` must hold on each new platform, or the gap is listed in §9.

## 3. Architecture

```
crates/
  store, shield, sync, devkit, ai   unchanged, shared by every shell
  engine-core     (new) platform-neutral engine logic: nav_policy, content_rules, downloads,
                  lifecycle, events, prompt_kind, burst, scripts
  browser-core    (new) the browser model without UI: profiles, spaces, tabs, folders, groups,
                  history, Shield levels and overrides, filter lists, downloads list, prompts
                  queue, commands. Exposes state plus actions; no GPUI and no NSH types.
  engine          macOS and Windows backends (GPUI hosts), depends on engine-core
  app             GPUI shell for macOS and Windows, moved onto browser-core step by step
  nsh-app   (new) NSH shell: phone, tablet and Linux desktop layouts over browser-core
  nsh-webview (new) Tarn's web-view component for NSH, one backend per platform (§4)
```

- **`engine-core`:** `content_rules` (the platform-neutral evaluator of WebKit content-blocker JSON,
  used on Android) is in `crates/engine`. Bursts, page scripts and the prompt model are shared through
  `engine::view_policy`, `engine::page_scripts` and `browser_core::prompts`.
- **`browser-core`:** the interface, the rule for new code and the timing are in `docs/shared-core.md`.
  - **Interface:** shells reach the web view through one contract, the method set of
    `engine::WebView` plus `engine::Event`, and `tarn.webview` follows it.
  - **Timing:** `browser-core` holds the app-level state that used to live in the per-window
    `Browser`; it was not a separate extraction from `crates/app`.
- **NSH integration:**
  - `nsh-app` uses NSH's Rust API, not TSX, so no QuickJS.
  - Heavy work (store, sync, filter-list conversion) stays off NSH's engine thread, because NSH
    warns that heavy synchronous work blocks the host UI thread.

## 4. Tarn web-view component (`nsh-webview`)

One NSH native component, registered by the host, with the same contract as `engine::WebView` today:
- **Commands:** load, back and forward, reload, stop, zoom, find, mute, snapshot, print or PDF.
- **Events:** the `engine::Event` set.
- **Shell scripts:** run in an isolated script world.

All three backends call the same `nav_policy::decide` and `level_action`.

| | iOS | Android | Linux |
|---|---|---|---|
| **View** | `WKWebView` (Swift wrapper or objc2). Most of `crates/engine`'s macOS WebKit code carries over; AppKit parts are replaced by UIKit. | `android.webkit.WebView` in a Kotlin component, called from Rust over JNI | `WebKitWebView` (WebKitGTK 6.0) as a GTK 4 widget |
| **Profiles** | `WKWebsiteDataStore(forIdentifier:)` per profile (iOS 17+) | `androidx.webkit` `ProfileStore`, one profile per Tarn profile (needs the multi-profile feature; to verify) | One `WebKitNetworkSession` per profile, with its own data and cache directories |
| **Private space** | `nonPersistent()` data store | A separate `:private` process with its own WebView data folder (`ProcessGlobalConfig.setDirectoryBasePaths`, a new random folder per session). The folder is deleted when the private space closes and swept at every launch (see §9) | An ephemeral `WebKitNetworkSession`, which keeps nothing on disk |
| **Shield** | `WKContentRuleList`, unchanged | `shouldInterceptRequest` with `engine-core::content_rules`, which needs a literal prefilter first (§6) | `WebKitUserContentFilterStore`, which takes the same JSON as on macOS |
| **Navigation policy** | `WKNavigationDelegate` (shared code) | `shouldOverrideUrlLoading` and `onCreateWindow` | `decide-policy` signal |
| **Dialogs and permissions** | `WKUIDelegate` | `WebChromeClient` (`onJsAlert`, `onPermissionRequest`, …) | `script-dialog` and `permission-request` signals |
| **HTTP sign-in and TLS** | delegate challenge, default trust | `onReceivedHttpAuthRequest`; `onReceivedSslError` always cancels | `authenticate` signal; TLS errors always fail |
| **Downloads** | `WKDownload` into the app container, shared through the Files app | `DownloadManager` or a stream to app storage | `download-started`; name sanitising and quarantine attribute (`user.xdg.origin.url`) |
| **Extensions** | `WKWebExtensionController` if available on iOS (to verify); otherwise none | None | Tarn extensions (`browser_core::tarn_ext`, see below); classic WebExtensions are not supported |

### Tarn extensions on Linux

Linux runs Tarn extensions (manifest `browser_specific_settings.tarn.runtime: 1`) on the shared
runtime in `browser_core::tarn_ext`; the shell only carries out its decisions.

- **Install, consent, enable, every-site, remove:** Settings › Extensions, the shared pane
  `tarn_nsh_shared::extensions` (file, GitHub repository, official catalog). A changed install
  reloads the profile's runtime (`linux/src/extensions`).
- **Content scripts and CSS:** `webkit_user_script_new_for_world` and
  `webkit_user_style_sheet_new_for_world` in the world `tarn-ext-<install id>`. WebKitGTK user
  content is pattern based: a view holds each frame's document at its response decision
  (`decide-policy`, response, the final address after redirects) until the core's gate has
  answered, and gives each approved script an allow-list of exactly that document's address,
  query included, which a later answer for the same address takes away again. Nothing the gate
  did not return is injected, a main-frame load clears the earlier answers and drops the documents
  still held for the page before (a late answer for them allows nothing), a view that goes while
  it waits ignores the held document, and a document whose question cannot reach the shell loads
  with nothing allowed at its address. The site prompt is raised for the main frame only.
- **Threads:** NSH runs the shell on its `nsh-engine` thread and WebKitGTK on the GTK main
  thread. The runtime lives on the shell thread (`linux/src/extensions`); it hands owned copies
  to the GTK thread through the default main context (`glib::MainContext::spawn`): each
  persistent store's worlds and rule lists, replies, and extension pages to make, post to or
  close. The views send frame questions and page messages back over a channel the shell thread
  drains; a tab's messages arrive as the view's `extensionMessage` event. Tested with the runtime
  on its own thread in `linux/src/webview/ext_display_tests.rs`.
- **Messages:** a script message handler registered for that world only
  (`webkit_user_content_manager_register_script_message_handler(name, world)`), so the page's
  main world cannot post; replies are evaluated in the same world.
- **Extension pages:** `tarn-extension://` registered on the web context
  (`webkit_web_context_register_uri_scheme`, secure and display-isolated). The handler serves the
  core's `Resources` with their CSP only to the extension's own page views; any other requester,
  web pages included, gets 403. Background pages are hidden views in the profile's network session
  with its Shield rules, started on demand and stopped after 30 s idle; the popup is shown under
  the toolbar. Page views may only navigate within their own origin.
- **Network rules:** each install's rule list compiles through `WebKitUserContentFilterStore`
  into the tab's content manager, for the hosts it holds a grant for.
- **Not in private spaces, burner tabs or Lockdown.** Files are hash-checked before every load; a
  changed install is turned off and its site grants are dropped.

Gaps, with their reason:

- **Store links in frames.** WebKitGTK's navigation policy decision names no frame: the pinned
  `webkit6` 0.6.1 `NavigationAction` has only `frame_name`, the target name a page may give, and
  no main-frame flag. So the user's own click (with a gesture) on an install link inside a frame
  also opens the consent for that catalog id. Frames a page loads and script navigations without
  a gesture never do (`linux/src/webview/display_tests.rs`), and the consent still shows only
  Tarn's verified catalog entry and waits for the user.

- **Messaging from subframes.** WebKitGTK gives the UI process no frame for a script message:
  in WebKitGTK 2.54.2 (Fedora 43, checked 2026-11-16) and the pinned `webkit6` 0.6.1,
  `script-message-received` carries only the value and `script-message-with-reply-received` adds
  only a `WebKitScriptMessageReply` (a return value or an error), and `WebKit-6.0.gir` has no
  `WebKitFrame`. `WebKitFrame` and per-frame script worlds exist only in
  `WebKitWebProcessExtension-6.0`, a shared library loaded into the web process, which needs
  bindings `webkit6` 0.6.1 does not have; `webkit_web_view_evaluate_javascript` reaches only the
  main frame. A frame named in the message body would make the body the sender's identity,
  which the contract forbids (`docs/shared-core.md` §1). So the shim's transport is set up only
  in the top frame: content scripts with `all_frames` run (JS and CSS) in a subframe only when
  the gate grants that frame's own host, but their `chrome.runtime` and `chrome.storage` calls
  reject with "The message could not be sent." and `tabs.sendMessage` reaches only the main
  frame (`linux/src/webview/ext_display_tests.rs`, cross-site frame test). The world's message
  handler itself is registered for the whole view, so the main frame's transport makes a random
  token per document, posts it first and prefixes every message with it; the shell takes the
  first token after each main-frame commit and drops any message without it, so a subframe's
  world posting through `window.webkit.messageHandlers` is not heard. A page the back/forward
  cache restores (a commit with no response of its own) keeps the tokens its document drew,
  remembered under the history item it left from (also one it made with `pushState` or a
  fragment) while the item is in the list. Messaging from subframes
  needs a web-process extension.
- **An opened file's subresources.** macOS WebKit, Chromium, iOS and Android let a page opened with
  Open File load `file:` subresources only from its own folder (`nav_policy::file_subresource_allowed`,
  WebKit's read-access grant). WebKitGTK's UI process sees only navigations, not subresource
  requests (`send-request` exists only in `WebKitWebProcessExtension-6.0`, which the pinned
  `webkit6` 0.6.1 cannot load), so on Linux such a page may load other local files the web
  process's sandbox lets it read as images or styles. Scripts still cannot read them (file pages
  have no file access from file URLs), and a page's own navigation to a `file:` URL is blocked.
- **`document_idle`** runs at document end: WebKitGTK user scripts have only start and end.

## 5. Platform pieces outside the web view

| | iOS | Android | Linux |
|---|---|---|---|
| **Secrets** (API token, sync key) | Keychain | Android Keystore | Secret Service (libsecret) |
| **Data directory** | App container | App-private storage | `$XDG_DATA_HOME/tarn`, mode `0700` |
| **Default browser** | `com.apple.developer.web-browser` entitlement | `ROLE_BROWSER` via `RoleManager` | `.desktop` file and `xdg-settings` |
| **Updates** | App Store | Play Store, plus a signed APK on GitHub | Distribution packages; engine updates come from the distribution |
| **Nightly channel** | Gap: the App Store installs only what Apple reviewed; TestFlight is the place for nightlies | Gap: the Play Store decides which build installs; its internal testing track is the place for nightlies | Gap: the package manager installs from the repositories the user added; `linux-repo.yml` publishes releases only |
| **Claude CLI bridge** | No | No | Unix socket in `$XDG_RUNTIME_DIR`, peer checked with `SO_PEERCRED` (later) |
| **Sync** (M9) | Encrypted file in iCloud Drive or Files | Storage Access Framework folder | Folder picker |

## 6. Phases

Each phase ends with an exit check, run on real devices where the platform needs one.

**P0: embedding checks, 2 weeks.**
1. **Linux:**
   - an NSH window with a `WebKitWebView` (WebKitGTK 6.0) next to NSH widgets: typing, focus moving between page and shell, resizing, HiDPI;
   - in Docker and on one real Wayland and one X11 desktop;
   - check that NSH works with WebKitGTK 6.0, since its docs mention 4.1.
2. **iOS:** a custom NSH component wrapping `WKWebView` with a content rule list and a navigation delegate, in the simulator and on a device.
3. **Android:** a custom component wrapping `WebView`, with `shouldInterceptRequest` calling `content_rules`, and `ProfileStore` with two profiles.
4. **Text input and accessibility:** Japanese IME and dead keys in the address field on all three; VoiceOver and TalkBack reach the tab list.
5. **Real UI:** the tab list and address bar in NSH on phone and Linux. Measure frame time while scrolling 500 tabs, plus cold start and memory.

**P1: shared core.** `browser-core` (`docs/shared-core.md` §3), plus what only the NSH shells need.
- Add the literal prefilter and a benchmark to `content_rules`, or use the planned
  `shield::matcher`, for Android.
- Exit: the existing tests pass, one macOS window behaves as before, and
  `browser-core` has its own tests for tab, space and profile behaviour.

**P2: Linux app.**
- `nsh-app` desktop layout:
  - sidebar with spaces, tabs and folders;
  - address field and command bar;
  - Shield levels and site overrides;
  - private space;
  - settings, history and downloads.
- Packages: `.deb`, `.rpm` and AUR that depend on the distribution's `webkitgtk-6.0`.
- CI: a Linux job builds `nsh-app` and runs the tests.
- Exit: the acceptance checks from #2 pass on current Fedora, Debian 13, Ubuntu 26.04 and Arch.

*Status:* built in `linux/` (crate `tarn-linux`, its own workspace;
`linux/README.md`). Exit checks ran in Docker under Xvfb only (`linux/evidence/README.md`); every
check passed there except the camera prompt (no device) and the AT-SPI role of the NSH address
field (NSH gap, see P0). Not verified: a real Wayland or X11 desktop, HiDPI, a composed IME, a
screen reader, window position on restore (GTK 4 leaves it to the window manager), and the
filter-list download (no signing key or feed in this build; a test-signed bundle was used).
- **Web view:** `tarn.webview` implements contract v2 for WebKitGTK 6.0, with three Linux additions
  documented in `linux/README.md`: `levels["*"]` carries the profile's level, a `lists` prop, and
  `download` events with `url` and `quarantined`. HTTPS-only is checked at navigation time on any
  port. WebKitGTK does not say whether a navigation targets the main frame, so JavaScript is never
  switched per navigation: a clicked link into a site with JavaScript off gets a rebuilt view, and
  each main-frame commit is checked again. Dialogs and permission requests from a cross-origin
  frame show the top page's origin, because WebKitGTK gives no frame origin.
- **Packages:** `.deb` (Debian 13, Ubuntu 26.04), `.rpm` (Fedora 43) and an AUR `PKGBUILD`, each
  depending on the distribution's WebKitGTK 6.0 (`linux/packaging/`). Each was built in a clean
  container of its distribution and installed and started in another (Debian 13, Ubuntu 26.04
  and Fedora 43 on arm64, Arch on emulated x86_64; `linux/evidence/README.md`). Not built:
  amd64 `.deb`/`.rpm`. The `PKGBUILD` checksum is `SKIP` until the `v0.1.0` tag exists.
- **CI:** job `linux` in `.github/workflows/ci.yml` runs in a `fedora:43` container
  (pinned by digest) on `ubuntu-24.04`: `cargo clippy --all-targets` and `cargo test` in `linux/`, then
  the WebKitGTK display tests under Xvfb (`linux/scripts/display-tests.sh`, which `test-display.sh` now
  runs inside Docker). Verified locally in the same container on arm64. The display test passes, then WebKitGTK
  crashes at process exit: libtest runs the test off the main thread, and WebKitGTK disposes its
  default context from an exit handler on the main thread (`WebProcessCache::clear` in
  `~WebProcessPool`). This happened before the change too (the old script exited 133); the script now
  requires a passing test result and then accepts only a crash at exit (status 133 or 134,
  SIGTRAP or SIGABRT).
- **Shared code:** moved, behaviour-neutral for macOS: the
  web-view policy into `engine::view_policy`, the filter-bundle conversion and debug key into
  `shield::groups`, address-bar parsing helpers into `browser_core::command`, the prompt model and
  crash-reload throttle into `browser_core::prompts`, and the theme, icons and generic controls into
  `nsh-shared/` (crate `tarn-nsh-shared`, also used by the mobile app). What stays per shell is in
  `linux/README.md` ("Shared code").
- **Smoke pass after the move:** Docker, Fedora 43, Xvfb, no window manager; evidence in
  `linux/evidence/polish/`.

  | Check | Result | Evidence |
  |---|---|---|
  | Browsing | Pass | `browse.*`, `start.png` |
  | Tabs (a `target=_blank` link opens a tab) | Pass | `tabs-popup-external.*` |
  | External scheme (`external` carries `origin`) | Pass | `external-mailto.*` |
  | Profiles | Host tests only (`browser::tests`) | |
  | Private space: no Personal cookie, marker nowhere on disk | Pass | `cookie-personal.*`, `private-space.*`, `private-disk.txt` |
  | Shield: private network from `public.test` | Pass: no `/pixel` request | `private-network.*` |
  | Shield: HTTPS-only at Strict, typed `localhost:8765` | Pass: upgraded, TLS attempted, no plain request | `settings-strict.png`, `https-only-typed-port.*` |
  | Prompts: alert, confirm, location | Pass (taken before the review fix that removed Always Allow on Linux, see `docs/mobile-plan.md` §2 `topOrigin`) | `prompt-*.png`, `prompts.txt` |
  | Downloads (`url`, `quarantined: true`) | Pass | `download.*` |
  | Find, zoom 125 % | Pass: 5 matches | `find.png`, `zoom-125.png`, `find-zoom.txt` |
  | AT-SPI tree | Pass for names: every button, segment and switch is named; switches are checkable (`check box`) | `atspi-tree.txt`, `settings.png` |

*Design:* the Linux app follows the macOS app (`crates/ui/src/theme.rs`): navy
backdrop gradient in the brand mark's lake blues, an inset sidebar card with the blue gradient,
lake-blue 16 % edge and elevation shadow, a Safari-style address capsule that shows the host at rest (`Not Secure — host`
in amber for http, long hosts cut at the start), a Spotlight-style start card with the brand mark over
the drifting ASCII fog (`tarn_nsh_shared::fog`, a canvas redrawn every `browser_core::fog::FRAME`, one
still frame under the desktop's reduced motion), an accent edge on the active tab row, an accent-tinted New Tab
pill, a tab close button that slides in on hover (`ElementRef::animate_layout`), flat surface cards for prompts, banners and the find bar, and System Settings-style panels. Its tokens are
mirrored by name in `nsh-shared/src/theme.rs`, shared with the mobile app (a test checks them against the GPUI source) and the
icons and brand mark are `crates/ui`'s own files. Deviations:
- The web view is drawn above NSH, so suggestions, prompts, banners and the find bar take layout
  space above the page instead of floating over it, and settings, history and downloads
  replace the page instead of opening their own window or popover.
- The address field shows the host centred at rest as a label; editing happens in the command palette, a card centred
  in the page area. The web view is drawn above NSH, so while the palette is open the page is hidden (its views count as
  hidden, as under a panel, so WebKitGTK repaints them afterwards) and the palette sits over a 35 % black dim of the
  backdrop instead of the page.
- The find pill shows the number of matches, not "3 of 17": WebKitGTK's find controller reports a count only.
- The address field's inline completion (`suggest::edit`) runs only when one character was typed at the end of what was typed before (the first key replaces the address selected on focus), since NSH inputs report the new text but not the caret or the kind of edit, so a paste never completes; Enter on the completed text opens the visited page it completes to.
- Settings have no Suggestions switch yet, so history rows and the inline completion are always on (macOS gates both on Settings › Suggestions); both are computed locally from history, as on macOS.
- Motion uses `tarn_nsh_shared::motion` (POP, FADE, EXIT, SLIDE as on macOS): pop-ups, the palette, prompts, the find pill
  and state glyphs pop in; tab rows grow and shrink, the sidebar and space switches move as layout animations; switch
  knobs slide. All of it lands at once under the desktop's reduced motion. Pop-ups do not animate out (NSH removes a
  view at once) and a space switch does not slide its list sideways; the start-page fog, the tab close button and the reload and tab-row spinners (`tarn_nsh_shared::motion`, turned on the UI thread) move. The fog and spinners stand still while the window is not the active one (`lifecycle()`), as on macOS.
- Pop-ups (panels, suggestions, popovers, tooltips) are views and NSH overlays inside the main window, and menus are
  GTK popovers of it, so none floats over another app's window when Tarn is behind it (checked with a second GTK
  window over an open panel: `linux/evidence/backlog-popup-*.png`). There is no window level to manage.
- Prompts have no Remember toggle and no "Block requests from this page" yet, so permission answers
  are not saved and a page can keep asking.
- Window controls are the desktop's: the window manager's title bar and buttons (openbox in the
  tests). Without a window manager NSH 0.5.0 drew no title bar of its own under X11.
- New Window (Ctrl+N, or "New Window" in the command bar) opens another browser window in the same
  process (`open_window`). As on macOS the windows share one `Rc<Core>` (store, profiles, the private
  session, data stores) and each keeps its own tab model, space, pages, panels, notices and toasts
  (`linux/src/ui.rs`: one `Browser` and `Ui` per window). A tab is live in one window at a time: a new
  window never opens on a tab another shows, and showing it in a second window puts it to sleep in the
  first (the page reloads there; macOS moves the live view with its state). A window reloads its
  spaces and tabs when it gains the focus. Download events reach every window and are recorded once.
  Closing a window burns its burner tab and deletes its saved row; closing the last one quits and
  keeps its row. The main window hides when closed while others stay open (NSH closes a main
  window's windows with it). Each window's row keeps its space, tab and size and reopens at launch;
  the first window also keeps its placement (`linux.window_placement`). GTK 4 sets no window
  position, so a new window does not open offset from the front one as on macOS. Evidence:
  `linux/evidence/backlog2-new-window.png`.
- Tab sleeping follows macOS: a tab sleeps beyond the six-tab live limit, or after 30 minutes off
  screen (one minute while `MemAvailable` in `/proc/meminfo` is under a tenth of `MemTotal`, the
  counterpart of the macOS kernel pressure level); pinned tabs and tiles skip the idle timer, and a
  tab playing audio or using the camera or microphone stays awake (WebKitGTK `is-playing-audio` and
  capture states). Sleeping rows are greyed (`Theme::asleep`, the site tile dimmed). Gap: WebKitGTK
  has no playback-state query like WebKit's `requestMediaPlaybackState`, so silent or muted video
  does not keep a tab awake. Settings has no "Sleep after" or live-tab choice yet; the macOS
  defaults apply. Evidence: `linux/evidence/parity-sleeping-tabs.png`.
- A two-finger horizontal swipe over the sidebar switches to the previous or next space, never
  towards one that does not exist (`linux/src/browser/swipe.rs`, through NSH `on_wheel` point
  deltas; mouse wheel lines are ignored). Gap: NSH 0.5's `WheelEvent` has no touch phase
  (nsh-platform-linux `wheel_event` drops GDK's zero-delta scroll-stop event, `gdk::ScrollEvent::is_stop`),
  so a gesture ends after 200 ms without scrolling and the list does not follow the fingers and
  spring back as with macOS `gestures::follow`. The direction follows the content, so it matches
  the fingers with natural scrolling on.
- A blocked pop-up's notice offers "Open in New Tab" for an http(s) address, as on macOS: the
  button opens it once as the user's own load, in the opener's space; the page alone cannot, nor
  can it replace a notice that already offers an action (`browser::tests`). Evidence:
  `linux/evidence/parity-popup-offer.png`, `parity-popup-opened.png`.
- Screenshots: `linux/evidence/design-*.png`.

*macOS features:* built on the shared APIs, with `tarn.webview` contract v3 (scripts in the WebKit world `tarn`,
nothing under Lockdown, answers checked in Rust); details and deviations in `linux/README.md`
("Features from the macOS app"). Verified in Docker under Xvfb: host tests, three display tests (v3
scripts, styles and zaps; nothing under Lockdown; no private or burner data on disk) and screenshots
`linux/evidence/feature-*.png`. Deviations: no download pause/resume or malware flag, Read Aloud only
with `spd-say` (no pause), the Shield card (the Mac's glacier lake from `browser_core::shield_card`, words drawn as native
text over its SVG landscape since NSH's SVG renderer draws no text) flips and takes a New Design but does not tilt, and Share card saves its front as a PNG through the save dialog (the
desktop's image loaders draw the SVG); Settings is a sheet with grouped navigation (You: Profile, Profiles; Browser:
General, Appearance, Search, Privacy, Permissions, Shortcuts) over scrolling panes, without the Mac's cross-fade,
and the History, Downloads and Reading List panels replace the page, so they neither grow in nor slide; memory per tab only where the web
process can be told apart; skipped: translation, startup trace, live peek.

**P3: iOS app.**
- Phone and tablet layouts: tab switcher, bottom toolbar, sheets for settings and Shield.
- Data store per profile, private space, Shield, downloads into Files, and the default-browser entitlement.
- CI: simulator build and tests on the macOS runner.
- Exit: TestFlight build that passes the egress and isolation tests.

**P4: Android app.**
- Same layouts as iOS, plus the Android-specific pieces in §4 and §5.
- CI: emulator tests.
- Exit: internal-test build on Play.

**P5: sync and parity.**
- M9 sync between desktop and mobile, extensions on iOS if `WKWebExtensionController` allows it, and Bitwarden through the system AutoFill providers on mobile.

### Themes and search bar effects on Linux

Settings › Appearance is `tarn_nsh_shared::appearance` on `browser_core::theme_library`. "Edit in Text
Editor" writes the theme to `$XDG_DATA_HOME/tarn/themes/<id>.css` (folder `0700`) and opens it with the
desktop's default application for the file (`xdg-open` through NSH), which is a text editor on common
desktops but is the desktop's choice; saved changes are read back every second while Settings is open, as on
macOS. Shift at launch is read from GDK's keyboard modifier state (checked under Xvfb with xdotool). Website
sheets run in the `tarn` world before the user's CSS (display test). GitHub themes, update checks and the
official catalog go through `browser_core::github` over libsoup (`linux/src/fetch.rs`); the theme's
`--tarn-font` is the window's font, security UI keeps the system font. Checked under Xvfb against a fake
GitHub (loopback TLS server, a test CA trusted only in the container, the GitHub hosts mapped in its
`/etc/hosts`): `linux/evidence/themes-github-*.png`.

Search bar effect cost (release build, Docker, Xvfb, Mesa llvmpipe, 1280×820 window, start page, 30 s
from `/proc`): fog alone 102 % of one core (main thread 5 %, llvmpipe 96 %); Flames with the fog 289–306 %
(main thread 21–22 %, llvmpipe 264–281 %). The cost is GTK's GL renderer redrawing the whole window on
every frame in software: it falls with the window size (900×560: llvmpipe 147 %) and not with the size of
the effect (a canvas clipped to the flames' band changed nothing, so it was not kept); with
`GSK_RENDERER=cairo` the same page costs 63 %. Tarn's own work per frame is the main thread's share; on a
desktop with a GPU the rest is GPU work. macOS: under 1 %. Evidence: `linux/evidence/themes-*.png`.

## 7. Testing

- **Shared tests:**
  - `browser-core` and `engine-core` keep the platform-neutral tests (nav policy table, content rules, download names);
  - they run on macOS, Linux and in CI for every platform.
- **Same scenarios on every platform:** each `nsh-webview` backend runs the macOS delegate scenarios against a local test server:
  - popups without a user gesture;
  - floods of dialogs;
  - HTTP sign-in;
  - a TLS error page;
  - private-network blocking, including disguised hosts;
  - a reload after a content-process crash.
- **Egress test:** Linux records with a proxy; mobile uses a test device behind a recording proxy.
- **Performance budgets:**
  - measured per platform: cold start, memory with 10 open tabs, frame time while scrolling the tab list, time for Shield to check a request (Android);
  - limits are set after the first measurement.

## 8. Effort (rough, one developer)

| Phase | Weeks |
|---|---|
| P0 embedding checks | 2 |
| P1 shared core | 3–4 |
| P2 Linux | 4–6 |
| P3 iOS | 5–7 |
| P4 Android | 5–7 |
| P5 sync and parity | open |

## 9. Risks and open decisions

- **NSH maturity:** pre-1.0, one company (Longbridge). Pin exact versions; keep the shell thin over `browser-core`, so switching to Whisker or GPUI-mobile means rewriting views, not logic.
- **Android Shield cost:** every request crosses into Rust on WebView's I/O thread. The literal prefilter and the benchmark come before filter lists are switched on.
- **Android private space:** Android offers the private space with the separate-process
  design in §4. Data is on disk while the space is open, and after a crash, force-stop or WebView update until the next launch. This deviates from the "never touches disk" rule; `docs/threat-model.md` ("Android private space", S8)
  states the deviation. It rests on:
  - the separate process and the folder deleted on close and swept at launch;
  - `FLAG_SECURE` or `setRecentsScreenshotEnabled(false)` on the private activity;
  - `IME_FLAG_NO_PERSONALIZED_LEARNING` on its text fields;
  - a decision on WebView's origin-visit file (`AwOriginVisitLoggerPrefs.xml`, reversible hashes,
    also written by normal mode), whose deletion was not tested;
  - handling of Android restarting a private session by itself after its process is killed.

  The private space shows a short note that data is stored temporarily while it is open.
- **Linux:** settled in P0. NSH's Linux backend uses WebKitGTK 6.0 (GTK 4), and P0 embedded it.
- **HTTPS-only with WebKit:** on Linux, `make-https` did not upgrade a plain-HTTP load on a
  non-default port (`localhost:8765`). If confirmed, macOS and iOS share it, and HTTPS-only needs a
  navigation-time check in addition to the content rule.
- **`javascript:` links:** no engine asks the app about them; a plain `WKWebView` on macOS behaves the same. They are not treated as a navigation; Lockdown stops them because JavaScript is off.
- **NSH gaps found in P0, to raise upstream:**
  - accessibility of text inputs and labels;
  - GStreamer always required on Linux;
  - iOS 15 as the packaged minimum while profiles need iOS 17 (an app raises it with `ios.infoPlist.MinimumOSVersion` in `nsh.json`; `mobile/app` sets 17.0);
  - no supported way to combine a Rust app with host components on Android;
  - Rust 1.97 or newer required.
- **Design:** native widgets limit how closely mobile and Linux can follow the custom GPUI look. A shared design language (colours, icons, spacing) is defined in P2 and P3; matching the desktop pixel for pixel is not a goal.
- **App Store review:** browser apps need a privacy manifest and the browser entitlement request; plan time for review.
