use super::*;

impl LibraryWindow {
    pub(super) fn begin_rename(&mut self, named: Named, window: &mut Window, cx: &mut Context<Self>) {
        let title = match named {
            Named::Bookmark(id) => self.browser.upgrade().and_then(|b| b.read(cx).bookmarks.iter().find(|bm| bm.id == id).map(|bm| bm.title.clone())),
            Named::Snapshot(id) => self.snapshots.iter().find(|s| s.id == id).map(|s| s.name.clone()),
        };
        let Some(title) = title else { return };
        let input = cx.new(|cx| {
            let mut input = TextInput::new("Name", cx);
            input.set_text(title, cx);
            input.select_all_from_start(cx);
            input
        });
        let events = cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
            TextInputEvent::Submit => this.end_rename(true, window, cx),
            TextInputEvent::Cancel => this.end_rename(false, window, cx),
            TextInputEvent::MenuClosed if !window.is_window_active() && !this.holding => this.close(false, window, cx),
            _ => {}
        });
        let focus = input.focus_handle(cx);
        let blur = cx.on_blur(&focus, window, |this, window, cx| this.end_rename(true, window, cx));
        window.focus(&focus);
        self.renaming = Some((named, input, [events, blur]));
        cx.notify();
    }

    pub(super) fn end_rename(&mut self, commit: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((named, input, _)) = self.renaming.take() else { return };
        let title = input.read(cx).text().trim().to_owned();
        if commit && !title.is_empty() {
            match named {
                Named::Bookmark(id) => drop(self.browser.update(cx, |b, cx| b.rename_bookmark(id, &title, cx))),
                Named::Snapshot(id) => {
                    if let Some(saved) = self.snapshots.iter().find(|s| s.id == id).cloned() {
                        self.browser.update(cx, |b, _| b.rename_snapshot(&saved, &title)).ok();
                        self.reload_snapshots(cx);
                    }
                }
            }
        }
        window.focus(&self.query.focus_handle(cx));
        cx.notify();
    }

    pub(super) fn new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let parent = match self.page {
            Page::Bookmarks(folder) => folder,
            _ => None,
        };
        let Some(id) = self.browser.update(cx, |b, cx| b.new_bookmark_folder(parent, cx)).ok().flatten() else { return };
        self.go(Page::Bookmarks(parent), window, cx);
        self.select(Named::Bookmark(id), cx);
        self.begin_rename(Named::Bookmark(id), window, cx);
    }

    pub(super) fn delete(&mut self, named: Named, window: &mut Window, cx: &mut Context<Self>) {
        let id = match named {
            Named::Bookmark(id) => id,
            Named::Snapshot(id) => {
                let Some(saved) = self.snapshots.iter().find(|s| s.id == id).cloned() else { return };
                let message = format!("Delete the snapshot “{}”?", bounded_title(&saved.name));
                return self.confirm(&message, None, "Delete", window, cx, move |this, cx| {
                    this.browser.update(cx, |b, _| b.delete_snapshot(&saved)).ok();
                    this.reload_snapshots(cx);
                });
            }
        };
        let Some(browser) = self.browser.upgrade() else { return };
        let b = browser.read(cx);
        let Some(bm) = b.bookmarks.iter().find(|bm| bm.id == id) else { return };
        let message = if bookmarks::subtree(&b.bookmarks, id).len() == 1 {
            format!("Delete the bookmark “{}”?", bounded_title(&bm.title))
        } else {
            format!("Delete “{}” and everything in it?", bounded_title(&bm.title))
        };
        self.confirm(&message, None, "Delete", window, cx, move |this, cx| {
            this.browser.update(cx, |b, cx| b.delete_bookmark(id, cx)).ok();
        });
    }

    pub(super) fn remove_history(&mut self, url: String, cx: &mut Context<Self>) {
        self.browser.update(cx, |b, cx| b.remove_history_url(&url, cx)).ok();
        self.history.retain(|h| h.url != url);
        *self.history_hits.borrow_mut() = None;
        self.shown = self.rows(cx);
        cx.notify();
    }

    /// Delete, F2 and Shift+F10 act on the selected row; the delete keys only while the search
    /// field is empty, so they still edit its text.
    pub(super) fn on_key(&mut self, event: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        let Some((_, r)) = self.shown.get(self.selected).cloned() else { return };
        if self.renaming.is_some() || self.holding || event.is_held {
            return;
        }
        match key.key.as_str() {
            "f2" if !key.modifiers.modified() => match r.named {
                Some(named) => self.begin_rename(named, window, cx),
                None => return,
            },
            "f10" if key.modifiers.shift => self.row_menu(r.item.clone(), r.named, window, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    /// Deletes the selected row for Delete or ⌘⌫ while the search field is empty, before the
    /// field, which holds the focus, takes the key.
    pub(super) fn delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, r)) = self.shown.get(self.selected).cloned() else { return cx.propagate() };
        if self.renaming.is_some() || self.holding || !self.query.read(cx).text().is_empty() {
            return cx.propagate();
        }
        match (r.named, visit(&r)) {
            (Some(named), _) => self.delete(named, window, cx),
            (None, Some(url)) => self.remove_history(url.to_owned(), cx),
            _ => return cx.propagate(),
        }
        cx.stop_propagation();
    }

    pub(super) fn confirm(
        &mut self,
        message: &str,
        detail: Option<&str>,
        action: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
    ) {
        let answer = self.ask(PromptLevel::Warning, message, detail, &[action, "Cancel"], window, cx);
        cx.spawn_in(window, async move |this, cx| {
            let yes = answer.await == Ok(0);
            this.update_in(cx, |this, window, cx| {
                this.answered(window, cx);
                if yes {
                    then(this, cx);
                }
                window.focus(&this.query.focus_handle(cx));
            })
            .ok();
        })
        .detach();
    }

    /// A dialog draws inside the popover, so the popover takes its full height while one is up.
    pub(super) fn ask(
        &mut self,
        level: PromptLevel,
        message: &str,
        detail: Option<&str>,
        answers: &[&str],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> oneshot::Receiver<usize> {
        self.holding = true;
        if let Some(fit) = &self.fit {
            self.place(SEARCH_HEIGHT + 2. + fit.max_list, window, cx);
        }
        crate::dialog::ask(level, message, detail, answers, window, cx)
    }

    pub(super) fn answered(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.holding = false;
        self.refit(window, cx);
    }

    pub(super) fn alert(&mut self, message: &str, detail: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        let answer = self.ask(PromptLevel::Info, message, detail, &["OK"], window, cx);
        cx.spawn_in(window, async move |this, cx| {
            answer.await.ok();
            this.update_in(cx, |this, window, cx| {
                this.answered(window, cx);
                window.focus(&this.query.focus_handle(cx));
            })
            .ok();
        })
        .detach();
    }

    /// Asks which data to bring from a source into the current space's profile, then reads it in
    /// the background (where macOS may show its keychain prompt) and writes it.
    pub(super) fn start_import(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(source), Some(browser)) = (self.sources.get(ix).cloned(), self.browser.upgrade()) else { return };
        let b = browser.read(cx);
        let Some(space) = b.model.current_space() else { return };
        let (private, profile_id, core) = (space.private, space.space.profile_id, b.core.clone());
        if core.importing.get() {
            return self.alert("An import is already running.", None, window, cx);
        }
        if core.import_target(private, profile_id).is_err() {
            let detail = "Private and Lockdown profiles keep website data in memory only, so imported sign-ins would not last.";
            return self.alert("Switch to a space of a normal profile to import into it.", Some(detail), window, cx);
        }
        let name = source.browser.name();
        let profile = core.disk.profile(profile_id).map(|p| p.name).unwrap_or_default();
        let data::Confirm { message, detail, cookies } = data::confirm(&source, &profile, ui::controls::THIS_DEVICE);
        let sealed = !cookies;
        let buttons: &[&str] = if sealed { &["Import", "Cancel"] } else { &["Import All", "History and Bookmarks Only", "Cancel"] };
        let answer = self.ask(PromptLevel::Info, &message, Some(&detail), buttons, window, cx);
        let (weak_browser, main) = (self.browser.clone(), self.main);
        cx.spawn_in(window, async move |this, cx| {
            let cookies = match answer.await {
                Ok(0) => !sealed,
                Ok(1) if !sealed => false,
                _ => return drop(this.update_in(cx, |this, window, cx| this.answered(window, cx))),
            };
            if core.importing.replace(true) {
                this.update_in(cx, |this, window, cx| {
                    this.answered(window, cx);
                    this.alert("An import is already running.", None, window, cx);
                })
                .ok();
                return;
            }
            let kinds = Kinds { cookies, history: true, bookmarks: true };
            let browser_kind = source.browser;
            let read = cx.background_executor().spawn(async move { data::read(&source, kinds) }).await;
            let (tx, rx) = oneshot::channel();
            let summary = match core.import(profile_id, browser_kind, read, move |r| drop(tx.send(r))) {
                // The engine answers each cookie write; a store closed meanwhile may never answer.
                Ok(()) => match futures::future::select(rx, cx.background_executor().timer(IMPORT_TIMEOUT)).await {
                    futures::future::Either::Left((report, _)) => data::summary(name, report.ok().as_ref()),
                    futures::future::Either::Right(_) => data::summary(name, None),
                },
                Err(_) => "Nothing was imported: this profile can no longer take imported data.".into(),
            };
            core.importing.set(false);
            let same_profile = weak_browser
                .update(cx, |b, cx| {
                    b.reload_bookmarks(cx);
                    b.model.current_space().is_some_and(|s| !s.private && s.space.profile_id == profile_id)
                })
                .unwrap_or(false);
            this.update_in(cx, |this, window, cx| {
                this.answered(window, cx);
                if same_profile {
                    this.history = core.disk.history(profile_id, HISTORY_LOAD).unwrap_or_default();
                    this.history_hits.take();
                }
                if !window.is_window_active() {
                    this.close(false, window, cx);
                }
            })
            .ok();
            main.update(cx, |_, window, cx| drop(window.prompt(PromptLevel::Info, &format!("Import from {name}"), Some(&summary), &["OK"], cx))).ok();
        })
        .detach();
    }

    pub(super) fn run(&mut self, cmd: Cmd, window: &mut Window, cx: &mut Context<Self>) {
        match cmd {
            Cmd::Open(item) => self.open(item, window, cx),
            Cmd::OpenInNewTab(url) => {
                if let Ok(url) = url::Url::parse(&url) {
                    self.main
                        .update(cx, |b, window, cx| {
                            let before = b.model.active;
                            b.new_tab(&url, window, cx);
                            if b.model.active != before && b.is_bookmarked(url.as_str()) {
                                b.mark_typed(&url);
                            }
                        })
                        .ok();
                }
                self.close(true, window, cx);
            }
            Cmd::Rename(named) => self.begin_rename(named, window, cx),
            Cmd::Move(id, parent) => {
                self.browser.update(cx, |b, cx| b.move_bookmark(id, parent, cx)).ok();
            }
            Cmd::Delete(named) => self.delete(named, window, cx),
            Cmd::Restore(saved, new_space) => self.restore(saved, new_space, window, cx),
            Cmd::NewFolder => self.new_folder(window, cx),
            Cmd::ToggleBookmark(key) => {
                self.browser
                    .update(cx, |b, cx| match b.tab_url(key).is_some_and(|url| b.is_bookmarked(&url)) {
                        true => b.unbookmark_tab(Some(key), cx),
                        false => drop(b.bookmark_tab(Some(key), cx)),
                    })
                    .ok();
            }
            Cmd::CopyLink(url) => cx.write_to_clipboard(ClipboardItem::new_string(url)),
            Cmd::RemoveDownload(id) => self.remove_download(Some(id), cx),
            Cmd::RemoveHistory(url) => self.remove_history(url, cx),
        }
    }

    pub(super) fn row_menu(&mut self, item: Item, named: Option<Named>, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let Some(browser) = self.browser.upgrade() else { return };
        let b = browser.read(cx);
        let Some(host) = b.host.clone() else { return };
        let url = match &item {
            Item::Url(url) => Some(url.clone()),
            Item::Tab(key) => b.tab_url(*key),
            _ => None,
        };
        let item_key = item.clone();
        let history = match &item {
            Item::Url(url) if named.is_none() => Some(url.clone()),
            _ => None,
        };
        let mut items = match &item {
            Item::Download(d) => {
                let mut items = vec![MenuItem::action(ui::controls::REVEAL, Cmd::Open(item.clone()))];
                items.extend((!d.url.is_empty()).then(|| MenuItem::action("Copy Download Link", Cmd::CopyLink(d.url.clone()))));
                items.extend([MenuItem::Separator, MenuItem::action("Remove from List", Cmd::RemoveDownload(d.id))]);
                items
            }
            Item::Snapshot(saved) => vec![
                MenuItem::action("Restore in New Space", Cmd::Restore(saved.clone(), true)),
                MenuItem::action("Restore in This Space", Cmd::Restore(saved.clone(), false)),
                MenuItem::Separator,
                MenuItem::action("Rename", Cmd::Rename(Named::Snapshot(saved.id))),
                MenuItem::action("Delete…", Cmd::Delete(Named::Snapshot(saved.id))),
            ],
            _ => vec![MenuItem::action("Open", Cmd::Open(item))],
        };
        let bookmarked = url.as_deref().is_some_and(|url| b.is_bookmarked(url));
        if let Some(url) = url {
            items.extend([MenuItem::action("Open in New Tab", Cmd::OpenInNewTab(url.clone())), MenuItem::action("Copy Link", Cmd::CopyLink(url))]);
        }
        if let Item::Tab(key) = item_key {
            items.push(MenuItem::action(if bookmarked { "Remove Bookmark" } else { "Bookmark Tab" }, Cmd::ToggleBookmark(key)));
        }
        if let Some(url) = history {
            items.extend([MenuItem::Separator, MenuItem::action("Remove from History", Cmd::RemoveHistory(url))]);
        }
        if let Some(Named::Bookmark(id)) = named {
            let parent = b.bookmarks.iter().find(|bm| bm.id == id).and_then(|bm| bm.parent_id);
            let mut folders = vec![MenuItem::action("Bookmarks", Cmd::Move(id, None)).checked(parent.is_none()).enabled(parent.is_some())];
            folders.extend(
                bookmarks::move_targets(&b.bookmarks, id)
                    .into_iter()
                    .map(|f| MenuItem::action(bounded_title(&f.title), Cmd::Move(id, Some(f.id))).checked(parent == Some(f.id)).enabled(parent != Some(f.id))),
            );
            items.extend([
                MenuItem::Separator,
                MenuItem::action("Rename", Cmd::Rename(Named::Bookmark(id))),
                MenuItem::Submenu("Move to Folder".into(), folders),
                MenuItem::action("New Folder", Cmd::NewFolder),
                MenuItem::Separator,
                MenuItem::action("Delete", Cmd::Delete(Named::Bookmark(id))),
            ]);
        }
        self.holding = true;
        cx.spawn_in(window, async move |this, cx| {
            let (tx, rx) = oneshot::channel();
            menu::show(&host, crate::menu_glyphs::with_glyphs(items), None, move |cmd| {
                tx.send(cmd).ok();
            });
            let cmd = rx.await.ok();
            this.update_in(cx, |this, window, cx| {
                this.holding = false;
                match cmd {
                    Some(cmd) => this.run(cmd, window, cx),
                    None if !window.is_window_active() => this.close(false, window, cx),
                    None => {}
                }
            })
            .ok();
        })
        .detach();
    }
}
