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

### Background tab memory budget (2026-11-15)

What it does: once a minute, awake background tabs whose web processes together hold more than
1 GB (split equally between windows) sleep, the heaviest first; a tab playing media, used in the
last minute (a pinned tab: 30 minutes), or in the split stays awake. Off when "Sleep background
tabs after" is Never.
Shared API: `engine::lifecycle::over_budget`, `engine::lifecycle::BACKGROUND_BUDGET`,
`engine::lifecycle::budget_idle`.
macOS reference: `crates/app/src/tabs.rs` (`sleep_idle_tabs`).

- [x] Windows — same code in `crates/app`; it sleeps nothing there while CEF names no renderer per tab (`docs/windows-plan.md`)
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
