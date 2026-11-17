# Changelog

All notable changes to Tarn are documented in this file. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and Tarn follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- About links the privacy notice, the imprint and the open-source licence list (macOS, Windows,
  Linux, iOS, Android).
- Linux, iOS and Android builds come with their third-party licence notices; Linux packages
  install them under `/usr/share/licenses/tarn` (the rpm now also ships `LICENSE` there), and on
  iOS and Android Settings › Open-source licences shows the list built into the app, searchable
  and offline.
- Tarn extensions: a package whose manifest sets `browser_specific_settings.tarn.runtime` to 1
  runs on Tarn's own runtime with content scripts, static network rules, `runtime`, `storage`,
  `tabs`, `action`, `i18n` and a background page, the same on every engine, installed from a file,
  GitHub or the official catalog with per-site grants (macOS, Windows, Linux, iOS, Android). On
  macOS they run in WebKit and Chromium tabs alike, cross-site frames included, with a toolbar
  button, badge and popup; on Windows they are the extensions Tarn can run.

### Changed

- The macOS disk image opens on a window drawn from the Tarn icon's lake, with an arrow from Tarn
  to the Applications folder and the Tarn volume icon.
- The privacy notice names Quavon UG (haftungsbeschränkt) as controller, gives the legal basis and
  your rights, and states per client what it describes.

### Security

- On Android, Tarn's own requests and file downloads refuse a server whose verified certificate
  path, root included, holds an RSA key under 2048 bits or an elliptic-curve key under 224 bits.

## [0.1.1] - 2026-11-16

### Added

- Every release carries a CycloneDX SBOM for each client (macOS, Windows, Linux, iOS, Android),
  and `SECURITY.md` explains how to verify a download's checksum, attestation and signature.
- Nightly builds: Settings › General › Nightly builds updates Tarn every day to a signed build of
  the latest changes (macOS, Windows). The `nightly` pre-release holds them and unsigned builds for
  Linux, iOS and Android.
- Releases are signed, notarised and published by CI; the Mac disk image, the Sparkle feed and the
  Windows update feed come with every release.
- The find bar counts the matches on the page ("3 matches") on macOS and Windows.
- Right click on the address shows Copy Link, Copy Clean Link, Paste and Go and Site Settings;
  every text field offers Cut, Copy, Paste and Select All, the address field also Paste and Go
  (macOS, Windows).
- Tab and Shift+Tab move through the controls of Site Settings, Downloads and Settings with a
  visible focus ring; Enter or Space acts on the focused one, and Up/Down walk the downloads list
  (macOS, Windows).
- Settings › Permissions lists the camera, microphone, location and pop-up answers a profile
  remembered, with Allow, Block and Forget (macOS, Windows, Linux, iOS, Android).
- Split view holds up to four tabs; drop a tab on another tab in the sidebar to split them
  (macOS, Windows, Linux). A window keeps several splits for the session, and selecting one of
  their tabs shows that split again. Each pane has a header to focus it, open its tab menu, remove
  or close it, or drag it onto another pane to swap the two; both dividers drag, a double click
  evens them out, Rotate Split Layout stacks the panes, and the tab menu has a Split View submenu.
  New shortcuts focus the next or previous pane (⌃⌘] / ⌃⌘[ on macOS, Ctrl+Alt+] / [ on Linux),
  swap panes, rotate and remove a tab. On iOS and Android tablets a split shows its panes side by
  side with a header each, and the Tabs sheet opens, rotates and closes it.
- macOS and Windows: `>` in the address field lists every command in a scrolling list, leaves out
  commands with nothing to act on and names Pin, Mute and Split by state; suggestions wrap at the
  ends and Page Up / Page Down jump to the first and last row; Esc declines a page's dialog or
  permission prompt and Enter accepts an alert or confirm; the tab, space and folder menus are
  regrouped (Rename Space, New Subfolder), library rows offer Copy Link, menus show colour dots
  and check marks, and a toast with a button stays 10 seconds.
