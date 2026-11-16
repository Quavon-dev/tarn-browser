# Release process

Releases and nightly builds are built, signed, notarised and published by CI. The signing keys are
secrets of two GitHub environments that only the release workflow's `sign` job enters (threat model
S4); no Mac is needed.

## What CI produces

Every push and pull request runs `.github/workflows/ci.yml`, which builds and tests every client
(macOS, Windows, Linux, iOS, Android) but publishes nothing. Pushing a tag `v*` runs
`.github/workflows/release.yml`, which builds every client, signs the macOS app and both update
feeds ("Signing in CI") and publishes a GitHub release with:

- `Tarn-<tag>.dmg` and `Tarn-<tag>.zip`: the macOS app, signed with the Developer ID certificate and
  notarised; the disk image is the download, the zip what Sparkle installs. `appcast.xml`, the
  signed Sparkle feed.
- `Tarn-<tag>-unsigned.zip`: the same app as `scripts/bundle.sh` built it, before signing.
- `Tarn-<tag>-windows-x64-unsigned.zip`: `tarn.exe` (CEF's sandbox launcher) and `tarn.dll` (Tarn,
  `crates/app-dll`) with the CEF runtime and the licence texts (`scripts/package-windows.sh`,
  `packaging/third-party-licenses-windows.txt`); `tarn.exe` and `tarn.dll` must carry the same
  certificate, or the launcher refuses the DLL; and
  `Tarn-<tag>-windows-x64-setup-unsigned.exe`, the Inno Setup installer of the same folder
  (`scripts/installer-windows.sh`, Inno Setup pinned by version and SHA-256 in the workflow), which
  users install and the updater downloads. `windows-update.json` and `.sig`, the signed Windows feed.
- `tarn_<version>_<distribution>_amd64.deb` (Debian, Ubuntu), the Fedora `.rpm` and the Arch
  `.pkg.tar.zst`, each built in a clean container by `linux/packaging/build-all.sh`.
- `Tarn-<tag>-ios-unsigned.zip` (`nsh package --target ios`) and
  `Tarn-<tag>-android-unsigned.apk` (`assembleRelease`): unsigned, because store signing keys
  never enter CI; the maintainer signs and submits them, which only Quavon UG
  (haftungsbeschränkt) may do for the app stores (`LICENSE-APP-STORE.md`).
- CycloneDX SBOMs, one per client, each listing the crates built into it for that client's target
  (`cargo cyclonedx`): `Tarn-<tag>.cdx.json` (macOS app, plus the embedded Sparkle framework and
  CEF), `Tarn-<tag>-windows-x64.cdx.json` (`crates/app-dll`, plus CEF), `Tarn-<tag>-linux.cdx.json`
  (the `linux/` workspace; WebKitGTK and the other system libraries come from the distribution or
  runtime), `Tarn-<tag>-ios.cdx.json` and `Tarn-<tag>-android.cdx.json` (`mobile/app`; NSH's own
  platform crates, which `nsh package` adds in its generated workspace, are not listed), and
  `Tarn-<tag>-android-java.cdx.json`, the APK's Java dependencies from Gradle's
  `releaseRuntimeClasspath` plus the pinned `nsh-embed-android` classes, written by the CycloneDX
  Gradle plugin (`-Psbom`) only after the APK is uploaded; the plugin is resolved but not applied in
  the APK build. A macOS-only release carries the macOS SBOM alone.
- `SHA256SUMS` for every file.

Nothing is published until a maintainer approves the run's `release-signing` deployment. Every build
job first runs `scripts/check-version.sh <tag>` (the macOS build skips it for engine updates), so a
release tag needs the same version in the root, `linux/` and `mobile/` manifests,
`mobile/app/nsh.json`, the Android Gradle file, the RPM spec and the PKGBUILD. Engine updates from
`chromium-update.yml` build the macOS bundle alone, so a failing Windows, Linux or mobile build
never holds back a Chromium security fix; the `fuzz` and `deny` gates still apply to them. A manual
run, `gh workflow run release.yml --ref v1.2.3`, does the same for a macOS-only release. It fails
unless started on a `v*` tag whose `release.yml` has the `workflow_dispatch` trigger (older tags
cannot use it), and it publishes nothing while a release for that tag exists.

