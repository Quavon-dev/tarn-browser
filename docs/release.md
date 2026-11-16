# Release process

Releases are signed and notarised on the maintainer's machine. CI holds no signing identity, Apple
credentials or update keys (threat model S4).

## What CI produces

Every push and pull request runs `.github/workflows/ci.yml`, which builds and tests every client
(macOS, Windows, Linux, iOS, Android) but publishes nothing. Pushing a tag `v*` runs
`.github/workflows/release.yml`, which creates a **draft** GitHub release with:

- `Tarn-<tag>-unsigned.zip`: the macOS app built by `scripts/bundle.sh` with CEF and its helpers,
  left unsigned (`TARN_SIGN_IDENTITY=none`) for the signing Mac. It is not for users.
- `Tarn-<tag>-windows-x64-unsigned.zip`: `tarn.exe` (CEF's sandbox launcher) and `tarn.dll` (Tarn,
  `crates/app-dll`) with the CEF runtime and the licence texts (`scripts/package-windows.sh`,
  `packaging/third-party-licenses-windows.txt`); `tarn.exe` and `tarn.dll` must carry the same
  certificate, or the launcher refuses the DLL; and
  `Tarn-<tag>-windows-x64-setup-unsigned.exe`, the Inno Setup installer of the same folder
  (`scripts/installer-windows.sh`, Inno Setup pinned by version and SHA-256 in the workflow).
  Neither is for users: the maintainer signs them on Windows (below).
- `tarn_<version>_<distribution>_amd64.deb` (Debian, Ubuntu), the Fedora `.rpm` and the Arch
  `.pkg.tar.zst`, each built in a clean container by `linux/packaging/build-all.sh`.
- `Tarn-<tag>-ios-unsigned.zip` (`nsh package --target ios`) and
  `Tarn-<tag>-android-unsigned.apk` (`assembleRelease`): unsigned, because store signing keys
  never enter CI; the maintainer signs and submits them, which only Quavon UG
  (haftungsbeschränkt) may do for the app stores (`LICENSE-APP-STORE.md`).
- `Tarn-<tag>.cdx.json`: CycloneDX SBOM of the `app` crate and its dependencies.
- `SHA256SUMS` for every file.

Nothing is published until the maintainer publishes the draft. Every build job first runs
`scripts/check-version.sh <tag>` (the macOS build skips it for engine updates), so a release tag
needs the same version in the root, `linux/` and `mobile/` manifests, `mobile/app/nsh.json`, the
Android Gradle file, the RPM spec and the PKGBUILD. Engine updates from `chromium-update.yml` build
the macOS bundle alone, so a failing Windows, Linux or mobile build never holds back a Chromium
security fix. A manual run, `gh workflow run release.yml --ref v1.2.3`, does the same for a
macOS-only release. It fails unless started on a `v*` tag whose `release.yml` has the
`workflow_dispatch` trigger (older tags cannot use it), and it drafts nothing while a release or
draft for that tag exists: delete the tag push's draft first, or let the push run fail. The SBOM
covers the root workspace as built on macOS; Windows-only dependencies and the `linux/` and
`mobile/` workspaces have none yet.

The build jobs hold no `id-token` permission. The macOS `build` job installs nothing unpinned
(Ninja for the Chromium build comes from `scripts/fetch-ninja.sh`, a release download checked
against the SHA-256 pinned there, as in CI); the Windows, Linux and mobile builds still install
distribution packages, Gradle dependencies and, on Arch, Rust from the distribution without a pin
(the containers are pinned by digest and rustup-init by SHA-256). The `sbom` job only writes the
SBOM. The `assets` job, which runs no build code, downloads each artifact by the ID its build job
reported and accepts only that job's own file names, so no job can replace another platform's
file; it writes `SHA256SUMS` and attests every binary, the macOS archive included.

The unsigned archive is attested (`actions/attest`, predicate type
`https://github.com/Quavon-dev/tarn-browser/attestations/release/v1`, predicate `{commit, tag}`).
A tag push, a manual macOS-only run and a call from `chromium-update.yml` build the same
unsandboxed bundle with `TARN_RELEASE_BUILD=1`, which turns on the one-time move out of a sandboxed
install's container.

