use super::*;

impl SettingsWindow {
    pub(super) fn shortcuts(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let stored = crate::shortcuts::stored(&crate::shared::core(cx).disk);
        let defaults = keymap::keymap(&crate::shortcuts::MAC, &Default::default());
        let changed: Vec<Shortcut> =
            keymap::keymap(&crate::shortcuts::MAC, &stored).into_iter().zip(&defaults).filter(|((k, _), (d, _))| k != d).map(|((_, a), _)| a).collect();
        let recording = self.recording.as_ref().map(|(a, _)| *a);
        let mut n = 0usize;
        let mut pane = div()
            .child(
                div()
                    .pt_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        note(if cfg!(windows) {
                            "Click a shortcut and press the new keys, with Ctrl, or Alt with Shift or a digit; Esc cancels."
                        } else {
                            "Click a shortcut and press the new keys, with ⌘ or ⌃; Esc cancels."
                        })
                        .flex_grow()
                        .flex_basis(px(0.))
                        .pt_0(),
                    )
                    .when(!changed.is_empty(), |d| {
                        d.child(button("shortcut-reset-all", "Reset All", false).on_click(cx.listener(|this, _, _, cx| this.reset_shortcuts(None, cx))))
                    }),
            )
            .children(self.shortcut_error.clone().map(|e| note(e).text_color(Theme::warning())));
        for (title, items) in crate::shortcuts::sections() {
            let mut list = group();
            for (ix, (name, keys)) in items.iter().enumerate() {
                let mut line = row(ix == 0).child(label(*name));
                line = match keys {
                    Fixed(keys) => line.child(value(ui::controls::keys(keys))),
                    Bound(action) => {
                        let action = *action;
                        n += 1;
                        let on = recording == Some(action);
                        let custom = changed.contains(&action);
                        let shown = match crate::shortcuts::gpui_action(action) {
                            _ if on => "Press keys…".to_owned(),
                            Some(gpui) => crate::shortcuts::shown(gpui, cx),
                            None => String::new(),
                        };
                        line.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .when(custom, |d| {
                                    d.child(
                                        button(("shortcut-reset", n), "Reset", false)
                                            .on_click(cx.listener(move |this, _, _, cx| this.reset_shortcuts(Some(action), cx))),
                                    )
                                })
                                .child(button(("shortcut", n), shown, on).on_click(cx.listener(move |this, _, window, cx| this.record(action, window, cx)))),
                        )
                    }
                };
                list = list.child(line);
            }
            pane = pane.child(section(title)).child(list);
        }
        pane.into_any_element()
    }

    /// Catches the next keystroke in the Settings window as `action`'s new keys; a key in another
    /// window stops recording and goes on as usual.
    fn record(&mut self, action: Shortcut, window: &mut Window, cx: &mut Context<Self>) {
        let me = cx.entity().downgrade();
        let settings = window.window_handle().window_id();
        let interceptor = cx.intercept_keystrokes(move |event, window, cx| {
            if window.window_handle().window_id() != settings {
                me.update(cx, |this, cx| {
                    if let Some((_, interceptor)) = this.recording.take() {
                        cx.defer(move |_| drop(interceptor));
                        cx.notify();
                    }
                })
                .ok();
                return;
            }
            cx.stop_propagation();
            let keystroke = event.keystroke.clone();
            if me.update(cx, |this, cx| this.recorded(&keystroke, cx)).unwrap_or(false) {
                cx.defer(crate::shortcuts::rebind);
            }
        });
        self.recording = Some((action, interceptor));
        self.shortcut_error = None;
        cx.notify();
    }

    /// Saves `keystroke` for the recording action unless it is Esc, invalid or taken; true when saved.
    fn recorded(&mut self, keystroke: &gpui::Keystroke, cx: &mut Context<Self>) -> bool {
        let Some((action, interceptor)) = self.recording.take() else { return false };
        cx.defer(move |_| drop(interceptor));
        cx.notify();
        if keystroke.key == "escape" && keystroke.modifiers == gpui::Modifiers::none() {
            return false;
        }
        if cfg!(windows) && keystroke.modifiers.platform {
            self.shortcut_error = Some("The Windows key belongs to Windows; use Ctrl, or Alt with Shift or a digit.".to_owned());
            return false;
        }
        let disk = &crate::shared::core(cx).disk;
        match keymap::save(&crate::shortcuts::MAC, disk, action, &crate::shortcuts::canonical(&keystroke.unparse())) {
            Ok(()) => true,
            Err(e) => {
                self.shortcut_error = Some(e);
                false
            }
        }
    }

    /// Back to the default keys for `action`, unless another action took them meanwhile, or for
    /// every action.
    fn reset_shortcuts(&mut self, action: Option<Shortcut>, cx: &mut Context<Self>) {
        self.recording = None;
        self.shortcut_error = keymap::reset(&crate::shortcuts::MAC, &crate::shared::core(cx).disk, action).err();
        cx.defer(crate::shortcuts::rebind);
        cx.notify();
    }
}
