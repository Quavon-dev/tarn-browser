use super::*;

impl SettingsWindow {
    pub(super) fn permissions(&self, s: Snapshot, cx: &mut Context<Self>) -> AnyElement {
        let body = if s.permissions.is_empty() {
            note("No site has asked yet. When a site asks for the camera, microphone, location or pop-ups and you choose Remember, the answer shows here.")
        } else {
            let mut list = group();
            for (ix, p) in s.permissions.into_iter().enumerate() {
                let set = |allow: Option<bool>, origin: String, kind: String| {
                    cx.listener(move |this: &mut Self, _, _, cx| {
                        let (origin, kind) = (origin.clone(), kind.clone());
                        this.with_browser(cx, move |b, _, cx| b.set_profile_permission(&origin, &kind, allow, cx));
                    })
                };
                list = list.child(
                    row(ix == 0)
                        .child(text_block(bounded_title(&p.site), p.label.clone()))
                        .child(button(("allow", ix), "Allow", p.allowed).on_click(set(Some(true), p.origin.clone(), p.kind.clone())))
                        .child(button(("block", ix), "Block", !p.allowed).on_click(set(Some(false), p.origin.clone(), p.kind.clone())))
                        .child(button(("forget", ix), "Forget", false).on_click(set(None, p.origin, p.kind))),
                );
            }
            list
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(note("This profile's answers to sites. A site you forget asks again next time; Lockdown sites never ask."))
            .child(body)
            .into_any_element()
    }

    pub(super) fn shield_levels(&self, s: &Snapshot, cx: &mut Context<Self>) -> Div {
        let level = s.profile.as_ref().map(|p| p.shield_level).unwrap_or_default();
        let name = s.profile.as_ref().map(|p| bounded_title(&p.name)).unwrap_or_default();
        ShieldLevel::ALL.into_iter().enumerate().fold(group(), |levels, (ix, l)| {
            let name = name.clone();
            levels.child(
                tab_stop(row(ix == 0).id(l.as_str()))
                    .cursor_pointer()
                    .hover(|s| s.bg(Theme::hover()))
                    .active(|s| s.bg(Theme::pressed()))
                    .child(radio(l == level))
                    .child(text_block(shield_badge(l), level_description(l)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if l != level {
                            this.set_level(l, &name, window, cx);
                        }
                    })),
            )
        })
    }

