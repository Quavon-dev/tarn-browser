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

## Coordinated disclosure

Private reports reach the repository's administrators listed in [`MAINTAINERS.md`](MAINTAINERS.md),
who bring in the other maintainers as needed.

1. We acknowledge the report within 3 working days and tell you whether we can reproduce it and
   how severe we rate it (CVSS) within 10 working days.
2. We fix it in a private fork of the advisory: high and critical issues within 7 days of
   confirmation, others in the next release, at the latest within 90 days of the report. We keep
   you informed and agree on a later date with you if a fix needs longer.
3. We publish the fix in a release whose notes list it under "Security", and on the same day
   publish the GitHub security advisory for this repository, with a CVE requested through GitHub,
   the affected and fixed versions, and credit.
4. Please keep the issue private until the advisory is published, or until 90 days after your
   report if we have not fixed it by then. A vulnerability we learn of while it is already being
   exploited or public is fixed and published as fast as possible.

Published advisories: <https://github.com/Quavon-dev/tarn-browser/security/advisories>. Every
vulnerability the maintainers rate as one in a released version is published there once fixed,
and every security-relevant fix is listed under "Security" in the release notes. A
vulnerability in an upstream engine is reported to its vendor (see "Scope"); when it affects the
Chromium that Tarn bundles, the update release notes name the Chromium version that fixes it.

## Supported versions

Tarn is in pre-release. Only the latest release receives security fixes; please check that a
problem still occurs there or on `main`.

Support covers every client in the release (macOS, Windows, Linux, iOS, Android) and lasts until
the next release is published. From that moment the earlier release receives no more fixes,
security fixes included. The macOS and Windows clients update themselves to it; Linux packages
update through the apt and dnf repositories and the AUR. Nightly builds are never supported. The
engine Tarn bundles (Chromium on macOS and Windows) is supported as part of the release; system
engines get their fixes from their vendors (see "Scope").

## Verifying a release

Every release on GitHub lists `SHA256SUMS`, and every binary carries a GitHub attestation signed
by the release workflow of this repository, naming the commit and tag it was built from. A
macOS-only release carries the previous release's `windows-update.json` and `.sig` over unchanged;
they are outside its `SHA256SUMS`, and their Ed25519 signature covers them. It also carries
`install.ps1`, which is listed in the `SHA256SUMS` of the release it names in its `$Tag` line;
check it there.

```sh
shasum -a 256 -c SHA256SUMS --ignore-missing
gh attestation verify <file> --repo Quavon-dev/tarn-browser --deny-self-hosted-runners \
  --cert-identity-regex '^https://github\.com/Quavon-dev/tarn-browser/\.github/workflows/release\.yml@refs/(tags/v[0-9]+\.[0-9]+\.[0-9]+|heads/main)$' \
  --predicate-type https://github.com/Quavon-dev/tarn-browser/attestations/release/v1 \
  --format json --jq '.[].verificationResult.statement.predicate.tag'
```

The attestation proves the file was built by `release.yml` of `Quavon-dev/tarn-browser` as it is on
`main` or on a release tag, on GitHub's own runners, so a file uploaded by anyone else or built from
another branch fails it; the command prints the tag it was built for, which must be the release you
meant to download (nightly builds print `nightly`). `SHA256SUMS` and the SBOMs are attested too. On
macOS, the app is also signed by Quavon UG (haftungsbeschränkt) and notarised by Apple: `codesign
--verify --deep --strict /Applications/Tarn.app` must succeed, `codesign -dv /Applications/Tarn.app`
must show `TeamIdentifier=CFV35FGSHF`, and `spctl -a -vv /Applications/Tarn.app` must say
`accepted`, `source=Notarized Developer ID`. The built-in updaters check this themselves: Sparkle
verifies its EdDSA-signed feed and archive, and the Windows updater the Ed25519-signed
`windows-update.json` and the installer's SHA-256 (`docs/cryptography.md`).

The Android APK on GitHub (`Tarn-<tag>-android.apk`) is signed with Tarn's sideload key. Its
certificate's SHA-256 fingerprint will be listed here when the key is created; releases carry no
APK until then. Once it is listed, `apksigner verify --print-certs Tarn-<tag>-android.apk` must
print it as the only signer's certificate SHA-256 digest. Nightly APKs carry the same key and
package name, so one installs over a release APK. Android refuses an update signed with another
key. Play Store installs are signed by Google Play with a different key, so you cannot switch
between a Play install and the APK without uninstalling first.

## Dependency and code scanning

Every pull request is checked by `cargo deny` (known vulnerabilities in RustSec, yanked crates,
licences, sources) on every Cargo workspace, by CodeQL (Rust, Java/Kotlin, GitHub Actions), by
Clippy with warnings as errors on every client, and zizmor reports workflow findings as code
scanning alerts. `main` takes no change while a required check fails: every job of `ci.yml`
(formatting, Clippy and tests of the shared crates and of every client, `deny` for `cargo deny` on
every workspace, `coverage`, and `dco`) and CodeQL; nor while a review conversation is unresolved.
Automatic Chromium update pull requests go
through the same checks and the same human review. The release workflow runs `cargo deny` again on
every workspace of the exact commit it releases, Chromium updates included, and signs nothing while
it fails.

- **Vulnerable dependencies:** any advisory against a dependency, whatever its severity, blocks
  merging and releasing. It is fixed by updating or removing the dependency, normally within 7
  days and before the next release. Only an advisory that cannot affect Tarn may be suppressed:
  in the workspace's `deny.toml` with the reason, and with a `not_affected` statement in the VEX
  document [`docs/vex.json`](docs/vex.json); CI fails when a suppression has no statement.
- **Licences:** a dependency whose licence is not on the allow list in `deny.toml` blocks merging
  and releasing. It is removed or replaced; the list changes only in a reviewed pull request.
- **Malicious or unknown sources:** crates come only from crates.io; a crate from another registry
  or a git dependency blocks merging (`[sources]` in `deny.toml`). A crate RustSec marks as
  malicious is an advisory and blocks like one.
- **Static analysis:** a CodeQL alert of high or critical security severity, or of error severity,
  blocks merging. Medium alerts are fixed within 30 days, low ones with the next change to that
  code; zizmor alerts follow the same thresholds. An alert is dismissed only as a false positive
  or not exploitable, with the reason in the dismissal. Clippy warnings block merging; review
  accepts a suppression only with a reason next to it.

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