## Signed build (maintainer)

Requires a "Developer ID Application" certificate in the login keychain and Xcode 16 or later.

```sh
git checkout v1.2.3
TARN_RELEASE_BUILD=1 TARN_SIGN_IDENTITY="Developer ID Application: Quavon UG (haftungsbeschrankt) (CFV35FGSHF)" scripts/bundle.sh
scripts/check-entitlements.sh
```

With an identity set, `bundle.sh` adds the secure timestamp notarisation requires.

## Notarisation

Store the credentials once in the login keychain under a profile name (an app-specific password
from appleid.apple.com, or `--key`/`--key-id`/`--issuer` for an App Store Connect API key). They
never go into the repository, a script argument or an environment variable; only the profile name
does:

```sh
xcrun notarytool store-credentials tarn-notary --apple-id "<apple-id>" --team-id "<TEAMID>"
```

`notarytool` asks for the password itself. Then for each release:

```sh
TARN_NOTARY_PROFILE=tarn-notary scripts/notarize.sh      # default: target/release/Tarn.app
ditto -c -k --keepParent target/release/Tarn.app "Tarn-v1.2.3.zip"
TARN_SIGN_IDENTITY="Developer ID Application: Quavon UG (haftungsbeschrankt) (CFV35FGSHF)" \
  TARN_NOTARY_PROFILE=tarn-notary scripts/dmg.sh target/release/Tarn.app Tarn-v1.2.3.dmg
```

The `tarn-notary` profile (team CFV35FGSHF) is stored on the maintainer's Mac since 2026-11-14.

`scripts/notarize.sh` refuses an ad-hoc signature, a signature that is not "Developer ID
Application", one without a secure timestamp or without the hardened runtime, and anything
`codesign --verify --deep --strict` rejects. It submits a zip with `notarytool submit --wait`,
fails unless the status is `Accepted` (printing the `notarytool log` command otherwise), staples
the ticket, runs `stapler validate`, and fails unless `spctl -a -vv -t execute` reports
`source=Notarized Developer ID`.

`scripts/dmg.sh` is the download for people installing Tarn: it refuses an app `spctl` does not
report as notarised, packs it beside an `Applications` link into an HFS+ disk image named Tarn
(opening it shows the window to drag Tarn into Applications), signs, notarises and staples the
image, fails unless `spctl -t open` reports it notarised, and mounts it to check it holds exactly
those two items with the app's signature intact. Sparkle updates keep using the zip.

Upload `Tarn-v1.2.3.zip` and `Tarn-v1.2.3.dmg` to the draft release. Anyone with write access can
replace a draft's files, so before publishing download both from the draft, mount the image with
`hdiutil attach -readonly` and unpack the zip, then check the image and both apps:

```sh
req='=anchor apple generic and certificate leaf[subject.OU] = "CFV35FGSHF"'
codesign --verify --deep --strict -R "$req" Tarn-v1.2.3.dmg <mounted>/Tarn.app <unpacked>/Tarn.app
# both must report source=Notarized Developer ID
spctl -a -vv -t open --context context:primary-signature Tarn-v1.2.3.dmg
spctl -a -vv -t execute <mounted>/Tarn.app
```

and that each app's `CFBundleShortVersionString` is the tag's version, so an older genuine file
cannot stand in. Then publish it.

## Updates (Sparkle)

`scripts/bundle.sh` embeds Sparkle as pinned in `packaging/sparkle.lock` (version, release date,
download host and the SHA-256 of the release archive; `scripts/fetch-sparkle.sh` refuses any other
host or hash and a framework that cannot verify a signed feed). The pinned version is 2.10.1,
whose archive digest matched the one GitHub lists for the release asset. To move to a newer
Sparkle, read its changelog and change the lock in a reviewed pull request; the signing Mac
compares the bundle's Sparkle with its own download of that lock, and signs a release that changes
the lock only from a maintainer-signed tag.