    pub(super) fn privacy(&self, s: Snapshot, cx: &mut Context<Self>) -> AnyElement {
        let target = s.profile.as_ref().map(|p| (s.private, p.id));
        let profile_name = s.profile.as_ref().map(|p| bounded_title(&p.name)).unwrap_or_default();

        let sites = if s.overrides.is_empty() {
            note("No site overrides. Use the Shield button in the address bar to set one for a site.")
        } else {
            let mut list = group();
            for (ix, (host, l)) in s.overrides.into_iter().enumerate() {
                list = list.child(
                    row(ix == 0).child(label(bounded_title(&host))).child(value(shield_badge(l))).child(
                        button(("remove-override", ix), "Remove", false)
                            .on_click(cx.listener(move |this, _, _, cx| this.with_browser(cx, |b, w, cx| b.remove_site_override(&host, w, cx)))),
                    ),
                );
            }
            list
        };

        let reset_zooms = (!s.zooms.is_empty()).then(|| {
            button("reset-zooms", "Reset All…", false).on_click(cx.listener(move |this, _, window, cx| {
                let Some((private, id)) = target else { return };
                this.ask("Reset every saved zoom level?", None, &["Reset All"], window, cx, move |this, _, cx| {
                    this.with_browser(cx, |b, _, cx| b.remove_site_zoom(private, id, None, cx))
                });
            }))
        });
        let zooms = if s.zooms.is_empty() {
            note(format!(
                "No saved zoom levels. {} and {} remember the zoom for the site; {} forgets it.",
                crate::shortcuts::shown(&crate::ZoomIn, cx),
                crate::shortcuts::shown(&crate::ZoomOut, cx),
                crate::shortcuts::shown(&crate::ZoomReset, cx)
            ))
        } else {
            let mut list = group();
            for (ix, (host, zoom)) in s.zooms.into_iter().enumerate() {
                list = list.child(row(ix == 0).child(label(bounded_title(&host))).child(value(format!("{:.0} %", zoom * 100.))).child(
                    button(("remove-zoom", ix), "Remove", false).on_click(cx.listener(move |this, _, _, cx| {
                        let Some((private, id)) = target else { return };
                        this.with_browser(cx, |b, _, cx| b.remove_site_zoom(private, id, Some(&host), cx));
                    })),
                ));
            }
            list
        };

        let zapped = if s.private {
            note("A private space keeps no hidden elements.")
        } else if s.zapped.is_empty() {
            note("No hidden elements. Zap Element in the command bar hides an element on a site in this profile.")
        } else {
            let mut list = group();
            for (ix, (host, selector)) in s.zapped.into_iter().enumerate() {
                list = list.child(row(ix == 0).child(label(bounded_title(&format!("{host} — {selector}")))).child(
                    button(("unzap", ix), "Show Again", false).on_click(cx.listener(move |this, _, _, cx| {
                        let (host, selector) = (host.clone(), selector.clone());
                        this.with_browser(cx, move |b, _, cx| b.unzap(&host, Some(&selector), cx));
                    })),
                ));
            }
            list
        };

        let muted = if s.muted.is_empty() {
            note("No muted sites. Mute Site in the Tabs menu or a tab's menu silences a site in this profile.")
        } else {
            let mut list = group();
            for (ix, host) in s.muted.into_iter().enumerate() {
                list = list.child(row(ix == 0).child(label(bounded_title(&host))).child(button(("unmute", ix), "Unmute", false).on_click(cx.listener(
                    move |this, _, _, cx| {
                        let Some((private, id)) = target else { return };
                        this.with_browser(cx, |b, _, cx| {
                            b.set_site_muted(private, id, Some(&host), false, true);
                            cx.notify();
                        });
                    },
                ))));
            }
            list
        };

        let count = |category: &str, unit: &str| match s.filters.counts.get(category) {
            Some(n) => format!(" {} {unit}.", thousands(*n)),
            None if filters::configured() => " Lists not installed yet.".into(),
            None => " Needs the filter lists, which this build does not download.".into(),
        };
        let on = |pref| s.prefs.get(pref);
        let blocking = group()
            .child(self.pref_row(true, Pref::BlockAds, "Block ads", format!("EasyList, AdGuard Base and uBlock filters stop ad requests and hide ad placeholders.{}", count("ads", "rules")), on(Pref::BlockAds), cx))
            .child(self.pref_row(false, Pref::BlockTrackers, "Block trackers", format!("EasyPrivacy and uBlock Privacy stop tracking requests.{}", count("trackers", "rules")), on(Pref::BlockTrackers), cx))
            .child(self.pref_row(false, Pref::BlockAnnoyances, "Block cookie banners", format!("EasyList Cookie and uBlock Cookie Notices hide consent pop-ups and stop the scripts that show them. Nothing is accepted or refused for you.{}", count("annoyances", "rules")), on(Pref::BlockAnnoyances), cx))
            .child(self.pref_row(false, Pref::YouTubeAds, "Block YouTube ads", "Removes ads from YouTube's player data before the player sees them and hides ad slots. YouTube changes often; if videos stop playing, turn this off.", on(Pref::YouTubeAds), cx))
            .child(self.pref_row(false, Pref::BlockPopups, "Block pop-ups and ad redirects", format!("Pages open windows only when you click, never to known pop-up ad sites, and cannot send you to one.{}", count("popups", "domains")), on(Pref::BlockPopups), cx))
            .child(self.pref_row(false, Pref::BlockMalware, "Block dangerous sites", format!("Malware, phishing and scam lists{}.{}", if cfg!(windows) { "" } else { ", plus WebKit's fraudulent-website check, which asks Apple's Safe Browsing service about hashed addresses" }, count("malware", "domains")), on(Pref::BlockMalware), cx))
            .child(self.pref_row(false, Pref::BlockAdult, "Block adult content", format!("Pornographic and other adult websites, with no way past the warning page.{}", count("adult", "domains")), on(Pref::BlockAdult), cx));
        let paused = if s.paused.is_empty() {
            note("Blocking runs on every site. Turn it off for one site with the Shield button in the address bar.")
        } else {
            let mut list = group();
            for (ix, host) in s.paused.into_iter().enumerate() {
                list = list.child(
                    row(ix == 0).child(label(bounded_title(&host))).child(
                        button(("resume-blocking", ix), "Turn On", false)
                            .on_click(cx.listener(move |this, _, _, cx| this.with_browser(cx, |b, w, cx| b.set_site_paused(&host, false, w, cx)))),
                    ),
                );
            }
            list
        };

        let updates = s.filters.updates;
        let filter_state = s.filters.summary();
        let connection = |first: bool, name: &'static str, detail: String| row(first).child(text_block(name, detail));
        let connections = group()
            .child(if filters::configured() {
                switch_row("filter-updates", true, text_block("Filter-list updates", format!("GitHub, once a day. {filter_state}")), updates)
                    .on_click(cx.listener(move |this, _, _, cx| this.with_browser(cx, |b, _, cx| b.set_filter_updates(!updates, cx))))
                    .into_any_element()
            } else {
                connection(true, "Filter-list updates", format!("GitHub, once a day. {filter_state}")).into_any_element()
            })
            .child({
                let on = s.extension_downloads;
                switch_row(
                    "extension-downloads",
                    false,
                    text_block(
                        "Extension and theme downloads",
                        "GitHub, only when you click Install, Check for updates or Show official extensions in Extensions or Appearance settings. GitHub learns your address and the repository.",
                    ),
                    on,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.with_browser(cx, |b, _, cx| b.set_extension_downloads(!on, cx))))
            })
            .child(connection(false, "Site icons", "The site you are on, or a server it already loaded from such as its CDN (without cookies), through the tab's web engine with its Shield rules; never under Lockdown.".into()))
            .child(connection(false, "Update checks", match (s.updater, s.update_checks) {
                (false, _) => "Not set up in this build; Tarn does not check for updates.".into(),
                (true, true) => "GitHub, once a day. Turn off in General.".into(),
                (true, false) => "Off; only Check for Updates in the Tarn menu or Check Now in General asks GitHub.".into(),
            }))
            .when(cfg!(feature = "translation"), |d| {
                d.child(connection(false, "Translation", "None. Pages are translated on this Mac with the language packs installed in System Settings; Tarn never downloads them.".into()))
            })
            .child(connection(false, "AI requests", format!("Not built yet; no page content leaves {}.", ui::controls::THIS_DEVICE)));

