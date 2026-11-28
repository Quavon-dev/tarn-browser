# Port backlog

Features some clients have and the others still need (`AGENTS.md`, "Partial ports").
Newest entry first. Delete an entry once every client is ticked.

## Template

```markdown
### <Feature name> (<YYYY-MM-DD>, <commit>)

What it does: <one or two sentences, from the user's point of view>.
Shared API: <crate::module::fn / type the clients call>.
Reference: <files of the clients that have it, e.g. crates/app/...>.

- [ ] macOS — <only if it started on another client>
- [ ] Windows — <what is missing, e.g. CEF hook for X>
- [ ] Linux — <linux/src/... to change>
- [ ] iOS — <mobile/app/... or mobile/ios/...>
- [ ] Android — <mobile/app/... or mobile/android/...>
```

## Open

### Tab previews setting (2026-11-28)

What it does: resting the pointer on a tab shows a snapshot of its page without holding Alt (not
for private-space tabs), and Settings › Appearance › Layout › Tab previews picks Page, Title or Off.
Shared API: `browser_core::prefs::{HOVER_CARDS, PAGE_PREVIEWS}`.
Reference: `crates/app/src/hover_card.rs` (`schedule_hover_card`, `request_shot`), `crates/app/src/settings.rs`.

- [x] Windows — same code in `crates/app`
- [ ] Linux — `linux/src/ui/peek.rs` still needs Alt for the snapshot; read the two switches and
  add the choice to its Appearance settings
- iOS and Android have no hover.

### Toolbar in the sidebar (2026-11-28)

What it does: Back, Forward, Reload and the address field sit at the top of the sidebar; under the
tiles a Chats section lists the latest assistant chats; a Tidy rule sorts loose tabs; loose tabs
come before group cards, and a closed group shows its colour, first sites and tab count; a dot
marks a tab that retitled itself in the background; cards at the bottom show an agent at work and
an update ready; the bottom edge holds downloads, the space switcher and the profile menu.
Pinned folders show as the same cards as groups, closed ones with their first sites and tab count;
the Chats section can be hidden (setting, right-click, `Command::SidebarChats`); the assistant is
a card beside the page drawn like the sidebar, with the model picker and a round Send in its
question field. Settings › Appearance › Sidebar picks large or compact pinned-site tiles, cards or the
classic group style, Chats, the changed-tab dot and dimmed sleeping tabs; Corners also applies in
the Classic design.
Shared API: `browser_core::sidebar::{chat_rows, ChatRow, CHATS_SHOWN, group_summary, folder_summary}`,
`browser_core::tabs::{Model::{note_title, seen, unread}, tile_row_sized, SMALL_TILE_MIN_WIDTH}`,
`browser_core::theme_library::classic_corners`, `palette::Command::{SidebarToolbar, SidebarChats}`.
Reference: `crates/app/src/sidebar.rs` (`content`, `footer`, `chats`, `tidy`, `pills`,
`group_card`, `folder`, `summary_row`), `crates/app/src/toolbar.rs` (`sidebar_address`, `history`,
`render`), `crates/app/src/assistant.rs` (`render`, `chat`, `model_chip`).

- [x] Windows — same code in `crates/app`; a 32 px strip above the page keeps the window
  buttons. Not yet run on Windows.
- [ ] Linux — `linux/src/ui.rs` `sidebar`: header with navigation and address, chats (and the
  switch to hide them), Tidy, group and folder cards with their summary, unread dot (call
  `note_title` from the title signal), footer, the assistant card; offer `Command::SidebarToolbar`
  and `Command::SidebarChats` in `linux/src/ui/tools.rs`
- [ ] iOS — iPad sidebar `mobile/app/src/ui/mac/sidebar.rs`: chats (and the switch to hide
  them), Tidy, group and folder cards with their summary, unread dot, footer, the assistant card;
  the address stays in its bar on touch
- [ ] Android — the same sidebar in the Mac layout

### Delete a profile's last space (2026-11-26)

What it does: deleting a profile's last space deletes the profile with it, after a confirmation
that says so; only the last space of the last profile stays.
Shared API: `browser_core::tabs::Model::can_delete_space`, `Model::profile_deleted_with`.
Reference: `crates/app/src/browser/context.rs` (`confirm_delete_space`), `linux/src/ui/profiles.rs` (`delete_row`).

- [x] Windows — same code in `crates/app`
- [x] Linux
- [ ] iOS — the Edit Space sheet (`mobile/app/src/ui/sheets.rs`) deletes other spaces only; the
  shell has no profile deletion that also clears the profile's `WKWebsiteDataStore`
- [ ] Android — the same sheet; profile deletion must also clear the profile's WebView data

### Inspector commands and docked Chromium DevTools (2026-11-26)

What it does: Show or Hide Inspector and Move Inspector to Other Side in the command palette; in a
Chromium tab DevTools docks beside the page.
Shared API: `browser_core::palette::Command::{ToggleInspector, MoveInspector}`,
`engine::WebView::{toggle_inspector, move_inspector}`, `engine::view_policy::dock`.
Reference: `crates/engine/src/chromium/view.rs` (`TarnBrowserViewDelegate`, `relayout`).

- [ ] Windows — DevTools still opens in a window of its own (`on_before_dev_tools_popup`); docking
  needs the inspector browser as a second child window placed by `relayout`
- [ ] Linux — `linux/src/ui/tools.rs` offers neither command; WebKitGTK's inspector can be shown
  through `webkit_web_view_get_inspector`
- iOS and Android have no web inspector in the app.

