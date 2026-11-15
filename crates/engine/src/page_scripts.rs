use url::Url;

use crate::handover;

/// Builds a selector of the shape `shield::lists::valid_zap` accepts: from the element up to the
/// nearest unique id, or to a child of `<body>`, each step its tag, up to two classes and its
/// position among siblings of the same tag.
pub const PICKER: &str = r"if (window.tarnPick) window.tarnPick();
return await new Promise(resolve => {
    const ident = /^[A-Za-z_][A-Za-z0-9_-]*$/, tag = /^[a-z][a-z0-9-]*$/;
    const box = document.createElement('div');
    box.style.cssText = 'position:fixed;z-index:2147483647;pointer-events:none;display:none;background:rgba(255,59,48,.2);outline:2px solid #ff3b30';
    document.documentElement.append(box);
    let target = null;
    const step = el => {
        if (el.id && ident.test(el.id) && document.querySelectorAll('#' + el.id).length === 1) return ['#' + el.id, true];
        let s = tag.test(el.localName) ? el.localName : '';
        for (const c of [...el.classList].filter(c => ident.test(c)).slice(0, 2)) s += '.' + c;
        const same = [...el.parentElement.children].filter(c => c.localName === el.localName);
        if (same.length > 1) s += ':nth-of-type(' + (same.indexOf(el) + 1) + ')';
        return [s, false];
    };
    const selector = el => {
        const parts = [];
        let e = el, anchored = false;
        for (; e && e.parentElement && e !== document.body; e = e.parentElement) {
            const [s, unique] = step(e);
            if (!s) return '';
            parts.unshift(s);
            if (unique) { anchored = true; break; }
        }
        if (!anchored) {
            if (e !== document.body) return '';
            parts.unshift('body');
        }
        const s = parts.join(' > ');
        return s.length <= 256 ? s : '';
    };
    const hidden = t => !(t instanceof Element) || t === document.body || t === document.documentElement || t === box;
    const move = e => {
        if (!e.isTrusted) return;
        target = hidden(e.target) ? null : e.target;
        if (!target) { box.style.display = 'none'; return; }
        const r = target.getBoundingClientRect();
        Object.assign(box.style, { display: 'block', left: r.left + 'px', top: r.top + 'px', width: r.width + 'px', height: r.height + 'px' });
    };
    const stop = e => { e.preventDefault(); e.stopImmediatePropagation(); };
    const click = e => { if (!e.isTrusted) return; stop(e); const s = target && selector(target); if (s) finish(s); };
    const key = e => { if (e.isTrusted && e.key === 'Escape') { stop(e); finish(''); } };
    const events = [['mouseover', move], ['mousedown', stop], ['mouseup', stop], ['click', click], ['keydown', key]];
    const finish = s => {
        for (const [type, f] of events) removeEventListener(type, f, true);
        box.remove();
        window.tarnPick = null;
        resolve(s);
    };
    window.tarnPick = () => finish('');
    for (const [type, f] of events) addEventListener(type, f, true);
});";

/// Most snippets a page gives Find in Tabs, the characters of context on each side of a match, the
/// longest query passed on, and how much of the page's text is searched.
pub const FIND_MATCHES: usize = 3;
const FIND_CONTEXT: usize = 60;
const FIND_QUERY_MAX: usize = 200;
const FIND_TEXT_MAX: usize = 1_000_000;
const FIND_NODES_MAX: usize = 20_000;

