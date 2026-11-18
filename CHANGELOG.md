# Changelog

All notable changes to Tarn are documented in this file. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and Tarn follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- On macOS, Windows and Linux, dragging a tab out of the window opens it in a window of its own.
  Settings › Tabs picks a temporary window, which lists only that tab's own temporary space in the
  same profile and deletes it with its tabs when the window closes, or a full window with every
  space and profile. Pinned tabs and tiles open in a full window, except from a temporary one.
  Outside the window a small preview follows the pointer and says which window letting go opens
  (macOS, Windows). On Linux the window's edge lights up with the same words as the dragged tab
  nears it, and the tab tears off as the pointer leaves the window.
- History lists pages under Today, Yesterday and Earlier with when each was last visited. Remove
  from History (row menu, the row's trash button, or ⌘⌫ on macOS and Delete on Windows) removes a
  page, and Clear browsing history in Settings › Privacy asks whether to clear the last hour, today
  or everything (macOS, Windows, Linux). On iOS and Android the History sheet groups pages by day,
  shows when each was last visited, removes one with its trash button and offers Clear History…
  with the same ranges.
- A search field in Settings finds the pane that holds a setting (macOS, Windows, Linux, iOS,
  Android).
- In the Library, F2 renames, ⌘⌫ (Delete on Windows) deletes and Shift+F10 opens the menu of the
  selected row, whose buttons stay visible; icon buttons have tooltips (macOS, Windows). On Linux, F2
  renames the focused bookmark and Delete deletes it (asking first) or removes the focused History
  row; its rows have no menu, their buttons sit on the row.
- Tablets: Settings › Appearance › Layout switches an iPad or Android tablet between the touch layout
  and a Mac layout like the macOS app: the sidebar with tiles, pinned tabs, folders, groups and tab
  menus, a toolbar with a command palette, Site settings, Downloads and Share as popovers under their
  buttons, New Space and Edit Space above the sidebar's + button, Clean Up with Undo Sort, group and
  folder icons, a Settings window, and the macOS keyboard shortcuts with a hardware keyboard (iOS,
  Android).
- Open File… shows a local file in a new tab: web pages with their images and styles, PDFs,
  images, text, SVG and video. ⌘O on macOS, Ctrl+O on Windows and Linux, and the menu on iOS and
  Android; the address bar shows the file's name (macOS, Windows, Linux, iOS, Android; no PDFs on
  Android, and on iOS and Android a page's images and styles from its folder do not load).
- Background tabs sleep, the heaviest first, once their web processes together hold more than
  1 GB, split between windows (Linux).
- The share menu has Save Page as PDF, through the save dialog and listed in Downloads (also a
  command row), and Print… (Ctrl+P) (Linux). Linux desktops have no system share sheet, so it has
  no More….
- A download from a site on the malware list waits, unsaved, until you choose Keep or Delete in the
  Downloads panel, as on macOS (Linux).
- Tab groups, Clean Up and closing unused tabs after a time set in Settings › Tabs, with an
  Archived Tabs panel next to the Reading List (Linux).
- Settings on Linux list site Shield overrides and saved zoom levels (with Remove and Reset All),
  switch localhost links from other apps, export the profile as JSON, clear each kind of data or
  delete everything in the profile, show the Tarn version and offer Make Default when Tarn is not
  the default browser (Linux).
- Malware, adult and pop-up ad domain sets stop sites in every frame, redirect and download before
  the request, with the warning page (Go Back, Visit Anyway for malware), the per-site switch for
  ad, tracker and cookie-banner blocking, pop-up rules, permission rows and Clear site data in the
  site popover, and the Privacy switches and paused-site list (Linux).
- Permission prompts offer Always Allow, saved per site and listed in Settings › Permissions, and
  every page prompt offers "Block requests from this page"; popup tabs share their opener's prompt
  and popup limits (Linux).
- Library on Linux: one search across open tabs, bookmarks, history and downloads, with links to
  History, Reading List, Downloads, Snapshots and Memory; history rows have Open in New Tab,
  snapshots can be renamed, and a download can be removed from the list or its link copied (Linux).
- Take Screenshot, a video's full screen that hides the toolbar and sidebar, full screen (F11 and a command)
  and back and forward by a two-finger swipe over the page (Linux).
- Profiles and spaces on Linux: Create and Edit Profile (name, colour, first space), Delete Profile
  with its website data retired as on macOS, the profile switcher with colour dots and Manage
  Profiles, the Settings › Profiles list with the Developer badge, the Edit Space card (icon,
  colour, name), Delete Space…, the space context menu, and Alt+1 … Alt+9 to switch to space 1–9
  (Linux).
- Settings › Developer: the developer tools switch (Web Inspector through Inspect Element, JSON
  viewer, responsive sizes, local servers) and the Developer profile; the Web Inspector never
  opens on Lockdown pages (Linux).
- Import Browser Data: history, bookmarks and cookies from Chrome, Edge, Brave, Chromium and Firefox, with
  Chromium's cookie key from the keyring (Linux).
- Filter-list updates: the signed Shield list bundle is downloaded once a day, with the same checks
  as on macOS, and Settings › Privacy shows the installed lists' date and the last error, with a
  switch to turn updates off (Linux).
- Bookmarks on Linux: Bookmark Tab… (`Ctrl+Shift+D`, the Tab panel and the command rows),
  Bookmark All Tabs (the New Tab row and the command rows), and a Bookmarks panel (`Ctrl+Alt+B`)
  with recent bookmarks, folders, search, rename, move, delete and New Folder (Linux).
- Settings › Search › Suggestions turns history suggestions and the address field's
  inline completion off, and Settings › Tabs chooses how many tabs stay awake and how
  long a background tab waits before it sleeps, as on macOS (Linux).
- Settings › Privacy › Block YouTube ads removes ads from YouTube's player data and hides ad
  slots, and Netflix keeps playing when the window loses focus (Linux).
- Tab actions: Mute Site with the Muted sites list, Rename Tab and Reset Title, Return to Pinned
  URL and Set Pinned URL, Duplicate Tab, Close All, Other and Below, Sleep Tab, Other and All
  Inactive Tabs, reopening a chosen recently closed tab, Delete Folder and the sidebar menu; closing
  a pinned tab unloads it (Linux).
- Settings › Tabs chooses whether New Tab opens the command card or the start page and
  which tab becomes active after closing one, and turns touchpad gestures off; Settings ›
  Appearance picks the accent colour and turns on Reduce motion, as on macOS (Linux).
- Compact toolbar, auto-hiding sidebar, the tab hover card with its Alt page snapshot, and link
  peek (Shift-click opens a link in a card over the page, in the opener's profile and Shield
  rules) (Linux).
- Linux: "Switch to Tab" rows for open tabs in address suggestions, an address field menu (Copy
  Link, Copy Clean Link, Paste and Go, Site Settings), Copy Link as Markdown and Copy Title, the
  Keyboard Shortcuts sheet (`Ctrl+/`), the macOS commands and shortcuts it was missing, switching
  to space 1–9 with `Alt+1`…`Alt+9`, and Settings › Privacy › Open localhost links from other apps.
- Tab groups and folders can show an icon: one of twenty symbols or a single emoji, set with
  Change Icon… in their menu (macOS, Windows, Linux; iOS and Android in the tablet Mac layout).
  Clean Up gives each group it makes the icon of its kind.
- File › Close Window (⇧⌘W), View › Stop Loading (⌘.) and Reload Without Cache (⌥⌘R); ⌘+ zooms
  in, and ⌘} ⌘{ and ⌥⌘→ ⌥⌘← go to the next and previous tab (macOS, Windows). Linux has Close
  Window (Ctrl+Shift+W), Reload Without Cache (Ctrl+F5), Stop Loading (Esc) as keys and command
  rows, and Ctrl+Page Down and Ctrl+Page Up for the next and previous tab (Linux).
- Tab and ⇧Tab move the keyboard through the toolbar's buttons and the address field (macOS,
  Windows).
