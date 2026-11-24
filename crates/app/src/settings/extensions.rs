use super::*;
use ui::controls::{empty_state, leading_icon, row_action};

const OFF: &str = "Extension downloads are off in Privacy settings.";

impl SettingsWindow {
    pub(super) fn extensions(&self, s: Snapshot, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if !s.extensions_allowed {
            return div().pt_6().child(empty_state(Icon::Lock, "Extensions are off in private and Lockdown profiles.")).into_any_element();
        }
        let mut page = div();
        if cfg!(windows) {
            page = page.child(note("Windows runs Tarn extensions only; Safari and Chrome extensions need their own engine and are not available here."));
        }
        page = self.extension_notices(page, &s, cx);
        if let Some(sheet) = s.store_sheet {
            return self.store_confirm(page, sheet, cx);
        }
        if let Some(c) = s.consent {
            return self.consent_confirm(page, c, cx);
        }
        page = self.extension_progress(page, &s, cx);
        let access = self.extension_access(&s, cx);
        let installed = self.installed_extensions(s.extensions, cx);
        let downloads = s.extension_downloads;
        let add = self.add_extension(downloads, window, cx);
        page = page.child(section("Installed")).child(installed).child(section("Access")).child(access).child(section("Add")).child(add);
        if !downloads {
            page = page.child(note("Extension downloads are off in Privacy settings; installing from a file still works."));
        }
        if crate::extensions::catalog_configured() {
            page = self.official_extensions(page, s.catalog, downloads, cx);
        }
        page.child(note("Extensions run in WebKit tabs of this profile only, never in the private space or in Lockdown profiles, and never on sites set to Lockdown. An extension that may not use every site asks for each site when it first meets it. Open pages reload when an extension is turned on or off."))
            .into_any_element()
    }

    fn extension_notices(&self, mut page: Div, s: &Snapshot, cx: &mut Context<Self>) -> Div {
        if s.all_sites_notice {
            page = page.child(
                group().child(
                    row(true)
                        .child(text_block(
                            "Extensions you had turned on keep access to every site they list",
                            "Their \u{201C}Allow on every site\u{201D} switch is on. Turn it off to be asked per site instead.",
                        ))
                        .child(
                            button("all-sites-notice", "OK", false)
                                .on_click(cx.listener(|this, _, _, cx| this.with_browser(cx, |b, _, cx| b.dismiss_all_sites_notice(cx)))),
                        ),
                ),
            );
        }
        if !s.extension_notices.is_empty() {
            let mut list = group();
            for (ix, n) in s.extension_notices.iter().enumerate() {
                list = list.child(row(ix == 0).child(div().flex_1().text_color(Theme::warning()).child(n.clone())));
            }
            page = page.child(
                list.child(
                    row(false).child(div().flex_1()).child(
                        button("extension-notices", "Dismiss", false)
                            .on_click(cx.listener(|this, _, _, cx| this.with_browser(cx, |b, _, cx| b.dismiss_extension_notices(cx)))),
                    ),
                ),
            );
        }
        page
    }

    fn store_confirm(&self, page: Div, sheet: (crate::extensions::StoreToken, String, Vec<String>), cx: &mut Context<Self>) -> AnyElement {
        let (token, title, lines) = sheet;
        let card = group()
            .child(row(true).child(text_block(title, "From Tarn's signed official catalog.")))
            .children(lines.into_iter().map(|l| row(false).child(div().flex_1().child(l))))
            .child(
                row(false)
                    .child(div().flex_1())
                    .child(
                        button("store-cancel", "Cancel", true)
                            .on_click(cx.listener(|this, _, _, cx| this.with_browser(cx, |b, _, cx| b.cancel_store_offer(cx)))),
                    )
                    .child(button("store-install", "Install", false).on_click(cx.listener(move |this, _, _, cx| {
                        let token = token.clone();
                        this.with_browser(cx, move |b, _, cx| b.confirm_store_offer(&token, cx))
                    }))),
            );
        page.child(section("Confirm")).child(card).into_any_element()
    }