### Agent setup as code with Copy (2026-11-26)

What it does: Settings › AI lists the command for Claude Code and the config for Codex as code,
each with a Copy button.
Shared API: `browser_core::agent::setup_steps`.
Reference: `crates/app/src/settings/ai.rs` (`agent_control`), `ui::controls::code_block`.

- [x] Windows — same code in `crates/app`
- [ ] Linux — `linux/src/ui/agent.rs` still shows `agent::setup` as one note
- iOS and Android have no agent control.

### Share in the address field (2026-11-26)

What it does: Share sits in the address field beside the bookmark star instead of the toolbar;
with a pointer both show while it is over the field or a button has keyboard focus, and the star
stays shown for a bookmarked page.
Shared API: none; layout only.
Reference: `crates/app/src/toolbar.rs` (`share_button`, `on_hover`), `linux/src/ui.rs` (`share_chip`).

- [x] Windows — same code in `crates/app`
- [x] Linux — Share only; its address field has no star
- [ ] iOS — the desktop-style toolbar in `mobile/app/src/ui/mac/toolbar.rs` (iPad with a pointer)
  still shows Share as its own button
- [ ] Android — the same toolbar on tablets with a pointer

### Give AI full access (2026-11-26)

What it does: one switch at the top of Settings › AI turns on, or off, agent control and the
profile's assistant; the profile and tabs modes keep their own switches. Every question Tarn asks
an agent stays.
Shared API: `browser_core::agent::{FULL_ACCESS, DETAIL_FULL_ACCESS}`, the existing
`agent::SETTING*` settings and `Store::set_profile_ai`.
Reference: `crates/app/src/settings/ai.rs` (`full_access`).

- [x] Windows — same code in `crates/app`
- [ ] Linux — `linux/src/ui/agent.rs` `settings` with the assistant switch of
  `tarn_nsh_shared::assistant::settings_pane`, which must then redraw
- [ ] iOS — after agent control reaches mobile; until then the switch would be the assistant's alone
- [ ] Android — same as iOS

### Owner check before trusting a certificate (2026-11-26)

What it does: before Tarn keeps a certificate exception from the certificate error page, the
device's owner confirms with Windows Hello or the system password, as Touch ID does on macOS.
Shared API: `engine::confirm_owner` (macOS: LocalAuthentication).
Reference: `crates/engine/src/biometrics.rs`, `crates/app/src/blocking.rs` (`trust_certificate`).

- [ ] Windows — `engine::confirm_owner` is a stub that answers yes; call Windows Hello through
  `IUserConsentVerifierInterop::RequestVerificationForWindowAsync` with the window handle.

### Find bar follows the tab (2026-11-25)

What it does: with the find bar open, switching tabs removes the marks from the tab left and
searches the bar's text in the new one, after its page has loaded when it was asleep.
Shared API: the clients' existing `find` / `clear_find` (`clearFind`) operations.
Reference: `crates/app/src/browser/views.rs` (`activate`) and the `find_after_load` handling in
`crates/app/src/browser/events.rs`.

- [ ] Linux — `linux/src/ui.rs`: on a tab switch with `ui.finding` set, `clear_find` the old tab and
  `find_typed` the text in the new one.
- [ ] iOS — `mobile/app/src/ui/panels.rs` (`find`): the same when the active tab changes while the
  find panel shows.
- [ ] Android — as iOS, shared `mobile/app` code.

### See-through window for the Glass design (2026-11-25)

What it does: with Settings › Appearance › Design on Glass, the window lets the desktop show
through behind the sidebar and toolbar, as far as text stays readable.
Shared API: `browser_core::theme_library::look` with the `Backing` the window can have.
Reference: `crates/app/src/themes.rs` (`window_background`), `linux/src/browser/themes.rs`.

- [ ] Windows — `crates/app/src/themes.rs` asks for `Backing::Opaque` off macOS; GPUI can give
  acrylic (`Blurred`), which needs trying over CEF views before the transparency rows show there.
- [ ] Android — `mobile/app/src/themes.rs` asks for `Backing::Opaque`; the activity could show the
  wallpaper behind the chrome (`FLAG_SHOW_WALLPAPER`, blurred with `setBackgroundBlurRadius` on
  API 31+), with `mobile/android` passing `Backing::Clear` or `Blurred` up.

### Picture in Picture on tab switch (2026-11-24)

What it does: a video playing with sound moves into Picture in Picture when its tab leaves the
screen, and back into the page when the tab returns; on by default, a switch in Settings › Tabs.
Shared API: `browser_core::prefs::AUTO_PICTURE_IN_PICTURE`, `browser_core::media::AutoPip`,
`engine::page_scripts::picture_in_picture_script(PipMode)`.
Reference: `crates/app/src/mini_player.rs` (`settle_screen`, called on every render from
`crates/app/src/content.rs`), which hands the tabs on screen to `AutoPip::screen`. Linux and
Android are gaps (`docs/nsh-plan.md`, `docs/mobile-plan.md`).

- [ ] iOS — a `pictureInPicture` command on the `tarn.webview` component in `mobile/ios` running
  the script for `Enter`/`Exit` (WKWebView with `allowsPictureInPictureMediaPlayback`), called by
  `mobile/app` when a tab leaves or returns to the screen, and the Settings switch.

### Ask before sending a form again (2026-11-24)

