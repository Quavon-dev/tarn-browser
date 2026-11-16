# Contributing

Everyone taking part follows the [Code of Conduct](CODE_OF_CONDUCT.md). Read
[`AGENTS.md`](AGENTS.md) for how the five clients and the shared crates fit together.

## Prerequisites

- macOS 14 or later with Xcode (or its command line tools).
- Rust through [rustup](https://rustup.rs); the version is pinned in `rust-toolchain.toml` and
  installed on first use, with clippy and rustfmt.
- [`cargo-deny`](https://github.com/EmbarkStudios/cargo-deny) for the licence and advisory check.
- For Linux, iOS and Android: Docker, [Native Shell](https://nsh.dev/) 0.5.0, the Android SDK, NDK
  and JDK 17+, as described in the README. The Linux app links the distribution's WebKitGTK 6.0
  and GTK 4; the Fedora 43 container in `linux/docker/` has them. Gradle 8.14.3 is fetched by
  `mobile/android/gradle.sh`.
- For Windows: Windows 10 or 11 on x86_64, Git for Windows (Git Bash), the Visual Studio Build
  Tools with the MSVC toolchain and the Windows SDK; Inno Setup 6.7.3 for the installer
  ([`docs/windows-plan.md`](docs/windows-plan.md)).
- For the Chromium engine (Windows, and macOS with the `chromium` feature): the CEF build from
  `scripts/fetch-cef.sh` and Ninja from `scripts/fetch-ninja.sh`, both pinned by SHA-256.

## Build and test

```sh
cargo run -p app            # build and start the macOS app
cargo test --locked         # root workspace tests
```

`linux/`, `mobile/` and `nsh-shared/` are separate Cargo workspaces; run their checks in their own
directory.

## Checks

Each must pass before a pull request is merged; CI (`.github/workflows/ci.yml`) runs them.

| Where | Command |
|---|---|
| root (`crates/*`) | `cargo clippy --all-targets --locked -- -D warnings && cargo test --locked` |
| `nsh-shared/` | same |
| `mobile/` | same, plus `cargo clippy --target aarch64-apple-ios-sim --all-targets --locked -- -D warnings` |
| `linux/` | same, plus `linux/scripts/display-tests.sh` |
| Android build | `mobile/android/build.sh` |
| Windows (Git Bash, MSVC) | `export CEF_PATH="$(cygpath -w "$(scripts/fetch-cef.sh windows64)")"; cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked` |
| every workspace | `cargo fmt --all --check` |
| every workspace (root, `linux/`, `mobile/`, `nsh-shared/`, `fuzz/`) | `cargo deny check` |

## Dependencies

- **Selected** only when the standard library, the platform or a crate already in the lockfile
  cannot do the job. A new dependency must be maintained, have a licence on the allow list in
  `deny.toml`, come from crates.io (Rust), Google's Maven or Maven Central (Android), and be added
  by a maintainer or with a maintainer's approval in the pull request (`AGENTS.md`, "Do not" 12).
- **Obtained** through each ecosystem's own tooling: Cargo with committed lockfiles and versions
  pinned exactly, Gradle for the Android app, the distribution's package manager for system
  libraries in the Linux containers. Tools and binaries without such tooling (CEF, Ninja, Gradle,
  Sparkle, Inno Setup, NSH) are downloaded by a script or workflow over HTTPS and checked against
  a SHA-256 pinned in the repository; GitHub Actions and container images are pinned by digest.
- **Tracked** by Renovate, which opens grouped update pull requests every Monday, by `cargo deny`
  on every pull request and nightly (`SECURITY.md`, "Dependency and code scanning"), by the
  automatic Chromium update (`docs/release.md`), and in the SBOM attached to every release. A
  change to a lockfile regenerates `packaging/third-party-licenses*.txt`.

## Changelog

Every pull request that changes what users see or how Tarn protects them adds an entry under
`## [Unreleased]` in [`CHANGELOG.md`](CHANGELOG.md): `Added`, `Changed`, `Fixed`, `Removed`, and
`Security` for every security fix, named by what it protects against. The release workflow
publishes each version's section as its release notes; an automatic Chromium update, which has no
section, gets a generated Security note followed by the entries under `[Unreleased]`.

## Commits

Use [Conventional Commits](https://www.conventionalcommits.org/): `feat: …`, `fix: …`,
`perf: …`, `docs: …`, `refactor: …`, `test: …`, `chore: …`. One commit covers one concern; a
feature may cover all clients in one commit.

## Contribution terms

Tarn is licensed under `GPL-3.0-or-later` ([`LICENSE`](LICENSE)). By submitting a contribution you
license it under `GPL-3.0-or-later` and grant Quavon UG (haftungsbeschränkt) the additional
permission in [`LICENSE-APP-STORE.md`](LICENSE-APP-STORE.md) for it, so every merged change can ship
in the official iOS and Android apps. Do not add code from other projects under the GPL or another
licence that rules out app store terms, unless its authors have granted the same permission. The Tarn name, logo and app icon are covered by [`TRADEMARKS.md`](TRADEMARKS.md).
Sign off every commit to certify the
[Developer Certificate of Origin](https://developercertificate.org/):

```sh
git commit -s
```

This adds a `Signed-off-by: Your Name <you@example.com>` line with the name and email of your Git
identity. The `dco` check refuses a pull request while any of its commits (other than merges) lacks
a sign-off with its author's email; `git rebase --signoff main` signs off a whole branch. Commits
made in GitHub's web editor are signed off by GitHub, which the repository requires.

## AI-assisted contributions

AI agents and AI-assisted tools are allowed. They follow the "Rules for AI agents" in
[`AGENTS.md`](AGENTS.md); you are the author of what they produce and answer for it. In your first
session the agent asks you to accept the AI agreement (`scripts/ai-agreement.sh show`): you review
everything it writes before it is committed or submitted, make sure it is correct, tested, secure
and of known origin, and certify the DCO for it. With that agreement the agent adds your sign-off to
the commits it makes for you; without it, it commits nothing. Your acceptance stays in your clone's
Git directory and is never committed. The rules cannot be waived; the only exceptions are the ones a
rule itself names. Maintainers close pull requests that break them.

## Issues

Open an issue from one of the forms. Labels are set by the forms and kept up to date by
`.github/workflows/issues.yml`.

| Form | Labels | Needs a vote |
|------|--------|--------------|
| Bug report | `type: bug`, `status: triage` | no |
| Feature request | `type: feature`, `status: voting` | yes |
| Change proposal | `type: change`, `status: voting` | yes |

Adding `type: feature` or `type: change` to any issue puts it to a vote too.

## Maintainers and access

The people with access and their responsibilities are listed in [`MAINTAINERS.md`](MAINTAINERS.md).

Write access, membership of `@Quavon-dev/maintainers`, admin rights, approval of the
`release-signing` environment and access to any secret are granted only after the existing
maintainers have reviewed the person: their merged contributions and reviews here, and how they
handled security reports. The decision is a vote of the maintainers in a private discussion, with
more 👍 than 👎 and at least two votes in favour. Each person gets only the access their work
needs. Organisation membership requires two-factor authentication. Maintainers review the list
once a year and when someone leaves, remove access that is no longer used, and rotate the secrets
that person could reach (`docs/release.md`, "Secrets and credentials").

## Voting

Maintainers (write access or above) vote with 👍 or 👎 reactions on the issue itself. The author's
own reaction does not count.

- More 👍 than 👎 → `status: approved`
- More 👎 than 👍 → `status: rejected`
- Otherwise → `status: voting`

Reactions do not trigger workflows, so votes are counted every five minutes and on every new comment on the
issue. Run the **Issues** workflow by hand to count right away.

## Pull requests

Every pull request to `main` needs the approval of a maintainer who did not author it; no person
can merge past that rule. The only exception is the update app, whose Chromium update pull
requests merge once CI on them has passed (`docs/release.md`, "Automatic engine updates").

A change to `Cargo.lock` needs `scripts/third-party-licenses.sh` run and its output committed; CI
checks it.

Link the issue with `Closes #123`. The issue gets `status: in progress`. A pull request that closes
an issue still in `status: voting` or `status: rejected` fails the **Issues** check; re-run it once
the issue is approved.
