use std::rc::Rc;

use engine::{Event, Host, WebView};
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    Bounds, Context, FontWeight, IntoElement, ParentElement, Pixels, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, WeakEntity,
    Window, WindowHandle, div, point, px, size,
};
use store::{Id, TabKind};
use ui::controls::{button, icon_button, open_popup, surface, tooltip};
use ui::icon::Icon;
use ui::motion::pop_in;
use ui::text::OneLine;
use ui::text::bounded_title;
use ui::theme::{Secure, TEXT, TEXT_SMALL, TOOLBAR_HEIGHT, Theme};
use url::Url;

use crate::browser::{Browser, TabKey, page_corner};
use crate::prefs::Pref;

const MAX_WIDTH: f32 = 1100.;
const MARGIN: f32 = 80.;
const HEADER: f32 = 44.;
const PAD: f32 = 8.;
/// Room around the card for its own shadow; the window has none.
const SHADOW: f32 = 24.;

/// A link opened over the page in its own web view, in the opener's profile and data store.
#[derive(Default)]
pub struct LinkPeek {
    pub(crate) window: Option<WindowHandle<LinkPeekWindow>>,
    pub(crate) view: Option<Rc<WebView>>,
    host: Option<Host>,
    page: (f64, f64, f64, f64),
    profile: Option<(bool, Id)>,
    /// The origin the peeked link was judged from, which also judges the peek's first load (S10).
    opener: Option<Url>,
    web_engine: Option<store::WebEngine>,
    generation: u64,
    url: Option<Url>,
    title: String,
    _escape: Option<engine::menu::EventMonitor>,
}

/// The peek window's frame over the main window's page area (right of a `left` wide sidebar), and
/// where its web view sits inside it.
fn layout(main: Bounds<Pixels>, left: f32) -> (Bounds<Pixels>, (f64, f64, f64, f64)) {
    let main = Bounds::new(point(main.origin.x + px(left), main.origin.y), size(main.size.width - px(left), main.size.height));
    let (w, h) = (f32::from(main.size.width), f32::from(main.size.height));
    let card_w = (w - 2. * MARGIN).clamp(320., MAX_WIDTH);
    let card_h = (h - TOOLBAR_HEIGHT - MARGIN / 2.).max(240.);
    let bounds = Bounds::new(
        point(main.origin.x + px((w - card_w) / 2. - SHADOW), main.origin.y + px(TOOLBAR_HEIGHT - SHADOW / 2.)),
        size(px(card_w + 2. * SHADOW), px(card_h + 1.5 * SHADOW)),
    );
    let page = (SHADOW + PAD, SHADOW / 2. + HEADER, card_w - 2. * PAD, card_h - HEADER - PAD);
    (bounds, (page.0.into(), page.1.into(), page.2.into(), page.3.into()))
}

impl LinkPeek {
    pub(crate) fn set_dark(&self, dark: bool) {
        if let Some(view) = &self.view {
            view.set_dark(dark);
        }
    }
}

impl Browser {
    fn sidebar_inset(&self) -> f32 {
        (self.sidebar_width + crate::sidebar::SIDEBAR_MARGIN) * self.sidebar_shown()
    }

