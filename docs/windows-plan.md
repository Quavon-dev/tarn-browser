# Windows plan

Status: in progress. The macOS build stays the reference; every Windows piece
must meet the same threat-model mitigations or the gap is listed in §8.

## 0. What is built

The engine is **CEF (Chromium) only**, not WebView2: it reuses the macOS Chromium
engine code, so Shield's request filter, navigation policy, prompts, popups
as offers and the staleness gate are the macOS Chromium code. §1–§3 below are the original
WebView2 plan and are kept for the security mapping; where they disagree, this section wins.

- **Build:** `scripts/fetch-cef.sh` (Git Bash) unpacks `windows64` from `packaging/cef.lock`;
  build with `CEF_PATH=<repo>/target/cef/windows64` and the MSVC tools. CEF is a plain Windows
  dependency of `engine`, so no feature flag is needed; the app's `chromium` feature stays the
  macOS dual-engine switch. `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo test --workspace` pass on Windows 10 / x86_64.
- **Process:** `tarn.exe` runs Chromium's child processes itself (`engine::chromium::subprocess`
  first in `app::run`). `crates/app/build.rs` embeds `packaging/windows/tarn.manifest` (Windows 10
  compatibility, which Chromium's GPU process needs; per-monitor DPI, which GPUI needs) and an
  8 MiB main-thread stack, which CEF's renderer needs. GPUI draws without DirectComposition,
  whose surface would cover Chromium's child windows.
- **Sandbox:** CEF ≥ 138 sandboxes children only through its `bootstrap.exe` launcher, so a
  release ships that launcher as `tarn.exe` and Tarn as `tarn.dll` (`crates/app-dll`, a cdylib
  exporting `RunWinMain` that hands the launcher's sandbox information to
  `engine::chromium::set_sandbox_info`, then runs `app::run`). Every child starts through the
  launcher too, so renderers and the storage service run at untrusted integrity with a
  restricted token in a job, and the GPU process at low integrity with a restricted token
  (checked on a packaged build). The network service runs at medium integrity without a
  restricted token, as CEF 156 starts it by default. The DLL delay-loads user32, gdi32 and the other
  libraries CEF's own client DLLs delay-load, which a sandboxed child must not load.
  `scripts/package-windows.sh` gives the launcher Tarn's manifest (`mt.exe`) and the 8 MiB stack
  (`editbin.exe`). The launcher loads `tarn.dll` only when both have the same signature status:
  both unsigned, or both validly signed with the same certificate (primary signature
  thumbprint), so the signing step must sign `tarn.exe` and `tarn.dll` together and with one
  certificate, after `package-windows.sh` has edited `tarn.exe`. `cargo run` keeps the plain
  `tarn.exe`, which has no launcher and so starts Chromium with `--no-sandbox` (development only).
  The launcher still carries CEF's icon until Tarn has a Windows icon to put in its place. On each CEF update, check that `bootstrap.exe` carries no `CEF_INSTALLER_CONFIG` resource:
  CEF's launcher can download and install CEF itself when built with one (`USE_INSTALLER`).
- **Engine surface** (`crates/engine/src/*_windows.rs`): `Host` (HWND, DPI-scaled placement),
  `WebView` (a Chrome-style CEF browser in a child window), `DataStore` (one Chromium profile per
  Tarn profile in `<data>/Chromium/<store uuid>`; private and Lockdown stores in memory), a Win32
  context menu, a message-only window that runs CEF's external pump on GPUI's thread, and a
  thread message hook that translates key presses for Chromium's windows and returns keyboard
  focus to GPUI on a click. When the pointer leaves a GPUI window (into a page, a pop-up or
  another window), GPUI gets a move just outside it, which clears the hover it would otherwise
  keep. Key presses with Ctrl or Alt (except editing keys) also go to GPUI.
- **Platform:** AI token in the Credential Manager, memory pressure from `GlobalMemoryStatusEx`,
  data in `%LOCALAPPDATA%\Tarn`, or the shell's Local AppData known folder (the one the installer
  and uninstaller use) when the variable is missing (user-only ACL inherited, no Unix modes). The
  installer needs no administrator rights and always installs into the current
  `{localappdata}\Programs\Tarn` (`UsePreviousAppDir=no`); at start the installed `tarn.exe`
  re-points an earlier Make Default registration that names another path, a development build's
  included.
