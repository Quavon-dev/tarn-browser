# Architecture

Who and what acts on a released Tarn, what each can do, and every interface the released apps
offer to the outside. Threats and mitigations for each boundary are in `threat-model.md` (the S
numbers below); the connections the shell makes are listed in `../PRIVACY.md`; the shared core
and the web view contract in `shared-core.md`.

## Components

| Component | Code | Runs on |
|---|---|---|
| Shell: windows, tabs, address bar, settings, panels | `crates/app`, `crates/ui` (macOS, Windows); `linux/`; `mobile/app` with `mobile/ios`, `mobile/android` | every client |
| Shared core: browser state, navigation policy, routing, downloads, data | `crates/browser-core`, `crates/store`, `nsh-shared` | every client |
| Shield: filter lists, request and navigation blocking, tracking-parameter removal | `crates/shield` | every client |
| Engine bindings: the web view contract over each platform's engine | `crates/engine` (WebKit, Chromium/CEF), `linux/src/webview` (WebKitGTK), `mobile/ios` (WKWebView), `mobile/android` (Android System WebView) | every client |
| Platform services: updater, XPC | `crates/platform` | macOS, Windows |
| Import from other browsers | `crates/import` | macOS |
| AI integration and its CLI helper | `crates/ai`, `crates/cli-bridge` | not built into any client |
| Sync file format | `crates/sync` | not built into any client |
| Release and filter-list pipelines | `.github/workflows/`, `crates/filter-bundle` | GitHub Actions |

## Actors and their actions

| Actor | Trust | Actions on Tarn | Threat model |
|---|---|---|---|
| User | Trusted | Opens sites, types URLs and searches, changes settings and Shield levels, installs extensions and themes, imports from another browser, downloads files, uses private spaces | S5, S6, S8, S9, S11 |
| Web content (pages, frames, scripts, servers) | Untrusted | Renders in the engine's sandboxed content processes; navigates, opens popups and dialogs, requests permissions and external schemes, starts downloads, sets cookies and storage. No bridge into the shell | S1, S5, S6, S10, S14, S23, S24 |
| Extensions (macOS) | Partly trusted; the user chooses them | Run with the permissions the user grants per site, read the pages they are allowed on, open tabs through the navigation policy | S18 |
| Theme authors | Untrusted data | Change the shell's colours and, for the sites a theme names, their CSS once the user turns that on | S29 |
| The user's own site styles | Trusted input, contained | CSS the user writes for a site, applied only to that site | S28 |
| Cookie bridge (macOS, both engines) | Internal | Moves a profile's cookies between WebKit and Chromium when a tab changes engine | S26 |
| Other apps on the device | Untrusted | Ask the OS to open an `http`/`https` link in Tarn when it is the default browser (macOS, Windows, Linux), or start it with URLs as arguments (macOS, Windows); a second launch hands its URLs to the running instance | S6, S22 |
| Other browsers' profiles | Untrusted data | Read on the user's request by the importer (bookmarks, history, cookies) | S27 |
| GitHub release pipeline (maintainers, CI) | Trusted, gated by review and signing | Builds, signs and publishes releases, update feeds and filter bundles | S4, S13, S20 |
| Update and filter-list sources | Untrusted transport, signed content | Serve the Sparkle feed, the Windows feed and the filter bundle over HTTPS; the app accepts them only with a valid signature | S4, S20 |
| AI provider (Anthropic, or a local Ollama) | Untrusted output | Planned, not built into any client: would receive only the page content the user sends | S2, S3, S17 |
| Operating system and engine vendors | Trusted platform | Supply WebKit, WebKitGTK, Android System WebView and the OS; Tarn bundles only Chromium (CEF) | "Who patches what" |
| Local programs | Untrusted | Nothing to connect to: no listening socket, debugging port or local API | S22 |

## External interfaces

| Interface | Direction | Clients | Description |
|---|---|---|---|
| Web pages over HTTP(S) | out | all | Through the engine; HTTPS-only where the Shield level sets it; certificate errors always fail (S7). Algorithms: `cryptography.md` |
| Shell connections | out | all | Update checks, filter lists, extension and theme catalogs, site icons: `../PRIVACY.md`, "Connections the browser itself makes" |
| URL handler | in | macOS (`CFBundleURLSchemes`), Windows (`packaging/windows/tarn.iss`), Linux (`linux/packaging/de.quavon.tarn.desktop`) | `http`/`https` links from other apps open in a new tab after `nav_policy` (S6). Linux registers `x-scheme-handler/http(s)` in its `.desktop` file and reads the URLs of its launch and, through NSH's single instance (GApplication over D-Bus), of later launches; iOS and Android do not register as browsers |
| Command line and environment | in | macOS, Windows, Linux (URLs); macOS, Windows, Linux (`TARN_DATA_DIR`) | `tarn <url>…` opens URLs; Chromium switches on the command line are ignored (S22). `TARN_DATA_DIR` selects the profile directory |
| Files | in/out | all | Downloads into the user's download folder with sanitised names, quarantined on macOS (S11); `file:` URLs only by explicit user action; imports read other browsers' profile files (S27) |
| Keychain | in | macOS, Linux (Secret Service over the session D-Bus) | Reads another browser's Safe Storage key during import, only after the user allows it on macOS; on Linux the keyring may ask to be unlocked (S27) |
| Extension API | in | macOS | Safari web extensions through `WKWebExtension`, with per-site permissions (S18) |
| Update installers | in | macOS (Sparkle), Windows | Signed feeds and archives, verified before install (S4) |
| Linux package repositories | out (published) | Linux | apt and dnf repositories signed with the repository key, and the AUR package (`docs/release.md`) |
| Release assets | out (published) | all | GitHub releases with checksums, attestations and SBOMs (`../SECURITY.md`, "Verifying a release") |
