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

### Shared tablet tables that macOS and Linux still duplicate (2026-11-17)

What it does: the tablet Mac layout reads its tab menu, keyboard shortcuts, command palette and trackpad
swipe from `browser-core`; macOS, Windows and Linux still keep their own copies, which can drift.
Shared API: `browser_core::tab_menu::items`, `browser_core::shortcuts::{SECTIONS, bindings, table}`,
`browser_core::palette::{COMMANDS, rows}`, `browser_core::sidebar::Swipe`.
macOS reference: `crates/app/src/browser/context.rs` (`tab_menu`), `crates/app/src/settings.rs` (`SHORTCUTS`),
`crates/app/src/lib.rs` (`COMMANDS`), `crates/app/src/gestures.rs` (`Swipe`).

- [ ] Windows — same as macOS (shares `crates/app`)
- [ ] macOS — not a mechanical swap: the tab menu has entries the core list lacks (Reload in Chromium,
  Bookmark Tab, Mute, Sleep, Return to pinned URL, the Split View and Add to Group submenus, key hints);
  `SHORTCUTS` covers actions the core table does not (Close All Tabs, split panes, Print, Picture in
  Picture, …) and binds GPUI actions; `COMMANDS` dispatches GPUI actions; `Swipe` reads touch phases
  and natural scrolling, where the core one times wheel pauses. Each needs the core type extended
  first. `Step` and the space neighbour already come from `browser_core::sidebar`.
- [ ] Linux — `linux/src/shortcuts.rs` keeps user-editable bindings the core table cannot hold yet;
  `linux/src/ui/tools.rs` (`commands`) and the This Tab panel are its palette and tab menu.
  `Swipe`, `Step` and the space neighbour already come from `browser_core::sidebar`.
- [x] iOS — the tablet Mac layout uses all four
- [x] Android — same as iOS

### Group and folder icons (2026-11-17)

What it does: a tab group or folder can show one of 20 symbols or one emoji in place of its letter
or folder glyph ("Change Icon…" in its menu); Clean Up gives each group it makes its category's icon.
Shared API: `browser_core::icon::{SYMBOLS, parse, set_group, set_folder}`, `Group::icon`,
`Folder::icon`; Clean Up through `browser_core::cleanup::apply`.
macOS reference: `crates/app/src/sidebar.rs` (`chosen_icon`, `icon_picker`, `group_avatar`),
`crates/app/src/groups.rs` (`begin_icon`, `set_icon`), `crates/app/src/browser/context.rs` (`Cmd::ChangeIcon`).

- [x] Windows — same code in `crates/app`; no Emoji & Symbols button, since GPUI cannot open the Windows emoji panel
- [ ] Linux — folders done (`linux/src/ui.rs` `folder_icon`, `linux/src/ui/panels.rs` `icon_choice`); group icons come with tab groups
- [ ] iOS — the tablet Mac layout shows folder and group icons and has Change Icon… with the 20 symbols and an
  emoji field (`mobile/app/src/ui/mac/sidebar.rs`, `tarn_nsh_shared::sidebar::{chosen_icon, group_avatar}`); the phone and
  Touch layouts have no folders or tab groups yet
- [ ] Android — same as iOS

### Clean Up (2026-11-16, reworked 2026-11-17)

What it does: the Clean Up button in the New Tab row sorts the space's loose tabs into tab groups by what each page
is (on this device: a known site decides, else host labels, path and title words), into the user's own
group when that group already holds two or more tabs of the same host and no other group does,
orders category groups and keeps tabs of one host together; Undo Sort in its toast puts the layout
back. It also archives tabs unused for longer than Settings › Tabs allows.
Shared API: `browser_core::cleanup::{plan, apply, undo, category}`. `apply` and `undo` each run one
`Store::apply_layout` transaction; `plan` and `category` take no `Store`. Archiving goes through
`Model::archive_unused`.
macOS reference: `crates/app/src/browser/context.rs` (`clean_up_tabs`, `undo_sort`), `crates/app/src/toast.rs`
(`Action::UndoSort`), `crates/app/src/sidebar.rs` (`ephemeral`).

- [x] Windows — same code in `crates/app`
- [ ] Linux — `linux/` has no tab groups or archiving yet; both first, then the row in `linux/src/ui.rs`
- [ ] iOS — the tablet Mac layout sorts by category with Undo Sort in its toast
  (`mobile/app/src/ui/mac/sidebar.rs`, `browser_core::sidebar::{clean_up, undo_sort}`); archiving unused tabs and the
  phone and Touch layouts are still missing
- [ ] Android — same as iOS

### Tile grid (2026-11-16)

What it does: the tiles above the pinned tabs arrange themselves: one row while they fit, then as
few rows as fit, filled evenly (six tiles in a 256 pt sidebar make 3 × 2); Add to Tiles and
Remove from Tiles in a tab's menu.
Shared API: `browser_core::tabs::tile_columns`, `browser_core::tabs::Model::tile_toggle`.
macOS reference: `crates/app/src/sidebar.rs` (`tiles`), `crates/app/src/browser/context.rs` (`Cmd::ToggleTile`).

- [x] Windows — same code in `crates/app`
- [x] Linux — `linux/src/ui.rs` (`tiles`), `linux/src/ui/panels.rs` (This Tab panel)
- [ ] iOS — the tablet Mac layout has pinned tabs and the tile grid with Add to Tiles and Remove from Tiles
  (`mobile/app/src/ui/mac/sidebar.rs`); the phone and Touch layouts' tab sheet in `mobile/app/src/ui/panels.rs`
  still shows every tab as a card
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
