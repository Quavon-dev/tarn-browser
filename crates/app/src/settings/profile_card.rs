use super::*;

fn week_chart(card: &Card) -> Div {
    const BARS: f32 = 92.;
    let days = browser_core::shield_card::day_totals(&card.week);
    if days.iter().all(|(_, n)| *n <= 0) {
        return group().child(row(true).child(text_block("Last 7 days", "Nothing stopped yet. Each day Shield stops something shows here.")));
    }
    let busiest = days.iter().map(|(_, n)| *n).max().unwrap_or(0);
    let bars = days.iter().map(|(day, n)| {
        let fill = if *n == 0 { 2. } else { (BARS * *n as f32 / busiest.max(1) as f32).max(4.) };
        let top = *n == busiest && busiest > 0;
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_end()
            .gap_1()
            .text_size(px(TEXT_TINY))
            .text_color(Theme::muted())
            .child(if *n > 0 { thousands(*n as u64) } else { String::new() })
            .child(div().w(px(22.)).h(px(fill)).rounded(px(Theme::radius_small())).bg(if top { Theme::space_label() } else { Theme::accent() }))
            .child(*day)
    });
    let total: i64 = days.iter().map(|(_, n)| *n).sum();
    group()
        .p_4()
        .gap_3()
        .child(
            div()
                .flex()
                .justify_between()
                .child(div().font_weight(FontWeight::SEMIBOLD).child("Last 7 days"))
                .child(value(format!("{} protected", thousands(total.max(0) as u64))).text_size(px(TEXT_SMALL))),
        )
        .child(div().h(px(BARS + 34.)).flex().items_end().gap_1().children(bars))
}

fn protected_sites(card: &Card) -> Div {
    let most = card.sites.iter().map(|(_, n)| *n).max().unwrap_or(0).max(1);
    let rows = card.sites.iter().map(|(host, n)| {
        let host = crate::toolbar::host_tail(&bounded_title(host), 40);
        let share = (*n).max(0) as f32 / most as f32;
        div()
            .h(px(28.))
            .flex()
            .items_center()
            .gap_2()
            .child(ui::controls::favicon(&host, 16.))
            .child(div().flex_none().w(px(110.)).one_line(host))
            .child(
                div()
                    .flex_1()
                    .h(px(4.))
                    .rounded_full()
                    .bg(Theme::separator())
                    .child(div().h_full().w(gpui::relative(share)).rounded_full().bg(Theme::accent())),
            )
            .child(value(thousands((*n).max(0) as u64)).w(px(44.)).flex_none().text_right())
    });
    group().p_4().gap_1().child(div().pb_1().font_weight(FontWeight::SEMIBOLD).child("Most protected sites")).children(rows)
}

