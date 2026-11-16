# Cryptography

What cryptography Tarn uses, the key lengths it relies on, and how to switch smaller keys off.
It is the evidence for the OpenSSF Best Practices answers `crypto_published`, `crypto_floss`,
`crypto_keylength`, `crypto_working`, `crypto_weaknesses` and `crypto_pfs`. Trust boundaries
and their mitigations are in `threat-model.md`.

The thresholds are the ones that criterion uses (NIST, valid through 2030): symmetric key 112
bits, RSA modulus 2048, discrete-logarithm group 2048 with a 224-bit key, elliptic curve 224 bits,
hash 224 bits. Password hashing is excluded.

## Tarn's own mechanisms

Tarn implements no primitive itself; each comes from an open-source library pinned in the
lockfiles. The algorithms and sizes below are fixed in code. No setting, file or server can select
a smaller one, so smaller key lengths are always off.

| Mechanism | Algorithm and size | Library | Code |
|---|---|---|---|
| Filter-list bundle | Ed25519 signature (255-bit curve), SHA-256 per file | `ed25519-dalek`, `sha2` | `crates/shield/src/bundle.rs` |
| Windows update feed | Ed25519 signature, SHA-256 of the installer | `ed25519-dalek`, `sha2` | `crates/platform/src/updater/windows.rs` |
| macOS updates | Sparkle EdDSA (Ed25519) signed feed and archive (`SURequireSignedFeed`, `SUVerifyUpdateBeforeExtraction`) | Sparkle 2 | `packaging/Info.plist`, `release.md` |
| Filter signing key | Ed25519 seed from the OS random number generator | `getrandom` | `crates/filter-bundle/src/main.rs` |
| Release files | SHA-256 (`SHA256SUMS`), GitHub artifact attestations | coreutils, GitHub | `.github/workflows/release.yml` |
| Sync file format (experimental, not built into any client) | Argon2id, at least 64 MiB, 3 passes (password hashing), into a 256-bit key; XChaCha20-Poly1305 with a random 192-bit nonce; weaker KDF headers are refused | `argon2`, `chacha20poly1305` | `crates/sync/src/file.rs` |

## TLS

TLS is never Tarn's code. Pages load through the engine's TLS stack. The shell's own requests
(updates, filter lists, extensions, themes, listed in `PRIVACY.md`) use the client below. Filter
lists and updates are Ed25519-signed, so their integrity does not rest on TLS key sizes. No client
lets a certificate error through (threat model S7).

| Client | Pages | Shell requests | Default key lengths | Switching smaller keys off |
|---|---|---|---|---|
| macOS | WebKit (Apple) or Chromium (CEF) | `ureq` with rustls and `ring`, certificates checked by the platform verifier; Sparkle uses `URLSession` | Apple: TLS server and issuing-CA RSA keys under 2048 bits and SHA-1 signatures are not trusted ([Apple](https://support.apple.com/103769)). rustls with `ring` verifies only RSA 2048–8192, ECDSA P-256/P-384 and Ed25519 signatures and offers only X25519, P-256 and P-384 key exchange. Chromium: see below. | WebKit, rustls: already off. Chromium: see below. |
| Windows | Chromium (CEF) | `ureq` with rustls and `ring` (as above); the certificate chain is checked by Windows | rustls as above for the server's key. Windows refuses RSA keys under 1024 bits in the chain by default. Chromium: see below. | Server key: already off. Chain: `certutil -setreg chain\minRSAPubKeyBitLength 2048` (Windows' weak-key setting). Chromium: see below. |
| Linux | WebKitGTK over glib-networking (GnuTLS in the Flatpak's GNOME runtime) | libsoup, same TLS stack | The GnuTLS priority of the distribution (Fedora's system crypto policy, for example) or of the Flatpak runtime; Ubuntu's default ends in `%PROFILE_MEDIUM` ([Ubuntu](https://ubuntu.com/server/docs/explanation/crypto/gnutls/)) | `%PROFILE_MEDIUM` requires 112-bit security: RSA/DH 2048, ECC 224 ([GnuTLS](https://www.gnutls.org/manual/html_node/Selecting-cryptographic-key-sizes.html)). glib-networking reads its priority from `G_TLS_GNUTLS_PRIORITY`: start Tarn with `G_TLS_GNUTLS_PRIORITY=NORMAL:%PROFILE_MEDIUM`, or for the Flatpak run `flatpak override --user --env=G_TLS_GNUTLS_PRIORITY=NORMAL:%PROFILE_MEDIUM de.quavon.tarn`. Package installs also follow `min-verification-profile = medium` in the host's GnuTLS system configuration; the Flatpak does not read the host's. |
| iOS | WKWebView (Apple) | `URLSession` | Apple, as on macOS | Already off |
| Android | Android System WebView (Chromium) | `HttpURLConnection` (Conscrypt, BoringSSL) with Android's certificate checking | Pages: Chromium, see below. Shell requests: Android sets the floor; Tarn adds none, and it is not verified | Pages: see below. Shell requests: no setting exists. Filter lists stay Ed25519-verified whatever the TLS key |

**Chromium (CEF, Android System WebView).** Chromium rejects RSA certificate keys under 1024 bits
and offers no finite-field Diffie-Hellman (DHE was removed in Chrome 53,
[Chrome](https://developer.chrome.com/blog/chrome-53-deprecations/)); its key-exchange groups are
X25519 and NIST P-256 and P-384, alone or in a hybrid with ML-KEM. Publicly trusted
certificates must have RSA keys of at least 2048 bits or P-256 and larger curves under the
CA/Browser Forum Baseline Requirements, so public sites meet the minimums. A certificate with a
1024–2047-bit RSA key chained to a root the user added locally still loads, and neither Chromium
nor CEF offers a setting that refuses it. Use the WebKit engine on macOS where that matters.

## Forward secrecy

rustls and Apple's `URLSession` under App Transport Security (no exception in `Info.plist`)
negotiate only ephemeral (EC)DHE key exchange, so the shell's requests on macOS, Windows and iOS
are forward secret. WebKit, WebKitGTK and Chromium choose their own cipher suites, and some may
still offer RSA key exchange to servers that support nothing else. That is not verified per engine
version, so forward secrecy for pages is not claimed. Signatures and the sync file format do not
negotiate sessions.

## Reading another browser's cookies

Import (`crates/import`) reads Chromium's existing cookie format: PBKDF2-HMAC-SHA1 (1003
iterations) over the browser's Safe Storage password, then AES-128-CBC. It runs only when the user
imports from that browser, only to decrypt that browser's data, and Tarn never uses the format for
its own storage. AES-128 meets the symmetric minimum. PBKDF2 here derives a key from a password
and is required to read the format. SHA-1 appears only as PBKDF2's HMAC because that format
requires it; Tarn protects nothing of its own with SHA-1, CBC or this key.

## Open source

Every primitive above has an open-source implementation, and Tarn uses one: RustCrypto and
`ed25519-dalek` for its own mechanisms, rustls and `ring` for its requests on macOS and Windows,
GnuTLS for WebKitGTK, and BoringSSL for Chromium and Android. WebKit's and `URLSession`'s TLS on
Apple platforms is part of the operating system; the same protocols are available in open-source
stacks, so all functionality that depends on cryptography can be implemented with FLOSS.
