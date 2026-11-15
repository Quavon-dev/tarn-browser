# M4 retried checks (2026-11-14)

The APK is the debug APK `mobile/android/build.sh` built with nsh 0.5.0
from `nsh-macos-arm64.tar.gz` (SHA-256 checked); a rebuild with the CI job's exact setup (that archive
plus `nsh-android.tar.gz`, both SHA-256 checked, and an empty `HOME`) gave an APK of the same size. The
iOS app is `nsh package --target ios-simulator`.

## iOS: private network from a public top origin (`ios/`)

iPhone 17 simulator, iOS 27.0, Shield level Standard. The private-network rule judges the URL, not
the DNS answer (`shield::rules::is_local`, S10), so the public top page is served from
`https://127.0.0.1.nip.io:8446`: a public DNS name whose answer is `127.0.0.1`, which the simulator
reaches through the Mac's loopback. `pn_server.py` serves it and the local targets
(`https://127.0.0.1:8447`, `http://127.0.0.1:8775`) and logs every request (`server.log`). Its
certificate is signed by a throwaway test CA (7 days) added to the simulator's keychain with
`xcrun simctl keychain … add-root-cert`; the CA and keys stayed outside the repository.

| Check | Result | Evidence |
|---|---|---|
| Subresources from the public page: HTTPS image and `fetch()` to `127.0.0.1` | Blocked: no request reached the local server; the page shows "fetch: refused". The page's HTTP image (mixed content, which WebKit refuses anyway) and its `localhost` image (the server listens on IPv4 only) prove nothing about Shield | `01-public-page.png`, `server.log` |
| Clicked local link, script navigation, `target=_blank` popup to `127.0.0.1` | Blocked, Shield notice | `02-local-link.png`, `02-script-to-local.png`, `02-local-popup.png` |
| Popup that redirects to `127.0.0.1` | Blocked: only the public `/redirect` request reached the server | `02-popup-redirecting-to-local.png`, `server.log` 18:38:24 |
| Control: the local page typed into the address bar loads its local HTTPS image | Pass (so the certificate is trusted and the refusals above come from Shield) | `03-local-control.png`, `server.log` 18:38:44 |

## Android phone: `Medium_Phone_API_36.1` (`android/`)

Android 16, WebView 157.0.8103.38. The emulator had 399 MB free; the 48 MB APK installed without wiping
it, and was uninstalled afterwards. `mobile/android/server.py` as logging server and proxy
(`server.log`), component events in `logcat.txt`.

| Check | Result | Evidence |
|---|---|---|
| Browsing | Pass | `01-pn-public.png` |
| Private network from `http://public.test/pn` | Pass: three `blocked {reason: shield}` events, no `/pixel` request | `01-pn-public.png`, `logcat.txt`, `server.log` |
| Tabs: `target=_blank`; no-gesture `window.open` | Pass: Tabs 1 → 2; no `no-gesture` request | `02-blank-target-tab.png`, `server.log` |
| External scheme | Pass: `external {url: mailto:…, origin: http://public.test}`, the sheet names the site | `03-mailto-asks.png`, `logcat.txt` |
| Navigation policy | Pass: link to `localhost` blocked (`shield`), never requested; `data:` refused by Chromium | `logcat.txt` 18:31:01 |
| Prompts | Pass: confirm named "http://public.test asks", OK → `confirm true` | `04-prompt-confirm.png`, `logcat.txt` |
| Find | Pass: `found {matched: 3}`, "3 matches" | `05-find.png` |
| Zoom | Pass: text grows with each Zoom In (`06-zoom-in-1.png`, `06-zoom-in-2.png` against `05-find.png`) | `06-*` |
| Downloads | Pass: `../../evil:name.txt` saved as `files/Downloads/evil_name.bin`, mode 0600, `url` and `quarantined: false` in `finished` | `07-*`, `logcat.txt` |
| HTTPS-only at Strict | Pass: typed `http://public.test/plain` sent only `CONNECT public.test:443` | `08-settings-strict.png`, `09-*` |
| TLS error | Pass: the self-signed certificate failed the load ("The site's certificate is not valid"), no way through | `09-https-only-strict.png`, `logcat.txt` 18:32:59 |
| Private space | Pass: `:private` process; marker in `private/<session>/` only, no Personal cookie sent; after Close the process ended, `private/` was empty and the marker was nowhere | `10-*`, `11-*` |
| Profiles | Pass: `tarn=personal` set in Personal; a new profile "Work" sent no cookie; back in Personal the cookie was sent; `app_webview` holds `Profile 1` and `Profile 2` | `12-*` |
| Accessibility tree | Pass: every button named (no unnamed button in the main screen or the menu) | `13-ax-*.txt` |
| Rotation | Pass: landscape shows the wide top bar, same process (pid unchanged) | `14-*` |

Not run here: crash reload, filter lists (no bundle), camera, microphone and location capture,
sign-in, TalkBack.
