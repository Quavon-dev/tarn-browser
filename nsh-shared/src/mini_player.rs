use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use browser_core::media::Control;
use nsh_core::platform::Lifecycle;
use nsh_core::prelude::*;

use crate::controls::{icon_button, icon_size, label, pressable};
use crate::icons;
use crate::motion::popping;
use crate::theme::Theme;

/// The mini-player of `media`, a title and whether it plays: pressing the title shows its tab,
/// the buttons send `Control`s. Nothing shows while `media` gives none.
pub fn mini_player(
    media: impl Fn() -> Option<(String, bool)> + Copy + 'static,
    open: impl Fn() + Copy + 'static,
    press: impl Fn(Control) + Copy + 'static,
) -> AnyElement {
    let shown = computed(move || media().is_some());
    let title = move || media().map(|m| m.0).unwrap_or_default();
    let playing = move || media().is_some_and(|m| m.1);
    dynamic(move || {
        shown.get().then(|| {
            let buttons: Vec<AnyElement> = Control::ALL
                .into_iter()
                .map(|control| {
                    dynamic(move || {
                        let playing = playing();
                        Some(icon_button(icons::icon(icons::glyph(control.glyph(playing)), icon_size()), control.label(playing), move || press(control)))
                    })
                    .into_element()
                })
                .collect();
            popping(
                view()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .p_1()
                    .rounded(px(Theme::radius_panel()))
                    .bg(Theme::panel())
                    .border_1()
                    .border_color(Theme::surface_edge())
                    .child(
                        pressable(move || Some(SharedString::from(title())), open)
                            .flex_1()
                            .min_w(px(0.))
                            .px_2()
                            .py_1()
                            .rounded(px(Theme::radius_row()))
                            .child(label(title).number_of_lines(1_u32)),
                    )
                    .children(buttons),
            )
        })
    })
    .into_element()
}

/// Runs `tick` every `period` while `running` holds and the app is active, with no timer otherwise.
pub fn every(period: Duration, running: impl Fn() -> bool + 'static, tick: impl Fn() + Clone + 'static) {
    let timer: Rc<RefCell<Option<Spawned<()>>>> = Rc::default();
    effect(move || {
        if let Some(old) = timer.borrow_mut().take() {
            old.cancel();
        }
        if lifecycle() == Lifecycle::Active && running() {
            let tick = tick.clone();
            *timer.borrow_mut() = Some(untracked(|| set_interval(period, tick)));
        }
    });
}