- F6 and Shift+F6 move the keyboard between the page, the address field and the sidebar (macOS,
  Windows, Linux).
- A link's menu offers Open Link in New Tab and Open Link in Split View, which opens the link
  beside the page, with the pop-up checks of any new tab (macOS, Windows, Linux; on iOS and Android
  in the link's long-press menu, Split View where the layout is wide enough for it).

### Changed

- Settings › Profile is now Protection and holds the only Shield level choice, with each level's
  description; the profile list is only in Profiles. The card's counters and the "Counted on this
  Mac only" label are no longer repeated next to it, and a week with nothing stopped shows one line
  instead of an empty chart (macOS, Windows; on Linux the pane is Protection and holds the only
  Shield level choice; on iOS and Android it is named Protection).
- Choosing Lockdown (from Settings or the menu), clearing history, deleting a profile or everything
  in it, removing an extension or a theme, resetting zoom levels and deleting a bookmark ask first
  in a dialog that names what goes, instead of a second click on the same button (macOS, Windows;
  on Linux Lockdown and a single bookmark delete ask in place, the others already asked once more).
  On iOS and Android, choosing Lockdown and removing a page from the Reading List ask the same way.
  Removing a theme or an extension asks in a dialog on Linux, iOS and Android too, as does Clear
  site data in the Linux site popover.
- Filter-list updates, extension downloads and developer tools are switches; the search engine is
  picked from a segmented control as in the profile card; an extension's Every site switch, Forget
  Site Answers and Remove sit on a line under its name; Appearance has one Theme heading, theme
  cards fill the row and the theme's own accent swatch is marked (macOS, Windows).
- New Space and Edit Space open as one card on every client: pick a profile or start a new one,
  then name the space and give it a colour and icon; Edit Space also sets the schedule, and changes
  apply on Save. On macOS and Windows it is a popover above the sidebar's + button, drawn like the
  Library, and the profile card takes the same look; on Linux a popover like the Library; on iOS and
  Android a sheet from Spaces, or in the tablet Mac layout a popover above the sidebar's + button
  (macOS, Windows, Linux, iOS, Android).
- Clean Up sorts by what each page is: a known site decides, other pages by their address and
  title (Money and Mail and Chat take known sites only); a loose tab joins your own group that
  already holds two or more tabs of its site (a known site goes to its category's group when
  there is one), new category groups (now also Learning, Games and Food) come in a fixed order,
  tabs of one site sit together, and Undo Sort puts everything back (macOS, Windows, Linux; iOS and
  Android in the tablet Mac layout).
- Toasts have a calmer design: a tone icon, the message and a quiet text action such as Undo
  (macOS, Windows, Linux, iOS, Android).
- Tiles stretch to fill their row, so two tiles share the sidebar's width (macOS, Windows, Linux).
- The profile button left the sidebar header; profiles stay in Settings › Profile and the menu
  (macOS, Windows, Linux).
- Appearance has no separate Theme picker; the Themes grid sets the theme.
- Bitwarden's privileges (Touch ID unlock through the Bitwarden app, clipboard and passkeys) are
  off until you turn on Settings › Extensions › Bitwarden integration; until then Bitwarden runs
  like any other app extension (macOS).
- The macOS disk image opens on a window drawn from the Tarn icon's lake, with an arrow from Tarn
  to the Applications folder and the Tarn volume icon.
- Settings › General lists every profile and marks the one the window uses.
- The space name in the sidebar header has no chevron, and the space icons in the sidebar's bottom
  bar sit in its centre (macOS, Windows, Linux).
- The compact toolbar's address field keeps the same inset around the shield and the trailing
  buttons, which now sit inside it.

### Fixed

- Renaming a space shows the name as you type instead of an empty field (macOS, Windows).
- Closing the find bar removes WebKit's match marker from the page, and the bar no longer stays as
  an empty strip after Esc (macOS). On iOS closing find also hides the marker. The last match
  stays selected, as in Safari (macOS, Windows, Linux, iOS; not on Android, whose WebView clears it).
- Settings and Library questions (Lockdown, clearing history, deleting a profile, a bookmark or a
  theme) open inside their window in the app's theme, and closing the window drops the question
  instead of leaving a stray dialog behind (macOS, Windows).
- One-line labels that do not fit end in an ellipsis instead of being cut mid-letter: tab rows,
  split headers, History rows, command card rows and theme cards (macOS, Windows).
- In a narrow Settings window the Shield card's buttons sit under the card, segmented choices move
  under their description, and an installed extension's permissions wrap (macOS, Windows).
- The Shield card's mountains and waves stay clear of its text, and on its first day the card says
  "Protected since today" (all clients).
- View › Reload is greyed when no page is shown (macOS, Windows); on Linux, iOS and Android the
  Reload button is greyed then, and on Linux F5, Ctrl+R and Reload Without Cache do nothing.
- The tab preview card follows the pointer to the next row instead of staying on the previous tab,
  and Close Tabs Below closes the tabs shown below, leaving collapsed groups alone (macOS, Windows,
  Linux).
- Dragging a tab onto the lower half of a row drops it below that row, split panes list one per
  row so their titles fit, Duplicate places the copy under the original in its group, and an empty
  tab name restores the page title (macOS, Windows, Linux).
- Unloading the active pinned tab moves to its neighbour instead of reloading it, Sleep Tab is off
  for panes on screen, Rename Tab skips tiles, a double click on a group or folder header renames
  it without collapsing it, and title matches in Find in Tabs switch to the tab (macOS, Windows; on Linux the
  pinned-tab neighbour, Sleep Tab and Find in Tabs parts).
- Sidebar rows can be reached with the keyboard: Up and Down move, Return opens, Shift+F10 opens
  the tab menu (macOS, Windows, Linux).
- Stars on the night Shield card no longer sit on the profile name or the level (all clients).
- Text fields scroll to keep the caret in view, undo and redo with ⌘Z and ⇧⌘Z (Ctrl on Windows),
  select a word on a double click and everything on a triple click, place the caret where a
  password field is clicked, show a selection only while focused, and no longer crash when an
  input method selects part of its text (macOS, Windows).
- Accent-coloured text (settings group labels, Undo in toasts, Clear in Downloads, the Commands
  chip, devtools labels), the caret and keyboard focus rings stay readable with any accent and
  theme; placeholders and secondary text meet 4.5:1 in every built-in theme, red warning text is
  readable in dark mode, sleeping tabs follow the theme's secondary text colour, and site initials
  and extension badges pick a readable letter colour (macOS, Windows, Linux).
  On iOS and Android, Done, Clear and Cancel, the focused address field, sleeping tab cards and
  extension badges follow the same rules.
- Every button, segment and colour swatch can be reached with Tab and pressed with Enter or Space
  where a window moves the focus on Tab, and a button that cannot be used looks dimmed (macOS,
  Windows, Linux).
- Menu bar commands with nothing to act on are greyed on macOS, as they were on Windows: Back,
  Forward, Reopen Closed Tab, Close All Tabs, Stop Loading, and Mute Site on a page without a host.
- File › Close Tab reads Unload Tab on a pinned tab, View › Reader reads Show or Hide Reader,
  Tabs › Search Tabs… is now Show Library, and the Spaces menu lists every space, not only the
  first nine (macOS, Windows; Show or Hide Reader on Linux).
- Menus opened from the start page's space and Shield pills and the sidebar's space name drop
  from the button instead of the pointer, and entries inside submenus get their glyphs (macOS).
- In the reader, ⌘F finds on the page the reader was made from, and ⌘W closes the tab instead of
  the reader; Esc still closes the reader (macOS, Windows, Linux).
- Zooming shows the new zoom level, and says why nothing changes while Responsive Size is on
  (macOS, Windows, Linux).
- The Keyboard Shortcuts sheet scrolls with the arrow keys, Page Up and Page Down, Space, Home and
  End (macOS, Windows, Linux).
- Segments, Shield level rows, accent swatches and theme cards in Settings take the keyboard with
  Tab, Enter and Space (macOS, Windows).
- A colour typed in the theme colour editor is saved when its field loses focus, not only on
  Return, and turning on Match system moves a light theme picked before into the light slot
  (macOS, Windows).
- Reduce motion shows as on while it is on in the system settings (macOS, Windows).
- Settings show changes made in another window or the command bar; Return in the GitHub fields of
  Extensions and Appearance installs; Choose File says when the file is not a .zip or .crx; and
  extension Install and update buttons look unavailable, with the reason, while downloads are off
  (macOS, Windows).
- A profile name another profile has says so instead of only greying out Save, and a hidden
  element comes back with Show Again rather than Undo (macOS, Windows, Linux).
- The Library panel stays within a narrow sidebar, and a list taller than the window ends below a
  row instead of cutting off the next heading (macOS, Windows).
- Pages follow Tarn's light or dark theme (`prefers-color-scheme`) instead of the system's
  appearance, and switch with it (macOS, Windows, Linux, iOS, Android).
- Find in page searches as you type, shows "1 of 2" where the engine reports the current match
  (Chromium, Android), and clears its highlight when the bar closes (macOS, Windows, Linux, iOS,
  Android).
- A toast stays while the pointer is over it, wraps a long message onto a second line, and its
  button is also a command row, so it can be reached from the keyboard (macOS, Windows, Linux; on
  Linux the button is also a tab stop).
- Translating a page shows Translating… while it works and then Show Original, which reloads the
  page in its own language (macOS).
- The Reader button shows when Reader is on and then reads Exit Reader, and the Shield button's
  tooltip says how many requests it stopped on the site (macOS, Windows).
- The address field keeps Reader open behind it and shows a sleeping tab's address; its
  suggestions no longer offer the current tab, show one Switch to Tab row for tabs on the same
  address, and Enter in an empty field opens nothing (macOS, Windows, Linux).
- Show QR Code and the system share sheet are offered only for web pages; on a file or other
  address Show QR Code says why (macOS, Windows, Linux).
- Failed and cancelled downloads stay in the Downloads list, a download in progress has a Cancel
  button, Clear offers Undo, a new download no longer hides a dangerous-file warning, and the
  Downloads panel stays inside the window (macOS, Windows, Linux, iOS, Android).
- Two downloads of the same name at once each get their own file, instead of the second failing
  or replacing the first (Linux, iOS, Android).
- Settings text no longer runs under the controls beside it, and text fields clip their
  placeholder.
- The settings window's close, minimise and zoom buttons respond on their whole area.
- In a split, an unfocused pane's header uses its full width for the address.
- After closing a tab, the tab that moves under the pointer shows its close button at once.
- Library rows show their edit and delete buttons side by side inside the row on hover.
- The Shield card's actions in Settings › Profile sit in an even grid of buttons with icons
  (macOS, Windows, Linux, iOS, Android).
- The start page's space and Shield pills open the space menu and a Shield level menu (macOS,
  Windows, Linux), and the start card keeps its suggestions inside the window, scrolling when the
  window is short.
