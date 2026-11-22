use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString, StatefulInteractiveElement, Styled, div,
    prelude::FluentBuilder, px,
};
use ui::controls::{icon_button, tooltip};
use ui::icon::Icon;
use ui::theme::{TEXT, Theme};

use crate::browser::{Browser, TabKey};
use crate::sidebar::{SIDEBAR_MARGIN, tab_icon};
use crate::tabs::display_title;

const TAB_WIDTH: f32 = 200.;
const MIN_TAB_WIDTH: f32 = 96.;

/// Tabs on top: the pinned tabs as icons, then the others, in a strip above the page.
pub fn render(b: &mut Browser, cx: &mut Context<Browser>) -> AnyElement {
    let strip = browser_core::sidebar::strip(&b.model);
    let pinned: Vec<AnyElement> = strip.pinned.iter().filter_map(|k| chip(b, *k, true, cx)).collect();
    let tabs: Vec<AnyElement> = strip.tabs.iter().filter_map(|k| chip(b, *k, false, cx)).collect();
    div()
        .id("tab-strip")
        .flex_none()
        .flex()
        .items_center()
        .gap(px(4.))
        .px(px(SIDEBAR_MARGIN))
        .pb(px(SIDEBAR_MARGIN))
        .overflow_x_scroll()
        .text_size(px(TEXT))
        .children(pinned)
        .when(!strip.pinned.is_empty() && !strip.tabs.is_empty(), |d| {
            d.child(div().flex_none().w(px(1.)).h(px(Theme::tab_row() - 10.)).mx_1().bg(Theme::separator()))
        })
        .children(tabs)
        .child(
            icon_button("strip-new-tab", Icon::Plus, true)
                .tooltip(crate::shortcuts::tip("New Tab", &crate::NewTab))
                .on_click(cx.listener(|b, _, window, cx| b.new_tab_page(window, cx))),
        )
        .into_any_element()
}

fn chip(b: &Browser, key: TabKey, pinned: bool, cx: &mut Context<Browser>) -> Option<AnyElement> {
    let t = b.model.tab(key)?;
    let active = b.model.active == Some(key);
    let title: SharedString = display_title(t).into();
    let asleep = b.sleeping(key) && !active;
    let row = div()
        .id(("strip-tab", key.id as u64))
        .group("strip-tab")
        .h(px(Theme::tab_row()))
        .flex()
        .items_center()
        .gap_2()
        .rounded(px(Theme::radius_row()))
        .cursor_pointer()
        .when(active, |d| d.bg(Theme::selected()).border_1().border_color(Theme::surface_edge()))
        .when(!active, |d| d.hover(|s| s.bg(Theme::hover())))
        .child(tab_icon(b, &t.url, 16., asleep))
        .on_click(cx.listener(move |b, _, window, cx| b.switch_to(key, window, cx)))
        .on_mouse_down(MouseButton::Right, cx.listener(move |b, _, window, cx| b.tab_menu(key, window, cx)));
    let row = if pinned {
        row.flex_none().w(px(Theme::tab_row() + 6.)).justify_center().tooltip(tooltip(title, None))
    } else {
        row.flex_1()
            .min_w(px(MIN_TAB_WIDTH))
            .max_w(px(TAB_WIDTH))
            .pl_2p5()
            .pr_0p5()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(if asleep { Theme::asleep() } else { Theme::text() })
                    .when(active, |d| d.font_weight(FontWeight::MEDIUM))
                    .child(title),
            )
            .child(
                icon_button(("strip-close", key.id as u64), Icon::Close, true)
                    .tab_stop(false)
                    .when(!active, |d| d.opacity(0.).group_hover("strip-tab", |s| s.opacity(1.)))
                    .tooltip(crate::shortcuts::tip("Close Tab", &crate::CloseTab))
                    .on_click(cx.listener(move |b, _, window, cx| {
                        cx.stop_propagation();
                        b.close_tab(key, window, cx);
                    })),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |b, _, window, cx| {
                    cx.stop_propagation();
                    b.close_tab(key, window, cx);
                }),
            )
    };
    Some(row.into_any_element())
}
