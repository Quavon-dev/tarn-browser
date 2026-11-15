# Final re-checks (2026-11-14)

Docker on Apple silicon, Fedora 43, WebKitGTK 6.0 2.54, Xvfb, driven with
`linux/scripts/{env,app,go,click,shot}.sh`. Test-only, as in `../README.md`: sandbox off, software GL,
a container-made test certificate for `trusted.test` and `open.test` (both `127.0.0.1` in `/etc/hosts`)
trusted inside the container. `trusted.test` was set to Lockdown in `site_overrides` before launch.
`tests/pages/unload.html` requests `/pixel?ev=…` from `onpagehide`, `onbeforeunload`, `onunload` and a
button's `onclick`; `/slow` answers after 6 s.

| Check | Result | Evidence |
|---|---|---|
| Lockdown page | Lockdown badge, JavaScript off | `01-unload-page.png` |
| GET form from the Lockdown page to `open.test/slow`, "beacon" clicked before and twice during the load | Pass: no `/pixel?ev=` request. The target stays without JavaScript, as a page-started navigation from a Lockdown view does (unchanged) | `02-form-from-lockdown.*` |
| Typed `open.test/slow` while the Lockdown page is shown, "beacon" clicked twice during the load | Pass: no `/pixel?ev=` request; the target's script ran (`SLOW-JS-RAN`) | `03-typed-from-lockdown.*` |
| Image and frame from `127.0.0.1:8765/auth` (401) on a `localhost:8765` page | Pass: no prompt; `localhost:8765/auth` itself still prompts | `04-third-party-auth.*` |

The finding's case (a form in a frame turning JavaScript back on for a page whose JavaScript a frame's
form had turned off, since WebKitGTK does not say which frame navigates) was not built as a page; the
code no longer enables JavaScript in the policy decision at all (`decide_navigation`), only at commit.
