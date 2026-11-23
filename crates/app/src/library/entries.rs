use super::*;

pub(super) fn row(item: Item, title: String, bookmark: Option<Id>) -> Entry {
    Entry::Row(Row { item, title, named: bookmark.map(Named::Bookmark), detail: None })
}

pub(super) fn bookmark_row(bm: &Bookmark) -> Entry {
    let item = match &bm.url {
        Some(url) => Item::Url(url.clone()),
        None => Item::Folder(bm.id),
    };
    row(item, title_or_host(&bm.title, bm.url.as_deref().unwrap_or_default()), Some(bm.id))
}

pub(super) fn entries(
    b: &Browser,
    page: Page,
    query: &str,
    history: &[&HistoryEntry],
    downloads: &[Download],
    snapshots: &[Snapshot],
    sources: &[Source],
) -> Vec<Entry> {
    let matches = |title: &str, url: &str| library::matches(query, title, url);
    let store = b.store(b.model.private());
    let bookmarks = &b.bookmarks;
    let found = |limit| library::bookmarks(bookmarks, query).take(limit).map(bookmark_row);
    let visits = history;
    let history = |limit| visits.iter().take(limit).map(|h| row(Item::Url(h.url.clone()), title_or_host(&h.title, &h.url), None));
    let downloads = |limit| {
        let now = now();
        library::downloads(downloads, query).take(limit).map(move |d| download_row(store, d, now))
    };
    let mut out = Vec::new();
    match page {
        Page::Home => {
            let tabs = library::open_tabs(&b.model.tabs, query);
            if query.is_empty() || !tabs.is_empty() {
                out.push(Entry::Header("Open Tabs", Some(tabs.len())));
            }
            if query.is_empty() && tabs.is_empty() {
                out.push(Entry::Empty("No open tabs".into()));
            }
            out.extend(tabs.into_iter().take(MAX_ROWS).map(|t| row(Item::Tab(b.model.key(t)), title_or_host(&t.title, &t.url), None)));
            if !query.is_empty() {
                let sections: [(&str, Vec<Entry>); 3] =
                    [("Bookmarks", found(MAX_ROWS).collect()), ("History", history(MAX_ROWS).collect()), ("Downloads", downloads(MAX_ROWS).collect())];
                for (name, rows) in sections.into_iter().filter(|(_, rows)| !rows.is_empty()) {
                    out.push(Entry::Header(name, None));
                    out.extend(rows);
                }
                if out.is_empty() {
                    out.push(Entry::Empty(empty_text(page, query)));
                }
            }
            out.push(Entry::Header("Library", None));
            out.push(row(Item::Page(Page::History), "History".into(), None));
            out.push(row(Item::Page(Page::Bookmarks(None)), "Bookmarks".into(), None));
            if let Some(folder) = b.reading_list() {
                out.push(row(Item::Page(Page::Bookmarks(Some(folder))), bookmarks::READING_LIST.into(), None));
            }
            out.push(row(Item::Page(Page::Downloads), "Downloads".into(), None));
            if !b.model.private() {
                out.push(row(Item::Page(Page::Snapshots), "Snapshots".into(), None));
            }
            out.push(row(Item::Page(Page::Memory), "Memory".into(), None));
            if query.is_empty() && data::available() {
                out.push(row(Item::Page(Page::Import), "Import Browser Data".into(), None));
            }
        }
        Page::History => {
            out.push(Entry::Back("History".into()));
            let now = now();
            let starts = library::day_starts(store, now);
            let mut last = None;
            for h in visits.iter().take(MAX_LIST) {
                let day = library::history_day(h.visited_at, starts);
                if last.replace(day) != Some(day) {
                    out.push(Entry::Header(day, None));
                }
                out.push(visit_row(store, h, now));
            }
        }
        Page::Bookmarks(folder) => {
            let name = folder.and_then(|id| bookmarks.iter().find(|bm| bm.id == id)).map_or("Bookmarks".into(), |bm| bounded_title(&bm.title));
            out.push(Entry::Back(name));
            if query.is_empty() {
                out.extend(bookmarks.iter().filter(|bm| bm.parent_id == folder).map(bookmark_row));
            } else {
                out.extend(found(MAX_LIST));
            }
        }
        Page::Downloads => {
            out.push(Entry::Back("Downloads".into()));
            out.extend(downloads(MAX_LIST));
            if out.len() > 1 && query.is_empty() {
                out.push(Entry::Divider);
                out.push(row(Item::ClearDownloads, "Clear List".into(), None));
            }
        }
        Page::Snapshots => {
            out.push(Entry::Back("Snapshots".into()));
            if snapshots.is_empty() {
                out.push(Entry::Empty("Save a space with “Save Space as Snapshot”".into()));
            }
            let now = now();
            out.extend(snapshots.iter().filter(|s| matches(&s.name, "")).map(|s| {
                let detail = Some(ago(store, s.created_at, now));
                Entry::Row(Row { item: Item::Snapshot(s.clone()), title: bounded_title(&s.name), named: Some(Named::Snapshot(s.id)), detail })
            }));
        }
        Page::Memory => memory_entries(b, query, &mut out),
        Page::Import => {
            out.push(Entry::Back("Import Browser Data".into()));
            if sources.is_empty() {
                out.push(Entry::Empty("No supported browser found".into()));
            }
            out.extend(
                sources.iter().enumerate().filter(|(_, s)| matches(s.browser.name(), &s.label)).map(|(ix, s)| {
                    Entry::Row(Row { item: Item::Source(ix), title: s.browser.name().into(), named: None, detail: Some(bounded_title(&s.label)) })
                }),
            );
        }
    }
    if out.len() == 1 {
        out.push(Entry::Empty(empty_text(page, query)));
    }
    out
}