/// The query goes in as a JS string literal, so no text the user types can break out of it, and is
/// matched as a case-insensitive regular expression on the text itself, so offsets stay exact where
/// lowercasing would change a string's length (İ). Text nodes are read until `FIND_TEXT_MAX`
/// characters, skipping the elements `innerText` leaves out. A slice can split a surrogate pair,
/// which NSString cannot hand over as UTF-8, so the answer is made well formed.
pub fn find_text_script(query: &str) -> String {
    let query = handover::js_string(&query.chars().take(FIND_QUERY_MAX).collect::<String>());
    format!(
        r"const q = {query};
        if (!q || !document.body) return '';
        const skip = /^(script|style|noscript|template|textarea|select)$/i;
        const hidden = el => el.checkVisibility && !el.checkVisibility({{ visibilityProperty: true, opacityProperty: true }});
        const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT, {{
            acceptNode: n => n.parentElement && (skip.test(n.parentElement.nodeName) || hidden(n.parentElement)) ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT
        }});
        // Text nodes in the same block join without a space, so a word split by inline tags stays whole.
        const inline = /^(a|abbr|b|bdi|bdo|cite|code|data|dfn|em|font|i|kbd|mark|q|s|samp|small|span|strong|sub|sup|time|u|var)$/i;
        const block = el => {{ while (el && el !== document.body && inline.test(el.nodeName)) el = el.parentElement; return el; }};
        let text = '', last = null;
        for (let node = walker.nextNode(), count = 0; node && text.length < {FIND_TEXT_MAX} && count < {FIND_NODES_MAX}; node = walker.nextNode(), count++) {{
            const here = block(node.parentElement);
            text += (here === last ? '' : ' ') + node.data;
            last = here;
        }}
        text = text.slice(0, {FIND_TEXT_MAX});
        const out = [];
        const re = new RegExp(q.replace(/[.*+?^${{}}()|[\]\\]/g, '\\$&'), 'giu');
        for (let m; out.length < {FIND_MATCHES} && (m = re.exec(text)); )
            out.push(text.slice(Math.max(0, m.index - {FIND_CONTEXT}), m.index + m[0].length + {FIND_CONTEXT}).replace(/\s+/g, ' ').trim());
        const joined = out.join('\n');
        return joined.toWellFormed ? joined.toWellFormed() : joined;"
    )
}

/// The script's answer, bounded again: the page's text is untrusted.
pub fn find_snippets(found: &str) -> Vec<String> {
    let max = 2 * FIND_CONTEXT + FIND_QUERY_MAX;
    found.lines().filter(|l| !l.trim().is_empty()).take(FIND_MATCHES).map(|l| l.chars().take(max).collect()).collect()
}

/// Reads the declared icons of the page at `origin` as `href<TAB>sizes` lines, then an empty
/// line and the hosts (default https port) the page has loaded resources from, at most 64; empty
/// once the tab shows another origin. An async function body for the engine's isolated world.
pub fn icon_links_script(origin: &Url) -> Option<String> {
    let origin = script_string(&origin.origin().ascii_serialization())?;
    Some(format!(
        r"if (location.origin !== {origin}) return '';
        const links = Array.from(document.querySelectorAll('link[rel][href]'))
            .filter(l => l.href.length <= 2048 && (!l.sizes || l.sizes.value.length <= 256) && /(^|\s)(icon|apple-touch-icon)(\s|$)/i.test(l.rel))
            .slice(0, 16)
            .map(l => [l.href, l.sizes ? l.sizes.value : ''].map(v => v.replace(/[\t\r\n]/g, ' ')).join('\t'));
        const hosts = new Set();
        for (const entry of performance.getEntriesByType('resource').slice(0, 2000)) {{
            if (hosts.size >= 64) break;
            try {{
                const url = new URL(entry.name);
                if (url.protocol === 'https:' && !url.port && url.hostname.length <= 253) hosts.add(url.hostname);
            }} catch (e) {{}}
        }}
        return links.join('\n') + '\n\n' + Array.from(hosts).join('\n');"
    ))
}

/// The page's visible text with runs of white space collapsed, at most `max_chars` UTF-16 units;
/// editable and hidden text is left out. At most 20000 text nodes are looked at, each cut to
/// `max_chars` first, so a huge page costs little.
/// Empty once the tab shows another origin. An async function body for the engine's isolated world.
pub fn page_text_script(origin: &Url, max_chars: usize) -> Option<String> {
    let origin = script_string(&origin.origin().ascii_serialization())?;
    Some(format!(
        r"if (location.origin !== {origin} || !document.body) return '';
        const skip = /^(script|style|noscript|template|textarea|select)$/i;
        const hidden = el => el.checkVisibility && !el.checkVisibility({{ visibilityProperty: true, opacityProperty: true }});
        let seen = 0;
        const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT, {{
            acceptNode: n => {{
                if (++seen > 20000) throw 'limit';
                const el = n.parentElement;
                return !el || skip.test(el.nodeName) || el.isContentEditable || hidden(el) ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT;
            }}
        }});
        const parts = [];
        let size = 0;
        try {{
            for (let node = walker.nextNode(); node && size < {max_chars}; node = walker.nextNode()) {{
                const text = node.data.slice(0, {max_chars}).replace(/\s+/g, ' ').trim();
                if (text) {{ parts.push(text); size += text.length + 1; }}
            }}
        }} catch (e) {{}}
        // A slice can split a surrogate pair, which NSString cannot hand over as UTF-8.
        const text = parts.join(' ').slice(0, {max_chars});
        return text.toWellFormed ? text.toWellFormed() : text;"
    ))
}

