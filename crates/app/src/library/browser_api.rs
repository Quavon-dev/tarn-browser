use super::*;

impl Browser {
    pub(crate) fn reload_bookmarks(&mut self, cx: &mut Context<Self>) {
        self.bookmarks_of = self.model.profile();
        if let Some(space) = self.model.current_space() {
            self.set_bookmarks(self.store(space.private).bookmarks(space.space.profile_id).unwrap_or_default());
        }
        cx.notify();
    }

    pub fn is_bookmarked(&self, url: &str) -> bool {
        bookmarks::find(&self.bookmarks, url).is_some()
    }

    fn bookmark_target(&self, key: Option<TabKey>) -> Option<(String, String)> {
        let tab = key.or(self.model.active).and_then(|k| self.model.tab(k))?;
        let url = self.tab_url(self.model.key(tab)).unwrap_or_else(|| tab.url.clone());
        (!url.is_empty()).then(|| (browser_core::text::stored_title_or_host(&tab.title, &url), url))
    }

    /// The top-level folder Save for Later files pages into, if it exists.
    pub(crate) fn reading_list(&self) -> Option<Id> {
        bookmarks::reading_list(&self.bookmarks)
    }

    /// Files the active page into the Reading List bookmark folder, made on first use, so it is
    /// kept, exported and deleted like any bookmark.
    pub fn save_for_later(&mut self, cx: &mut Context<Self>) {
        let (Some((title, url)), Some(space)) = (self.bookmark_target(None), self.model.current_space()) else { return };
        let (private, profile_id) = (space.private, space.space.profile_id);
        self.reload_bookmarks(cx);
        let saved = bookmarks::save_for_later(self.store(private), profile_id, &self.bookmarks, &url, &title, now());
        self.reload_bookmarks(cx);
        let (tone, text) = browser_core::toast::reading_list(saved);
        self.show_toast_as(tone, text, None, cx);
    }

    /// Bookmarks the tab, or returns its bookmark when it already has one.
    pub fn bookmark_tab(&mut self, key: Option<TabKey>, cx: &mut Context<Self>) -> Option<Id> {
        let (title, url) = self.bookmark_target(key)?;
        let space = self.model.current_space()?;
        let id = bookmarks::bookmark(self.store(space.private), space.space.profile_id, &self.bookmarks, &url, &title, now())?;
        self.reload_bookmarks(cx);
        Some(id)
    }

    pub fn unbookmark_tab(&mut self, key: Option<TabKey>, cx: &mut Context<Self>) {
        let Some((_, url)) = self.bookmark_target(key) else { return };
        bookmarks::unbookmark(self.store(self.model.private()), &self.bookmarks, &url);
        self.reload_bookmarks(cx);
    }

    /// Saves every tab into a new dated folder and opens the library on its name field.
    pub fn bookmark_all_tabs(&mut self, cx: &mut Context<Self>) {
        let Some(space) = self.model.current_space().filter(|_| !self.model.tabs.is_empty()) else { return };
        let pages: Vec<(&str, String)> = self
            .model
            .tabs
            .iter()
            .filter(|t| !t.url.is_empty())
            .map(|t| (t.title.as_str(), self.tab_url(self.model.key(t)).unwrap_or_else(|| t.url.clone())))
            .collect();
        let Some(folder) = bookmarks::bookmark_all(self.store(space.private), space.space.profile_id, pages.iter().map(|(t, u)| (*t, u.as_str())), now())
        else {
            return;
        };
        self.reload_bookmarks(cx);
        if let Some(main) = self.peek.main {
            cx.defer(move |cx| {
                main.update(cx, |b, window, cx| b.show_library(Some(folder), window, cx)).ok();
            });
        }
    }

    pub fn new_bookmark_folder(&mut self, parent: Option<Id>, cx: &mut Context<Self>) -> Option<Id> {
        let space = self.model.current_space()?;
        let id = bookmarks::new_folder(self.store(space.private), space.space.profile_id, parent, now())?;
        self.reload_bookmarks(cx);
        Some(id)
    }

