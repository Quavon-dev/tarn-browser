//! Per-profile text snippets (`store::Snippet`). Typing `;name` in a shell field (address or
//! command bar) puts the snippet there; the page gets one only through the explicit Insert
//! Snippet command, which types it into the focused field with `engine::page_scripts::
//! insert_text_script`. No world of the page listens for keys.

use std::rc::Rc;

use store::{Id, Snippet};

use crate::{Core, WebsiteData};

pub use store::{MAX_SNIPPET, MAX_SNIPPET_NAME, valid_snippet_name as valid_name};

/// What starts a snippet's name in a shell field.
pub const PREFIX: char = ';';

/// What Settings says about snippets, on every client.
pub const NOTE: &str = "Type ;name in the address or command bar to put a snippet there. Insert Snippet\u{2026} types one into the focused field of the page.";
/// The hint under a snippet's name field.
pub const NAME_RULE: &str = "Lowercase letters, digits, - and _, up to 32.";
/// The notice when Insert Snippet found no text field with the focus.
pub const NO_FIELD: &str = "Click a text field on the page first";
/// The notice when Insert Snippet runs on a Lockdown page, where no script of Tarn's runs.
pub const NO_INSERT: &str = "Snippets cannot be typed into a Lockdown page";
/// The notice when the tab left the page Insert Snippet opened on before a snippet was picked.
pub const PAGE_CHANGED: &str = "The page changed, so the snippet was not typed";
/// What Insert Snippet lists when the profile has none.
pub const EMPTY: &str = "No snippets yet. Add them in Settings \u{203a} Snippets.";

/// `text` with a trailing `;name` replaced by that snippet's body on one line, when `name` is one
/// of `snippets` and the `;` starts the text or follows white space; `None` otherwise.
pub fn expand(text: &str, snippets: &[Snippet]) -> Option<String> {
    let at = text.rfind(PREFIX)?;
    let (before, name) = (&text[..at], &text[at + 1..]);
    if !before.chars().next_back().is_none_or(char::is_whitespace) {
        return None;
    }
    let snippet = snippets.iter().find(|s| s.name == name)?;
    Some(format!("{before}{}", one_line(&snippet.body)))
}

/// Line breaks and tabs as spaces, for a one-line shell field.
pub fn one_line(body: &str) -> String {
    body.replace("\r\n", " ").replace(['\r', '\n', '\t'], " ")
}

/// The snippets Insert Snippet lists for `query`: names starting with it first, then names and
/// bodies containing it, without case.
pub fn matching<'a>(query: &str, snippets: &'a [Snippet]) -> Vec<&'a Snippet> {
    let query = query.trim().trim_start_matches(PREFIX).to_lowercase();
    let mut found: Vec<(bool, &Snippet)> = snippets
        .iter()
        .filter_map(|s| {
            let starts = s.name.starts_with(&query);
            (starts || s.name.contains(&query) || s.body.to_lowercase().contains(&query)).then_some((!starts, s))
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));
    found.into_iter().map(|(_, s)| s).collect()
}

/// The first line of `body`, cut for a list row.
pub fn preview(body: &str) -> String {
    crate::text::bounded_title(body.lines().find(|l| !l.trim().is_empty()).unwrap_or_default())
}

/// Whether `insert_text_script` typed the text: it answers "1" when a field had the focus.
pub fn inserted(answer: &str) -> bool {
    answer == "1"
}

impl<D: WebsiteData> Core<D> {
    /// A saved profile's snippets, read once until `snippets_changed`, which an edit, a reset of
    /// the profile's data (`reset_profile_data`) and a deleted profile call.
    pub fn snippets(&self, profile: Id) -> Rc<Vec<Snippet>> {
        let mut cache = self.snippets.borrow_mut();
        match &*cache {
            Some((cached, list)) if *cached == profile => list.clone(),
            _ => {
                let list = Rc::new(self.disk.snippets(profile).unwrap_or_default());
                *cache = Some((profile, list.clone()));
                list
            }
        }
    }

    pub fn snippets_changed(&self) {
        self.snippets.borrow_mut().take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cached_snippets_go_with_a_reset_of_the_profiles_data() {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::TestCore::open(dir.path(), false);
        let p = core.disk.profiles().unwrap().remove(0).id;
        core.disk.save_snippet(p, "sig", "Ada").unwrap();
        assert_eq!(core.snippets(p).len(), 1);
        core.disk.save_snippet(p, "addr", "Reef Road").unwrap();
        assert_eq!(core.snippets(p).len(), 1, "read once until a change is announced");
        core.reset_profile_data(false, p).unwrap();
        assert!(core.snippets(p).is_empty());
    }

    fn snippet(name: &str, body: &str) -> Snippet {
        Snippet { id: 0, profile_id: 1, name: name.into(), body: body.into() }
    }

    #[test]
    fn a_trailing_name_after_a_space_expands_on_one_line() {
        let all = [snippet("sig", "Best,\r\nAda\tL"), snippet("gh", "github.com/tarn")];
        assert_eq!(expand(";gh", &all), Some("github.com/tarn".into()));
        assert_eq!(expand("see ;sig", &all), Some("see Best, Ada L".into()));
        assert_eq!(expand("a;gh", &all), None, "inside a word");
        assert_eq!(expand(";g", &all), None, "an unknown name");
        assert_eq!(expand(";gh ", &all), None, "only at the end");
        assert_eq!(expand("plain", &all), None);
        assert_eq!(expand(";", &all), None);
        assert_eq!(expand("é ;gh", &all), Some("é github.com/tarn".into()));
    }

    #[test]
    fn insert_snippet_lists_name_matches_first() {
        let all = [snippet("addr", "1 Reef Road"), snippet("reef", "Coral"), snippet("sig", "Reef team")];
        let names = |q| matching(q, &all).into_iter().map(|s| s.name.as_str()).collect::<Vec<_>>();
        assert_eq!(names("ree"), ["reef", "addr", "sig"]);
        assert_eq!(names(";si"), ["sig"]);
        assert_eq!(names(""), ["addr", "reef", "sig"]);
        assert!(names("zzz").is_empty());
        assert_eq!(preview("\n  \nfirst\nsecond"), "first");
        assert!(inserted("1") && !inserted("") && !inserted("true"));
        assert!(valid_name("sig") && !valid_name(";sig"));
    }
}
