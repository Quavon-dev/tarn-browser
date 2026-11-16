# Security policy

## Reporting a vulnerability

Report vulnerabilities privately through GitHub: open the repository's **Security** tab and choose
**Report a vulnerability**. Please do not open a public issue, pull request or discussion for a
security problem.

Include:

- the Tarn version or commit;
- the client and its operating system version: macOS, Windows, Linux (distribution and WebKitGTK
  version), iOS or Android (and the Android System WebView version);
- the engine the tab used: WebKit or Chromium on macOS, Chromium on Windows, WebKitGTK on Linux,
  WKWebView on iOS, Android System WebView on Android;
- the Shield level, steps to reproduce and the impact you expect.

## What to expect

- Acknowledgement within 3 working days.
- For a high-severity issue in Tarn's own code, the target is a fix release within 7 days of
  confirmation.
- Credit in the release notes, unless you ask not to be named.

## Supported versions

Tarn is in pre-release. Only the latest release receives security fixes; please check that a
problem still occurs there or on `main`.

## Scope

In scope: everything in this repository on every client, including:

- the shells and the shared crates: Shield and its filter lists, navigation and request policy,
  private spaces and burner tabs, cookies and website data, downloads, imports from other
  browsers, sync, themes and website styles, extensions handling, the AI integration and the CLI
  bridge;
- isolation between web pages and Tarn: no page-to-shell bridge, isolated script worlds where the
  engine has them, and the cleanup of private data;
- update and delivery channels: the macOS update feed, the Windows update feed, the signed filter
  bundle, the theme and extension catalogs, and the Linux apt and dnf repositories with their
  signing;
- Tarn shipping an outdated engine where Tarn bundles it: Chromium (CEF) on macOS and Windows. A
  fixed Chromium vulnerability that a current Tarn release still ships is a Tarn issue.

Out of scope, report upstream instead:

- **WebKit** (rendering, JavaScript, TLS, content-process sandbox) on macOS and iOS: Apple
  (<https://security.apple.com>). Tarn uses the system WebKit and cannot ship WebKit fixes.
- **WebKitGTK** on Linux: the WebKit project (<https://webkit.org/security-policy/>). Tarn's
  distribution packages use the distribution's WebKitGTK, so fixes arrive with distribution updates;
  the Flatpak uses the WebKitGTK of the GNOME Flatpak runtime, so fixes arrive with runtime updates.
- **Chromium** engine bugs, including the Android System WebView: Google
  (<https://bughunters.google.com>). If Tarn's bundled Chromium is behind a fixed release, report
  that to us as well.
- **CEF** (the Chromium Embedded Framework) itself: the CEF project
  (<https://github.com/chromiumembedded/cef>).
- The operating systems, the Claude CLI, Ollama, browser extensions and third-party filter lists:
  their vendors or maintainers. A problem in how Tarn handles them is in scope.

The threat model is in [`docs/threat-model.md`](docs/threat-model.md).
