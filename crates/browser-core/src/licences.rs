#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub title: String,
    pub body: String,
}

/// Splits a `packaging/third-party-licenses*.txt` file at its `=====` lines; the first line of
/// each part is its title.
pub fn sections(text: &str) -> Vec<Section> {
    text.split("\n=====\n")
        .filter_map(|part| {
            let part = part.trim_matches('\n');
            let (title, body) = part.split_once('\n').unwrap_or((part, ""));
            (!title.trim().is_empty()).then(|| Section { title: title.trim().to_owned(), body: body.trim_matches('\n').to_owned() })
        })
        .collect()
}

pub fn matching<'a>(sections: &'a [Section], query: &str) -> Vec<(usize, &'a Section)> {
    let query = query.trim().to_lowercase();
    sections.iter().enumerate().filter(|(_, s)| query.is_empty() || s.title.to_lowercase().contains(&query)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "Third-party software in Tarn\n\nWebKit is part of iOS. Rust crates:\n\n=====\nserde 1.0.0\nLicence: MIT\n\n--- LICENSE ---\nMIT text\n\n=====\nMIT licence text, for the crates above that ship none\n\nPermission is hereby granted\n";

    #[test]
    fn splits_header_crates_and_licence_texts() {
        let found = sections(FILE);
        assert_eq!(
            found.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(),
            ["Third-party software in Tarn", "serde 1.0.0", "MIT licence text, for the crates above that ship none"]
        );
        assert_eq!(found[0].body, "WebKit is part of iOS. Rust crates:");
        assert_eq!(found[1].body, "Licence: MIT\n\n--- LICENSE ---\nMIT text");
        assert_eq!(found[2].body, "Permission is hereby granted");
    }

    #[test]
    fn matching_filters_titles_and_keeps_indexes() {
        let found = sections(FILE);
        assert_eq!(matching(&found, "").len(), 3);
        assert_eq!(matching(&found, " SERDE ").iter().map(|(ix, _)| *ix).collect::<Vec<_>>(), [1]);
        assert!(matching(&found, "tokio").is_empty());
    }

    #[test]
    fn shipped_lists_parse() {
        for text in [include_str!("../../../packaging/third-party-licenses-ios.txt"), include_str!("../../../packaging/third-party-licenses-android.txt")] {
            let found = sections(text);
            assert!(found.len() > 100);
            assert_eq!(found[0].title, "Third-party software in Tarn");
            assert!(found.iter().any(|s| s.title.starts_with("nsh-core ")));
        }
    }
}