/// Fetches the first of `urls` that answers with at most `max_bytes`, as base64; an SVG icon is
/// rasterised to PNG by the engine first. The engine makes the request, so the profile's content
/// rules, proxy settings and user agent apply; cookies go only to the page's own origin. A
/// same-site URL must land on https on the page's site; another site's URL is fetched without
/// credentials as a CORS request and must land on https on its own host. Runs only while the tab
/// still shows `origin`, so another site's service worker cannot answer. An async function body
/// for the engine's isolated world.
pub fn fetch_icon_script(origin: &Url, urls: &[Url], max_bytes: usize) -> Option<String> {
    let origin = script_string(&origin.origin().ascii_serialization())?;
    let list: Vec<String> = urls.iter().filter_map(|u| script_string(u.as_str())).collect();
    Some(format!(
        r"if (location.origin !== {origin}) return '';
        const rasterize = async (blob) => {{
            const src = URL.createObjectURL(blob);
            try {{
                const img = new Image();
                img.src = src;
                await img.decode();
                const w = img.naturalWidth || 128, h = img.naturalHeight || 128, scale = 128 / Math.max(w, h);
                const canvas = document.createElement('canvas');
                canvas.width = Math.max(1, Math.round(w * scale));
                canvas.height = Math.max(1, Math.round(h * scale));
                canvas.getContext('2d').drawImage(img, 0, 0, canvas.width, canvas.height);
                const png = await new Promise(done => canvas.toBlob(done, 'image/png'));
                return png ? new Uint8Array(await png.arrayBuffer()) : null;
            }} finally {{
                URL.revokeObjectURL(src);
            }}
        }};
        const base = location.hostname.replace(/^www\./, '');
        const sameSite = host => host === location.hostname || host === base || host.endsWith('.' + base);
        for (const url of [{list}]) {{
            try {{
                const target = new URL(url), cross = !sameSite(target.hostname);
                const signal = AbortSignal.timeout(10000);
                const response = await fetch(url, cross ? {{ mode: 'cors', credentials: 'omit', redirect: 'error', signal }} : {{ credentials: 'same-origin', redirect: 'error', signal }});
                const landed = new URL(response.url);
                if (!response.ok || !response.body || landed.protocol !== 'https:' || !(cross ? landed.host === target.host : sameSite(landed.hostname))) continue;
                const reader = response.body.getReader();
                const chunks = [];
                let size = 0;
                for (;;) {{
                    const {{ done, value }} = await reader.read();
                    if (done) break;
                    size += value.length;
                    if (size > {max_bytes}) {{ reader.cancel(); size = 0; break; }}
                    chunks.push(value);
                }}
                if (size === 0) continue;
                let bytes = new Uint8Array(await new Blob(chunks).arrayBuffer());
                if (/svg/i.test(response.headers.get('content-type') || '') || landed.pathname.toLowerCase().endsWith('.svg')) {{
                    bytes = await rasterize(new Blob([bytes], {{ type: 'image/svg+xml' }}));
                    if (!bytes) continue;
                }}
                let text = '';
                for (let i = 0; i < bytes.length; i += 8192) text += String.fromCharCode.apply(null, bytes.subarray(i, i + 8192));
                return btoa(text);
            }} catch (e) {{}}
        }}
        return '';",
        list = list.join(",")
    ))
}

/// A double-quoted JS string literal, or None for text that would need escaping; URLs from the
/// `url` crate never do, since it percent-encodes quotes, backslashes and non-ASCII.
pub fn script_string(text: &str) -> Option<String> {
    (!text.contains(['"', '\\', '\n', '\r'])).then(|| format!("\"{text}\""))
}

/// Netflix pauses or stops playback once its tab is hidden or the window loses focus, so on its
/// pages the document always reports itself visible and focused.
pub const ALWAYS_VISIBLE: &str = "(() => { if (!/(^|\\.)netflix\\.com$/.test(location.hostname)) return; \
    const D = Document.prototype; \
    for (const [name, value] of [['hidden', false], ['webkitHidden', false], ['visibilityState', 'visible'], ['webkitVisibilityState', 'visible']]) \
    Object.defineProperty(D, name, { configurable: true, get: () => value }); \
    D.hasFocus = { hasFocus() { return true; } }.hasFocus; \
    for (const type of ['visibilitychange', 'webkitvisibilitychange', 'blur']) \
    window.addEventListener(type, e => { if (e.target === window || e.target === document) e.stopImmediatePropagation(); }, true); })();";

