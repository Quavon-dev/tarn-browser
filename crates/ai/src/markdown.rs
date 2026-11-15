//! Model output as a render model for native drawing. Images become their alt text,
//! HTML stays literal text, and links only carry their URL for display.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
    /// Shown to the user; never opened without an explicit action.
    pub link: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading { level: u8, spans: Vec<Span> },
    Paragraph(Vec<Span>),
    ListItem { depth: u8, number: Option<u64>, spans: Vec<Span> },
    Quote(Vec<Span>),
    Code { lang: String, code: String },
    Rule,
}

#[derive(Default)]
struct Builder {
    blocks: Vec<Block>,
    spans: Vec<Span>,
    style: Style,
    link: Option<String>,
    heading: Option<u8>,
    lists: Vec<Option<u64>>,
    item_number: Option<u64>,
    quote: u8,
    code: Option<(String, String)>,
}

impl Builder {
    fn text(&mut self, text: &str) {
        if let Some((_, code)) = &mut self.code {
            code.push_str(text);
            return;
        }
        let style = self.style;
        match self.spans.last_mut() {
            Some(last) if last.style == style && last.link == self.link => last.text.push_str(text),
            _ => self.spans.push(Span { text: text.into(), style, link: self.link.clone() }),
        }
    }

    fn flush(&mut self) {
        if self.spans.is_empty() {
            return;
        }
        let spans = std::mem::take(&mut self.spans);
        self.blocks.push(if let Some(level) = self.heading {
            Block::Heading { level, spans }
        } else if !self.lists.is_empty() {
            Block::ListItem { depth: self.lists.len().min(u8::MAX.into()) as u8, number: self.item_number.take(), spans }
        } else if self.quote > 0 {
            Block::Quote(spans)
        } else {
            Block::Paragraph(spans)
        });
    }
}

fn display_url(url: &str) -> Option<String> {
    let lower = url.to_ascii_lowercase();
    ["https://", "http://", "mailto:"]
        .iter()
        .any(|p| lower.starts_with(p))
        .then(|| url.chars().filter(|c| !c.is_control() && !crate::page_context::is_bidi(*c)).collect())
}

pub fn parse(markdown: &str) -> Vec<Block> {
    let mut b = Builder::default();
    for event in Parser::new_ext(markdown, Options::ENABLE_STRIKETHROUGH) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                b.flush();
                b.heading = Some(level as u8);
            }
            Event::End(TagEnd::Heading(_)) => {
                b.flush();
                b.heading = None;
            }
            Event::Start(Tag::List(start)) => {
                b.flush();
                b.lists.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                b.flush();
                b.lists.pop();
            }
            Event::Start(Tag::Item) => {
                b.flush();
                b.item_number = b.lists.last_mut().and_then(|n| {
                    let current = *n;
                    if let Some(n) = n {
                        *n += 1;
                    }
                    current
                });
            }
            Event::Start(Tag::BlockQuote(_)) => {
                b.flush();
                b.quote = b.quote.saturating_add(1);
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                b.flush();
                b.quote = b.quote.saturating_sub(1);
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                b.flush();
                let lang = match kind {
                    pulldown_cmark::CodeBlockKind::Fenced(l) => l.to_string(),
                    pulldown_cmark::CodeBlockKind::Indented => String::new(),
                };
                b.code = Some((lang, String::new()));
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((lang, code)) = b.code.take() {
                    b.blocks.push(Block::Code { lang, code });
                }
            }
            Event::End(TagEnd::Paragraph | TagEnd::Item) => b.flush(),
            Event::Start(Tag::Emphasis) => b.style.italic = true,
            Event::End(TagEnd::Emphasis) => b.style.italic = false,
            Event::Start(Tag::Strong) => b.style.bold = true,
            Event::End(TagEnd::Strong) => b.style.bold = false,
            Event::Start(Tag::Strikethrough) => b.style.strike = true,
            Event::End(TagEnd::Strikethrough) => b.style.strike = false,
            Event::Start(Tag::Link { dest_url, .. }) => b.link = display_url(&dest_url),
            Event::End(TagEnd::Link) => b.link = None,
            Event::Code(code) => {
                b.style.code = true;
                b.text(&code);
                b.style.code = false;
            }
            Event::Text(t) | Event::Html(t) | Event::InlineHtml(t) => b.text(&t),
            Event::SoftBreak => b.text(" "),
            Event::HardBreak => b.text("\n"),
            Event::Rule => {
                b.flush();
                b.blocks.push(Block::Rule);
            }
            _ => {}
        }
    }
    b.flush();
    b.blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_spans(blocks: &[Block]) -> Vec<Span> {
        blocks
            .iter()
            .flat_map(|b| match b {
                Block::Heading { spans, .. } | Block::Paragraph(spans) | Block::ListItem { spans, .. } | Block::Quote(spans) => spans.clone(),
                _ => vec![],
            })
            .collect()
    }

    #[test]
    fn image_becomes_alt_text_only() {
        let blocks = parse("see ![x](http://evil/a.png) here");
        let spans = all_spans(&blocks);
        let text: String = spans.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(text, "see x here");
        assert!(spans.iter().all(|s| s.link.is_none()));
        assert!(!format!("{blocks:?}").contains("evil"));
    }

    #[test]
    fn link_is_display_only_text() {
        let blocks = parse("[click](https://a.test/x) and [js](javascript:alert(1))");
        let spans = all_spans(&blocks);
        assert_eq!(spans[0], Span { text: "click".into(), style: Style::default(), link: Some("https://a.test/x".into()) });
        assert!(spans.iter().filter(|s| s.text == "js").all(|s| s.link.is_none()));
    }

    #[test]
    fn link_drops_bidi_controls() {
        let blocks = parse("[x](https://a.test/\u{202E}gpj.exe)");
        assert_eq!(all_spans(&blocks)[0].link.as_deref(), Some("https://a.test/gpj.exe"));
    }

    #[test]
    fn html_shown_as_text() {
        let blocks = parse("<img src=x onerror=alert(1)>\n\nhi <b>x</b>");
        let text: String = all_spans(&blocks).iter().map(|s| s.text.as_str()).collect();
        assert!(text.contains("<img src=x onerror=alert(1)>"));
        assert!(text.contains("<b>x</b>"));
    }

    #[test]
    fn structure() {
        let blocks = parse("# T\n\n1. a\n2. **b**\n   - c\n\n> q\n\n```rs\nfn f() {}\n```\n\n---");
        assert_eq!(blocks[0], Block::Heading { level: 1, spans: vec![Span { text: "T".into(), style: Style::default(), link: None }] });
        assert!(matches!(&blocks[1], Block::ListItem { depth: 1, number: Some(1), .. }));
        assert!(matches!(&blocks[2], Block::ListItem { depth: 1, number: Some(2), spans } if spans[0].style.bold));
        assert!(matches!(&blocks[3], Block::ListItem { depth: 2, number: None, .. }));
        assert!(matches!(&blocks[4], Block::Quote(_)));
        assert_eq!(blocks[5], Block::Code { lang: "rs".into(), code: "fn f() {}\n".into() });
        assert_eq!(blocks[6], Block::Rule);
    }
}