    pub fn open_link_peek(&mut self, opener: TabKey, url: Url, from: Option<Url>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(profile_id) = self.profile_of(opener) else { return };
        if self.link_peek.view.is_some() {
            // The open peek's page would judge the new link's redirects (S10), so it gets a view of its own.
            self.close_link_peek(cx);
        }
        self.link_peek.opener = from;
        let (bounds, page) = layout(window.bounds(), self.sidebar_inset());
        let display = window.display(cx).map(|d| d.id());
        let reduce = self.pref(Pref::ReduceMotion);
        let browser = cx.entity().downgrade();
        self.link_peek.profile = Some((opener.private, profile_id));
        // Only WebKit views peek links, so an opener with an engine of its own ran in WebKit.
        self.link_peek.web_engine = self.model.tab(opener).and_then(|t| t.web_engine).map(|_| store::WebEngine::WebKit);
        // The new window renders from the browser at once, so it must open after this update ends.
        cx.defer(move |cx| {
            let inset = [SHADOW / 2., SHADOW, SHADOW, SHADOW];
            let (handle, popup) =
                open_popup(cx, bounds, display, true, inset, Theme::radius_panel(), |window, cx| LinkPeekWindow::new(browser.clone(), reduce, window, cx));
            let (Some(handle), Some(popup)) = (handle, popup) else { return };
            browser.update(cx, |b, cx| b.attach_link_peek(handle, &popup, opener.private, profile_id, url, page, cx)).ok();
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn attach_link_peek(
        &mut self,
        handle: WindowHandle<LinkPeekWindow>,
        popup: &Host,
        private: bool,
        profile_id: Id,
        url: Url,
        page: (f64, f64, f64, f64),
        cx: &mut Context<Self>,
    ) {
        let Some((generation, view)) = self.peek_view(popup, private, profile_id, &url, page) else {
            cx.defer(move |cx| drop(handle.update(cx, |_, window, _| window.remove_window())));
            return;
        };
        // A second peek asked for before the first one's window opened replaces it.
        if let Some(old) = self.link_peek.window.take() {
            cx.defer(move |cx| drop(old.update(cx, |_, window, _| window.remove_window())));
        }
        let (tx, mut rx) = mpsc::unbounded();
        self.link_peek._escape = popup.on_escape(move || drop(tx.unbounded_send(())));
        cx.spawn(async move |this, cx| {
            if rx.next().await.is_some() {
                this.update(cx, |b, cx| b.close_link_peek(cx)).ok();
            }
        })
        .detach();
        self.link_peek = LinkPeek {
            window: Some(handle),
            view: Some(Rc::new(view)),
            host: Some(popup.clone()),
            page,
            url: Some(url),
            generation,
            ..std::mem::take(&mut self.link_peek)
        };
        cx.notify();
    }

    fn peek_view(&mut self, popup: &Host, private: bool, profile_id: Id, url: &Url, (x, y, w, h): (f64, f64, f64, f64)) -> Option<(u64, WebView)> {
        let (generation, sink) = self.peek_sink();
        let view = self.web_view(popup, private, profile_id, url.as_str(), sink)?;
        view.set_corner_radius(page_corner());
        view.set_dark(!ui::theme::Theme::is_light());
        view.set_passkeys(self.pref(Pref::Passkeys));
        view.set_frame(x, y, w, h);
        view.set_visible(true);
        view.load_link(url, self.link_peek.opener.as_ref());
        view.focus();
        Some((generation, view))
    }

    pub(crate) fn on_peek_event(&mut self, generation: u64, event: Event, cx: &mut Context<Self>) {
        if generation != self.link_peek.generation {
            return;
        }
        match event {
            Event::Committed { url, title, .. } | Event::Finished { url, title, .. } => {
                // From here on the peek's own page judges what it loads, also after a rebuild.
                self.link_peek.opener = Some(url.clone());
                self.link_peek.url = Some(url);
                self.link_peek.title = title;
            }
            Event::TitleChanged(title) => self.link_peek.title = title,
            Event::UrlChanged { url, .. } => self.link_peek.url = Some(url),
            Event::PeekLink(url, from) => {
                self.link_peek.opener = from;
                if let Some(view) = &self.link_peek.view {
                    view.load_link(&url, None);
                }
            }
            Event::Blocked { url, category: Some(category), .. } if category.warns() => {
                self.close_link_peek(cx);
                if let Some(key) = self.model.active {
                    self.show_blocked(key, url, category);
                }
            }
            Event::ShieldLevelChanged(url) | Event::RebuildForLink(url) => {
                if let (Some(host), Some((private, profile_id))) = (self.link_peek.host.clone(), self.link_peek.profile)
                    && let Some((generation, view)) = self.peek_view(&host, private, profile_id, &url, self.link_peek.page)
                {
                    self.link_peek.view = Some(Rc::new(view));
                    self.link_peek.generation = generation;
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// Moves the peeked page, with its history, into a new tab of the current space.
    pub fn link_peek_to_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.burner_shown() {
            let url = self.link_peek.url.clone();
            self.close_link_peek(cx);
            return url.into_iter().for_each(|url| self.navigate(&url, window, cx));
        }
        let target = self.model.current_space().map(|s| (s.private, s.space.profile_id, s.space.id));
        let view = self.link_peek.view.take().and_then(|v| Rc::try_unwrap(v).ok());
        let url = view.as_ref().and_then(|v| v.url()).or(self.link_peek.url.clone());
        let title = self.link_peek.title.clone();
        let space = target.filter(|(private, profile, _)| Some((*private, *profile)) == self.link_peek.profile);
        let web_engine = self.link_peek.web_engine;
        self.close_link_peek(cx);
        let (Some(view), Some(host), Some(url), Some((private, _, space_id))) = (view, self.host.clone(), url, space) else { return };
        let Ok(tab) = self.store(private).create_tab(space_id, TabKind::Ephemeral, None, url.as_str(), &title) else { return };
        self.store(private).set_tab_web_engine(tab.id, web_engine).ok();
        let key = TabKey { private, id: tab.id };
        let view = view.rehost(&host, self.sink(key));
        self.view_prefs(&view);
        self.views.insert(key, Rc::new(view));
        self.attach_tarn(key);
        self.reload_space();
        self.activate(key, window, cx);
    }

    pub(crate) fn link_peek_profile(&self) -> Option<(bool, Id)> {
        self.link_peek.profile
    }

    pub fn close_link_peek(&mut self, cx: &mut Context<Self>) {
        let peek = std::mem::take(&mut self.link_peek);
        if let Some(handle) = peek.window {
            cx.defer(move |cx| drop(handle.update(cx, |_, window, _| window.remove_window())));
        }
        if let Some(view) = self.active_view() {
            view.focus();
        }
        cx.notify();
    }
}

pub struct LinkPeekWindow {
    browser: WeakEntity<Browser>,
    reduce: bool,
    _subscriptions: Vec<Subscription>,
}

impl LinkPeekWindow {
    fn new(browser: WeakEntity<Browser>, reduce: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = vec![cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                this.browser.update(cx, |b, cx| b.close_link_peek(cx)).ok();
            }
        })];
        subscriptions.extend(browser.upgrade().map(|b| cx.observe(&b, |_, _, cx| cx.notify())));
        LinkPeekWindow { browser, reduce, _subscriptions: subscriptions }
    }

    fn with_main(&self, cx: &mut Context<Self>, run: impl FnOnce(&mut Browser, &mut Window, &mut Context<Browser>) + 'static) {
        let Some(main) = self.browser.upgrade().and_then(|b| b.read(cx).peek.main) else { return };
        cx.defer(move |cx| drop(main.update(cx, run)));
    }
}

impl Render for LinkPeekWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(browser) = self.browser.upgrade() else { return div().into_any_element() };
        let b = browser.read(cx);
        let crate::toolbar::Site { host, insecure, .. } = crate::toolbar::site_of(b.link_peek.url.clone());
        let title = bounded_title(&b.link_peek.title);
        let header = div()
            .h(px(HEADER))
            .flex_none()
            .pl_3()
            .pr_2()
            .flex()
            .items_center()
            .gap_2()
            .child(peek_origin(host, insecure))
            .child(div().flex_1().text_size(px(TEXT_SMALL)).text_color(Theme::muted()).one_line(SharedString::from(title)))
            .child(button("peek-open", "Open in Tab", true).on_click(cx.listener(|this, _, _, cx| this.with_main(cx, |b, w, cx| b.link_peek_to_tab(w, cx)))))
            .child(icon_button("peek-close", Icon::Close, true).tooltip(tooltip("Close", Some("Esc".into()))).on_click(cx.listener(|this, _, _, cx| {
                this.browser.update(cx, |b, cx| b.close_link_peek(cx)).ok();
            })));
        let card = surface(Theme::radius_panel())
            .absolute()
            .top(px(SHADOW / 2.))
            .left(px(SHADOW))
            .right(px(SHADOW))
            .bottom(px(SHADOW))
            .flex()
            .flex_col()
            .text_size(px(TEXT))
            .text_color(Theme::text())
            .shadow(Theme::elevation(2., SHADOW / 3.))
            .child(header)
            .child(div().flex_1().mx(px(PAD)).mb(px(PAD)).rounded(px(page_corner() as f32)).bg(Theme::page()));
        pop_in(div().size_full().child(card), "link-peek", self.reduce).into_any_element()
    }
}

/// The peeked page's origin on a chip in the fixed security palette and the system font (threat
/// model S29), as in the toolbar.
fn peek_origin(host: Option<String>, insecure: bool) -> gpui::Div {
    div()
        .flex_none()
        .px_2()
        .py_0p5()
        .rounded(px(Theme::radius_small()))
        .bg(Secure::chip())
        .font_family(ui::theme::SYSTEM_FONT)
        .font_weight(FontWeight::MEDIUM)
        .text_color(if insecure { Secure::warning() } else { Secure::text() })
        .child(SharedString::from(match (insecure, host) {
            (true, Some(h)) => format!("Not Secure — {h}"),
            (_, h) => h.unwrap_or_default(),
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_fits_inside_the_card() {
        let main = Bounds::new(point(px(100.), px(50.)), size(px(1280.), px(820.)));
        let (bounds, (x, y, w, h)) = layout(main, 0.);
        assert_eq!(f32::from(bounds.size.width), MAX_WIDTH + 2. * SHADOW);
        assert_eq!(f32::from(bounds.origin.x) + f32::from(bounds.size.width) / 2., 100. + 640.);
        assert!(x + w <= f64::from(f32::from(bounds.size.width) - SHADOW) && y + h <= f64::from(f32::from(bounds.size.height) - SHADOW));
        let small = layout(Bounds::new(point(px(0.), px(0.)), size(px(720.), px(480.))), 0.).0;
        assert_eq!(f32::from(small.size.width), 720. - 2. * MARGIN + 2. * SHADOW);
        let beside = layout(main, 260.).0;
        assert_eq!(f32::from(beside.origin.x) + f32::from(beside.size.width) / 2., 100. + 260. + 510.);
    }
}