- The reload arrow turns smoothly while a page loads: about 30 even steps a second on macOS and
  Windows, the toolkit's own animation on Linux, iOS and Android.
- Dark mode for a site leaves pages that are already dark as they are instead of inverting them to
  light (macOS, Windows, Linux, iOS, Android).
- Linux: links from other apps open in Tarn. The desktop entry made Tarn a browser for http and
  https links, but the URLs it was started with were ignored; now they, and those of a later
  launch, open as new tabs in the running Tarn, under the same rules as on macOS.
- Page actions are greyed or left out while no tab is open: reader, copy link and title, duplicate,
  pin, share, QR code, screenshot, PDF, save for later, bookmark, find, zoom and split view
  (macOS, Windows, Linux; on iOS and Android Find in Page and Zoom).
- Bookmark Tab, Bookmark All Tabs and Save for Later keep the page's whole title, cleaned of
  control and bidi characters, instead of a shortened one, and renaming starts from the saved name
  (macOS, Windows, Linux; Save for Later on iOS and Android).
- A profile name longer than 48 characters is no longer cut when saved: the name field says "Use
  at most 48 characters." and Continue, Create Profile or Create Space stays off until it fits
  (macOS, Windows, Linux, iOS, Android).
- A finished download whose file was moved or deleted reads "Missing · moved or deleted" with a
  warning, and offers no Reveal or Show in Folder, in the Downloads panel and the Library
  (macOS, Windows, Linux).