- **Window and input:** the toolbar and the settings header are the caption (the hook starts the
  system move; a double click maximizes), with Windows minimize/maximize/close buttons. Pop-ups
  opened without focus (address suggestions, hover cards, peek) stay inactive until clicked,
  since GPUI shows every window activated; the library and other focused pop-ups take keys.
  Shortcut hints read Ctrl/Alt/Shift (`ui::controls::keys`): ⌘ is Ctrl, ⌃+digit is Alt, ⌃⌘ is
  Alt+Shift, full screen is F11. Back/forward state follows Chromium's settled history.
  AppKit's event monitors (`Host::on_mouse_moved`, `on_mouse_down`, `on_page_mouse_down`,
  `on_escape`, `on_navigation_buttons`) are the same thread hook watching the messages of
  Chromium's child windows: the floating sidebar and hover card follow the pointer over pages, a
  click in a pane focuses it, Esc closes a peek from its page, and the mouse's back and forward
  buttons over a page step the tab's history.
- **Page requests:** camera and microphone ask with the page's origin as in WebKit tabs (screen
  capture is refused); HTTP Basic and Digest ask only for the page's own origin or the one its
  main frame is navigating to, three times at most per page load (proxy and integrated
  authentication are refused); a link to another app's scheme (`mailto:`, `tel:`) asks before it
  is handed to Windows. Shift-click opens a link in a peek; Ctrl- and middle-click and the page
  menu's Open Link in New Tab open a tab under the pop-up rules, its first load judged as the
  page's link (S10). The page menu keeps Chrome's editing, link, image and media items; saving
  goes through Tarn's downloads. With developer tools on, the page menu's Inspect opens DevTools
  in a window of its own (no debugging port, S22) whose client shows only `devtools://` pages,
  cancels every web request it would make, refuses downloads and offers the links it opens in the
  inspected tab, where the user's click opens them.
- **Pop-up windows:** a clicked pop-up the site's rule and the block lists allow (`window.open`,
  `target=_blank`) is made by Chromium as a view of Tarn's own, hidden in the browser window, and
  the shell adopts it into a tab (`Event::Popup`, `WebView::adopt`), so it keeps `window.opener`
  for sign-in and payment windows; the events it sends before that wait in the `Popup`. It is
  never routed to another space, which would cost it the opener. A Lockdown view's pop-ups are
  offered instead: their page must not load before its service workers are bypassed. It is
  judged by the page that opened it (S10, downloads), takes the opener's Lockdown and filtering,
  and gets the page scripts from its next page on.
- **Page background:** Chromium's back-forward cache is off. With it, a navigation moves the page
  into a new view that Chrome paints in its theme's new-tab colour, so in dark mode every page
  without a background of its own showed on dark grey after a link or pop-up; pages still see the
  system's colour scheme. A page left is not kept in memory, so Back loads it again.
- **Page full screen:** a page's element full screen (a video) makes the window full screen with
  the page alone in it (`Event::FullScreen`) while Chrome's own bubble names the origin and Esc;
  leaving, Esc, another tab, a crash, or anything the shell must show (a prompt, the address
  field, find, a notice) restores the window and the tab, and a page cannot return to it while a
  prompt waits.
- **Downloads:** saved without a dialog to `~\Downloads` under a cleaned, unused name (device names
  and `<>"|?*` replaced), with a `Zone.Identifier` stream (zone 3, `HostUrl` without credentials)
  so SmartScreen treats the file as downloaded; never opened. File pickers use the system dialog.
  A download whose URL, first URL or page is on the malware list gets no path until the user keeps
  it at the same prompt as on macOS ("may harm your PC"); declined or quit, Chromium cancels it
  and removes its part. It waits paused. A host the user has not let through with "Visit Anyway"
  is still refused by the request filter before any request, as macOS's gate does.
  Chromium's PDF viewer is the only extension frame allowed.
