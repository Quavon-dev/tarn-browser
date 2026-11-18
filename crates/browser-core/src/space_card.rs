use store::{Id, Profile, Schedule, Store};

use shield::ShieldLevel;

use crate::profiles::NewProfile;
use crate::tabs::SpaceRef;
use crate::text::is_bidi;

pub const MAX_NAME: usize = 48;
pub const DEFAULT_PROFILE_COLOR: &str = "blue";
pub const DEFAULT_SPACE_ICON: &str = "house";
pub const DEFAULT_SCHEDULE: Schedule = Schedule { days: 0, start: 9, end: 17 };

pub fn valid_name<'a>(raw: &str, taken: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let name: String = name_chars(raw).take(MAX_NAME).collect();
    let name = name.trim_end().to_owned();
    let free = taken.into_iter().all(|t| !t.trim().eq_ignore_ascii_case(&name));
    (!name.is_empty() && free).then_some(name)
}

/// Whether a typed name is longer than `MAX_NAME`, so saving it would cut it.
pub fn name_too_long(raw: &str) -> bool {
    name_chars(raw).count() > MAX_NAME
}

/// What a new profile's name field warns: a name too long to save whole, or another profile's.
pub fn profile_name_note<'a>(raw: &str, taken: impl IntoIterator<Item = &'a str>) -> Option<String> {
    if name_too_long(raw) {
        return Some(format!("Use at most {MAX_NAME} characters."));
    }
    crate::profiles::name_taken(raw, taken).then(|| "Another profile has this name.".to_owned())
}

fn name_chars(raw: &str) -> impl Iterator<Item = char> + '_ {
    raw.trim().chars().filter(|c| !c.is_control() && !is_bidi(*c))
}

pub fn space_name(raw: &str, spaces: &[SpaceRef], private: bool, profile: Option<Id>, editing: Option<Id>) -> Option<String> {
    let taken = spaces.iter().filter(|s| s.private == private && Some(s.space.profile_id) == profile && Some(s.space.id) != editing);
    valid_name(raw, taken.map(|s| s.space.name.as_str()))
}

