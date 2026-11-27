use super::*;

impl Browser {
    pub fn open_command(&mut self, mode: CommandMode, window: &mut Window, cx: &mut Context<Self>) {
        if self.in_main(window, cx, move |b, window, cx| b.open_command(mode, window, cx)) {
            return;
        }
        self.new_tab_group = None;
        self.overlay_exit = None;
        self.shortcuts_open = false;
        let text = match mode {
            CommandMode::Current => {
                self.active_view().and_then(|v| v.url()).map(String::from).or_else(|| self.model.active_tab().map(|t| t.url.clone())).unwrap_or_default()
            }
            CommandMode::NewTab | CommandMode::FindInTabs | CommandMode::Snippets => String::new(),
        };
        let placeholder = match mode {
            CommandMode::FindInTabs => "Find in this space's tabs",
            CommandMode::Snippets => "Insert a snippet",
            _ => "Search or enter address",
        };
        let private = self.model.private();
        self.history_cache = self.model.current_space().map(|s| s.space.profile_id).and_then(|p| self.store(private).history(p, 2000).ok()).unwrap_or_default();
        self.history_text = None;
        self.command_mode = Some(mode);
        if self.devtools_on() {
            self.refresh_local_servers(cx);
        }
        self.command.update(cx, |input, cx| {
            input.set_placeholder(placeholder);
            input.set_text(text, cx);
        });
        if self.model.active.is_some() && !self.profile_editor.as_ref().is_some_and(|e| !e.floating) {
            self.open_inline(window, cx);
        } else if let Some(view) = self.active_view() {
            view.set_visible(false);
        }
        if let Some(host) = &self.host {
            host.focus();
        }
        let focus = self.command.focus_handle(cx);
        window.focus(&focus);
        self.update_suggestions(cx);
        cx.notify();
    }

    /// F6 and Shift+F6: moves the keyboard between the page, the address field and the sidebar.
    pub(crate) fn cycle_focus(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        use browser_core::focus::{Region, next_region};
        let open = self.command_mode.is_some();
        let from = if open {
            Region::Address
        } else if self.focus.is_focused(window) {
            Region::Page
        } else {
            Region::Sidebar
        };
        match next_region(from, back, self.active_view().is_some(), self.sidebar_shown() > 0.) {
            Region::Address => self.open_command(CommandMode::Current, window, cx),
            Region::Page if open => self.close_command(window, cx),
            Region::Page => {
                window.focus(&self.focus);
                if let Some(view) = self.active_view() {
                    view.focus();
                }
            }
            Region::Sidebar => {
                if open {
                    self.close_command(window, cx);
                }
                if let Some(host) = &self.host {
                    host.focus();
                }
                window.focus(&self.focus);
                window.focus_next();
            }
        }
    }