    pub fn rename_bookmark(&mut self, id: Id, title: &str, cx: &mut Context<Self>) {
        self.store(self.model.private()).rename_bookmark(id, title).ok();
        self.reload_bookmarks(cx);
    }

    pub fn move_bookmark(&mut self, id: Id, parent: Option<Id>, cx: &mut Context<Self>) {
        self.store(self.model.private()).move_bookmark(id, parent).ok();
        self.reload_bookmarks(cx);
    }

    pub fn delete_bookmark(&mut self, id: Id, cx: &mut Context<Self>) {
        self.store(self.model.private()).delete_bookmark(id).ok();
        self.reload_bookmarks(cx);
    }

    /// Switches to a tab already showing `url`, otherwise opens it in a new one.
    pub fn open_url(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.command_mode.is_some() {
            self.close_command(window, cx);
        }
        if let Some(key) = self.model.tabs.iter().find(|t| t.url == url).map(|t| self.model.key(t)) {
            return self.activate(key, window, cx);
        }
        if let Ok(url) = url::Url::parse(url) {
            let before = self.model.active;
            self.new_tab(&url, window, cx);
            if self.model.active != before && self.is_bookmarked(url.as_str()) {
                self.mark_typed(&url);
            }
        }
    }

    /// Saves the current space's tabs as a snapshot named after it and opens the library on its name.
    pub fn save_snapshot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(space) = self.model.current_space() else { return };
        let name = format!("{} {}", space.space.name, date(now()));
        if let Some(saved) = snapshot::save(self.store(space.private), space, &name, now()) {
            self.open_library(Page::Snapshots, Some(Named::Snapshot(saved.id)), window, cx);
        }
    }

    /// The current profile's snapshots, newest first; a private space has none.
    pub(crate) fn snapshots(&self) -> Vec<Snapshot> {
        self.model.current_space().filter(|s| !s.private).and_then(|s| self.store(false).snapshots(s.space.profile_id).ok()).unwrap_or_default()
    }

    pub fn rename_snapshot(&mut self, saved: &Snapshot, name: &str) {
        self.store(false).rename_snapshot(saved.profile_id, saved.id, name).ok();
    }

    pub fn delete_snapshot(&mut self, saved: &Snapshot) {
        self.store(false).delete_snapshot(saved.profile_id, saved.id).ok();
    }

    /// Opens the snapshot's tabs in a new space of its profile, or at the end of the current space.
    pub fn restore_snapshot(&mut self, saved: &Snapshot, new_space: bool, window: &mut Window, cx: &mut Context<Self>) -> Result<(), &'static str> {
        let Some(current) = self.model.current_space().filter(|s| !s.private && s.space.profile_id == saved.profile_id).map(|s| s.space.id) else {
            return Err("Switch to a space of this snapshot's profile to restore it.");
        };
        let store = self.store(false);
        let snapshot = SpaceSnapshot::load(store, saved.profile_id, saved.id).map_err(snapshot_error)?;
        let target = if new_space {
            let names = std::iter::once(saved.name.clone()).chain((2..50).map(|n| format!("{} {n}", saved.name)));
            let name = names.filter_map(|n| crate::spaces::space_name(&n, &self.model.spaces, false, Some(saved.profile_id), None)).next();
            let space = store.create_space(saved.profile_id, name.as_deref().unwrap_or("Snapshot"), "");
            space.map_err(|_| "Tarn could not create a space for this snapshot.")?.id
        } else {
            current
        };
        let last = snapshot.restore(store, target);
        if new_space {
            self.reload_spaces();
            if let Some(ix) = self.model.spaces.iter().position(|s| !s.private && s.space.id == target) {
                self.switch_space(ix, window, cx);
            }
        } else {
            self.reload_space();
        }
        if let Some(id) = last {
            self.activate(TabKey { private: false, id }, window, cx);
        }
        Self::sync_other_windows(cx);
        cx.notify();
        Ok(())
    }

    pub(crate) fn downloads(&self) -> Vec<Download> {
        self.model.current_space().and_then(|s| self.store(s.private).downloads(s.space.profile_id).ok()).unwrap_or_default()
    }

    /// Removes one entry, or with `None` the whole list; the files stay.
    pub fn remove_download(&mut self, id: Option<Id>) {
        let Some(space) = self.model.current_space() else { return };
        let (store, profile_id) = (self.store(space.private), space.space.profile_id);
        match id {
            Some(id) => store.delete_download(profile_id, id),
            None => store.clear_downloads(profile_id),
        }
        .ok();
    }

    pub(crate) fn close_library(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.library.take() {
            cx.defer(move |cx| drop(handle.update(cx, |library, window, cx| library.close(true, window, cx))));
            cx.notify();
        }
    }

    pub fn toggle_library(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.library {
            Some(_) => self.close_library(cx),
            // The click that took focus from the library already closed it; it must not reopen it.
            None if self.library_closed.is_some_and(|t| t.elapsed() < ui::motion::REOPEN_GUARD) => {}
            None => self.show_library(None, window, cx),
        }
    }

    /// Opens the library above the space row; with `edit`, on that bookmark with its name field open.
    pub fn show_library(&mut self, edit: Option<Id>, window: &mut Window, cx: &mut Context<Self>) {
        let page = match edit.and_then(|id| self.bookmarks.iter().find(|b| b.id == id)) {
            Some(bm) => Page::Bookmarks(bm.parent_id),
            None => Page::Home,
        };
        self.open_library(page, edit.map(Named::Bookmark), window, cx);
    }

    /// Opens the library on `page`; with `edit`, that row's name field is open.
    pub(crate) fn open_library(&mut self, page: Page, edit: Option<Named>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(handle) = self.library {
            cx.defer(move |cx| drop(handle.update(cx, |library, window, cx| library.go(page, window, cx))));
            return;
        }
        let Some(main) = window.window_handle().downcast::<Browser>().or(self.peek.main) else { return };
        let inset = SIDEBAR_MARGIN;
        self.peek.owns[0] = self.peek.is_window(window);
        if !self.peek.owns[0] {
            self.close_peek(cx);
        }
        let anchor = self.main_bounds(window);
        let room = f32::from(anchor.size.height) - 2. * inset - SPACE_ROW - TOOLBAR_HEIGHT - SEARCH_HEIGHT - 2.;
        let max_list = room.max(ROW * 3.);
        let downloads = self.downloads();
        let snapshots = self.snapshots();
        let sources = if page == Page::Import { detect_sources() } else { Vec::new() };
        let history = self.model.current_space().and_then(|s| self.store(s.private).history(s.space.profile_id, HISTORY_LOAD).ok()).unwrap_or_default();
        let rows: Vec<&HistoryEntry> = history.iter().collect();
        let listed = entries(self, page, "", &rows, &downloads, &snapshots, &sources);
        let height = px(card_height(&listed, row_boundary(&listed, max_list)));
        let (x, bottom, width) = (inset + 4., f32::from(anchor.size.height) - inset - SPACE_ROW, CARD_WIDTH.min(self.sidebar_width - 8.));
        let bounds = Bounds::new(point(anchor.origin.x + px(x), anchor.origin.y + px(bottom) - height), size(px(width), height));
        let display = window.display(cx).map(|d| d.id());
        let browser = cx.entity().downgrade();
        // The new window renders from the browser at once, so it must open after this update ends.
        cx.defer(move |cx| {
            let handle = cx.open_window(popup_options(bounds, display, true), |window, cx| {
                if let Some(host) = engine::Host::from_window(window) {
                    host.hide_on_deactivate();
                    host.round_corners(0., 0., 0., 0., f64::from(ui::theme::Theme::radius_panel()));
                }
                cx.new(|cx| LibraryWindow {
                    page,
                    fit: Some(Fit { x, bottom, width, max_list }),
                    sources: sources.clone(),
                    ..LibraryWindow::new(browser.clone(), main, history, downloads.clone(), edit, window, cx)
                })
            });
            browser
                .update(cx, |b, cx| {
                    b.library = handle.ok();
                    cx.notify();
                })
                .ok();
            crate::peek::popovers_over_peek(&browser, cx);
        });
    }
}