pub fn next_space_name(spaces: &[SpaceRef], profile: Option<Id>) -> String {
    let count = spaces.iter().filter(|s| !s.private && Some(s.space.profile_id) == profile).count();
    (count + 1..).map(|n| format!("Space {n}")).find(|n| space_name(n, spaces, false, profile, None).is_some()).unwrap_or_default()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpaceDraft {
    pub editing: Option<Id>,
    pub private: bool,
    pub profile: Option<Id>,
    pub name: String,
    pub color: String,
    pub icon: String,
    pub schedule: Schedule,
    pub profile_name: String,
    pub profile_color: String,
}

#[derive(Debug)]
pub enum Unsaved {
    Invalid,
    Failed(store::Error),
}

impl SpaceDraft {
    pub fn new(spaces: &[SpaceRef], current: Option<&SpaceRef>, profiles: &[Profile]) -> Self {
        let profile = current.filter(|s| !s.private).map(|s| s.space.profile_id).or_else(|| profiles.first().map(|p| p.id));
        SpaceDraft {
            editing: None,
            private: false,
            profile,
            name: next_space_name(spaces, profile),
            color: String::new(),
            icon: DEFAULT_SPACE_ICON.into(),
            schedule: DEFAULT_SCHEDULE,
            profile_name: String::new(),
            profile_color: DEFAULT_PROFILE_COLOR.into(),
        }
    }

    pub fn edit(space: &SpaceRef) -> Self {
        SpaceDraft {
            editing: Some(space.space.id),
            private: space.private,
            profile: Some(space.space.profile_id),
            name: space.space.name.clone(),
            color: space.space.color.clone(),
            icon: space.space.icon.clone(),
            schedule: Some(space.space.schedule).filter(|s| s.days != 0).unwrap_or(DEFAULT_SCHEDULE),
            profile_name: String::new(),
            profile_color: String::new(),
        }
    }

    pub fn ready(&self, spaces: &[SpaceRef], profiles: &[Profile]) -> Option<(String, Option<String>)> {
        if let Some(id) = self.editing {
            spaces.iter().find(|s| s.private == self.private && s.space.id == id)?;
        }
        let name = space_name(&self.name, spaces, self.private, self.profile, self.editing)?;
        let profile_name = match self.profile {
            Some(_) => None,
            None if name_too_long(&self.profile_name) => return None,
            None => Some(valid_name(&self.profile_name, profiles.iter().map(|p| p.name.as_str()))?),
        };
        Some((name, profile_name))
    }

    pub fn save(&self, disk: &Store, store: &Store, spaces: &[SpaceRef], profiles: &[Profile]) -> Result<Id, Unsaved> {
        let (name, profile_name) = self.ready(spaces, profiles).ok_or(Unsaved::Invalid)?;
        let written = match (self.editing, self.profile, profile_name) {
            (Some(id), _, _) => store.batch(|store| {
                store.update_space(id, &name, &self.color, &self.icon)?;
                if !self.private {
                    store.set_space_schedule(id, self.schedule)?;
                }
                Ok(id)
            }),
            (None, Some(profile), _) => store.batch(|store| {
                let id = store.create_space(profile, &name, &self.icon)?.id;
                store.update_space(id, &name, &self.color, &self.icon)?;
                Ok(id)
            }),
            (None, None, Some(profile_name)) => {
                let profile = NewProfile {
                    name: &profile_name,
                    color: &self.profile_color,
                    level: ShieldLevel::Standard,
                    engine: crate::command::ENGINES[0].name,
                    share: None,
                    space: [&name, &self.color, &self.icon],
                };
                crate::profiles::create(disk, &profile, &uuid::Uuid::new_v4().to_string()).map(|(_, space)| space)
            }
            (None, None, None) => return Err(Unsaved::Invalid),
        };
        written.map_err(Unsaved::Failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use store::Space;

    fn space(id: Id, profile_id: Id, name: &str, private: bool) -> SpaceRef {
        SpaceRef {
            private,
            space: Space { id, profile_id, name: name.into(), icon: String::new(), position: 0, color: String::new(), schedule: Schedule::default() },
        }
    }

    fn refs(store: &Store) -> Vec<SpaceRef> {
        store.profiles().unwrap().iter().flat_map(|p| store.spaces(p.id).unwrap()).map(|space| SpaceRef { private: false, space }).collect()
    }

    #[test]
    fn names_are_trimmed_cleaned_and_unique() {
        assert_eq!(valid_name("  Work  ", []), Some("Work".into()));
        assert_eq!(valid_name("   ", []), None);
        assert_eq!(valid_name("Wo\u{202e}rk\n", []), Some("Work".into()));
        assert_eq!(valid_name("work", ["Personal", " Work"]), None);
        assert_eq!(valid_name(&"x".repeat(200), []).map(|n| n.chars().count()), Some(MAX_NAME));
        assert!(name_too_long(&"x".repeat(MAX_NAME + 1)));
        assert!(!name_too_long(&format!("  {}\u{202e}  ", "x".repeat(MAX_NAME))));
        for wide in ['ü', '\u{1F600}', '\u{6F22}'] {
            let at_limit = format!("{}{wide}", "x".repeat(MAX_NAME - 1));
            assert!(!name_too_long(&at_limit) && valid_name(&at_limit, []) == Some(at_limit.clone()));
            let over = format!("{}{wide}{wide}{wide}", "x".repeat(MAX_NAME - 2));
            assert!(name_too_long(&over));
            assert_eq!(valid_name(&over, []).map(|n| n.chars().count()), Some(MAX_NAME));
        }
        assert!(name_too_long(&"\u{6F22}".repeat(200)));
    }

    #[test]
    fn a_new_profile_name_warns_when_too_long_or_taken() {
        assert_eq!(profile_name_note(&"x".repeat(MAX_NAME + 1), ["Work"]), Some(format!("Use at most {MAX_NAME} characters.")));
        assert_eq!(profile_name_note(" work ", ["Work"]), Some("Another profile has this name.".into()));
        assert_eq!(profile_name_note(&"x".repeat(MAX_NAME), ["Work"]), None);
        assert_eq!(profile_name_note("", ["Work"]), None);
    }

    #[test]
    fn a_new_profile_name_too_long_is_not_cut_on_save() {
        let disk = Store::open_in_memory().unwrap();
        let home = disk.create_profile("Personal", "uuid", false).unwrap();
        disk.create_space(home.id, "Home", "").unwrap();
        let profiles = disk.profiles().unwrap();
        let mut draft = SpaceDraft::new(&refs(&disk), None, &profiles);
        draft.profile = None;
        draft.profile_name = "x".repeat(MAX_NAME + 1);
        assert_eq!(draft.ready(&refs(&disk), &profiles), None);
        draft.profile_name = "x".repeat(MAX_NAME);
        assert!(draft.ready(&refs(&disk), &profiles).is_some_and(|(_, p)| p == Some("x".repeat(MAX_NAME))));
    }

    #[test]
    fn space_names_are_unique_within_their_profile_only() {
        let spaces = [space(1, 1, "Work", false), space(2, 2, "Home", false), space(1, 9, "Private", true)];
        assert_eq!(space_name(" work ", &spaces, false, Some(1), None), None);
        assert_eq!(space_name("Work", &spaces, false, Some(1), Some(1)), Some("Work".into()));
        assert_eq!(space_name("Work", &spaces, false, Some(2), None), Some("Work".into()));
        assert_eq!(space_name("Work", &spaces, false, None, None), Some("Work".into()));
        assert_eq!(space_name("private", &spaces, false, Some(9), None), Some("private".into()));
        assert_eq!(space_name("private", &spaces, true, Some(9), None), None);
        assert_eq!(space_name("  ", &spaces, false, Some(1), None), None);
    }

    #[test]
    fn a_new_space_takes_the_next_free_name_of_the_current_profile() {
        let spaces = [space(1, 1, "Home", false), space(2, 1, "Space 3", false), space(3, 2, "Home", false), space(4, 9, "Private", true)];
        let profiles =
            Store::open_in_memory().unwrap().create_profile("Personal", "uuid", false).into_iter().map(|p| Profile { id: 1, ..p }).collect::<Vec<_>>();
        let draft = SpaceDraft::new(&spaces, spaces.get(2), &profiles);
        assert_eq!((draft.profile, draft.name.as_str(), draft.profile_color.as_str()), (Some(2), "Space 2", DEFAULT_PROFILE_COLOR));
        let draft = SpaceDraft::new(&spaces, spaces.get(3), &profiles);
        assert_eq!((draft.profile, draft.name.as_str()), (Some(1), "Space 4"));
        assert_eq!(next_space_name(&spaces, Some(3)), "Space 1");
    }

    #[test]
    fn saving_creates_edits_and_creates_with_a_new_profile() {
        let disk = Store::open_in_memory().unwrap();
        let home = disk.create_profile("Personal", "uuid", false).unwrap();
        disk.create_space(home.id, "Home", "").unwrap();
        let profiles = disk.profiles().unwrap();

        let mut draft = SpaceDraft::new(&refs(&disk), None, &profiles);
        draft.color = "blue".into();
        draft.icon = "code".into();
        let id = draft.save(&disk, &disk, &refs(&disk), &profiles).unwrap();
        let made = refs(&disk).into_iter().find(|s| s.space.id == id).unwrap();
        assert_eq!((made.space.name.as_str(), made.space.color.as_str(), made.space.icon.as_str()), ("Space 2", "blue", "code"));

        let mut edit = SpaceDraft::edit(&made);
        edit.name = "Home".into();
        assert!(matches!(edit.save(&disk, &disk, &refs(&disk), &profiles), Err(Unsaved::Invalid)));
        edit.name = "Code".into();
        assert!(matches!(SpaceDraft { editing: Some(999), ..edit.clone() }.save(&disk, &disk, &refs(&disk), &profiles), Err(Unsaved::Invalid)));
        (edit.color, edit.icon) = (String::new(), "star".into());
        edit.schedule.days = 0b011_1110;
        edit.save(&disk, &disk, &refs(&disk), &profiles).unwrap();
        let saved = refs(&disk).into_iter().find(|s| s.space.id == id).unwrap();
        assert_eq!(
            (saved.space.name.as_str(), saved.space.color.as_str(), saved.space.icon.as_str(), saved.space.schedule),
            ("Code", "", "star", Schedule { days: 0b011_1110, start: 9, end: 17 })
        );

        let mut fresh = SpaceDraft::new(&refs(&disk), None, &profiles);
        fresh.profile = None;
        fresh.profile_name = "personal".into();
        assert!(matches!(fresh.save(&disk, &disk, &refs(&disk), &profiles), Err(Unsaved::Invalid)));
        fresh.profile_name = "Work".into();
        fresh.profile_color = "green".into();
        let id = fresh.save(&disk, &disk, &refs(&disk), &profiles).unwrap();
        let work = disk.profiles().unwrap().into_iter().find(|p| p.name == "Work").unwrap();
        assert_eq!(work.color, "green");
        assert_eq!(disk.spaces(work.id).unwrap().iter().map(|s| s.id).collect::<Vec<_>>(), [id]);
        assert_eq!(work.shield_level, shield::ShieldLevel::Standard);
        assert_eq!(disk.setting(&crate::command::engine_key(false, work.id)).unwrap().as_deref(), Some(crate::command::ENGINES[0].name));
        assert_eq!(disk.setting(&crate::shared_key(work.id)).unwrap().as_deref(), Some(""), "a new profile keeps its own website data, as on macOS");
    }

    #[test]
    fn a_new_space_starts_with_the_first_space_icon() {
        let disk = Store::open_in_memory().unwrap();
        disk.create_profile("Personal", "uuid", false).unwrap();
        let draft = SpaceDraft::new(&refs(&disk), None, &disk.profiles().unwrap());
        assert_eq!(draft.icon, DEFAULT_SPACE_ICON);
    }
}