The bundle carries Sparkle's installer service only (`Installer.xpc`); the downloader service is
removed because the unsandboxed app downloads updates itself. Its Info.plist sets
`SUEnableInstallerLauncherService`, `SUEnableAutomaticChecks` and `SUAutomaticallyUpdate` (daily
checks and installs without asking), `SUVerifyUpdateBeforeExtraction` and
`SURequireSignedFeed`. The app runs without the App Sandbox, so it holds none of the mach-lookup
exceptions Sparkle's sandboxing guide names.

`packaging/updates.conf` names the feed
(`https://github.com/Quavon-dev/tarn-browser/releases/latest/download/appcast.xml`, the
`appcast.xml` of the latest published release) and the public key since 2026-11-14. Update checks
stay off in a build whose `updates.conf` lacks either, and in debug bundles, which never get the
feed and key so Sparkle cannot replace them with a release; `bundle.sh`
then writes them into Info.plist as `SUFeedURL` and `SUPublicEDKey` and refuses a feed that is not
an https URL or a key that is not 44 base64 characters. The app loads Sparkle only when both are
present and valid (`platform::updater::configured`), so an unconfigured or unbundled build never
starts it, and its delegate answers `feedURLStringForUpdater:` with the Info.plist feed, which
Sparkle asks before any feed URL stored in user defaults; Settings › General then shows "Not set up in this build" and Tarn › Check for Updates…
is disabled.

Tarn installs an updater delegate (`platform::updater`). It records the time of each check whose
appcast signature Sparkle verified (`signingValidationStatus` "succeeded") and whose appcast lists
an item with a `sparkle:version` at least the running `CFBundleVersion`
(`platform::updater::lists_current`: dot-separated numbers, anything else only matches itself;
kept in the setting
`update_feed_verified`, so it survives a relaunch without network; a stored time after the
launch time is dropped) and when a check found a critical update (setting
`update_critical_found`, cleared by running another version), which then waits for a relaunch. Both feed the Chromium staleness gate
(`browser_core::staleness`): no verified check for seven days, or a critical update
found and not installed after two days, closes Chromium tabs. A feed that is not signed with
`sign_update appcast.xml` therefore never counts as verified. The signature carries no date: a
replayed appcast from before the running version no longer counts, but one that lists the running
version still does and can hold back a newer (critical) release until the 28-day limit, so the
feed URL must stay https. The appcast must list each release when it ships: a build the feed does
not list yet gets no verified check and loses Chromium after seven days.
A feed Sparkle cannot fetch (the repository is private) never verifies, so such a build never
opens Chromium. `TARN_LOCAL_CHROMIUM=1` at build time (`browser_core::staleness::LOCAL_BUILD`)
treats a bundle like a debug build: each launch counts as a verified check, so only the 28-day
limit from `packaging/cef.lock`'s `release_date` closes Chromium. It is for a maintainer's own
bundles; a build with `TARN_RELEASE_BUILD` set refuses to compile with it, and the release
workflow never sets it.

The key pair was made on the maintainer's Mac on 2026-11-14 (never on CI):
`generate_keys --account dev.tarn.browser` keeps the private key in the login keychain under the
account `dev.tarn.browser`; a copy (`generate_keys --account dev.tarn.browser -x`) is in
`~/Library/Application Support/Tarn Release/sparkle-ed25519-private-key` (mode `0600`, folder
`0700`). Move that copy to offline storage; losing the key means users can no longer update. On
another Mac, import it with `generate_keys --account dev.tarn.browser -f <file>`.

For each release, after notarisation:

```sh
ditto -c -k --keepParent target/release/Tarn.app "Tarn-v1.2.3.zip"
scripts/appcast.sh Tarn-v1.2.3.zip v1.2.3              # add --critical for a security release
gh release upload v1.2.3 Tarn-v1.2.3.zip appcast.xml
```

