use std::collections::HashSet;

use devkit::json_view::{self, Key, Kind, Node};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString, StatefulInteractiveElement, Styled, div,
    prelude::FluentBuilder, px,
};
use ui::controls::button;
use ui::icon::{Icon, icon};
use ui::theme::{TEXT_SMALL, Theme};
use url::Url;

use crate::browser::{Browser, TabKey};

const MAX_ROWS: usize = 2_000;
const MAX_RAW_BYTES: usize = 256 * 1024;

pub struct JsonPage {
    pub url: Url,
    root: Node,
    pretty: SharedString,
    pub raw: bool,
    open: HashSet<Vec<usize>>,
    rows: Vec<Row>,
}

struct Row {
    depth: usize,
    path: Vec<usize>,
    label: String,
    value: String,
    kind: Kind,
    open: bool,
}

impl JsonPage {
    pub fn parse(url: Url, text: &str) -> Result<Self, json_view::Error> {
        let (root, pretty) = json_view::parse_with_pretty(text.as_bytes(), MAX_RAW_BYTES)?;
        let mut page = JsonPage { url, root, pretty: plain(&pretty, true).into(), raw: false, open: HashSet::from([Vec::new()]), rows: Vec::new() };
        page.rows = page.rows();
        Ok(page)
    }

    pub fn toggle(&mut self, path: Vec<usize>) {
        if !self.open.remove(&path) {
            self.open.insert(path);
        }
        self.rows = self.rows();
    }

    fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut path = Vec::new();
        self.walk(&self.root, 0, &mut path, &mut rows);
        rows
    }

    fn walk(&self, node: &Node, depth: usize, path: &mut Vec<usize>, rows: &mut Vec<Row>) {
        if rows.len() >= MAX_ROWS {
            return;
        }
        let open = self.open.contains(path);
        let label = match &node.key {
            Key::Root => String::new(),
            Key::Name(name) => plain(name, false),
            Key::Index(i) => i.to_string(),
        };
        let value = match node.kind {
            Kind::Array => format!("[{}]", node.child_count),
            Kind::Object => format!("{{{}}}", node.child_count),
            Kind::String => format!("\"{}\"", plain(&node.text, false)),
            _ => node.text.clone(),
        };
        rows.push(Row { depth, path: path.clone(), label, value, kind: node.kind, open });
        if open {
            for (i, child) in node.children.iter().enumerate() {
                path.push(i);
                self.walk(child, depth + 1, path, rows);
                path.pop();
            }
        }
    }
}

/// Page-supplied text drawn in the shell: control and bidi formatting characters are replaced.
fn plain(text: &str, keep_newlines: bool) -> String {
    text.chars().map(|c| if (keep_newlines && c == '\n') || !(c.is_control() || ui::text::is_bidi(c)) { c } else { '\u{fffd}' }).collect()
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
        .child(button("json-tree", "Tree", !page.raw).on_click(cx.listener(move |b, _, _, cx| b.set_json_raw(key, false, cx))))
        .child(button("json-raw", "Raw", page.raw).on_click(cx.listener(move |b, _, _, cx| b.set_json_raw(key, true, cx))));

    let body = if page.raw {
        div().p_3().font_family("Menlo").text_size(px(TEXT_SMALL)).child(page.pretty.clone()).into_any_element()
    } else {
        let rows = &page.rows;
        let truncated = rows.len() >= MAX_ROWS;
        div()
            .py_2()
            .font_family("Menlo")
            .text_size(px(TEXT_SMALL))
            .children(rows.iter().enumerate().map(|(ix, row)| {
                let expandable = matches!(row.kind, Kind::Array | Kind::Object);
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
                    .when(!row.label.is_empty(), |d| d.child(div().text_color(Theme::accent()).child(SharedString::from(format!("{}:", row.label)))))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_follow_open_containers() {
        let url = Url::parse("https://api.test/").unwrap();
        let mut page = JsonPage::parse(url, r#"{"a":[1,2],"b":"x"}"#).unwrap();
        let labels = |p: &JsonPage| p.rows().iter().map(|r| format!("{}={}", r.label, r.value)).collect::<Vec<_>>();
        assert_eq!(labels(&page), ["={2}", "a=[2]", "b=\"x\""]);
        page.toggle(vec![0]);
        assert_eq!(labels(&page), ["={2}", "a=[2]", "0=1", "1=2", "b=\"x\""]);
        page.toggle(Vec::new());
        assert_eq!(labels(&page), ["={2}"]);
        assert!(JsonPage::parse(Url::parse("https://api.test/").unwrap(), "{").is_err());
        let spoof = JsonPage::parse(Url::parse("https://api.test/").unwrap(), "{\"a\\u202eb\":\"x\\ny\"}").unwrap();
        assert_eq!(labels(&spoof), ["={1}", "a\u{fffd}b=\"x\u{fffd}y\""]);
    }
}
