use store::Tab;

/// Most rows Find in Tabs lists.
pub const MAX_ROWS: usize = 12;

/// The tabs whose shown title (from `title`) or address contains `query`, without case, at most
/// `MAX_ROWS`; none for an empty query.
pub fn named<'a>(tabs: &'a [Tab], query: &str, title: impl Fn(&Tab) -> String) -> Vec<&'a Tab> {
    let lower = query.to_lowercase();
    tabs.iter().filter(|t| !query.is_empty() && (title(t).to_lowercase().contains(&lower) || t.url.to_lowercase().contains(&lower))).take(MAX_ROWS).collect()
}

/// What the find bar says about `count` matches; nothing before a search ran.
pub fn count_label(count: Option<usize>) -> String {
    match count {
        None => String::new(),
        Some(0) => "No matches".to_owned(),
        Some(1) => "1 match".to_owned(),
        Some(n) => format!("{n} matches"),
    }
}

/// Puts `rows` after the `text_rows` already listed, keeps the selection on its row and at most `MAX_ROWS` rows.
pub fn insert_text_rows<T>(list: &mut Vec<T>, selected: &mut usize, text_rows: &mut usize, rows: Vec<T>) {
    let at = *text_rows;
    if *selected >= at && !list.is_empty() {
        *selected = Some(*selected + rows.len()).filter(|ix| *ix < MAX_ROWS).unwrap_or(0);
    }
    *text_rows = (at + rows.len()).min(MAX_ROWS);
    list.splice(at..at, rows);
    list.truncate(MAX_ROWS);
}

#[cfg(test)]
mod tests {
    use store::TabKind;

    use super::*;

    fn rows(labels: &[&'static str]) -> Vec<&'static str> {
        labels.to_vec()
    }

    #[test]
    fn the_find_bar_names_the_count() {
        assert_eq!(count_label(None), "");
        assert_eq!(count_label(Some(0)), "No matches");
        assert_eq!(count_label(Some(1)), "1 match");
        assert_eq!(count_label(Some(12)), "12 matches");
    }

    #[test]
    fn text_rows_go_above_title_rows_and_the_selection_follows_its_row() {
        let (mut list, mut selected, mut text_rows) = (rows(&["named a", "named b"]), 1, 0);
        insert_text_rows(&mut list, &mut selected, &mut text_rows, rows(&["x1", "x2"]));
        assert_eq!(list, ["x1", "x2", "named a", "named b"]);
        assert_eq!((selected, text_rows), (3, 2));
        insert_text_rows(&mut list, &mut selected, &mut text_rows, rows(&["y1"]));
        assert_eq!(list, ["x1", "x2", "y1", "named a", "named b"], "a row titled like the marker is still a text row");
        assert_eq!((selected, text_rows), (4, 3));
        selected = 1;
        insert_text_rows(&mut list, &mut selected, &mut text_rows, rows(&["z1"]));
        assert_eq!(selected, 1, "a selected text row stays put");
    }

    #[test]
    fn rows_stop_at_twelve() {
        let (mut list, mut selected, mut text_rows) = (rows(&["named"; 4]), 3, 0);
        insert_text_rows(&mut list, &mut selected, &mut text_rows, rows(&["x"; 10]));
        assert_eq!((list.len(), text_rows, selected), (MAX_ROWS, 10, 0), "a selection pushed off the list goes to the top");
        insert_text_rows(&mut list, &mut selected, &mut text_rows, rows(&["y"; 5]));
        assert_eq!((list.len(), text_rows), (MAX_ROWS, MAX_ROWS));
        assert_eq!(list[10..], ["y", "y"]);
    }

    #[test]
    fn tabs_are_named_by_title_or_address() {
        let tab = |id, title: &str, url: &str| Tab {
            id,
            space_id: 1,
            folder_id: None,
            kind: TabKind::Ephemeral,
            url: url.into(),
            title: title.into(),
            position: 0,
            last_active: 0,
            group_id: None,
            pinned_url: None,
            custom_title: String::new(),
            web_engine: None,
        };
        let tabs = [tab(1, "Rust Book", "https://doc.rust-lang.org/book"), tab(2, "News", "https://news.test/rust"), tab(3, "Mail", "https://mail.test")];
        let ids = |q: &str| named(&tabs, q, |t| t.title.clone()).iter().map(|t| t.id).collect::<Vec<_>>();
        assert_eq!(ids("RUST"), [1, 2]);
        assert_eq!(ids("mail"), [3]);
        assert!(ids("").is_empty());
    }
}