impl SettingsWindow {
    fn card_frame(&mut self, card: &Card, reduce: bool, window: &mut Window, cx: &mut Context<Self>) -> Option<Arc<RenderImage>> {
        const RATES: [f32; 5] = [12., 12., 8., 6.5, 25.];
        let m = &mut self.motion;
        let key = (card.clone(), !Theme::is_light());
        if m.faces.as_ref().is_none_or(|(k, _)| *k != key) && m.drawing.as_ref() != Some(&key) {
            m.drawing = Some(key.clone());
            let card = key;
            cx.spawn(async move |this, cx| {
                let made = cx.background_spawn({
                    let (card, dark) = card.clone();
                    async move { shield_card::faces(&card, dark) }
                });
                let Some(faces) = made.await else { return };
                this.update(cx, |this, cx| {
                    if this.motion.drawing.as_ref() == Some(&card) {
                        this.motion.drawing = None;
                        this.motion.faces = Some((card, Arc::new(faces)));
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
        let faces = m.faces.as_ref()?.1.clone();
        let flip = if m.flipped { 1. } else { 0. };
        let goal = if reduce || !m.hover { [0., 0., 0., flip, 0.] } else { [m.aim.0, m.aim.1, 1., flip, if m.pressed { 1. } else { 0. }] };
        let passed = m.stepped.replace(Instant::now()).map_or(0.05, |t| t.elapsed().as_secs_f32()).min(0.05);
        for ((at, goal), rate) in m.at.iter_mut().zip(goal).zip(RATES) {
            *at = if reduce || (goal - *at).abs() < 0.002 { goal } else { *at + (goal - *at) * (1. - (-passed * rate).exp()) };
        }
        if m.at != goal {
            window.request_animation_frame();
        }
        if m.drawn.as_ref().is_none_or(|(at, from, _)| *at != m.at || !Arc::ptr_eq(from, &faces)) {
            let [across, down, glow, flip, press] = m.at;
            let image =
                Arc::new(RenderImage::new([image::Frame::new(shield_card::frame(&faces, &ui::tilt::Pose { tilt: (across, down), glow, flip, press }))]));
            if let Some((_, _, old)) = m.drawn.replace((m.at, faces, image)) {
                cx.drop_image(old, Some(window));
            }
        }
        m.drawn.as_ref().map(|(_, _, image)| image.clone())
    }

    pub(super) fn profile_tab(&mut self, s: &Snapshot, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(card) = s.card.clone() else { return note("No profile is open.").into_any_element() };
        let target = s.profile.as_ref().map(|p| (s.private, p.id));
        let frame = self.card_frame(&card, s.reduce, window, cx);
        let [_, _, glow, flip, _] = self.motion.at;
        let facing = (flip * std::f32::consts::PI).cos().abs();
        let (width, height, margin) = (shield_card::W as f32, shield_card::H as f32, shield_card::MARGIN as f32);
        let bounds = self.motion.bounds.clone();
        let face = div()
            .id("shield-card")
            .relative()
            .w(px(width))
            .h(px(height))
            .cursor_pointer()
            .child(
                div()
                    .absolute()
                    .top_0()
                    .h_full()
                    .left(px(width / 2. * (1. - facing)))
                    .w(px(width * facing))
                    .rounded(px(18.))
                    .shadow(Theme::elevation(5. + 5. * glow, 9. + 7. * glow)),
            )
            .children(frame.map(|f| img(f).absolute().left(px(-margin)).top(px(-margin)).w(px(width + 2. * margin)).h(px(height + 2. * margin))))
            .child(canvas(move |at, _, _| bounds.set(at), |_, _, _, _| {}).absolute().size_full())
            .on_mouse_move(cx.listener(|this, moved: &MouseMoveEvent, _, cx| {
                let card = this.motion.bounds.get();
                let from_centre = moved.position - card.center();
                this.motion.aim = ((from_centre.x / (card.size.width / 2.)).clamp(-1., 1.), (from_centre.y / (card.size.height / 2.)).clamp(-1., 1.));
                cx.notify();
            }))
            .on_hover(cx.listener(|this, over: &bool, _, cx| {
                this.motion.hover = *over;
                this.motion.pressed &= *over;
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.motion.pressed = true;
                    cx.notify();
                }),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.motion.pressed = false;
                this.motion.flipped ^= true;
                cx.notify();
            }));
        let hero = self.hero(&card, face, target, cx);
        let mut page = div()
            .child(hero)
            .children(self.card_notice.clone().map(|n| note(n).pt_2()))
            .child(div().pt_5().flex().gap_4().child(week_chart(&card).flex_1()).when(!card.sites.is_empty(), |d| d.child(protected_sites(&card).flex_1())));
        if s.default_browser == Some(false) {
            page = page.child(section("Default browser")).child(
                group().child(
                    row(true)
                        .child(text_block("Tarn isn't your default browser", "Links from other apps open in another browser."))
                        .child(button("make-default", "Make Default", true).on_click(cx.listener(|this, _, _, cx| this.make_default(cx)))),
                ),
            );
        }
        let levels = self.shield_levels(s, cx);
        let to_privacy =
            |id: &'static str, text: &'static str| button(id, text, false).on_click(cx.listener(|this, _, _, cx| this.set_pane(SettingsPane::Privacy, cx)));
        let overrides = match s.overrides.len() {
            0 => "None".to_owned(),
            1 => "1 site".to_owned(),
            n => format!("{n} sites"),
        };
        page = page
            .child(note({
                let kept = if s.private { "Counts stay in memory and are gone once the private space closes.".to_owned() } else { format!("Counted on {} only and never sent anywhere.", ui::controls::THIS_DEVICE) };
                if cfg!(windows) {
                    format!("{kept} Only what Tarn stops itself counts.")
                } else {
                    format!("{kept} Only what Tarn stops itself counts: ad and tracker blocks in WebKit tabs are not counted, because WebKit does not report what its blocking rules stop.")
                }
            }))
            .child(section("Shield level"))
            .child(levels)
            .child(note("Applies to this profile. Open pages reload with the new level."))
            .child(section("Shield"))
            .child(
                group()
                    .child(row(true).child(label("Site overrides")).child(value(overrides)).child(to_privacy("shield-change", "Change…")))
                    .child(row(false).child(text_block("Filter lists", s.filters.summary()))),
            );
        let filter_lists = match (filters::configured(), s.filters.updates) {
            (false, _) => "Not set up",
            (true, true) => "GitHub, once a day",
            (true, false) => "Off",
        };
        let updates = match (s.updater, s.update_checks) {
            (false, _) => "Not set up",
            (true, true) => "GitHub, once a day",
            (true, false) => "Only when you ask",
        };
        page.child(section("Connections Tarn makes"))
            .child(
                group()
                    .child(row(true).child(label("Filter-list updates")).child(value(filter_lists)))
                    .child(row(false).child(label("Extensions and themes")).child(value(if s.extension_downloads {
                        "GitHub, when you install"
                    } else {
                        "Off"
                    })))
                    .child(row(false).child(label("Site icons")).child(value("The site you are on or its CDN")))
                    .child(row(false).child(label("Update checks")).child(value(updates)))
                    .child(
                        row(false)
                            .child(text_block("Nothing else", "No accounts, no telemetry, no AI requests."))
                            .child(to_privacy("connections-details", "Details…")),
                    ),
            )
            .into_any_element()
    }

    fn hero(&self, card: &Card, face: impl IntoElement, target: Option<(bool, Id)>, cx: &mut Context<Self>) -> Div {
        let private = target.is_some_and(|(private, _)| private);
        let since = match (card.days, &card.since) {
            (Some(days), Some(since)) => browser_core::shield_card::protected_text(days, since),
            _ if private => "Counts stay in memory until the private space closes.".to_owned(),
            _ => "Counting starts with the first thing Shield stops.".to_owned(),
        };
        let saved = card.clone();
        let shared = card.clone();
        let system = cfg!(target_os = "macos");
        let share = if system { "Share Card" } else { "Save as Image" };
        let action = |id: &'static str, glyph, text: &'static str| tab_stop(icon_text_button(id, glyph, text)).justify_center();
        let actions = div()
            .grid()
            .grid_cols(2)
            .gap_2()
            .child(action("flip-card", Icon::Reload, "Flip").on_click(cx.listener(|this, _, _, cx| {
                this.motion.flipped ^= true;
                cx.notify();
            })))
            .child(action("new-design", Icon::Palette, "Change Design").on_click(cx.listener(move |this, _, _, cx| {
                let Some((private, id)) = target else { return };
                this.with_browser(cx, |b, _, cx| {
                    b.store(private).new_shield_design(id).ok();
                    shield_card::touch();
                    cx.notify();
                });
            })))
            .when(!private, |d| {
                d.child(action("share-card", if system { Icon::Share } else { Icon::FileImage }, share).on_click(cx.listener(move |this, _, window, cx| {
                    if system {
                        this.share_card(shared.clone(), window, cx);
                    } else {
                        this.save_card(shared.clone(), cx);
                    }
                })))
            })
            .when(system && !private, |d| {
                d.child(action("save-card", Icon::FileImage, "Save as Image").on_click(cx.listener(move |this, _, _, cx| this.save_card(saved.clone(), cx))))
            });
        div().pt_4().flex().flex_wrap().items_start().gap_6().child(face).child(
            div()
                .flex_1()
                .min_w(px(300.))
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .child(div().text_size(px(TEXT_HEADING)).font_weight(FontWeight::BOLD).one_line(card.name.clone()))
                        .child(value(since).text_size(px(TEXT_SMALL))),
                )
                .child(actions),
        )
    }

    fn share_card(&mut self, card: Card, window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = engine::Host::from_window(window) else { return };
        let at = self.motion.bounds.get();
        cx.spawn(async move |this, cx| {
            let name = engine::downloads::sanitize_filename(&format!("Tarn Shield {}.png", card.name));
            let dark = !Theme::is_light();
            let written = cx
                .background_spawn(async move {
                    let png = shield_card::card_png(&card, dark)?;
                    let path = browser_core::shield_card::export_path(&name).ok()?;
                    std::fs::write(&path, png).ok()?;
                    url::Url::from_file_path(path).ok()
                })
                .await;
            let Some(url) = written else {
                this.update(cx, |this, cx| {
                    this.card_notice = Some("Could not draw the image.".into());
                    cx.notify();
                })
                .ok();
                return;
            };
            let (x, y, w, h) = (f32::from(at.origin.x), f32::from(at.origin.y), f32::from(at.size.width), f32::from(at.size.height));
            host.share(url.as_str(), x.into(), y.into(), w.into(), h.into());
        })
        .detach();
    }
}
