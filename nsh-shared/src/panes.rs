#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Profile,
    General,
    Appearance,
    Tabs,
    Search,
    Privacy,
    Permissions,
    Snippets,
    Notes,
    Ai,
    Developer,
    Profiles,
    Shortcuts,
}

pub const PANES: [(Pane, &str); 13] = [
    (Pane::Profile, "Protection"),
    (Pane::General, "General"),
    (Pane::Appearance, "Appearance"),
    (Pane::Tabs, "Tabs"),
    (Pane::Search, "Search"),
    (Pane::Privacy, "Privacy"),
    (Pane::Permissions, "Permissions"),
    (Pane::Snippets, "Snippets"),
    (Pane::Notes, "Notes"),
    (Pane::Ai, "AI"),
    (Pane::Developer, "Developer"),
    (Pane::Profiles, "Profiles"),
    (Pane::Shortcuts, "Shortcuts"),
];

pub const GROUPS: [(&str, &[Pane]); 2] = [
    ("You", &[Pane::Profile, Pane::Profiles]),
    (
        "Browser",
        &[
            Pane::General,
            Pane::Appearance,
            Pane::Tabs,
            Pane::Search,
            Pane::Privacy,
            Pane::Permissions,
            Pane::Snippets,
            Pane::Notes,
            Pane::Ai,
            Pane::Developer,
            Pane::Shortcuts,
        ],
    ),
];

pub fn name(pane: Pane) -> &'static str {
    PANES.iter().find(|(p, _)| *p == pane).map_or("", |(_, n)| n)
}

/// The words Settings search finds a pane by, besides its name.
pub fn words(pane: Pane) -> &'static str {
    match pane {
        Pane::Profile => "shield level standard strict lockdown card filter lists site overrides connections default browser",
        Pane::General => "downloads folders import about version licence source privacy imprint",
        Pane::Appearance => {
            "theme dark light match system accent colour color motion reduce toolbar compact density sidebar search bar effect tips design glass liquid transparency translucent blur corners round"
        }
        Pane::Tabs => "awake sleep memory new tab start page close archive gestures swipe drag tear off temporary full window",
        Pane::Search => "search engine suggestions history text shortcuts bangs",
        Pane::Privacy => {
            "block ads trackers cookie banners youtube pop-ups malware adult zoom muted hidden elements connections history clear export delete data"
        }
        Pane::Permissions => "camera microphone location pop-ups site answers allow block",
        Pane::Snippets => "text expand insert type template signature reply",
        Pane::Notes => "notes write search ideas assistant",
        Pane::Ai => "assistant chat claude api key ollama model provider group tabs",
        Pane::Developer => "web inspector developer tools local servers localhost",
        Pane::Profiles => "profile new edit delete space rules",
        Pane::Shortcuts => "keyboard keys",
    }
}

/// The Extensions entry's name and search words; NSH clients list it beside the panes.
pub const EXTENSIONS: (&str, &str) = ("Extensions", "install github file bitwarden every site official catalog store updates");

/// Whether Settings search for `query` shows `pane`.
pub fn matches(pane: Pane, query: &str) -> bool {
    browser_core::settings::words_match(query, &format!("{} {}", name(pane), words(pane)))
}

pub fn icon(pane: Pane) -> &'static str {
    match pane {
        Pane::Profile => crate::icons::SHIELD,
        Pane::General => crate::icons::GEAR,
        Pane::Appearance => crate::icons::PALETTE,
        Pane::Tabs => crate::icons::APP_WINDOW,
        Pane::Search => crate::icons::SEARCH,
        Pane::Privacy => crate::icons::HAND,
        Pane::Permissions => crate::icons::CAMERA,
        Pane::Snippets => crate::icons::FILE_TEXT,
        Pane::Notes => crate::icons::PENCIL,
        Pane::Ai => crate::icons::SPARKLES,
        Pane::Developer => crate::icons::CODE,
        Pane::Profiles => crate::icons::PERSON,
        Pane::Shortcuts => crate::icons::KEYBOARD,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panes_follow_the_mac_order_and_have_names() {
        assert_eq!(PANES[0].0, Pane::Profile);
        assert!(PANES.iter().all(|(p, n)| name(*p) == *n && !n.is_empty() && !icon(*p).is_empty()));
        let grouped: Vec<Pane> = GROUPS.iter().flat_map(|(_, panes)| panes.iter().copied()).collect();
        assert_eq!(grouped.len(), PANES.len());
        assert!(PANES.iter().all(|(p, _)| grouped.contains(p)), "every pane is in one group");
    }

    #[test]
    fn search_finds_panes_by_name_and_words() {
        let found = |q: &str| PANES.iter().filter(|(p, _)| matches(*p, q)).map(|(_, n)| *n).collect::<Vec<_>>();
        assert_eq!(found("lockd"), ["Protection"]);
        assert_eq!(found("COOKIE ban"), ["Privacy"]);
        assert_eq!(found("density"), ["Appearance"]);
        assert_eq!(found("").len(), PANES.len());
        assert!(found("ock").is_empty());
    }
}