`scripts/appcast.sh` checks the copy it signs and refuses an archive holding anything but a
`Tarn.app` folder, an app that `spctl` does not report as notarised, one not signed by
team CFV35FGSHF, another bundle ID and a version other than the tag's, runs
`generate_appcast` with the keychain key (one item, the archive's `sparkle:edSignature` and the
feed signature, since Info.plist requires a signed feed), checks the feed with
`sign_update --verify` and writes `appcast.xml` next to the archive. Publish the release only
with both files: users' feed is the latest published release's `appcast.xml`, and a build no
published feed lists loses Chromium after seven days. Any later edit of the feed needs signing
again.

What the pinned Sparkle does with the feed signature (headers of 2.10.1,
`SPUAppcastSigningValidationStatus.h`): with `SURequireSignedFeed` the feed is checked against
`SUPublicEDKey`. If the check fails, Sparkle still offers the feed's items in a fallback mode (no
critical or informational marking, no release notes or links); the archive itself must still carry
a valid EdDSA signature and a higher version. Sparkle reports the result per appcast
(`SUAppcast.signingValidationStatus`) to an updater delegate, which Tarn installs (above).
`SUSignedFeedFailureExpirationInterval` exists in the binary; its behaviour was not
checked and Tarn does not set it.

## Windows release (maintainer)

Signing happens on the maintainer's Windows machine; CI holds no certificate (S4). Requires the
Windows SDK (`signtool.exe`), Inno Setup 6 (`ISCC.exe`, per-user install is enough) and one of:

- a code signing certificate in the user's certificate store (`Cert:\CurrentUser\My`, a hardware
  token's certificate included), chosen by its SHA-1 thumbprint;
- Azure Trusted Signing: the `Azure.CodeSigning.Dlib.dll` of Microsoft's Trusted Signing client
  tools, a `metadata.json` naming the endpoint, account and certificate profile, and an Azure
  login the client can use (`az login`). No secret goes into the repository or a script argument.

The installer (`packaging/windows/tarn.iss`) installs per user (no administrator) into
`%LOCALAPPDATA%\Programs\Tarn` with a Start menu entry and an uninstaller. It packages whatever
folder `scripts/package-windows.sh` made, first deleting the old version's DLLs, `.pak`, `.bin`,
`.json` files and `locales`. Before it replaces files it waits up to 30 seconds for any `tarn.exe`
running from that folder to quit, then lets Restart Manager close what is left. With `/TARNUPDATE`
(the updater runs `<setup>.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /TARNUPDATE` after Tarn
quit) it starts Tarn again when done. The uninstaller refuses while Tarn runs and keeps the data
folder `%LOCALAPPDATA%\Tarn` unless the user answers Yes to deleting it (a silent uninstall keeps
it).

For each release, after CI drafted it:

1. Download the unsigned zip from the draft and check that the release workflow built it, since
   anyone with write access can replace a draft's files:
   `gh release download v1.2.3 -p "Tarn-v1.2.3-windows-x64-unsigned.zip"`, then
   `gh attestation verify Tarn-v1.2.3-windows-x64-unsigned.zip --repo Quavon-dev/tarn-browser --signer-workflow Quavon-dev/tarn-browser/.github/workflows/release.yml`.
   Sign nothing that fails it. Build the installer with Inno Setup 6.7.3, the version CI pins.
