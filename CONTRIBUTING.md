# Contributing

Everyone taking part follows the [Code of Conduct](CODE_OF_CONDUCT.md). Read
[`AGENTS.md`](AGENTS.md) for how the five clients and the shared crates fit together.

## Prerequisites

- macOS 14 or later with Xcode (or its command line tools).
- Rust through [rustup](https://rustup.rs); the version is pinned in `rust-toolchain.toml` and
  installed on first use, with clippy and rustfmt.
- [`cargo-deny`](https://github.com/EmbarkStudios/cargo-deny) for the licence and advisory check.
- For Linux, iOS and Android: Docker, [Native Shell](https://nsh.dev/) 0.5.0, the Android SDK, NDK
  and JDK 17+, as described in the README.

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
| root | `cargo deny check` |

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
identity.

## AI-assisted contributions

AI agents and AI-assisted tools are allowed. They follow the "Rules for AI agents" in
[`AGENTS.md`](AGENTS.md); you are the author of what they produce, you review it before you submit
it, and you sign it off yourself. The rules cannot be waived; the only exceptions are the ones a rule
itself names. Maintainers
close pull requests that break them.

## Issues

Open an issue from one of the forms. Labels are set by the forms and kept up to date by
`.github/workflows/issues.yml`.

| Form | Labels | Needs a vote |
|------|--------|--------------|
| Bug report | `type: bug`, `status: triage` | no |
| Feature request | `type: feature`, `status: voting` | yes |
| Change proposal | `type: change`, `status: voting` | yes |

Adding `type: feature` or `type: change` to any issue puts it to a vote too.

## Voting

Maintainers (write access or above) vote with 👍 or 👎 reactions on the issue itself. The author's
own reaction does not count.

- More 👍 than 👎 → `status: approved`
- More 👎 than 👍 → `status: rejected`
- Otherwise → `status: voting`

Reactions do not trigger workflows, so votes are counted every five minutes and on every new comment on the
issue. Run the **Issues** workflow by hand to count right away.

## Pull requests

A change to `Cargo.lock` needs `scripts/third-party-licenses.sh` run and its output committed; CI
checks it.

Link the issue with `Closes #123`. The issue gets `status: in progress`. A pull request that closes
an issue still in `status: voting` or `status: rejected` fails the **Issues** check; re-run it once
the issue is approved.
