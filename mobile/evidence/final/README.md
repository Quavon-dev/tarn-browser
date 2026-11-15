# Final re-checks (2026-11-14)

Android: `Medium_Phone_API_36.1` (Android 16, WebView 157), debug APK from
`mobile/android/build.sh`, `mobile/android/server.py` as logging server and proxy. iOS: iPhone 17
simulator (iOS 27.0), `nsh package --target ios-simulator`, the same server on `localhost:8771`.

Test-only on Android: Lockdown is HTTPS-only, and these emulator images are production builds (no root;
the system certificate installer did not accept a file), so the Android runs used a local debug build whose
`network_security_config.xml` named a throwaway test CA (7 days, key kept outside the repository) as a
`debug-overrides` trust anchor. That change was never committed; the app was reinstalled from the
committed sources afterwards.

## Finding 1: JavaScript under a JavaScript-off page (Android)

`locked.test` set to Lockdown in the Shield panel; `/unload` has `onpagehide`, `onbeforeunload`,
`onunload` and a button's `onclick` that each request `/beacon?ev=…`; `open.test/slow` answers after 6 s
and sets its title from a script.

| Check | Result | Evidence |
|---|---|---|
| Lockdown page has no JavaScript | Pass: clicking "beacon" sent nothing | `server-fixed.log` 19:22:58 |
| Typed JavaScript-on address while the Lockdown page is shown | Pass: the view is rebuilt first (`web view on store` at 19:23:11), no `/beacon` request, the target's script ran (`SLOW-JS-RAN`) | `server-fixed.log`, `logcat-fixed.txt`, `android/01-typed-from-lockdown.png` |
| Back from the Lockdown page to the JavaScript-on page | Pass: view rebuilt, no `/beacon`; the tab's history starts over (`canGoBack=false`) | `server-fixed.log` 19:23:33, `logcat-fixed.txt` |
| Baseline: `TarnWebView.java` from 245a1c1, same steps, plus "beacon" clicked during the 6 s load | No `/beacon` either: WebView 157 did not run the old page's handlers after `setJavaScriptEnabled(true)`, so the finding was not reproduced on this WebView; the fix is kept as the guarantee | `server-baseline.log`, `logcat-baseline.txt` |

## Finding 2: sign-in prompts

| Check | Android | iOS |
|---|---|---|
| Image and frame from another origin answer 401 | Pass: no prompt (`server-auth-popups.log` 19:26:17, `android/02-third-party-auth.png`) | Pass: no prompt (`server.log`, `ios/01-third-party-auth.png`) |
| Own origin, three wrong passwords | Pass: three prompts naming `http://public.test`, then the 401 page (`android/03-*.png`, `android/04-*.png`) | Pass: three prompts naming `http://localhost:8771`, then the 401 page (`ios/02-*.png`, `ios/03-*.png`, `server.log`) |

The first iOS run found an endless request loop once the attempt limit was reached (about 650 requests a
second, `ios/server-before-reject-fix.txt`): default handling retried the last failed credential. That
happened with the old limit too. Fixed by rejecting the protection space for refused password challenges
(`mobile/ios/src/handler.rs`); the rerun stopped after the third failure.

## Finding 3: popups

| Check | Result | Evidence |
|---|---|---|
| Android: five clicks on a `target=_blank` link within 10 s | Pass: three `popup` events, then `popup over the burst limit refused` twice | `logcat-auth-popups.txt` 19:27:36–19:27:44, `android/05-popup-burst.png` |
| iOS: one click calling `window.open` five times | Pass: three tabs opened (Tabs 18 → 21), three `/opened` requests | `server.log`, `ios/04-popup-burst.png` |
| iOS: `window.open` 300 ms after load without a gesture | Pass: no tab (Tabs 21 → 21) | `server.log`, `ios/05-no-gesture-popup.png` |
