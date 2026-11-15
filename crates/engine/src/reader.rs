//! Reader mode: the page's article, read as plain text in the engine's isolated world and bounded
//! again here, so the shell renders only short plain strings (S14).

/// The article's text, in bytes.
pub const MAX_BYTES: usize = 200 * 1024;
/// One paragraph, in characters.
pub const MAX_BLOCK: usize = 10_000;
const MAX_BLOCKS: usize = 2_000;
/// Elements looked at in each pass, so a huge page cannot stall its own loads.
const MAX_SCAN: usize = 20_000;
const MAX_TITLE: usize = 1024;
const MAX_BYLINE: usize = 256;
/// Paragraph text one parent must hold before the page counts as an article.
const MIN_SCORE: usize = 1_000;

#[derive(Debug, Clone, PartialEq)]
pub struct Article {
    pub title: String,
    pub byline: String,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Heading(String),
    Paragraph(String),
}

/// An async function body for the isolated world. The element whose `<p>` children (and, at half
/// weight, grandchildren) hold the most text is the article, or its parent when its siblings hold
/// much more, as in articles split into sections. With `probe` it returns "1" when there is one,
/// else the article as JSON for `Article::parse`; empty when there is none.
/// Empty once the tab shows another origin than `origin`.
pub fn script(origin: &str, probe: bool) -> String {
    let origin = crate::handover::js_string(origin);
    format!(
        r"if (location.origin !== {origin}) return '';
        // Cut by code point, so no half of a surrogate pair reaches JSON.
        const clean = (s, n) => Array.from((s || '').replace(/[\u0000-\u001f\u007f-\u009f\u2028\u2029]+/g, ' ').replace(/\s+/g, ' ').trim()).slice(0, n).join('');
        const shown = e => e.getClientRects().length > 0;
        const scores = new Map();
        let seen = 0;
        for (const p of document.querySelectorAll('p')) {{
            if (++seen > {MAX_SCAN}) break;
            const n = p.textContent.trim().length;
            if (n < 40 || !p.parentElement || !shown(p)) continue;
            scores.set(p.parentElement, (scores.get(p.parentElement) || 0) + n);
            const up = p.parentElement.parentElement;
            if (up && up !== document.body) scores.set(up, (scores.get(up) || 0) + n / 2);
        }}
        let best = null, score = 0;
        for (const [e, n] of scores) if (n > score) {{ best = e; score = n; }}
        if (!best || score < {MIN_SCORE}) return '';
        const up = best.parentElement;
        if (up && up !== document.body && (scores.get(up) || 0) * 2 >= score * 1.5) best = up;
        if ({probe}) return '1';
        const blocks = []; let total = 0, visited = 0;
        for (const e of best.querySelectorAll('h2, h3, h4, p, li, pre, blockquote')) {{
            if (++visited > {MAX_SCAN}) break;
            const outer = e.parentElement && e.parentElement.closest('p, li, pre, blockquote');
            if ((outer && best.contains(outer)) || e.closest('nav, aside, footer, figure, table') || !shown(e)) continue;
            const text = clean(e.innerText, {MAX_BLOCK});
            if (!text) continue;
            total += text.length;
            if (total > {MAX_BYTES} || blocks.length >= {MAX_BLOCKS}) break;
            blocks.push([/^H\d$/.test(e.tagName) ? 'h' : 'p', text]);
        }}
        const meta = n => {{ const m = document.querySelector(`meta[name='${{n}}'], meta[property='${{n}}']`); return m ? m.content : ''; }};
        const h1 = best.querySelector('h1') || document.querySelector('article h1') || document.querySelector('h1');
        const by = document.querySelector('[rel=author], [itemprop=author], .byline');
        return JSON.stringify({{
            t: clean(h1 && h1.innerText, {MAX_TITLE}) || clean(meta('og:title'), {MAX_TITLE}) || clean(document.title, {MAX_TITLE}),
            b: clean(meta('author'), {MAX_BYLINE}) || clean(by && by.innerText, {MAX_BYLINE}),
            p: blocks,
        }});"
    )
}

impl Article {
    /// The script's JSON, with every limit applied again; `None` for anything malformed or empty.
    pub fn parse(json: &str) -> Option<Article> {
        // The script counts UTF-16 units; three UTF-8 bytes each, plus JSON escapes.
        if json.len() > 4 * MAX_BYTES {
            return None;
        }
        let value: serde_json::Value = serde_json::from_str(json).ok()?;
        let line = |key: &str, max: usize| value.get(key).and_then(|v| v.as_str()).map(|s| clean(s, max)).unwrap_or_default();
        let mut total = 0;
        let mut blocks = Vec::new();
        for item in value.get("p")?.as_array()?.iter().take(MAX_BLOCKS) {
            let Some([kind, text]) = item.as_array().map(Vec::as_slice) else { continue };
            let text = clean(text.as_str().unwrap_or_default(), MAX_BLOCK);
            if text.is_empty() {
                continue;
            }
            total += text.len();
            if total > MAX_BYTES {
                break;
            }
            blocks.push(if kind.as_str() == Some("h") { Block::Heading(text) } else { Block::Paragraph(text) });
        }
        (!blocks.is_empty()).then(|| Article { title: line("t", MAX_TITLE), byline: line("b", MAX_BYLINE), blocks })
    }
}