What it does: the user's own Back, Forward or Reload to a page a form was sent for, no longer kept, asks
"would get your form again" with Send Again or Cancel instead of sending the form (an order, a
message) silently; a page's `history.back()` never sends it and shows `nav_policy::RESEND_STOPPED`.
Shared API: `engine::PromptKind::ResendForm`, `browser_core::prompts::RESEND_ASK` and
`meaning("resend form")`.
Reference: `crates/engine/src/delegates.rs` (`ask_to_resend`), `crates/app/src/content.rs`,
`mobile/ios/src/handler.rs`, `mobile/app/src/ui/prompt.rs`. Windows (Chromium) shows its own
resubmission page; Android's WebView never sends the form again.

- [ ] Linux — hold the `NavigationType::FormResubmitted` decision in `linux/src/webview/mod.rs`
  (`allow_navigation`) behind a `resend form` prompt in `linux/src/browser`.

### Agent control in the user's profile or tabs (2026-11-24)

What it does: two more opt-in settings in Settings › AI let the user approve an agent to open its
own tabs in their front window's profile (Use My Logins) or to work in their open tabs outside
private spaces (Use My Tabs); per-site approval, no extensions, passkeys or history in the agent's
window, a bar with Stop on every tab the agent controls (threat model S22, S31).
Shared API: `browser_core::agent::{Mode, SETTING_PROFILE, SETTING_TABS, DETAIL_PROFILE,
DETAIL_TABS, web_site}`, `control::Ctx::{profile_on, profile, tabs_on, yours}`, `control::Effect::Switch`,
`control::Choice::{Profile, Tabs}`, `tabs::Model::open_agent_in`, `engine::Features`.
Reference: `crates/app/src/agent.rs`, `crates/app/src/settings/ai.rs`.

- [ ] Windows — `crates/app/src/agent.rs` `SIGNED_IN` keeps both modes off: a CEF view cannot
  turn passkeys off (`engine::WebView::set_passkeys` does nothing there), so WebAuthn must be
  refused per view before the modes can be offered. The same holds for Chromium tabs on macOS,
  which Use My Tabs refuses and an agent's window never uses.
- [ ] Linux — the two switches in Settings › AI, `Ctx::{profile_on, profile, tabs_on, yours}` from the front window's
  profile and the user's windows in `linux/src/ui/agent.rs`, `Effect::Switch` (today it ends the
  session), a per-tab gate and the bar in every window, no extensions or history in the agent's
  window.
- [ ] iOS — after the Agent control entry below.
- [ ] Android — after the Agent control entry below.

### Agent control (2026-11-24)

What it does: an AI agent on the same device browses for the user in a throwaway window of its
own over a local MCP socket, off by default, with the user approving the agent, each site and each
click on a page holding a filled password or payment field (threat model S31).
Shared API: `browser_core::agent::{mcp, control}` (`mcp::serve`, `control::Controller`),
`engine::agent_socket`, `engine::agent_scripts`, `tabs::Model::open_agent` and `burn_agent`.
Reference: `crates/app/src/agent.rs`, `linux/src/ui/agent.rs`.

- [x] Linux — `linux/src/ui/agent.rs`; the gate holds clicks and form submissions only (see
  `docs/nsh-plan.md`).
- [ ] iOS — no local process can reach an app's socket while it is in the background; needs a
  design (for example an agent inside the app) before any port.
- [ ] Android — a socket in the app's private storage is out of reach of other apps; needs an
  agent on the device that can reach it, then the `agent` command and gate in `TarnWebView`.

### Insert an assistant answer into the page (2026-11-24)

What it does: the chat's Insert into Page types the last answer into the page's focused field, as
Insert Snippet does, refused under Lockdown.
Shared API: `browser_core::assistant::Session::last_answer`, then each client's Insert Snippet path
(`page_scripts::insert_text_script`, `snippets::inserted`).
Reference: `crates/app/src/assistant.rs` (`insert_last_answer`), `crates/app/src/snippets.rs`.

- [ ] Linux — a bubble action in `nsh-shared/src/assistant.rs` through a new `Host` method that
  `linux/src/browser/snippets.rs` answers with its Insert Snippet request.
- [ ] iOS — the same `Host` method answered by `mobile/app/src/snippets.rs`.
- [ ] Android — as iOS; the insert script runs in the page's world there (threat model S1).

### Portable download (2026-11-24)

What it does: a download that runs without installing and keeps all its data in a `Data` folder
beside the executable, with a command-line install for the regular build.
Shared API: `browser_core::portable_dir` (a `tarn-portable` file beside the executable), used by
`browser_core::data_dir`; Windows only so far.
Reference: `.github/workflows/release.yml` (`windows` job, portable zip; `assets` job,
`install.ps1`), `crates/platform/src/updater/windows.rs` (portable updater), `packaging/homebrew/`.

