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

### Tab sleeping choices and the Suggestions switch (2026-11-17)

What it does: Settings chooses how many tabs stay awake (3, 6, 10, 16) and how long a background tab
waits before it sleeps (Never, 10 minutes, 30 minutes, 2 hours); a switch turns history
suggestions, the inline completion and full-text matches off.
Shared API: `browser_core::settings::{live_tabs, sleep_after, max_idle, LIVE_TABS, SLEEP_AFTER, SUGGESTIONS}`,
`engine::lifecycle::Lifecycle::set_max_live`.
macOS reference: `crates/app/src/settings.rs` (`tabs`, `search`), `crates/app/src/prefs.rs` (`set_choice`), `crates/app/src/tabs.rs` (`sleep_idle_tabs`).

- [x] Windows — same code in `crates/app`
- [x] Linux — `linux/src/ui/settings.rs` (General › Tabs, Search › Suggestions), `linux/src/memory.rs`
- [ ] iOS — `mobile/app` keeps a fixed `LIVE_TABS` of 6 and always suggests; the Settings sheet needs both choices and the switch
- [ ] Android — same as iOS

### Save Page as PDF and Print… in the share menu (2026-11-17)

What it does: the share menu saves the page as a PDF (listed in Downloads) and opens the print
dialog.
Shared API: `browser_core::text::capture_name` (the PDF's file name); web view operations `pdf`
and `print` (`docs/shared-core.md` §1).
macOS reference: `crates/app/src/share.rs`, `crates/app/src/capture.rs`, `crates/app/src/browser.rs` (`print`).

- [x] Windows
- [x] Linux
- [ ] iOS — `print` and `pdf` in `mobile/ios` (`UIPrintInteractionController`, `WKWebView.createPDF`)
  and the two entries in the share sheet of `mobile/app`
- [ ] Android — `print` and `pdf` in `TarnWebView` (`PrintManager` with `createPrintDocumentAdapter`)
  and the two entries in `mobile/app`

### Bookmarks manager (2026-11-17)

What it does: Bookmark Tab… and Bookmark All Tabs, and a manager with recent bookmarks, folders,
search, rename, move, delete and New Folder.
Shared API: `browser_core::bookmarks::{bookmark, unbookmark, bookmark_all, new_folder, move_targets, subtree, recent, search,
save_for_later, reading_list_pages, title_or_host}`;
`store::Store::{rename_bookmark, move_bookmark, delete_bookmark}`.
macOS reference: `crates/app/src/library.rs` (`Page::Bookmarks`, `row_menu`), `crates/app/src/menubar.rs` (Bookmarks menu).

- [x] Windows — same code in `crates/app`
- [x] Linux — `linux/src/ui/library.rs` (`bookmarks_panel`), `linux/src/browser/bookmarks.rs`
- [ ] iOS — `mobile/app` has only the Reading List, with its own copy of Save for Later that saves a page twice
  (`mobile/app/src/features.rs`); move it onto `bookmarks::save_for_later`, then Bookmark Tab, Bookmark All Tabs and a
  bookmarks sheet
- [ ] Android — same as iOS

### Tab actions and Mute Site (2026-11-17)

What it does: Mute Site (saved per profile, a Lockdown mute lasts while the tab stays on the host;
a mark on the tab and a Muted sites list in Settings › Privacy), Rename Tab and Reset Title,
Return to Pinned URL and Set Pinned URL to Current Page, Duplicate Tab, Close All, Other and Below,
Sleep Tab, Other and All Inactive Tabs, reopening a chosen recently closed tab, Delete Folder,
the sidebar menu, and closing a pinned tab unloads it.
Shared API: `browser_core::site::saves_mute`, `Store::{muted_sites, set_site_muted}`,
`browser_core::tabs::{custom_title, Model::pinned_drift, Model::bookmark_all,
Model::folder_contents}`, `ClosedTabs::{recent, take}`; web view `set_muted` (NSH: `muted` prop).
macOS reference: `crates/app/src/browser/context.rs` (`tab_menu`, `sidebar_menu`, `folder_menu`),
`crates/app/src/tabs.rs`; Linux: `linux/src/browser/tab_actions.rs`, `linux/src/ui/panels.rs`.

- [x] Windows — same code in `crates/app`
- [x] Linux — `linux/src/browser/tab_actions.rs`, the Tab, Sidebar, Folder and History panels
- [ ] iOS — the `muted` prop in `mobile/ios` (`WKWebView` media muting) and the tab actions in the
  tab sheet of `mobile/app/src/ui/panels.rs`
- [ ] Android — the `muted` prop in `TarnWebView` and the same tab actions in `mobile/app`

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

### Profile editor and space card (2026-11-17)

What it does: Create and Edit Profile (name, colour, first space, Shield level, search engine,
shared data), Delete Profile with its website data retired, the profile switcher with colour dots
and Manage Profiles, the Settings › Profiles list with the Developer badge, the Edit Space card
(icon, colour, name), Delete Space…, the space context menu, and switching to space 1–9 by key.
Shared API: `browser_core::profiles::{create, delete, deletable, profile_name, space_name,
next_space_name, save_space, retired_stores, set_retired}`; `Model::can_delete_space`,
`Model::space_to_delete`.
macOS reference: `crates/app/src/profiles.rs`, `crates/app/src/spaces.rs`,
`crates/app/src/browser/context.rs` (`space_menu`, `delete_space`), `crates/app/src/menubar.rs`.

- [x] Windows — same code in `crates/app`
- [x] Linux — `linux/src/ui/profiles.rs`, `linux/src/browser/profiles.rs`
- [ ] iOS — `mobile/app` creates profiles but cannot edit or delete them, has no space card or Delete Space; the store retirement needs the `WKWebsiteDataStore` removal macOS uses
- [ ] Android — same as iOS, retiring the profile's WebView data folder

### Group and folder icons (2026-11-17)

What it does: a tab group or folder can show one of 20 symbols or one emoji in place of its letter
or folder glyph ("Change Icon…" in its menu); Clean Up gives each group it makes its category's icon.
Shared API: `browser_core::icon::{SYMBOLS, parse, set_group, set_folder}`, `Group::icon`,
`Folder::icon`; Clean Up through `browser_core::cleanup::apply`.
macOS reference: `crates/app/src/sidebar.rs` (`chosen_icon`, `icon_picker`, `group_avatar`),
`crates/app/src/groups.rs` (`begin_icon`, `set_icon`), `crates/app/src/browser/context.rs` (`Cmd::ChangeIcon`).

- [x] Windows — same code in `crates/app`; no Emoji & Symbols button, since GPUI cannot open the Windows emoji panel
- [x] Linux — `linux/src/ui.rs` `folder_icon`, `linux/src/ui/panels.rs` `icon_choice`, `linux/src/ui/groups.rs` `badge` and the group panel
- [ ] iOS — `mobile/app` has no folders or tab groups yet; both first
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
- [x] Linux — `linux/src/ui.rs` (`new_tab_row`), `linux/src/ui/groups.rs` (group card and panel), `linux/src/browser/groups.rs` (`clean_up_tabs`, `undo_sort`, `ToastAction::UndoSort`), Settings › Tabs in `linux/src/ui/settings.rs`
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

### YouTube ad script and Netflix visibility script (2026-11-15)

What it does: on YouTube the ad placements are stripped from the page's own data before the
player reads them (Settings › Privacy, block YouTube ads); on Netflix the page always reports
itself visible, so playback does not pause when the window loses focus.
Shared API: `browser_core` `youtube_script` (the script the app builds), `engine::page_scripts::ALWAYS_VISIBLE`;
both run at document start in the page world, the YouTube one only where blocking is not paused
and never under Lockdown.
macOS reference: `crates/engine/src/data_store.rs` (`install_scripts`); Windows:
`crates/engine/src/chromium/view.rs` (`page_script_for`, `rescript`); Linux, whose web view loads
the bundle itself: `linux/src/webview/scripts.rs` (`apply_page_scripts`) with the script from
`filters::youtube_script` in `linux/src/webview/filters.rs` (`shield::youtube::script` of the
config `shield::groups::load` returns), switched by the `youtube` view prop.

- [x] Windows — Chromium `Page.addScriptToEvaluateOnNewDocument`
- [x] Linux — document-start page-world user script through WebKitGTK's user content manager in `linux/src/webview`
- [ ] iOS — page-world `WKUserScript` in `mobile/ios`
- [ ] Android — `WebViewCompat.addDocumentStartJavaScript` in `TarnWebView` (page world)
