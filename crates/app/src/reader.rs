use std::time::{Duration, Instant};

use engine::reader::{Article, Block};
use engine::speech::{self, Speech};
use gpui::{
    AnyElement, Context, Div, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString, StatefulInteractiveElement, Styled, Task, Window,
    actions, div, prelude::FluentBuilder, px,
};
use shield::ShieldLevel;
use ui::controls::{button, key_hint, segment, segmented, surface};
use ui::text::bounded_title;
use ui::theme::{Elevation, TEXT, Theme};
use url::Url;

use crate::browser::{Browser, Msg, TabKey};

actions!(tarn, [ToggleReader, SaveForLater]);

const SIZES: [f32; 6] = [14., 16., 18., 20., 23., 26.];
const WIDTHS: [(f32, &str); 2] = [(620., "Narrow"), (820., "Wide")];
const SPEEDS: [f32; 5] = [1., 1.25, 1.5, 2., 0.75];
const POLL: Duration = Duration::from_millis(250);
/// The synthesizer reports speaking a moment after it is asked to; an utterance never heard
/// within this time counts as finished.
const GRACE: Duration = Duration::from_secs(2);

/// The article shown in place of the active tab's page; no web content is involved.
pub struct Reader {
    pub key: TabKey,
    title: SharedString,
    byline: SharedString,
    /// Text, and whether it is a heading.
    blocks: Vec<(SharedString, bool)>,
    /// Stops speaking when the reader is dropped: closed, or its tab closed, navigated or slept.
    speech: Option<Speech>,
    reading: Option<Reading>,
    paused: bool,
    poll: Option<Task<()>>,
}

struct Reading {
    block: usize,
    since: Instant,
    heard: bool,
}

impl Reader {
    /// Reading aloud, paused or not.
    pub fn speaking(&self) -> bool {
        self.reading.is_some()
    }

    /// Queues block `from`, if there is one, at `speed` times the default rate.
    fn speak_from(&mut self, from: usize, speed: f32) {
        self.reading = None;
        let (Some(speech), Some((text, _))) = (&self.speech, self.blocks.get(from)) else { return };
        speech.speak(text, speech::DEFAULT_RATE * speed, None);
        self.reading = Some(Reading { block: from, since: Instant::now(), heard: false });
    }
}

/// Whether the reader survives an overlay closing: only one that speaks, and only when the
/// command bar or shortcut sheet covered it, so closing the reader itself stops the speech.
pub(crate) fn keeps_reader(covered: bool, speaking: bool) -> bool {
    covered && speaking
}

/// Text size, width and reading speed, kept for the window's session.
#[derive(Clone, Copy)]
pub struct Layout {
    size: usize,
    width: usize,
    speed: usize,
}

impl Default for Layout {
    fn default() -> Self {
        Layout { size: 2, width: 0, speed: 0 }
    }
}

pub fn listen(root: Div, cx: &mut Context<Browser>) -> Div {
    root.on_action(cx.listener(|b, _: &ToggleReader, w, cx| b.toggle_reader(w, cx))).on_action(cx.listener(|b, _: &SaveForLater, _, cx| b.save_for_later(cx)))
}

impl Browser {
    /// Asks the active page for its article; the reader opens once the engine answers.
    pub fn toggle_reader(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.reader.as_ref().is_some_and(|r| self.model.active == Some(r.key)) {
            self.reader = None;
            return self.close_command(window, cx);
        }
        let Some(key) = self.model.active else { return };
        let Some((view, url)) = self.views.get(&key).and_then(|v| Some((v, v.url()?))) else { return };
        let events = self.events.clone();
        view.article(&url.clone(), false, move |json| drop(events.unbounded_send(Msg::Article(key, url.clone(), false, json))));
    }

    /// The tab moved on: its reader closes and its button goes until the page is asked again.
    pub(crate) fn forget_article(&mut self, key: TabKey, window: &mut Window, cx: &mut Context<Self>) {
        self.pages.entry(key).or_default().article = false;
        if self.reader.as_ref().is_some_and(|r| r.key == key) {
            self.reader = None;
            if self.model.active == Some(key) {
                self.close_command(window, cx);
            }
        }
    }