The build jobs hold no `id-token` permission. The macOS `build` job installs nothing unpinned (Ninja
for the Chromium build comes from `scripts/fetch-ninja.sh`, a release download checked against the
SHA-256 pinned there, as in CI); the Windows, Linux and mobile builds still install distribution
packages, Gradle dependencies and, on Arch, Rust from the distribution without a pin (the containers
are pinned by digest and rustup-init by SHA-256). The `sbom` job only writes the SBOMs. The `deny`
job runs `cargo deny check` (advisories, bans, licences, sources) on every workspace of the released
commit, engine updates included, and `sign` waits for it (`SECURITY.md`, "Dependency and code
scanning"). The `assets` job, which runs no build code, downloads each artifact by the ID its build
job reported and accepts only that job's own file names, so no job can replace another platform's
file; it writes `SHA256SUMS` and attests every binary, the macOS archives included.

The archives are attested (`actions/attest`, predicate type
`https://github.com/Quavon-dev/tarn-browser/attestations/release/v1`, predicate `{commit, tag}`).
A tag push, a manual macOS-only run and a call from `chromium-update.yml` build the same
unsandboxed bundle with `TARN_RELEASE_BUILD=1`, which turns on the one-time move out of a sandboxed
install's container.

## Nightly builds

`.github/workflows/nightly.yml` builds the head of `main` every night at 02:23 UTC with
`release.yml` (`nightly: true`), for every client. It skips the night when the `nightly` tag
already names that commit, and builds nothing unless the CI run of that push to `main` succeeded.
`gh workflow run nightly.yml` builds at once, also a commit the nightly already holds, and fails
while CI on that commit has not succeeded.

Each build replaces the pre-release `nightly` and its tag with the files of a release, named
`Tarn-nightly-…`, signed and notarised in the `nightly-signing` environment without approval, with
`appcast.xml`, `windows-update.json`, `SHA256SUMS` and the attestation, whose predicate names the
tag `nightly`. Its macOS and Windows apps carry the version `<latest release>.<run number>`
(`TARN_BUILD_VERSION`, `0.0.0.<run>` before the first release); the Linux packages keep the
manifests' version. Users get nightlies through
the Nightly channel ("Update channels" below). The latest-release feeds never name a pre-release and
`linux-repo.yml` takes only published releases that are not pre-releases. A release the workflow
token creates starts no other workflow. The `nightly` tag is outside the `v*` ruleset, since the
workflow token moves it; anyone with write access can move it too, so the attestation, not the tag,
names the commit a file was built from. Each build deletes the tag first, so a plain `git fetch
--tags` refuses the moved tag; `scripts/publish-release.sh` fetches only `v*` tags. While a build
replaces it, the pre-release is missing for the minutes its upload takes.

## Signing in CI

`release.yml`'s `build` job builds the macOS bundle unsigned (`TARN_SIGN_IDENTITY=none`). The
`fuzz` job builds the fuzz targets of the exact commit being released with the ClusterFuzzLite
image and runs each one for 60 seconds under AddressSanitizer from its seeds; a crashing input is
uploaded as the `fuzz-crashes` artifact. Every release path (tag, Start release, engine update,
nightly) goes through it, and `sign` does not start unless it passed. The `sign` job, which runs
no build code, enters the `release-signing` environment (`nightly-signing` for nightly builds)
and:

1. imports the Developer ID certificate into a temporary keychain with a random password, stores the
   notary API key as the notarytool profile `tarn-notary` in it and writes the update key to a file
   under `RUNNER_TEMP`;
2. signs the bundle inside out with `scripts/codesign-app.sh` (Sparkle, CEF, the helpers with their
   entitlements, the app; hardened runtime and a secure timestamp), checks the entitlements
   (`scripts/check-entitlements.sh`), notarises and staples it (`scripts/notarize.sh`);
3. writes `Tarn-<tag>.zip`, the disk image `Tarn-<tag>.dmg` (`scripts/dmg.sh`, signed, notarised and
   stapled) and the signed `appcast.xml` (`scripts/appcast.sh`);
4. when the run built Windows, writes `windows-update.json` and `.sig` for that run's unsigned
   installer (`scripts/windows-feed.sh`);
5. deletes the keychain and the key files.

The `assets` job takes the signed files with the rest, writes `SHA256SUMS` and attests them, and
`publish` publishes the release as the latest one and starts `linux-repo.yml`. A tag whose message
has the line `critical` marks both feeds critical.

To release from GitHub, raise the version in every client in a pull request and rename `##
[Unreleased]` in `CHANGELOG.md` to `## [<version>] - <date>` (`scripts/check-version.sh` checks
both; that section becomes the release notes), merge it, wait for its CI on `main`, then run Actions
› Start release (tick "Security release" for a critical one). `start-release.yml` checks that CI
passed on `main`'s latest commit, that every version file carries the same version and that
`v<version>` does not exist, then calls `release.yml` for every client. Approve the run's
`release-signing` deployment once its builds have finished: the `sign` job creates the tag
`v<version>` with the update app (the tag ruleset lets the app create `v*` tags, not GitHub
Actions), signs, and the release is published a few minutes later. If a later job fails after the
tag exists, re-run the failed jobs; to start over, a maintainer deletes the tag first.

The same release can start from a maintainer's machine: `scripts/publish-release.sh` (with `git`
and `gh`) runs the same checks, pushes a signed tag `v<version>` (`--critical` for a security
release) and points to the run, whose `release-signing` deployment is approved the same way.