- History rows show when the page was visited and its site on a second line, so the title gets
  the row's width, and ⌘L (Ctrl+L) from the Library opens the address field (macOS, Windows,
  Linux; the second line on iOS and Android).

## [0.1.2] - 2026-11-17

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
- Extension store: an "Install in Tarn" link on extensions.tarnbrowser.com opens a confirmation
  in Settings › Extensions built only from Tarn's verified catalog, and nothing is downloaded
  until you press Install there; Settings › Extensions links the store (macOS, Windows, Linux,
  iOS, Android).
- Site icons in tiles, tab rows, split panes, history, bookmarks, suggestions and the site and share
  popovers on Linux, and on tab cards and the share sheet on iOS, kept per profile as on macOS.
  Android shows letter tiles and makes no icon request of its own.
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

- The privacy notice names Quavon UG (haftungsbeschränkt) as controller, gives the legal basis and
  your rights, and states per client what it describes.
- Toasts are centred pills sized to their message on every client. A copied link or a saved page
  shows a green check, and a failure such as a Reading List save that did not work shows an amber
  warning.
- The Shield card follows the theme: dawn with the sun behind the peak in light themes, night
  with stars, a crescent and an aurora in dark ones; New Design still reshapes the ridges (macOS,
  Windows, Linux, iOS, Android).