- Linux: Up and Down pick address suggestions, right click opens the menu of any tab, the find bar
  counts matches and Ctrl+G / Ctrl+Shift+G step through them, icon buttons have tooltips, and Clear
  History, Clear Website Data and snapshot Delete ask for a confirmation.
- Linux: `>` lists every command in a scrolling list, leaves out commands with nothing to act on
  and names Pin and Split by state; suggestions wrap and Page Up / Page Down jump to the ends; Esc
  declines a page's dialog or permission prompt and Enter accepts an alert or confirm; the find
  bar's count clears while typing; a toast with a button stays 10 seconds; the space and folder
  panels rename (New Subfolder in a folder's), History and Reading List rows offer Copy Link, each
  panel closes from its header, both Clear buttons for downloads ask for a confirmation, the
  sidebar's space buttons are 28 points and wrap to a second row, and dragging a tab row onto
  another row's edge moves it there.
- Android: the system back gesture closes a prompt, a sheet or the address field before it goes
  back in the page; iOS and Android sheets have a Back button to the sheet they came from.
- iOS and Android tablets: a split's dividers drag and a double tap evens them out, a pane header
  dropped on another pane swaps the two, and a tap on a pane's page focuses it. With a hardware
  keyboard, Esc declines a page's dialog or permission prompt without remembering it, Enter
  accepts an alert or confirm, arrows and Page Up / Page Down pick address suggestions, and
  ⌘G / ⇧⌘G (Ctrl on Android) and Esc work in the find bar. The menu and Tabs sheet leave out
  Reopen Closed Tab, Close Duplicates and Zap Element when they have nothing to act on, every tab
  card offers Move to Space, and deleting a snapshot asks first.
- Clean Up under New Tab sorts tabs into groups on this device and archives unused tabs (macOS,
  Windows).
- Tiles arrange themselves: one row while they fit, then as few rows as fit, filled evenly; Add to
  Tiles and Remove from Tiles in a tab's menu (macOS, Windows, Linux).
- Windows: camera and microphone requests, HTTP sign-in prompts, and links to other apps
  (`mailto:`, `tel:`) ask as on macOS; Shift-click opens a link in a peek, Ctrl- and middle-click
  open it in a new tab, and the page menu offers link, image and media items.
- Windows: the floating sidebar and hover card follow the pointer over pages, a click focuses a
  split pane, Esc closes a peek, and the mouse's back and forward buttons work over pages.
- Windows: sign-in and payment pop-ups open as tabs that keep their link to the page; a video's
  full screen fills the screen; Inspect opens DevTools in a window kept away from the web; the
  menu shows colour dots and site icons; pages fade on a tab switch; two-finger swipes over the
  sidebar and toolbar switch spaces and tabs; Arc's history and bookmarks can be imported.
- Windows: a tab that sleeps, is reopened or is restored at launch keeps its back and forward
  history.
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
- Windows: the memory panel measures each tab and the memory budget sleeps tabs; filter-list
  element hiding reaches subframes, including cross-site ones.
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

### Changed

- A split's tabs show side by side in one row of the sidebar, and in one card of the tab grid on
  iPad and Android tablets (macOS, Windows, Linux, iOS, Android).
- The app identifier is `de.quavon.tarn` (was `dev.tarn.browser`) on macOS, iOS and Android. On
  macOS the first start moves the website data over; Tarn has to be made the default browser again,
  camera and microphone permissions are asked again, and a saved Anthropic API token has to be
  entered again.

### Fixed

- Chromium tabs on macOS count toward the memory budget and the heavy-tab dot, so heavy
  background Chromium tabs sleep like WebKit tabs instead of holding their memory.
- The loading spinner starts when a page starts loading instead of when it first draws, stops
  when a Chromium page is stopped or cancelled, and turns evenly (macOS, Windows, Linux, iOS,
  Android).
- When Tarn could not open its database, it no longer deletes Chromium's cookies and site data at
  start and quit as if their profile were gone.
- A Lockdown profile that shares another profile's website data no longer clears that data when it
  opens; it clears only its own (macOS, Windows).
- iOS and Android: Back and Forward are dimmed when there is nowhere to go, also after a page
  changes its address in place (`pushState`, a fragment); on iOS, typing in the find bar starts
  again at the first match instead of stepping forward with every key; a primary button's label
  turns dark when white would drop under 3:1 on a theme's accent (also Linux); the switch's thumb
  and off track keep their contrast in the light theme.
- macOS and Windows: the loading spinner stops when a load fails or the tab sleeps; banners show
  up to three lines; the site settings popover scrolls when it is tall; the hover card stays
  inside the window; the reader's controls stay in view while the article scrolls; a switch that
  is off is visible in the light theme and a primary button's label stays readable on any accent;
  Windows no longer leaves room for macOS window buttons.
- iOS and Android: opening a sheet or the address field no longer pauses the page's audio and
  video, and banners float over the page instead of pushing it down.
- Windows: pages without a background of their own no longer turn dark grey after a link or a
  pop-up in dark mode.
- Chromium tabs closed because no update check has verified yet, as on a new install, say so
  instead of claiming that none verified for seven days (macOS, Windows).
- Windows: a release without the signed installer carries a signed update feed for its unsigned
  installer, so installs from it open pages and receive updates.
- Sites can use the camera and microphone again on macOS: the app declares both and asks macOS;
  Chromium tabs ask through Tarn's prompt instead of refusing.
- Chromium tabs apply a site's pop-up rule and count only refused pop-ups; tracking parameters
  removed from typed addresses count on Linux, iOS and Android; more refused local-network
  pop-ups count on iOS and Android.
- Less work while pages load: spinners step 12 times a second instead of redrawing every display
  frame; new site icons no longer rebuild the menu bar; Chromium's background timer slows down
  with no Chromium page on screen.
- macOS Chromium tabs: ⌘ and ⌃ shortcuts reach Tarn again; editing keys stay with the page.
- Chromium tabs: the page menu keeps Chrome's copy, media, look-up and history items, and Inspect
  where Web Inspector is on (Settings › Developer or a developer profile).
- Closing several tabs at once reloads the sidebar once and picks the nearest tab that stays open; a
  closed Chromium page no longer lingers on screen; rows close up at once under the pointer, so
  repeated clicks close the next tab.
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

- Pop-ups share one look and motion on macOS and Windows: toasts, the QR code card, the tab hover
  card and tooltips cast the same shadow as Site Settings and Downloads, the QR card fades out,
  the shortcuts sheet and reader rise in, and shadows follow one elevation scale. Menus opened from
  a button (the Windows menu button, "Always open in") drop from the button instead of the pointer.
  The find bar uses icon buttons with their shortcuts in tooltips. Dragging a tab over another
  shows a line where it will land. The start page's fog and search effect animate only in the
  front window.
- Tarn is now licensed under the GNU General Public License v3.0 or later (`GPL-3.0-or-later`).
  Only Quavon UG (haftungsbeschränkt) may distribute it through app stores
  (`LICENSE-APP-STORE.md`), and the Tarn name, logo and app icon are trademarks (`TRADEMARKS.md`).

### Security

- The cookie bridge between WebKit and Chromium no longer gives Chromium a less restricted copy of
  a cookie set without `SameSite` (macOS): WebKit reports such cookies as `SameSite=None`, and the
  bridge now reads them as `Lax`, as Chromium does. Sign-ins made in WebKit reach Chromium tabs
  again.

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

[Unreleased]: https://github.com/Quavon-dev/tarn-browser/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/Quavon-dev/tarn-browser/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/Quavon-dev/tarn-browser/releases/tag/v0.1.0