/// Removes the style sheet `site_styles_script` added, so running it again replaces it.
pub const REMOVE_SITE_STYLES: &str = "globalThis.tarnSiteStyle = null; document.querySelectorAll('style[data-tarn-site]').forEach(e => e.remove());";

/// A start-of-document script that gives the top frame of an `http(s)` page the style sheets whose
/// pattern matches its host, joined in the order given, as the text of a `<style>` element, so the
/// CSS is never parsed as markup or script. A pattern is a host, `*.host` (it and its subdomains) or
/// `*` (every host), several joined by `,`. The element moves to the end of the root once parsed, after `<body>`, so it
/// wins ties against the page's own sheets.
pub fn site_styles_script(styles: &[(String, String)]) -> Option<String> {
    if styles.is_empty() {
        return None;
    }
    let entries: Vec<String> = styles.iter().map(|(host, css)| format!("[{},{}]", handover::js_string(host), handover::js_string(css))).collect();
    Some(format!(
        "(() => {{ if (window.top !== window || !/^https?:$/.test(location.protocol)) return; {REMOVE_SITE_STYLES} \
        const h = location.hostname.replace(/\\.$/, '').toLowerCase(); if (!h) return; \
        const css = [{}].filter(([m]) => m.split(',').some(p => p === '*' || p === h || (p.startsWith('*.') && (h === p.slice(2) || h.endsWith(p.slice(1)))))) \
        .map(e => e[1]).filter(Boolean).join('\\n'); if (!css) return; \
        const s = document.createElement('style'); s.dataset.tarnSite = ''; s.textContent = css; globalThis.tarnSiteStyle = s; \
        const add = () => {{ if (globalThis.tarnSiteStyle !== s) return true; const r = document.documentElement; if (r) r.append(s); return !!r; }}; \
        if (!add()) new MutationObserver((_, o) => {{ if (add()) o.disconnect(); }}).observe(document, {{ childList: true }}); \
        document.addEventListener('DOMContentLoaded', add, {{ once: true }}); }})();",
        entries.join(",")
    ))
}

/// A script that gives a frame on `host` the filter lists' element hiding `css` as a `<style>`
/// element; it does nothing in a document of another host.
pub fn frame_hiding_script(host: &str, css: &str) -> String {
    format!(
        "(() => {{ if (location.hostname.replace(/\\.$/, '').toLowerCase() !== {}) return; \
        const s = document.createElement('style'); s.textContent = {}; \
        const add = () => {{ const r = document.documentElement; if (r) r.append(s); return !!r; }}; \
        if (!add()) new MutationObserver((_, o) => {{ if (add()) o.disconnect(); }}).observe(document, {{ childList: true }}); }})();",
        handover::js_string(&host.trim_end_matches('.').to_ascii_lowercase()),
        handover::js_string(css)
    )
}

pub const MAX_THEME_SHEETS: usize = 64 * 1024;

pub fn outside_lockdown(script: String, levels: &crate::view_policy::Levels) -> String {
    let locked = levels.default == shield::ShieldLevel::Lockdown;
    let mut hosts: Vec<String> =
        levels.sites.iter().filter(|(_, l)| (**l == shield::ShieldLevel::Lockdown) != locked).map(|(h, _)| handover::js_string(h)).collect();
    if !locked && hosts.is_empty() {
        return script;
    }
    hosts.sort();
    let skip = if locked { "!" } else { "" };
    format!("(() => {{ if ({skip}[{}].includes(location.hostname.replace(/\\.$/, '').toLowerCase())) return; {script} }})();", hosts.join(","))
}

pub fn theme_sheets(value: &serde_json::Value) -> Vec<(String, String)> {
    let mut total = 0;
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|pair| Some((pair.get(0)?.as_str()?.to_owned(), pair.get(1)?.as_str()?.to_owned())))
        .filter(|(hosts, _)| {
            hosts.split(',').all(|h| {
                h == "*" || h.strip_prefix("*.").unwrap_or(h).split('.').all(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
            })
        })
        .take_while(|(_, css)| {
            total += css.len();
            total <= MAX_THEME_SHEETS
        })
        .collect()
}

