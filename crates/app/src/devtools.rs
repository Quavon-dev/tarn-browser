use devkit::json_view::{self, Kind, Tree};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString, StatefulInteractiveElement, Styled, div,
    prelude::FluentBuilder, px,
};
use ui::controls::button;
use ui::icon::{Icon, icon};
use ui::theme::{TEXT_SMALL, Theme};
use url::Url;

use crate::browser::{Browser, TabKey};

pub struct JsonPage {
    pub url: Url,
    tree: Tree,
    pretty: SharedString,
}

impl JsonPage {
    pub fn parse(url: Url, text: &str) -> Result<Self, json_view::Error> {
        let tree = Tree::parse(text)?;
        Ok(JsonPage { url, pretty: tree.pretty.clone().into(), tree })
    }

    pub fn set_raw(&mut self, raw: bool) {
        self.tree.raw = raw;
    }

    pub fn toggle(&mut self, path: Vec<usize>) {
        self.tree.toggle(&path);
    }
}

fn value_color(kind: Kind) -> gpui::Rgba {
    match kind {
        Kind::String => Theme::success(),
        Kind::Number | Kind::Bool => Theme::warning(),
        Kind::Null => Theme::muted(),
        Kind::Array | Kind::Object => Theme::muted(),
    }
}

pub fn render_json(key: TabKey, page: &JsonPage, cx: &mut Context<Browser>) -> AnyElement {
    let header = div()
        .h(px(40.))
        .flex_none()
        .px_3()
        .flex()
        .items_center()
        .gap_2()
        .border_b_1()
        .border_color(Theme::hairline())
        .child(icon(Icon::Braces, 14., Theme::muted()))
        .child(div().font_weight(FontWeight::MEDIUM).child("JSON"))
        .child(div().flex_1())
        .child(button("json-tree", "Tree", !page.tree.raw).on_click(cx.listener(move |b, _, _, cx| b.set_json_raw(key, false, cx))))
        .child(button("json-raw", "Raw", page.tree.raw).on_click(cx.listener(move |b, _, _, cx| b.set_json_raw(key, true, cx))));

    let body = if page.tree.raw {
        div().p_3().font_family(ui::theme::MONO_FONT).text_size(px(TEXT_SMALL)).child(page.pretty.clone()).into_any_element()
    } else {
        let rows = &page.tree.rows;
        let truncated = page.tree.truncated();
        div()
            .py_2()
            .font_family(ui::theme::MONO_FONT)
            .text_size(px(TEXT_SMALL))
            .children(rows.iter().enumerate().map(|(ix, row)| {
                let expandable = row.expandable();
                let path = row.path.clone();
                div()
                    .id(("json-row", ix))
                    .h(px(20.))
                    .pl(px(12. + row.depth as f32 * 16.))
                    .pr_3()
                    .flex()
                    .items_center()
                    .gap_1()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .when(expandable, |d| {
                        d.cursor_pointer().hover(|s| s.bg(Theme::hover())).on_click(cx.listener(move |b, _, _, cx| b.toggle_json_node(key, path.clone(), cx)))
                    })
                    .child(
                        div()
                            .w(px(12.))
                            .flex_none()
                            .when(expandable, |d| d.child(icon(if row.open { Icon::ChevronDown } else { Icon::ChevronRight }, 10., Theme::muted()))),
                    )
                    .when(!row.label.is_empty(), |d| d.child(div().text_color(Theme::accent_text()).child(SharedString::from(format!("{}:", row.label)))))
                    .child(div().text_color(value_color(row.kind)).text_ellipsis().child(SharedString::from(row.value.clone())))
            }))
            .when(truncated, |d| d.child(div().px_3().text_color(Theme::muted()).child("Only the first 2,000 rows are shown.")))
            .into_any_element()
    };

    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .rounded(px(Theme::radius_page()))
        .bg(Theme::surface())
        .border_1()
        .border_color(Theme::hairline())
        .child(header)
        .child(div().id("json-body").flex_1().min_h_0().overflow_y_scroll().child(body))
        .into_any_element()
}
