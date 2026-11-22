//! Clean Up: sorts a space's loose tabs into tab groups on this device, by what each page is.
//! Nothing leaves the device and no model runs: a page's address and title decide.
//!
//! A known site (`rules::CATEGORIES`) is final. Only for other sites do the host's labels, the
//! path and the title vote, since a page controls all three. A loose tab of a host the user
//! already keeps at least twice in one of their own groups, and in no other group, joins that
//! group instead, unless it is a known site whose category already has a group; a group named
//! after a category teaches no host. No group is ever named after a site, and only loose tabs move into groups.
//! `ai` is the one exception to "no model runs": an assistant's grouping, asked for by the user.

pub mod ai;
mod rules;

use std::collections::{HashMap, HashSet};

use store::{Group, Id, Layout, Slot, Store, Tab, TabKind};
use url::Url;

use rules::CATEGORIES;

/// What the Clean Up button does, for its tooltip.
pub const DESCRIPTION: &str = "Clean Up: sorts tabs into groups and archives tabs unused for longer than Settings › Tabs allows. Runs on this device.";

const MAX_LABELS: usize = 10;
const MAX_SEGMENTS: usize = 8;
const MAX_TITLE: usize = 1024;
const LABEL_VOTE: u8 = 4;
const PATH_VOTE: u8 = 4;
const WORD_VOTE: u8 = 2;
const MAX_WORDS: u8 = 3;
/// A vote wins with at least this score and this lead over the runner-up.
const MIN_SCORE: u8 = 2;
const MIN_LEAD: u8 = 2;
/// A host the user keeps in one of their groups at least this often teaches that group.
const LEARN_FROM: usize = 2;

/// A category by its index in `CATEGORIES`, and whether a known site decided it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Verdict {
    category: usize,
    known: bool,
}

/// The host of an http or https page, lowercase and without `www.` or a trailing dot.
fn web_host(url: &Url) -> Option<String> {
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.trim_end_matches('.').to_ascii_lowercase();
    Some(host.strip_prefix("www.").map(str::to_owned).unwrap_or(host))
}

fn host_of(url: &str) -> Option<String> {
    web_host(&Url::parse(url).ok()?)
}

/// Whether `host` is `site` or one of its subdomains.
fn on_site(host: &str, site: &str) -> bool {
    host == site || host.strip_suffix(site).is_some_and(|rest| rest.ends_with('.'))
}

fn classify(url: &str, title: &str) -> Option<Verdict> {
    let url = Url::parse(url).ok()?;
    let host = web_host(&url)?;
    let segments: Vec<String> =
        url.path_segments().map(|s| s.take(MAX_SEGMENTS).filter(|s| !s.is_empty()).map(str::to_ascii_lowercase).collect()).unwrap_or_default();

    let mut known: Option<((usize, bool), usize)> = None;
    for (i, c) in CATEGORIES.iter().enumerate() {
        for entry in c.sites {
            let (site, path) = entry.split_once('/').map_or((*entry, None), |(site, path)| (site, Some(path)));
            if on_site(&host, site) && path.is_none_or(|p| segments.first().is_some_and(|s| s == p)) {
                let rank = (site.len(), path.is_some());
                if known.is_none_or(|(best, _)| rank > best) {
                    known = Some((rank, i));
                }
            }
        }
    }
    if let Some((_, category)) = known {
        return Some(Verdict { category, known: true });
    }

    let labels: Vec<&str> = host.rsplit('.').skip(1).take(MAX_LABELS).collect();
    let title: String = title.chars().take(MAX_TITLE).collect::<String>().to_lowercase();
    let words: Vec<&str> = title.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let has = |phrase: &str| {
        let parts: Vec<&str> = phrase.split(' ').collect();
        words.windows(parts.len()).any(|w| w == parts.as_slice())
    };
    let scores: Vec<u8> = CATEGORIES
        .iter()
        .map(|c| {
            let label = if c.labels.iter().any(|l| labels.contains(l)) { LABEL_VOTE } else { 0 };
            let path = if c.paths.iter().any(|p| segments.iter().any(|s| s == p)) { PATH_VOTE } else { 0 };
            let hits = c.words.iter().filter(|w| has(w)).count().min(MAX_WORDS.into()) as u8;
            label + path + hits * WORD_VOTE
        })
        .collect();
    let (category, &best) = scores.iter().enumerate().max_by_key(|&(i, s)| (*s, std::cmp::Reverse(i)))?;
    let runner_up = scores.iter().enumerate().filter(|&(i, _)| i != category).map(|(_, s)| *s).max().unwrap_or(0);
    (best >= MIN_SCORE && best - runner_up >= MIN_LEAD).then_some(Verdict { category, known: false })
}

/// The category a page belongs to: by its site if known, else by its host, path and title.
pub fn category(url: &str, title: &str) -> Option<&'static str> {
    classify(url, title).map(|v| CATEGORIES[v.category].name)
}

fn category_of_group(group: &Group) -> Option<usize> {
    CATEGORIES.iter().position(|c| c.name.eq_ignore_ascii_case(&group.name))
}