        let mut data = group();
        if !s.private {
            data = data.child(row(true).child(label("Export this profile's data to Downloads as JSON")).child(
                button("export-profile", "Export", false).on_click(cx.listener(move |this, _, _, cx| {
                    let Some((private, id)) = target else { return };
                    this.with_browser(cx, |b, _, cx| b.export_profile(private, id, cx));
                })),
            ));
        }
        let clear_name = profile_name.clone();
        let delete_name = profile_name.clone();
        data = data
            .child(
                row(s.private)
                    .child(text_block(
                        "Clear browsing history",
                        "The last hour or today removes visited pages; everything also removes site icons, zoom levels, muted sites, protected sites and hidden elements.",
                    ))
                    .child(button("clear-history", "Clear…", false).on_click(cx.listener(move |this, _, window, cx| {
                        let Some((private, id)) = target else { return };
                        let ranges = HistoryRange::ALL.map(HistoryRange::label);
                        let question = format!("Clear the browsing history of \u{201C}{clear_name}\u{201D}?");
                        this.ask(&question, Some("Choose how far back to clear."), &ranges, window, cx, move |this, ix, cx| {
                            this.with_browser(cx, |b, _, cx| b.clear_history(private, id, HistoryRange::ALL[ix], cx))
                        });
                    }))),
            )
            .child(
                row(false).child(label(format!("Delete everything in the profile \u{201C}{profile_name}\u{201D}"))).child(
                    button("delete-profile", "Delete…", false).on_click(cx.listener(move |this, _, window, cx| {
                        let Some((private, id)) = target else { return };
                        let question = format!("Delete everything in the profile \u{201C}{delete_name}\u{201D}?");
                        let detail = "Its spaces, tabs, history, cookies, site data and site settings are deleted. This cannot be undone.";
                        this.ask(&question, Some(detail), &["Delete Everything"], window, cx, move |this, _, cx| {
                            this.with_browser(cx, |b, w, cx| b.delete_profile_data(private, id, w, cx))
                        });
                    })),
                ),
            );

