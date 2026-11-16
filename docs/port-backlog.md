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

### Split view with up to four tabs, drag to split (2026-11-16)

What it does: a split shows two to four tabs of a space (two side by side, three with the right
column stacked, four in a grid; Rotate Split Layout stacks them instead: two one above the other,
three with one on top). A window keeps any number of splits for the session and a tab is in one at
most: selecting a tab outside hides the split and lets its tabs sleep like any other, selecting
one of its tabs shows it again with that pane focused. A closed tab leaves its split, the focus
goes to the neighbouring pane, and a split left with one pane ends. Each pane has a header above
its page (site icon, title, host, Remove from Split View, Close Tab): a click focuses the pane, a
right click opens the tab's menu, dragging it onto another pane swaps the two. The column and the
row divider drag; a double click evens the panes out. Dropping a tab on the middle of another
tab's row, or on a pane, splits them, adds it to that split or swaps two panes of one split, and
the row names which; a full split takes no fifth tab and is never replaced. The tabs of a split
carry an edge in the sidebar, in the accent for the split on screen. The tab menu has a Split
View submenu (Open in or Add to Split View, Remove from Split View, Rotate Layout, Close Split
View). Commands: Focus Next Pane ⌃⌘], Focus Previous Pane ⌃⌘[, Swap Panes ⌃⌘S, Rotate Split
Layout ⌃⌘R, Remove Tab from Split View ⌃⌘W, Close Split View (no key; ⌃⌘N opens or closes).
A blocked page or a JSON page fills its own pane only. In a burner, which has one tab, opening a
split says that it needs a second tab.
Shared API: `browser_core::split`: `Splits` (`of`, `contains`, `can_join`, `join`, `leave`, `heir`,
`close`, `retain`, `swap`, `rotate`, `resize`, `equalise`, `menu`, `drop_label`), `Split` (`panes`,
`others`, `next`, `layout`, `dividers`), `Divider`, `Entry`, `Menu`, and the functions `layout`,
`dividers`, `ratio_at`, `pane_at`, `with_pane`, `drops_into`; `MAX_PANES`, `MIN_RATIO`.
macOS reference: `crates/app/src/split.rs`, `crates/app/src/sidebar.rs` (`tab_row`, `split_drop`),
`crates/app/src/browser/context.rs` (`split_menu`, `close_batch`), `crates/app/src/browser.rs`
(`activate`, `wake`).

- [x] Windows — same code in `crates/app`
- [ ] Linux — no split view yet: several `linux/src/webview` views placed by `layout` in `linux/src/ui.rs`, and drop targets on sidebar rows
- [ ] iOS — no split view yet; a tablet-width layout in `mobile/app` could place two web views by `layout`
- [ ] Android — same as iOS

### Clean Up (2026-11-16)

What it does: Clean Up under New Tab sorts the space's loose tabs into tab groups by what each page
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
