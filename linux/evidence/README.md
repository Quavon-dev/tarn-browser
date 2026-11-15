# P2 exit checks (Linux)

Run 2026-11-14 in Docker on Apple silicon (aarch64): Fedora 43, GTK 4.20, WebKitGTK 6.0 2.54, NSH
0.5.0, Rust 1.99.1, Xvfb 1280×800 without a window manager unless noted, software GL. Driven with
xdotool through `linux/scripts/scenario.sh`; each `<name>.txt` holds the component's events and the
test server's request log, `<name>.png` the screen afterwards. Screens taken before the design pass
(`nav-*`, `popups*`, `popup-*`, `https-only-*`, `private-network-*`, `shield-site-strict`) show the
first, unstyled layout; the behaviour they record is unchanged.

Test-only: `WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1`, `LIBGL_ALWAYS_SOFTWARE=1`, a test
certificate trusted inside the container for `trusted.test` (`scripts/trust-test-cert.sh`) and a
filter bundle signed with a throwaway key (`scripts/test-bundle.sh`, `TARN_TEST_FILTER_KEY`, debug
builds only). Not run anywhere: a real Wayland or X11 desktop, HiDPI, a composed IME, a screen reader.

| Check | Result | Evidence |
|---|---|---|
| Browsing | Pass | `https-only-typed-default` (example.com), `design-page.png`, `keys.txt` |
| Tabs and spaces | Pass: new, close, reopen, Ctrl+1, Ctrl+Tab, back/forward, popups as tabs, two profiles' spaces | `keys.txt`, `keys-*.png`, `popups*`, `popup-open*`; host tests `browser::tests` |
| Profiles isolated | Pass: profile B started without A's cookie; after A set `tarn=a`, B's store still held its own value | `profile-b-cookie.*`, `profile-a-cookie-2.*`, `profiles-disk.txt` |
| Private space leaves nothing on disk | Pass: before/after file lists differ only in `tarn.db` (WAL checkpoint) and the Xvfb log; the test value is nowhere on disk. Display test: an ephemeral session's cookie, localStorage and IndexedDB entry reach no file, while the persistent control's do | `private-space.*`, `private-before.txt`, `private-after.txt`, `private-diff.txt`, `ephemeral-test.txt` |
| Private cookies survive a space switch | Pass (after review fix) | `fix-private-session.*` |
| Shield: private-network block | Pass: subresource from a public page not requested; clicked link, popup and Ctrl-click to 127.0.0.1 blocked; control from a local page loads | `private-network-public.*`, `private-network-local-control.*`, `fix-popup-private-network.*` |
| Shield: HTTPS-only incl. non-default port | Pass: typed `localhost:8765` and a clicked link to it were upgraded (TLS attempted, no plain `GET /insecure`); `http://example.com/` committed as https | `https-only-typed-port.*`, `https-only-link-port.*`, `https-only-typed-default.*` |
| Shield: Lockdown | Pass: page JavaScript off for a site set to Lockdown, by address and by a clicked link | `lockdown.*`, `fix-link-into-lockdown.*`, `https-trusted-control.*` (JavaScript on at Standard) |
| Filter lists with a bundle | Pass: the signed bundle's rule blocked the marked request, the control loaded; switching "Block ads" off reloaded the current page with it allowed | `filter-lists.*`, `settings-ads-off.png` |
| Prompts | Pass: alert, confirm (`confirm:true`), text (`prompt:linux`), location (Deny → `geo:1`), HTTP sign-in with origin and realm, credentials stored nowhere, a flood of 20 alerts limited to 5 | `prompt-*`. Camera **not tested** (no capture device in the container) |
| TLS errors | Pass: untrusted certificate fails, no way to continue | `tls-error.*` |
| Crash reload | Pass: first crash reloads, a second within a minute shows an error | `crash.txt`, `crash-second.png` |
| Downloads | Pass: saved to `~/Downloads` as `evil_gpj.sh` (from `../evil<RLO>gpj.sh`), mode 0644, `user.xdg.origin.url` set, not opened; listed in the panel | `download.*`, `design-downloads.png` |
| Find | Pass: 5 matches, highlights | `find.txt`, `find-bar.png` |
| Zoom | Pass: 125 %, stored per site and restored | `zoom.*` |
| Reopen closed tab | Pass | `keys.txt`, `keys-3-closed.png`, `keys-4-reopened.png` |
| Window restore | Pass for size, space and tab (private space not restored); position is left to the window manager, which GTK 4 cannot override on X11 | `window-restore.txt`, `openbox-resized.png`, `window-restored.png` |
| Keyboard shortcuts | Pass: Ctrl+T, Ctrl+W, Ctrl+Shift+T, Ctrl+L, Alt+←/→, Ctrl+1, Ctrl+Tab, Ctrl+F, Ctrl+=, Ctrl+Shift+N, Ctrl+Q, also with the page focused | `keys.txt`, `find.txt`, `zoom.txt` |
| AT-SPI tree | Partial: buttons carry names, tab rows and web content are present; the NSH address field is only a `grouping` without a text role (NSH gap) | `atspi-tree.txt` |
| With and without a window manager | Pass: openbox and no WM | `openbox-resized.png`, all others |
| Design | Matches the macOS look (see `docs/nsh-plan.md` P2 "Design") | `design-*.png`, `prompt-*.png`, `find-bar.png` |
| Distribution packages | Pass on all four, see below | `install-*.txt`, `install-*.png` |

## Packages in clean containers

Built from commit `ac34d96` with `linux/packaging/build-all.sh` in a clean container of each
distribution, installed with its package manager in another clean container
(`linux/packaging/install-test.sh`), then started under Xvfb for 25 s.

| Distribution | Package | WebKitGTK 6.0 installed from the distribution | Result |
|---|---|---|---|
| Debian 13 (arm64) | `tarn_0.1.0_debian13_arm64.deb`, `Depends: … libwebkitgtk-6.0-4 (>= 2.39.7)` (dpkg-shlibdeps) | `libwebkitgtk-6.0-4` 2.54.0-1~deb13u3 | Pass: installs, links `/lib/aarch64-linux-gnu/libwebkitgtk-6.0.so.4`, starts, start card shown |
| Ubuntu 26.04 (arm64) | `tarn_0.1.0_ubuntu26.04_arm64.deb` (built on Ubuntu; needs glibc 2.43, so it is not the Debian package) | `libwebkitgtk-6.0-4` 2.52.6-0ubuntu0.26.04.1 | Pass |
| Fedora 43 (arm64) | `tarn-0.1.0-1.fc43.aarch64.rpm`, `Requires: webkitgtk6.0`, `libwebkitgtk-6.0.so.4()(64bit)` | `webkitgtk6.0` 2.54.2-1.fc43 | Pass |
| Arch (x86_64, emulated) | `tarn-0.1.0-1-x86_64.pkg.tar.zst` from the `PKGBUILD` with a local source tarball, `Depends On: webkitgtk-6.0 gtk4 …` | `webkitgtk-6.0` 2.54.2-1 | Pass |

Not covered: amd64 `.deb`/`.rpm` (only arm64 built here), an AUR upload, the `PKGBUILD` with the
real tag download (checksum `SKIP` until `v0.1.0` exists), desktop integration on a real session
(`.desktop` entry and icon are installed; `xdg-settings` was not run).