- [ ] Linux — turn `portable_dir` on for Linux, refusing a folder the user does not own or others
  can write (a marker in `/tmp` would take a victim's profile); key `single_instance`
  (`linux/src/main.rs`) on the data folder as Windows does; and build a release archive (AppImage or
  tarball with `tarn-portable`) in `linux/packaging/build-all.sh`, which needs WebKitGTK 6 from the
  distribution or bundles it.

### Hand freed memory back to the system (2026-11-24)

What it does: once a minute, after tabs closed or slept, and once when Tarn leaves the front, the
shell returns memory it freed to the system, so it holds less while idle.
Shared API: `engine::lifecycle::release_memory(closed, was_active, active)`; macOS calls
`platform::memory::release_free`, Linux `engine::trim::release_free`.
Reference: `crates/app/src/shared.rs` (minute tick), `linux/src/ui.rs` (`SCHEDULE_TICK`).

- [x] Windows — same code in `crates/app`; the Windows heap decommits free pages itself
- [x] Linux — `malloc_trim` through `engine::trim`
- [ ] iOS — call `malloc_zone_pressure_relief` from the shell's minute tick or memory warning in
  `mobile/app`
- [ ] Android — no allocator call needed beyond `onTrimMemory`; wire the decision into
  `mobile/android` once the shell has a minute tick

### Client certificate picker (2026-11-23)

What it does: when the page's own site asks for a TLS client certificate, a tab-modal prompt lists
the certificates the user has that the server accepts and sends only the one they pick; Cancel
sends none and the page loads as it would without one. Other sites never get to ask.
Shared API: `engine::PromptKind::ClientCertificate { names }`, `engine::Prompt::choose(index)`;
NSH clients use the `"certificate"` kind with `browser_core::prompts::{Prompt::ask, meaning}` and
`browser_core::glyphs::glyph`, and `view_policy::may_ask_credentials` for the origin check.
Reference: `crates/engine/src/delegates.rs` (`auth_challenge`, `keychain_identities`),
`crates/engine/src/chromium/view.rs` (`on_select_client_certificate`, `ask_certificate`),
`crates/app/src/content.rs` (`prompt_card`), `crates/app/src/browser/events.rs`
(`choose_certificate`).

- [x] Windows — same code in `crates/app` and the Chromium engine
- [ ] Linux — WebKitGTK raises `AuthenticationScheme::ClientCertificateRequested`, which
  `authenticate` in `linux/src/webview/mod.rs` cancels; GTK has no system certificate store, so
  answering it needs a way to pick a certificate file and `Credential::for_certificate`
- [ ] iOS — `challenge` in `mobile/ios/src/handler.rs` takes the default handling; an app cannot
  read the system's identities, so this needs certificates imported into Tarn's own keychain
- [ ] Android — `onReceivedClientCertRequest` in `TarnWebView` with
  `KeyChain.choosePrivateKeyAlias`, behind the same origin check

### Insert Snippet in the page's context menu (2026-11-22)

What it does: right-clicking (long-pressing) a text field on a page offers Insert Snippet…, which
opens the same snippet list as the command bar; macOS and Windows also take multi-line snippet text
in Settings, as the NSH pane does.
Shared API: `palette::Command::InsertSnippet`, `browser_core::snippets`; web view operation
`insert_text` / component `insertText`.
Reference: `crates/app/src/snippets.rs`, `linux/src/browser/snippets.rs`,
`nsh-shared/src/snippets.rs`.

- [ ] macOS — an entry in WebKit's editable-field menu (`crates/engine/src/delegates.rs`,
  `context_menu`) and Chromium's, sent to the shell as an engine event; a multi-line field for the
  text in `crates/app/src/settings/snippets.rs` (`ui::TextInput` is one line)
- [ ] Windows — the same in `crates/app` and the Chromium menu
- [ ] Linux — WebKitGTK's `context-menu` signal for editable targets in `linux/src/webview`
- [ ] iOS — the edit menu of a focused field in `mobile/ios`
- [ ] Android — the text selection action mode in `TarnWebView`

### Measure (2026-11-22)

What it does: a developer tool that shows rulers and a box the user drags over a still of the page,
labelled with its size and position in CSS pixels, with guides at the pointer; a click or a second
use closes it; offered only with the developer tools on.
Shared API: `browser_core::measure::{ticks, css_box, label, is_click, point_label, HINT}`, `palette::Command::Measure`
with `palette::State::developer`; `tarn_nsh_shared::page_overlay` (`Purpose::Measure`).
Reference: `crates/app/src/capture_overlay.rs`, `linux/src/ui/capture.rs`.

- [x] Windows — same code in `crates/app`
- [x] Linux
- [ ] iOS — needs the Settings › Developer pane first (entry below); then a two-tap distance on the
  shared overlay in `mobile/app`, with `State::developer` from `devkit::SETTING`
- [ ] Android — same as iOS

### Mini-player while the sidebar is hidden (2026-11-22)

What it does: the mini-player of a tab left playing stays reachable when the sidebar is hidden,
as on macOS, where the floating sidebar shows it.
Shared API: `browser_core::media::Player`; `tarn_nsh_shared::mini_player`.
Reference: `crates/app/src/mini_player.rs`, `crates/app/src/sidebar.rs`.

- [x] Windows — same code in `crates/app`
- [ ] Linux — `linux/src/ui.rs` draws the mini-player and runs its poll timer in the sidebar only;
  show it (and keep the timer) while the sidebar is hidden
- [x] iOS — above the bottom bar on phones, in the sidebar on iPad
- [x] Android — above the bottom bar

### Springy pop-ups and cards (2026-11-21)

What it does: pop-ups, toasts, cards, link previews, hover cards, the Library's page turn, the
floating sidebar, folder chevrons and switches spring into place instead of easing in flat, most
with a small overshoot.
Shared API: `ui::motion::{SETTLE, POPUP_TRAVEL, arrive, rise_in, popover_motion}` (GPUI).
Reference: `crates/ui/src/motion.rs`, `crates/ui/src/controls.rs` (`sliding_switch`),
`crates/app/src/toast.rs`, `crates/app/src/sidebar.rs` (`floating`, `chevron`).

