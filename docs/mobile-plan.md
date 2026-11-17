# Mobile apps: iOS and Android on Native Shell

Status: in progress (nsh-plan P3/P4). This builds on `docs/nsh-plan.md` (decisions and P0 results) and
`docs/shared-core.md` (web-view contract, `browser-core`). macOS and Windows code is not changed by this work.

## 1. Layout

```
mobile/                       own Cargo workspace (NSH stays out of the root workspace and Cargo.lock)
  app/                        crate `tarn-mobile`: the NSH Rust app (phone and tablet), nsh.json; theme, icons
                              and controls come from `../nsh-shared` (crate `tarn-nsh-shared`)
  ios/                        crate `tarn-mobile-ios`: `tarn.webview` for iOS (WKWebView, objc2) and
                              `WebsiteData` for iOS (WKWebsiteDataStore per profile, nonPersistent)
  android/                    Gradle host app embedding the Rust app (`NshView.runRustApp`), the Java
                              `tarn.webview` component, and crate `tarn-mobile-android` (JNI: policy,
                              Shield verdicts, `WebsiteData` for Android)
```

`tarn-mobile` depends on `browser-core`, `store`, `shield`, `engine` (platform-neutral modules) by path. It
picks the data type per target in `app/src/data.rs` (`tarn_mobile_ios::Data`, `tarn_mobile_android::Data`, and
the in-memory `data::Memory` for host tests). Everything platform-specific the shell needs sits behind the
`data::Platform` trait implemented for those three types: `store_id()` (the `profile` prop), `data_dir()` (Android:
`files/`, or `private/<session>/files/` in the private process), `private_process()`, and
`open_private_space()` / `close_private_space()` (Android: hand off to the `:private` process; iOS: `None`, the
shell switches spaces in place). The rest of the shell and UI is one code path. The iOS component is registered
with `#[native_components(ios)]` in `app/src/lib.rs`; on Android the host app registers it
(`ShellActivity.registerComponent`). Build: `cd mobile/app && nsh package --target ios-simulator`,
`mobile/android/build.sh`; tests: `cd mobile && cargo test`.

## 2. The `tarn.webview` component (contract v3)

One contract for iOS, Android and Linux (`linux/src/webview/`); the pure policy all three share is
`engine::view_policy` (site levels with `"*"`, bursts, the zoom range, the Linux navigation verdict). One component per tab. Every policy decision is made inside the component, synchronously: `engine::nav_policy`
(`decide`, `level_action`) for main-frame navigations and new windows, and Shield's rules
(`WKContentRuleList` on iOS, `engine::content_rules` or `shield::matcher` on Android) for requests. Events
only report. There is no page-to-app bridge: no script message handlers or JavaScript interfaces. JavaScript is
off at Lockdown, TLS errors are never bypassed, and there is no remote debugging.