fn memory_entries(b: &Browser, query: &str, out: &mut Vec<Entry>) {
    out.push(Entry::Back("Memory".into()));
    let costs = b.tab_costs();
    let mut tabs: Vec<(TabKey, &Tab)> = b.model.tabs.iter().filter(|t| library::matches(query, &t.title, &t.url)).map(|t| (b.model.key(t), t)).collect();
    tabs.sort_by_key(|(key, t)| (Reverse(costs.get(key).map_or(0, |c| c.footprint)), Reverse(t.last_active)));
    out.extend(tabs.into_iter().map(|(key, t)| {
        let detail = crate::memory::detail(costs.get(&key).copied(), b.sleeping(key));
        Entry::Row(Row { item: Item::Tab(key), title: title_or_host(&t.title, &t.url), named: None, detail: Some(detail) })
    }));
    if out.len() > 1 && query.is_empty() {
        out.push(Entry::Divider);
        out.push(row(Item::SleepInactive, "Sleep All Inactive Tabs".into(), None));
    }
}

pub(super) fn tally(n: usize) -> Option<String> {
    (n > 0).then(|| thousands(n as u64))
}

/// Rows listed on a page; on a page cut at `MAX_LIST` the cap reads "200+", never as the total.
pub(super) fn listed_tally(n: usize, capped: bool) -> Option<String> {
    if capped && n >= MAX_LIST { Some(format!("{MAX_LIST}+")) } else { tally(n) }
}

pub(super) fn empty_text(page: Page, query: &str) -> String {
    if !query.is_empty() {
        return format!("No results for “{}”", bounded_title(query));
    }
    match page {
        Page::History => "No history yet",
        Page::Bookmarks(None) => "No bookmarks yet",
        Page::Bookmarks(Some(_)) => "This folder is empty",
        Page::Downloads => "No downloads yet",
        _ => "Nothing here",
    }
    .into()
}

pub(super) fn entry_height(entry: &Entry) -> f32 {
    match entry {
        Entry::Header(..) => HEADER_ROW,
        Entry::Back(_) => BACK_ROW,
        Entry::Empty(_) => EMPTY_ROW,
        Entry::Divider => DIVIDER,
        Entry::Row(r) if r.detail.is_some() => DETAIL_ROW,
        Entry::Row(_) => ROW,
    }
}

/// The tallest list up to `max_list` that ends below a row, so a cut list never shows a heading or
/// divider without the row under it, or half a row; `max_list` when the whole list fits.
pub(super) fn row_boundary(entries: &[Entry], max_list: f32) -> f32 {
    let mut end = LIST_PAD;
    let mut fit = None;
    for (ix, entry) in entries.iter().enumerate() {
        end += entry_height(entry) + if ix > 0 { 1. } else { 0. };
        if end > max_list {
            return fit.unwrap_or(max_list);
        }
        if !matches!(entry, Entry::Header(..) | Entry::Divider) {
            fit = Some(end);
        }
    }
    max_list
}

/// The card's height for `entries`, with the list capped at `max_list`.
pub(super) fn card_height(entries: &[Entry], max_list: f32) -> f32 {
    let list: f32 = entries.iter().map(entry_height).sum::<f32>() + entries.len().saturating_sub(1) as f32 + LIST_PAD;
    SEARCH_HEIGHT + 2. + list.min(max_list)
}