- The sidebar opens with a space switcher, downloads and the profile's avatar; the selected tab is
  an outlined row with its close button, pinned and sleeping tabs show a glyph, and Library, the
  spaces, New Space and Settings sit at the bottom (macOS, Windows, Linux); Clean Up is a button
  beside New Tab Group and New Folder (macOS, Windows).
- The address field starts with a Shield chip that shows how much Shield stopped on the site and
  opens Site Settings; Split View and Share sit beside it, and Settings moved to the sidebar
  (macOS, Windows, Linux). On phones the address and the Shield chip sit in the bottom bar above
  Back, Forward, New Tab, Tabs and Menu (iOS, Android).
- Typing an address or `>` opens a centred palette over the page with sections, a glyph per row,
  shortcuts as key caps and key hints (macOS, Windows, Linux).
- Site Settings shows what Shield stopped on the site, the site's Shield switch, what the chosen
  level does and the site's options in one grouped list; rarer ones sit behind More site settings
  (macOS, Windows, Linux).
- Share opens a menu with Copy Link, Copy Clean Link, Show QR Code and, where the system offers
  them, More…, Save Page as PDF and Print… (macOS, Windows, Linux); on phones a share sheet slides
  up (iOS, Android).
- Permission prompts lead with a glyph, the site in bold and a line on what allowing means; the
  find bar is a pill at the page's top right; downloads show a file-type tile, the size and a
  progress bar (macOS, Windows, Linux, iOS, Android).