2. Sign the binaries, build the installer from them and sign it (PowerShell, in a checkout of the
   tag):

   ```powershell
   scripts\sign-windows.ps1 -Package Tarn-v1.2.3-windows-x64-unsigned.zip -Version 1.2.3 -Thumbprint <SHA-1>
   # or
   scripts\sign-windows.ps1 -Package Tarn-v1.2.3-windows-x64-unsigned.zip -Version 1.2.3 -Dlib <path>\Azure.CodeSigning.Dlib.dll -Metadata <path>\metadata.json
   ```

   It signs every `.exe` and `.dll` without a valid signature (`tarn.exe`, and CEF's files only
   if they are unsigned) with SHA-256 and an RFC 3161 timestamp (`-TimestampUrl`, default DigiCert
   for a certificate, Microsoft's for Trusted Signing), compiles the installer with `ISCC`, which
   signs the installer and its uninstaller with the same command, and checks every binary with
   `signtool verify /pa` and for a timestamp. The result is `dist\Tarn-v1.2.3-windows-x64-setup.exe`.
   Add `-Critical` for a security release.
3. The script prints three commands for the Mac that holds the update key (the Sparkle key above):
   they sign the installer with `sign_update --account dev.tarn.browser`, write
   `windows-update.json` with that signature and sign the feed into `windows-update.json.sig`.
   Copy the installer there, run them, and bring both files back. Any other Ed25519 tool that
   holds the same private key will do: `windows-update.json.sig` is the base64 of the 64-byte
   signature over the exact bytes of `windows-update.json`, with no newline. If the installer's
   signature was made elsewhere, `scripts\sign-windows.ps1 -Installer <setup.exe>
   -InstallerSignature <base64>` writes the feed instead.
4. Check the pair against the public key in `packaging/updates.conf`:
   `scripts\sign-windows.ps1 -Check windows-update.json -Installer Tarn-v1.2.3-windows-x64-setup.exe`.
   It verifies both Ed25519 signatures, the installer's length and SHA-256, and the feed's exact
   format (`{"version":…,"critical":…,"installer":{"url":…,"sha256":…,"length":…,"signature":…}}`,
   URL `https://github.com/Quavon-dev/tarn-browser/releases/download/v<version>/Tarn-v<version>-windows-x64-setup.exe`).
5. Upload: `gh release upload v1.2.3 Tarn-v1.2.3-windows-x64-setup.exe windows-update.json windows-update.json.sig`.

Windows builds read the feed from the latest published release
(`https://github.com/Quavon-dev/tarn-browser/releases/latest/download/windows-update.json`), so
publish the release only with all three files. An engine update (`chromium-update.yml`) or a
macOS-only run builds no Windows files; its `publish` job carries the latest release's
`windows-update.json` and `.sig` over unchanged (they are not in that release's `SHA256SUMS`, which the `assets` job wrote before; their Ed25519 signature covers them), which stay valid, so Windows keeps a verified feed; without them Windows checks
fail and Chromium closes after seven days. Publish a normal release before any
engine update or macOS-only run that follows it, or the later release carries the older feed. Windows still needs its own release within 28 days of
each Chromium update, since the staleness gate also closes Chromium by its age.
`scripts/sign-windows.ps1 -Check` is a maintainer self-check; the authority is the client's own verification (`crates/platform/src/updater/windows.rs`). `scripts/sign-windows.test.ps1` (run by the
`windows` CI job) checks the Ed25519 verification against RFC 8032's test vectors and the feed
format.

## Repository settings (maintainer)

Applied by hand in the repository and organisation settings; no workflow changes them. Check them again after any change to the workflows or runners.

- [ ] Ruleset on `main`: require a pull request (one approval, review from Code Owners,
      approval of the most recent reviewable push, dismiss stale approvals when new commits are
      pushed), require the status checks
      `check`, `macos` and `deny` from GitHub Actions with the branch up to date, block force
      pushes, restrict deletions. Bypass: only the update app below, for pull requests only (its
      tests run as a reusable workflow, which posts no checks on the pull request). Never
      GitHub Actions itself: every workflow token on every branch acts as GitHub Actions.
- [ ] `.github/CODEOWNERS`: its team `@Quavon-dev/maintainers` must exist and have write
      access; it covers itself,
      `.github/workflows/`, `scripts/`, `packaging/`, `deny.toml`, `.cargo/`, `Cargo.toml`,
      `Cargo.lock`, `renovate.json`, build scripts, the updater, XPC, Chromium runtime and helper
      sources, the staleness gate, the filter-bundle signer, the files that instruct or configure
      AI agents (`AGENTS.md`, `CLAUDE.md`, `.claude/`, `.agents/`, `.mcp.json`), all of `.github/`,
      `docs/`, the contribution, security and privacy policies and the code that
      enforces a mitigation (`crates/ai`, `crates/import`, `crates/sync`, `crates/cli-bridge`, the engine's policy files); the file
      itself is the full list. Until then GitHub ignores
      its lines and no Code Owners review is required.
- [ ] Ruleset on tags `v*`: restrict creation, update and deletion; bypass for the maintainers and
      the update app (it creates the tag of its merge).
- [ ] Update app: a GitHub App owned by the organisation, installed on this repository only,
      with contents and pull requests read and write and nothing else. Environment
      `chromium-update` with deployment branches limited to `main`, holding the variable
      `CHROMIUM_UPDATE_APP_CLIENT_ID` and the secret `CHROMIUM_UPDATE_APP_KEY` (the app's private
      key); only the `merge` job of `chromium-update.yml` uses it. Repository variable
      `CHROMIUM_UPDATE_APP_SLUG` (the app's slug), so a tag the app pushes does not start the
      tag-push build of `release.yml` as well.
- [ ] Actions › General: "Allow GitHub Actions to create and approve pull requests" stays off;
      with it, a workflow on any branch could approve its author's pull request. The update app
      opens the update pull requests.
- [ ] Environment `filter-signing` with deployment branches limited to `main`, holding
      `FILTER_SIGNING_KEY`; the repository-level secret of that name is deleted, since a
      workflow on any branch can read repository secrets.
- [ ] Organisation: two-factor authentication required for every member.
- [ ] Actions › General: default workflow permissions read-only; fork pull request workflows
      from outside collaborators need approval for all outside collaborators.
- [ ] No workflow uses self-hosted runners; every job runs on GitHub-hosted images.
- [ ] No job that pull requests, issues or comments can start enters an environment;
      `scripts/check-workflows.sh` refuses it, because those events run with `main` as their ref
      and pass an environment's branch rule.
- [ ] `scripts/check-workflows.sh` passes; it runs in the `check` job and fails when a workflow
      that outside accounts can start (pull requests, issues, comments) runs on anything but
      `ubuntu-24.04`, `macos-15` or `windows-2025`, so a self-hosted label such as `ubuntu-fast` is refused too.

## Automatic engine updates

`.github/workflows/chromium-update.yml` runs every three hours from `main`:

1. `watch` reads `https://cef-builds.spotifycdn.com/index.json` and takes the newest stable CEF
   build present for both architectures that `cef-dll-sys` builds against: the crate's own CEF
   version or an older build of its major.minor. Its Chromium version must be newer than the
   pinned one and on Google's Mac stable or extended stable list (CEF follows either); the
   release date is Google's first serving date. It computes each archive's SHA-256.
2. `propose` writes `packaging/cef.lock`, bumps the patch version in `Cargo.toml` and
   `Cargo.lock` and pushes `chromium-update/<CEF version>`.
3. `test` runs `ci.yml` on that commit; `merge`, with the update app's token, opens the pull
   request and merges it naming the tested commit (a later push to the branch makes the merge fail) and tags
   the merge commit `v<version>`. It merges only while `main` is still the commit the branch was
   cut from, so the merge carries the tested tree; if `main` moved, it closes the pull request and
   deletes the branch, and the next scheduled run proposes the update again on the new `main`.
4. `build` calls `release.yml` with the merge commit: unsigned Chromium bundle, attestation, draft
   release.

Any failure, and a Chrome stable release left without a usable CEF build for seven days, opens an
issue labelled `security`. To retry a failed `merge` or `build`, re-run the failed jobs of that
run: `merge` picks up an already merged pull request and an existing tag. To retry a failed
`propose` or `test`, delete the branch `chromium-update/<CEF version>`. To reject an update,
cancel the run before `merge` and keep the branch: `watch` then proposes nothing for that CEF
build. `merge` opens the pull request itself; a re-run refuses a commit whose pull request was
closed without merging. If `merge` fails because
`main` moved in the moment between its check and the merge (GitHub's merge call cannot name the
expected base), the update is on `main` untested and untagged and a re-run fails the same way: let
CI run on `main`, then release it with a maintainer-signed tag. A `main` that keeps moving while
the tests run only delays the update; the seven-day issue above still fires.

The pipeline never moves the `cef` crate. A newer CEF build than the crate's own (a patch build
included, since `cef-dll-sys` refuses those) and a new Chromium milestone come through Renovate,
are reviewed by hand, carry their `packaging/cef.lock` and are released with a maintainer-signed
tag; after that the pipeline can only follow with lock updates up to the crate's CEF version.

### Signing Mac (maintainer)

`scripts/sign-release.sh` signs only what §6 allows. For each draft release it refuses, before
unpacking anything: an attestation that is not from `release.yml` on `main` called by
`chromium-update.yml` on `main` on a GitHub-hosted runner (or, for a maintainer-signed tag, from
`release.yml` on that tag); a tag whose commit is not on `main`; a predicate commit that is not
the tag's; a diff since the last signed release that changes anything but `packaging/cef.lock`,
the workspace version in `Cargo.toml` and the workspace crates' versions in `Cargo.lock`, unless
the tag is signed by a key in `allowed_signers`; an app version that is not higher or a Chromium
version that is lower than the last signed release; a lock with other keys, another download
host, a release date in the future or one Google did not publish; a CEF build this Mac first saw
less than 24 hours ago; its own CEF download with another SHA-256; a second download of the same
archive over the configured `second_route` that differs from the first. Without `second_route` it
refuses before downloading anything, unless it is run for one tag with
`--acknowledge-single-source` and the CEF version is typed on stdin; it then warns and appends the
tag, CEF version and account to `<state>/single-source.log`. The attestation's
`buildSignerDigest` and `buildConfigDigest` must name commits whose `.github/workflows` tree is
the tag's, so a workflow changed on `main` after the tag cannot vouch for it. It also downloads
Sparkle as the tag's `packaging/sparkle.lock` pins it (host `github.com`, SHA-256) and removes the
downloader service as `bundle.sh` does. After unpacking as a separate user it refuses a bundle
with a special or set-id file, a symlink outside `Sparkle.framework`, any file besides the app
binary, icon, `Info.plist`, `Sparkle-LICENSE.txt`, the three notices (`Chromium-CREDITS.html` and
`CEF-LICENSE.txt` equal to its own CEF download, `Third-Party-Licenses.txt` equal to the tag's
`packaging/third-party-licenses.txt`), five helpers and the two frameworks, a CEF
framework or `Sparkle.framework` that differs from its own download in any file, mode, symlink
or entry, a Sparkle licence that differs, an `Info.plist` that is not the tag's
`packaging/Info.plist` with the app version plus `SUFeedURL` and `SUPublicEDKey` from its
configuration (so no other `SU*` key and no changed pinned value), or a helper with `SU*` keys.
Then it signs Sparkle inside out (`Installer.xpc`, `Autoupdate`, `Updater.app`, the framework,
the order of Sparkle's Developer ID instructions), the GPU and renderer helpers with its own
`Helper-JIT.entitlements`, the plain helper and the app with its own `Tarn.entitlements` (camera
and microphone), all with hardened runtime and a timestamp, notarises,
staples and uploads `Tarn-<tag>.zip` to the draft.
`scripts/sign-release.test.sh` (run by the `macos` CI job) checks every refusal on fixtures.

Not built: the signing Mac does not yet run `scripts/appcast.sh` or publish; until then the
maintainer does both by hand for each signed draft (above). The second route is only as independent as its setup: a proxy on
another network (another ISP, a VPS, a VPN exit) defeats an attacker on the signing Mac's network
or one serving that network another archive, not the CDN serving the same wrong archive to
everyone; a mirror on another host helps against that only if it fetched the archive on its own.
No independent CEF build exists to compare with.

Setup, once:

1. A dedicated macOS account (here `tarn-sign`) and an unprivileged account without a keychain
   (here `tarn-unpack`). Allow `tarn-sign` to run commands as `tarn-unpack` without a password:
   `tarn-sign ALL=(tarn-unpack) NOPASSWD: ALL` in `/etc/sudoers.d/tarn-sign`.
2. In `tarn-sign`'s login keychain: the Developer ID Application identity; notary credentials
   (`xcrun notarytool store-credentials tarn-notary ...`); a fine-grained token for this
   repository only, with contents read and write and attestations read, stored with
   `security add-generic-password -s tarn-release-token -a tarn-sign -w`.
3. Install `scripts/sign-release.sh` from a reviewed commit, by hand, to
   `/usr/local/libexec/tarn-sign/`, together with `packaging/Helper-JIT.entitlements`, `packaging/Tarn.entitlements` and an
   `allowed_signers` file listing the maintainers' SSH signing keys
   (`<email> ssh-ed25519 AAAA...`). Never update it from the pipeline.
4. `git clone --bare https://github.com/Quavon-dev/tarn-browser.git ~/tarn.git`; the script
   fetches `main` and the tags into it on every run.
5. A configuration file, owned by `tarn-sign` and writable only by it, read as data:

   ```
   repo=Quavon-dev/tarn-browser
   mirror=/Users/tarn-sign/tarn.git
   state=/Users/tarn-sign/state
   allowed_signers=/usr/local/libexec/tarn-sign/allowed_signers
   jit_entitlements=/usr/local/libexec/tarn-sign/Helper-JIT.entitlements
   media_entitlements=/usr/local/libexec/tarn-sign/Tarn.entitlements
   identity=Developer ID Application: <Name> (<TEAMID>)
   notary_profile=tarn-notary
   unpack_user=tarn-unpack
   token_service=tarn-release-token
   feed_url=<feed URL from packaging/updates.conf>
   public_ed_key=<public key from packaging/updates.conf>
   second_route=proxy https://<proxy on another network>:<port>
   ```

   `second_route` is `proxy https://HOST[:PORT]` (an HTTPS proxy the second download tunnels
   through to `cef-builds.spotifycdn.com`) or `mirror https://HOST[/PATH]` (another host serving
   the same file names; not the CDN itself, by name or by redirect). No credentials in the URL; a
   proxy that needs them is set up so that it does not (for example by source address). The
   direct download ignores proxy settings in the environment, and every download ignores
   `~/.curlrc`.

6. A LaunchAgent for `tarn-sign` that runs
   `/usr/local/libexec/tarn-sign/sign-release.sh --config <file>` hourly while the account is
   logged in (the keychain must be unlocked).
7. The first release this Mac signs needs a maintainer-signed tag (`git tag -s`), because there is
   no previous signed release to compare with.

For an exploited Chromium bug, skip the soak by hand: `sign-release.sh --config <file> --skip-soak
v<version>`.

## Licences in the bundle

`bundle.sh` copies Chromium's credits (`CREDITS.html` of the CEF archive), CEF's `LICENSE.txt`,
Sparkle's licence and `packaging/third-party-licenses.txt` into `Contents/Resources`; Settings ›
General › About shows them in Finder. `scripts/third-party-licenses.sh` writes that file from
`cargo metadata` (every crate compiled into the app and the Chromium helper on
`aarch64-apple-darwin`, with its licence expression and the licence files it ships); the `check`
CI job runs it with `--check`, so a dependency change must regenerate it.

## Patch process

- Renovate (`renovate.json`) opens grouped pull requests for Cargo and GitHub Actions updates
  weekly; actions stay pinned by commit SHA. A Cargo update fails the `check` job until
  `scripts/third-party-licenses.sh` is run on its branch and the regenerated
  `packaging/third-party-licenses.txt` and `packaging/third-party-licenses-windows.txt` are pushed.
- `.github/workflows/audit.yml` runs `cargo deny check advisories` nightly, and the `deny` job of
  `ci.yml` checks advisories with bans, licences and sources on every pull request; not when
  `chromium-update.yml` calls it, so an advisory in an unrelated crate does not hold back an engine
  update.
- Target for a high-severity issue in Tarn's own code: fix release within 7 days (`SECURITY.md`).