- [x] Windows — same code in `crates/app`
- [ ] Linux — `nsh-shared/src/motion.rs` `pop_in` and `nsh-shared/src/popover.rs` still ease in
  flat; give pop-ups their own overshooting spec over `SETTLE` (not through `popping`, which also
  moves small icons), keep opacity off the overshooting curve, and spring the switch knob in
  `nsh-shared/src/controls.rs`
- [ ] iOS — the same shared NSH change reaches `mobile/app`
- [ ] Android — same as iOS

### Livelier tab motion (2026-11-21)

What it does: a new tab opens its gap, then sweeps in from the side and lands with a small spring;
a closed tab sweeps off before its gap closes; the active tab's highlight swells from the row's
middle while the previous one fades; a tab's icon springs up when loading starts or ends; tiles rise
in; the rows of the address palette and of the start page's suggestions rise in one after another.
Shared API: `ui::motion::{TAB_IN, TAB_OUT, SWEEP, STAGGER, bounce, phase, animate_linear,
stagger_in}` (GPUI); NSH has `tarn_nsh_shared::motion::popping`.
Reference: `crates/app/src/sidebar.rs` (`note_motion`, `tab_row`, `leaving_row`, `tiles`),
`crates/app/src/content.rs` (`suggestion_rows`).

- [x] Windows — same code in `crates/app`
- [ ] Linux — the icon springs up when loading starts or ends (`linux/src/ui.rs` `tab_row`), but
  also when the row is first built; still needs the sweep in and out of tab rows (an
  entering/exiting `MotionSpec` on `TranslateX`), the highlight swell and fade on a child view (a
  colour transition on the whole row would also slow the drop-target colour), rising tiles and
  staggered palette rows (`Animation::stagger_from`)
- [ ] iOS — the same icon spring in the tablet sidebar and its tiles
  (`mobile/app/src/ui/mac/sidebar.rs` `tab_icon`), also when the row is first built; still needs
  the row sweep, the highlight swell and fade (on a child view, so press feedback stays instant),
  rising tiles, the staggered address suggestions and the phone tab cards
  (`mobile/app/src/ui/panels.rs`)
- [ ] Android — same as iOS

### Native page frame set only when it changes (2026-11-21)

What it does: the shell no longer sets the page's native frame and opacity again on every repaint
(a turning spinner, a fade) when they have not changed, so the page is not laid out anew.
Shared API: none; each engine compares with the native view's current frame in `set_frame`.
Reference: `crates/engine/src/webview.rs`, `crates/engine/src/chromium/view.rs` (macOS branch).

- [ ] Windows — `crates/engine/src/chromium/view.rs` `set_frame` still calls `SetWindowPos` every
  time; compare with the child window's current rect (`GetWindowRect` mapped to the parent), not a
  cached request, since Chromium may move its own window
- [ ] Linux — check `linux/src/webview` for the same repeated allocation
- [ ] iOS — check the WKWebView frame updates in `mobile/ios`
- [ ] Android — check `TarnWebView` layout updates

### Smooth sidebar slide (2026-11-21)

What it does: showing or hiding the sidebar moves the page without laying it out on every frame (it
keeps one size and takes its final size when the slide ends), and a toggle mid-slide turns back from
where the sidebar stands instead of jumping to the far edge.
Shared API: `browser_core::layout::sliding_width`, `browser_core::layout::slide_at`.
Reference: `crates/app/src/content.rs` (`sidebar_shown`, `sidebar_slide`, the page canvas),
`crates/app/src/split.rs` (`pane`), `crates/engine/src/view.rs` (`clipped_by_window`).

- [x] Windows — same code in `crates/app`
- [ ] Linux — `linux/src/ui.rs` `toggle_sidebar` slides with NSH's layout animation
  (`animate_next(SLIDE)`); measure whether the WebKitGTK view is laid out on every frame and whether
  a second toggle mid-slide jumps, and hold the width with `sliding_width` where it is
- [ ] iOS — the same for the WKWebView beside the sidebar in `mobile/app` (`features.rs`
  `toggle_sidebar`)
- [ ] Android — the same for `TarnWebView`

### Download notices float over the page (2026-11-20)

What it does: a finished or failed download (and a saved screenshot or PDF) shows a toast over the
page instead of a banner that moves it, only while the tab that was in front is active; a finished
download's toast disappears after 5 seconds, a warning stays until dismissed and nothing replaces it.
Toasts about downloads keep the security colours (S29).
Shared API: `browser_core::toast::download`.
Reference: `crates/app/src/browser/views.rs` (`notify_download`), `crates/app/src/toast.rs` (`show_tab_toast`,
`secure_card`).

- [x] Windows — same code in `crates/app`
- [ ] Linux — `linux/src/browser/pages.rs` (`on_download`) shows no notice for a finished download; add a
  tab toast with `browser_core::toast::download`
- [ ] iOS — `mobile/app` `download_notices` / `notice`: an overlay bound to the tab, 5 s unless it warns
- [ ] Android — the same in `mobile/app`

### Passkeys and security keys switch (2026-11-20)

What it does: Settings › Privacy › Passkeys and security keys (on by default) turns WebAuthn sign-in
on or off for pages; off, `PublicKeyCredential` is gone from pages once they reload.
Shared API: `browser_core::settings::{PASSKEYS, PASSKEYS_NOTE, passkeys}`; `engine::WebView::set_passkeys`.
Reference: `crates/app/src/browser/context.rs` (`view_prefs`), `crates/app/src/settings/privacy.rs` (Sign-in),
`crates/engine/src/webview.rs` (`set_passkeys`).

