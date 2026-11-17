# Port backlog

Features built for macOS only that the other clients still need (`AGENTS.md`, "macOS-only mode").
Newest entry first. Delete an entry once every client is ticked.

## Template

```markdown
### <Feature name> (<YYYY-MM-DD>, <commit>)

What it does: <one or two sentences, from the user's point of view>.
Shared API: <crate::module::fn / type the clients call>.
macOS reference: <crates/app/... files>.

- [ ] Windows — <what is missing, e.g. WebView2 hook for X>
- [ ] Linux — <linux/src/... to change>
- [ ] iOS — <mobile/app/... or mobile/ios/...>
- [ ] Android — <mobile/app/... or mobile/android/...>
```

## Open

### Tab, accent and motion settings (2026-11-17)

What it does: "New tab opens" (command card or start page), "After closing a tab" (next or
previous), Touchpad gestures, the accent colour and Reduce motion, stored under the macOS keys.
Shared API: `browser_core::prefs` (`NEW_TAB_START_PAGE`, `CLOSE_SELECTS_PREVIOUS`, `GESTURES`,
`REDUCE_MOTION`, `accent_choice`, `accented`); `tarn_nsh_shared::motion::{set_reduced, reduced}`;
the Accent and Reduce motion rows in `tarn_nsh_shared::appearance` show where
`Host::accent_and_motion` is true.
macOS reference: `crates/app/src/settings.rs` (`appearance`, `tabs`), `crates/app/src/prefs.rs`.

- [x] Windows — same code in `crates/app`
- [x] Linux — `linux/src/ui/settings.rs`, `linux/src/ui.rs` (`new_tab`, `go`), `linux/src/browser/themes.rs`
- [ ] iOS — `mobile/app`: `accent_and_motion` in `ui/appearance.rs`, `prefs::accented` and
  `motion::set_reduced` in `themes.rs`, `motion::reduced` for its own `reduce_motion()` checks
  (`ui/mod.rs`, `ui/bars.rs`),
  `CLOSE_SELECTS_PREVIOUS` in `features.rs` (`close_neighbour`); new-tab and gesture rows where they apply
- [ ] Android — same as iOS

### Clean Up (2026-11-16)

What it does: the Clean Up button in the New Tab row sorts the space's loose tabs into tab groups by what each page
is (on this device, from known sites and title words) and archives tabs unused for longer than
Settings › Tabs allows.
Shared API: `browser_core::cleanup::{plan, apply, category}`; archiving through
`Model::archive_unused`.
macOS reference: `crates/app/src/browser/context.rs` (`clean_up_tabs`), `crates/app/src/sidebar.rs` (`ephemeral`).

- [x] Windows — same code in `crates/app`
- [ ] Linux — `linux/` has no tab groups or archiving yet; both first, then the row in `linux/src/ui.rs`
- [ ] iOS — `mobile/app` has no tab groups or archiving yet
- [ ] Android — same as iOS

### Tile grid (2026-11-16)

What it does: the tiles above the pinned tabs arrange themselves: one row while they fit, then as
few rows as fit, filled evenly (six tiles in a 256 pt sidebar make 3 × 2); Add to Tiles and
Remove from Tiles in a tab's menu.
Shared API: `browser_core::tabs::tile_columns`, `browser_core::tabs::Model::tile_toggle`.
macOS reference: `crates/app/src/sidebar.rs` (`tiles`), `crates/app/src/browser/context.rs` (`Cmd::ToggleTile`).

- [x] Windows — same code in `crates/app`
- [x] Linux — `linux/src/ui.rs` (`tiles`), `linux/src/ui/panels.rs` (This Tab panel)
- [ ] iOS — the tab sheet in `mobile/app/src/ui/panels.rs` shows every tab as a card and has no pinned tabs or tiles yet; tiles need pinning on mobile first
- [ ] Android — same as iOS

### Background tab memory budget (2026-11-15)

What it does: once a minute, awake background tabs whose web processes together hold more than
1 GB (split equally between windows) sleep, the heaviest first; a tab playing media, used in the
last minute (a pinned tab: 30 minutes), or in the split stays awake. Off when "Sleep background
tabs after" is Never.
Shared API: `engine::lifecycle::over_budget`, `engine::lifecycle::BACKGROUND_BUDGET`,
`engine::lifecycle::budget_idle`.
macOS reference: `crates/app/src/tabs.rs` (`sleep_idle_tabs`).

- [x] Windows — same code in `crates/app`, with each tab's main-frame renderer as it reports itself (`docs/windows-plan.md`)
- [ ] Linux — call it from the sample tick in `linux/src/ui.rs` with the costs of `linux/src/memory.rs`; needs a playing-media signal (`is-playing-audio`) in `linux/src/webview` first, and Linux has no tab sleeping timer yet
- [x] iOS — gap: an app cannot read the WebContent process's memory (`docs/mobile-plan.md`)
- [x] Android — gap: an app cannot read the renderer process's memory (`docs/mobile-plan.md`)

### YouTube ad script and Netflix visibility script (2026-11-15)

What it does: on YouTube the ad placements are stripped from the page's own data before the
player reads them (Settings › Privacy, block YouTube ads); on Netflix the page always reports
itself visible, so playback does not pause when the window loses focus.
Shared API: `browser_core` `youtube_script` (the script the app builds), `engine::page_scripts::ALWAYS_VISIBLE`;
both run at document start in the page world, the YouTube one only where blocking is not paused
and never under Lockdown.
macOS reference: `crates/engine/src/data_store.rs` (`install_scripts`); Windows:
`crates/engine/src/chromium/view.rs` (`page_script_for`, `rescript`).

- [x] Windows — Chromium `Page.addScriptToEvaluateOnNewDocument`
- [ ] Linux — document-start page-world user script through WebKitGTK's user content manager in `linux/src/webview`
- [ ] iOS — page-world `WKUserScript` in `mobile/ios`
- [ ] Android — `WebViewCompat.addDocumentStartJavaScript` in `TarnWebView` (page world)
