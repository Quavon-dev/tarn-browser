# Changelog

All notable changes to Tarn are documented in this file. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and Tarn follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Linux, iOS and Android apps on Native Shell over the same core crates.
- Cookie-banner blocking.
- Signed filter-list updates from this repository's releases, once a day.
- App icons for the iOS and Android apps.
- CI builds and tests Windows; a `v*` tag builds a draft release for macOS, Windows, Linux, iOS and
  Android, with a licence list for the Windows build.
- Settings › About names the licence and links the source code and the licence text.
- Themes and search bar effects on Linux, iOS and Android: the theme picker with Match system, the
  colour editor, themes from a file, the clipboard or a GitHub release, the official theme catalog,
  theme fonts, website styles per profile, withdrawals, and Shift at launch on Linux.
- Linux, iOS and Android: Open Anyway on local-network notices, page-text search after a typing
  pause, the redesigned Library, Settings and Shield card, the reload spinner, and fog, effects and
  spinners that stop in the background.
- Linux: New Window (Ctrl+N), greyed sleeping tabs, two-finger space swipe, and Open in New Tab for
  blocked pop-ups.
- iOS and Android: Share sits in the address field and the toolbar gets a New Tab button that opens
  the address search.
- Windows release builds run Chromium's child processes in its sandbox (`tarn.exe` is CEF's launcher,
  Tarn loads from `tarn.dll`).
- Windows per-user installer (Inno Setup), Authenticode signing on the maintainer's machine, and an
  unsigned installer in each draft release.
- Windows updates itself from signed GitHub releases (`windows-update.json`), installing on Restart to
  Update or on quit while automatic checks are on.
- Windows app menu (toolbar button and F10), touchpad swipe navigation, rounded page corners, the
  YouTube ad script and the Netflix visibility script in Chromium tabs.
- Background tabs whose web pages together use more than 1 GB of memory sleep, the heaviest
  first, after a minute unused, pinned tabs after 30 minutes (macOS; off when Sleep background
  tabs after is Never).

### Fixed

- File upload fields open the system file picker on macOS (WebKit and Chromium) and Android.
- Picking a tab in the sidebar, or with ⌘1–9 or Ctrl-Tab, closes an open new-tab launcher.
- ⌘T works again after closing the new-tab launcher by picking a tab in the floating sidebar.
- Switching spaces, closing tabs or groups, returning a pinned tab and adding a space from the
  floating sidebar keep the keyboard in the main window, so shortcuts and the space card's name
  field keep working.
- Shortcuts keep working after a rename in the sidebar is saved by clicking the page or switching
  spaces, and after Delete Profile Data closes an open find bar.
- Shortcuts keep working when Reader, the shortcuts sheet, an empty space or a JSON page hides an
  open find bar or a page's text or sign-in prompt.
- Switching spaces closes an open launcher, so it no longer suggests pages from the previous
  space's history, such as a private space's or a burner's.
- Esc on the new-tab start page returns to the tab it replaced.
- Closing a pinned tab that is not the current one no longer switches tabs or ends the start page.
- Closing a window closes its Settings window instead of leaving one that can no longer change
  anything.
- Windows: Ctrl+H and Ctrl+Alt+H can be assigned (they were held by the macOS-only Hide actions),
  and the shortcut reset message names keys as Windows shows them.
- A screenshot or PDF is saved under the page it was taken of, even when its tab closes first,
  and is no longer filed under a new tab that took the old tab's place.
- The Picture in Picture notice and the playing marker no longer land on a tab that replaced the
  one they were asked about.
- Closing a window during an extension install or catalog download no longer leaves extensions
  busy in every other window.
- Windows: Edit › Undo, Redo, Cut, Copy, Paste and Select All in the app menu act on the page.
- The tab preview waits at most five seconds for a page that does not answer, so it keeps
  working afterwards, and no longer shows another tab's page under a reused tab.
- ⌘T in split view keeps the split, and Esc on the start page returns to both panes.
- Extension updates queued in a window that closes carry on in another window.
- Linux, iOS and Android: a new tab that reuses a closed tab's id gets its own page instead of the
  closed tab's, which kept running and wrote its address and history to the new tab.
- Linux: switching spaces clears the start card's text and suggestions, so a private space's
  history no longer shows in another space; closing or moving a background tab keeps Reader open
  on the current one.
- iOS and Android: opening the tabs or menu sheet while editing the address closes the keyboard,
  so the address field works again afterwards.
- History opened from the menu shows at full height; Library › Downloads updates as downloads
  progress; the Memory page keeps the selection on its tab while the list re-sorts.
- Live Tabs and Auto-hide Sidebar apply to every open window.
- Site CSS follows the page when it moves to another site while the site card is open.
- Dropping a tab on a row puts it at that row after tabs were closed; a tab can no longer be
  moved into another space's folder.
- Reopening a closed tab no longer puts it into a new group that took a deleted group's place,
  archiving deletes only the groups it emptied, and private and normal folders open and close
  separately.
- The menu bar shows a changed shortcut at once, while Settings is still in front.
- A closed private tab's row no longer flashes in the matching normal space, and reopening a tab
  right after closing it no longer drops its collapsing row early.

### Changed

- Tarn is now licensed under the GNU General Public License v3.0 or later (`GPL-3.0-or-later`).
  Only Quavon UG (haftungsbeschränkt) may distribute it through app stores
  (`LICENSE-APP-STORE.md`), and the Tarn name, logo and app icon are trademarks (`TRADEMARKS.md`).

## [0.1.0] - 2026-11-14

### Added

- Native macOS browser: a Rust shell with a GPUI interface and the system WebKit for pages, with an
  optional Chromium engine while automatic updates are on.
- Shield privacy levels (Standard, Strict, Lockdown): tracker and ad blocking, tracking-parameter
  stripping, local-network protection, HTTPS-only and no-JavaScript modes.
- Switchable blocking for ads, trackers, YouTube ads, pop-ups, malware, phishing,
  scam and adult sites, with signed filter lists updated daily.
- Spaces and profiles with separate cookies, history and site data, and memory-only private spaces.
- One address field for URLs, search, open tabs and app actions, with configurable shortcuts.
- Split View, link peek, tab groups, pinned tabs that remember their page and sleeping tabs.

[Unreleased]: https://github.com/Quavon-dev/tarn-browser/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Quavon-dev/tarn-browser/releases/tag/v0.1.0