    fn consent_confirm(&self, page: Div, c: crate::extensions::Consent, cx: &mut Context<Self>) -> AnyElement {
        let list = |title: &'static str, items: &[String]| {
            (!items.is_empty()).then(|| text_block(title, items.iter().map(|i| crate::extensions::shown(i)).collect::<Vec<_>>().join(", ")))
        };
        let mut card = group()
            .child(row(true).child(text_block(format!("Install {} {}?", c.name, c.version), format!("The extension's own words. Source: {}", c.badge))))
            .children(list("Permissions", &c.apis).map(|d| row(false).child(d)))
            .children(list("Optional permissions", &c.optional_apis).map(|d| row(false).child(d)))
            .children(list("Sites it asks for (you are asked per site)", &c.hosts).map(|d| row(false).child(d)))
            .children(list("Optional sites", &c.optional_hosts).map(|d| row(false).child(d)))
            .children(list("Changes", &c.changes).map(|d| row(false).child(d)));
        if c.active_tab {
            card = card.child(row(false).child(note("Clicking its toolbar button gives it the page you are on, in that tab.")));
        }
        if c.unreviewed {
            card = card.child(row(false).child(div().flex_1().text_color(Theme::warning()).child(crate::extensions::UNREVIEWED)));
        }
        card = card.child(
            row(false)
                .child(div().flex_1())
                .child(
                    button("consent-cancel", "Cancel", true)
                        .on_click(cx.listener(|this, _, _, cx| this.with_browser(cx, |b, _, cx| b.cancel_extension_install(cx)))),
                )
                .child(
                    button("consent-install", "Install", false)
                        .on_click(cx.listener(|this, _, _, cx| this.with_browser(cx, |b, _, cx| b.confirm_extension_install(cx)))),
                ),
        );
        page.child(section("Confirm")).child(card).into_any_element()
    }

    fn extension_progress(&self, mut page: Div, s: &Snapshot, cx: &mut Context<Self>) -> Div {
        if !s.extension_choice.is_empty() {
            let mut list = group().child(row(true).child(label("The release offers several packages. Pick the one for Chromium or Safari:")));
            for (ix, name) in s.extension_choice.iter().enumerate() {
                list = list.child(
                    row(false).child(label(name.clone())).child(
                        button(("extension-asset", ix), "Install", false)
                            .on_click(cx.listener(move |this, _, _, cx| this.with_browser(cx, move |b, _, cx| b.choose_extension_asset(ix, cx)))),
                    ),
                );
            }
            list = list.child(
                row(false).child(div().flex_1()).child(
                    button("extension-asset-cancel", "Cancel", true)
                        .on_click(cx.listener(|this, _, _, cx| this.with_browser(cx, |b, _, cx| b.cancel_extension_install(cx)))),
                ),
            );
            page = page.child(list);
        }
        if let Some(busy) = s.extension_busy.clone() {
            page = page.child(
                group().child(
                    row(true).child(label(busy)).child(
                        button("install-cancel", "Cancel", false)
                            .on_click(cx.listener(|this, _, _, cx| this.with_browser(cx, |b, _, cx| b.cancel_extension_install(cx)))),
                    ),
                ),
            );
        }
        page
    }

    fn extension_access(&self, s: &Snapshot, cx: &mut Context<Self>) -> Div {
        group()
            .child(self.pref_row(
                true,
                Pref::ExtensionsEverySite,
                "Allow extensions on every site",
                "App and official extensions read and change every site they list without asking. Unverified extensions always ask per site.",
                s.prefs.get(Pref::ExtensionsEverySite),
                cx,
            ))
            .when(cfg!(target_os = "macos"), |g| {
                g.child(self.pref_row(
                    false,
                    Pref::ExtensionsBitwarden,
                    "Bitwarden integration",
                    "Lets Bitwarden's app extension unlock with Touch ID through the Bitwarden app, use the clipboard and offer passkeys on sites. Off, it runs like any other app extension.",
                    s.prefs.get(Pref::ExtensionsBitwarden),
                    cx,
                ))
            })
    }

    fn installed_extensions(&self, extensions: Vec<crate::extensions::Row>, cx: &mut Context<Self>) -> Div {
        if extensions.is_empty() {
            empty_state(Icon::Braces, "No extensions yet. Apps with a Safari web extension, such as Bitwarden from the Mac App Store, appear here.")
        } else {
            let mut list = group();
            for (ix, e) in extensions.into_iter().enumerate() {
                let on = e.enabled;
                let (install_id, app_path) = (e.install_id.clone(), e.app_path.clone());
                let mut actions = div().flex().flex_wrap().items_center().gap_2();
                if let (Some(all), Some(id)) = (e.all_sites, e.install_id.clone()) {
                    actions = actions.child(
                        tab_stop(div().id(("extension-all", ix)))
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .cursor_pointer()
                            .text_size(px(TEXT_SMALL))
                            .child(switch(all))
                            .child("Every site")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let id = id.clone();
                                this.with_browser(cx, move |b, w, cx| b.set_extension_all_sites(&id, !all, w, cx))
                            })),
                    );
                }
                if let Some(id) = e.install_id.clone() {
                    actions =
                        actions.child(row_action(("extension-forget", ix), Icon::Undo, "Forget Site Answers").on_click(cx.listener(move |this, _, _, cx| {
                            let id = id.clone();
                            this.with_browser(cx, move |b, w, cx| b.forget_extension_answers(&id, w, cx))
                        })));
                }
                if let Some(id) = e.install_id {
                    let question = format!("Remove the extension \u{201C}{}\u{201D}?", bounded_title(&e.name));
                    actions =
                        actions.child(row_action(("extension-remove", ix), Icon::Trash, "Remove\u{2026}").on_click(cx.listener(move |this, _, window, cx| {
                            let id = id.clone();
                            this.ask(&question, Some("Its files and the data it stored are deleted."), &["Remove"], window, cx, move |this, _, cx| {
                                this.with_browser(cx, move |b, _, cx| b.remove_extension(&id, cx))
                            });
                        })));
                }
                let r = row(ix == 0)
                    .child(leading_icon(Icon::Braces))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1p5()
                            .child(text_block(format!("{} \u{00B7} {}", e.name, e.badge), e.permissions).flex_initial())
                            .child(actions),
                    )
                    .child(tab_stop(div().id(("extension-switch", ix))).cursor_pointer().child(switch(on)).on_click(cx.listener(move |this, _, _, cx| {
                        let (install_id, app_path) = (install_id.clone(), app_path.clone());
                        this.with_browser(cx, move |b, w, cx| match (install_id, app_path) {
                            (Some(id), _) => b.set_extension(&id, !on, w, cx),
                            (None, Some(path)) => b.enable_app_extension(path, w, cx),
                            _ => {}
                        })
                    })));
                list = list.child(r);
            }
            list
        }
    }

    fn add_extension(&self, downloads: bool, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let input = self.repo_input.clone();
        let focused = input.focus_handle(cx).is_focused(window);
        group()
            .child(row(true).child(leading_icon(Icon::Code)).child(field(focused).flex_1().py_0p5().child(input)).child(available(
                button("install-github", "Install", false).when(downloads, |d| d.on_click(cx.listener(|this, _, _, cx| this.install_from_github(cx)))),
                downloads,
                OFF,
            )))
            .child(row(false).child(leading_icon(Icon::FileArchive)).child(label("A .zip or .crx file")).child(
                button("install-file", "Choose File\u{2026}", false).on_click(cx.listener(|this, _, _, cx| {
                    let paths = cx.prompt_for_paths(gpui::PathPromptOptions { files: true, directories: false, multiple: false, prompt: None });
                    let main = this.main;
                    cx.spawn(async move |_, cx| {
                        let Ok(Ok(Some(paths))) = paths.await else { return };
                        let Some(path) = paths.into_iter().next() else { return };
                        main.update(cx, |b, _, cx| {
                            if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip") || e.eq_ignore_ascii_case("crx")) {
                                b.install_extension(crate::extensions::Install::File(path), None, cx);
                            } else {
                                b.core.extensions.borrow_mut().notices.push("Choose a .zip or .crx extension file.".to_owned());
                                cx.notify();
                            }
                        })
                        .ok();
                    })
                    .detach();
                })),
            ))
            .child(
                row(false).child(leading_icon(Icon::Reload)).child(label("Installed from GitHub or the catalog")).child(available(
                    button("extension-updates", "Check for Updates", false)
                        .when(downloads, |d| d.on_click(cx.listener(|this, _, _, cx| this.with_browser(cx, |b, _, cx| b.check_extension_updates(cx))))),
                    downloads,
                    OFF,
                )),
            )
    }

    fn official_extensions(&self, mut page: Div, catalog: (Vec<crate::extensions::CatalogRow>, bool), downloads: bool, cx: &mut Context<Self>) -> Div {
        let (rows, old) = catalog;
        let mut official = group().child(
            row(true).child(leading_icon(Icon::ShieldCheck)).child(label("Reviewed and signed by Tarn")).child(available(
                button("catalog", "Show Official Extensions", false)
                    .when(downloads, |d| d.on_click(cx.listener(|this, _, _, cx| this.with_browser(cx, |b, _, cx| b.fetch_catalog(cx))))),
                downloads,
                OFF,
            )),
        );
        for (ix, e) in rows.into_iter().enumerate() {
            let id = e.id.clone();
            official = official.child(row(false).child(text_block(format!("{} {}", e.name, e.version), e.description)).child(if e.installed {
                value("Installed").into_any_element()
            } else {
                available(
                    button(("catalog-install", ix), "Install", false).when(downloads, |d| {
                        d.on_click(cx.listener(move |this, _, _, cx| {
                            let id = id.clone();
                            this.with_browser(cx, move |b, _, cx| b.install_official(&id, cx))
                        }))
                    }),
                    downloads,
                    OFF,
                )
                .into_any_element()
            }));
        }
        official = official.child(self.link_row(
            "extension-store",
            "Browse the store",
            "Official extensions with screenshots and search; Install in Tarn asks here first.",
            browser_core::extensions::STORE_URL,
            cx,
        ));
        page = page.child(section("Official")).child(official);
        if old {
            page = page.child(note("This list is more than 30 days old. Show official extensions to get the current one."));
        }
        page
    }

    fn profile_row(&self, s: &Snapshot, ix: usize, p: &Profile, deletable: bool, cx: &mut Context<Self>) -> gpui::Div {
        let id = p.id;
        let current = s.profile.as_ref().is_some_and(|c| c.id == id && c.is_private == p.is_private);
        row(ix == 0)
            .child(if p.is_private { icon(Icon::Person, 14., Theme::warning()).into_any_element() } else { profile_dot(&p.color, 12.).into_any_element() })
            .child(label(bounded_title(&p.name)))
            .when(current, |d| d.child(value("This window")))
            .when(p.is_private, |d| d.child(value("Private, not saved")))
            .when(p.is_dev, |d| d.child(value("Developer")))
            .child(value(shield_badge(p.shield_level)))
            .when(!p.is_private, |d| {
                d.child(
                    row_action(("edit-profile", ix), Icon::Pencil, "Edit…")
                        .on_click(cx.listener(move |this, _, _, cx| this.with_browser(cx, |b, w, cx| b.open_profile_editor(Some(id), w, cx)))),
                )
            })
            .when(!p.is_private && deletable, |d| {
                let question = format!("Delete the profile \u{201C}{}\u{201D}?", bounded_title(&p.name));
                d.child(row_action(("delete-profile", ix), Icon::Trash, "Delete…").on_click(cx.listener(move |this, _, window, cx| {
                    let detail = "Its spaces, tabs, history and website data are deleted. Website data another profile shares stays for that profile.";
                    this.ask(&question, Some(detail), &["Delete Profile"], window, cx, move |this, _, cx| {
                        this.with_browser(cx, |b, w, cx| b.delete_profile(id, w, cx))
                    });
                })))
            })
    }

    pub(super) fn profiles(&self, s: &Snapshot, cx: &mut Context<Self>) -> AnyElement {
        let deletable = s.profiles.iter().filter(|p| !p.is_private).count() > 1;
        let list = s.profiles.iter().enumerate().fold(group(), |list, (ix, p)| list.child(self.profile_row(s, ix, p, deletable, cx)));
        let rules = if s.space_rules.is_empty() {
            empty_state(Icon::Folder, "No rules. \u{201c}Always open in\u{201d} in a site's settings opens its new tabs in the space you pick.")
        } else {
            let mut list = group();
            for (ix, (id, pattern, space)) in s.space_rules.iter().cloned().enumerate() {
                list = list.child(
                    row(ix == 0).child(label(bounded_title(&pattern))).child(value(space)).child(
                        row_action(("delete-space-rule", ix), Icon::Trash, "Remove")
                            .on_click(cx.listener(move |this, _, _, cx| this.with_browser(cx, |b, _, cx| b.delete_space_rule(id, cx)))),
                    ),
                );
            }
            list
        };
        div()
            .child(heading(
                "Profiles",
                icon_text_button("new-profile", Icon::Plus, "New Profile…")
                    .on_click(cx.listener(|this, _, _, cx| this.with_browser(cx, |b, w, cx| b.open_profile_editor(None, w, cx)))),
            ))
            .child(list)
            .child(note(
                "Delete removes a profile with its spaces, tabs, history and website data. Website data another profile shares stays for that profile.",
            ))
            .child(section("Space Rules"))
            .child(rules)
            .into_any_element()
    }
}