        div()
            .child(section("Blocking"))
            .child(blocking)
            .child(note("Applies to every profile. Malware, phishing and adult lists stop whole sites with a warning page; ad, tracker and cookie-banner lists act inside pages."))
            .child(section("Sites without ad, tracker and cookie-banner blocking"))
            .child(paused)
            .when(!cfg!(windows), |d| {
                d.child(section("Sign-in"))
                    .child(group().child(self.pref_row(true, Pref::Passkeys, "Passkeys and security keys", browser_core::settings::PASSKEYS_NOTE, on(Pref::Passkeys), cx)))
            })
            .child(section("Links from other apps"))
            .child(group().child(self.pref_row(true, Pref::LocalhostFromApps, "Open localhost links from other apps", format!("A link to {} (localhost, 127.0.0.1) from a terminal, editor or other app opens like a typed address, and the page it shows may lead elsewhere on your network as a typed one can. Off: Tarn refuses it, since a link in a mail could reach services running here; Open Anyway on the notice still opens it. Links to other addresses on your network are refused; Open Anyway on the notice opens them. Applies to every profile.", ui::controls::THIS_DEVICE), on(Pref::LocalhostFromApps), cx)))
            .child(section("Site overrides"))
            .child(sites)
            .child(heading("Site zoom levels", div().children(reset_zooms)))
            .child(zooms)
            .child(section("Muted sites"))
            .child(muted)
            .child(section("Hidden elements"))
            .child(zapped)
            .child(section("Connections Tarn makes"))
            .child(connections)
            .child(note("Pages you open connect wherever they link to; this list covers Tarn itself. GitHub sees your IP address when Tarn checks for filter lists; turning updates off keeps the installed lists."))
            .child(section("Your data"))
            .child(data)
            .child(note(s.notice.clone().unwrap_or_else(|| {
                let shared = if s.shared_data { " Cookies and site data are shared with another profile and are deleted there too." } else { "" };
                format!("Delete everything removes spaces, tabs, history, cookies, site data and site settings of this profile.{shared} Files you downloaded stay in Downloads.")
            })))
            .into_any_element()
    }
}

impl SettingsWindow {
    pub(super) fn developer(&self, s: Snapshot, cx: &mut Context<Self>) -> AnyElement {
        let on = s.developer;
        let dev_profile = s.profiles.iter().find(|p| p.is_dev).map(|p| bounded_title(&p.name));
        let servers = match &s.local_servers {
            None => note("Not checked yet."),
            Some(ports) if ports.is_empty() => note("Nothing is listening on the common dev ports (3000, 5173, 8080 and others)."),
            Some(ports) => {
                let mut list = group();
                for (ix, port) in ports.iter().copied().enumerate() {
                    list = list.child(
                        row(ix == 0).child(label(format!("localhost:{port}"))).child(
                            button(("open-server", ix), "Open", false)
                                .on_click(cx.listener(move |this, _, _, cx| this.with_browser(cx, |b, w, cx| b.open_local_server(port, w, cx)))),
                        ),
                    );
                }
                list
            }
        };
        div()
            .child(section("Developer tools"))
            .child(group().child(
                switch_row("developer-tools", true, label("Web Inspector, JSON viewer, responsive sizes and local servers"), on)
                    .on_click(cx.listener(move |this, _, _, cx| this.with_browser(cx, |b, w, cx| b.set_developer(!on, w, cx)))),
            ))
            .child(note("Applies to every profile; open pages reload. Right-click a page and choose Inspect Element; the toolbar gains responsive sizes and Clear Site Data."))
            .child(section("Developer profile"))
            .child(match dev_profile {
                Some(name) => group().child(row(true).child(label(name)).child(value("Created"))),
                None => group().child(row(true).child(label("A separate profile for development")).child(
                    button("create-dev-profile", "Create", false)
                        .on_click(cx.listener(|this, _, _, cx| this.with_browser(cx, |b, w, cx| b.create_dev_profile(w, cx)))),
                )),
            })
            .child(note("Web Inspector and the developer tools are always on there; caches are cleared after every page load and Reload bypasses the cache."))
            .child(heading(
                "Local servers",
                button("refresh-servers", "Refresh", false).on_click(cx.listener(|this, _, _, cx| this.with_browser(cx, |b, _, cx| b.refresh_local_servers(cx)))),
            ))
            .child(servers)
            .into_any_element()
    }
}
