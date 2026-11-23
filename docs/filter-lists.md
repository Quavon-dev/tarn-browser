# Filter lists

The `Filter lists` workflow (`.github/workflows/filter-lists.yml`) builds a signed bundle daily and
uploads it to the `filter-lists` GitHub release. The app verifies it with `shield::bundle::verify`
before it uses anything in it (threat S20).

1. **`build` job (no secrets).** Compiles `filter-bundle` without a cache, downloads every source in
   [`packaging/filters/sources.json`](../packaging/filters/sources.json) and the `!#include` files
   they name (plain file names from the same folder only), fetches the last published
   `manifest.json`, and runs `filter-bundle prepare`. That step applies uBlock Origin's `!#if`
   blocks for Tarn's tokens, inlines includes, removes `$domain=` entries that are not valid IDNA
   names from network rules (a rule limited only to such sites is dropped, and so is one whose
   site options its parser and the content-blocking conversion would read differently: more than
   one `$`, or more than one `domain=` or `from=` option; on every client, since older releases
   cannot convert it), normalises hosts files, wildcard lists
   and `||host^` rules to hostnames, drops public suffixes, single labels and the hosts in
   `shield::domains::PROTECTED`, checks each source against its `min`/`max` count, refuses a file
   whose count shrank by more than a fifth or grew by more than half since the last bundle, and
   writes the bundle files with `entries.json`.
2. **`sign` job (signing key).** Runs the binary from `build`; `filter-bundle build` only hashes the
   prepared files, writes `manifest.json` and signs it.
3. **`publish` job.** Uploads the list files first and `manifest.json` and `manifest.sig` last.

Run it by hand with **allow_change** ticked after checking upstream when a list legitimately
changed by more than the limits.

## Sources

| Category | Sources | Becomes |
|---|---|---|
| Ads | EasyList, AdGuard Base (without EasyList), uBlock filters, uBlock Unbreak | Network rules and element hiding |
| Trackers | EasyPrivacy, uBlock Privacy | Network rules |
| Annoyances | EasyList Cookie List (`fanboy-cookiemonster.txt` with its uBlock include), uBlock Cookie Notices (`annoyances-cookies.txt`) | Network rules and element hiding; the uBlock list is mostly scriptlets, which Tarn drops |
| Malware | URLhaus (malware-filter mirror), uBlock Badware, CERT Polska, Scam Blocklist, Global Anti-Scam Org, stalkerware indicators, HaGeZi Threat Intelligence mini | One domain set; URLhaus and Badware also as subresource rules |
| Adult | OISD NSFW, HaGeZi NSFW | One domain set |
| Pop-ups | HaGeZi Pop-Up Ads, plus pure-domain `$popup` rules of the ad and tracker lists (never the cookie lists) | One domain set |

Licences (checked 2026-11-13 at each project; EasyList Cookie 2026-11-14): EasyList, EasyPrivacy and EasyList Cookie GPL-3.0-or-later or CC BY-SA 3.0-or-later, attributed to "The EasyList authors"; AdGuard,
uBlock Origin, HaGeZi and OISD GPL-3.0; URLhaus mirror CC0 and MIT; CERT Polska open data for any
purpose; Scam Blocklist MIT; Global Anti-Scam Org BSD-3-Clause; stalkerware indicators CC BY 4.0.
`licenses.txt` in every bundle repeats them with the URLs. Prepared files are modified versions
under their source's licence; Tarn's code does not include them. HaGeZi's threat feed aggregates
other feeds whose own terms were not checked; it is the only phishing source, and dropping it
lowers the malware set from about 279,000 to about 158,000 domains.

Rejected: AdGuard Tracking Protection (15 MB, overlaps EasyPrivacy), AdGuard URL Tracking and uBlock
Quick fixes (almost only `$removeparam` and scriptlets, which Tarn cannot run), Peter Lowe's list
(forbids money-making redistribution), URLhaus direct (needs a key), OpenPhish, Phishing Army and
phishing-filter (OpenPhish terms forbid redistribution), blocklistproject (stale), UT1 (4.6 million
entries).

## Bundle format 2