- Settings groups its panes under You and Browser; the Profile pane shows the card beside its
  numbers and Shield level with Flip, Change Design and Share Card, one chart of the week and the
  most protected sites; Appearance is a grouped list (macOS, Windows, Linux, iOS, Android).
- Menus Tarn builds show a glyph beside each entry (macOS, Windows, Linux). Popovers, sheets and
  prompts pop in and out, switches slide, tab rows grow and shrink, all with the shared timings
  and none under Reduce Motion (macOS, Windows, Linux, iOS, Android).
- A split's tabs show side by side in one row of the sidebar, and in one card of the tab grid on
  iPad and Android tablets (macOS, Windows, Linux, iOS, Android).
- The app identifier is `de.quavon.tarn` (was `dev.tarn.browser`) on macOS, iOS and Android. On
  macOS the first start moves the website data over; Tarn has to be made the default browser again,
  camera and microphone permissions are asked again, and a saved Anthropic API token has to be
  entered again.
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
- Windows: the installer always installs into the current user's `AppData\Local\Programs\Tarn`,
  also after the user profile moved to another drive, and a link set up with Make Default follows
  the install to its new folder. Without `LOCALAPPDATA` in its environment, Tarn keeps its data in
  the Local AppData folder Windows reports instead of the folder it was started from.

### Security

- On Android, Tarn's own requests and file downloads refuse a server whose verified certificate
  path, root included, holds an RSA key under 2048 bits or an elliptic-curve key under 224 bits.
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

[Unreleased]: https://github.com/Quavon-dev/tarn-browser/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/Quavon-dev/tarn-browser/compare/v0.1.0...v0.1.2
[0.1.0]: https://github.com/Quavon-dev/tarn-browser/releases/tag/v0.1.0
