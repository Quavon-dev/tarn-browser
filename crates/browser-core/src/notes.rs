//! Per-profile notes (`store::Note`) the user writes: the list, search and editor every client
//! renders. A note reaches the assistant only as `as_pill`, after the user attaches it; notes are
//! never synced and a private space or burner tab has none.

use store::{Id, MAX_NOTE, MAX_NOTE_TITLE, Note, Store};

use crate::assistant::Pill;

/// What Settings says about notes, on every client.
pub const INTRO: &str = "Notes stay in this profile on this device. Attach one to an assistant question to ask about it.";
/// What a private space or burner tab shows in place of notes.
pub const PRIVATE: &str = "Notes belong to a saved profile. Open them outside a private space.";
pub const EMPTY: &str = "No notes yet.";
pub const NO_MATCH: &str = "No notes match.";
pub const UNTITLED: &str = "Untitled";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: Id,
    pub title: String,
    pub preview: String,
}

/// The list for `query`: every note when it is blank, else notes whose title or text contains it
/// without case, title matches first; each keeps the store's order (last edited first).
pub fn rows(notes: &[Note], query: &str) -> Vec<Row> {
    let query = query.trim().to_lowercase();
    let mut found: Vec<(bool, &Note)> = notes
        .iter()
        .filter_map(|n| {
            let in_title = n.title.to_lowercase().contains(&query);
            (in_title || n.body.to_lowercase().contains(&query)).then_some((!in_title, n))
        })
        .collect();
    found.sort_by_key(|(in_body, _)| *in_body);
    found.into_iter().map(|(_, n)| Row { id: n.id, title: title(n), preview: preview(n) }).collect()
}

/// The note's title, else its first line, else `UNTITLED`, bounded for display.
pub fn title(note: &Note) -> String {
    let shown = crate::text::bounded_title(if note.title.trim().is_empty() { first_line(&note.body) } else { &note.title });
    if shown.is_empty() { UNTITLED.to_owned() } else { shown }
}

pub(crate) fn first_line(body: &str) -> &str {
    body.lines().find(|l| !l.trim().is_empty()).unwrap_or_default()
}

/// The first line of the text that is not the shown title, for a list row.
pub fn preview(note: &Note) -> String {
    let mut lines = note.body.lines().filter(|l| !l.trim().is_empty());
    if note.title.trim().is_empty() {
        lines.next();
    }
    crate::text::bounded_title(lines.next().unwrap_or_default())
}

/// The pill that attaches `note` to an assistant question: its title and text.
pub fn as_pill(note: &Note) -> Pill {
    Pill::Note(note.id, if note.title.is_empty() { note.body.clone() } else { format!("{}\n\n{}", note.title, note.body) })
}

/// A note being written (`id` none) or edited.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    pub id: Option<Id>,
    pub title: String,
    pub body: String,
}

impl Draft {
    pub fn of(note: &Note) -> Draft {
        Draft { id: Some(note.id), title: note.title.clone(), body: note.body.clone() }
    }

    /// Saves the draft in `profile`; the message to show when it cannot. The store refuses a
    /// private profile, so nothing of a private space is ever written.
    pub fn save(&self, store: &Store, profile: Id) -> Result<Note, String> {
        if self.title.trim().is_empty() && self.body.trim().is_empty() {
            return Err("Write a title or some text.".into());
        }
        if self.title.chars().count() > MAX_NOTE_TITLE {
            return Err(format!("A title holds up to {MAX_NOTE_TITLE} characters."));
        }
        if self.body.len() > MAX_NOTE {
            return Err(format!("A note holds up to {} KB.", MAX_NOTE / 1024));
        }
        store.save_note(profile, self.id, &self.title, &self.body).map_err(|_| "The note could not be saved.".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: Id, title: &str, body: &str) -> Note {
        Note { id, profile_id: 1, title: title.into(), body: body.into(), updated: 0 }
    }

    #[test]
    fn search_puts_title_matches_first_and_rows_fall_back_to_the_text() {
        let all = [note(1, "Groceries", "reef salt"), note(2, "Reef trip", "Day 1\nsnorkel"), note(3, "", "\n reef notes\nmore"), note(4, "", " ")];
        let ids = |q| rows(&all, q).into_iter().map(|r| r.id).collect::<Vec<_>>();
        assert_eq!(ids("REEF"), [2, 1, 3]);
        assert_eq!(ids("  "), [1, 2, 3, 4]);
        assert!(ids("coral").is_empty());
        let shown = rows(&all, "");
        assert_eq!((shown[1].title.as_str(), shown[1].preview.as_str()), ("Reef trip", "Day 1"));
        assert_eq!((shown[2].title.as_str(), shown[2].preview.as_str()), ("reef notes", "more"));
        assert_eq!(shown[3].title, UNTITLED);
    }

    #[test]
    fn a_pill_carries_title_and_text() {
        assert_eq!(as_pill(&note(7, "T", "b")), Pill::Note(7, "T\n\nb".into()));
        assert_eq!(as_pill(&note(8, "", "b")), Pill::Note(8, "b".into()));
    }

    #[test]
    fn drafts_save_checked_and_never_in_a_private_profile() {
        let store = Store::open_in_memory().unwrap();
        let profile = store.create_profile("P", "u1", false).unwrap().id;
        let private = store.create_profile("Private", "u2", true).unwrap().id;
        let draft = |id, title: &str, body: &str| Draft { id, title: title.into(), body: body.into() };
        assert!(draft(None, " ", "\n").save(&store, profile).is_err());
        assert!(draft(None, &"t".repeat(MAX_NOTE_TITLE + 1), "x").save(&store, profile).is_err());
        assert!(draft(None, &"é".repeat(MAX_NOTE_TITLE), "x").save(&store, profile).is_ok(), "the limit counts characters");
        assert!(draft(None, "t", &"x".repeat(MAX_NOTE + 1)).save(&store, profile).is_err());
        let saved = draft(None, "Trip", "Day 1").save(&store, profile).unwrap();
        let edited = Draft { body: "Day 2".into(), ..Draft::of(&saved) }.save(&store, profile).unwrap();
        assert_eq!((edited.id, edited.body.as_str()), (saved.id, "Day 2"));
        assert_eq!(draft(None, "Trip", "x").save(&store, private), Err("The note could not be saved.".into()));
        assert!(store.notes(private).unwrap().is_empty());
    }
}