/// Forced dark mode: the page is inverted and its media inverted back. The page keeps a light
/// colour scheme, as a dark one would be inverted to light.
pub const DARK_CSS: &str = "html { color-scheme: light !important; filter: invert(1) hue-rotate(180deg) !important; background-color: #fff; } \
    img, video, picture, canvas, iframe, embed, object, svg image, [style*=\"url(\"] { filter: invert(1) hue-rotate(180deg) !important; } \
    :is(picture, embed, object, [style*=\"url(\"]) :is(img, video, picture, canvas, iframe, svg image, [style*=\"url(\"]) { filter: none !important; }";

/// Each host's style sheet: forced dark first, then the user's own CSS, which may adjust it.
pub fn site_styles(css: Vec<(String, String)>, dark: Vec<String>) -> Vec<(String, String)> {
    let mut styles: std::collections::BTreeMap<String, String> = dark.into_iter().map(|host| (host, DARK_CSS.to_owned())).collect();
    for (host, css) in css {
        let sheet = styles.entry(host).or_default();
        if !sheet.is_empty() {
            sheet.push('\n');
        }
        sheet.push_str(&css);
    }
    styles.into_iter().collect()
}

/// The page's JSON as text when it is a JSON document of at most `max_bytes`, else empty; empty
/// once the tab shows another origin. An async function body for the engine's isolated world.
pub fn json_text_script(origin: &Url, max_bytes: usize) -> Option<String> {
    let origin = script_string(&origin.origin().ascii_serialization())?;
    Some(format!(
        r"if (location.origin !== {origin}) return '';
            if (!/^application\/([a-z0-9.+-]+\+)?json$/i.test(document.contentType)) return '';
            const text = document.body ? document.body.innerText : '';
            return text.length > {max_bytes} || new TextEncoder().encode(text).length > {max_bytes} ? '' : text;"
    ))
}

/// Toggles picture in picture for the page's playing or largest video: "pip", "exit", "none" or
/// "refused". An async function body for the engine's isolated world.
pub const PICTURE_IN_PICTURE: &str = r"const area = v => v.clientWidth * v.clientHeight;
            const videos = Array.from(document.querySelectorAll('video')).filter(v => v.readyState > 0).sort((a, b) => area(b) - area(a));
            const video = videos.find(v => !v.paused) || videos[0];
            if (!video) return 'none';
            if (document.pictureInPictureElement === video) { await document.exitPictureInPicture(); return 'exit'; }
            if (video.webkitPresentationMode === 'picture-in-picture') { video.webkitSetPresentationMode('inline'); return 'exit'; }
            try { await video.requestPictureInPicture(); return 'pip'; } catch (e) {}
            if (!video.disablePictureInPicture && video.webkitSupportsPresentationMode && video.webkitSupportsPresentationMode('picture-in-picture')) {
                video.webkitSetPresentationMode('picture-in-picture');
                return 'pip';
            }
            return 'refused';";

#[cfg(test)]
mod tests {
    use super::*;

    const HOSTILE_CSS: &str = "</style><script>globalThis.pwned=1</script>\"'`${x}\\\u{2028}\nbody{}";

    #[test]
    fn site_styles_carry_no_markup_and_need_a_sheet() {
        assert_eq!(site_styles_script(&[]), None);
        let script = site_styles_script(&[("a.test".into(), HOSTILE_CSS.into())]).unwrap();
        assert!(!script.contains(['<', '\n', '\u{2028}']) && script.contains(&crate::handover::js_string(HOSTILE_CSS)));
    }

    #[test]
    fn frame_hiding_names_its_host_and_quotes_the_sheet() {
        let css = ".ad{display:none}\"'";
        let script = frame_hiding_script("Ads.Example.", css);
        assert!(script.contains(&format!("!== {}) return;", handover::js_string("ads.example"))), "{script}");
        assert!(script.contains(&format!("s.textContent = {};", handover::js_string(css))), "{script}");
    }