/// A sort Clean Up can apply: the space's new layout, and what it changes.
#[derive(Debug)]
pub struct Sort {
    pub layout: Layout,
    /// Loose tabs that go into a group.
    pub moved: usize,
    /// Whether anything moves at all, into a group or within the order.
    pub changed: bool,
    tabs_reordered: bool,
    /// Each tab's group and place before the sort.
    before: Vec<(Id, Option<Id>, i64)>,
    groups_before: Vec<Id>,
    /// Collapsed groups the sort opens by putting tabs in them, with their names.
    opened: Vec<(Id, String)>,
}

impl Sort {
    /// The toast text after Clean Up, once the sort was applied or failed (`applied`) and
    /// archiving took `archived` tabs; `archiving_on` is whether Settings › Tabs archives at all.
    pub fn summary(&self, applied: bool, archived: usize, archiving_on: bool) -> String {
        let tabs = |n: usize| if n == 1 { "tab" } else { "tabs" };
        let sorted = match (applied, self.moved, self.tabs_reordered) {
            (false, ..) if self.changed => Some("Could not sort the tabs".to_owned()),
            _ if !self.changed => None,
            (_, 0, true) => Some("Put tabs of the same site together".to_owned()),
            (_, 0, false) => Some("Put the groups in order".to_owned()),
            (_, m, _) => Some(format!("Sorted {m} {} into groups", tabs(m))),
        };
        match (sorted, archived) {
            (None, 0) if !archiving_on => "Nothing to sort. Settings › Tabs can archive unused tabs too.".to_owned(),
            (None, 0) => "Already tidy".to_owned(),
            (Some(s), 0) => s,
            (None, a) => format!("Archived {a} unused {}", tabs(a)),
            (Some(s), a) => format!("{s}, archived {a} unused"),
        }
    }
}

/// Sorts the space's top-level unpinned tabs: loose ones into the user's group that holds their
/// host, else into their category's group (new once two tabs share a category); groups that take
/// tabs, and the tabs left loose, are ordered by host. Category groups come after the user's own
/// groups, in category order. `url` is a tab's current page.
pub fn plan(tabs: &[Tab], groups: &[Group], url: impl Fn(&Tab) -> String) -> Sort {
    let mut zone: Vec<&Tab> = tabs.iter().filter(|t| t.kind == TabKind::Ephemeral && t.folder_id.is_none()).collect();
    zone.sort_by_key(|t| t.position);
    let urls: HashMap<Id, String> = zone.iter().map(|t| (t.id, url(t))).collect();
    let hosts: HashMap<Id, String> = urls.iter().filter_map(|(&id, url)| Some((id, host_of(url)?))).collect();

    let category_groups: HashSet<Id> = groups.iter().filter(|g| category_of_group(g).is_some()).map(|g| g.id).collect();
    let mut kept: HashMap<&str, HashMap<Id, usize>> = HashMap::new();
    for t in zone.iter().filter(|t| t.group_id.is_some()) {
        if let (Some(host), Some(group)) = (hosts.get(&t.id), t.group_id) {
            *kept.entry(host.as_str()).or_default().entry(group).or_default() += 1;
        }
    }
    let learned = |host: &str| -> Option<Id> {
        let groups = kept.get(host)?;
        let (&group, &count) = groups.iter().next().filter(|_| groups.len() == 1)?;
        (count >= LEARN_FROM && !category_groups.contains(&group)).then_some(group)
    };
    let group_for = |category: usize| groups.iter().find(|g| category_of_group(g) == Some(category)).map(|g| g.id);

    let mut slots: HashMap<Id, Slot> = HashMap::new();
    let mut new_groups: Vec<usize> = Vec::new();
    let mut pending: HashMap<usize, Vec<Id>> = HashMap::new();
    for t in zone.iter().filter(|t| t.group_id.is_none()) {
        let verdict = classify(&urls[&t.id], &t.title);
        let host_group = hosts.get(&t.id).and_then(|h| learned(h));
        let category_group = verdict.and_then(|v| group_for(v.category));
        let target = match (host_group, verdict) {
            (Some(_), Some(v)) if v.known && category_group.is_some() => category_group,
            (Some(group), _) => Some(group),
            (None, _) => category_group,
        };
        match (target, verdict) {
            (Some(group), _) => {
                slots.insert(t.id, Slot::Group(group));
            }
            (None, Some(v)) => pending.entry(v.category).or_default().push(t.id),
            (None, None) => {}
        }
    }
    let mut categories: Vec<usize> = pending.iter().filter(|(_, ids)| ids.len() >= 2).map(|(&c, _)| c).collect();
    categories.sort_unstable();
    for category in categories {
        for id in &pending[&category] {
            slots.insert(*id, Slot::New(new_groups.len()));
        }
        new_groups.push(category);
    }

    let mut group_order: Vec<(Option<usize>, Slot)> = groups.iter().map(|g| (category_of_group(g), Slot::Group(g.id))).collect();
    group_order.extend(new_groups.iter().enumerate().map(|(i, &c)| (Some(c), Slot::New(i))));
    group_order.sort_by_key(|&(category, _)| category.map_or(0, |c| c + 1));

    let new_groups = new_groups.iter().map(|&c| (CATEGORIES[c].name.to_owned(), CATEGORIES[c].color.to_owned(), CATEGORIES[c].icon.to_owned())).collect();
    let group_order = group_order.into_iter().map(|(_, s)| s).collect();
    build(
        &zone,
        groups,
        &slots,
        new_groups,
        group_order,
        |members, slot, receiving| {
            if slot == Slot::Loose || receiving { by_host(members, &hosts) } else { members }
        },
    )
}