`scripts/notarize.sh` refuses an ad-hoc signature, a signature that is not "Developer ID
Application", one without a secure timestamp or without the hardened runtime, and anything
`codesign --verify --deep --strict` rejects. It submits a zip with `notarytool submit --wait`,
fails unless the status is `Accepted`, staples the ticket, runs `stapler validate`, and fails unless
`spctl -a -vv -t execute` reports `source=Notarized Developer ID`. `scripts/dmg.sh` refuses an app
`spctl` does not report as notarised, packs it beside an `Applications` link into an HFS+ disk
image named Tarn, signs, notarises and staples the image, fails unless `spctl -t open` reports it
notarised, and mounts it to check it holds exactly those two items with the app's signature intact.
Sparkle updates use the zip.

## Updates (Sparkle)

`scripts/bundle.sh` embeds Sparkle as pinned in `packaging/sparkle.lock` (version, release date,
download host and the SHA-256 of the release archive; `scripts/fetch-sparkle.sh` refuses any other
host or hash and a framework that cannot verify a signed feed). The pinned version is 2.10.1,
whose archive digest matched the one GitHub lists for the release asset. To move to a newer
Sparkle, read its changelog and change the lock in a reviewed pull request.

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
(`browser_core::staleness`): no verified check yet on a new install, none for seven days, or a critical update
found and not installed after two days, closes Chromium tabs. A feed that is not signed with
`sign_update appcast.xml` therefore never counts as verified. The signature carries no date: a
replayed appcast from before the running version no longer counts, but one that lists the running
version still does and can hold back a newer (critical) release until the 28-day limit, so the
feed URL must stay https. The appcast must list each release when it ships: a build the feed does
not list yet gets no verified check: a new install of it never opens Chromium, and an update to it
loses Chromium seven days after the last verified check.
A feed Sparkle cannot fetch (the repository is private) never verifies, so such a build never
opens Chromium. `TARN_LOCAL_CHROMIUM=1` at build time (`browser_core::staleness::LOCAL_BUILD`)
treats a bundle like a debug build: each launch counts as a verified check, so only the 28-day
limit from `packaging/cef.lock`'s `release_date` closes Chromium. It is for a maintainer's own
bundles; a build with `TARN_RELEASE_BUILD` set refuses to compile with it, and the release
workflow never sets it.