    #[test]
    fn theme_sheets_keep_their_order_and_refuse_bad_hosts_and_excess() {
        let v = serde_json::json!([["*", "a{}"], ["b.test,*.c.test", "b{}"], ["", "x"], ["a b", "y"], ["a\\", "q"], [1, "z"], ["d.test"]]);
        assert_eq!(theme_sheets(&v), [("*".to_owned(), "a{}".to_owned()), ("b.test,*.c.test".to_owned(), "b{}".to_owned())]);
        let big = serde_json::json!([["*", "p".repeat(MAX_THEME_SHEETS)], ["e.test", "x"]]);
        assert_eq!(theme_sheets(&big).len(), 1);
        assert!(theme_sheets(&serde_json::json!({ "a.test": "x" })).is_empty());
        let odd = serde_json::json!([["a*b", "x"], ["*.*.x", "x"], ["a..b", "x"], ["*x", "x"], ["*.ok.test,ok.test", "y"]]);
        assert_eq!(theme_sheets(&odd), [("*.ok.test,ok.test".to_owned(), "y".to_owned())]);
        let levels = |v: serde_json::Value| crate::view_policy::Levels::parse(&v, shield::ShieldLevel::Standard);
        assert_eq!(outside_lockdown("run();".into(), &levels(serde_json::json!({ "a.test": "strict" }))), "run();");
        let skips = outside_lockdown("run();".into(), &levels(serde_json::json!({ "bank.test": "lockdown", "a.test": "strict" })));
        assert!(skips.contains("if ([\"bank.test\"].includes(") && skips.ends_with("return; run(); })();"), "{skips}");
        let only = outside_lockdown("run();".into(), &levels(serde_json::json!({ "*": "lockdown", "open.test": "standard", "b.test": "lockdown" })));
        assert!(only.contains("if (![\"open.test\"].includes("), "a Lockdown default runs only on hosts set to another level: {only}");
    }

    /// `cargo test -p engine site_styles -- --ignored`.
    #[test]
    #[ignore = "needs node"]
    fn site_styles_apply_to_matching_hosts_top_frame_only() {
        let check = r#"
            const vm = require('vm');
            const run = (host, top, late, again) => {
                const added = new Set(), observers = [], loaded = [];
                const root = { append: s => added.add(s) };
                const document = { documentElement: late ? null : root, createElement: () => ({ textContent: '', dataset: {}, remove() { added.delete(this); } }),
                    querySelectorAll: () => [...added].map(s => ({ remove: () => added.delete(s) })),
                    addEventListener: (type, f) => type === 'DOMContentLoaded' && loaded.push(f) };
                class MutationObserver { constructor(f) { this.f = f; observers.push(this); } observe() {} disconnect() { this.done = true; } }
                const window = {}; window.top = top ? window : {};
                const [protocol, hostname] = host.includes('//') ? host.split('//') : ['https:', host];
                const context = vm.createContext({ window, document, MutationObserver, location: { protocol, hostname } });
                vm.runInContext(process.argv[1], context);
                if (again) { context.location.hostname = 'none.test'; vm.runInContext(process.argv[1], context); }
                if (late) { document.documentElement = root; observers.forEach(o => o.f([], o)); }
                loaded.forEach(f => f());
                if (globalThis.pwned) throw new Error('ran');
                const [sheet] = added;
                if (added.size !== 1) return added.size;
                return sheet.textContent === process.argv[2] ? 'css' : sheet.textContent.split('\n').join('+');
            };
            console.log([
                run('a.test', true), run('a.test', true, true), run('a.test', false), run('www.a.test', true), run('b.test', true),
                run('A.TEST.', true), run('file://a.test', true), run('about://', true), run('x.sub.test', true), run('sub.test', true),
                run('notsub.test', true), run('other.test', true),
                run('a.test', true, true, true), run('a.test', true, false, true),
            ].join(' '));
        "#;
        let script = site_styles_script(&[
            ("a.test".into(), HOSTILE_CSS.into()),
            ("b.test".into(), String::new()),
            ("*".into(), "all".into()),
            ("*.sub.test".into(), "sub".into()),
            ("other.test,*.sub.test".into(), "both".into()),
        ])
        .unwrap();
        let output = std::process::Command::new("node").args(["-e", check, &script, &format!("{HOSTILE_CSS}\nall")]).output().expect("node");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "css css 0 all all css 0 0 all+sub+both all+sub+both all all+both all all",
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn dark_comes_first_and_hosts_stay_apart() {
        let styles = site_styles(vec![("a.test".into(), "p{}".into()), ("c.test".into(), "q{}".into())], vec!["a.test".into(), "b.test".into()]);
        let hosts: Vec<&str> = styles.iter().map(|(h, _)| h.as_str()).collect();
        assert_eq!(hosts, ["a.test", "b.test", "c.test"]);
        assert!(styles[0].1.starts_with(DARK_CSS) && styles[0].1.ends_with("\np{}"));
        assert_eq!(styles[1].1, DARK_CSS);
        assert_eq!(styles[2].1, "q{}");
    }
}

#[cfg(all(test, target_os = "macos"))]
mod javascript_core_tests {
    use super::*;