fn build(
    zone: &[&Tab],
    groups: &[Group],
    slots: &HashMap<Id, Slot>,
    new_groups: Vec<(String, String, String)>,
    group_order: Vec<Slot>,
    arrange: impl for<'t> Fn(Vec<&'t Tab>, Slot, bool) -> Vec<&'t Tab>,
) -> Sort {
    let slot_of = |t: &Tab| slots.get(&t.id).copied().unwrap_or(t.group_id.map_or(Slot::Loose, Slot::Group));
    let receiving: HashSet<Slot> = slots.values().copied().collect();
    let mut order: Vec<(Id, Slot)> = Vec::with_capacity(zone.len());
    for slot in group_order.iter().copied().chain([Slot::Loose]) {
        let members: Vec<&Tab> = zone.iter().copied().filter(|t| slot_of(t) == slot).collect();
        order.extend(arrange(members, slot, receiving.contains(&slot)).into_iter().map(|t| (t.id, slot)));
    }

    let moved = slots.len();
    let old_order: Vec<(Id, Slot)> = zone.iter().map(|t| (t.id, t.group_id.map_or(Slot::Loose, Slot::Group))).collect();
    let in_slot = |list: &[(Id, Slot)], slot: Slot| list.iter().filter(|(_, s)| *s == slot).map(|(id, _)| *id).collect::<Vec<_>>();
    let tabs_reordered = group_order.iter().copied().chain([Slot::Loose]).any(|s| in_slot(&order, s) != in_slot(&old_order, s));
    let groups_reordered = group_order != groups.iter().map(|g| Slot::Group(g.id)).collect::<Vec<_>>();
    Sort {
        layout: Layout { new_groups, tabs: order, groups: group_order, ..Default::default() },
        moved,
        changed: moved > 0 || tabs_reordered || groups_reordered,
        tabs_reordered,
        before: zone.iter().enumerate().map(|(rank, t)| (t.id, t.group_id, rank as i64)).collect(),
        groups_before: groups.iter().map(|g| g.id).collect(),
        opened: groups.iter().filter(|g| g.collapsed && receiving.contains(&Slot::Group(g.id))).map(|g| (g.id, g.name.clone())).collect(),
    }
}

/// Tabs of one host next to each other, at the place of the first of them; tabs without a web
/// host keep their place.
fn by_host<'a>(tabs: Vec<&'a Tab>, hosts: &HashMap<Id, String>) -> Vec<&'a Tab> {
    let mut first: HashMap<&str, usize> = HashMap::new();
    let mut keyed: Vec<(usize, usize, &Tab)> =
        tabs.iter().enumerate().map(|(i, t)| (hosts.get(&t.id).map_or(i, |h| *first.entry(h.as_str()).or_insert(i)), i, *t)).collect();
    keyed.sort_by_key(|&(cluster, i, _)| (cluster, i));
    keyed.into_iter().map(|(.., t)| t).collect()
}

/// What `apply` changed, so `undo` can put it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Undo {
    space_id: Id,
    /// Each tab with its group and position before the sort, and its group after it.
    tabs: Vec<(Id, Option<Id>, i64, Option<Id>)>,
    groups: Vec<Id>,
    /// Groups the sort made and groups it opened, by id and name, so a group that took the id of
    /// one deleted since is left alone.
    created: Vec<(Id, String)>,
    opened: Vec<(Id, String)>,
}

impl Undo {
    pub fn space_id(&self) -> Id {
        self.space_id
    }
}

/// Applies the sort to `space_id` at once, or not at all.
pub fn apply(store: &Store, space_id: Id, sort: &Sort) -> Result<Undo, store::Error> {
    let created = store.apply_layout(space_id, &sort.layout)?;
    let after: HashMap<Id, Option<Id>> = match store.tabs(space_id) {
        Ok(tabs) => tabs.into_iter().map(|t| (t.id, t.group_id)).collect(),
        Err(_) => sort
            .layout
            .tabs
            .iter()
            .map(|&(id, slot)| {
                let group = match slot {
                    Slot::Loose => None,
                    Slot::Group(g) => Some(g),
                    Slot::New(i) => created.get(i).copied(),
                };
                (id, group)
            })
            .collect(),
    };
    let tabs = sort.before.iter().filter_map(|&(id, group, position)| Some((id, group, position, *after.get(&id)?))).collect();
    let created = created.into_iter().zip(sort.layout.new_groups.iter().map(|(name, ..)| name.clone())).collect();
    Ok(Undo { space_id, tabs, groups: sort.groups_before.clone(), created, opened: sort.opened.clone() })
}