- **Import:** profiles of Chrome, Edge, Brave and Firefox under `%LOCALAPPDATA%`/`%APPDATA%`, and
  Arc's in its app package's folder (`Packages\TheBrowserCompany.Arc_*`);
  history and bookmarks from all, cookies from Firefox. Firefox's cookies go into the profile's
  Chromium jar (`CookieManager::set_cookie`) under the macOS import's rules, shared as
  `engine::cookies::admit_imported`: nothing that overlaps a cookie the jar holds, the bridge's
  per-site caps, script-readable cookies capped at the bridge's lifetime and others at 400 days.
  Chromium browsers seal their cookies with DPAPI and app-bound keys Tarn cannot read, so their
  import copies history and bookmarks only and says so before it starts.
- **Shared with macOS:** rebindable shortcuts (canonical `cmd-…` keys,
  translated where GPUI binds them, `shortcuts::platform`; the Win key and macOS-only reservations
  stay out), download progress with Chromium's own pause and resume, sorted download folders, read
  aloud (WinRT speech synthesis), the share sheet and QR codes, site icons, Find in Tabs, the
  reader, page text in history, page snapshots and PDF (DevTools), picture in picture, the element
  picker, and Make Default (registers Tarn under HKCU, then opens Windows' default-apps settings;
  links arrive after `--single-argument`, which Chromium's own parser honours).
- **One Tarn per data folder** (`engine::instance`): a second launch hands its links to the
  running Tarn through `WM_COPYDATA` to a message-only window and exits, as a macOS app does. The
  taskbar jump list's New Window arrives the same way (`--dock-action N`).
- **Zap Element and site styles:** Chromium has no content rule lists, so the profile's style
  sheets and hidden elements become one style sheet (`site_styles_of`), put into each page at load
  start and into open pages whenever it changes (`restyle`); a zapped element can show for a moment
  while a page loads. Theme website styles take the same path, by host pattern; a restyle replaces
  the earlier sheet by its `data-tarn-site` marker, and `set_site_styles` never asks for views to be
  remade. The filter lists' element hiding (ad placeholders, cookie notices) takes the same path:
  `shield::lists::HideSheets` turns the rules `compile_cosmetic` gives WebKit into a generic style
  sheet, built once per list update, plus each page host's site rules and the generic rules a
  domain unhides, from a host index; the Shield's matcher carries them with the ads and annoyances
  switches, and a site where blocking is paused gets none. One CSS rule per selector, so a selector
  Chromium rejects drops only itself. With the bundled lists a page gets about 20,000 rules
  (≈0.8 MB, ≈0.6 ms to assemble; Chrome parses it in ≈10–15 ms). Unlike WebKit's rule lists, it
  reaches subframes only at their load start, through CEF's `Frame::execute_java_script` in the
  frame's own page world (each gets its own host's rules, by the top page's pause state at that
  load; a later pause or list switch reaches a subframe when it loads again, and a sheet sent as a
  frame swaps documents can miss the new one), is a
  `<style>` element the page can read, and a page shows its ads
  for a moment while it loads. A Lockdown view gets no sheet. The Shield card counts the requests
  each profile's Chromium blocked. A link
  handed to an elevated Tarn is refused by Windows (UIPI) and gets the "already running" alert.
- **Lockdown:** a Lockdown site gets a Chromium view of its own, made without JavaScript, whose
  requests the filter lets through under Lockdown's policy (HTTPS only, no local network, no
  site styles; service workers bypassed). A main-frame navigation across the Lockdown boundary is
  judged first (S10, the list gate, local network), then the shell rebuilds the view and loads the
  URL again: as typed only when it was (`ShieldLevelChanged`), otherwise as the page's link
  (`RebuildForLink`). At most four rebuilds in ten seconds, then a notice. A rebuilt view starts a
  new back list, and a form's POST across the boundary arrives as a plain GET. Other views still
  cancel Lockdown sites' requests.