- [ ] Windows and macOS Chromium tabs — CEF has no per-view WebAuthn switch; Chromium's
  `--disable-blink-features=WebAuth` at CEF start (`crates/engine/src/chromium`), so the switch would
  take effect after a restart; then show the row on Windows (hidden today) and drop "in WebKit tabs"
  from `PASSKEYS_NOTE`
- [ ] Linux — the switch in `linux/src/ui/settings.rs`; WebKitGTK's WebAuthn setting in `linux/src/webview`
- [ ] iOS — the switch in `mobile/app`; the same `_WKFeature` flags on the `WKWebView` in `mobile/ios`
- [ ] Android — the switch in `mobile/app`; `WebSettingsCompat.setWebAuthenticationSupport` in `TarnWebView`

### Typed input keeps a tab awake; sleep keeps the scroll offset (2026-11-19)

What it does: a background tab whose page has form input the user typed is not put to sleep by the
idle timer, the live-tab limit or the memory budget (Sleep Tab still sleeps it); a tab that sleeps
scrolls back to where it was once its page loads again. Neither applies under Lockdown, where no
script of Tarn's runs on the page.
Shared API: `engine::page_scripts::{PAGE_KEEP, page_keep, PageKeep, scroll_script}`;
`engine::WebView::{page_keep, scroll_to}`.
Reference: `crates/app/src/tabs.rs` (`sleep_keeping_scroll`), `crates/app/src/browser.rs`
(`PageState::scroll_back`), `crates/app/src/browser/events.rs` (`Event::Finished`).

- [x] Windows — same code in `crates/app`, `crates/engine/src/view_windows.rs`
- [ ] Linux — run `PAGE_KEEP` in the isolated world before `sleep_tab` / `unload` in
  `linux/src/memory.rs`, and `scroll_script` on the next load-finished of that page
- [ ] iOS — `mobile/ios` web view: the same two scripts around the `mobile/app` lifecycle eviction
- [ ] Android — `TarnWebView`: the same two scripts around the `mobile/app` lifecycle eviction

### New Private Window and New Burner Window from the Dock (2026-11-19)

What it does: the app icon's menu (Dock on macOS, jump list on Windows) opens a new window straight
into the private space or a burner tab.
Shared API: none beyond the existing private space and burner (`browser_core::tabs::Model::{private_space, open_burner}`).
Reference: `crates/app/src/lib.rs` (`set_dock_menu`), `crates/app/src/shared.rs` (`new_window_then`).

- [x] Windows — same code in `crates/app` (jump list)
- [ ] Linux — `Actions=` entries in the `.desktop` file that start Tarn with a private or burner window
- [ ] iOS — Home Screen quick actions (`UIApplicationShortcutItem`)
- [ ] Android — launcher shortcuts (`ShortcutManager` / `shortcuts.xml`) for `PrivateActivity` and a burner tab

### Settings › Developer (2026-11-17)

What it does: a switch for the developer tools (Web Inspector through Inspect Element, the JSON viewer,
responsive sizes, local servers) and the Developer profile; the Web Inspector never opens on Lockdown
pages.
Shared API: `devkit::SETTING`, `Store::create_dev_profile`, `Profile::is_dev`;
`tarn_nsh_shared::panes::Pane::Developer`.
macOS reference: `crates/app/src/settings/privacy.rs` (`developer`); Linux: `linux/src/browser/devtools.rs`,
`linux/src/ui/settings.rs`.

