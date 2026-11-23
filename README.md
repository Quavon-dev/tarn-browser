<div align="center">

<img src="docs/media/icon.png" width="112" alt="" />

# Tarn

**An open-source, keyboard-first browser with built-in tracker and ad blocking.**<br>
Switch tabs by typing, browse with separate profiles, and set privacy controls per site. Built in Rust, with no telemetry or accounts.

**[Download Tarn](https://tarnbrowser.com/download)** · [See the demo](#demo) · [Contribute](CONTRIBUTING.md)

[![CI](https://github.com/Quavon-dev/tarn-browser/actions/workflows/ci.yml/badge.svg)](https://github.com/Quavon-dev/tarn-browser/actions/workflows/ci.yml)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/Quavon-dev/tarn-browser/badge)](https://scorecard.dev/viewer/?uri=github.com/Quavon-dev/tarn-browser)
[![Licence: GPL-3.0-or-later](https://img.shields.io/badge/licence-GPL--3.0--or--later-blue.svg)](LICENSE)
![Platforms](https://img.shields.io/badge/platforms-macOS%20%C2%B7%20Windows%20%C2%B7%20Linux%20%C2%B7%20iOS%20%C2%B7%20Android-1f75b0.svg)

[Features](#features) · [Screenshots](#screenshots) · [Platforms](#platforms) · [Build](#build-and-run) · [Shortcuts](#shortcuts) · [Development](#development)

<a name="demo"></a>
<img src="docs/media/demo.gif" alt="Tarn demo: the address field switching to an open tab, a command opening Split View, link peek, Reader, the Shield site card, the shortcut sheet and the Ember theme" width="100%" />

<sub><a href="https://raw.githubusercontent.com/Quavon-dev/tarn-browser/main/docs/media/demo.mp4">Download the demo video (MP4)</a></sub>

</div>

> [!NOTE]
> **Status:** pre-1.0. Features, settings and stored data can change between releases without a
> migration path.

## Get Tarn

Start with the macOS DMG or a Linux package from the [latest GitHub release](https://github.com/Quavon-dev/tarn-browser/releases/latest).
Windows packages are provided as unsigned previews. iOS and Android builds are still in testing.
For platform requirements and installation options, visit the [Tarn download page](https://tarnbrowser.com/download).

## Features

<table>
<tr>
<td width="50%" valign="top">

### 🛡️ Private by default
**Shield** blocks trackers and ads, strips tracking parameters and keeps pages off your local
network. **Strict** adds HTTPS-only and no cross-site referrer; **Lockdown** turns off JavaScript
and keeps no site data. Every level can be set per site.

</td>
<td width="50%" valign="top">

### 🧱 Blocking you can switch
Ads (EasyList, AdGuard and uBlock filters, placeholders hidden), trackers, cookie banners, YouTube
ads, pop-ups and pop-up redirects, malware, phishing and scam sites, and adult sites if you want.
Lists are signed and updated daily; dangerous sites get a warning page drawn by Tarn.

</td>
</tr>
<tr>
<td valign="top">

### ⌨️ Keyboard first
One field for addresses, search, open tabs and app actions, with inline completion. A shortcut for
nearly everything, most of them rebindable; <kbd>⌘</kbd><kbd>/</kbd> lists them all.

</td>
<td valign="top">

### 🗂️ Spaces and profiles
Each space has its own name, colour and icon; profiles keep cookies, history and site data fully
apart. Private spaces and burner tabs live in memory only; the Dock menu (the jump list on Windows)
opens a New Private Window or a New Burner Window. Hosts can be routed to a space.

</td>
</tr>
<tr>
<td valign="top">

### 🪟 Tabs that stay out of the way
Vertical tabs, pinned tabs that remember their page, tiles, folders, tab groups, Split View and
link peek. Unused tabs sleep, and the heaviest background tabs sleep once they use over 1 GB (macOS).

</td>
<td valign="top">

### 🎨 Yours to restyle
Built-in themes, a colour editor and themes from a file, the clipboard, GitHub or the official
catalog. Reader view, per-site zoom, dark mode and CSS, Picture in Picture and extensions.

</td>
</tr>
</table>

Native on every platform: GPUI draws the macOS and Windows interface, and the same Rust core runs
the Linux, iOS and Android apps. Pages render in each platform's own engine.

## Screenshots

<table>
  <tr>
    <td width="50%"><img src="docs/media/hero.png" alt="Tarn on macOS with vertical tabs, pinned tabs and a tab group in the sidebar" /></td>
    <td width="50%"><img src="docs/media/address-bar.png" alt="The address field suggesting a search and open tabs to switch to" /></td>
  </tr>
  <tr>
    <td align="center"><sub><b>Sidebar</b> · pinned tabs, groups and sleeping tabs</sub></td>
    <td align="center"><sub><b>One field</b> · addresses, search and Switch to Tab</sub></td>
  </tr>
  <tr>
    <td><img src="docs/media/command-bar.png" alt="App actions with their shortcuts listed in the command bar" /></td>
    <td><img src="docs/media/split-view.png" alt="Two tabs side by side in Split View" /></td>
  </tr>
  <tr>
    <td align="center"><sub><b>Commands</b> · type <code>&gt;</code> or an action name</sub></td>
    <td align="center"><sub><b>Split View</b> · <kbd>⌃⌘N</kbd></sub></td>
  </tr>
  <tr>
    <td><img src="docs/media/site-settings.png" alt="Site card with the connection, the Shield level, pop-ups, mute, dark mode and zoom for this site" /></td>
    <td><img src="docs/media/reader.png" alt="A Wikipedia article in Reader view" /></td>
  </tr>
  <tr>
    <td align="center"><sub><b>Site settings</b> · Shield level and permissions per site</sub></td>
    <td align="center"><sub><b>Reader</b> · <kbd>⌥⌘R</kbd>, with Read Aloud</sub></td>
  </tr>
  <tr>
    <td><img src="docs/media/themes.png" alt="Appearance settings with the Ember theme applied to Tarn" /></td>
    <td><img src="docs/media/shortcuts.png" alt="The keyboard shortcut sheet" /></td>
  </tr>
  <tr>
    <td align="center"><sub><b>Themes</b> · Ember, one of the built-in themes, or your own CSS</sub></td>
    <td align="center"><sub><b>Shortcut sheet</b> · <kbd>⌘/</kbd></sub></td>
  </tr>
</table>

### iPhone, iPad and Android

<table>
  <tr>
    <td width="25%"><img src="docs/media/store/ios-phone/02-browse.png" alt="Tarn on iPhone showing a Wikipedia article with the address bar at the bottom" /></td>
    <td width="25%"><img src="docs/media/store/ios-phone/03-shield-card.png" alt="The Shield card on iPhone counting tracking parameters removed on this device" /></td>
    <td width="25%"><img src="docs/media/store/android-phone/05-tabs.png" alt="The tab grid on Android" /></td>
    <td width="25%"><img src="docs/media/store/android-phone/06-themes.png" alt="Tarn on Android in the Ember theme with the theme picker open" /></td>
  </tr>
</table>
<table>
  <tr>
    <td width="50%"><img src="docs/media/store/ios-tablet/01-split.png" alt="Split View on iPad with the WebKit blog and a Wikipedia article side by side" /></td>
    <td width="50%"><img src="docs/media/store/android-tablet/01-split.png" alt="Split View on an Android tablet with Hacker News and the WebKit blog side by side" /></td>
  </tr>
</table>

<sub>App Store and Google Play sets at their store sizes: <a href="docs/media/store/"><code>docs/media/store/</code></a>.</sub>

## Platforms

macOS is the lead client; every feature is meant to reach all five.

| Platform | Interface | Web engine | Requires |
|---|---|---|---|
| **macOS** | GPUI | System WebKit (Chromium optional) | macOS 14 Sonoma or later, Apple silicon or Intel |
| **Windows** | GPUI | Chromium (CEF) | Windows 10 or 11, x64 |
| **Linux** | [Native Shell](https://nsh.dev/) | WebKitGTK 6.0 | Debian 13, Ubuntu 26.04, Fedora 43 or Arch |
| **iOS / iPadOS** | Native Shell | `WKWebView` | iOS 17 or later |
| **Android** | Native Shell | System WebView | Android 9 or later |

Shared by all of them: `browser-core` (state and decisions), `engine`, `shield` (blocking and
policy), `store` and `sync`. Contract: [`docs/shared-core.md`](docs/shared-core.md); security
model and per-platform differences: [`docs/threat-model.md`](docs/threat-model.md).

## Build and run

### macOS

Needs the Xcode command line tools and the Rust toolchain pinned in `rust-toolchain.toml` (rustup
installs it on first use).

```sh
cargo run -p app --release
cargo run -p app -- https://example.com   # open a URL in a new tab
```

Profile data lives in `~/Library/Application Support/Tarn`; set `TARN_DATA_DIR` to use another
directory. Debug builds also read `TARN_TEST_FILTER_KEY`, a 64-digit hex Ed25519 public key that
replaces the filter-list signing key so tests can install bundles they signed themselves, and
`TARN_TEST_CATALOG_KEY`, which replaces the extension catalog's signing key in the same way;
release builds ignore both.

### Windows

From Git Bash with the MSVC toolchain; Chromium comes from the pinned CEF build:

```sh
export CEF_PATH="$(cygpath -w "$(scripts/fetch-cef.sh windows64)")"
cargo run -p app --release
```

Details, installer and signing: [`docs/windows-plan.md`](docs/windows-plan.md).

### Linux, iOS and Android

Each has its own Cargo workspace on [Native Shell](https://nsh.dev/) 0.5.0, so NSH stays out of the
root workspace.

| Directory | What |
|---|---|
| `nsh-shared/` | Theme tokens, icons and controls shared by the three apps (`cargo test` there) |
| `linux/` | The Linux desktop app ([`linux/README.md`](linux/README.md)) |
| `mobile/` | The phone and tablet app (`app/`), the iOS web view (`ios/`) and the Android host app (`android/`, [`mobile/android/README.md`](mobile/android/README.md)) |

```sh
# Linux, in a Fedora 43 container (no Linux desktop needed)
docker build -t tarn-linux-fedora -f linux/docker/fedora.Dockerfile linux/docker
linux/scripts/cargo.sh test --locked && linux/scripts/test-display.sh
linux/packaging/build-all.sh deb-debian|deb-ubuntu|rpm|arch    # .deb, .rpm, AUR package

# Mobile (nsh 0.5.0 on PATH: https://nsh.dev/docs/installation)
cd mobile && cargo test --locked                               # shell and UI tests on the host
cd mobile/app && nsh package --target ios-simulator            # iOS simulator app
mobile/android/build.sh                                        # debug APK (ANDROID_HOME, NDK, JDK 17+)
```

The Linux app and its packages build with free and open-source tools only: the Fedora, Debian,
Ubuntu and Arch containers (`linux/docker/`, `linux/packaging/`) bring GCC, Clang, GTK, WebKitGTK
and Rust from their distributions and rustup, every crate has an open-source licence (`cargo
deny`), and Docker runs them. The macOS and iOS clients need Apple's SDK and the Windows
client Microsoft's MSVC toolchain.

Packages: `.deb` for Debian 13 and Ubuntu 26.04, `.rpm` for Fedora 43 and an AUR `PKGBUILD`, each
depending on the distribution's WebKitGTK 6.0; no Flatpak or AppImage. The iOS and Android apps have
no signed or store builds yet. Plans: [`docs/nsh-plan.md`](docs/nsh-plan.md) and
[`docs/mobile-plan.md`](docs/mobile-plan.md).

Verified so far: the Linux app in Docker under Xvfb (Fedora 43; packages installed and started on
Debian 13, Ubuntu 26.04, Fedora 43 and Arch); the mobile app in the iOS simulator (iPhone 17, iPad
Pro 13-inch) and the Android emulator (phone and 10-inch tablet, Android 16). Not yet: a real Linux
desktop, HiDPI, IME, VoiceOver and TalkBack, physical iPhones, iPads or Android devices, and amd64
Linux packages.

## Shortcuts

The macOS defaults (Windows and Linux use <kbd>Ctrl</kbd> where macOS uses <kbd>⌘</kbd>). Most can
be changed in Settings › Shortcuts; tab and space numbers, Esc, Undo/Redo and ⌘Q stay fixed. On
Linux, Reader is <kbd>Ctrl+Alt+E</kbd>, since <kbd>Ctrl+Alt+R</kbd> rotates the split layout.

| | |
|---|---|
| <kbd>⌘T</kbd> / <kbd>⌘L</kbd> | New tab / edit the current address |
| <kbd>⌘W</kbd> / <kbd>⇧⌘T</kbd> | Close tab / reopen closed tab |
| <kbd>⌘1</kbd> … <kbd>⌘9</kbd> · <kbd>⌃⇥</kbd> | Select a tab · next tab |
| <kbd>⌃1</kbd> … <kbd>⌃9</kbd> | Switch space |
| <kbd>⌃⌘N</kbd> | Split View |
| <kbd>⇧</kbd>-click a link | Link peek |
| <kbd>⌥⌘R</kbd> | Reader |
| <kbd>⌘S</kbd> | Show or hide the sidebar |
| <kbd>⇧⌘N</kbd> | Private space |
| <kbd>⌘/</kbd> | All shortcuts |

<details>
<summary><b>Full shortcut list</b></summary>

| Keys | Action |
|---|---|
| ⌘N | New window |
| ⌘T | New tab: the command palette opens over a shown page, otherwise the command card, with the cursor in it |
| ⌘L | Edit the current tab's address in the command palette |
| → | In the command palette, with the cursor at the end: the highlighted row's actions (Esc goes back) |
| ⌘W | Close tab (pinned tabs and tiles are unloaded, not removed) |
| ⇧⌘T | Reopen closed tab |
| ⇧⌘K | Close all tabs |
| ⌘1 … ⌘8 / ⌘9 | Select tab 1–8 / the last tab (in sidebar order) |
| ⌃⇥ / ⌃⇧⇥ | Next / previous tab (wraps around) |
| ⇧⌘C | Copy link of the current tab |
| ⌥⇧⌘C | Copy link as Markdown, `[title](url)` (Copy Title is in the Tabs menu) |
| ⌃⌘N | Open or close Split View: the current tab beside the space's last used tab outside a split, or a copy of it |
| ⌃⌘] / ⌃⌘[ | Focus the next / previous split pane |
| ⌃⌘S | Swap the focused split pane with the next one |
| ⌃⌘R | Rotate the split layout: side by side or stacked |
| ⌃⌘W | Remove the current tab from Split View |
| ⌥⌘S | Screenshot of the visible page to Downloads and the clipboard (Save Page as PDF is in the File menu) |
| ⌃⌘P | Picture in Picture for the page's largest playing video |
| ⌘/ | Keyboard shortcut cheat sheet |
| ⌘D | Pin or unpin the current tab |
| ⌥⌘G | New tab group (with the current tab) |
| ⇧⌘D | Bookmark the current tab |
| ⇧⌘A | Library: open tabs, history and bookmarks |
| ⇧⌘J | Recent downloads |
| ⌥⌘B / ⌘Y | Manage bookmarks / show history (open the library) |
| ⌥⌘R | Reader |
| ⌥⇧⌘N | Burner tab |
| ⌘M | Minimize the window |
| ⌃⌘F | Enter or exit full screen |
| ⌘H / ⌥⌘H | Hide Tarn / hide other apps (Show All is in the Tarn menu) |
| ⌘[ / ⌘] | Back / forward |
| ⌘R | Reload (the arrow spins while the page loads) |
| ⇧⌘R | Reload without cache |
| ⌘F / ⌘G / ⇧⌘G | Find in page / next / previous |
| ⇧⌘F | Find in tabs |
| ⌘P | Print |
| ⌘S | Show or hide the sidebar |
| ⌥⇧⌘T | Tabs on top: a strip of tabs above the page in place of the sidebar, and back |
| ⌘, | Settings window (Shield level, site overrides, history); ⌘W or Esc closes it |
| ⌘= / ⌘- / ⌘0 | Zoom in / out / reset |
| ⌃1 … ⌃9 | Switch space |
| ⇧⌘S | New space: pick a profile or create a new, fully separate one; name, colour and icon |
| ⇧⌘N | Private space (memory only) |
| ⌘Z / ⇧⌘Z | Undo / redo |
| ⇧-click a link | Open it in a peek over the page; Open in Tab keeps its history, Esc or a click outside closes it |

Trackpad (Settings › Tabs › Trackpad gestures): swipe left or right over the sidebar to switch
spaces; swipe over the address field to step through tabs; swipe right over the toolbar to show the
sidebar. The mouse's back and forward buttons always navigate.

</details>

<details>
<summary><b>How tabs, spaces and the address field behave</b></summary>

In the address field and the command bar, an address already open in the profile offers Switch to
Tab first (other spaces show their name). An action name ("settings", "new space") or a leading `>`
lists app actions with their shortcuts. Typing the start of an address you have typed or bookmarked
before, or have open, completes it in place (the added part selected: type on to replace it,
Backspace drops it, → keeps it); arrows or Tab / ⇧Tab move the selection, Enter runs it, Esc or a
click elsewhere closes the field.

Mute Site (Tabs menu, a tab's menu or the toolbar's menu) silences a host in the profile; an unmute
button shows in the address field while the page is muted.

Drag tabs between the tile grid, the pinned section, folders and the tab list. The Shield badge in
the address field opens Site Settings for the page: Shield level for the site, zoom, mute, the
permissions remembered for it, Clear Site Data… and whether the connection is encrypted; the
profile-wide level is set in Settings. Its "Always open in" picks a space for the host: new tabs of
it (links from other apps, ⌘-clicks, links that open a new window and addresses typed into a new
tab) then open there, while a script's pop-up (which keeps its opener) and links followed inside a
tab stay put. Settings › Profiles › Space Rules lists and removes them; the private space neither
sends tabs elsewhere nor receives them.

A pinned tab or tile remembers the page it was pinned on; once it shows another page its row gets a
↩ button (Return to &lt;host&gt; in the tab's menu and the Tabs menu, next to Set Pinned URL to
Current Page). Rename… (or a double-click on the row) gives a tab its own title until Reset Title.
Tabs without a live page are dimmed; Sleep Tab and Sleep Other Tabs unload them on demand.
Library › Memory (or Show Memory Use in the command bar) lists the space's tabs by the memory and
CPU of their web content process, with Sleep per tab and Sleep All Inactive Tabs; the sidebar's
heavy-tab dot ranks by the same figures. Once awake background tabs hold more than 1 GB together,
the heaviest of those unused for ten minutes sleep, pinned tabs only after 30 minutes unused (tabs
playing media stay awake; off when sleeping is Never). On macOS and Windows, a background tab whose
page has form input you typed stays awake until you sleep it yourself (a page can use this to keep
its own tab awake, as with media), and a tab that slept scrolls back to where you left it; neither
applies under Lockdown, where no script of Tarn's runs on the page. Chromium tabs count by their
renderer process. Resting the
pointer on a tab row shows its full title, address and state. Right-click a space (its name or
icon) and choose Edit Space… to change its name, colour and icon.

</details>

### Search shortcuts

With Settings › Search › Search shortcuts on (it is off at first), a `!key` before or after your
words searches that site instead of your search engine: `!yt cats` or `cats !yt`. A shortcut on its
own opens the site. Common ones: `!g` Google, `!b` Bing, `!ddg` DuckDuckGo, `!yt` YouTube, `!w`
Wikipedia, `!wa` Wolfram|Alpha, `!gh` GitHub, `!so` Stack Overflow, `!mdn` MDN, `!crates`
crates.io, `!npm` npm, `!r` Reddit, `!a` Amazon, `!gm` Google Maps, `!gt` Google Translate. The
full list is `BANGS` in `crates/browser-core/src/command.rs`.

### Passkeys

Settings › Privacy › Passkeys and security keys (on at first) lets sites ask for a passkey or a
security key to sign you in (WebAuthn); off, WebKit tabs hide passkey sign-in from every site and
open pages follow once they reload (macOS). Passkeys saved in Apple Passwords (iCloud Keychain)
also need Apple's `com.apple.developer.web-browser.public-key-credential` entitlement in the signed
app, which Apple grants per developer team; a build without it offers security keys only. Tarn
cannot fill passwords from Apple Passwords: macOS gives third-party browsers no API to read them
(AuthenticationServices' password APIs serve credential providers and an app's own domains).

## Development

Checks per workspace (each runs in its own directory; CI runs all of them):

| Where | Command |
|---|---|
| root (`crates/*`) | `cargo fmt --all --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked` |
| `nsh-shared/` | same |
| `mobile/` | same, plus `cargo clippy --target aarch64-apple-ios-sim --all-targets --locked -- -D warnings` |
| `linux/` | same, plus `linux/scripts/display-tests.sh` |
| Android build | `mobile/android/build.sh` |

`cargo deny check` runs where a workspace has a `deny.toml`. `unsafe` code is allowed only in
`crates/engine` (and later `crates/platform`); every other crate is `#![forbid(unsafe_code)]`.
Start with a [good first issue](https://github.com/Quavon-dev/tarn-browser/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22)
and read [`CONTRIBUTING.md`](CONTRIBUTING.md); AI-assisted contributions follow
[`AGENTS.md`](AGENTS.md). All of Tarn's code, packaging and release automation live in this
repository; no other repository is part of the codebase.

<details>
<summary><b>On-device translation</b></summary>

On-device translation is behind the `translation` feature (`cargo run -p app --features
translation`; `TARN_EXTRA_FEATURES=app/translation scripts/bundle.sh` for a bundle); it needs a
Swift toolchain with the macOS 26 SDK. Its build script reads `MACOSX_DEPLOYMENT_TARGET` (default
`14.0`) for the Swift shim, which weak-links the Translation framework, and Cargo's own `OUT_DIR`
and `CARGO_CFG_TARGET_ARCH`. `TARN_TEST_TRANSLATION=1` runs its integration test, which needs an
installed German–English language pack. The framework must stay weak-linked so the app starts on
macOS 14: `otool -l target/debug/tarn | grep -A2 LC_LOAD_WEAK_DYLIB` lists
`Translation.framework`.

</details>

<details>
<summary><b>Startup timing</b></summary>

`TARN_STARTUP_TRACE=1` makes the app print `tarn-startup <unix ms> <ms since main> <step>` to
stderr when each startup step first finishes: `main`, `store open`, `first window render`,
`built-in rules attached`, `youtube rules attached`, `filter lists attached` and `first page
commit`. Without the variable nothing is printed; the output never names a URL.
`scripts/startup-trace.sh [runs] [url]` builds the release binary, launches it `runs` times
(default 5) against a temporary data folder, restoring one tab of `url` (default
`https://example.com/`), and prints each step's median. It opens windows and loads a page over the
network, so it runs on a Mac desktop session, not in CI; compare its output before and after a
startup change on the same machine. WebKit keeps the temporary profile's website data store under
`~/Library/WebKit`.

`TARN_FRAME_TRACE=1` prints `tarn-frame <section> <µs>` to stderr for every render: the time
since the previous one (`interval`), the whole render and its `sidebar`, `toolbar` and `content`
parts, and `tarn-frame resize` or `tarn-frame move` each time a web view's native frame changes size
or only position; Windows sets the frame on every call, so there every call prints one of them. Render times cover
building the element tree; GPUI's layout and paint show in `interval`. Every client also prints
`tab-switch`, `space-switch` and `palette-query` for each switch and each query typed into the
address or command field. The output carries section names and numbers only.
`cargo run --release -p engine --example frame-summary < log` prints each section's count, median
and 95th percentile.

</details>

<details>
<summary><b>Filter-list signing key</b></summary>

`FILTER_SIGNING_KEY` holds the base64 Ed25519 seed that `filter-bundle build` signs filter-list
bundles with. It lives only in a GitHub Actions secret used by the `Filter lists` workflow;
`filter-bundle keygen` prints it once to the terminal during setup. Never store it in the
repository. Setup and rotation: [`docs/filter-lists.md`](docs/filter-lists.md).

</details>

<details>
<summary><b>Packaging (macOS)</b></summary>

```sh
TARN_SIGN_IDENTITY=<team identity> scripts/bundle.sh   # target/release/Tarn.app with CEF, helpers and pinned Sparkle
scripts/check-entitlements.sh   # fails if the signed entitlements differ from the expected set
scripts/third-party-licenses.sh # regenerates the licence lists of all five clients in packaging/ after a dependency change
```

`scripts/bundle.sh` builds for the Mac it runs on; there is no universal binary.
`TARN_SIGN_IDENTITY=none` leaves the bundle unsigned for CI's signing job; an ad-hoc signature
cannot load the CEF framework. The app runs without the App Sandbox (Chromium's helpers and
WebKit's content processes keep theirs): its data lives in `~/Library/Application Support/Tarn`
and `~/Library/WebKit/de.quavon.tarn`, and the first start after a sandboxed version moves it
there once. WebKit stays the default engine; Chromium is offered while automatic updates are on.
A bundle whose update feed it cannot reach (a private repository, or no release yet) never gets a
verified check and so never opens Chromium; `TARN_LOCAL_CHROMIUM=1 TARN_SIGN_IDENTITY=<identity>
scripts/bundle.sh` builds one that opens it until 28 days after its CEF release, for your own Mac.
The bundle carries Chromium's credits, CEF's and Sparkle's licences and the Rust crates' licences
(Settings › General › About). Signing, notarisation and releases are described in
[`docs/release.md`](docs/release.md).

</details>

## Privacy and security

Tarn sends no telemetry and has no accounts. The connections the browser itself makes are listed in
[`PRIVACY.md`](PRIVACY.md); report vulnerabilities privately as described in
[`SECURITY.md`](SECURITY.md).

## Licence

Tarn is free software under the GNU General Public License, version 3 or (at your option) any later
version (`GPL-3.0-or-later`); see [`LICENSE`](LICENSE). An additional permission in
[`LICENSE-APP-STORE.md`](LICENSE-APP-STORE.md) allows only Quavon UG (haftungsbeschränkt) to
distribute Tarn through app stores such as Apple's App Store. The Tarn name, logo and app icon are
trademarks; forks need a name and icons of their own ([`TRADEMARKS.md`](TRADEMARKS.md)).
Third-party licences shipped with the app are in
[`packaging/third-party-licenses.txt`](packaging/third-party-licenses.txt).
