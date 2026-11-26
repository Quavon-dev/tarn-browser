use store::{Id, SavedWindow, Store};

/// `[x, y, width, height]` in global screen points, origin top-left.
pub type Frame = [f64; 4];

/// A window keeps its frame while this much of it shows on some screen.
const MIN_VISIBLE: f64 = 80.;

/// `screen` less the `[left, top, right, bottom]` fractions of it the menu bar, Dock or taskbar
/// cover; whole when the fractions leave less than half of it.
pub fn work_area(screen: Frame, insets: [f64; 4]) -> Frame {
    let [l, t, r, b] = insets.map(|v| if v.is_finite() { v.max(0.) } else { 0. });
    if l + r > 0.5 || t + b > 0.5 {
        return screen;
    }
    let [x, y, w, h] = screen;
    [x + w * l, y + h * t, w * (1. - l - r), h * (1. - t - b)]
}

/// `frame` shrunk to the screen that shows it with its top edge on it, so the title bar
/// stays reachable; or, when no screen shows enough of it, moved and shrunk onto the first screen.
pub fn onto_screen(frame: Frame, screens: &[Frame]) -> Frame {
    let [x, y, w, h] = frame;
    let shows = |s: &&Frame| (x + w).min(s[0] + s[2]) - x.max(s[0]) >= MIN_VISIBLE && (y + h).min(s[1] + s[3]) - y.max(s[1]) >= MIN_VISIBLE;
    if let Some(&[sx, sy, sw, sh]) = screens.iter().find(shows) {
        let (x, y) = (if w > sw { sx } else { x }, if h > sh { sy } else { y.max(sy) });
        return [x, y, w.min(sw), h.min(sh)];
    }
    match screens.first() {
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
    fn frames_taller_than_their_screen_fit_it() {
        let screen = [[0., 0., 1280., 720.]];
        assert_eq!(onto_screen([0., -50., 1280., 820.], &screen), [0., 0., 1280., 720.], "centred but taller than the screen");
        assert_eq!(onto_screen([100., -30., 800., 600.], &screen), [100., 0., 800., 600.], "title bar above the screen");
        assert_eq!(onto_screen([100., 500., 800., 600.], &screen), [100., 500., 800., 600.], "may hang below the screen");
        assert_eq!(onto_screen([-700., 50., 800., 600.], &screen), [-700., 50., 800., 600.], "partly off to the side");
        assert_eq!(onto_screen([-2000., 0., 3000., 600.], &screen), [0., 0., 1280., 600.], "wider than the screen");
        assert_eq!(onto_screen([1500., 10., 800., 600.], &SCREENS), [1500., 10., 800., 600.], "on the second screen");
    }

    #[test]
    fn work_area_leaves_out_the_taskbar() {
        let screen = [0., 0., 1000., 800.];
        assert_eq!(work_area(screen, [0., 0., 0., 0.1]), [0., 0., 1000., 720.], "taskbar at the bottom");
        assert_eq!(work_area(screen, [0.1, 0.05, 0., 0.]), [100., 40., 900., 760.], "Dock on the left, menu bar on top");
        assert_eq!(work_area(screen, [0., 0., 0., 0.9]), screen, "a work area this small is not trusted");
        assert_eq!(work_area(screen, [f64::NAN, -0.1, 0., 0.]), screen);
        let [_, y, _, h] = onto_screen([0., -50., 1280., 820.], &[work_area([0., 0., 1280., 720.], [0., 0., 0., 48. / 720.])]);
        assert_eq!((y, h), (0., 672.), "a window fits above the taskbar");
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