- [x] Windows — same code in `crates/app`
- [x] Linux — `linux/src/browser/devtools.rs`, `linux/src/ui/settings.rs`
- [ ] iOS — the pane in `mobile/app` (the Mac layout's Settings window skips `Pane::Developer` today) and
  `WKWebView.isInspectable` in `mobile/ios` behind the switch, off under Lockdown
- [ ] Android — the same pane and `WebView.setWebContentsDebuggingEnabled` in `TarnWebView`, which
  turns it off today, behind the switch

### Tab sleeping choices and the Suggestions switch (2026-11-17)

What it does: Settings chooses how many tabs stay awake (3, 6, 10, 16) and how long a background tab
waits before it sleeps (Never, 10 minutes, 30 minutes, 2 hours); a switch turns history
suggestions, the inline completion and full-text matches off.
Shared API: `browser_core::settings::{live_tabs, sleep_after, max_idle, LIVE_TABS, SLEEP_AFTER, SUGGESTIONS}`,
`engine::lifecycle::Lifecycle::set_max_live`.
macOS reference: `crates/app/src/settings.rs` (`tabs`, `search`), `crates/app/src/prefs.rs` (`set_choice`), `crates/app/src/tabs.rs` (`sleep_idle_tabs`).

- [x] Windows — same code in `crates/app`
- [x] Linux — `linux/src/ui/settings.rs` (Tabs, Search › Suggestions), `linux/src/memory.rs`
- [ ] iOS — `mobile/app` keeps a fixed `LIVE_TABS` of 6 and always suggests; the Settings sheet needs both choices and the switch
- [ ] Android — same as iOS

### Save Page as PDF and Print… in the share menu (2026-11-17)

What it does: the share menu saves the page as a PDF (listed in Downloads) and opens the print
dialog.
Shared API: `browser_core::text::capture_name` (the PDF's file name); web view operations `pdf`
and `print` (`docs/shared-core.md` §1).
macOS reference: `crates/app/src/share.rs`, `crates/app/src/capture.rs`, `crates/app/src/browser/events.rs` (`print`).

- [x] Windows
- [x] Linux
- [ ] iOS — `print` and `pdf` in `mobile/ios` (`UIPrintInteractionController`, `WKWebView.createPDF`)
  and the two entries in the share sheet of `mobile/app`
- [ ] Android — `print` and `pdf` in `TarnWebView` (`PrintManager` with `createPrintDocumentAdapter`)
  and the two entries in `mobile/app`

### Bookmarks manager (2026-11-17)

What it does: Bookmark Tab… and Bookmark All Tabs, and a manager with recent bookmarks, folders,
search, rename, move, delete and New Folder.
Shared API: `browser_core::bookmarks::{bookmark, unbookmark, bookmark_all, new_folder, move_targets, subtree, recent, search,
save_for_later, reading_list_pages, title_or_host}`;
`store::Store::{rename_bookmark, move_bookmark, delete_bookmark}`.
macOS reference: `crates/app/src/library.rs` (`Page::Bookmarks`), `crates/app/src/library/actions.rs` (`row_menu`), `crates/app/src/menubar.rs` (Bookmarks menu).

- [x] Windows — same code in `crates/app`
- [x] Linux — `linux/src/ui/library.rs` (`bookmarks_panel`), `linux/src/browser/bookmarks.rs`
- [ ] iOS — `mobile/app` has only the Reading List, with its own copy of Save for Later that saves a page twice
  (`mobile/app/src/features.rs`); move it onto `bookmarks::save_for_later`, then Bookmark Tab, Bookmark All Tabs and a
  bookmarks sheet
- [ ] Android — same as iOS

### Tab actions and Mute Site (2026-11-17)

What it does: Mute Site (saved per profile, a Lockdown mute lasts while the tab stays on the host;
a mark on the tab and a Muted sites list in Settings › Privacy), Rename Tab and Reset Title,
Return to Pinned URL and Set Pinned URL to Current Page, Duplicate Tab, Close All, Other and Below,
Sleep Tab, Other and All Inactive Tabs, reopening a chosen recently closed tab, Delete Folder,
the sidebar menu, and closing a pinned tab unloads it.
Shared API: `browser_core::site::saves_mute`, `Store::{muted_sites, set_site_muted}`,
`browser_core::tabs::{custom_title, Model::pinned_drift, Model::bookmark_all,
Model::folder_contents}`, `ClosedTabs::{recent, take}`; web view `set_muted` (NSH: `muted` prop).
macOS reference: `crates/app/src/browser/context.rs` (`tab_menu`, `sidebar_menu`, `folder_menu`),
`crates/app/src/tabs.rs`; Linux: `linux/src/browser/tab_actions.rs`, `linux/src/ui/panels.rs`.

- [x] Windows — same code in `crates/app`
- [x] Linux — `linux/src/browser/tab_actions.rs`, the Tab, Sidebar, Folder and History panels
- [ ] iOS — the `muted` prop in `mobile/ios` (`WKWebView` media muting) and the tab actions in the
  tab sheet of `mobile/app/src/ui/panels.rs`
- [ ] Android — the `muted` prop in `TarnWebView` and the same tab actions in `mobile/app`

### Tab, accent and motion settings (2026-11-17)

What it does: "New tab opens" (command card or start page), "After closing a tab" (next or
previous), Touchpad gestures, the accent colour and Reduce motion, stored under the macOS keys.
Shared API: `browser_core::prefs` (`NEW_TAB_START_PAGE`, `CLOSE_SELECTS_PREVIOUS`, `GESTURES`,
`REDUCE_MOTION`, `accent_choice`, `accented`); `tarn_nsh_shared::motion::{set_reduced, reduced}`;
the Accent and Reduce motion rows in `tarn_nsh_shared::appearance` show where
`Host::accent_and_motion` is true.
macOS reference: `crates/app/src/settings.rs` (`appearance`, `tabs`), `crates/app/src/prefs.rs`.

- [x] Windows — same code in `crates/app`
- [x] Linux — `linux/src/ui/settings.rs`, `linux/src/ui.rs` (`new_tab`, `go`), `linux/src/browser/themes.rs`
- [ ] iOS — `mobile/app`: `accent_and_motion` in `ui/appearance.rs`, `prefs::accented` and
  `motion::set_reduced` in `themes.rs`, `motion::reduced` for its own `reduce_motion()` checks
  (`ui/mod.rs`, `ui/bars.rs`),
  `CLOSE_SELECTS_PREVIOUS` in `features.rs` (`close_neighbour`); new-tab and gesture rows where they apply
- [ ] Android — same as iOS

### Profile editor and space card (2026-11-17)

What it does: Create and Edit Profile (name, colour, first space, Shield level, search engine,
shared data), Delete Profile with its website data retired, the profile switcher with colour dots
and Manage Profiles, the Settings › Profiles list with the Developer badge, Delete Space…, the
space context menu (Rename, Edit Space, New Space, Delete), and switching to space 1–9 by key.
Shared API: `browser_core::profiles::{create, delete, deletable, profile_name, retired_stores,
set_retired}`; `browser_core::space_card::SpaceDraft` for the New/Edit Space card;
`Model::can_delete_space`, `Model::space_to_delete`.
macOS reference: `crates/app/src/profiles.rs`, `crates/app/src/spaces.rs`,
`crates/app/src/browser/context.rs` (`space_menu`, `delete_space`), `crates/app/src/menubar.rs`.

- [x] Windows — same code in `crates/app`
- [x] Linux — `linux/src/ui/profiles.rs`, `linux/src/browser/profiles.rs`
- [ ] iOS — `mobile/app` has the New/Edit Space sheets and creates profiles, but cannot edit or delete profiles and has no Delete Space; the store retirement needs the `WKWebsiteDataStore` removal macOS uses
- [ ] Android — same as iOS, retiring the profile's WebView data folder

### Group and folder icons (2026-11-17)

What it does: a tab group or folder can show one of 20 symbols or one emoji in place of its letter
or folder glyph ("Change Icon…" in its menu); Clean Up gives each group it makes its category's icon.
Shared API: `browser_core::icon::{SYMBOLS, parse, set_group, set_folder}`, `Group::icon`,
`Folder::icon`; Clean Up through `browser_core::cleanup::apply`.
macOS reference: `crates/app/src/sidebar.rs` (`chosen_icon`, `icon_picker`, `group_avatar`),
`crates/app/src/groups.rs` (`begin_icon`, `set_icon`), `crates/app/src/browser/context.rs` (`Cmd::ChangeIcon`).

- [x] Windows — same code in `crates/app`; no Emoji & Symbols button, since GPUI cannot open the Windows emoji panel
- [x] Linux — `linux/src/ui.rs` `folder_icon`, `linux/src/ui/panels.rs` `icon_choice`, `linux/src/ui/groups.rs` `badge` and the group panel
- [ ] iOS — the tablet Mac layout shows folder and group icons and has Change Icon… with the 20 symbols and an
  emoji field (`mobile/app/src/ui/mac/sidebar.rs`, `tarn_nsh_shared::sidebar::{chosen_icon, group_avatar}`); the phone and
  Touch layouts have no folders or tab groups yet
- [ ] Android — same as iOS

### Clean Up (2026-11-16, reworked 2026-11-17)

What it does: the Clean Up button in the New Tab row sorts the space's loose tabs into tab groups by what each page
is (on this device: a known site decides, else host labels, path and title words), into the user's own
group when that group already holds two or more tabs of the same host and no other group does,
orders category groups and keeps tabs of one host together; Undo Sort in its toast puts the layout
back. It also archives tabs unused for longer than Settings › Tabs allows.
Shared API: `browser_core::cleanup::{plan, apply, undo, category}`. `apply` and `undo` each run one
`Store::apply_layout` transaction; `plan` and `category` take no `Store`. Archiving goes through
`Model::archive_unused`.
macOS reference: `crates/app/src/browser/context.rs` (`clean_up_tabs`, `undo_sort`), `crates/app/src/toast.rs`
(`Action::UndoSort`), `crates/app/src/sidebar.rs` (`ephemeral`).

- [x] Windows — same code in `crates/app`
- [x] Linux — `linux/src/ui.rs` (`new_tab_row`), `linux/src/ui/groups.rs` (group card and panel), `linux/src/browser/groups.rs` (`clean_up_tabs`, `undo_sort`, `ToastAction::UndoSort`), Settings › Tabs in `linux/src/ui/settings.rs`
- [ ] iOS — the tablet Mac layout sorts by category with Undo Sort in its toast
  (`mobile/app/src/ui/mac/sidebar.rs`, `browser_core::sidebar::{clean_up, undo_sort}`); archiving unused tabs and the
  phone and Touch layouts are still missing
- [ ] Android — same as iOS

### Tile grid (2026-11-16)

What it does: the tiles above the pinned tabs arrange themselves: one row while they fit, then as
few rows as fit, filled evenly (six tiles in a 256 pt sidebar make 3 × 2); Add to Tiles and
Remove from Tiles in a tab's menu.
Shared API: `browser_core::tabs::tile_columns`, `browser_core::tabs::Model::tile_toggle`.
macOS reference: `crates/app/src/sidebar.rs` (`tiles`), `crates/app/src/browser/context.rs` (`Cmd::ToggleTile`).

- [x] Windows — same code in `crates/app`
- [x] Linux — `linux/src/ui.rs` (`tiles`), `linux/src/ui/panels.rs` (This Tab panel)
- [ ] iOS — the tablet Mac layout has pinned tabs and the tile grid with Add to Tiles and Remove from Tiles
  (`mobile/app/src/ui/mac/sidebar.rs`); the phone and Touch layouts' tab sheet in `mobile/app/src/ui/panels.rs`
  still shows every tab as a card
- [ ] Android — same as iOS

### YouTube ad script and Netflix visibility script (2026-11-15)

What it does: on YouTube the ad placements are stripped from the page's own data before the
player reads them (Settings › Privacy, block YouTube ads); on Netflix the page always reports
itself visible, so playback does not pause when the window loses focus.
Shared API: `browser_core` `youtube_script` (the script the app builds), `engine::page_scripts::ALWAYS_VISIBLE`;
both run at document start in the page world, the YouTube one only where blocking is not paused
and never under Lockdown.
macOS reference: `crates/engine/src/data_store.rs` (`install_scripts`); Windows:
`crates/engine/src/chromium/view.rs` (`page_script_for`, `rescript`); Linux, whose web view loads
the bundle itself: `linux/src/webview/scripts.rs` (`apply_page_scripts`) with the script from
`filters::youtube_script` in `linux/src/webview/filters.rs` (`shield::youtube::script` of the
config `shield::groups::load` returns), switched by the `youtube` view prop.

- [x] Windows — Chromium `Page.addScriptToEvaluateOnNewDocument`
- [x] Linux — document-start page-world user script through WebKitGTK's user content manager in `linux/src/webview`
- [ ] iOS — page-world `WKUserScript` in `mobile/ios`
- [ ] Android — `WebViewCompat.addDocumentStartJavaScript` in `TarnWebView` (page world)