The key pair was made on the maintainer's Mac on 2026-11-14 with `generate_keys --account
dev.tarn.browser`. Its private key, exported with `generate_keys --account dev.tarn.browser -x
<file>`, is the secret `SPARKLE_ED_PRIVATE_KEY` of both signing environments; keep a copy in offline
storage, since losing the key means users can no longer update.

`scripts/appcast.sh` checks the copy it signs and refuses an archive holding anything but a
`Tarn.app` folder, an app that `spctl` does not report as notarised, one not signed by team
CFV35FGSHF, another bundle ID and a version other than the tag's (`TARN_BUILD_VERSION` for the
nightly), runs `generate_appcast` with the key file in `TARN_ED_KEY_FILE` (one item, the archive's
`sparkle:edSignature` and the feed signature, since Info.plist requires a signed feed), checks the
feed with `sign_update --verify` and writes `appcast.xml` next to the archive. Users' feed is the
latest published release's `appcast.xml`; a build no published feed lists never opens Chromium on a
new install and loses it seven days after the last verified check on an updated one.

### Update channels

Settings › General › Nightly builds (macOS and Windows, setting `nightly_updates`) switches the
updater to the same feed files in the `nightly` release (`platform::updater::nightly_feed`).
Nightly builds have the version `<latest release>.<nightly run number>`, so each nightly is newer
than the release it follows and older than the next one. The nightly feeds carry their channel
inside the signed bytes (`sparkle:channel` `nightly` from `generate_appcast --channel`, and
`"channel":"nightly"` in `windows-update.json`): on the Release channel Sparkle offers no item of
that channel (`allowedChannelsForUpdater:`) and the Windows updater refuses such a feed, so a
nightly feed put into a release is never offered to Release users. Back on the Release channel a
nightly counts the release feed as current for its release version
(`platform::updater::channel_version`) and stays on until a later release, since no updater
installs a lower version. On Windows, switching channels drops an installer already downloaded
from the other one; on macOS an update Sparkle has already downloaded still installs at quit. A nightly downloaded
by hand starts on the Release channel until the switch is turned on. Linux packages update through
the package manager and iOS and Android through their stores, so they have no channel switch
(`docs/nsh-plan.md`, `docs/mobile-plan.md`).

What the pinned Sparkle does with the feed signature (headers of 2.10.1,
`SPUAppcastSigningValidationStatus.h`): with `SURequireSignedFeed` the feed is checked against
`SUPublicEDKey`. If the check fails, Sparkle still offers the feed's items in a fallback mode (no
critical or informational marking, no release notes or links); the archive itself must still carry
a valid EdDSA signature and a higher version. Sparkle reports the result per appcast
(`SUAppcast.signingValidationStatus`) to an updater delegate, which Tarn installs (above).
`SUSignedFeedFailureExpirationInterval` exists in the binary; its behaviour was not
checked and Tarn does not set it.

## Windows release (maintainer)

CI signs the update feed, not the installer (below). Authenticode signing by hand with
`scripts/sign-windows.ps1` needs the Windows SDK (`signtool.exe`), Inno Setup 6 (`ISCC.exe`,
per-user install is enough) and one of:

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

Every release ships the unsigned installer CI built, `Tarn-<tag>-windows-x64-setup-unsigned.exe`.
The `sign` job signs it and writes the feed (`scripts/windows-feed.sh`): the version, the installer's
URL in this repository's release downloads, its SHA-256, length and Ed25519 signature, signed again
as a whole into `windows-update.json.sig` (`scripts/windows-feed.test.sh` covers the format and
each refusal). The client checks the Ed25519 signatures, length and SHA-256, not Authenticode.
No code-signing certificate is set up, so Windows warns about an unknown publisher on a fresh
install; `scripts/sign-windows.ps1` (Authenticode, run by hand on Windows) is kept for when one is,
but a feed for its signed installer needs the update key, which only the signing environments hold.

Windows builds read the feed from the latest published release
(`https://github.com/Quavon-dev/tarn-browser/releases/latest/download/windows-update.json`), so
every release carries all three files. An engine update (`chromium-update.yml`) or a
macOS-only run builds no Windows files; its `publish` job carries the latest release's
`windows-update.json` and `.sig` over unchanged (they are not in that release's `SHA256SUMS`, which the `assets` job wrote before; their Ed25519 signature covers them), which stay valid, so Windows keeps a verified feed; without them Windows checks
fail: new installs open no page, and existing ones close Chromium seven days after their last
verified check. Windows still needs its own release within 28 days of
each Chromium update, since the staleness gate also closes Chromium by its age.
`scripts/sign-windows.ps1 -Check` checks a release feed by hand; the authority is the client's own verification (`crates/platform/src/updater/windows.rs`). `scripts/sign-windows.test.ps1` (run by the
`windows` CI job) checks the Ed25519 verification against RFC 8032's test vectors and the feed
format.