/// One line of at most `max` characters, without control characters.
fn clean(text: &str, max: usize) -> String {
    text.split(|c: char| c.is_control() || c.is_whitespace()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ").chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn articles_are_bounded_plain_text() {
        let json = serde_json::json!({
            "t": format!("Title\u{0}\n{}", "x".repeat(2000)),
            "b": "By\tsomeone",
            "p": [["h", "Head"], ["p", "a\u{7}b  \n c"], ["p", ""], ["x"], 3, ["p", "y".repeat(MAX_BLOCK + 5)]],
        })
        .to_string();
        let article = Article::parse(&json).unwrap();
        assert_eq!(article.title.chars().count(), MAX_TITLE);
        assert!(article.title.starts_with("Title x"));
        assert_eq!(article.byline, "By someone");
        assert_eq!(article.blocks[..2], [Block::Heading("Head".into()), Block::Paragraph("a b c".into())]);
        assert_eq!(article.blocks.len(), 3);
        assert_eq!(article.blocks[2], Block::Paragraph("y".repeat(MAX_BLOCK)));
    }

    /// `cargo test -p engine reader -- --ignored`.
    #[test]
    #[ignore = "needs node"]
    fn scripts_parse_as_async_function_bodies() {
        for probe in [true, false] {
            let source = format!("(async () => {{ {} }})", script("https://a.test", probe));
            let output = std::process::Command::new("node").args(["-e", "new Function(process.argv[1])", &source]).output().expect("node");
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        }
    }

    /// Runs the script against a small DOM of an article split into sections:
    /// `cargo test -p engine reader -- --ignored`.
    #[test]
    #[ignore = "needs node"]
    fn the_article_spans_its_sections_and_skips_asides() {
        let output = std::process::Command::new("node")
            .args(["-e", NODE_DOM, &script("https://a.test", true), &script("https://a.test", false)])
            .output()
            .expect("node");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut lines = stdout.lines();
        assert_eq!(lines.next(), Some("1"), "{}", String::from_utf8_lossy(&output.stderr));
        let article = Article::parse(lines.next().unwrap_or_default()).unwrap();
        assert_eq!(article.title, "Tarn");
        let texts: Vec<_> = article
            .blocks
            .iter()
            .map(|b| match b {
                Block::Heading(t) | Block::Paragraph(t) => t.chars().next().unwrap(),
            })
            .collect();
        assert_eq!(texts, ['a', 'b', 'E', 'c', 'd']);
        let output = std::process::Command::new("node")
            .args(["-e", NODE_DOM, &script("https://other.test", true), &script("https://other.test", false)])
            .output()
            .expect("node");
        assert_eq!(String::from_utf8_lossy(&output.stdout), "\n\n", "another origin gets nothing");
    }

    const NODE_DOM: &str = r"
        class El { constructor(tag, kids = [], text = '') { this.tagName = tag.toUpperCase(); this.kids = kids; this.own = text; this.parentElement = null; kids.forEach(k => k.parentElement = this); }
            get textContent() { return this.own + this.kids.map(k => k.textContent).join(''); } get innerText() { return this.textContent; }
            getClientRects() { return [1]; } contains(e) { for (; e; e = e.parentElement) if (e === this) return true; return false; }
            matches(sel) { return sel.split(',').map(s => s.trim().toUpperCase()).includes(this.tagName); }
            closest(sel) { for (let e = this; e; e = e.parentElement) if (e.matches(sel)) return e; return null; }
            all() { return this.kids.flatMap(k => [k, ...k.all()]); } querySelectorAll(sel) { return this.all().filter(e => e.matches(sel)); }
            querySelector(sel) { return this.querySelectorAll(sel)[0] || null; } }
        const p = c => new El('p', [], c.repeat(400)), section = kids => new El('section', kids);
        const main = new El('div', [section([p('a'), p('b')]), section([new El('h2', [], 'Etym'), p('c'), p('d')]), new El('aside', [p('z')])]);
        const body = new El('body', [new El('h1', [], 'Tarn'), main]);
        const document = { body, title: 'T', querySelectorAll: s => body.querySelectorAll(s), querySelector: s => /meta|rel=|itemprop|byline/.test(s) ? null : body.querySelector(s) };
        const run = src => new Function('document', 'location', `return (async () => { ${src} })()`)(document, { origin: 'https://a.test' });
        (async () => { console.log(await run(process.argv[1])); console.log(await run(process.argv[2])); })();
    ";

    #[test]
    fn too_much_or_nothing_is_no_article() {
        assert_eq!(Article::parse(""), None);
        assert_eq!(Article::parse("{\"t\":\"T\",\"p\":[]}"), None);
        assert_eq!(Article::parse(&"x".repeat(4 * MAX_BYTES + 1)), None);
        let many: Vec<_> = (0..20).map(|_| ["p".to_owned(), "\u{4e2d}".repeat(MAX_BLOCK)]).collect();
        let article = Article::parse(&serde_json::json!({ "p": many }).to_string()).unwrap();
        assert!(!article.blocks.is_empty() && article.blocks.len() * 3 * MAX_BLOCK <= MAX_BYTES);
    }
}
