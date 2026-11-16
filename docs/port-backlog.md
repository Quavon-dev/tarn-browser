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

### Linux page tools from the UI review (2026-11-16)

What it does: Clear History, Clear Website Data and deleting a snapshot ask for a confirmation in
place before they act; the find bar counts matches ("3 matches", "No matches") and Ctrl+G /
Ctrl+Shift+G step to the next and previous match; right click on any sidebar tab opens that tab's
menu, not only the active one's.
Shared API: none; shell behaviour over the existing store and web view calls.
Linux reference: `linux/src/ui/panels.rs` (`confirmed`), `linux/src/ui/tools.rs`, `linux/src/ui.rs`.

- [ ] macOS — check each against `crates/app` (`library.rs`, `settings.rs`, the find bar in `content.rs`, `browser/context.rs`) and add what is missing
- [ ] Windows — same code as macOS
- [ ] iOS — confirmations in `mobile/app/src/ui/panels.rs`; find count exists, next/previous with a hardware keyboard
- [ ] Android — same as iOS

### Keyboard, labels and contrast from the desktop UI review (2026-11-16)

What it does: Up and Down wrap at the ends of a suggestion list and Page Up/Page Down jump to its
first and last row; `>` lists every command in a scrolling list; the command list leaves out
commands with nothing to act on (Reset Tab Title, Return to Pinned URL, Reopen Closed Tab, Close
Duplicate Tabs, split commands outside a split) and names Pin/Mute/Split by state. Esc declines a
page's dialog or permission prompt without remembering it, Enter accepts an alert or confirm, and
Esc closes the find bar, whose "No matches" clears while typing. A primary button's label turns
dark when white would drop under 3:1 on the accent. A toast with a button stays 10 s. The tab
menu is regrouped, the space menu offers Rename for the current space, a folder offers New
Subfolder, and library rows offer Copy Link.
Shared API: `browser_core::suggest::step_selection`, `browser_core::theme::on_fill`.
macOS reference: `crates/app/src/browser.rs` (`select_suggestion`, `suggestion_key`, `suggest`,
`dismiss_prompt`, `prompt_key`), `crates/app/src/browser/context.rs` (`tab_menu`, `space_menu`,
`folder_menu`), `crates/app/src/toast.rs`, `crates/ui/src/controls.rs` (`button`).

- [x] Windows — same code in `crates/app`
- [ ] Linux — suggestion keys and command filtering in `linux/src/ui.rs`, prompt keys, find bar, `on_fill` for `accent_pill` labels in `nsh-shared`
- [ ] iOS — command filtering and `on_fill` labels in `mobile/app`; prompt and suggestion keys with a hardware keyboard
- [ ] Android — same as iOS

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
- [x] Linux — `linux/src/browser/split.rs`, `linux/src/ui/split.rs`
- [ ] iOS — wide layouts have panes, headers and the Tabs sheet actions (`mobile/app/src/ui/mod.rs` `web_view`, `pane_header`; `panels.rs`), and a phone shows the active pane only; still missing: a draggable divider with double-tap to even out, dragging a header onto a pane to swap, and focusing a pane by tapping its page (Linux reports that through the web view's `pressed` event)
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