- **Page scripts:** YouTube's ad script (where blocking is not paused) and Netflix's visibility
  script (every view but Lockdown's) run in the page's main world from document start, registered
  with `Page.addScriptToEvaluateOnNewDocument` before a view's first load; a new YouTube script
  or its switch reaches each open view's next page (`view::rescript`). Cross-site frames in another
  renderer (an embedded YouTube player) do not get them. Strict's referrer trimming has no script:
  the request filter drops the `Referer` of every cross-origin request a Strict page sends,
  navigations included, which Windows views decide per page rather than per view; Chromium's own
  `strict-origin-when-cross-origin` caps what `document.referrer` can show to the origin.
- **Tab fade and menus:** the page's child window fades through `WS_EX_LAYERED` and is plain
  again once the fade ends. The toolbar's menu shows the menu bar's colour dots and site icons as
  menu bitmaps (PNG data read as an icon image); system symbols have no Windows counterpart.
- **Pop-ups:** GPUI draws transparency only with DirectComposition, which would cover Chromium's
  child windows, so each pop-up is clipped to its card's rounded rectangle
  (`Host::round_corners`); the card's shadow, and the downloads panel's caret, are not drawn on
  Windows.
- **Page corners:** each page's child window is clipped to the card's rounded corners
  (`SetWindowRgn`), set again whenever its pixel size, DPI or the theme's radius changes.
- **Gestures:** the Gestures setting turns Chrome's own two-finger swipe back and forward on a
  precision touchpad on or off through its Local State preference
  `settings.a11y.overscroll_history_navigation`, which holds for every view at once (the setting
  is app-wide anyway). CEF refuses to change it while running, so Tarn writes it into Local State
  after Chromium shuts down and it holds from the next launch. Chrome shows its arrow instead of WebKit's sliding page, and swipes only
  when the page cannot scroll further sideways. Pinch to zoom stays on when gestures are off:
  Chrome has no run-time switch for it. GPUI reports a touchpad's two-finger scrolling as wheel
  lines without touch phases, so the sidebar's and toolbar's swipes take a burst of them as one
  gesture, started after a pause of 180 ms and ended at the next. Only fractions of a line count,
  which a touchpad sends and a notched wheel does not; a free-spinning or fine-step mouse wheel
  can still swipe.
- **Updater** (`platform::updater`, release builds only): at launch and then daily (wall-clock,
  an hour after a failed check) while automatic checks are on, and on Check for Updates…, Tarn
  fetches `windows-update.json` and its `.sig` from the latest GitHub release next to the feed in
  `packaging/updates.conf` (embedded at compile time with its Ed25519 key). The signature over the
  feed's exact bytes is checked before parsing; the feed is capped at 64 KiB, the installer at
  512 MiB, and the installer must come over https from this repository's release downloads. A feed
  listing the running version or a later one counts as verified for the staleness gate; a critical
  later one starts the two days. Only a later version is downloaded, into `<data>/updates/<version>`
  (kept across launches while it still verifies; older versions' folders go), and only after its
  length, SHA-256 and Ed25519 signature check out. A toast, Settings › General and the Tarn menu
  offer Restart to Update (Settings › General also has Check Now); otherwise a
  normal quit installs it while automatic checks are on, as Sparkle does. On quit Tarn hashes the
  file again through a handle that bars writes and starts it with
  `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART` (plus `/TARNUPDATE` after Restart to Update, which makes
  the installer start Tarn again). Every release must carry `windows-update.json` and its `.sig`,
  also engine-only releases (`docs/release.md`), or Windows checks fail: new installs open no
  page, and existing ones close Chromium seven days after their last verified check. v0.1.0,
  published without the feed, showed that: a new install opens no page. `release.yml`'s
  `sign` job signs a feed for the unsigned installer (`scripts/windows-feed.sh`). Updating through a feed is
  not yet tried against a real release.
- **Release:** CI builds and tests Windows on every push (`ci.yml` `windows` job). A `v*` tag adds
  the unsigned zip and an unsigned per-user Inno Setup installer (`packaging/windows/tarn.iss`,
  `scripts/installer-windows.sh`) to the release; the `sign` job signs the installer and the update
  feed `windows-update.json` with the Sparkle Ed25519 key in CI (`docs/release.md` "Signing in
  CI"). No Authenticode certificate is set up; `scripts/sign-windows.ps1` (certificate by
  thumbprint or Azure Trusted Signing, SHA-256 with RFC 3161 timestamp, `signtool verify /pa`) signs
  by hand once one is. Settings › General › Nightly builds switches to the `nightly` release's feed
  (`docs/release.md` "Update channels"). The installer, Start menu entry and uninstaller use `packaging/windows/tarn.ico`
  (from `packaging/Tarn.icns`); `tarn.exe`, CEF's launcher, still carries CEF's icon and version
  resources: replacing them needs a resource editor step the packaging does not have yet.
- **Checked by hand:** pages load and render (example.com, DuckDuckGo, Wikipedia), typing in
  pages and in the address field with suggestions, Shield's per-profile request filter, tab
  titles, shortcuts from page focus, new and switched tabs, back, settings, window move,
  maximize and close, session restore, private space in memory, PDF, a download with its zone
  stream, library search by keyboard, importing history from Edge.

**Open, in order:**
1. Classic (Safari or Chrome) extensions: a gap until CEF can scope them (§8); Tarn extensions run
   (`engine::tarn_ext` over the in-process DevTools channel, pages in `engine::ExtensionPage`).
   Translation stays macOS-only: Windows has no on-device
   translation. The memory panel and budget measure each tab's main-frame renderer, which reports its own
   process id at each main-frame load (`RENDERER_PID`); Tarn takes it only when that process runs Tarn's own
   executable. A page whose renderer sends no report keeps the last one until the next.
2. ARM64: no build or installer yet.
3. Data: a store wiped while Chromium has it open is emptied of cookies, cache and credentials at
   once and its folder goes before CEF starts at the next launch (`<data>/Chromium/WIPED`);
   site storage stays until then. Delete Everything on a profile that keeps its store clears
   cookies, cache and the storage of the sites its open tabs show; storage of other sites stays
   (no CEF call clears all origins). Clear Site Data reaches storage only through an open tab of
   the same store. Request contexts of closed private spaces stay in memory until quit. A profile
   folder Chromium cannot write leaves its tabs blank without a notice.
4. Profiles that share a store have their own Shield in their tabs; requests of service workers
   follow the Shield of the profile that opened the store first.

## 1. Approach

- **Engine:** WebView2 (Evergreen runtime, Chromium) inside `crates/engine` behind
  `#[cfg(windows)]`, next to the existing `#[cfg(target_os = "macos")]` modules. Binding:
  `webview2-com` (Microsoft's COM interfaces generated through the `windows` crate), not `wry`, for
  the same reason as on macOS: we need request filtering, per-profile environments and download
  events that `wry` hides. `unsafe` stays in `crates/engine` and `crates/platform`, as on macOS.
- **No shared trait up front.** The public surface the app uses today (`WebView`,
  `DataStore`, `Host`, `LevelFor`, `Popup`, `open_external`, `reveal_in_finder`, `app_active`,
  `reduce_motion`, `option_held`, `Event`) gets a
  Windows implementation with the same names and signatures. A trait is introduced only if the two
  implementations diverge in a way `cfg` cannot hide. `nav_policy`, `lifecycle`, `downloads`
  (name sanitising) and `events` are already platform-neutral.
- **Runtime:** require the Evergreen WebView2 runtime (preinstalled on Windows 11 and current
  Windows 10). Microsoft updates it, which matches the WebKit "patched by the vendor" row. A
  fixed-version runtime is rejected: we would own Chromium security updates.
- **Targets:** Windows 11 and Windows 10 22H2 while Microsoft supports it, x64 and ARM64.

## 2. GPUI on Windows

GPUI has a Windows backend (DirectX 11 renderer, used by Zed on Windows since 2025). Questions to
answer in a spike before any port work:

1. GPUI exposes the window `HWND` through `raw-window-handle`, and a WebView2 controller can be
   parented to it with its bounds set from GPUI layout (same model as the macOS subview).
2. Focus and keyboard: WebView2 takes focus on click, returns it to GPUI on ⌃L / command bar,
   and accelerators reach GPUI while the page has focus (`AcceleratorKeyPressed`).
3. IME in the command bar (GPUI's TSF support) and in the page.
4. Window backdrop: Mica/Acrylic via `DwmSetWindowAttribute` replaces macOS blur; a plain graphite
   background is acceptable if GPUI's surface does not compose with it.
5. The pinned GPUI version builds on Windows with the workspace's `runtime_shaders` setting or its
   Windows equivalent.

Native child windows draw above GPUI content on Windows too (airspace), so the macOS rule of
hiding or shrinking the web view under overlays carries over unchanged.

If item 1 fails, the fallback is a sidebar in its own WebView2 with the stricter S1 rules.

## 3. Mapping of macOS-specific pieces

| macOS | Windows | Notes |
|---|---|---|
| `WKWebView` | `ICoreWebView2Controller` + `ICoreWebView2` | One controller per live tab; same lifecycle and hibernation rules. |
| `WKWebsiteDataStore dataStoreForIdentifier:` per profile | One `ICoreWebView2Environment` per profile with its own user data folder `%LOCALAPPDATA%\Tarn\Profiles\<data_store_uuid>` | Separate folders give separate browser processes, cookies, cache, permissions. Alternative inside one environment: `ICoreWebView2ControllerOptions::ProfileName`; chosen per spike, folders preferred for wipe-by-delete. |
| `nonPersistentDataStore` (private) | `ControllerOptions::IsInPrivateModeEnabled = TRUE` in a dedicated environment | InPrivate still uses a temporary folder on disk that WebView2 deletes; the rule that the private profile never touches disk cannot be met strictly, see §8. |
| `WKContentRuleList` (Shield, private-network rule, HTTPS upgrade) | `AddWebResourceRequestedFilter("*", ALL)` + `WebResourceRequested` handler that evaluates the `adblock-rust` engine in-process | No compile step and no per-list limit; cost is a callback per request, so the engine must answer in microseconds (bench gate). HTTPS-only: cancel and re-navigate main-frame HTTP; subresources blocked. |
| `interactionState` | No equivalent | The back list cannot be restored; only the URL is. Documented loss. |
| Navigation delegate (`decidePolicyForNavigationAction`) | `NavigationStarting` (cancel), `FrameNavigationStarting`, `NewWindowRequested` (popups, gesture via `IsUserInitiated`) | `nav_policy::decide` called from the same places; `Source` mapping unchanged. |
| JS dialogs, permissions, HTTP auth | `ScriptDialogOpening` (deferral, tab-modal GPUI prompt), `PermissionRequested`, `BasicAuthenticationRequested` | Origin shown in every prompt (S5). |
| TLS errors | `ServerCertificateErrorDetected` | Never `AlwaysAllow`; no bypass at any level (S7). |
| Content-process crash reload | `ProcessFailed` | Same rate limit. |
| Script message handlers (none) | `AreHostObjectsAllowed = FALSE`, `IsWebMessageEnabled = FALSE`, no `AddHostObjectToScript` | S1 holds. Also `AreDevToolsEnabled` only in the dev profile; `AreDefaultContextMenusEnabled` reviewed. |
| Downloads + `com.apple.quarantine` | `DownloadStarting` → our sanitised `unique_path`; Mark-of-the-Web: write `Zone.Identifier` ADS (`ZoneId=3`, `ReferrerUrl`, `HostUrl`) | Also call `IAttachmentExecute::Save` so Defender/SmartScreen scan; never auto-open. Same `is_dangerous` list plus Windows extensions (`.exe .msi .bat .cmd .ps1 .lnk .scr .hta .js .vbs .appx .msix`). |
| `open_external` / `reveal_in_finder` | `ShellExecuteExW` (after prompt) / `SHOpenFolderAndSelectItems` | Never pass untrusted strings to a shell command line. |
| Sparkle 2 (EdDSA, appcast) | WinSparkle (EdDSA support, appcast from GitHub Releases) or a small Rust updater verifying an Ed25519 signature over an MSIX/installer, then Authenticode check | Prefer WinSparkle for parity of the appcast and keys; downgrade protection required either way (S4). |
| Keychain (API token, optional sync key) | Credential Manager (`CredWriteW`, `CRED_TYPE_GENERIC`, per user) | Data is DPAPI-protected per user; same "only here" rule and grep test. |
| XPC helper for the Claude CLI | Separate `cli-bridge.exe` reached over a named pipe `\\.\pipe\tarn-cli-<user SID>-<random>` created with an ACL allowing only the current user SID and `PIPE_REJECT_REMOTE_CLIENTS`; server checks the client process image path and Authenticode signer | Same S17 rules: fixed args, no shell, stdin prompt, minimal env, timeout, output cap. |
| App Sandbox | AppContainer / Less Privileged AppContainer for the shell: feasibility spike | WebView2 supports running under AppContainer only in limited configurations; if the shell cannot run in one, fall back to a normal medium-IL process plus the WebView2 renderer sandbox, and record the gap in S12. |
| `security-scoped bookmark` for the sync folder | Plain path chosen through `IFileOpenDialog` | No sandbox, so no bookmark needed unless AppContainer is adopted (then a broker or `StorageFolder` capability). |
| `LSFileQuarantineEnabled`, hardened runtime, notarisation | Authenticode signing, SmartScreen reputation, CFG/CET (`/guard:cf`, `/CETCOMPAT`) linker flags, ASLR/DEP defaults | |
| File permissions `0600`/`0700` | DACL on `%LOCALAPPDATA%\Tarn` granting only the user and SYSTEM, inheritance off | `store::open` gets a `cfg(windows)` branch. |
| NSFilePresenter (sync change notifications) | `ReadDirectoryChangesW` on the sync folder | Poll fallback unchanged. |

## 4. Crates

- `crates/engine`: `webview_windows.rs`, `data_store_windows.rs`, `delegates_windows.rs` behind
  `cfg(windows)`; Windows-only deps under `[target.'cfg(windows)'.dependencies]`, exact pins.
- `crates/platform`: `updater_windows.rs`, `credentials_windows.rs`, `helper_client_windows.rs`.
- `crates/shield`: unchanged rule logic; add an in-process matcher API used by
  `WebResourceRequested`.
- `crates/app`: keybindings map ⌘ to Ctrl; menus via GPUI.
- `deny.toml`: add `x86_64-pc-windows-msvc` and `aarch64-pc-windows-msvc` to `graph.targets`, and
  any licence the `windows` crates need (they are MIT OR Apache-2.0, so likely none).

## 5. CI

- New job on `windows-2025` runners (x64), actions pinned by hash like the existing jobs:
  `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace
  --locked`, both for `x86_64-pc-windows-msvc`; ARM64 cross-compile check
  (`cargo check --target aarch64-pc-windows-msvc`).
- `cargo deny check` gains the Windows targets so Windows-only dependencies are audited.
- Integration tests that need a WebView2 runtime run on the runner (runtime preinstalled on
  Windows Server 2025 images; job fails if missing instead of downloading at test time).
- The egress test gets a Windows variant using a recording proxy and `netstat -ano`
  for the no-listener check.

## 6. Signing and release

- Authenticode with an EV or OV certificate on a hardware token, or Azure Trusted Signing.
  None is set up yet; the Sparkle key signs the update feed in CI (S4).
- Package: a per-user Inno Setup installer, signed; update payloads additionally carry the
  EdDSA signature checked by the updater (built, §0 "Release").
- SBOM attached to Windows releases: `Tarn-<tag>-windows-x64.cdx.json`, the crates of
  `crates/app-dll` for `x86_64-pc-windows-msvc` (built, `docs/release.md`).

## 7. Test plan

- Spike checklist from §2, plus the per-profile isolation test re-run against a local server.
- Table tests for `nav_policy` already run on Windows unchanged.
- Delegates: the macOS local test pages reused (popups without gesture, dialogs flood, HTTP
  auth, TLS error page, crash reload via `ICoreWebView2::get_BrowserProcessId` kill).
- Downloads: `../../x`, double extensions, Windows reserved names (`CON`, `NUL`, `COM1`, trailing
  dots and spaces), `Zone.Identifier` present with `ZoneId=3`.
- Shield: tracker test page, zero hits on the local counting server; private-network rule
  including `0x7f.1`, `[::ffff:127.0.0.1]` and `localhost.` forms.
- Credentials: token absent from all files and logs (grep test).
- Named pipe: a second process under another user, and a non-signed process under the same
  user, both fail to connect.
- Performance budgets measured separately for Windows; numbers may differ, gates
  are set after the first measurement.

## 8. Open risks

- **Private profile on disk:** WebView2 InPrivate keeps temporary data in the user data folder
  during the session. The private profile's "never touches disk" rule cannot be met strictly; mitigation is a dedicated folder deleted on
  close and at next launch. Needs a maintainer decision to accept this deviation.
- **No `interactionState`:** CEF and the DevTools protocol can read a tab's back list but not set
  one. Tarn saves the list's URLs instead (`view_policy::SavedHistory`, at most 50 on each side,
  web pages only) and, after a tab sleeps, is reopened or is restored at launch, steps through them
  itself: Back and Forward past Chromium's own list load the saved page in place of the current
  entry, as the user's own load. Scroll positions and form contents are not kept, and a page left
  that way while Chromium has forward entries of its own comes after them.
- **Hung pages:** the hung-page card's Reload builds a new view; Chromium may place it in the same
  renderer process as the hung page when the site matches, and CEF offers no call to end one view's
  renderer, so such a page can stay hung until it lets go.
- **AppContainer:** may not be feasible with WebView2 and GPUI's DirectX device; then S12 is
  weaker on Windows than on macOS.
- **Request filtering cost:** `WebResourceRequested` for every request is slower than compiled
  WebKit rules; a slow matcher would hurt page loads.
- **Classic extensions (M8), a gap; Tarn extensions run.** Packages for Tarn's own runtime
  (`browser_specific_settings.tarn.runtime`) work on Windows as on macOS: content scripts in an
  isolated world per install through new-document scripts and a binding of that world only (cross-site
  frames attached as targets of their own), rules
  in the request filter, pages from `tarn-extension://` registered per request context, all through
  the in-process DevTools channel (S22). Their popup is a Chromium view placed under the toolbar
  button that closes on a second click, not a native popover. For full extensions: CEF 156 (Chrome
  style) has no extension API. A spike found that
  `--disable-extensions-except` / `--load-extension` load unpacked MV3 extensions, run their
  content scripts and open their popups in a separate browser, but only at Chromium's start and
  for every request context at once: an extension cannot be limited to one profile, kept out of
  private spaces, or asked for per site (CEF has no host-permission callback; Chromium resets
  hand-written permission prefs), and `Extensions.loadUnpacked` needs a remote-debugging pipe,
  which S22 refuses. Shipping that would break S18 (per-profile installs, per-site prompts, none in
  private or Lockdown), so Settings › Extensions says only Tarn extensions run on Windows until CEF
  offers per-context loading and host permissions.
- **GPUI Windows maturity:** fewer users than macOS; IME and accessibility (UI Automation) gaps
  may block release.
- **Code-signing cost and SmartScreen reputation** for an unfunded open-source project.
