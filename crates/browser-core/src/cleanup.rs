//! Clean Up: sorts a space's loose tabs into tab groups on this device, by what each page is.
//! Nothing leaves the device; known sites and words in the title decide, never a remote model.

use std::collections::BTreeMap;

use store::{Group, Id, Store, Tab, TabKind};
use url::Url;

/// A category: its group name, its colour and the sites and title words that belong to it.
struct Category {
    name: &'static str,
    color: &'static str,
    hosts: &'static [&'static str],
    words: &'static [&'static str],
}

const CATEGORIES: [Category; 11] = [
    Category {
        name: "Development",
        color: "indigo",
        hosts: &[
            "github.com",
            "gitlab.com",
            "bitbucket.org",
            "stackoverflow.com",
            "stackexchange.com",
            "docs.rs",
            "crates.io",
            "npmjs.com",
            "pypi.org",
            "developer.mozilla.org",
            "developer.apple.com",
            "rust-lang.org",
            "go.dev",
            "vercel.com",
            "netlify.com",
            "codeberg.org",
            "huggingface.co",
            "localhost",
            "127.0.0.1",
        ],
        words: &["api", "docs", "documentation", "pull request", "issue", "commit", "repository", "sdk", "npm", "cargo"],
    },
    Category {
        name: "AI",
        color: "pink",
        hosts: &["chatgpt.com", "chat.openai.com", "claude.ai", "gemini.google.com", "perplexity.ai", "copilot.microsoft.com", "mistral.ai", "poe.com"],
        words: &["chatgpt", "claude", "gemini", "copilot"],
    },
    Category {
        name: "Mail and Chat",
        color: "blue",
        hosts: &[
            "mail.google.com",
            "outlook.live.com",
            "outlook.office.com",
            "mail.proton.me",
            "proton.me",
            "icloud.com",
            "slack.com",
            "discord.com",
            "web.whatsapp.com",
            "web.telegram.org",
            "teams.microsoft.com",
            "messenger.com",
        ],
        words: &["inbox", "mail"],
    },
    Category {
        name: "Work",
        color: "graphite",
        hosts: &[
            "docs.google.com",
            "drive.google.com",
            "sheets.google.com",
            "calendar.google.com",
            "notion.so",
            "figma.com",
            "miro.com",
            "linear.app",
            "atlassian.net",
            "trello.com",
            "asana.com",
            "monday.com",
            "office.com",
            "sharepoint.com",
            "dropbox.com",
            "airtable.com",
            "canva.com",
        ],
        words: &["spreadsheet", "document", "calendar", "board", "dashboard", "invoice"],
    },
    Category {
        name: "Social",
        color: "orange",
        hosts: &[
            "x.com",
            "twitter.com",
            "facebook.com",
            "instagram.com",
            "reddit.com",
            "linkedin.com",
            "threads.net",
            "bsky.app",
            "mastodon.social",
            "tiktok.com",
            "pinterest.com",
            "tumblr.com",
        ],
        words: &[],
    },
    Category {
        name: "Video and Music",
        color: "red",
        hosts: &[
            "youtube.com",
            "youtu.be",
            "netflix.com",
            "twitch.tv",
            "vimeo.com",
            "spotify.com",
            "music.apple.com",
            "soundcloud.com",
            "disneyplus.com",
            "primevideo.com",
            "max.com",
            "crunchyroll.com",
        ],
        words: &["trailer", "podcast", "episode", "playlist"],
    },
    Category {
        name: "Shopping",
        color: "green",
        hosts: &[
            "amazon.com",
            "amazon.de",
            "amazon.co.uk",
            "ebay.com",
            "ebay.de",
            "etsy.com",
            "aliexpress.com",
            "temu.com",
            "zalando.de",
            "otto.de",
            "ikea.com",
            "bestbuy.com",
            "walmart.com",
            "kleinanzeigen.de",
        ],
        words: &["cart", "checkout", "warenkorb", "shop", "kaufen", "buy"],
    },
    Category {
        name: "News",
        color: "vermilion",
        hosts: &[
            "nytimes.com",
            "bbc.com",
            "bbc.co.uk",
            "cnn.com",
            "theguardian.com",
            "reuters.com",
            "apnews.com",
            "spiegel.de",
            "zeit.de",
            "faz.net",
            "sueddeutsche.de",
            "tagesschau.de",
            "heise.de",
            "golem.de",
            "theverge.com",
            "arstechnica.com",
            "techcrunch.com",
            "news.ycombinator.com",
            "washingtonpost.com",
            "bloomberg.com",
        ],
        words: &["breaking", "nachrichten"],
    },
    Category {
        name: "Reading",
        color: "graphite",
        hosts: &["wikipedia.org", "medium.com", "substack.com", "arxiv.org", "scholar.google.com", "goodreads.com", "dev.to"],
        words: &["wiki", "paper", "guide", "tutorial", "blog"],
    },
    Category {
        name: "Travel",
        color: "blue",
        hosts: &["booking.com", "airbnb.com", "expedia.com", "maps.google.com", "tripadvisor.com", "bahn.de", "skyscanner.net", "kayak.com"],
        words: &["flight", "hotel", "maps", "route"],
    },
    Category {
        name: "Money",
        color: "green",
        hosts: &["paypal.com", "revolut.com", "n26.com", "wise.com", "stripe.com", "coinbase.com", "binance.com", "tradingview.com"],
        words: &["bank", "banking", "konto", "stocks"],
    },
];

