use crate::tabs::SpaceRef;

/// A space switch by hand keeps the window off the schedules this long, in ms.
pub const MANUAL_HOLD: i64 = 3_600_000;

/// The first non-private space whose schedule covers `hour` of `weekday`.
pub fn scheduled(spaces: &[SpaceRef], weekday: u8, hour: u8) -> Option<usize> {
    spaces.iter().position(|s| !s.private && s.space.schedule.covers(weekday, hour))
}

/// The space a window on `current` switches to now, unless the user changed space by hand within
/// `MANUAL_HOLD` of `now` (`switched_at`, in ms).
pub fn due(spaces: &[SpaceRef], current: usize, weekday: u8, hour: u8, switched_at: Option<i64>, now: i64) -> Option<usize> {
    if switched_at.is_some_and(|at| now - at < MANUAL_HOLD) {
        return None;
    }
    scheduled(spaces, weekday, hour).filter(|ix| *ix != current)
}

#[cfg(test)]
mod tests {
    use store::{Id, Schedule, Space};

    use super::*;

    fn space(id: Id, profile_id: Id, name: &str, private: bool) -> SpaceRef {
        SpaceRef {
            private,
            space: Space { id, profile_id, name: name.into(), icon: String::new(), position: 0, color: String::new(), schedule: Schedule::default() },
        }
    }

    #[test]
    fn the_first_scheduled_ordinary_space_wins() {
        let mut spaces = [space(1, 1, "Home", false), space(2, 1, "Work", false), space(3, 1, "Late", false), space(1, 9, "Private", true)];
        spaces[1].space.schedule = Schedule { days: 0b011_1110, start: 9, end: 17 };
        spaces[2].space.schedule = Schedule { days: 0b011_1110, start: 12, end: 2 };
        spaces[3].space.schedule = Schedule { days: 0b111_1111, start: 0, end: 0 };
        assert_eq!(scheduled(&spaces, 1, 10), Some(1));
        assert_eq!(scheduled(&spaces, 1, 13), Some(1));
        assert_eq!(scheduled(&spaces, 1, 20), Some(2));
        assert_eq!(scheduled(&spaces, 6, 1), Some(2));
        assert_eq!(scheduled(&spaces, 6, 10), None);
    }

    #[test]
    fn a_manual_switch_holds_the_schedule_for_an_hour() {
        let mut spaces = [space(1, 1, "Home", false), space(2, 1, "Work", false)];
        spaces[1].space.schedule = Schedule { days: 0b011_1110, start: 9, end: 17 };
        assert_eq!(due(&spaces, 0, 1, 10, None, 0), Some(1));
        assert_eq!(due(&spaces, 1, 1, 10, None, 0), None);
        assert_eq!(due(&spaces, 0, 1, 10, Some(1_000), 1_000 + MANUAL_HOLD - 1), None);
        assert_eq!(due(&spaces, 0, 1, 10, Some(1_000), 1_000 + MANUAL_HOLD), Some(1));
    }
}
