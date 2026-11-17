# Tarn for Linux

The Linux desktop app (plan `docs/nsh-plan.md` P2): a Native Shell (NSH 0.5.0) shell in Rust over
`browser-core`, with Tarn's own `tarn.webview` component on the distribution's WebKitGTK 6.0. It is
its own Cargo workspace, so NSH stays out of the root workspace and `Cargo.lock`.

## Layout

| Path | What |
|---|---|
| `src/webview/` | `tarn.webview` (contract v3, `docs/mobile-plan.md` §2): `mod.rs` the component (its navigation verdicts, bursts and origins are `engine::view_policy`), `scripts.rs` the v3 script runs, site styles and zaps, `session.rs` one `NetworkSession` per profile store, ephemeral and burner ones, downloads, `filters.rs` Shield rules through `UserContentFilterStore` (groups from `shield::groups`) |
| `src/data.rs` | `browser_core::WebsiteData` for Linux: a profile's store key (`<uuid>` or `private:<session>`); website data lives in `$XDG_DATA_HOME/tarn/webkit/<uuid>/{data,cache}` |
| `src/browser.rs`, `src/snapshot.rs` | The window's state over `browser_core::{Core, tabs::Model}` and the view model the UI renders; no NSH or GTK types, host-tested |
| `src/ui.rs`, `src/ui/panels.rs` | The NSH desktop layout |
| `src/controls.rs` | The desktop's text styles, rows and panels over `tarn_nsh_shared::controls` (desktop density); theme and icons are `tarn_nsh_shared::{theme, icons}` (`../nsh-shared`) |
| `src/input.rs` | The search-engine setting and history suggestions (with full-text matches); parsing is `browser_core::command` |
| `src/browser/{pages,site,spaces,tools}.rs` | Reader, page text, Find in Tabs, Shield counters, downloads; site CSS, dark mode, zaps; schedules, burner tab, routing, snapshots, space commands; copy link, QR code, Reading List |
| `src/shortcuts.rs`, `src/memory.rs`, `src/speech.rs` | Rebindable shortcuts (`keymap.tarn::<Action>` settings, NSH keystroke text); per-tab memory and CPU from `/proc`; read aloud through `spd-say` |
| `src/ui/{tools,library,settings}.rs` | Toasts, reader, command rows, QR code; space, memory and Reading List panels; Settings |
| `packaging/` | `.deb`, `.rpm`, AUR `PKGBUILD`, desktop entry |
| `scripts/` | Container build, Xvfb environment, test server, scenarios, display tests |
| `evidence/` | Exit-check results (P2) |

## Contract v2 on Linux

Linux's additions (`levels["*"]`, `lists`, `download.url`/`quarantined`, `external.origin`) are part of
contract v2 (`docs/mobile-plan.md` §2), which also lists what stays different. Linux-specific behaviour:
- JavaScript follows the view's level. A page's navigation into a site with JavaScript off turns it
  off before the request; a clicked link there gets a rebuilt view; each main-frame commit whose
  site needs JavaScript off while the view has it on is stopped and rebuilt (`blocked {reason:
  "rebuild"}`). A frame of such a site turns JavaScript off for the rest of its page; the next
  main-frame commit to a site with JavaScript on turns it back on and reloads that page; a form
  submission turns it back on before its request, so a POST is never sent twice.
- HTTPS-only is checked at navigation time: a plain-HTTP main-frame load to an HTTPS-only site is
  upgraded when the user started it (typed, a clicked link, a history step) on any port, and blocked
  otherwise; an upgrade that comes back to HTTP is blocked (`blocked {reason: "https_only"}`).
- WebKitGTK does not say whether a navigation targets the main frame. `data:` and `about:`
  navigations without a user gesture are treated as frames and allowed; all others get the
  main-frame checks.
- `zoom` scales the whole page (desktop behaviour); `download` files get `user.xdg.origin.url`.
- `download` events go to the app by download id (`session::route`), not through the view, so a
  download keeps running and reporting after its tab closes, is evicted or its burner is burned (as on macOS).