## Repository settings (maintainer)

Applied by hand in the repository and organisation settings; no workflow changes them. Check them again after any change to the workflows or runners.

- [ ] Ruleset on `main`: require a pull request (one approval, review from Code Owners,
      approval of the most recent reviewable push, dismiss stale approvals when new commits are
      pushed), require the status checks
      `check`, `macos` and `deny` from GitHub Actions with the branch up to date, require code
      scanning results from CodeQL (security alerts high or higher, alerts errors), block force
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
- [ ] Environment `release-signing` also holds the variable `CHROMIUM_UPDATE_APP_CLIENT_ID` and the
      secret `CHROMIUM_UPDATE_APP_KEY` (a private key of the update app of its own), so the `sign`
      job of a release started with `start-release.yml` can create the `v*` tag.
- [ ] Environment `release-signing`: deployment branches and tags limited to `main` and `v*`,
      required reviewers the maintainers (prevent self-review off, so a maintainer can approve their
      own release). Environment `nightly-signing`: deployments from `main` only, no reviewers. Both
      hold the secrets `MACOS_CERTIFICATE_P12` (the Developer ID Application certificate and key,
      base64 of the `.p12`), `MACOS_CERTIFICATE_PASSWORD`, `NOTARY_API_KEY` (the App Store Connect
      API key's `.p8` text), `NOTARY_API_KEY_ID`, `NOTARY_API_ISSUER` and
      `SPARKLE_ED_PRIVATE_KEY`. Create both before the first run: GitHub creates a missing one
      without these rules. No repository-level secret of these names exists.
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

## Secrets and credentials

- **Stored** only as secrets of the GitHub environments named above (`release-signing`,
  `nightly-signing`, `chromium-update`, `filter-signing`) and of `linux-repo` and `aur`
  (`linux-repo.yml`), never as repository secrets, in the repository, in workflow files or in logs.
  Store signing keys (iOS, Android) never enter CI.
- **Accessed** only by the one job that needs each one: `sign` for the Apple and notary credentials
  and the update key (`SPARKLE_ED_PRIVATE_KEY`, which signs both the Sparkle feed and the Windows
  feed); the `merge` job of `chromium-update.yml` and `sign` for the update app key; the signing job
  of `filter-lists.yml` for `FILTER_SIGNING_KEY`; the `build` job of `linux-repo.yml` for
  `LINUX_REPO_GPG_KEY` and `LINUX_REPO_GPG_PASSPHRASE`, its `aur` job for `AUR_SSH_PRIVATE_KEY`.
  `release-signing` needs a maintainer's approval for every run; a maintainer may approve their own
  release. `nightly-signing` holds the same Apple credentials and update key without approval, but
  only `main` may deploy to it. Maintainers with admin rights can set secrets but cannot read them
  back.
- **Rotated** at once when a secret may have leaked, when a maintainer with access to it leaves, or
  when a workflow that reads it was compromised; otherwise the Developer ID certificate when it
  expires, and the notary API key, the update app's private key and the AUR SSH key every year.
  Replacing the update key (both updaters) or the filter signing key also needs an app release
  carrying the new public key before the old key is retired, so installed apps keep verifying
  updates; a new apt/dnf repository key is published in the repository and announced in the release
  notes before the old one stops signing, since installed systems trust the old one. A rotation
  after a suspected leak is tracked in a private security advisory.

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
4. `build` calls `release.yml` with the merge commit: the macOS bundle, signed and published as a
   macOS-only release once a maintainer approves its `release-signing` deployment.

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
  `chromium-update.yml` calls it, so an advisory in an unrelated crate does not stop an engine
  update from merging; `release.yml`'s `deny` job still holds back its release until the advisory
  is fixed or suppressed with a VEX statement (`SECURITY.md`).
- Target for a high-severity issue in Tarn's own code: fix release within 7 days (`SECURITY.md`).