| File | Kind | Content |
|---|---|---|
| `manifest.json` | | `{"format":2,"version":<u64>,"created":"<rfc3339>","lists":[{"name","sha256","size","category","kind","count"}]}`; `version` is UTC epoch seconds and must increase |
| `<source>.txt` | `filters` | Preprocessed EasyList-syntax text, category `ads`, `trackers` or `annoyances` |
| `<category>.set` | `set` | Sorted, de-duplicated first 12 bytes of SHA-256 over each hostname (`shield::domains::DomainSet`) |
| `malware-rules.txt` | `domains` | One hostname per line, for subresource rules |
| `youtube.json` | `config` | `{"keys":[...]}`: the player fields the YouTube script removes |
| `theme-revocations.json` | `config` | Category `themes`; `{"sha256":[...]}`: theme files withdrawn as unsafe (`shield::revocations`, threat model S29), from `packaging/filters/theme-revocations.json`; the app keeps every hash it has seen |
| `licenses.txt` | `license` | Sources and licences |
| `manifest.sig` | | Raw 64-byte Ed25519 signature over the exact bytes of `manifest.json` |

The app rejects a bundle with a bad signature, an unknown format, a version not newer than the last
accepted one, a name outside `[a-z0-9._-]+`, a category outside `[a-z]{1,16}`, a file over the cap
for its kind (filters 12 MiB, sets 12 MiB, domains 2 MiB, config 64 KiB), a size or SHA-256
mismatch, or more than 32 MiB in total. Entries of a category or kind it does not know are verified
and ignored.

## Signing key setup (maintainer, once)

1. On a trusted machine, run `cargo run -p filter-bundle -- keygen`. It prints the seed and the public key to stdout and writes nothing to disk. Keep the terminal output out of shell history, logs and screenshots.
2. In the repository settings, add an Actions secret `FILTER_SIGNING_KEY` with the base64 seed (the value after `FILTER_SIGNING_KEY=`). Do not store the seed anywhere else unless it is in a password manager.
3. Put the printed public key bytes into `FILTER_BUNDLE_PUBLIC_KEY` and the release download URL (`https://github.com/<owner>/<repo>/releases/download/filter-lists/`) into `FILTER_BUNDLE_URL`, both in `crates/shield/src/bundle.rs`, and commit. While either is unset the app downloads nothing and rejects every bundle; the YouTube and pop-up protection work without them.
4. Every job runs on a fresh GitHub-hosted runner: `build` handles third-party downloads; the signer binary is built and hashed by the `signer` job, which reads no list, and `sign` runs only that binary after checking its hash, with the key from the `filter-signing` environment.
5. Run the workflow once by hand (Actions → Filter lists → Run workflow) and check that the `filter-lists` release has the bundle files.

## Key rotation

Generate a new key as above, replace the secret, update `FILTER_BUNDLE_PUBLIC_KEY` and ship an app release. Builds with the old key reject bundles signed with the new one, so they keep their last good bundle until updated. If the seed may have leaked, rotate immediately and delete the release assets signed with it.

## In the app

`shield::update` checks once a day while **Settings › Privacy › Filter-list updates** is on (the
default), on macOS, Windows (`app::filters`) and Linux (`linux/src/filter_updates.rs`). It downloads `manifest.json` and `manifest.sig`, verifies them, then downloads every list,
refuses a bundle older than 30 days, and stores it under `<data dir>/filters/bundle` with
`state.json`. Each rule-list group (`ads`, `trackers`, `cosmetic`, `annoyances`, `malware`, and the built-in
`youtube`) is compiled by WebKit under `tarn.lists.<group>.<fingerprint>.<n>` (on Linux, WebKitGTK's
`tarn-<group>-<key>-<n>`) and looked up again on
launch, so conversion runs only when a group's input or the converter changes. Ads and trackers each
carry the other's exceptions, so either switch works alone; `annoyances` holds the cookie lists'
network and element-hiding rules, with the network exceptions of all three (element-hiding exceptions only from the cookie lists themselves), so uBlock Unbreak's fixes for
consent scripts apply, under the **Block cookie banners** switch (on by default). Element-hiding
caps apply per group: EasyList Cookie has about 15,000 generic selectors, so those past the first
10,000 are left out. Hiding a banner neither accepts nor refuses anything; a site
that locks scrolling until consent may stay locked, since the scriptlets that undo that are not
run. Domain sets are read, checked and kept
in memory only for categories switched on; turning one on reads the cached bundle again, verified.
A new bundle replaces the cached one only after every group compiled; a failed check or rebuild
keeps the installed lists, the cached bundle and the version floor, and shows the error in Settings.

Debug builds accept a test signing key from `TARN_TEST_FILTER_KEY` (64 hex digits) so
`app/tests/blocking.rs` can install a bundle it signed; release builds never read it.
