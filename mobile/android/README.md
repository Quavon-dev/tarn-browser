# Tarn for Android

The Android host of the Tarn mobile app (`docs/mobile-plan.md` M3).

| Path | What it is |
|---|---|
| `host/` | Gradle app `dev.tarn.browser`: `MainActivity` and `PrivateActivity` run the NSH Rust app with `NshView.runRustApp`, `TarnWebView` is the `tarn.webview` component (contract v2), `SweepService` deletes a closed private session |
| `rust/` | Crate `tarn-mobile-android`: the JNI calls the component makes (`nav_policy`, Shield verdicts, download names), the bridge back to Java, and `Data`, the `browser_core::WebsiteData` for Android |
| `build.sh` | NSH embed runtime (SHA-256 checked), `nsh package --target android` of the shared app `../app`, `libnsh.so` into the host, `gradlew clean assembleDebug` |
| `server.py` | Logging test server and proxy for the exit checks |
| `evidence/` | Results of the M3 exit checks on the emulator |

## How the pieces fit

- `tarn-mobile-android` is linked into the app's `libnsh.so` as an rlib, so the JNI symbols, `Data` and the
  Rust app share one copy of the state. Java's `Native` class loads `libnsh.so` and calls `Native.start`
  before `runRustApp`; Rust reaches Java through the `Host` class it was given there.
- **Profiles:** `Data::store_id()` is the component's `profile` prop. A persistent store is the
  `ProfileStore` profile `tarn-<uuid>`. `private:<session>` is the `:private` process's own WebView
  directory there, and a throwaway profile `tarn-private-<session>` in the main process (Lockdown), deleted
  at the next launch.
- **Private space:** `open_private_space()` starts `PrivateActivity` in `:private` with a one-time
  session token. That process sets `ProcessGlobalConfig.setDirectoryBasePaths` to `private/<session>/`
  before any WebView exists, disables its Recents screenshot and sets `IME_FLAG_NO_PERSONALIZED_LEARNING`
  on every text field. Closing it (`close_private_space()`, Back at the root, or finishing the activity)
  destroys the views, asks `SweepService` in the main process to delete `private/` once the process has
  exited, and kills the process. Every main-process launch with no `:private` process running deletes
  `private/`, the unused tokens and `shared_prefs/AwOriginVisitLoggerPrefs.xml`. A private activity that
  Android restores by itself finds no token, closes and triggers the sweep.
- **Shield:** built-in rules go through `engine::content_rules` (built on a background thread at
  process start), filter lists through `shield::matcher` (`install_filters`). Service-worker requests
  of every profile are checked at Standard with no top page, which counts as third-party: the
  private-network rule and third-party filter rules apply, first-party-only rules do not, and a local
  web app's service worker cannot fetch from its own local address. Every built-in rule is understood
  by `engine::content_rules` (tested); a rule with a trigger it does not know is skipped, an exception
  included, so it can only block more. WebSocket requests are not visible to WebView clients.
- **Downloads** go to `files/Downloads/`; in the private space to `private/<session>/Downloads/`, which
  is deleted with the session.
- **Level changes** apply from the next navigation. A main-frame request WebView does not announce
  (a form POST) into a site whose level turns JavaScript off is refused with `blocked` reason
  `rebuild`; the shell tells the user to open it again from the address bar (a GET shell load).
  JavaScript is never switched on while a JavaScript-off page is loaded: a shell load or history step
  from such a page to a JavaScript-on page rebuilds the view first, so that tab's history starts over.
  Links followed from it keep JavaScript off (a page-started load keeps the stricter level).

## How `tarn-mobile` uses it

`tarn-mobile-android` is a member of `mobile/Cargo.toml` and an Android-only dependency of `mobile/app`.
The shell passes `Data::store_id()` as `profile`, takes its data directory from `host()`, and goes through
the `data::Platform` trait for the private space: "Private Space" calls `open_private_space()`, and in the
private process the shell shows only the private space and "Close Private Space" calls
`close_private_space()`. A `popup` tab gets the opener's committed URL as `load.opener`.

## Build and run the checks

```sh
cd mobile/android
./build.sh                                   # nsh 0.5.0 on PATH, ANDROID_HOME, rustup with the Android targets
adb install -r host/build/outputs/apk/debug/tarn-android-debug.apk
adb shell "run-as dev.tarn.browser sh -c 'echo localhost:8771 > /data/user/0/dev.tarn.browser/debug-proxy'"
openssl req -x509 -newkey rsa:2048 -nodes -days 7 -subj /CN=public.test \
  -addext subjectAltName=DNS:public.test -keyout key.pem -out cert.pem   # outside the repository
python3 server.py 8771 server.log cert.pem key.pem &
adb reverse tcp:8771 tcp:8771
adb logcat -s tarn
```

The debug build only: cleartext is allowed for `localhost`, `127.0.0.1` and `*.test`, and the
`debug-proxy` file sends WebView traffic through `server.py`, so `http://public.test/` pages count as
public. Release builds allow no cleartext and have no proxy hook or logging.