    pub fn close_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let field = self.command_mode.is_some();
        let covered = field || self.shortcuts_open;
        self.launcher_from = None;
        self.actions_page = None;
        self.rejected = 0;
        self.close_inline(cx);
        self.overlay_exit = None;
        self.command_mode = None;
        self.history_text_search = None;
        self.profile_editor = None;
        self.shortcuts_open = false;
        self.tab_switcher = None;
        self.reader = self.reader.take().filter(|r| field || crate::reader::keeps_reader(covered, r.speaking()));
        window.focus(&self.focus);
        for key in self.model.active.and_then(|a| Some(self.splits.of(a)?.others(a))).unwrap_or_default() {
            if let Some(view) = self.views.get(&key) {
                view.set_visible(true);
            }
        }
        if let Some(view) = self.active_view().filter(|_| !self.overlay_open()) {
            view.set_visible(true);
            view.focus();
        }
        self.layout_changed();
        self.prepare_prompt(window, cx);
        cx.notify();
    }

    /// The command card and profile editor replace the page, so the native web view is hidden under
    /// them; the address field opened in the toolbar leaves it visible.
    pub fn overlay_open(&self) -> bool {
        self.card_open() || self.reader_shown()
    }

    /// An overlay other than the reader.
    pub(crate) fn card_open(&self) -> bool {
        (self.command_mode.is_some() && !self.inline_editing())
            || self.profile_editor.as_ref().is_some_and(|e| !e.floating)
            || self.shortcuts_open
            || self.switcher_shown()
    }

    pub(crate) fn reader_shown(&self) -> bool {
        self.reader.as_ref().is_some_and(|r| self.model.active == Some(r.key))
    }

    pub(crate) fn select_suggestion(&mut self, forward: bool, cx: &mut Context<Self>) {
        self.select_suggestion_at(suggest::step_selection(self.selected, self.suggestions.len(), forward), cx);
    }

    pub(crate) fn select_suggestion_at(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.selected = ix.min(self.suggestions.len().saturating_sub(1));
        self.suggestions_scroll.scroll_to_item(self.selected);
        cx.notify();
    }

    /// Tab, Page Up and Page Down move the highlight in the field's suggestion list.
    pub(crate) fn suggestion_key(&mut self, event: &gpui::KeyDownEvent, cx: &mut Context<Self>) {
        let last = self.suggestions.len().saturating_sub(1);
        match event.keystroke.key.as_str() {
            "tab" => self.select_suggestion(!event.keystroke.modifiers.shift, cx),
            "pageup" => self.select_suggestion_at(0, cx),
            "pagedown" => self.select_suggestion_at(last, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    pub(super) fn on_command_event(&mut self, _: &Entity<TextInput>, event: &TextInputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            TextInputEvent::Changed => {
                self.rejected = 0;
                if self.command_mode != Some(CommandMode::Snippets) {
                    self.expand_snippet(cx);
                }
                self.update_suggestions(cx)
            }
            TextInputEvent::Submit => self.submit_command(self.selected, window, cx),
            TextInputEvent::Cancel if self.actions_page.is_some() => self.leave_actions(cx),
            TextInputEvent::Cancel => self.dismiss_overlay(window, cx),
            TextInputEvent::PastEnd => self.open_actions(self.selected, cx),
            TextInputEvent::Up => self.select_suggestion(false, cx),
            TextInputEvent::Down => self.select_suggestion(true, cx),
            TextInputEvent::MenuClosed if !window.is_window_active() && self.inline_editing() => self.close_command(window, cx),
            TextInputEvent::MenuClosed => {}
        }
    }

    /// "Switch to" and "Move Tab to" rows for the other spaces, ranked like the command table.
    pub(super) fn space_commands(&self, query: &str) -> Vec<Suggestion> {
        let mut rows: Vec<(String, Box<dyn gpui::Action>)> = Vec::new();
        for (ix, s) in self.model.spaces.iter().enumerate().filter(|(ix, _)| *ix != self.model.space_ix) {
            rows.push((format!("Switch to {}", ui::text::bounded_title(&s.space.name)), Box::new(crate::menubar::SwitchToSpace { ix })));
        }
        if let Some(key) = self.model.active {
            for ix in self.model.move_targets(&self.core, key) {
                let name = ui::text::bounded_title(&self.model.spaces[ix].space.name);
                rows.push((format!("Move Tab to {name}"), Box::new(crate::menubar::MoveToSpace { ix })));
            }
        }
        command::rank_commands(query, rows.iter().map(|(label, _)| (label.as_str(), "space workspace")))
            .into_iter()
            .map(|ix| Suggestion { section: None, label: rows[ix].0.clone(), detail: String::new(), target: Target::Action(rows[ix].1.boxed_clone()) })
            .collect()
    }

    pub(super) fn update_suggestions(&mut self, cx: &mut Context<Self>) {
        self.suggest(None, cx);
    }

    /// What an actions page can act on for a row, if anything.
    pub(crate) fn row_item(&self, s: &Suggestion) -> Option<Item> {
        use browser_core::command::Section;
        match &s.target {
            Target::Tab(key) | Target::Find(key, _) => Some(Item::Tab(key.id)),
            Target::Url(u) if s.section == Some(Section::Favorites) => Some(Item::Bookmark(u.to_string())),
            Target::Url(u) => Some(Item::History(u.to_string())),
            Target::Input(Input::Command(ix)) => browser_core::palette::COMMANDS.get(*ix).map(|c| Item::Command(c.2)),
            _ => None,
        }
    }

    /// Lists the actions of row `ix` in place of the rows; Esc goes back to them.
    pub(crate) fn open_actions(&mut self, ix: usize, cx: &mut Context<Self>) {
        if !matches!(self.command_mode, Some(CommandMode::NewTab | CommandMode::Current)) || self.actions_page.is_some() {
            return;
        }
        let Some((row, item)) = self.suggestions.get(ix).and_then(|s| Some((s, self.row_item(s)?))) else { return };
        let title = row.label.clone();
        let spaces: Vec<String> = match item {
            Item::Tab(id) => {
                let key = TabKey { private: self.model.private(), id };
                self.model.move_targets(&self.core, key).into_iter().map(|ix| self.model.spaces[ix].space.name.clone()).collect()
            }
            _ => Vec::new(),
        };
        let names: Vec<&str> = spaces.iter().map(String::as_str).collect();
        let shortcut = match item {
            Item::Command(c) => crate::command_action(c).map(|a| crate::shortcuts::shown(a, cx)).unwrap_or_default(),
            _ => String::new(),
        };
        self.suggestions = browser_core::palette::actions(&item, &names)
            .into_iter()
            .map(|(label, action)| {
                let detail = if action == browser_core::palette::Action::Run { shortcut.clone() } else { String::new() };
                Suggestion { section: None, label, detail, target: Target::Act(item.clone(), action) }
            })
            .collect();
        self.actions_page = Some((title, ix));
        self.select_suggestion_at(0, cx);
    }

    pub(crate) fn leave_actions(&mut self, cx: &mut Context<Self>) {
        let Some((_, back)) = self.actions_page.take() else { return };
        self.suggest(self.suggested.clone(), cx);
        self.select_suggestion_at(back, cx);
    }

    /// Runs an action of the actions page, mostly as the row it stands for would run.
    pub(super) fn run_action(&mut self, ix: usize, item: Item, action: browser_core::palette::Action, window: &mut Window, cx: &mut Context<Self>) {
        use browser_core::palette::Action;
        let private = self.model.private();
        let url = match &item {
            Item::Tab(id) => self.tab_url(TabKey { private, id: *id }).map(|u| u.to_string()),
            Item::Bookmark(u) | Item::History(u) => Some(u.clone()),
            Item::Command(_) => None,
        };
        let target = match (item, action) {
            (Item::Command(c), Action::Run) => Target::Input(Input::Command(c.index())),
            (Item::Tab(id), Action::Switch) => Target::Tab(TabKey { private, id }),
            (Item::Tab(id), Action::Close) => {
                self.close_command(window, cx);
                return self.close_tab(TabKey { private, id }, window, cx);
            }
            (Item::Tab(id), Action::MoveToSpace(n)) => {
                let key = TabKey { private, id };
                let Some(space) = self.model.move_targets(&self.core, key).get(n).copied() else { return };
                self.close_command(window, cx);
                return self.move_to_space(key, space, window, cx);
            }
            (Item::History(u), Action::RemoveFromHistory) => {
                self.remove_history_url(&u, cx);
                self.actions_page = None;
                return self.suggest(None, cx);
            }
            (_, Action::CopyLink) => match url {
                Some(u) => Target::Copy(u),
                None => return,
            },
            (_, Action::OpenInNewTab) => {
                self.command_mode = Some(CommandMode::NewTab);
                match url.and_then(|u| Url::parse(&u).ok()) {
                    Some(u) => Target::Url(u),
                    None => return,
                }
            }
            (_, Action::Open) => match url.and_then(|u| Url::parse(&u).ok()) {
                Some(u) => Target::Url(u),
                None => return,
            },
            _ => return,
        };
        self.actions_page = None;
        self.suggestions[ix].target = target;
        self.submit_command(ix, window, cx);
    }

    pub(crate) fn refresh_open_suggestions(&mut self, cx: &mut Context<Self>) {
        if self.command_mode.is_some()
            && self.actions_page.is_none()
            && let Some(again) = self.suggested.clone()
        {
            self.suggest(Some(again), cx);
        }
    }

    /// Builds the suggestions for the field's text, or `again` for the query and completion they
    /// were last built for.
    pub(super) fn suggest(&mut self, again: Option<(String, Option<suggest::Completion>)>, cx: &mut Context<Self>) {
        let _span =
            (again.is_none() && self.command_mode != Some(CommandMode::FindInTabs)).then(|| engine::startup::span(engine::startup::PALETTE_QUERY)).flatten();
        let rebuilt = again.is_some();
        if rebuilt && self.actions_page.is_some() {
            return;
        }
        self.actions_page = None;
        let (query, kept) = again.map_or_else(|| (self.command.read(cx).text().trim().to_owned(), None), |(q, c)| (q, Some(c)));
        if !rebuilt {
            (self.history_text_search, self.suggested) = (None, None);
        }
        match self.command_mode {
            Some(CommandMode::FindInTabs) => return self.find_in_tabs(query, cx),
            Some(CommandMode::Snippets) => return self.snippet_rows(&query, cx),
            _ => {}
        }
        let site = self.site();
        let state = self.palette_state();
        let zappable = site.as_ref().is_some_and(|(k, u)| self.zappable(*k, u));
        let offered = |c: Command| {
            crate::command_action(c).is_some()
                && match c {
                    Command::ZapElement => zappable,
                    Command::SiteSettings => site.is_some(),
                    Command::ImportBrowserData => browser_core::import::available(),
                    _ => true,
                }
        };
        let commands = self.ranked_commands(&query, &state, offered);
        if browser_core::palette::sectioned(&query) {
            self.suggestions = self.sectioned_rows(&state, offered, cx);
            self.selected = usize::MAX;
            self.suggested = Some((query, None));
            cx.notify();
            return;
        }
        let space_rows = self.space_commands(&query);
        if query.starts_with('>') {
            self.suggestions = commands.into_iter().map(|c| command_row(c, cx)).chain(space_rows).collect();
            self.select_suggestion_at(0, cx);
            return;
        }
        let (mut out, exact) = self.typed_rows(&query, commands, space_rows, cx);
        let suggest_on = self.pref(crate::prefs::Pref::Suggestions);
        let mut pages: Vec<Page> = self.model.tabs.iter().map(Page::from).collect();
        if !query.is_empty() {
            pages.extend(self.history_cache.iter().map(Page::from));
        }
        let field = self.command.read(cx);
        let completion = kept.unwrap_or_else(|| {
            (suggest_on && !exact && suggest::may_complete(field.text(), field.inserted(), field.caret_at_end()))
                .then(|| suggest::inline_completion(&query, &pages, now()))
                .flatten()
        });
        if let Some(c) = &completion {
            out.insert(0, self.completion_row(&query, c));
        }
        let answers = browser_core::palette::answers(&query);
        let answered = answers.len();
        out.splice(
            0..0,
            answers.into_iter().map(|a| Suggestion {
                section: None,
                label: a.label,
                detail: browser_core::palette::ANSWER_DETAIL.to_owned(),
                target: Target::Copy(a.copy),
            }),
        );
        let typed = out.len();
        let (history, search_later) = self.history_rows(&query, &pages, &out, completion.as_ref().map(|c| c.url.as_str()), suggest_on);
        out.extend(history);
        out.truncate(if suggest_on { MAX_SUGGESTIONS } else { typed });
        // Late text matches move rows; the highlight stays on the row the user chose, if it is still listed.
        // An empty field highlights nothing, so Enter opens nothing until the user picks a row.
        let chosen = self.suggestions.get(self.selected).filter(|_| rebuilt).map(|s| (s.label.clone(), s.detail.clone()));
        self.suggestions = out;
        let first = if query.is_empty() { None } else { browser_core::palette::default_row(answered, self.suggestions.len()) }.unwrap_or(usize::MAX);
        self.selected = chosen.and_then(|(label, detail)| self.suggestions.iter().position(|s| s.label == label && s.detail == detail)).unwrap_or(first);
        self.suggested = Some((query.clone(), completion.clone()));
        self.history_text_search = search_later.map(|profile| self.search_history_text_later(profile, query, cx));
        if let (Some(c), false) = (completion, rebuilt) {
            self.command.update(cx, |input, cx| input.append_selected(&c.suffix, cx));
        }
        cx.notify();
    }

    fn palette_state(&self) -> browser_core::palette::State {
        let tab = self.model.active_tab();
        let idle = |a: &dyn gpui::Action| self.idle(a);
        browser_core::palette::State {
            pinned: tab.is_some_and(|t| t.kind != store::TabKind::Ephemeral),
            split: self.model.active.is_some_and(|a| self.splits.contains(a)),
            reopenable: !idle(&crate::ReopenClosedTab),
            duplicates: !self.duplicate_tabs().is_empty(),
            no_tab: tab.is_none(),
            no_page: self.model.active.and_then(|k| self.tab_url(k)).is_none(),
            private: self.model.private(),
            muted: self.model.active.filter(|_| !idle(&crate::menubar::ToggleMute)).map(|k| self.tab_muted(k)),
            custom_title: tab.is_some_and(|t| !t.custom_title.is_empty()),
            renamable: tab.is_some(),
            drifted: self.model.active.and_then(|k| self.pinned_drift(k)).is_some(),
            sleep_others: true,
            unpinned: !idle(&crate::CloseAllTabs),
            loading: !idle(&crate::StopLoading),
            toast: self.toast_command(),
            reader: Some(self.reader_shown()),
            developer: self.devtools_on(),
            assistant: self.assistant_allowed(),
            ..Default::default()
        }
    }

    fn ranked_commands(&self, query: &str, state: &browser_core::palette::State, offered: impl Fn(Command) -> bool) -> Vec<(String, Command)> {
        let mut commands: Vec<(String, Command)> = browser_core::palette::ranked(query, state, offered, &[])
            .into_iter()
            .filter_map(|(label, row)| match row {
                browser_core::palette::Row::Command(c) => Some((label, c)),
                browser_core::palette::Row::Extra(_) => None,
            })
            .collect();
        // Reading the switch target costs a profile and lock file read.
        if commands.iter().any(|(_, c)| matches!(c, Command::ReloadInChromium | Command::ReloadInWebKit)) {
            let to = self.model.active.and_then(|k| self.web_engine_switch(k));
            commands.retain(|(_, c)| match c {
                Command::ReloadInChromium => to == Some(WebEngine::Chromium),
                Command::ReloadInWebKit => to == Some(WebEngine::WebKit),
                _ => true,
            });
        }
        commands
    }

    fn sectioned_rows(&self, state: &browser_core::palette::State, offered: impl Fn(Command) -> bool, cx: &App) -> Vec<Suggestion> {
        let private = self.model.private();
        let data = browser_core::palette::Data {
            tabs: &self.model.tabs,
            active: self.model.active.map(|k| k.id),
            bookmarks: &self.bookmarks,
            history: &self.history_cache,
            now: now(),
        };
        let mut out = Vec::new();
        for (section, entries) in browser_core::palette::sections(state, offered, &data) {
            for entry in entries {
                let row = match entry.item {
                    Item::Command(c) => command_row((entry.label, c), cx),
                    Item::Tab(id) => Suggestion { section: None, label: entry.label, detail: entry.detail, target: Target::Tab(TabKey { private, id }) },
                    Item::Bookmark(url) | Item::History(url) => {
                        let Ok(url) = Url::parse(&url) else { continue };
                        Suggestion { section: None, label: entry.label, detail: entry.detail, target: Target::Url(url) }
                    }
                };
                out.push(Suggestion { section: Some(section), ..row });
            }
        }
        out
    }

    fn typed_rows(&self, query: &str, commands: Vec<(String, Command)>, space_rows: Vec<Suggestion>, cx: &App) -> (Vec<Suggestion>, bool) {
        let mut out = Vec::new();
        let shortcuts = self.pref(crate::prefs::Pref::SearchShortcuts);
        let default = &command::ENGINES[self.search_engine()];
        let engine = command::search_engine(query, default, shortcuts);
        let input = command::classify_input(query, default, shortcuts);
        if let Some(Input::Navigate(u)) = &input {
            out.extend(self.open_tabs_at(&self.clean_user_url(u.clone())));
        }
        let exact = commands.first().is_some_and(|(label, c)| label.eq_ignore_ascii_case(query) || c.label().eq_ignore_ascii_case(query));
        if exact {
            out.extend(commands.iter().cloned().map(|c| command_row(c, cx)));
        }
        if let Some(input) = input {
            let (label, detail) = match &input {
                Input::Navigate(u) => (u.to_string(), "Open".to_owned()),
                Input::Search(_) => (query.to_owned(), format!("Search {}", engine.name)),
                Input::External(u) => (u.to_string(), format!("Open in another app ({})", u.scheme())),
                Input::Rejected => (query.to_owned(), "This kind of address cannot be opened".to_owned()),
                Input::Command(_) => unreachable!("classify_input never names a command"),
            };
            out.push(Suggestion { section: None, label, detail, target: Target::Input(input) });
        }
        if !exact {
            out.extend(commands.into_iter().map(|c| command_row(c, cx)));
        }
        out.extend(space_rows);
        (out, exact)
    }

    fn completion_row(&self, query: &str, c: &suggest::Completion) -> Suggestion {
        let private = self.model.private();
        let shown = format!("{query}{}", c.suffix);
        // Enter opens the visited page the field completes to, or the tab already showing it.
        let (target, detail) = match self.model.tabs.iter().find(|t| t.url == c.url.as_str() && self.model.active != Some(TabKey { private, id: t.id })) {
            Some(t) => (Target::Tab(TabKey { private, id: t.id }), "Switch to Tab".to_owned()),
            None => {
                let detail = self.clean_user_url(c.url.clone()).as_str().trim_start_matches("https://").trim_end_matches('/').to_owned();
                (Target::Input(Input::Navigate(c.url.clone())), detail)
            }
        };
        Suggestion { section: None, label: ui::text::bounded_url(&shown), detail: ui::text::bounded_url(&detail), target }
    }

    fn history_rows(&self, query: &str, pages: &[Page], out: &[Suggestion], completed: Option<&str>, suggest_on: bool) -> (Vec<Suggestion>, Option<Id>) {
        let private = self.model.private();
        let tabs = &self.model.tabs;
        let listed: HashSet<TabKey> = out.iter().filter_map(|s| if let Target::Tab(k) = s.target { Some(k) } else { None }).collect();
        let skip = |ix: usize| {
            if ix < tabs.len() {
                let key = TabKey { private, id: tabs[ix].id };
                listed.contains(&key) || self.model.active == Some(key)
            } else {
                Some(pages[ix].url) == completed
            }
        };
        let mut ranked: Vec<usize> = suggest::rank_rows(query, pages, tabs.len(), skip, now());
        let search_text = !private && suggest::searches_text(query) && suggest_on && self.pref(crate::prefs::Pref::HistoryText);
        let text_profile = self.model.current_space().filter(|_| search_text).map(|s| s.space.profile_id);
        let cached = self.history_text.as_ref().filter(|(p, q, _)| Some(*p) == text_profile && q == query).map(|(_, _, found)| found.clone());
        let search_later = text_profile.filter(|_| cached.is_none());
        let found = cached.unwrap_or_default();
        // Text matches get up to half the room left; a page in both lists shows once, where it ranks if it stays there.
        let room = MAX_SUGGESTIONS.saturating_sub(out.len());
        let open: HashSet<&str> = tabs.iter().map(|t| t.url.as_str()).collect();
        let found: Vec<&HistoryEntry> = found.iter().filter(|f| !open.contains(f.url.as_str()) && Some(f.url.as_str()) != completed).collect();
        let top = &ranked[..ranked.len().min(room)];
        let extra = found.iter().filter(|f| !top.iter().any(|&ix| pages[ix].url == f.url)).count();
        ranked.truncate(room - extra.min(room / 2));
        let mut found: Vec<&HistoryEntry> = found.into_iter().filter(|f| !ranked.iter().any(|&ix| pages[ix].url == f.url)).collect();
        found.truncate(room - ranked.len());
        let history_row = |url: &str, title: &str| {
            Url::parse(url).ok().map(|target| Suggestion {
                section: None,
                label: ui::text::bounded_title(if title.is_empty() { url } else { title }),
                detail: ui::text::bounded_url(url.trim_start_matches("https://").trim_end_matches('/')),
                target: Target::Url(target),
            })
        };
        let mut rows = Vec::new();
        for ix in ranked {
            let page = pages[ix];
            if ix < tabs.len() {
                rows.push(Suggestion {
                    section: None,
                    label: ui::text::bounded_title(if page.title.is_empty() { page.url } else { page.title }),
                    detail: "Switch to Tab".to_owned(),
                    target: Target::Tab(TabKey { private, id: tabs[ix].id }),
                });
            } else {
                rows.extend(history_row(page.url, page.title));
            }
        }
        rows.extend(found.into_iter().filter_map(|h| history_row(&h.url, &h.title)));
        (rows, search_later)
    }

    pub(crate) fn forget_text_matches(&mut self) {
        (self.history_text, self.history_text_search) = (None, None);
    }

    pub(super) fn search_history_text_later(&self, profile: Id, query: String, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(suggest::TEXT_SEARCH_DELAY).await;
            this.update(cx, |b, cx| {
                let found = b.core.disk.search_history_text(profile, &query, MAX_TEXT_MATCHES).unwrap_or_default();
                b.history_text = Some((profile, query.clone(), found));
                if b.command_mode.is_some()
                    && let Some(again) = b.suggested.clone().filter(|(q, _)| *q == query)
                {
                    b.suggest(Some(again), cx);
                }
            })
            .ok();
        })
    }

    pub fn submit_command(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mode) = self.command_mode else { return };
        let Some(suggestion) = self.suggestions.get(ix) else { return };
        // A search's results page is not an address the user typed, so it earns no completion credit.
        let credit = !matches!(suggestion.target, Target::Input(Input::Search(_)));
        let url = match &suggestion.target {
            Target::Tab(key) => {
                let key = *key;
                self.close_command(window, cx);
                if self.model.tab(key).is_none()
                    && let Ok(tab) = self.store(key.private).tab(key.id)
                    && let Some(ix) = self.model.spaces.iter().position(|s| s.private == key.private && s.space.id == tab.space_id)
                {
                    self.switch_space(ix, window, cx);
                }
                self.activate(key, window, cx);
                return;
            }
            Target::Find(key, query) => {
                let (key, query) = (*key, query.clone());
                self.close_command(window, cx);
                return self.open_text_match(key, query, window, cx);
            }
            Target::Input(Input::Command(ix)) => {
                let Some((name, action)) = browser_core::palette::COMMANDS.get(*ix).and_then(|c| Some((c.0, crate::command_action(c.2)?))) else { return };
                let keys = crate::shortcuts::shown(action, cx);
                self.close_command(window, cx);
                window.dispatch_action(action.boxed_clone(), cx);
                cx.defer_in(window, move |b, window, cx| b.offer_hint(name, &keys, window, cx));
                return;
            }
            Target::Action(action) => {
                let action = action.boxed_clone();
                self.close_command(window, cx);
                window.dispatch_action(action, cx);
                return;
            }
            Target::Snippet(body) => {
                let body = body.clone();
                self.close_command(window, cx);
                return self.type_snippet(&body, cx);
            }
            Target::Act(item, action) => {
                let (item, action) = (item.clone(), *action);
                return self.run_action(ix, item, action, window, cx);
            }
            Target::Copy(text) => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.clone()));
                self.close_command(window, cx);
                return self.show_toast_as(browser_core::toast::Tone::Success, "Copied", None, cx);
            }
            Target::Url(u) => u.clone(),
            Target::Input(Input::Navigate(u)) => self.clean_user_load(u.clone(), cx),
            Target::Input(Input::Search(u)) => u.clone(),
            Target::Input(Input::External(u)) => {
                self.pending_external = Some(PendingExternal { tab: None, origin: None, url: u.clone() });
                self.layout_changed();
                self.close_command(window, cx);
                return;
            }
            Target::Input(Input::Rejected) => {
                self.rejected += 1;
                return cx.notify();
            }
        };
        let group = self.new_tab_group.take();
        self.close_command(window, cx);
        match mode {
            CommandMode::NewTab => {
                let before = self.model.active;
                let routed = self.open_routed(&url, window, cx);
                if !routed {
                    self.new_tab(&url, window, cx);
                    if credit && self.model.active != before {
                        self.mark_typed(&url);
                    }
                }
                if let Some((key, group)) = self.model.active.filter(|k| !routed && Some(*k) != before).zip(group) {
                    self.set_tab_group(key, Some(group), cx);
                }
            }
            CommandMode::Current | CommandMode::FindInTabs | CommandMode::Snippets => {
                self.navigate(&url, window, cx);
                if credit {
                    self.mark_typed(&url);
                }
            }
        }
    }

    pub(crate) fn mark_typed(&mut self, url: &Url) {
        if let Some(key) = self.model.active {
            self.pages.entry(key).or_default().typed = Some(url.to_string());
        }
    }

    /// "Switch to Tab" rows for open tabs showing `url`: the current space's first, then other
    /// spaces of the same profile and privacy, named in the detail.
    pub(super) fn open_tabs_at(&self, url: &Url) -> Vec<Suggestion> {
        self.model
            .tabs_showing(&self.core, url)
            .into_iter()
            .map(|(key, t, space)| Suggestion {
                section: None,
                label: ui::text::bounded_title(if t.title.is_empty() { &t.url } else { &t.title }),
                detail: browser_core::tabs::switch_detail(space.as_deref()),
                target: Target::Tab(key),
            })
            .collect()
    }

    pub fn resolve_external(&mut self, open: bool, cx: &mut Context<Self>) {
        if open && (self.pending_external.is_none() || !self.layout_stable()) {
            return;
        }
        if let Some(pending) = self.pending_external.take()
            && open
        {
            engine::open_external(&pending.url);
        }
        cx.notify();
    }

    pub fn dismiss_error(&mut self, cx: &mut Context<Self>) {
        if let Some(page) = self.model.active.and_then(|k| self.pages.get_mut(&k)) {
            page.error = None;
            page.notice = None;
            page.offer = None;
        }
        cx.notify();
    }
}

fn command_row((label, c): (String, Command), cx: &App) -> Suggestion {
    let detail = crate::command_action(c).map(|a| crate::shortcuts::shown(a, cx)).unwrap_or_default();
    Suggestion { section: None, label, detail, target: Target::Input(Input::Command(c.index())) }
}