| Kind | Name | Payload | Meaning |
|---|---|---|---|
| prop | `tab` | string | Stable tab id; the component's identity |
| prop | `profile` | string | Data store id from `Data::store_id()`: the canonical (lower-case) store UUID for a persistent store, `private:<32 hex digits>` for an ephemeral one (private space and Lockdown). Every ephemeral store gets a new id, so a new id means a new, empty store |
| prop | `level` | `standard` / `strict` / `lockdown` | Shield level the view is built at (site overrides come as `levels`) |
| prop | `levels` | object host→level | Per-site overrides, so the component can apply `level_action` itself; the key `"*"` is the profile's own level, used for hosts without an entry (`level` when it is absent). An unknown level counts as Lockdown |
| prop | `lists` | array of `ads`, `trackers`, `cosmetic`, `malware` | The switched-on filter-list groups (`shield::groups`) |
| prop | `load` | `{url, seq, opener?}` | A shell load (typed, restored, new tab); a new `seq` loads again even for the same URL. `opener` is the committed URL of the tab that emitted `popup`: the load is then judged as a popup from that page (`nav_policy` source `Popup`, private network checked against the opener, redirects before commit too) |
| prop | `active` | bool | Foreground tab; background views may be suspended |
| command | `back`, `forward`, `reload`, `stop` | none | |
| command | `find` | `{text, backwards}` | Emits `found` |
| command | `answer` | `{id, accept, text?, user?, password?}` | Answers a `prompt`; an unanswered prompt is declined when the component is destroyed |
| command | `zoom` | `{factor}` | 0.5 to 3.0 (`engine::view_policy::zoom_factor`); 1.0 is the page's own size |
| event | `committed` | `{url, title, canGoBack, canGoForward}` | Main frame committed |
| event | `finished` | `{url, title, canGoBack, canGoForward}` | Load finished |
| event | `title` | `{title}` | Title changed (pages often set it after `finished`) |
| event | `failed` | `{url, message}` | Load failed (not for cancellations) |
| event | `blocked` | `{url, reason, popup?}` | `nav_policy`, `https_only`, `local_network` (S10 refused a main-frame navigation of this tab into the local network; the only reason that may offer Open Anyway), `shield` (any other Shield block: rules, filter lists, Android subresources) or `rebuild` (the level needs a new view). `popup` is true when the page's pop-up was refused; the opener keeps loading and gets no Open Anyway |
| event | `external` | `{url, origin}` | Another app's scheme; the shell asks before handing it off. `origin` is the page that asked, empty for the shell's own loads |
| event | `popup` | `{url, link?}` | A user-gesture popup allowed by `nav_policy`; the shell opens it as a new tab. `link` (v3) is true for a link that opens a new window (iOS: `WKNavigationTypeLinkActivated`; Android: the tap's hit test is an anchor). As on macOS, only a link pop-up goes to the space a rule names, as a fresh tab without an opener; a script's `window.open` opens in the opener's space with `load.opener` |
| event | `prompt` | `{id, kind, origin, topOrigin?, message?, default?, realm?, insecure?}` | `kind`: alert, confirm, text, camera, microphone, location, credentials |
| event | `download` | `{state, name, path?, message?, url?, quarantined?}` | `state`: started, finished, failed; `finished` carries the download's `url` (without user name and password; empty for `blob:` and `data:` on iOS and Linux, which Android sends unchanged) and whether the file was marked with its origin (`quarantined`). Names are sanitised (`engine::downloads`); downloads are never opened automatically |
| event | `found` | `{matched}` | The number of matches, 0 for none |
| event | `history` | `{canGoBack, canGoForward}` | The back/forward list changed without a commit: a same-document navigation (`pushState`, a fragment). iOS observes `canGoBack`/`canGoForward`, Android sends it from `doUpdateVisitedHistory` |
| event | `pressed` | none | A touch went down on the page; it still reaches the page. A split pane on a tablet takes the focus. iOS reports it from the component's view (`hitTest:withEvent:` for touches, not hover), Android from the web view's touch listener |
| event | `crashed` | none | The content process ended; the shell reloads at most once per minute |

Store ids: an ephemeral store lives as long as the shell holds its `Data` (until the private space closes, Clear
Data, a change of the profile's default level, or the end of the process). iOS keeps one non-persistent
`WKWebsiteDataStore` per id and releases it when the `Data` is dropped; Android maps `private:<session>` to the
`:private` process's own WebView directory there, and to a throwaway `ProfileStore` profile in the main process
(Lockdown), deleted at the next launch. A UUID that is not valid, or one the process may not open (Android's
private process), fails the view with `failed` on Android; iOS opens anything that is not a UUID as an empty
non-persistent store.

Platform differences left in the contract, and why:
- `zoom` scales text on iOS and Android (Android's text zoom; on iOS `-webkit-text-size-adjust` from the
  isolated world, times the boost WebKit's text autosizing gave the page's main paragraph, because
  `pageZoom` raises the page scale and autosizing then shrinks its boost) and the whole page on Linux, as a
  desktop browser does (`WebKitWebView` zoom level, like the macOS app);
- `find` on iOS: WebKit finds from the page's selection, so a new text clears the selection first and
  starts again at the first match; the same text steps from the current one, as Android's `findNext`;
- `found.matched` on iOS: WebKit says only whether anything matched; the count is the page's visible text
  searched case-insensitively in the shell's isolated world (frames are not counted, at least 1 when
  WebKit found a match); Android and Linux report the engine's count;
- `lists`: Linux compiles the groups into `UserContentFilter`s per view; iOS has no filter lists until a
  bundle is published and ignores the prop; Android applies the installed `shield::matcher` to every view
  (no bundle yet either);
- `prompt.topOrigin`: iOS sets it only for an embedded frame, Android always (the page's origin), Linux
  always empty because WebKitGTK gives no frame origin; the shells remember a permission only when
  `topOrigin` is absent or equals `origin` (`browser_core::prompts`), so Linux never remembers one and
  offers only Allow Once and Deny;
- zoom is visible to the page on every platform (computed font sizes; on iOS also the inline
  `-webkit-text-size-adjust` on `<html>`, which replaces a value the page set there itself); iOS
  measures the autosizing boost once per document (capped at 4), so a rotation keeps the first value;
- `download.quarantined`: only Linux marks files (`user.xdg.origin.url`); iOS and Android have no such
  attribute and send `false`;
- `rebuild` comes from a level change on iOS and Linux (a new view); Android switches JavaScript on the same
  view and emits it only for main-frame requests it was not told about (form POSTs); Linux also rebuilds
  when a commit needs JavaScript off;
- `crashed`: iOS reloads in the same view, Android rebuilds the view on the tab's committed page;
- all cap prompts at 10 per committed page (Linux: 8 pending, 5 dialogs in 10 s) and downloads at 3 per
  profile family in 10 s (all ephemeral stores are one family); Android additionally shows JavaScript
  dialogs one at a time and keeps at most 4 pending prompts;
- `data:` navigations: blocked by `nav_policy` everywhere, but Chromium refuses some before the hook sees
  them, and WebKitGTK cannot tell frames from the main frame, so Linux allows `data:`/`about:` frames;
- HTTPS-only: Linux checks at navigation time (WebKitGTK's `make-https` misses non-default ports); iOS
  relies on the content rule and ATS, Android upgrades in the request hook.

Permission answers are per request and never saved by the engine (no `SavesInProfile`); the shell remembers
decisions in `store` per profile, as on macOS.

### Contract v3

v3 keeps every v2 row above and adds what the macOS features of `777ee6a..413105d` need. A component that does not have v3 yet ignores the new props and commands, so shell and component
can land in either order. Every page script below is built in Rust by a shared crate; a component never writes
page scripts of its own, and Android's Java component gets them through `tarn-mobile-android` (JNI).

**Script runs.** A row that names a script runs it as an async function body in the component's isolated
world and answers with its string result: iOS `callAsyncJavaScript` in `WKContentWorld.defaultClientWorld`
(main frame); Linux `call_async_javascript_function` in the script world `tarn`; Android has no isolated
world, so it runs the script in the page world (gap below). Rules for every run:
- no script runs while the committed page's level (`levels`, else `level`) is Lockdown: the component
  answers the empty result at once;
- the answer carries `url`, the committed URL the script ran on; the shell drops an answer whose `url` is not
  the tab's current page;
- every answer is page data: the shell checks it with the Rust check named in its row before use, and shows
  it as bounded plain text only (S14);
- `seq` is the shell's number for the request, echoed unchanged so a late answer can be dropped.

Android runner: a synchronous body (`find_text_script` is not used there, see `findText`; `page_text_script`)
runs as `evaluateJavascript("(() => {" + body + "})()")` and the JSON string result is decoded. An async body
(`reader::script`, `PICKER`, and the icon scripts should a shell send them) runs as `(async () => {body})().then(r => { window[K] = String(r); })` with a
fresh random 128-bit name `K` per run, then the component polls `(() => { const r = window[K]; delete window[K];
return r; })()` every 100 ms and answers empty after 10 s (the picker waits until `pickCancel`, another `pick`
or the next commit). The page can read and forge these values; the Rust checks limit that to its own article,
its own text and a selector hiding part of itself.

| Kind | Name | Payload | Meaning |
|---|---|---|---|
| prop | `profile` | string | v2, plus `burner:<32 hex digits>`: a burner tab's store. Each burner id is an ephemeral store of its own, never shared with the private space, Lockdown or another burner, and never written to disk (S8; Android: the Lockdown deviation, see below) |
| prop | `styles` | object host→CSS | Per-site style sheets, already merged by `engine::page_scripts::site_styles(css, dark)` (the forced-dark sheet `DARK_CSS` first, then the user's CSS from `Store::site_css`, dark hosts from `Store::dark_sites`). Applied to the top frame of exactly that host at document start, as a `<style>` element's text (never markup), never under Lockdown. iOS and Linux add `page_scripts::site_styles_script(styles)` as one user script in the isolated world, main frame only; Android adds one `addDocumentStartJavaScript` per host, allowed origins `https://host` and `http://host`, holding only `site_styles_script(&[(host, css)])`, so a page never sees another host's sheet. A change applies from the next load; the shell reloads that host's open tabs, as macOS restarts them |
| prop | `zaps` | object host→array of selectors | Hidden elements per host (`Store::zapped_elements`); the shell sends `{}` for private and burner spaces. iOS compiles `shield::lists::compile_zaps` into a `WKContentRuleList` and Linux into a `UserContentFilter` (the same JSON, each rule limited to its exact host); Android appends `shield::lists::zap_css(selectors)` to that host's sheet on the `styles` path. Never under Lockdown |
| prop | `sortDownloads` | bool | Finished files go into the Downloads subfolder `engine::downloads::folder_for(name)` names (`sorted_dir`, never following a link; other types stay at the top), with the same sanitised unique names. Default false |
| command | `pick` | `{seq}` | Element picker: runs `engine::page_scripts::PICKER` on the top document; hovering (a tap on touch screens) outlines an element, a click picks it, Escape cancels. A second `pick` ends the first with an empty selector. Emits `picked` |
| command | `pickCancel` | none | Ends a running picker; it answers an empty selector |
| event | `picked` | `{seq, url, selector}` | Empty when cancelled. The shell stores it only if `shield::lists::valid_zap(selector)`, the tab still shows `url`'s host, the space is not private or burner, and the host has fewer than `shield::lists::MAX_ZAPS_PER_SITE` |
| command | `readArticle` | `{seq, probe}` | Runs `engine::reader::script(origin, probe)` (`origin` of the committed URL). The shell sends `probe: true` after each `finished` on an http(s) page to show the reader button, `false` to open the reader. Not on Android, where scripts run in the page world: there the shell probes nothing and offers the reader in the menu for every http(s) page |
| event | `article` | `{seq, url, text}` | With `probe`: `"1"` when the page has an article, else empty. Without: JSON for `engine::reader::Article::parse`, which refuses anything malformed or too large; the shell draws the article as text and stores nothing of it. Read Aloud speaks the parsed text with the platform's on-device voice |
| command | `readText` | `{seq, maxChars}` | Full-text history: runs `engine::page_scripts::page_text_script(origin, maxChars)`. The shell asks only after `finished` (never after a same-document change), with `maxChars` = `store::MAX_HISTORY_TEXT`, never for private or burner stores, and only while the setting is on |
| event | `pageText` | `{seq, url, text}` | The shell keeps `store::clip_history_text(text)` with `Store::set_history_text` for its visit of `url` |
| command | `readIconLinks` | `{seq}` | Site icons: runs `engine::page_scripts::icon_links_script(origin)`. The shell asks after `finished` only for `engine::favicon::icon_host(url, lockdown)` (https, default port, never Lockdown) when `favicon::Tracker::request` says so (no fresh icon stored, under `favicon::MAX_TRIES` failures this session, none in flight). Not on Android, where scripts run in the page world (see the gap below) |
| event | `iconLinks` | `{seq, url, text}` | `href<TAB>sizes` lines, an empty line, then the hosts the page loaded from. The shell keeps only `engine::favicon::candidates(url, text)`, while the tab still shows `url`'s origin, and sends them in `fetchIcon`; none withdraws the request |
| command | `fetchIcon` | `{seq, page, urls}` | Runs `engine::page_scripts::fetch_icon_script(page, urls, favicon::MAX_BYTES)` on at most `favicon::MAX_CANDIDATES` https `urls`, so it fetches nothing once the tab shows another origin than the `page` that asked |
| event | `iconData` | `{seq, url, text}` | The icon as base64, or empty. The shell keeps only `engine::favicon::normalize(text)` (a 64×64 PNG), stores it with `Store::set_favicon` when `Core::keeps_records` (never for burner profiles; a private profile's only in its in-memory store) and replaces a stale one (older than `favicon::MAX_AGE`) only on success; anything else counts as a failure. Clearing history or closing a private space forgets them |
| command | `findText` | `{seq, query}` | Find in Tabs: runs `engine::page_scripts::find_text_script(query)` (the query is a JS string literal, at most 200 characters) and cuts the answer with `page_scripts::find_snippets`. Android never sends the query to a page world: it reads the page's text with `page_text_script` and matches it in Rust (see the v3 differences) |
| event | `textFound` | `{seq, url, snippets, matched?}` | At most `page_scripts::FIND_MATCHES` snippets, each at most 320 characters; `matched` (the page's total) only on Android. The shell lists title and address matches of every tab with `browser_core::find::named` at once and puts text rows above them as they arrive with `browser_core::find::insert_text_rows`; snippets are never stored |
| event | `shielded` | `{kind, url}` | For the per-profile Shield counters (`Store::count`, shown by `Store::shield_week`): `https_upgrade` (`Counter::HttpsUpgrades`), `local_network` (`LocalBlocked`), `listed` (`ListedStopped`: a page stopped because a domain set names its site) and `popup` (`PopupsBlocked`: a pop-up refused for lacking a gesture; the shell skips it when `shield::rules::local_target(url)`, as macOS does). The shell counts each kind at most once per committed page |
| event | `download` | v2 payload plus `id`, and the states below | `id` (string, unique for the app's run) is on every `download` event. `started` adds `pausable` (bool). New states: `progress` `{id, done, total?}` at most twice a second while saving; `paused` `{id, done}`; `resumed` `{id}`; `flagged` `{id, name, host}`: the request, response or page host is on the switched-on malware set (`engine::downloads::flagged_source` over `[request, response, page]` with the component's `nav_policy::Site`), even for a host the user let through for browsing; the file waits, unsaved, until `downloadKeep`. A paused download never kept is checked again (request and page) before it resumes. A declined download ends without a `failed` event |
| command | `downloadPause`, `downloadResume`, `downloadCancel` | `{id}` | Only for a `pausable` download; iOS keeps WebKit's resume data in memory, Android keeps the `.part` file it was writing (resumed with `Range` and `If-Range`). Cancel removes the part already saved. Downloads belong to the app, not to a view (as on macOS): closing or unmounting the tab that started one keeps it running or paused, and its later events reach the shell through any live view (held until one exists); the shell matches them by `id`, sends commands through any live view and records the finished or failed file in the profile it started in |
| command | `downloadKeep` | `{id, keep}` | Answers `flagged`; anything but `keep: true` cancels. An unanswered one waits, like a paused one, after its component is destroyed |

What the shells build from shared code without the component: copy a clean link
(`shield::url_clean::clean_link`, counted as `Counter::TrackingParams`), the QR code (`engine::qr::encode`,
rows of dark modules with a one-module border; the macOS app keeps CoreImage), space schedules
(`browser_core::schedule::due` each minute with `Store::local_time`, after a manual switch held for
`schedule::MANUAL_HOLD`), burner tabs (`browser_core::tabs::Model::open_burner` / `burn`), close duplicates
(`tabs::duplicates`), routing new tabs by space rules (`browser_core::routing`), space snapshots
(`browser_core::snapshot`), the Shield week (`Store::shield_week`), favicon refresh
(`Store::stale_favicons`, icons stored over 30 days ago), cookie-banner lists (`shield::groups`), the Reading List folder and the share action (Linux copies the
link; iOS and Android need a native share sheet from their host).

v3 platform differences:
- Android runs the picker, reader and page-text scripts in the page world (no isolated world, no bridge);
  `findText` reads the page's text there with `page_text_script` (at most 1,000,000 characters) and matches
  the query in Rust (`tarn-mobile-android`'s `find_snippets`, case-insensitive, 60 characters of context), so
  the query never reaches the page; it sends snippets and `matched`, the page's total. A view reads a page's
  text once and keeps it for later queries until the page changes or a minute passes without a query, and
  matches off the UI thread. `findAllAsync` was
  not used because it finds nothing in a hidden view (the tab sheet hides every view). `styles` and `zaps`
  are CSS on every platform there, so a zapped element can flash before the sheet applies;
  `addDocumentStartJavaScript` origin rules name a port or the default one, so a host's sheet reaches only
  its default ports (`https://host`, `http://host`), not `host:8080`;
- `pausable`: iOS (`WKDownload` resume data) yes; Linux no (WebKitGTK has no pause, only cancel); Android only
  for downloads the component makes itself, `false` for anything handed to the system's download manager;
- `flagged` needs the malware domain set, which none of the three has until a filter bundle is published;
  until then no download is flagged. iOS holds a flagged download's destination until `downloadKeep`; Android
  does not emit `flagged` (its transfers check Shield before each request instead);
- `shielded`: iOS and Android report `https_upgrade` (a main-frame load upgraded at Strict) and
  `local_network` (a page refused by the private-network rule); both report `popup` with the opener's URL
  for a pop-up over the burst limit (`view_policy::POPUP_BURST`), and Android also for one without a
  gesture; iOS cannot see those, because WebKit refuses them before any delegate is asked; neither reports
  `listed`: the components apply no domain set (`Site::new` names none) until a filter bundle with one is
  published, so no page is stopped for being listed;
- Android's `pausable` transfers resume with an HTTP `Range` request and start over when the server answers
  200; a file paused after its last byte finishes instead, and a resume answered 416 with the full length is
  finished too. A paused transfer keeps its `.part` file in the Downloads folder until it resumes or is
  cancelled; the main process deletes leftover `download*.part` files at launch (the private space's go
  with its session). iOS resumes a paused download through the view that started it or, once that view is
  gone, an off-screen `WKWebView` of the same store with JavaScript off;
- `burner:` is the shell's mapping of a burner profile's `private:<id>` store (same id, so a platform
  releases both forms when the `Data` drops); `burner:` on iOS is its own `nonPersistentDataStore` per id (v2 shared one per session for every non-UUID
  id; `private:` keeps that); on Linux its own ephemeral `NetworkSession` per id; on Android a throwaway
  `ProfileStore` profile in the main process, deleted when released and swept at the next launch, like a
  Lockdown store (the documented S8 deviation). Android tries to delete a released main-process ephemeral
  profile (burner or Lockdown) once no view uses it, but WebView keeps a profile it has rendered "in use"
  for the rest of the process, so in practice it goes at the next launch's sweep of `tarn-burner-*` and
  `tarn-private-*` (checked: `deleted=true` at relaunch); it offers no burner tab inside the `:private`
  process, whose own store is already throwaway;
- Linux runs the scripts in the world `tarn` and adds `styles` as a `UserScript` in that world.

### Tarn extensions on Android

`tarn_mobile_android::tarn_ext` implements the engine hooks (`set_gate`, `set_scripts`, `set_rule_lists`,
`serve`, `worlds_supported`, and on Android `frame_event`), `tarn.webview` the `extensionPost` command and the
`extensionMessage` and `extensionFrame` events, and the `tarn.extpage` component the extension pages. All
decisions come from `browser_core::tarn_ext`.

- **Isolated worlds.** androidx.webkit 1.18.0 (pinned in `host/build.gradle.kts`) has named execution worlds
  behind `WebViewFeature.JS_INJECTION_IN_FRAME_AND_WORLD`: `WebViewCompat.getExecutionWorld`,
  `addJavaScriptOnEvent(..., world)` and `addWebMessageListener(..., world, ...)`. Each install's world
  `tarn-ext-<install id>` gets a document-start loader and a message port that exist only in that world, in
  every http and https frame. The loader takes the port off the world's globals and reports the document's
  URL. NSH runs the shell on its own engine thread, not on the Android main thread of the WebView callbacks,
  so the component asks with the `extensionFrame` event (`{request, tab, store, world, url, top, main,
  origin}`, `origin` as WebView reports it for the frame); the shell passes it to `tarn_ext::frame_event`,
  which refuses a URL outside that origin, asks the gate (`frame_decision`) and hands the scripts back by
  JNI. The component runs only those, in that frame and world, through the frame's `JavaScriptReplyProxy`
  (an answer for a document the frame has since left is dropped): the port setup and bootstrap, each
  script's CSS as a `<style>` element at once, and its JS as classic scripts at `run_at` (`document_end` on
  `DOMContentLoaded`, `document_idle` a task later). The sender of `extensionMessage` is the world whose listener received it and the frame its
  reply proxy belongs to (main frame 0, others numbered per view); messages of a frame the gate gave
  nothing are dropped, and the page's main world has no port at all.
- **Difference: when the gate is asked.** WebView has no synchronous per-frame hook before commit, so the
  gate is asked when the frame's loader reports at document start (a round trip through the shell's
  thread). Scripts and CSS for `document_start` therefore run shortly after the document starts parsing; a
  page script that runs first can see the page before the extension's CSS applies.
- **Gap: WebView without isolated worlds.** An Android System WebView that lacks
  `JS_INJECTION_IN_FRAME_AND_WORLD` (or `WEB_MESSAGE_LISTENER`) runs no content scripts and no content-script
  CSS; `worlds_supported()` is false and the shell says "Content scripts are not supported by this Android
  System WebView". Content-script JS and its bridge cannot run in the page's main world, where the page
  could read and drive them (spec, "Bridge and trust"). CSS alone is not injected there either: a
  main-world document-start script can be limited only by origin rules, not by the gate's match patterns,
  `exclude_matches` and grants per frame, so it would style frames the core did not allow. Network rules,
  the action entry, popup, background page and storage still work there.
- **Network rules.** `set_rule_lists` parses each list with `engine::content_rules`; the component applies
  them per request in `shouldInterceptRequest` and in the main-frame checks of `shellLoad`, link navigations
  and history steps, never at Lockdown (the view's or the site's level). As with Shield on Android, an
  `upgradeScheme` rule upgrades a main-frame load and blocks a subresource (WebView cannot rewrite a
  subresource request). WebSocket and service-worker requests are not seen (the WebView gap above). While a
  navigation is pending WebView cannot say whether a subresource belongs to the page being left or the page
  coming, so an extension rule acts on it only if it acts for both: an extension never acts on a site it holds
  no grant for, and the first requests of a granted page reached from another site may go unfiltered.
- **Extension pages** (`tarn.extpage`): a view of the profile's WebView profile that loads only
  `tarn-extension://<origin id>/` URLs, answered in `shouldInterceptRequest` from the store's `serve()`
  handler with its status and MIME type; a `tarn-extension://` request for another origin is refused with
  403. Top-level navigation elsewhere and new windows are cancelled; JavaScript dialogs, permissions,
  geolocation and file choosers are refused; http and https subresources get the profile level's Shield and
  private-network rules (no top page, as for service workers). It does not open at Lockdown or for a
  non-persistent store.
- **Difference: custom scheme.** WebView parses `tarn-extension://` as a scheme without a host and gives its
  documents an opaque origin, which neither `'self'` nor a host source matches. So (1) the
  `Content-Security-Policy` is the core's with `'self'` replaced by the scheme source `tarn-extension:`;
  with the 403 above the page still loads scripts only from its own origin, and inline scripts and `eval`
  stay refused; (2) WebView takes no origin rule for the scheme, so the document-start script and the port are
  registered for every origin: the script runs the bootstrap only in a top document whose URL is
  `tarn-extension://<origin id>/`, and the component takes messages only from the main frame while it
  shows that origin. An http frame inside an extension page gets the port object but nothing it posts is
  read, and nothing is posted to it.

Checked on the `slideout_3gb` emulator (Android 16, Android System WebView 157.0.8103.38) with a fresh app
data directory, an Unverified test extension installed from Downloads, and local test servers on
`127.0.0.1` and `localhost` (HTTP, and HTTPS with a throwaway CA installed as a user certificate; the debug
build trusts user certificates for the debug test hosts only): consent sheet, the per-site prompt (Allow Once,
Always Allow, Deny), content script and CSS in the install's world while the page's main world sees no
`chrome`, `browser` or `__tarnExt*` globals, a message round trip with the background page, storage, the popup
with its badge, a network rule blocking a subresource only after the grant, "Forget Site Answers" stopping an
open page's messages and asking again, a cross-site `all_frames` frame that runs and messages only once its
own origin is allowed, a frame of a Lockdown host inside an allowed page running nothing, a web page unable to
fetch, embed or frame the extension's files (a top-level link is offered to another app and never loads),
the popup's frames limited to its own origin, nothing in the private space or a burner tab, and nothing at
Lockdown for the site or the profile (no scripts, no rules, no popup).

### Tarn extensions: the shared shell and iOS

The engine hooks every mobile engine provides are `tarn_ext::{set_gate, set_scripts, set_rule_lists, serve,
worlds_supported, frame_event}` in its crate, the `extensionPost` command and `extensionMessage` event of
`tarn.webview`, and the `tarn.extpage` component (`mobile/app` reaches the crate as `platform::ext`). All
decisions come from `browser_core::tarn_ext`.

- **Store install link** (`docs/shared-core.md` §1). iOS cancels it in the navigation delegate
  (`ios/src/component.rs`, `ios/src/store_link.rs`: a shell load or a main-frame `linkActivated`
  navigation). iOS uses no private WebKit API and has no public user-gesture or redirect flag, so a page's
  own script-started link click, or a redirect after a link click, can raise the confirmation built from the
  held catalog entry; the shell takes at most one store link per 10 s (`Core::admit_store_link`, on every
  client), and nothing is requested or installed until the user presses Install on it. Android in `shouldOverrideUrlLoading` (main frame, `hasGesture`, not `isRedirect`) and
  in its shell load (`Native.storeInstall`); both emit `storeInstall`, and the shared shell checks
  `Shell::store_link_allowed` and opens Settings › Extensions on the confirmation, or shows the notice on
  the tab (`tarn_nsh_shared::extensions::store_link`). A routed pop-up to the install address keeps its
  opener, so it never asks. Not yet run in the simulator or the emulator.
- **Shared shell** (`app/src/extensions.rs`, `app/src/ui/extensions.rs`). For the persistent profile on
  screen it loads each enabled install with `tarn_ext::load`, so the tree is hashed against the stored hash
  before anything runs (a mismatch turns the install off and drops its site answers, as on desktop); a
  popup or background page opens from that checked load, as on desktop and Linux. It hands the engine the worlds, the content rule
  lists (`rule_list` with the current `Session::scope`, sent again when a grant starts or a Once grant ends)
  and the profile's `Resolver`. The gate is `frame_decision`; a host it wants answered raises the per-site
  prompt over the tab (Allow Once for an hour or until the tab closes, Always Allow, Deny, Never), and an
  allow reloads the tab. Messages go through `router::Router::handle`; its `Out`s become `extensionPost`
  commands, page posts, background start/stop, new tabs. A 5 s timer runs `Router::tick` (an idle background
  page stops after 30 s). Closing a tab ends its Once grants and pending replies; a main-frame commit ends an
  `activeTab` grant. The menu lists each extension with an action, its title and badge; choosing it opens the
  popup in a sheet or sends `action.onClicked`. Settings › Extensions is `tarn_nsh_shared::extensions`
  (shared with Linux): install from a file (the system document picker), a GitHub repository or the official
  catalog (shown only in builds with a catalog key), the consent sheet from `manifest.consent()` with the
  source's trust label, on/off per profile, "every site it lists" for App and Official installs, forget site
  answers, check for update, remove. Only Tarn extensions install here; a classic package is refused by name.
  Install folders are found from the extensions folder, not the stored absolute path, because iOS moves the
  app's data container on every update. "Forget Site Answers" drops the stored answers and this session's
  (Allow Once and Deny), so it asks again on every site at once. Nothing runs in a private space, a burner tab
  or at Lockdown; at profile Lockdown the action entry stays in the menu but opens no page, as on desktop.
- **iOS engine** (`ios/src/tarn_ext.rs`, `ext_view.rs`, `ext_scripts.rs`, `extpage.rs`). NSH runs the shell on
  its own thread, so the hooks hand owned copies to the main queue. Each install gets
  `WKContentWorld worldWithName:` (`tarn-ext-<install id>`, never the page world or Tarn's own client world),
  `WKUserScript`s in that world and a `WKScriptMessageHandler` added with
  `addScriptMessageHandler:contentWorld:name:` for that world only. The gate: when the navigation policy
  allows a frame navigation (main frame and subframes) in a view whose profile has extensions, the decision
  is held, the view emits `extensionFrame`, `frame_event` asks the gate on the shell thread and the answer
  records exactly the returned scripts for that frame in the shared `tarn_ext::Decisions` before WebKit
  continues. The world's scripts are `tarn_ext::injections` with iOS's own setup (`ext_scripts::SETUP`): the
  gate script looks its document up in the decisions, defines `__tarnExtPost` and runs the bootstrap and the
  CSS, and every JS file of the world checks the gate's answer first, so nothing runs in a frame the gate did
  not approve. Subframes are numbered by a per-document token and checked against the origin WebKit reports
  for the frame, which also goes with each subframe message for the router's grant check. A reply to a subframe runs only
  where the document's token matches (`engine::tarn_ext::reply`), so it never runs in the frame's next document. Messages over 1 MiB are dropped. Network rules are compiled into
  `WKContentRuleList`s and added to views of the profile not at Lockdown. `tarn.extpage` serves
  `tarn-extension://` through a `WKURLSchemeHandler` from the profile's resolver (status, MIME type, the
  page CSP) for its own origin only, only navigates its main frame inside its own origin, gets the profile level's built-in rules,
  runs the bootstrap in its main frame's page world on its own origin only and hears only that frame; it
  does not open at Lockdown or for a non-persistent store.
- **Differences on iOS.** `WKUserScript` has two injection times, so `document_idle` runs at document end.
  A frame a redirect ends on that WebKit does not pass through the navigation policy is not approved, so the
  extension does not run there. A frame the gate did not approve logs "Tarn: not granted here." in Web
  Inspector (the guard stops the script by throwing). Content-script CSS is a `<style>` element the
  page can remove.

Checked on the iPhone 18 Pro simulator (iOS 27.0) with a fresh install and a test extension (content
script and CSS on `example.com`, storage, background page, popup): consent sheet, install from Files, the
per-site prompt, the content script running after Allow Once with storage and a background reply, the
badge in the menu, the popup reading storage, a new prompt after relaunch for a Once answer, and nothing
in the private space for a site allowed with Always Allow in the profile. Evidence:
`mobile/ios/evidence/extensions/`. Unit tests: `app/src/extensions/tests.rs` (grants, prompt, storage,
messaging, background start and idle stop, popup and badge, private space, changed files),
`crates/browser-core/src/extensions/install.rs` (install, consent, classic refusal, a tree changed before consent) and
`ios/src/ext_scripts.rs` (guards).

Checked again on the same simulator with a fresh install and the local test servers used for Android (HTTPS
with a throwaway CA added to the simulator's keychain only): everything listed for Android above, including
the network rule acting only after the grant, cross-site frames, and Lockdown for the site and the profile.
An open page keeps its message handlers when the shell sends the worlds again (any change in Settings ›
Extensions); only an install that goes away loses its handler.

## 3. Data stores (`browser_core::WebsiteData`)

| | iOS | Android |
|---|---|---|
| persistent | `WKWebsiteDataStore(forIdentifier:)` per profile UUID (iOS 17+) | `androidx.webkit` `ProfileStore` profile per UUID |
| ephemeral | `WKWebsiteDataStore.nonPersistent()` | Private space in a separate `:private` process with `ProcessGlobalConfig.setDirectoryBasePaths`, a fresh folder per session, deleted on close and swept at every launch |
| remove all | `removeData(ofTypes: allWebsiteDataTypes, modifiedSince: .distantPast)` | `ProfileStore.deleteProfile` once no view uses it, otherwise at next launch |

The ephemeral store must never write website data to disk (S8), except on Android, where the documented
deviation applies. Each implementation has a test for its guarantee.

Cookie import on Android runs `engine::cookies::admit_imported` against an empty jar: `CookieManager`
returns only `name=value` pairs, without domain, path or flags, so an imported cookie is not checked
against one the WebView already holds (Tarn's own sign-in may be shadowed) and the per-site caps count
only the imported cookies.

## 4. Phases and exit checks

**M1 shared app (tarn-mobile):**
- tab switcher (grid);
- address bar with suggestions from history;
- bottom toolbar (back, forward, new tab, tabs, menu), share inside the address capsule;
- spaces and profiles;
- private space;
- per-site Shield level and badge;
- settings: default level, search engine, filter lists, clear data;
- history;
- downloads list;
- reopen closed tab.

State lives in `browser-core`; host tests run with an in-memory `WebsiteData` and NSH's testing crate.

*Status M1:* built in `mobile/app` (`tarn-mobile`). `Shell<D>` (`shell.rs`, `events.rs`) holds
`browser_core::Core` and `tabs::Model` and adds only what the contract needs: per-tab `load {url, seq}`, page
state from events, the prompt queue, crash reloads (one per tab until a page finishes loading, as on macOS) and the data-store id per
profile (`<uuid>` persistent, `private:<session>` and `lockdown:<id>:<session>` ephemeral; the private session
is renewed when the private space closes). Address-bar parsing, search engines and fuzzy matching moved from
`ui::command` into `browser_core::command` (pure move, `ui` re-exports it). The UI (`ui/`) is NSH Rust: address
bar with history suggestions, bottom toolbar (Back, Forward, New Tab, Tabs, Menu; on screens 700 pt and wider the
same buttons sit in the top bar; Share sits at the right end of the address capsule), tab grid, menu, per-site Shield panel and badge, settings (default level,
search engine, filter-list switches, clear data), history, downloads, spaces and profiles, private space, reopen
closed tab, find bar and zoom. Share copies the link (NSH has no share sheet). Filter lists: the switches are
stored with the desktop keys, but no bundle is published yet (`FILTER_BUNDLE_URL` is empty), so only Shield's
built-in rules apply on mobile. Tests: `cargo test` in `mobile/` runs 14 shell tests on in-memory website data
(`data::Memory`) and one headless NSH UI test (`tests/ui.rs`).

**macOS features on mobile:** the shell (`app/src/features.rs`,
`app/src/scripts.rs`, UI in `app/src/ui/sheets.rs`) implements the macOS features on contract
v3, with host tests in `app/tests/features.rs` and `site_ui.rs`:
- tabs: Close Duplicates (one reopen-stack entry, "N tabs closed" with Undo), toasts with Undo for closed
  tabs, Move to Space from a tab card, burner tab (Menu and the tab sheet; flame on the Shield button and
  tab cards; closing its tab or leaving its space burns it and returns to the space it came from; records
  no history or downloads), routing pop-ups of a ruled host into its space, space snapshots (Spaces sheet:
  save, restore in a new space, delete), schedules (pencil on a space: weekdays and From/To hours; checked
  at launch and each minute while browsing, held for an hour after a switch by hand, never from the
  private space);
- the site sheet (Shield and Settings for This Site): dark mode, Site CSS (64 KiB, line breaks become
  spaces), hidden elements with Zap Element and Show All, "Open new tabs of this site in";
- Share opens the system share sheet (`UIActivityViewController`; Android `ACTION_SEND` chooser) and falls
  back to copying; Show QR Code draws `engine::qr::encode` on an NSH canvas; Copy Clean Link counts removed
  parameters on the Shield card;
- Reader: probe after each http(s) load, a book button in the address capsule, the article as plain text
  with A−/A+, Read Aloud (`AVSpeechSynthesizer`; Android `TextToSpeech`, split at its input limit), Save
  for Later; Reading List sheet;
- Find in Tabs sheet (titles and addresses at once, text rows above them as tabs answer);
- Settings: one sheet grouped under the desktop's pane names (Profile, General, Search, Privacy;
  `tarn_nsh_shared::panes`); the Shield card is the Mac's glacier lake (`browser_core::shield_card`, the
  landscape as SVG and its words as native text, scaled to the sheet), Flip and New Design, "Last 7 days"
  per counter in faint tracks and the sites protected most (no tilt, NSH has no 3D transform); Cookie banners filter switch (`block_annoyances`), "Search the text of visited
  pages" (`history_text`, History gets a search field), "Sort downloads into folders" (`sort_downloads`);
- Downloads sheet: running transfers with progress, Pause/Resume/Cancel, Keep/Delete for flagged ones.
Not on mobile: shortcut rebinding, memory panel, live peek, translation, startup trace. Android runs no
icon script and shows letter tiles for sites: Android WebView offers no isolated world for Tarn's icon
scripts (a page could notice a Tarn user, `docs/threat-model.md`, page scripts on Android), and its own
icon download cannot be limited to the hosts S14 allows, so the web view's icon download is switched
off wherever WebView supports the setting (`WebSettingsCompat.setDownloadFaviconsEnabled`) and the shell
takes no icon from it. Android implements `readIconLinks` and `fetchIcon` but the shell never sends
them. The background tab memory budget is a gap: neither iOS nor
Android lets an app read the memory of the web content or renderer process, and both reclaim it
themselves under pressure.

Device checks, evidence in `mobile/evidence/features/` (Android: `Pixel_9_Pro`, Android 16, the test pages
of `serve.py` through the debug proxy so `zap.test` counts as public; iOS: iPhone 18 Pro simulator, iOS 27.0, the same pages on `localhost:8790`,
screenshots in `ios/`).

| Check | Android | iOS |
|---|---|---|
| Reader probe, reader, Read Aloud (TTS engine loaded and spoke), Save for Later, Reading List | Pass `a01`–`a04` | Pass `i01`–`i03`, `i06` (AVSpeechSynthesizer started after fix 1) |
| QR code, share sheet | Pass `a05`, `a06` | Pass `i07`, `i04` (after fix 1); Copy Clean Link dropped `utm_source` (`i08`) |
| Zap Element: tap picks `#banner`, stored, sheet hides it after the reload | Pass `a07`, `a08` | Pass `i12`, `i13` after fix 2 |
| Dark mode and Site CSS for one host | Pass `a10-*`, `a09` | Pass `i09*`–`i11` |
| Find in Tabs: title row, then three text snippets | Pass `a11` | Pass `i14` |
| Burner tab: own `tarn-burner-<id>` profile, Strict, flame; closing it returns to Home | Pass `a12`, `a13`; the profile stays "in use" by WebView until the process ends and is deleted at the next launch (logcat `deleted=true`) | Pass `i26`, `i27`; the store is released at once after fix 3 (`~WebsiteDataStore`, `destroySession` in `burner-store-after-fix.log`) |
| Downloads: progress, pause (2.0 MB), resume, cancel removes the part | Pass `a14`–`a16` | Pass `i32`–`i34` (paused at 1.4 MB, the finished file matches the source byte for byte) |
| Shield card front and back, settings switches | Pass `a17`–`a19` | Pass `i28`–`i30` after fix 5; full-text history search `i31` |
| Spaces: snapshot saved, space added, schedule editor | Pass `a21`–`a23` | Pass `i18`–`i21`, `i23` after fix 4 (`i22` shows the extra tab before it) |
| Close Duplicates with Undo, Move to Space | Pass `a24`–`a26` | Pass `i15`–`i17`, `i24`, `i25` |

The iOS run found five bugs, since fixed: (1) Share and Read Aloud ran on NSH's engine thread,
so UIKit and AVSpeechSynthesizer refused and Share always copied (`platform.rs` now goes through
`run_on_main_thread`); (2) iOS WebKit makes no click from a tap unless an element on the path listens,
so the picker (listening on `window`) never picked (the iOS component adds a listener on the root
element while it runs); (3) burner and private stores were released from the engine thread's map, so
they stayed alive until the app quit; (4) a restored snapshot got an extra blank tab; (5) every sheet
kept the previous sheet's scroll offset, so Settings opened from a scrolled Menu hid the Shield card.

Known: a word still being composed by the Android keyboard reaches NSH's `on_change` only once committed
(a space or another key), so a button pressed right after typing a single word can see the old value
(seen with the automation keyboard; NSH input, not changed here).

**M2 iOS:** the component and `Data` for iOS, an `nsh package --target ios-simulator` build, phone and iPad
layouts.
- Exit: in the simulator, browsing, tabs, profiles, the private space, Shield (private-network block,
  HTTPS-only, filter lists when present), prompts, downloads into Files, rotation and VoiceOver reach the
  address field.

*Status M2:* `mobile/ios` (`tarn-mobile-ios`) implements contract v2: every prop, command and event
in §2. Policy is decided in the delegates before the handler returns; props are batched and applied on the
main queue; a change of `profile`, `level` or `levels` rebuilds the view only when the current page's level
changes and then loads the current URL, never the original `load`; a new `seq` loads again; no view loads
before Shield's rule list compiled (a compile error fails closed). Cancelled loads (`NSURLErrorDomain` -999,
`WebKitErrorDomain` 102) are not `failed`. Prompts carry ids; `answer` settles them and the component's `Drop`
declines the rest; ids are unique for the app's run. A page gets at most 10 dialogs (JavaScript dialogs,
sign-in, camera and microphone) per committed page, later ones are declined without asking; sign-in stops
after three failures; the shell shows and answers only the active tab's prompts, queues at most 8 per tab,
never replaces a pending "open in another app" choice and names the asking site; permissions of embedded
frames (`topOrigin` set) are never remembered. At most ten downloads per tab in ten seconds (S11, `view_policy::DOWNLOAD_BURST`).
Tabs hidden by a space switch or evicted from the live set drop their view, load and prompts and come back
at the page they showed. Private stores live until the next private session, Lockdown stores for the
launch; Clear Data gives a private or Lockdown profile a new store (a new ephemeral id releases the older one
of the same profile). A pending "open in another app" choice goes away when another tab is shown. Camera plus microphone asks twice and grants only if both are allowed. Downloads go to
`Documents/Downloads` with `engine::downloads` names (`UIFileSharingEnabled`, `LSSupportsOpeningDocumentsInPlace`).
Popups: WebKit refuses ones without a gesture; allowed ones are checked for the private network against the
opener and emitted as `popup`. `Data` maps a UUID to `dataStoreForIdentifier:` and anything else to one shared
`nonPersistentDataStore` per session (`ios/src/data.rs` has the unit test of that rule; the disk check is below);
`wipe_persistent` removes all data types. `nsh.json` raises `MinimumOSVersion` to 17.0 through `ios.infoPlist`,
which `nsh package` accepts. `.app` (debug runner): 52 MB.

Exit checks, iPhone 17 simulator (iOS 27.0) and iPad Pro 13-inch (iOS 26.2), local server
`mobile/ios/testserver/server.py` (HTTP on 127.0.0.1:8765, HTTPS on 127.0.0.1:8444 and on the Mac's global
IPv6 address as the "public" origin, with a test CA added to the simulators' keychains). Evidence in
`mobile/ios/evidence/`, request log `server.log`. These checks ran on the build before the design pass (§5) and
the review fixes above; the restyled build was checked for browsing, tabs, menus, settings, Shield, a prompt,
the private space, rotation and iPad (`mobile/ios/evidence/design/`); the full table was rerun on the shared
integration build (see the integration status below). The store ids named above were replaced by the scheme in §2.

| Check | Result | Evidence |
|---|---|---|
| Browsing | Pass | `02-local.png`, `03-public.png` |
| Tabs (grid, new, close, `target=_blank`, gesture `window.open`, switching without reload) | Pass | `12-tab-grid.png`, `11-blank-link-tab.png`, `11-window-open-tab.png`, `server.log` |
| Profiles (cookie `tarn=A` in Personal, none in a new profile, back in Personal) | Pass | `16-*.png`, `18-website-data-stores.txt` (two stores on disk) |
| Private space (cookie set, space closed: no new files in the container, value found nowhere) | Pass | `18-private.png`, `18-data-*.txt` (empty diff) |
| Shield: private network from a public HTTPS page (two images, link, script redirect, `_blank` popup) | Pass, no request reached the server | `03-public.png`, `04-*.png`, `server.log` |
| Shield: HTTPS-only (`http://example.com` link) | Pass at Strict (upgraded to `https://example.com/`); at Standard ATS refuses the load | `07-http-strict.png`, `06-http-standard.png` |
| Shield: per-site Lockdown (scripts off; the current page reloads, not the original load) | Pass | `08-*.png`, `server.log` 12:41:39 |
| Shield: filter lists | Not tested: no filter bundle is published | |
| Navigation policy (`data:` blocked, `mailto:` asks first, no-gesture popup blocked) | Pass | `05-mailto-asks.png`, `server.log` (no `auto-popup`) |
| Prompts: alert, confirm, text, flood of 30 alerts, HTTP sign-in, camera | Pass | `09-*.png`, `10-flood-done.png`, `13-*.png`, `15-*.png` |
| Downloads into Files (name `../tarn report.txt` saved as `_tarn report.txt`, listed in Files › On My iPhone › Tarn › Downloads) | Pass | `14-*.png`, `14-documents-listing.txt`, `22-files-app.png` |
| Crash reload (at most once a minute) | Pass | `19-crash.txt`, `19-crash-*.png` |
| Rotation and iPad layout | Pass (phone landscape and iPad use the wide top bar) | `20-landscape.png`, `23-ipad-*.png` |
| VoiceOver reaches the address field | Partly: the accessibility tree has the address `TextField` inside a group labelled "Address" (NSH puts the label on the container); VoiceOver itself was not run | `21-ax-tree.txt` |
| Find, zoom, location prompts | Not tested on the simulator; location: the app has no location usage description, so WebKit denies geolocation without asking | |

Known gaps: NSH has no share sheet (Share copies the link); the inputs' accessibility labels land on their
container (buttons take theirs: the toolbar icons read "Back", "Tabs 5", "Shield: Standard"); a download whose view is rebuilt mid-way loses its delegate, so its `finished` event may not arrive (not tested); `found.matched` is 1 or 0
(WebKit on iOS does not count matches).

**M3 Android:** the host app, the component and `Data` for Android, the `:private` process, and a debug APK.
- Exit: the same checks on the emulator, plus the private-space checks from the follow-up (the folder is
  gone after close, swept after force-stop, and the private activity is excluded from Recents with
  `FLAG_SECURE` or `setRecentsScreenshotEnabled(false)` and `IME_FLAG_NO_PERSONALIZED_LEARNING`).

**M3 status:** built in `mobile/android/` (layout and
`tarn-mobile` integration steps in its `README.md`). The component was driven by a small NSH test shell,
not by `tarn-mobile`, which did not exist yet. Run on `Medium_Phone_API_36.1` (Android 16, WebView 157)
and `Tablet_10in` (Android 16, WebView 157 after Play updated 133 during the run) with `server.py` as
logging server and proxy; evidence in `mobile/android/evidence/{phone,tablet}/`.

| Check | Result | Evidence (phone unless noted) |
|---|---|---|
| Browsing, tabs, back/forward, find, zoom, late `title` | Pass | `17-*`, `07a-*` (popup opened in tab 2), `16-site-levels.log` |
| Profiles A, B and ephemeral | Pass: cookies stay in their store | `13-profiles.log`, `server.log` |
| Private network | Pass: local pixels blocked from `http://public.test/`, loaded from a local top | `04a`, `04b`, `04c`; tablet `02-*`, `logcat.txt` |
| HTTPS-only | Pass: typed `http://` upgraded at Strict (only `CONNECT :443` reached the server); a page link into a Strict site blocked | `05a`, `05b`; tablet `05a` |
| Navigation policy | Pass: `mailto:`/`intent:` → `external`, page and redirect into localhost → `blocked`; `data:` refused by Chromium; `javascript:` runs at Standard (known, see nsh-plan §9) | `06-*`; tablet `02-*` |
| Filter lists (`shield::matcher`) | Pass with a test list (`\|\|ads.test^`) | `12-filter-lists.log`; tablet `logcat.txt` |
| JavaScript per level | Pass: Lockdown "No", Standard "Yes" on a real HTTPS page; per-site `levels` apply to their host only | `15a`, `15b`, `16-*`; tablet `15a`, `15b` |
| Popups | Pass: `target=_blank` and gesture `window.open` → `popup`; no-gesture `window.open` never reached the server; a popup that redirects into localhost is blocked (the shell passes `load.opener`) | `07a`, `07b`; tablet `logcat-fixes.txt` |
| Prompts | Pass: confirm, text, credentials (HTTP sign-in), location and camera; a location grant is asked again after reload | `08-*`, `09-*`, `10-*`, `10b-*` |
| TLS error | Pass: cancelled, `failed` | `14-*`; tablet `logcat.txt` |
| Downloads | Pass: name sanitised (`../../evil:name.txt` → `evil_name.bin`), saved in app storage | `11-*` |
| Content-process crash | Pass: `crashed` on every tab of the renderer, reload rebuilds the view on the tab's committed page | `18-*`; tablet `logcat.txt` |
| Rotation | Pass on phone and tablet, same process | `26-*`; tablet `26-*` |
| Accessibility | Partial: address field and web content reachable; NSH buttons unlabeled (nsh-plan §9); TalkBack not run | `26-rotation.txt`, tablet `26-a11y-landscape.txt` |
| Private: separate process, data only in `private/<session>/` | Pass: cookie, localStorage, IndexedDB and Cache Storage markers only there | `20a`, `20b`; tablet `20b` |
| Private: folder gone after close | Pass: removed about 0.1 s after the process died | `20c-*`; tablet `20c` |
| Private: swept after force-stop | Pass: left on disk until the next launch, then deleted | `23a`, `23b`; tablet `23a`, `23b` |
| Private: Recents | Pass with `setRecentsScreenshotEnabled(false)`: the stored snapshot is blank; while the private task is in front, the launcher's live tile shows it | `22a`, `22b`; tablet `22b` |
| Private: `IME_FLAG_NO_PERSONALIZED_LEARNING` | Pass on the page field (`imeOptions=0x13000002`) and the NSH field (`0x9000006`) | `21-*`; tablet `21-*` |
| Private: restored by Android after a kill | Pass: refused (no fresh token), process ends, folder swept | `25-*`; tablet `25c` |
| Private: downloads | Pass: saved in `private/<session>/Downloads/`, gone after close; reopening right after a close starts a working session | tablet `27-*`, `logcat-fixes.txt` |

Decisions and findings:
- **Origin-visit file:** deleting `AwOriginVisitLoggerPrefs.xml` in the launch sweep is safe for normal
  mode (profile A kept its cookie, WebView recreated the file), so the sweep deletes it whenever no
  private process runs. The private process did not write it during a session (mtime unchanged). It is
  written although the manifest opts out of WebView metrics.
- **Shield cost:** 230 verdicts, median 82 µs, p90 225 µs, max 16 ms. Building the three built-in lists
  takes 1.9–6.7 s on the emulator at startup; requests that arrive earlier wait for the list they need.
- **Before commit, subresources are judged against the old and the new page:** a local page opened from
  a public one loads without its local subresources until reloaded (`04b`, `04c`).
- **Lockdown in the main process** uses a throwaway `ProfileStore` profile, which is on disk until the
  next launch, like the private space.

Review fixes after the runs above, rebuilt and rechecked on the tablet where marked: popups judged from
their opener (checked), private downloads in the session folder (checked), the close sweep limited to
its own session (checked), a sweep before each new session (covers a task swiped away from Recents),
`stop`, failed loads and same-document history steps end the pending navigation, history steps are
checked before JavaScript changes, unannounced main-frame requests (form POSTs) into a site whose level
turns JavaScript off are refused with `rebuild`, download cookies only for the page's own host,
service-worker requests of every profile checked at Standard, persistent profiles refused without
`MULTI_PROFILE`, cookie import reporting what it handed to WebView, popups' redirects judged from the
opener (checked), `committed` sent when WebView records the navigation (`doUpdateVisitedHistory`), so
Back and Forward across `pushState` entries update the URL (checked, tablet `logcat-round3.txt`), and a
fresh session token on every "Private" (reopening during a close works, checked).

Not verified: `Data::wipe_persistent` and `import_cookies` on a device (no caller in the test shell); the
`rebuild` refusal and the service-worker check on a device;
closing the private space with Back; the `active` prop's effect; a dialog flood cap (dialogs are shown one
at a time, with no burst limit); camera, microphone and location capture (the app declares no Android
runtime permissions, so a granted prompt yields `NotAllowedError` or position unavailable); WebSocket
requests (no WebView hook sees them); IME composition; TalkBack; the release
build on a device (release allows no cleartext, so plain-HTTP sites fail). Android switches JavaScript on
the same view, so `rebuild` is emitted only for the unannounced requests above. `zoom` sets the text
zoom.

**Integration status:** one shared app (`mobile/app`) runs on
both platforms; `tarn-mobile-android` is a member of `mobile/Cargo.toml`, `android/build.sh` packages
`mobile/app`, and the Android test shell is gone. Changes made while integrating:
- one store-id scheme (§2) from `Data::store_id()` on both platforms, and the `data::Platform` trait (§1) for
  the Android private process; Clear Data and a change of the default level reopen the profile, so an
  ephemeral profile gets a new store;
- `load.opener` on iOS (the shell sets it for every `popup` tab), so both components judge a popup tab from
  its opener;
- Android: prompt ids were strings, which the shared shell ignored; they are now numbers unique for the run;
  prompts are capped at 10 per committed page and downloads at 10 per tab in 10 s, as on iOS;
  private-network refusals report `shield` instead of `nav_policy`;
- a main-frame request Shield refused on Android was reported as `finished`, and the shell stored the refused
  URL on the tab, so restoring the tab loaded it as a user-started shell load (seen on the tablet before the
  fix: a restored tab loaded `localhost:8771/local-from-public`). Android no longer reports `finished` or a
  commit for a URL it refused, and a tab's saved URL now comes only from a commit (see the review fixes).

Review fixes (impact and security review of the integration): a popup tab saves no URL until it commits, so
an evicted, reopened or relaunched tab never reloads a blocked redirect as a typed load (checked on the
tablet: no request after a relaunch, `android/tablet/08-*`); a tab's saved URL, a closed tab's entry and the
title are taken only from the last commit (a later `finished` updates the address shown, not the saved URL),
page events for a tab without a view are ignored, and a page that has not committed opens no popup; the opener is the opener tab's last committed
page, not a load still in flight; a `finished` event is dropped only while a load awaits its commit, so a
same-document URL change (`pushState`) still ends the load; iOS forgets the opener when the popup's
navigation is cancelled or fails; Android reports neither `finished` nor `committed` (from any of its commit paths) for a URL it refused
(`tablet/07-*`), keeps a failed load's error when it then commits its own error page,
accepts only hyphenated non-nil store UUIDs, and the private process shows nothing (with a notice) when its
space cannot open; Clear Data in Android's private space says that website data goes when the space closes
(the process keeps one store). Rechecked on the tablet after the last round: the blocked popup tab, a
popup that commits (saved, restored after a relaunch, `tablet/10-*`), and a failed load (Android reports
`failed`, then commits its own error page, which stays on screen, `tablet/09-*`). Deferred: the opener is the tab's top-level page on both platforms, so a
public frame inside a local page opens popups judged as local (contract-level change); a component that
builds after its `Data` is dropped can recreate an empty iOS non-persistent store for the old id that is
never released (memory only, no live path found).

Exit checks with the shared UI, evidence in `mobile/evidence/integration/` (iOS: iPhone 17 iOS 27.0 and iPad
Pro 13-inch iOS 26.2 with `mobile/ios/testserver/server.py`, request log `ios/server.log`; Android:
`Medium_Phone_API_36.1` and `Tablet_10in`, Android 16, with `mobile/android/server.py` as proxy, log
`android/server.log`).

| Check | iOS (restyled build) | Android (shared UI) |
|---|---|---|
| Browsing | Pass `ios/01`, `02` | Pass `android/phone/01`, `02` |
| Tabs: grid, `target=_blank`, gesture `window.open`, no-gesture popup | Pass `04`, `16`, `17`, `18`; no `auto-popup` request | Pass `phone/05`, `06`; no `no-gesture` request |
| Popup redirecting into localhost (opener) | Pass: blocked, only the redirect reached the server `03` | Pass: blocked `phone/04`, tablet `02`, `06` |
| Popup redirecting to a public page | Pass `05` | Not run |
| Profiles: cookie in Personal, none in a new profile, back in Personal | Pass `21`–`23`, two stores on disk `23-website-data-stores.txt` | Pass `phone/15`–`17`, `16-stores.log` |
| Private space | Pass: marker cookie set, space closed, marker found nowhere in the container `24-data-*.txt` (the only diffs are the Personal store's WAL files after Personal reloaded, and a rule list) | Pass: `:private` process, files only in `private/<session>/`; after Close the process ended and the folder was gone `phone/18`, `19`, tablet `04`, `05` |
| Shield private network from a public page (images, link, script, popup) | Pass: no local `/pixel` request `06` | Pass: no pixel request `phone/03`; local link refused |
| HTTPS-only at Strict | Pass: `http://example.com` opened as `https://` `09`, `11` | Pass: typed `http://` became `CONNECT public.test:443` `phone/09`, `10` |
| Per-site Lockdown | Pass: scripts off, the current page reloaded `25` | Not rerun (M3 `15`, `16`) |
| Navigation policy: `mailto:` asks, `data:` refused | Pass `07` | Pass `phone/07`; `data:` refused |
| Prompts: alert, confirm, text, sign-in | Pass `12`–`14`, `19` | Pass `phone/11`–`13` (sign-in not rerun) |
| Prompt cap (10 per page) | Pass: 3 + 7 answered, the rest of 30 declined `15` | Pass: prompts 9 and 10 shown, 11 declined without an event |
| Downloads | Pass: `../tarn report.txt` saved as `_tarn report.txt` in `Documents/Downloads` `20` | Pass: `evil_name.bin` in `files/Downloads` `phone/14*`; the 4th download within 10 s refused `14-downloads.log` |
| Find | Pass (count shown as "Found") `26` | Pass: `matched=3` `phone/20` |
| Zoom | **Fail**: Zoom In makes autosized body text smaller (`WKWebView.pageZoom` with WebKit text autosizing), Actual Size restores `27-*` | Pass (text zoom) `phone/21` |
| Rotation, tablet layout | Pass: wide top bar in landscape and on iPad, no reload `28`, `30-*` | Pass: wide top bar in landscape and on the tablet, same process, sheets 640 dp wide `phone/22`, tablet `01`, `03` |
| Accessibility tree of the address field | Partly: `TextField` inside the group "Address" `29-ax-tree.txt`; VoiceOver not run | Partly: `EditText` inside the group "Address" `phone/23-ax-tree.txt`; toolbar buttons are labelled now, but pill and segmented buttons (Open/Cancel, prompt buttons, Settings segments) have no accessible name; TalkBack not run |

Not verified in this run: crash reload, filter lists (no bundle published), camera, microphone and location
capture, TLS errors, the Android release build, Android sign-in, Clear Data and the default-level change on a
device (host tests only), `install_filters` (not called: there are no filter lists to install until a bundle is
published; the call belongs where lists are loaded), an Android download from a proxied host (the download
worker does not use the debug proxy, so it was checked on `localhost`).

**Polish status:**
- **Zoom on iOS** (the integration failure): `WKWebView.pageZoom` raises the page scale, and WebKit's text
  autosizing gives autosized text less boost at a higher scale, so Zoom In made it smaller. CSS `zoom`,
  a plain `-webkit-text-size-adjust` percentage and magnifying the scroll view hit the same compensation.
  The component now measures, once per document and from the shell's isolated world, the boost
  autosizing gave the page's longest paragraph and sets `-webkit-text-size-adjust` to factor × boost
  (`ios/src/zoom.rs`), waits for the next `resize` while the view is hidden behind a sheet, and reapplies
  after each load. Safari's own text-size control gives the same result (`zoom-safari-reference.png`).
  Limit: on iPad, WebKit serves desktop-class pages that ignore `-webkit-text-size-adjust` and shrink
  with `pageZoom` too, so zoom has no effect there (`09-ipad-zoom-130-no-change.png`).
- **Accessibility:** pills, segments, rows, Done, Cancel and the suggestion, history and space rows set
  their own label, so Android names them as iOS does; the switch rows carry the switch role.
- **One contract:** §2 takes Linux's additions; `engine::view_policy` holds the shared site levels with
  `"*"` and the zoom range; iOS counts find matches; the remaining differences are listed in §2.
- **Shared code:** `nsh-shared/` (theme with the GPUI comparison test, icons, controls in touch and
  desktop density) replaces `mobile/app/src/theme.rs`'s copy and Linux's; `browser_core::prompts` (prompt
  parsing, what may be remembered) and `browser_core::command` (`engine_key`,
  `display_host`, the text of the macOS address label `address`) replace the shell copies. A permission is remembered only for the top page's own
  origin; before, the shell remembered nothing whenever a component named `topOrigin`, which Android
  always does.

Smoke pass, evidence in `mobile/evidence/polish/` (iOS: iPhone 17 iOS 27.0 and iPad Pro 13-inch iOS 26.2 with
`ios/testserver/server.py`; Android: `Tablet_10in` with `android/server.py` as proxy).

| Check | iOS | Android (tablet) |
|---|---|---|
| Browsing | Pass `01` | Pass `tablet-01` |
| Tabs (`target=_blank`) | Pass `04` | Pass (Tabs 3 → 4) |
| Profiles | Host tests only | Host tests only |
| Private space | Pass: no Personal cookie sent, marker nowhere in the container after closing `06-*` | Pass: `:private` process, no main cookie; after Close the process ended and `private/` was empty `tablet-09-*` |
| Shield private network | Not run here (no global IPv6 address); passed in the M4 retry from `https://127.0.0.1.nip.io` (`../m4/`) | Pass: no `/pixel` request from `http://public.test/pn` `tablet-02` |
| HTTPS-only at Strict | Pass: `http://example.com` opened as `example.com` `07` | Pass: only `CONNECT public.test:443` `tablet-07`, `tablet-https-only-server.txt` |
| External scheme | — | Pass: `external {url, origin}`, Cancel/Open named `tablet-03`, `tablet-logcat-1.txt` |
| Prompts | Pass: confirm `02` | Pass: confirm, OK/Cancel named `tablet-04` |
| Downloads | Pass: `_tarn report (2).txt` `03` | Pass: `url`, `quarantined: false` `tablet-download.txt` |
| Zoom | Pass on iPhone (`zoom-*`), no effect on iPad (see above) | Pass: text zoom 150 % `tablet-08-*` |
| Find | Pass: "2 matches", "1 match" `05` | Pass: "3 matches" `tablet-05` |
| Accessibility tree | Pass: every control named `08-ax-*`, `09-ipad-ax-menu.txt` | Pass: every control named `ax-*`, `tablet-ax-*` |

Not run in this pass: `Medium_Phone_API_36.1` (the emulator had 344 MB free and refused the APK; run in M4,
`mobile/evidence/m4/`), crash reload, camera and microphone, TalkBack and VoiceOver themselves.

**M4 CI and docs:**
- CI builds and tests `mobile/` (host tests; iOS simulator build on the macOS runner; Android build);
- threat-model entries for iOS and Android;
- README.

*Status M4:*
- **CI** (`.github/workflows/ci.yml`): job `ios` on `macos-15` runs `cargo clippy --all-targets` and
  `cargo test` in `mobile/`, `cargo clippy --target aarch64-apple-ios-sim --all-targets` and the iOS
  device build; job `android` on `ubuntu-24.04`, in parallel, runs `cargo clippy --target
  aarch64-linux-android` with the runner's NDK (`$ANDROID_NDK_HOME`, API 28 clang; lib and bin targets
  only, because `nsh-testing`'s QuickJS bindings do not build for Android), the arm64-only debug APK
  (`android/build.sh` with nsh 0.5.0's Linux CLI from its release archive, SHA-256 checked) and the
  host unit tests (`testDebugUnitTest`); job `nsh-shared` on `ubuntu-24.04`. `release.yml` splits its
  iOS and Android builds the same way. No secrets. Each command was run locally on macOS (macOS 27,
  NDK 28.2; the APK from a fresh nsh install; `nsh-shared` in an `ubuntu:24.04` container); the
  Android build on a Linux host runs only in CI. The nsh CLI and its Android runtime
  (`nsh-android.tar.gz`) are both downloaded with pinned SHA-256 sums. The simulator app itself is not
  packaged in CI (`nsh package --target ios-simulator`), only checked. Engine update pull requests
  run these jobs like any other; `chromium-update.yml` refreshes the `mobile/`, `linux/` and
  `nsh-shared/` lockfiles together with the root version bump.
- **Threat model:** `docs/threat-model.md`, section "iOS, Android and Linux", including the Android
  private-space wording (S8).
- **Retried checks** (`mobile/evidence/m4/README.md`): the iOS private-network block from a public top
  origin passed (public origin `https://127.0.0.1.nip.io`, a public name that resolves to loopback; the
  rule judges URLs, not DNS answers); the smoke pass on `Medium_Phone_API_36.1` passed (the APK installed
  with 399 MB free, no wipe needed).
- Still open: a physical iPhone, iPad and Android device, VoiceOver and TalkBack, zoom on iPad (no
  effect, see the polish status), crash reload on the phone, camera, microphone and location capture,
  signing and store accounts.

### Themes and search bar effects

Settings › Appearance comes from `tarn_nsh_shared::appearance` (shared with Linux) on the shared core
(`browser_core::theme_library`); website sheets reach the engines as `themeStyles` (threat model S29,
"Themes on Linux, iOS and Android"). Effects come from `tarn_nsh_shared::search_fx` behind the start page's
search card; they keep a still frame with Reduce Motion and stop while the app is not active. Checked on the
iPhone 18 Pro simulator (theme switch in place, Flames, a theme file from Files, website styles on
example.com) and on an Android emulator (Sparkle); evidence in `mobile/evidence/themes/`.
Gaps: no external text editor on iOS or Android, so "Edit in Text Editor" and "Save as File" are desktop-only
(Edit Colours remains); no Shift-at-launch safe mode, since neither platform has a modifier key at launch
(Reset Appearance gives the same result for good). Themes from GitHub (with the update check and the
summary of renames, new owners, changed files, colours and sites) and the signed catalog use the Mac's code
through a `NSURLSession` (iOS) or `HttpURLConnection` (Android host) transport; withdrawals arrive with the
catalog, cached and merged at launch. Gap: withdrawals from the filter bundle, because neither app downloads
the filter bundle at all (Shield's lists and the malware set are missing for the same reason, see above);
they arrive once the bundle download is ported. Checked on the iPhone simulator with a fake GitHub on a
loopback HTTPS server through a loopback proxy (an app built with `TARN_TEST_PROXY`) and a test CA in
the simulator's keychain; evidence in
`mobile/evidence/themes-github/`. Android carries theme sheets in the page world, as it does
site CSS (S28/S29).

### Nightly builds

Not on iOS or Android, a platform gap: the stores decide which build a phone installs, so Settings ›
General › Nightly builds (macOS and Windows, `docs/release.md` "Update channels") has no counterpart.
The nightly pre-release carries the unsigned iOS and Android builds for testers; nightlies for phones
would go through TestFlight and the Play internal testing track, uploaded by hand.

## 5. Design

The mobile app looks like the macOS app: dark appearance (`nsh.json` `"appearance": "dark"`), the navy
backdrop gradient in the brand mark's lake blues, flat opaque surface cards, the Tarn accent (`#1f75b0`, the default) and the eight macOS accents, the profile colours, the
same radii and text sizes, and the same Lucide icons. `nsh-shared/src/theme.rs` (shared with the Linux app; `mobile/app/src/theme.rs` re-exports it and adds the touch sizes) holds the tokens of
`crates/ui/src/theme.rs` under the same names (`Theme::backdrop`, `surface`, `surface_edge`, `control`, `pressed`, `text`, `muted`, `faint`, `accent`, `warning`, `elevation`, `RADIUS_*`,
`TEXT*`, `TOOLBAR_HEIGHT`, `ROW_HEIGHT`, `ACCENTS`, `PROFILE_COLORS`), so they can move into a shared
design-tokens crate; the UI uses nothing else. Mobile-only sizes (`TOUCH_TARGET`, `SHEET_WIDTH`, `WIDE_LAYOUT`, badge and tile sizes) stay in `mobile/app/src/theme.rs`; control sizes (switch, pill, segment) are the touch density of `tarn_nsh_shared::controls`. The icons are `crates/ui/icons/*.svg` and the brand mark
`crates/ui/brand/mark.svg`, linked file by file into `mobile/app/assets/` (NSH's packager takes symlinks to
files, not to folders) together with the Lucide licence.

Kept from the macOS design: a Safari-style address capsule that shows only the host at rest (`Not Secure — host`
in amber for http, long hosts cut from the start, S5) and the full URL selected while editing, with the
shield button inside it naming the level only when it is not Standard (amber shield in the private space)
and Share at its right end while a page is loaded (hidden on an empty tab and while editing); New Tab works
like ⌘T: it opens an empty tab with the address field focused, the keyboard up and history suggestions as you
type, and Cancel on a tab nothing was typed into discards it and returns to the tab before (the Mac never
creates a tab when its new-tab command bar is dismissed);
surface cards for the prompt card, notices and the find bar; pill buttons with a pressed fill; segmented
controls for the Shield level and search engine; switches with the accent track; tiles with radius 14 and an
accent ring for the active tab; the brand mark on an empty tab over the drifting ASCII fog
(`tarn_nsh_shared::fog`, one still frame under the system's reduced motion).

Mobile conventions instead of desktop ones:
- the toolbar sits at the bottom on phones (Back, Forward, New Tab, Tabs, Menu) and moves into the top bar
  beside the address capsule at 700 pt and wider (phone landscape, iPad);
- every control is at least 44 pt (`TOUCH_TARGET`); text uses `TEXT_TITLE` (15) for primary labels and
  `TEXT` (13) for secondary ones, since 13 pt is too small as the main size on a phone;
- menus, the tab grid, settings, history, downloads, Shield and spaces are sheets (a surface card with radius
  16 and a Done button) instead of popovers and windows; on iPad the sheet is 640 pt wide and centred;
- safe areas pad the top bar and the bottom toolbar.

Deviations: the page has no rounded frame (the web view fills the content area edge to edge);
there is no motion yet, no tooltips (touch) and no accent picker (the stored macOS accent is not read). The
macOS icon set has no share or ellipsis glyph, so Share uses `arrow-up-right` and Menu uses `sidebar`.
Permission prompts offer Allow Once, Deny and Always Allow instead of the Mac's Allow and Deny with a Remember
toggle, which would be a second tap target on a phone; Always Allow is what Remember saves.
Settings have no Suggestions switch yet, so history rows and the address field's inline completion are
always on (macOS gates both on Settings › Suggestions); both are computed on the device from local history, as on macOS.
Screenshots: `mobile/ios/evidence/design/` (phone and iPad).

### Tablet layout: Touch or Mac

On a tablet (shorter side at least 600 pt) whose window is at least 700 pt wide, Settings › Appearance ›
Layout chooses Touch (the default, the layout above) or Mac, stored as `layout.tablet` and decided by
`browser_core::layout::effective`. Mac switches the shared controls to desktop density and arranges the
app like the macOS client: sidebar, toolbar and a Settings window (`mobile/app/src/ui/mac/`); the page
area and its web views stay mounted across a switch. Phones and narrow multitasking windows always use
Touch.

The Mac layout mirrors the macOS surfaces: the sidebar (space switcher, Downloads, profile, tiles, pinned
tabs, folders, groups, Clean Up, tab rows with one tab menu in the macOS order, footer); the toolbar
(sidebar toggle, Back, Forward, Reload, the private-space mark while the sidebar is hidden, the address
capsule with the Shield chip, Split View, Share) and the command palette with key caps on its commands;
Site settings, Downloads, Share and QR as popovers anchored under their buttons with the page still shown
behind them; the Settings window with one pane at a time, the Shortcuts pane read from
`browser_core::shortcuts`, and Back stepping from a licence to the list and then to the pane. A hardware
keyboard runs the macOS shortcuts from the same table (⌘ on iPad, Ctrl on Android), the sidebar's shown
state is stored like the macOS one (`sidebar_open`), and the address and palette fields have
autocorrection, spell-check and capitals off.

Gaps (NSH 0.5.0): iPadOS lists no hardware-keyboard shortcut in the ⌘-hold overlay or the menu bar, since
`nsh-platform-ios` matches keys in `pressesBegan` and registers no `UIKeyCommand` or menu builder;
Android delivers no secondary click as a context-menu event, so tab menus open with a long press there.
NSH 0.5.0's press events carry no pointer type, and its iOS long-press timer runs for a trackpad press as
well, so the sidebar tells a held trackpad click from a held finger by whether the pointer hovers the row:
a hovered row opens its tab on a long press, an unhovered one opens the menu. A hovering Apple Pencil
therefore also opens the tab. The Settings window is a card inside the app's surface, since separate OS
windows are desktop-only in NSH.
Sidebar tab rows and tiles drag (to reorder, pin, tile, file into a folder or a group) with a trackpad
or mouse only: NSH 0.5.0 has pans but no lift-to-drag interaction, a long press already opens the tab's
menu, and a finger pan on a row would take the list's scrolling; a touch user pins, tiles and groups
from the menu. A pan reports only its start and end, so no insertion line follows the pointer while
dragging, as it does on macOS.

Not in scope until there is an account to do it with: device builds needing signing, App Store and Play
submission, the iOS default-browser entitlement.