- `print` opens the GTK print dialog; `pdf {path}` prints the page to that absolute path through
  WebKitGTK's print operation (GTK's file printer) and reports the file on the same route as a
  finished `download`, so it is listed like one.

## Features from the macOS app

Same store schema and setting keys as macOS (`block_annoyances`, `history_text`, `sort_downloads`,
`keymap.tarn::<Action>`):
- tabs: close duplicate tabs, toasts with Undo, copy link and clean link (tracking parameters counted),
  QR code (`engine::qr`, drawn by `tarn_nsh_shared::qr`), all three in the toolbar's share menu with Save
  Page as PDF (through the save dialog, listed in Downloads) and Print… (Linux has no system share
  sheet, so the menu has no More…; `docs/nsh-plan.md` §5), burner tab (its own ephemeral `NetworkSession` per `burner:<32 hex>` id, released when
  burned), Reading List (the `Reading List` bookmark folder);
- bookmarks (`browser_core::bookmarks`): Bookmark Tab… (`Ctrl+Shift+D`, Tab panel, command rows)
  opens the Bookmarks panel on the new bookmark's name, Bookmark All Tabs (New Tab row, command rows)
  files every tab into a dated folder; the Bookmarks panel (`Ctrl+Alt+B`, Manage Bookmarks, and
  from History and the Reading List) lists recent bookmarks and the folder tree, searches titles and
  addresses, and renames, moves, deletes and adds folders, in place of the macOS Bookmarks menu and
  Library › Bookmarks;
- tab groups: a card per group with its colour, collapse and close, New Tab Group in the New Tab row
  (`Ctrl+Alt+G`), Add to Group and Remove from Group in the tab panel, rename, colour, Ungroup and
  Close Group in the group's panel, a dropped tab joins the group it lands in; Clean Up in the New
  Tab row (`browser_core::cleanup`); Settings › Tabs closes unused tabs into the `Archived Tabs`
  bookmark folder (`Model::archive_unused`, every 10 minutes and on launch), listed in the
  Archived Tabs panel next to the Reading List and as a folder in the Bookmarks panel;
- spaces: schedules by weekday and hour with the one-hour manual hold, snapshots (save, restore in a
  new or this space, delete), "Always open in" rules, switch to and move tabs between spaces from
  the command rows of the address field;
- site panel: dark mode and site CSS (`styles` prop), Zap Element (`pick`, `zaps` prop), hidden
  elements with Undo;
- reader view (`readArticle`) with text size and width; Read Aloud through speech-dispatcher's
  `spd-say`, shown only when it is on `PATH` (Read Aloud / Stop, no pause: `spd-say` has none);
- full-text history (`readText`, off by default) feeding address suggestions; Find in Tabs
  (`Ctrl+Shift+F`, `findText`);
- site icons (`readIconLinks`, `fetchIcon`): fetched inside the tab after each https load, kept per profile
  by `engine::favicon::Tracker`, stored with `Store::set_favicon` and drawn wherever a site's letter
  tile was (tiles, tab rows, split panes, history, bookmarks, suggestions, site and share popovers);
- Shield: cookie-banner lists switch, Block YouTube ads (the YouTube page script and ad-slot rules,
  `youtube` prop), the Netflix visibility script in every view but Lockdown's, the Shield card (front and stamps side, no 3D tilt in NSH)
  with the last seven days and the sites protected most; counters from `shielded` events;
- downloads: `id`, progress (percentage next to the downloads button, sizes in the panel), cancel,
  sorting into Downloads subfolders; a download whose request, response or page host is on the
  installed malware set (Block malware sites on) is `flagged` and waits, unsaved, for Keep or Delete
  in the Downloads panel (`downloadKeep`); no pause or resume (WebKitGTK cannot);