    /// Runs `find_text_script` in JavaScriptCore (through osascript) over a page body whose text
    /// nodes are `nodes`, each as (parent element, text).
    fn find_in_nodes(nodes: &[(&str, &str)], query: &str) -> Vec<String> {
        let nodes: Vec<String> = nodes.iter().map(|(tag, text)| format!("[{}, {}]", handover::js_string(tag), handover::js_string(text))).collect();
        let script = format!(
            "const NodeFilter = {{ SHOW_TEXT: 4, FILTER_ACCEPT: 1, FILTER_REJECT: 2 }};
            const nodes = [{}];
            const document = {{ body: {{}}, createTreeWalker: (root, what, filter) => {{
                let i = 0;
                return {{ nextNode: () => {{
                    while (i < nodes.length) {{
                        const [nodeName, data] = nodes[i++], node = {{ data, parentElement: {{ nodeName }} }};
                        if (filter.acceptNode(node) === NodeFilter.FILTER_ACCEPT) return node;
                    }}
                    return null;
                }} }};
            }} }};
            (function () {{ {} }})()",
            nodes.join(","),
            find_text_script(query)
        );
        let out = std::process::Command::new("osascript").args(["-l", "JavaScript", "-e", &script]).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        find_snippets(String::from_utf8_lossy(&out.stdout).trim_end_matches('\n'))
    }

    fn find_in(text: &str, query: &str) -> Vec<String> {
        find_in_nodes(&[("P", text)], query)
    }

    #[test]
    fn find_text_returns_bounded_snippets_around_each_match() {
        let page = format!("{}Needle\n\n{} needle two and NEEDLE three, needle four", "a".repeat(100), "b".repeat(100));
        let found = find_in(&page, "needle");
        assert_eq!(found.len(), FIND_MATCHES);
        assert_eq!(found[0], format!("{}Needle {}", "a".repeat(60), "b".repeat(58)), "60 characters each side, whitespace folded");
        assert!(found[1].contains("needle two"));
        assert!(find_in(&page, "absent").is_empty());
        assert_eq!(find_in(&format!("{}😀{}needle", "x".repeat(10), "y".repeat(59)), "needle"), [format!("\u{fffd}{}needle", "y".repeat(59))]);
        assert!(find_in(&page, "").is_empty());
    }

    #[test]
    fn find_text_takes_the_query_as_a_string_literal() {
        assert_eq!(find_in(r#"say "it's" \ </script> here"#, r#""it's" \ </script>"#), [r#"say "it's" \ </script> here"#]);
        assert!(find_text_script(&"q".repeat(500)).contains(&format!("\"{}\"", "q".repeat(FIND_QUERY_MAX))));
    }

    #[test]
    fn find_text_reads_text_nodes_and_keeps_offsets_exact() {
        let found = find_in_nodes(&[("P", "first "), ("SCRIPT", "needle in code"), ("B", "needle"), ("P", " last")], "NEEDLE");
        assert_eq!(found, ["first needle last"]);
        let dotted = "\u{130}".repeat(70);
        assert_eq!(find_in(&format!("{dotted}needle{}", "x".repeat(70)), "needle"), [format!("{}needle{}", "\u{130}".repeat(60), "x".repeat(60))]);
        assert_eq!(find_in("a.b axb", "a.b"), ["a.b axb"], "the query is literal text");
        assert_eq!(find_in_nodes(&[("B", "Ne"), ("B", "edle")], "needle"), ["Needle"], "inline tags do not split a word");
    }

    #[test]
    fn find_snippets_bounds_what_the_page_returns() {
        let long = "x".repeat(10_000);
        let found = find_snippets(&format!("{long}\n\n \na\nb\nc\nd"));
        assert_eq!(found.len(), FIND_MATCHES);
        assert_eq!(found[0].chars().count(), 2 * FIND_CONTEXT + FIND_QUERY_MAX);
        assert_eq!(found[1..], ["a", "b"]);
    }
}