    /// Asks a web page that finished loading whether it has an article, for the toolbar's button;
    /// never under Lockdown, where the shortcut still works.
    pub(crate) fn probe_article(&mut self, key: TabKey, url: &Url) {
        if !matches!(url.scheme(), "http" | "https") || self.level_for_tab(key, url.as_str()) == ShieldLevel::Lockdown {
            return;
        }
        let Some(view) = self.views.get(&key) else { return };
        let (events, url) = (self.events.clone(), url.clone());
        view.article(&url.clone(), true, move |found| drop(events.unbounded_send(Msg::Article(key, url.clone(), true, found))));
    }

    pub(crate) fn on_article(&mut self, key: TabKey, url: Url, probe: bool, text: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.views.get(&key).and_then(|v| v.url()).as_ref() != Some(&url) {
            return;
        }
        if probe {
            self.pages.entry(key).or_default().article = text == "1";
            return cx.notify();
        }
        // A late answer must not replace an overlay the user opened since asking.
        if self.model.active != Some(key) || self.overlay_open() {
            return;
        }
        let Some(Article { title, byline, blocks }) = Article::parse(&text) else {
            return self.show_toast("No article found on this page", None, cx);
        };
        self.close_command(window, cx);
        let blocks = blocks
            .into_iter()
            .map(|b| match b {
                Block::Heading(t) => (t.into(), true),
                Block::Paragraph(t) => (t.into(), false),
            })
            .collect();
        self.reader =
            Some(Reader { key, title: bounded_title(&title).into(), byline: byline.into(), blocks, speech: None, reading: None, paused: false, poll: None });
        if let Some(view) = self.active_view() {
            view.set_visible(false);
        }
        self.layout_changed();
        if let Some(host) = &self.host {
            host.focus();
        }
        cx.notify();
    }

    pub(crate) fn drop_reader(&mut self, key: TabKey) {
        if self.reader.as_ref().is_some_and(|r| r.key == key) {
            self.reader = None;
        }
    }

    /// Starts reading aloud, or pauses and resumes.
    fn read_aloud(&mut self, cx: &mut Context<Self>) {
        let speed = SPEEDS[self.reader_layout.speed];
        let Some(reader) = self.reader.as_mut() else { return };
        if let (Some(speech), Some(_)) = (&reader.speech, &reader.reading) {
            if reader.paused {
                speech.resume()
            } else {
                speech.pause()
            }
            reader.paused = !reader.paused;
            return cx.notify();
        }
        if reader.speech.is_none() {
            reader.speech = Speech::new();
        }
        reader.paused = false;
        reader.speak_from(0, speed);
        if reader.reading.is_none() {
            return self.show_toast(if cfg!(windows) { "This PC could not start speaking" } else { "This Mac could not start speaking" }, None, cx);
        }
        let key = reader.key;
        reader.poll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                if !this.update(cx, |b, cx| b.reader_tick(key, cx)).unwrap_or(false) {
                    break;
                }
            }
        }));
        cx.notify();
    }

    fn stop_reading(&mut self, cx: &mut Context<Self>) {
        let Some(reader) = self.reader.as_mut() else { return };
        if let Some(speech) = &reader.speech {
            speech.stop();
        }
        (reader.reading, reader.paused, reader.poll) = (None, false, None);
        cx.notify();
    }

    /// Moves on to the next block once the synthesizer is done; false once there is nothing left.
    fn reader_tick(&mut self, key: TabKey, cx: &mut Context<Self>) -> bool {
        let speed = SPEEDS[self.reader_layout.speed];
        let Some(reader) = self.reader.as_mut().filter(|r| r.key == key) else { return false };
        let (Some(speech), Some(reading)) = (&reader.speech, &mut reader.reading) else { return false };
        if speech.is_speaking() {
            reading.heard = true;
            return true;
        }
        if reader.paused || (!reading.heard && reading.since.elapsed() < GRACE) {
            return true;
        }
        let next = reading.block + 1;
        reader.speak_from(next, speed);
        cx.notify();
        reader.reading.is_some()
    }

    fn set_reader_layout(&mut self, f: impl FnOnce(&mut Layout), cx: &mut Context<Self>) {
        f(&mut self.reader_layout);
        cx.notify();
    }
}