/// Puts back the tabs that still sit in the group the sort put them in, the order of the groups and
/// the groups it opened, and removes each group the sort made once it is empty; the ids of the
/// groups removed. Tabs moved to another group or closed since stay as they are, and a tab opened
/// since stays behind the tab it followed.
pub fn undo(store: &Store, undo: &Undo) -> Result<Vec<Id>, store::Error> {
    let mut zone: Vec<Tab> = store.tabs(undo.space_id)?.into_iter().filter(|t| t.kind == TabKind::Ephemeral && t.folder_id.is_none()).collect();
    zone.sort_by_key(|t| t.position);
    let record: HashMap<Id, (Option<Id>, i64, Option<Id>)> = undo.tabs.iter().map(|&(id, group, position, after)| (id, (group, position, after))).collect();
    let mut anchor = (-1, 0);
    let mut keyed: Vec<((i64, usize), Id, Option<Id>)> = zone
        .iter()
        .map(|t| match record.get(&t.id) {
            Some(&(group, rank, after)) if t.group_id == after => {
                anchor = (rank, 0);
                (anchor, t.id, group)
            }
            _ => {
                anchor.1 += 1;
                (anchor, t.id, t.group_id)
            }
        })
        .collect();
    keyed.sort_by_key(|&(key, ..)| key);
    let names: HashMap<Id, String> = store.groups(undo.space_id)?.into_iter().map(|g| (g.id, g.name)).collect();
    let still = |list: &[(Id, String)]| list.iter().filter(|(id, name)| names.get(id) == Some(name)).map(|(id, _)| *id).collect::<Vec<_>>();
    let layout = Layout {
        new_groups: Vec::new(),
        tabs: keyed.into_iter().map(|(_, id, group)| (id, group.map_or(Slot::Loose, Slot::Group))).collect(),
        groups: undo.groups.iter().filter(|g| names.contains_key(g)).map(|&g| Slot::Group(g)).collect(),
        drop_if_empty: still(&undo.created),
        collapse: still(&undo.opened),
    };
    store.apply_layout(undo.space_id, &layout)?;
    let left: HashSet<Id> = store.groups(undo.space_id)?.into_iter().map(|g| g.id).collect();
    Ok(layout.drop_if_empty.into_iter().filter(|id| !left.contains(id)).collect())
}