/// Whether `host` is `site` or one of its subdomains.
fn on_site(host: &str, site: &str) -> bool {
    host == site || host.strip_suffix(site).is_some_and(|rest| rest.ends_with('.'))
}

/// The category a page belongs to: by its site, then by whole words in its title.
pub fn category(url: &str, title: &str) -> Option<&'static str> {
    let host = Url::parse(url).ok()?.host_str()?.trim_start_matches("www.").to_ascii_lowercase();
    if let Some(c) = CATEGORIES.iter().find(|c| c.hosts.iter().any(|site| on_site(&host, site))) {
        return Some(c.name);
    }
    let title = title.to_lowercase();
    let words: Vec<&str> = title.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let has = |phrase: &str| {
        let parts: Vec<&str> = phrase.split(' ').collect();
        words.windows(parts.len()).any(|w| w == parts.as_slice())
    };
    CATEGORIES.iter().find(|c| c.words.iter().any(|w| has(w))).map(|c| c.name)
}

fn color(name: &str) -> &'static str {
    CATEGORIES.iter().find(|c| c.name == name).map_or("graphite", |c| c.color)
}

/// Where Clean Up puts tabs: into an existing group of the same name, else a new one.
#[derive(Debug, PartialEq)]
pub enum Target {
    Group(Id),
    New { name: String, color: &'static str },
}

/// Groups for the space's loose unpinned tabs: each category with at least two tabs gets one; a
/// group already named like a category takes that category's tabs. `url` is a tab's current page.
pub fn plan(tabs: &[Tab], groups: &[Group], url: impl Fn(&Tab) -> String) -> Vec<(Target, Vec<Id>)> {
    let mut found: BTreeMap<&'static str, Vec<Id>> = BTreeMap::new();
    for tab in tabs.iter().filter(|t| t.kind == TabKind::Ephemeral && t.group_id.is_none()) {
        if let Some(name) = category(&url(tab), &tab.title) {
            found.entry(name).or_default().push(tab.id);
        }
    }
    found
        .into_iter()
        .filter_map(|(name, ids)| match groups.iter().find(|g| g.name.eq_ignore_ascii_case(name)) {
            Some(group) => Some((Target::Group(group.id), ids)),
            None if ids.len() >= 2 => Some((Target::New { name: name.to_owned(), color: color(name) }, ids)),
            None => None,
        })
        .collect()
}

/// Makes the planned groups in `space_id` and moves the tabs into them; the number of tabs moved.
pub fn apply(store: &Store, space_id: Id, plan: Vec<(Target, Vec<Id>)>) -> usize {
    let mut moved = 0;
    for (target, ids) in plan {
        let group = match target {
            Target::Group(id) => id,
            Target::New { name, color } => match store.create_group(space_id, &name) {
                Ok(group) => {
                    store.set_group_color(group.id, color).ok();
                    group.id
                }
                Err(_) => continue,
            },
        };
        moved += ids.into_iter().filter(|id| store.set_tab_group(*id, Some(group)).is_ok()).count();
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_are_sorted_by_site_then_by_title_words() {
        assert_eq!(category("https://github.com/rust-lang/rust", "rust"), Some("Development"));
        assert_eq!(category("https://www.youtube.com/watch?v=1", ""), Some("Video and Music"));
        assert_eq!(category("https://de.wikipedia.org/wiki/Tarn", ""), Some("Reading"));
        assert_eq!(category("https://mail.google.com/", ""), Some("Mail and Chat"), "a subdomain rule wins over google.com");
        assert_eq!(category("https://notgithub.com/", ""), None, "a lookalike host is not the site");
        assert_eq!(category("https://example.com/", "My shopping cart"), Some("Shopping"));
        assert_eq!(category("https://example.com/", "Rapid response"), None, "words match whole, not inside other words");
        assert_eq!(category("https://example.com/", "Open pull request #4"), Some("Development"));
        assert_eq!(category("not a url", "docs"), None);
    }

    #[test]
    fn a_plan_groups_pairs_and_fills_existing_groups() {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::TestCore::open(dir.path(), false);
        let store = &core.disk;
        let space = store.spaces(store.profiles().unwrap()[0].id).unwrap()[0].id;
        let add = |url: &str| store.create_tab(space, TabKind::Ephemeral, None, url, "").unwrap();
        let (a, b, c, d) = (add("https://github.com/a"), add("https://gitlab.com/b"), add("https://youtube.com/c"), add("https://example.com/"));
        store.create_tab(space, TabKind::Pinned, None, "https://github.com/pinned", "").unwrap();
        let news = store.create_group(space, "news").unwrap();
        let e = add("https://bbc.com/x");
        let tabs = store.tabs(space).unwrap();
        let groups = store.groups(space).unwrap();
        let plan = plan(&tabs, &groups, |t| t.url.clone());
        assert_eq!(plan, [(Target::New { name: "Development".into(), color: "indigo" }, vec![a.id, b.id]), (Target::Group(news.id), vec![e.id])]);
        assert_eq!(apply(store, space, plan), 3);
        let tabs = store.tabs(space).unwrap();
        let group_of = |id: Id| tabs.iter().find(|t| t.id == id).unwrap().group_id;
        assert_eq!(group_of(a.id), group_of(b.id));
        assert!(group_of(a.id).is_some());
        assert_eq!((group_of(c.id), group_of(d.id), group_of(e.id)), (None, None, Some(news.id)));
    }
}