/// The reader for the active tab, if it is open on it.
pub fn render(b: &Browser, cx: &mut Context<Browser>) -> Option<AnyElement> {
    let reader = b.reader.as_ref().filter(|r| b.model.active == Some(r.key))?;
    let Layout { size, width, speed } = b.reader_layout;
    let speak = match (&reader.reading, reader.paused) {
        (None, _) => "Read Aloud",
        (Some(_), true) => "Resume",
        (Some(_), false) => "Pause",
    };
    let text = SIZES[size];
    let controls = div()
        .flex()
        .items_center()
        .gap_2()
        .child(button("reader-smaller", "A−", false).on_click(cx.listener(|b, _, _, cx| b.set_reader_layout(|l| l.size = l.size.saturating_sub(1), cx))))
        .child(
            button("reader-larger", "A+", false).on_click(cx.listener(|b, _, _, cx| b.set_reader_layout(|l| l.size = (l.size + 1).min(SIZES.len() - 1), cx))),
        )
        .child(segmented().children(WIDTHS.iter().enumerate().map(|(ix, (_, name))| {
            segment(("reader-width", ix), *name, ix == width).on_click(cx.listener(move |b, _, _, cx| b.set_reader_layout(|l| l.width = ix, cx)))
        })))
        .child(button("reader-speak", speak, false).on_click(cx.listener(|b, _, _, cx| b.read_aloud(cx))))
        .when(reader.reading.is_some(), |d| d.child(button("reader-stop", "Stop", false).on_click(cx.listener(|b, _, _, cx| b.stop_reading(cx)))))
        .child(
            button("reader-speed", format!("{}×", SPEEDS[speed]), false)
                .on_click(cx.listener(|b, _, _, cx| b.set_reader_layout(|l| l.speed = (l.speed + 1) % SPEEDS.len(), cx))),
        )
        .child(button("reader-save", "Save for Later", false).on_click(cx.listener(|b, _, _, cx| b.save_for_later(cx))))
        .child(key_hint("Esc to close"));
    let reading = reader.reading.as_ref().map(|r| r.block);
    let blocks = reader.blocks.iter().enumerate().map(|(ix, (t, heading))| {
        let block = div().rounded(px(Theme::radius_small())).when(reading == Some(ix), |d| d.bg(Theme::selected()));
        match heading {
            true => block.pt(px(text * 0.6)).text_size(px(text * 1.2)).font_weight(FontWeight::SEMIBOLD).child(t.clone()),
            false => block.child(t.clone()),
        }
    });
    let page = div()
        .w_full()
        .max_w(px(WIDTHS[width].0))
        .flex()
        .flex_col()
        .gap(px(text * 0.9))
        .text_size(px(text))
        .line_height(px(text * 1.6))
        .child(div().text_size(px(text * 1.7)).line_height(px(text * 2.1)).font_weight(FontWeight::BOLD).child(reader.title.clone()))
        .when(!reader.byline.is_empty(), |d| d.child(div().text_size(px(TEXT)).text_color(Theme::muted()).child(reader.byline.clone())))
        .children(blocks);
    // The controls stay put while the article scrolls under them.
    let article = div().id("reader").flex_1().min_h_0().w_full().overflow_y_scroll().flex().flex_col().items_center().px_6().pb_6().child(page);
    let card = surface(Theme::radius_page())
        .size_full()
        .shadow(Elevation::Page.shadow())
        .flex()
        .flex_col()
        .gap_6()
        .child(div().flex_none().w_full().px_6().pt_6().flex().justify_end().child(controls))
        .child(article);
    Some(div().flex_1().min_h_0().flex().child(card).into_any_element())
}

#[cfg(test)]
mod tests {
    use super::keeps_reader;

    #[test]
    fn only_a_speaking_reader_under_an_overlay_survives_it_closing() {
        assert!(keeps_reader(true, true));
        assert!(!keeps_reader(true, false));
        assert!(!keeps_reader(false, true));
        assert!(!keeps_reader(false, false));
    }
}