/// Whether Clean Up has at least two loose tabs to sort.
pub fn cleanable(tabs: &[Tab]) -> bool {
    tabs.iter().filter(|t| t.kind == TabKind::Ephemeral && t.group_id.is_none()).count() >= 2
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
    fn real_pages_land_in_their_category() {
        let pages = [
            ("https://github.com/rust-lang/rust/pull/1", "Fix the build", "Development"),
            ("https://gitlab.example.test/team/app/-/merge_requests/3", "Draft", "Development"),
            ("https://docs.example.test/getting-started", "Getting started", "Development"),
            ("https://stackoverflow.com/questions/1/x", "How do I", "Development"),
            ("https://localhost:3000/", "My app", "Development"),
            ("https://chatgpt.com/c/123", "New chat", "AI"),
            ("https://claude.ai/new", "Claude", "AI"),
            ("https://outlook.office.com/mail/", "Mail", "Mail and Chat"),
            ("https://app.slack.com/client/T1", "Slack", "Mail and Chat"),
            ("https://webmail.example.test/", "Webmail inbox", "Unsorted"),
            ("https://docs.google.com/document/d/1/edit", "Plan", "Work"),
            ("https://team.atlassian.net/browse/X-1", "X-1", "Work"),
            ("https://www.figma.com/file/1", "Design", "Work"),
            ("https://intranet.example.test/", "Home", "Work"),
            ("https://www.reddit.com/r/rust/", "r/rust", "Social"),
            ("https://bsky.app/profile/x", "x", "Social"),
            ("https://m.youtube.com/watch?v=1", "A video", "Video and Music"),
            ("https://open.spotify.com/album/1", "An album", "Video and Music"),
            ("https://example.test/watch/42", "Episode 3", "Video and Music"),
            ("https://www.amazon.de/dp/B0", "Some thing", "Shopping"),
            ("https://shop.example.test/item/7", "Shoes", "Shopping"),
            ("https://example.test/de/warenkorb", "Ihr Warenkorb", "Shopping"),
            ("https://www.apple.com/shop/buy-mac", "Buy Mac", "Shopping"),
            ("https://www.apple.com/mac/", "Mac", "Unsorted"),
            ("https://www.spiegel.de/politik/a-1", "Politik", "News"),
            ("https://news.ycombinator.com/item?id=1", "HN", "News"),
            ("https://news.example.test/today", "Today", "News"),
            ("https://en.wikipedia.org/wiki/Rust", "Rust", "Reading"),
            ("https://someone.substack.com/p/post", "Post", "Reading"),
            ("https://blog.example.test/2026/hello", "Hello", "Reading"),
            ("https://www.coursera.org/learn/ml", "ML", "Learning"),
            ("https://example.test/courses/101/lesson/2", "Lesson 2", "Learning"),
            ("https://www.google.com/maps/place/Berlin", "Berlin", "Travel"),
            ("https://www.google.com/search?q=maps", "Results", "Unsorted"),
            ("https://www.booking.com/hotel/de/x.html", "Hotel X", "Travel"),
            ("https://int.bahn.de/en", "DB", "Travel"),
            ("https://www.paypal.com/myaccount/", "PayPal", "Money"),
            ("https://banking.example.test/banking", "Konto", "Unsorted"),
            ("https://store.steampowered.com/app/1", "Game", "Games"),
            ("https://lichess.org/abc", "Play", "Games"),
            ("https://www.chefkoch.de/rezepte/1/x.html", "Kuchen", "Food"),
            ("https://example.test/recipes/soup", "Soup", "Food"),
            ("https://github.com/shop/cart", "Shop cart checkout buy", "Development"),
            ("https://example.test/", "Hello", "Unsorted"),
            ("https://example.test/", "Shopping cart and recipe", "Unsorted"),
        ];
        for (url, title, want) in pages {
            assert_eq!(category(url, title).unwrap_or("Unsorted"), want, "{url} {title}");
        }
    }

    #[test]
    fn a_page_cannot_outvote_a_known_site() {
        let title = "Cart checkout buy sale deals";
        assert_eq!(category("https://github.com/shop/cart/checkout", title), Some("Development"));
        assert_eq!(category("https://shop.example.test/cart/checkout", title), Some("Shopping"));
    }

    #[test]
    fn only_known_sites_are_money_or_mail() {
        assert_eq!(category("https://www.sparkasse.de/", ""), Some("Money"));
        assert_eq!(category("https://sparkasse-login.test/banking/konto", "Online-Banking Konto Sparkasse"), None);
        assert_eq!(category("https://mail.example.test/inbox", "Inbox - Mail"), None);
    }

    #[test]
    fn hosts_in_every_form_are_read_safely() {
        assert_eq!(category("https://github.com./x", "bank konto"), Some("Development"), "a trailing dot is the same host");
        assert_eq!(category("https://GitHub.COM:8443/x", ""), Some("Development"), "case and port do not matter");
        assert_eq!(category("https://user:pw@github.com/x", ""), Some("Development"));
        assert_eq!(category("https://github.com.evil.test/x", ""), None, "a known host inside another is not the site");
        assert_eq!(category("https://bücher.example/cart", ""), Some("Shopping"), "IDN hosts are read as punycode");
        assert_eq!(category("http://192.168.0.1/cart", ""), Some("Shopping"));
        assert_eq!(category("http://[::1]/", "Ünïcödé 🎉 title"), None);
        assert_eq!(category("https://./", "cart"), Some("Shopping"));
    }

    #[test]
    fn non_web_urls_are_left_alone() {
        for url in ["file:///Users/x/docs/api.html", "about:blank", "data:text/html,shop", "blob:https://github.com/1", "tarn-extension://x/shop"] {
            assert_eq!(category(url, "Shopping cart docs"), None, "{url}");
        }
    }

    #[test]
    fn empty_path_segments_count_toward_the_bound() {
        let url = format!("https://example.test{}cart", "/".repeat(MAX_SEGMENTS + 1));
        assert_eq!(category(&url, ""), None);
        assert_eq!(category("https://example.test//cart", ""), Some("Shopping"));
    }

    #[test]
    fn long_urls_are_bounded() {
        let deep = format!("https://example.test/{}cart", "a/".repeat(MAX_SEGMENTS));
        assert_eq!(category(&deep, ""), None, "only the first path segments are read");
        let labels = format!("https://shop.{}example.test/", "a.".repeat(MAX_LABELS));
        assert_eq!(category(&labels, ""), None, "only the last host labels are read");
        let query = "https://example.test/?q=cart#checkout";
        assert_eq!(category(query, ""), None, "query and fragment are not read");
        let title = format!("{} cart", "x ".repeat(MAX_TITLE));
        assert_eq!(category("https://example.test/", &title), None, "only the start of a title is read");
        assert_eq!(category(&format!("https://example.test/{}", "x/".repeat(100_000)), "cart"), Some("Shopping"));
    }

    #[test]
    fn rules_are_unambiguous() {
        let colors = ["graphite", "green", "blue", "indigo", "orange", "pink", "red", "vermilion"];
        let mut seen = HashSet::new();
        for c in CATEGORIES {
            assert!(colors.contains(&c.color), "{}", c.name);
            assert!(crate::icon::SYMBOLS.contains(&c.icon), "{}", c.name);
            for list in [c.sites, c.labels, c.paths, c.words] {
                for entry in list {
                    assert!(!entry.is_empty() && *entry == entry.to_lowercase() && !entry.starts_with("www."), "{entry}");
                }
            }
            for site in c.sites {
                assert!(site.is_ascii() && seen.insert(*site), "{site} is listed twice");
                assert!(site.split_once('/').is_none_or(|(_, path)| !path.is_empty() && !path.contains('/')), "{site}");
            }
        }
        let names: HashSet<String> = CATEGORIES.iter().map(|c| c.name.to_ascii_lowercase()).collect();
        assert_eq!(names.len(), CATEGORIES.len());
    }

    /// Each tab's id, group and position, and the groups in order.
    type Arrangement = (Vec<(Id, Option<Id>, i64)>, Vec<Id>);

    struct Space<'a> {
        store: &'a Store,
        id: Id,
    }

    impl Space<'_> {
        fn add(&self, url: &str) -> Id {
            self.store.create_tab(self.id, TabKind::Ephemeral, None, url, "").unwrap().id
        }
        fn group(&self, name: &str, urls: &[&str]) -> Id {
            let group = self.store.create_group(self.id, name).unwrap().id;
            for url in urls {
                self.store.set_tab_group(self.add(url), Some(group)).unwrap();
            }
            group
        }
        fn plan(&self) -> Sort {
            plan(&self.store.tabs(self.id).unwrap(), &self.store.groups(self.id).unwrap(), |t| t.url.clone())
        }
        fn group_of(&self, id: Id) -> Option<Id> {
            self.store.tabs(self.id).unwrap().into_iter().find(|t| t.id == id).unwrap().group_id
        }
        fn group_named(&self, name: &str) -> Option<Id> {
            self.store.groups(self.id).unwrap().into_iter().find(|g| g.name == name).map(|g| g.id)
        }
        fn layout(&self) -> Arrangement {
            let mut tabs: Vec<_> = self.store.tabs(self.id).unwrap().into_iter().map(|t| (t.id, t.group_id, t.position)).collect();
            tabs.sort();
            (tabs, self.store.groups(self.id).unwrap().into_iter().map(|g| g.id).collect())
        }
    }

    fn with_space(test: impl FnOnce(Space)) {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::TestCore::open(dir.path(), false);
        let store = &core.disk;
        let id = store.spaces(store.profiles().unwrap()[0].id).unwrap()[0].id;
        test(Space { store, id });
    }

    #[test]
    fn a_plan_groups_pairs_and_fills_existing_groups() {
        with_space(|s| {
            let (a, b, c, d) = (s.add("https://github.com/a"), s.add("https://gitlab.com/b"), s.add("https://youtube.com/c"), s.add("https://example.com/"));
            s.store.create_tab(s.id, TabKind::Pinned, None, "https://github.com/pinned", "").unwrap();
            let news = s.group("news", &[]);
            let e = s.add("https://bbc.com/x");
            let sort = s.plan();
            assert_eq!(sort.moved, 3);
            assert_eq!(sort.layout.new_groups, [("Development".to_owned(), "indigo".to_owned(), "code".to_owned())]);
            apply(s.store, s.id, &sort).unwrap();
            let dev = s.group_named("Development");
            assert_eq!(s.store.groups(s.id).unwrap().into_iter().find(|g| Some(g.id) == dev).unwrap().icon, "code");
            assert_eq!((s.group_of(a), s.group_of(b), s.group_of(c), s.group_of(d), s.group_of(e)), (dev, dev, None, None, Some(news)));
        });
    }

    #[test]
    fn groups_and_tabs_come_out_ordered_and_a_second_run_moves_nothing() {
        with_space(|s| {
            let mine = s.group("Holiday", &["https://example.test/1"]);
            let news = s.group("News", &["https://bbc.com/1"]);
            let (yt, gh1, other, gh2, bbc) = (
                s.add("https://youtube.com/1"),
                s.add("https://github.com/1"),
                s.add("https://plain.test/"),
                s.add("https://github.com/2"),
                s.add("https://bbc.com/2"),
            );
            let yt2 = s.add("https://youtube.com/2");
            let other2 = s.add("https://plain.test/2");
            apply(s.store, s.id, &s.plan()).unwrap();
            let dev = s.group_named("Development").unwrap();
            let video = s.group_named("Video and Music").unwrap();
            assert_eq!(s.layout().1, [mine, dev, video, news], "the user's groups first, then categories in order");
            assert_eq!(
                (s.group_of(gh1), s.group_of(gh2), s.group_of(yt), s.group_of(yt2), s.group_of(bbc)),
                (Some(dev), Some(dev), Some(video), Some(video), Some(news))
            );
            let tabs = s.store.tabs(s.id).unwrap();
            let pos = |id: Id| tabs.iter().find(|t| t.id == id).unwrap().position;
            assert!(pos(other) < pos(other2));
            let again = s.plan();
            assert!(!again.changed && again.moved == 0, "{again:?}");
            let carts: Vec<Id> = ["cart", "cart", "Hello"]
                .iter()
                .map(|title| s.store.create_tab(s.id, TabKind::Ephemeral, None, "https://mixed.test/", title).unwrap().id)
                .collect();
            apply(s.store, s.id, &s.plan()).unwrap();
            let shopping = s.group_named("Shopping");
            assert_eq!(carts.iter().map(|&id| s.group_of(id)).collect::<Vec<_>>(), [shopping, shopping, None]);
            let again = s.plan();
            assert!(!again.changed && again.moved == 0, "a category group teaches no host: {again:?}");
        });
    }

    #[test]
    fn loose_leftovers_sit_next_to_their_host() {
        with_space(|s| {
            let (a1, b1, a2) = (s.add("https://a.test/1"), s.add("https://b.test/1"), s.add("https://a.test/2"));
            let blank = s.add("about:blank");
            let sort = s.plan();
            assert_eq!(sort.moved, 0);
            assert!(sort.changed);
            assert_eq!(sort.layout.tabs.iter().map(|(id, _)| *id).collect::<Vec<_>>(), [a1, a2, b1, blank]);
            assert!(sort.layout.new_groups.is_empty(), "no group is named after a site");
        });
    }

    #[test]
    fn a_tab_follows_its_host_into_the_users_group() {
        with_space(|s| {
            let trip = s.group("Trip", &["https://hotel.test/a", "https://hotel.test/b"]);
            let loose = s.add("https://hotel.test/c");
            let gh = s.add("https://github.com/x");
            let rust = s.group("Rust", &["https://github.com/a", "https://github.com/b"]);
            let sort = s.plan();
            assert_eq!(sort.moved, 2);
            apply(s.store, s.id, &sort).unwrap();
            assert_eq!((s.group_of(loose), s.group_of(gh)), (Some(trip), Some(rust)), "the user's group beats a built-in category");
        });
    }

    #[test]
    fn a_lookalike_page_stays_out_of_the_money_group() {
        with_space(|s| {
            let money = s.group("Money", &["https://www.sparkasse.de/"]);
            let fake = s.add("https://sparkasse-login.test/banking/konto");
            let real = s.add("https://www.sparkasse.de/konto");
            apply(s.store, s.id, &s.plan()).unwrap();
            assert_eq!((s.group_of(fake), s.group_of(real)), (None, Some(money)));
            for url in ["https://sparkasse-login.test/1", "https://sparkasse-login.test/2"] {
                s.store.set_tab_group(s.add(url), Some(money)).unwrap();
            }
            let drifted = s.add("https://sparkasse-login.test/3");
            let mine = s.group("Trip", &["https://hotel.test/1", "https://hotel.test/2"]);
            let hotel = s.add("https://hotel.test/3");
            apply(s.store, s.id, &s.plan()).unwrap();
            assert_eq!(s.group_of(drifted), None, "tabs that drifted into Money teach it nothing");
            for url in ["https://sparkasse-login.notion.site/1", "https://sparkasse-login.notion.site/2"] {
                s.store.set_tab_group(s.add(url), Some(money)).unwrap();
            }
            let hosted = s.add("https://sparkasse-login.notion.site/3");
            apply(s.store, s.id, &s.plan()).unwrap();
            assert_ne!(s.group_of(hosted), Some(money), "a known site of another category is no bank either");
            assert_eq!(s.group_of(hotel), Some(mine), "the user's own groups still learn");
        });
    }

    #[test]
    fn one_drifted_tab_teaches_nothing() {
        with_space(|s| {
            s.group("Bank", &["https://mybank.test/", "https://evil.test/"]);
            let evil = s.add("https://evil.test/x");
            s.group("A", &["https://split.test/1", "https://split.test/2"]);
            s.group("B", &["https://split.test/3", "https://split.test/4"]);
            let split = s.add("https://split.test/5");
            let dev = s.group("Development", &[]);
            s.group("Mine", &["https://github.com/1", "https://github.com/2"]);
            let gh = s.add("https://github.com/3");
            apply(s.store, s.id, &s.plan()).unwrap();
            assert_eq!(s.group_of(evil), None, "one tab of a host is not enough");
            assert_eq!(s.group_of(split), None, "a host kept in two groups teaches neither");
            assert_eq!(s.group_of(gh), Some(dev), "a known site goes to its category's group when there is one");
        });
    }

    #[test]
    fn undo_restores_layout_and_loses_no_tab() {
        with_space(|s| {
            let news = s.group("News", &["https://bbc.com/1"]);
            s.store.update_group(news, "News", true).unwrap();
            for url in ["https://github.com/1", "https://a.test/", "https://gitlab.com/2", "https://cnn.com/3", "https://a.test/2"] {
                s.add(url);
            }
            let before = s.layout();
            let undo_record = apply(s.store, s.id, &s.plan()).unwrap();
            assert_ne!(s.layout(), before);
            assert!(!s.store.groups(s.id).unwrap().iter().find(|g| g.id == news).unwrap().collapsed, "a group that takes tabs opens");
            let dropped = undo(s.store, &undo_record).unwrap();
            assert_eq!(s.layout(), before);
            assert_eq!(dropped.len(), 1, "the Development group the sort made");
            assert!(s.store.groups(s.id).unwrap().iter().find(|g| g.id == news).unwrap().collapsed, "a group the sort opened closes again");
            assert_eq!(s.store.tabs(s.id).unwrap().len(), 6);
        });
    }

    #[test]
    fn undo_keeps_what_changed_since() {
        with_space(|s| {
            let (a, b, c) = (s.add("https://github.com/1"), s.add("https://gitlab.com/2"), s.add("https://codeberg.org/3"));
            let undo_record = apply(s.store, s.id, &s.plan()).unwrap();
            let dev = s.group_named("Development").unwrap();
            s.store.set_tab_group(c, None).unwrap();
            s.store.delete_tab(b).unwrap();
            let mine = s.group("Mine", &[]);
            s.store.set_tab_group(a, Some(mine)).unwrap();
            let late = s.add("https://github.com/late");
            s.store.set_tab_group(late, Some(dev)).unwrap();
            let fresh = s.add("https://fresh.test/");
            assert!(undo(s.store, &undo_record).unwrap().is_empty(), "a group still in use stays");
            let zone: Vec<Id> = {
                let mut tabs = s.store.tabs(s.id).unwrap();
                tabs.sort_by_key(|t| t.position);
                tabs.into_iter().map(|t| t.id).collect()
            };
            assert_eq!(zone.last(), Some(&fresh), "a tab opened after the sort stays last");
            assert_eq!((s.group_of(a), s.group_of(late)), (Some(mine), Some(dev)), "tabs moved by hand stay, and so does a group still in use");
            assert_eq!(s.group_of(c), None);
            assert_eq!(s.store.tabs(s.id).unwrap().len(), 4);
        });
    }

    #[test]
    fn the_summary_says_what_happened() {
        let sort = |moved, tabs_reordered, changed| Sort {
            layout: Layout::default(),
            moved,
            changed,
            tabs_reordered,
            before: Vec::new(),
            groups_before: Vec::new(),
            opened: Vec::new(),
        };
        assert_eq!(sort(3, true, true).summary(true, 0, true), "Sorted 3 tabs into groups");
        assert_eq!(sort(1, false, true).summary(true, 2, true), "Sorted 1 tab into groups, archived 2 unused");
        assert_eq!(sort(0, true, true).summary(true, 0, true), "Put tabs of the same site together");
        assert_eq!(sort(0, false, true).summary(true, 0, true), "Put the groups in order");
        assert_eq!(sort(2, true, true).summary(false, 4, true), "Could not sort the tabs, archived 4 unused");
        assert_eq!(sort(0, false, false).summary(true, 1, true), "Archived 1 unused tab");
        assert_eq!(sort(0, false, false).summary(true, 0, true), "Already tidy");
        assert_eq!(sort(0, false, false).summary(true, 0, false), "Nothing to sort. Settings › Tabs can archive unused tabs too.");
    }

    #[test]
    fn undo_keeps_a_new_tab_behind_the_tab_it_followed() {
        with_space(|s| {
            let (a, b, c, d, f) =
                (s.add("https://github.com/1"), s.add("https://b.test/"), s.add("https://c.test/"), s.add("https://github.com/2"), s.add("https://f.test/"));
            let undo_record = apply(s.store, s.id, &s.plan()).unwrap();
            for id in [b, c, f] {
                s.store.delete_tab(id).unwrap();
            }
            let e = s.add("https://e.test/");
            undo(s.store, &undo_record).unwrap();
            let mut tabs = s.store.tabs(s.id).unwrap();
            tabs.sort_by_key(|t| t.position);
            assert_eq!(tabs.iter().map(|t| t.id).collect::<Vec<_>>(), [a, d, e]);
            assert!(tabs.iter().all(|t| t.group_id.is_none()));
        });
    }

    #[test]
    fn undo_works_when_a_group_id_went_to_another_space() {
        with_space(|s| {
            let mine = s.group("Mine", &["https://x.test/"]);
            s.add("https://github.com/1");
            s.add("https://gitlab.com/2");
            let undo_record = apply(s.store, s.id, &s.plan()).unwrap();
            let profile = s.store.profiles().unwrap()[0].id;
            let other = s.store.create_space(profile, "Other", "").unwrap().id;
            let dev = s.group_named("Development").unwrap();
            s.store.ungroup(dev).unwrap();
            s.store.ungroup(mine).unwrap();
            let taken = s.store.create_group(other, "Elsewhere").unwrap().id;
            assert!(taken == dev || taken == mine);
            undo(s.store, &undo_record).unwrap();
            assert_eq!(s.store.groups(other).unwrap().len(), 1);
        });
    }

    #[test]
    fn undo_leaves_a_group_that_took_a_deleted_groups_id_alone() {
        with_space(|s| {
            s.add("https://github.com/1");
            s.add("https://gitlab.com/2");
            let undo_record = apply(s.store, s.id, &s.plan()).unwrap();
            let dev = s.group_named("Development").unwrap();
            s.store.delete_group(dev).unwrap();
            let mine = s.store.create_group(s.id, "Mine").unwrap().id;
            assert_eq!(mine, dev, "SQLite gives a deleted group's id to the next one");
            assert!(undo(s.store, &undo_record).unwrap().is_empty());
            assert_eq!(s.group_named("Mine"), Some(mine));
        });
    }

    #[test]
    fn undo_drops_a_created_group_once_empty() {
        with_space(|s| {
            s.add("https://github.com/1");
            s.add("https://gitlab.com/2");
            let undo_record = apply(s.store, s.id, &s.plan()).unwrap();
            assert!(s.group_named("Development").is_some());
            undo(s.store, &undo_record).unwrap();
            assert_eq!(s.group_named("Development"), None);
        });
    }
}
