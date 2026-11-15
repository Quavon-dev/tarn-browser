use store::{Id, SavedWindow, Store};

/// `[x, y, width, height]` in global screen points, origin top-left.
pub type Frame = [f64; 4];

/// A window keeps its frame while this much of it shows on some screen.
const MIN_VISIBLE: f64 = 80.;

/// `frame`, or, when no screen shows enough of it, the frame moved and shrunk onto the first screen.
pub fn onto_screen(frame: Frame, screens: &[Frame]) -> Frame {
    let [x, y, w, h] = frame;
    let shows = |s: &Frame| (x + w).min(s[0] + s[2]) - x.max(s[0]) >= MIN_VISIBLE && (y + h).min(s[1] + s[3]) - y.max(s[1]) >= MIN_VISIBLE;
    match screens.first() {
        Some(_) if screens.iter().any(shows) => frame,
        Some(&[sx, sy, sw, sh]) => {
            let (w, h) = (w.min(sw), h.min(sh));
            [sx + (sw - w) / 2., sy + (sh - h) / 2., w, h]
        }
        None => frame,
    }
}

/// The space and tab a saved window reopens on, each `None` when it is gone or the tab moved to
/// another space; a `None` space means the default space.
pub fn restore(window: &SavedWindow, store: &Store) -> (Option<Id>, Option<Id>) {
    let spaces = store.profiles().unwrap_or_default().into_iter().flat_map(|p| store.spaces(p.id).unwrap_or_default());
    let space = window.space_id.filter(|id| spaces.into_iter().any(|s| s.id == *id));
    let tab = space.and(window.active_tab_id).filter(|id| store.tab(*id).is_ok_and(|t| Some(t.space_id) == space));
    (space, tab)
}

/// What a window saves of its place: nothing for the private space, whose rows never reach disk.
pub fn saved_place(private: bool, space: Option<Id>, tab: Option<Id>) -> (Option<Id>, Option<Id>) {
    if private { (None, None) } else { (space, tab) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use store::TabKind;

    const SCREENS: [Frame; 2] = [[0., 0., 1440., 900.], [1440., -200., 1920., 1080.]];

    #[test]
    fn frames_off_every_screen_move_onto_the_first() {
        assert_eq!(onto_screen([100., 100., 800., 600.], &SCREENS), [100., 100., 800., 600.]);
        assert_eq!(onto_screen([2000., 0., 800., 600.], &SCREENS), [2000., 0., 800., 600.], "on the second screen");
        assert_eq!(onto_screen([-700., 50., 800., 600.], &SCREENS), [-700., 50., 800., 600.], "partly visible");
        assert_eq!(onto_screen([5000., 5000., 800., 600.], &SCREENS), [320., 150., 800., 600.]);
        assert_eq!(onto_screen([-790., 50., 800., 600.], &SCREENS), [320., 150., 800., 600.], "a sliver is not enough");
        assert_eq!(onto_screen([5000., 0., 3000., 2000.], &SCREENS), [0., 0., 1440., 900.], "shrunk to fit");
        assert_eq!(onto_screen([5000., 0., 800., 600.], &[]), [5000., 0., 800., 600.]);
    }

    #[test]
    fn restore_keeps_live_places_and_drops_gone_ones() {
        let store = Store::open_in_memory().unwrap();
        let p = store.create_profile("P", "uuid", false).unwrap();
        let (a, b) = (store.create_space(p.id, "A", "").unwrap(), store.create_space(p.id, "B", "").unwrap());
        let in_a = store.create_tab(a.id, TabKind::Ephemeral, None, "https://a.test/", "").unwrap();
        let in_b = store.create_tab(b.id, TabKind::Ephemeral, None, "https://b.test/", "").unwrap();
        let window = |space_id, active_tab_id| SavedWindow { space_id, active_tab_id, ..Default::default() };
        assert_eq!(restore(&window(Some(a.id), Some(in_a.id)), &store), (Some(a.id), Some(in_a.id)));
        assert_eq!(restore(&window(Some(a.id), Some(in_b.id)), &store), (Some(a.id), None), "a tab of another space");
        assert_eq!(restore(&window(Some(999), Some(in_a.id)), &store), (None, None), "a space that is gone");
        assert_eq!(restore(&window(None, Some(in_a.id)), &store), (None, None));
        assert_eq!(saved_place(true, Some(a.id), Some(in_a.id)), (None, None), "private ids would name disk rows");
        assert_eq!(saved_place(false, Some(a.id), Some(in_a.id)), (Some(a.id), Some(in_a.id)));
    }
}