- keyboard shortcuts rebound in Settings (typed as `Ctrl+Shift+Y`; Linux has its own action list);
- desktop input: Up and Down pick a suggestion row of the address field or the start card and Enter
  runs it; a right click (or Shift+F10) on a tab or tile opens the Tab panel for that tab; `Ctrl+G` and
  `Ctrl+Shift+G` step through the find bar's matches (fixed keys); icon buttons have tooltips, placed
  beside the toolbar's buttons because one below them would be hidden behind the web view; Clear
  History, Clear Website Data and a snapshot's Delete ask once more before they run;
- split view (`browser_core::split`, session only): two to four panes with headers, dividers that
  drag and even out on a double click, drops on a sidebar row's middle, a pane header or a page;
  a click in a pane's page focuses it through `pressed` (a Linux addition the component emits from
  a click it does not claim), as WebKitGTK keeps the page's pointer events from the shell;
- memory panel and heavy-tab marks: WebKitGTK gives no process id, so the component reports
  `process {pid}` (a Linux addition) when exactly one new `WebKitWebProcess` below the app appears
  between a load's start and its commit; tabs it cannot tell apart are "Not measured".

Not ported: on-device translation and the startup trace (macOS-only), the live page peek (optional,
skipped).

## Build and test (Docker; no Linux desktop is needed)

```sh
docker build -t tarn-linux-fedora -f linux/docker/fedora.Dockerfile linux/docker
linux/scripts/cargo.sh test --locked            # host tests (Fedora 43 container)
linux/scripts/cargo.sh clippy --all-targets --locked -- -D warnings
linux/scripts/cargo.sh build                    # debug binary in the volume tarn-linux-p2-target
linux/scripts/test-display.sh                   # WebKitGTK tests under Xvfb (ephemeral store); runs scripts/display-tests.sh in the container
linux/scripts/env.sh && linux/scripts/app.sh    # Xvfb + test server + the app; WM=openbox for a WM
linux/scripts/scenario.sh <name> <address> [x y]...   # drives it, writes evidence/<name>.{txt,png}
linux/packaging/build-all.sh deb-debian|deb-ubuntu|rpm|arch
```

CI (job `linux` in `.github/workflows/ci.yml`) runs clippy, the host tests and `scripts/display-tests.sh` in a
`fedora:43` container. GTK runs on one thread per process, so the script runs each display test in a process
of its own. WebKitGTK crashes at process exit after the display test has passed (it disposes its
default context on the main thread, libtest ran the test on another); the script then accepts only exit status 133 or 134
(SIGTRAP or SIGABRT) after a passing test result.

Test-only settings, never in the app or packages: `WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1`
(bubblewrap does not run in Docker), `LIBGL_ALWAYS_SOFTWARE=1`, `TARN_TEST_FILTER_KEY` (debug
builds only) and the test certificate `scripts/trust-test-cert.sh` adds to the container's trust
store.

## Shared code

Moved out of this crate, with the macOS app using the same code where it had a copy:
- `nsh-shared/` (crate `tarn-nsh-shared`): theme tokens with the test against
  `crates/ui/src/theme.rs`, the Lucide icons and brand mark, and the generic controls (pill,
  segmented control, switch row, icon button, group, surface card), shared with the mobile app;
- `engine::view_policy`: site levels, navigation and popup verdicts, bursts, the zoom range
  (`engine::delegates` uses its bursts and title cut);
- `shield::groups`: the bundle-to-rule-list conversion and the debug signing key (`crates/app` uses it);
- `browser_core::command`: input parsing, `engine_key`, the address label (`display_host`, the text of
  the macOS label `address`); `browser_core::prompts`: the prompt model and what may be remembered (the mobile shell
  uses both). A crashed page reloads once and stays until a page finishes loading, as on macOS.

Still per shell: `src/browser.rs` and the mobile `Shell` keep their own page, tab and event handling
(different effect models and store-id schemes; merging them is a larger change), and the panels,
which differ between the desktop layout and the mobile sheets.
