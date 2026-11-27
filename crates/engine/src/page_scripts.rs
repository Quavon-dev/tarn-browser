use url::Url;

use crate::handover;

/// Builds a selector of the shape `shield::lists::valid_zap` accepts: from the element up to the
/// nearest unique id, or to a child of `<body>`, each step its tag, up to two classes and its
/// position among siblings of the same tag. `pick_rect_script` answers a rectangle instead.
pub const PICKER: &str = r"if (window.tarnPick) window.tarnPick();
return await new Promise(resolve => {
    const asRect = typeof pickRect !== 'undefined';
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
    const rectOf = el => { const r = el.getBoundingClientRect(); return JSON.stringify([r.left, r.top, r.width, r.height]); };
    const click = e => { if (!e.isTrusted) return; stop(e); const s = target && (asRect ? rectOf(target) : selector(target)); if (s) finish(s); };
    const key = e => { if (e.isTrusted && e.key === 'Escape') { stop(e); finish(''); } };
    const events = [['mouseover', move], ['mousedown', stop], ['mouseup', stop], ['click', click], ['keydown', key]];
    const finish = s => {
        for (const [type, f] of events) removeEventListener(type, f, true);
        box.remove();
        window.tarnPick = null;
        if (asRect && s) requestAnimationFrame(() => requestAnimationFrame(() => resolve(s)));
        else resolve(s);
    };
    window.tarnPick = () => finish('');
    for (const [type, f] of events) addEventListener(type, f, true);
});";

/// `PICKER` answering the picked element's rectangle in the visible page, `[x, y, width, height]`
/// in CSS pixels as JSON, once its outline is off the screen; for Capture Element
/// (`browser_core::capture::element_area`).
pub fn pick_rect_script() -> String {
    format!("const pickRect = true;\n{PICKER}")
}

/// Types `text` into the focused text field of the top document with the editing command the
/// page's own undo knows, as the user's explicit Insert Snippet; answers "1" when a field had the
/// focus and "" otherwise (none, read only, or a password field). It listens for nothing. An async
/// function body for the engine's isolated world, run as the user's gesture.
pub fn insert_text_script(text: &str) -> String {
    let text = handover::js_string(text);
    format!(
        r"const el = document.activeElement;
            const typed = el && el.localName === 'input' && /^(text|search|email|url|tel)$/.test(el.type);
            if (!el || !(el.isContentEditable || el.localName === 'textarea' || typed) || el.readOnly || el.disabled) return '';
            return document.execCommand('insertText', false, {text}) ? '1' : '';"
    )
}

/// Most snippets a page gives Find in Tabs, the characters of context on each side of a match, the
/// longest query passed on, and how much of the page's text is searched.
pub const FIND_MATCHES: usize = 3;
const FIND_CONTEXT: usize = 60;
pub const FIND_QUERY_MAX: usize = 200;
pub const FIND_TEXT_MAX: usize = 1_000_000;
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

/// Clears the page's selection, from which WebKit's find continues.
pub const FIND_RESTART: &str = "getSelection().removeAllRanges()";

/// Most matches the find bar counts.
pub const FIND_COUNT_MAX: usize = 1000;

/// Counts how often `query` occurs in the page's visible text, without case, up to
/// `FIND_COUNT_MAX`; the engine's own find decides whether there is a match at all. A query past
/// `FIND_QUERY_MAX` counts none, since a shortened one could count too many.
pub fn find_count_script(query: &str) -> String {
    let query = if query.chars().count() > FIND_QUERY_MAX { String::new() } else { query.to_lowercase() };
    let query = handover::js_string(&query);
    format!(
        r"const q = {query};
        if (!q || !document.body) return '0';
        const text = document.body.innerText.slice(0, {FIND_TEXT_MAX}).toLowerCase();
        let count = 0;
        for (let at = text.indexOf(q); at !== -1 && count < {FIND_COUNT_MAX}; at = text.indexOf(q, at + q.length)) count++;
        return String(count);"
    )
}

/// The number of matches to show: none when the engine found none, at least one when it found
/// some (the page's count misses text in frames), at most `FIND_COUNT_MAX`.
pub fn find_count(found: bool, answer: &str) -> usize {
    if !found {
        return 0;
    }
    answer.trim().parse::<usize>().unwrap_or(0).clamp(1, FIND_COUNT_MAX)
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
///
/// `DARK_CSS` gets its own element ahead of the rest, enabled only while the page looks light: a
/// page whose own colour scheme is dark (or light and dark while the system is dark) never is; else
/// the first opaque background of `<body>` or `<html>` decides, else white. It
/// is decided again on `DOMContentLoaded`, `load` and, 100 ms after the last one, changes to the
/// theme attributes of `<html>` and `<body>`, to `<html>`'s children or to the system's appearance.
pub fn site_styles_script(styles: &[(String, String)]) -> Option<String> {
    if styles.is_empty() {
        return None;
    }
    let entries: Vec<String> = styles.iter().map(|(host, css)| format!("[{},{}]", handover::js_string(host), handover::js_string(css))).collect();
    Some(format!(
        "(() => {{ if (window.top !== window || !/^https?:$/.test(location.protocol)) return; {REMOVE_SITE_STYLES} \
        const h = location.hostname.replace(/\\.$/, '').toLowerCase(); if (!h) return; \
        const all = [{}].filter(([m]) => m.split(',').some(p => p === '*' || p === h || (p.startsWith('*.') && (h === p.slice(2) || h.endsWith(p.slice(1)))))) \
        .map(e => e[1]).filter(Boolean).join('\\n'); if (!all) return; \
        const D = {}, css = all.split(D).join(''); \
        const sheet = text => {{ const e = document.createElement('style'); e.dataset.tarnSite = ''; e.textContent = text; return e; }}; \
        const s = sheet(css), d = css !== all && sheet(D); globalThis.tarnSiteStyle = s; let t; \
        const bg = e => {{ const c = e ? getComputedStyle(e).backgroundColor : ''; const v = c.startsWith('rgb') ? c.match(/[\\d.]+/g).map(Number) : []; \
        return v.length > 2 && v[3] !== 0 ? 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2] > 128 : null; }}; \
        const light = () => {{ d.media = 'not all'; const m = document.querySelector('meta[name=color-scheme]'); \
        const cs = getComputedStyle(document.documentElement).colorScheme + ' ' + (m ? m.content : ''); \
        if (cs.includes('dark') && (!cs.includes('light') || matchMedia('(prefers-color-scheme: dark)').matches)) return false; \
        const b = bg(document.body); if (b !== null) return b; const r = bg(document.documentElement); return r !== null ? r : true; }}; \
        const check = () => {{ if (globalThis.tarnSiteStyle !== s) return o.disconnect(); if (!document.documentElement) return; \
        const want = light() ? '' : 'not all'; if (d.media !== want) d.media = want; \
        if (document.body) o.observe(document.body, {{ attributes: true, attributeFilter: ['class', 'style', 'bgcolor'] }}); }}; \
        const o = d && new MutationObserver(() => {{ clearTimeout(t); t = setTimeout(check, 100); }}); \
        const add = () => {{ if (globalThis.tarnSiteStyle !== s) return true; const r = document.documentElement; if (!r) return false; \
        if (d) {{ r.append(d); o.observe(r, {{ attributes: true, attributeFilter: ['class', 'style', 'data-theme', 'data-color-mode'], childList: true }}); check(); }} if (css) r.append(s); return true; }}; \
        if (!add()) new MutationObserver((_, o) => {{ if (add()) o.disconnect(); }}).observe(document, {{ childList: true }}); \
        document.addEventListener('DOMContentLoaded', add, {{ once: true }}); if (d) {{ addEventListener('load', check, {{ once: true }}); matchMedia('(prefers-color-scheme: dark)').addEventListener('change', check); }} }})();",
        entries.join(","),
        handover::js_string(DARK_CSS)
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
/// colour scheme, as a dark one would be inverted to light. The top layer is drawn outside
/// `<html>`'s filter: dialogs and pop-overs there are inverted on their own, and what is in full
/// screen shows as it is.
pub const DARK_CSS: &str = "html { color-scheme: light !important; filter: invert(1) hue-rotate(180deg) !important; background-color: #fff; } \
    img, video, picture, canvas, iframe, embed, object, input[type=image], svg image, [style*=\"url(\"], :is(:modal, :popover-open) { filter: invert(1) hue-rotate(180deg) !important; } \
    :is(picture, embed, object, [style*=\"url(\"]) :is(img, video, picture, canvas, iframe, svg image, [style*=\"url(\"]), \
    :fullscreen, :fullscreen :is(img, video, picture, canvas, iframe, embed, object, svg image, [style*=\"url(\"]) { filter: none !important; }";

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

/// What `picture_in_picture_script` does with the page's video.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipMode {
    /// The user's command: the playing or largest video goes in, or back out.
    Toggle,
    /// The tab left the screen: only a playing, audible video of at least 200 by 200 CSS pixels
    /// goes in, and nothing while a video of the page already is in.
    Enter,
    /// The tab is back on screen: the video in picture in picture goes back into the page.
    Exit,
}

const PIP_ENTER_PICK: &str = "document.pictureInPictureElement || videos.some(shown) ? 'open' : videos.find(v => !v.paused && !v.ended && !v.muted && v.volume > 0 && v.videoWidth > 0 && v.clientWidth >= 200 && v.clientHeight >= 200)";
const PIP_ENTER: &str = r"try { await video.requestPictureInPicture(); return 'pip'; } catch (e) {}
            if (!video.disablePictureInPicture && video.webkitSupportsPresentationMode && video.webkitSupportsPresentationMode('picture-in-picture')) {
                video.webkitSetPresentationMode('picture-in-picture');
                return 'pip';
            }
            return 'refused';";

/// Picture in picture for the page's video, as `mode` says: "pip", "open" (one already in),
/// "exit", "none" or "refused". An async function body for the engine's isolated world.
pub fn picture_in_picture_script(mode: PipMode) -> String {
    match mode {
        PipMode::Toggle => pip_script("videos.find(v => !v.paused) || videos[0]", "return await leave(video);", PIP_ENTER),
        PipMode::Enter => pip_script(PIP_ENTER_PICK, "return 'open';", PIP_ENTER),
        PipMode::Exit => pip_script("videos.find(shown)", "return await leave(video);", ""),
    }
}

/// `PipMode::Enter`'s choice without entering: "ready" when it would ask for picture in picture.
/// Run without a gesture, so a page lends one only for a video Enter takes.
pub fn picture_in_picture_ready_script() -> String {
    pip_script(PIP_ENTER_PICK, "return 'open';", "return 'ready';")
}

fn pip_script(pick: &str, shown: &str, enter: &str) -> String {
    format!(
        r"const area = v => v.clientWidth * v.clientHeight;
            const shown = v => document.pictureInPictureElement === v || v.webkitPresentationMode === 'picture-in-picture';
            const leave = async v => {{
                if (document.pictureInPictureElement === v) await document.exitPictureInPicture();
                else v.webkitSetPresentationMode('inline');
                return 'exit';
            }};
            const videos = Array.from(document.querySelectorAll('video')).filter(v => v.readyState > 0).sort((a, b) => area(b) - area(a));
            const video = {pick};
            if (video === 'open') return 'open';
            if (!video) return 'none';
            if (shown(video)) {shown}
            {enter}"
    )
}

/// `lifecycle::Hang`'s ping: it reads nothing, and answers only once the page's main thread is free.
pub const PING: &str = "return '';";

/// Most UTF-16 units of the page's selection `SELECTED_TEXT` hands over.
pub const SELECTION_MAX: usize = 64 * 1024;

/// The top document's selected text, at most `SELECTION_MAX` UTF-16 units; empty while a password,
/// card or one-time-code field has the focus, also inside a shadow root. An async function body for
/// the engine's isolated world.
pub const SELECTED_TEXT: &str = r"let active = document.activeElement;
            while (active && active.shadowRoot && active.shadowRoot.activeElement) active = active.shadowRoot.activeElement;
            const secret = t => /^(cc-.+|current-password|new-password|one-time-code)$/.test(t);
            if (active && ((active.localName === 'input' && active.type === 'password')
                || String(active.getAttribute && active.getAttribute('autocomplete') || '').toLowerCase().split(/\s+/).some(secret))) return '';
            const selection = getSelection();
            const text = selection ? String(selection).slice(0, 65536) : '';
            return text.toWellFormed ? text.toWellFormed() : text;";

/// `body` answering the address of the document it read, a newline, then its own answer, so the
/// shell can drop an answer from a page the tab has left (`browser_core::assistant::read_here`).
pub fn with_address(body: &str) -> String {
    format!("const read = await (async () => {{ {body} }})();\n            return location.href + '\\n' + (typeof read === 'string' ? read : '');")
}

/// Hands `done` the address and the text a `with_address` body answered; both empty for an empty
/// answer, as under Lockdown.
pub fn split_address(done: impl Fn(String, String) + 'static) -> impl Fn(String) + 'static {
    move |answer| {
        let (url, text) = answer.split_once('\n').unwrap_or_default();
        done(url.to_owned(), text.to_owned());
    }
}

/// The furthest one `MediaCommand::SeekBy` moves, in seconds either way.
pub const MEDIA_SEEK_MAX: f64 = 3600.;

/// What the shell asks of the page's first audible media element.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MediaCommand {
    Play,
    Pause,
    SeekBy(f64),
}

/// The first audible media element of the top document: playing, else paused after it started.
const MEDIA_PICK: &str = r"const media = [...document.querySelectorAll('video, audio')].slice(0, 200);
            const audible = m => !m.muted && m.volume > 0 && m.readyState > 0 && !m.ended;
            const m = media.find(m => audible(m) && !m.paused) || media.find(m => audible(m) && m.currentTime > 0);
            if (!m) return '';
            ";

/// Answers the first audible media element as JSON `{playing, title, duration, position}`, with
/// the media session's title, else the document's; empty when there is none. It only reads: no
/// artwork is fetched. An async function body for the engine's isolated world.
pub fn media_state_script() -> String {
    format!(
        r"{MEDIA_PICK}const meta = navigator.mediaSession && navigator.mediaSession.metadata;
            const time = v => Number.isFinite(v) && v >= 0 ? v : null;
            const title = String((meta && meta.title) || document.title || '').slice(0, {});
            const pip = document.pictureInPictureElement === m || m.webkitPresentationMode === 'picture-in-picture';
            return JSON.stringify({{ playing: !m.paused, title: title.toWellFormed ? title.toWellFormed() : title, duration: time(m.duration), position: time(m.currentTime), pip }});",
        crate::view_policy::MAX_TITLE
    )
}

/// Plays, pauses or seeks the element `media_state_script` reads; a seek stays inside the media.
/// An async function body for the engine's isolated world, run as the user's gesture.
pub fn media_command_script(command: MediaCommand) -> String {
    let action = match command {
        MediaCommand::Play => "await m.play().catch(() => {});".to_owned(),
        MediaCommand::Pause => "m.pause();".to_owned(),
        MediaCommand::SeekBy(by) => {
            let by = if by.is_finite() { by.clamp(-MEDIA_SEEK_MAX, MEDIA_SEEK_MAX) } else { 0. };
            format!("m.currentTime = Math.max(0, Math.min(Number.isFinite(m.duration) ? m.duration : Infinity, m.currentTime + ({by})));")
        }
    };
    format!("{MEDIA_PICK}{action}\n            return '';")
}

/// The document's scroll offset and size as `[x, y, width, height]` in CSS pixels, for an engine
/// that needs a full-page snapshot's area spelled out.
pub const DOCUMENT_SIZE: &str = r"const d = document.documentElement;
            return JSON.stringify([scrollX, scrollY, Math.max(d.scrollWidth, d.clientWidth), Math.max(d.scrollHeight, d.clientHeight)]);";

/// `DOCUMENT_SIZE`'s answer, the size cut to `view_policy::SNAPSHOT_MAX_SIDE` and its height to
/// what fits `view_policy::SNAPSHOT_MAX_PIXELS` at `scale` device pixels per CSS pixel.
pub fn document_size(answer: &str, scale: f64) -> Option<[f64; 4]> {
    let [x, y, w, h]: [f64; 4] = serde_json::from_str(answer).ok()?;
    let side = |v: f64| (v.is_finite() && v >= 1.).then(|| v.min(crate::view_policy::SNAPSHOT_MAX_SIDE));
    let offset = |v: f64| if v.is_finite() { v.max(0.) } else { 0. };
    let w = side(w)?;
    Some([offset(x), offset(y), w, crate::view_policy::snapshot_height(w, side(h)?, scale)?])
}

/// What sleeping would lose on a page: whether the user changed a form field, and the scroll offset.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct PageKeep {
    pub edited: bool,
    pub scroll: (f64, f64),
}

/// Answers `PageKeep` as JSON. A field counts once the user has interacted with it (`:user-valid`
/// or `:user-invalid`), or while it has focus on a page the user has used, so values a page script
/// fills in do not keep a tab awake. Only the verdict leaves the page, never a value. An async function body for the engine's
/// isolated world.
pub const PAGE_KEEP: &str = r"const first = e => { const o = [...e.options], d = o.findIndex(o => o.defaultSelected); return d >= 0 ? d : e.size > 1 ? -1 : o.findIndex(o => !o.disabled); };
            const used = typeof navigator === 'undefined' || !navigator.userActivation || navigator.userActivation.hasBeenActive;
            const user = e => { try { return e.matches(':user-valid, :user-invalid'); } catch { return true; } };
            const changed = e => e.type === 'checkbox' || e.type === 'radio' ? e.checked !== e.defaultChecked
                : e.localName === 'select' ? (e.multiple ? [...e.options].some(o => o.selected !== o.defaultSelected) : e.selectedIndex !== first(e))
                : e.type === 'file' ? e.files.length > 0 : e.value !== e.defaultValue;
            const skip = /^(hidden|submit|button|reset|image)$/, active = document.activeElement;
            const fields = [...document.querySelectorAll('input, textarea, select')].slice(0, 2000);
            const edited = fields.some(e => !skip.test(e.type) && !e.disabled && (used && e === active || user(e)) && changed(e))
                || !!(used && active && active.isContentEditable && active.textContent.trim());
            return JSON.stringify({ edited, x: scrollX, y: scrollY });";

pub fn page_keep(answer: &str) -> PageKeep {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(answer) else { return PageKeep::default() };
    let offset = |key: &str| value[key].as_f64().filter(|v| v.is_finite() && *v > 0.).unwrap_or(0.);
    PageKeep { edited: value["edited"].as_bool().unwrap_or(false), scroll: (offset("x"), offset("y")) }
}

/// Scrolls back to `(x, y)`, retrying while the page grows toward it, until the user scrolls. An
/// async function body for the engine's isolated world.
pub fn scroll_script((x, y): (f64, f64)) -> String {
    let (x, y) = (if x.is_finite() { x.max(0.) } else { 0. }, if y.is_finite() { y.max(0.) } else { 0. });
    format!(
        r"let last = -1;
            for (let i = 0; i < 10; i++) {{
                if (last >= 0 && Math.abs(scrollY - last) > 1) return '';
                scrollTo({{ left: {x}, top: {y}, behavior: 'instant' }});
                last = scrollY;
                if (Math.abs(scrollY - {y}) < 2) return '';
                await new Promise(r => setTimeout(r, 200));
            }}
            return '';"
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn page_keep_reads_the_verdict_and_a_sane_offset() {
        assert_eq!(page_keep(r#"{"edited":true,"x":0,"y":1200.5}"#), PageKeep { edited: true, scroll: (0., 1200.5) });
        assert_eq!(page_keep(r#"{"edited":"yes","x":-4,"y":null}"#), PageKeep::default());
        assert_eq!(page_keep(""), PageKeep::default(), "Lockdown and failed scripts answer empty");
        assert!(scroll_script((f64::NAN, -3.)).contains("left: 0, top: 0,"));
        assert!(scroll_script((0., 640.)).contains("left: 0, top: 640,"));
    }

    #[test]
    fn inserted_text_stays_a_string_literal() {
        let script = insert_text_script(HOSTILE_CSS);
        assert!(script.contains(&format!("execCommand('insertText', false, {})", handover::js_string(HOSTILE_CSS))));
        assert!(!script.contains("</script>"), "{script}");
        assert!(!script.contains("addEventListener"), "it listens for nothing");
    }

    #[test]
    fn an_answer_names_the_address_it_was_read_from() {
        let script = with_address(SELECTED_TEXT);
        assert!(script.contains(SELECTED_TEXT) && script.ends_with("return location.href + '\\n' + (typeof read === 'string' ? read : '');"), "{script}");
        let got = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let kept = got.clone();
        let split = split_address(move |url, text| kept.borrow_mut().push((url, text)));
        split("https://a.test/p\nline one\nline two".into());
        split(String::new());
        assert_eq!(*got.borrow(), [("https://a.test/p".into(), "line one\nline two".into()), (String::new(), String::new())]);
    }

    #[test]
    fn the_selection_read_skips_secret_fields_inside_shadow_roots_too() {
        assert!(SELECTED_TEXT.contains("active.shadowRoot.activeElement"));
        for token in ["cc-.+", "current-password", "new-password", "one-time-code"] {
            assert!(SELECTED_TEXT.contains(token), "{token}");
        }
        assert!(SELECTED_TEXT.contains(&format!(".slice(0, {SELECTION_MAX})")), "the script cuts at SELECTION_MAX");
    }

    #[test]
    fn document_size_is_bounded_and_media_seeks_stay_finite() {
        let max = crate::view_policy::SNAPSHOT_MAX_SIDE;
        assert_eq!(document_size("[10, -5, 1280, 1e9]", 1.), Some([10., 0., 1280., max]));
        assert_eq!(document_size("[0, 0, 1280, 16000]", 2.), Some([0., 0., 1280., 13107.]), "at most 64 MiP at the screen's scale");
        assert_eq!(document_size("[0, 0, 16384, 16384]", 1.), Some([0., 0., 16384., 4096.]));
        assert_eq!(document_size("[0, 0, 0, 800]", 1.), None);
        assert_eq!(document_size("[0, 0, 800]", 1.), None);
        assert_eq!(document_size("", 1.), None, "Lockdown answers empty");
        assert!(media_command_script(MediaCommand::SeekBy(f64::INFINITY)).contains("m.currentTime + (0)"));
        assert!(media_command_script(MediaCommand::SeekBy(-1e9)).contains(&format!("m.currentTime + ({})", -MEDIA_SEEK_MAX)));
    }

    use super::*;

    #[test]
    fn only_the_toggle_and_enter_ask_for_picture_in_picture() {
        let [toggle, enter, exit] = [PipMode::Toggle, PipMode::Enter, PipMode::Exit].map(picture_in_picture_script);
        assert!(toggle.contains("requestPictureInPicture") && toggle.contains("return await leave(video)"));
        assert!(enter.contains("requestPictureInPicture") && enter.contains("!v.muted") && enter.contains("return 'open'"));
        assert!(!enter.contains("return await leave(video)"), "entering never takes a video out");
        assert!(!exit.contains("requestPictureInPicture") && !exit.contains("webkitSetPresentationMode('picture-in-picture')"));
        let ready = picture_in_picture_ready_script();
        assert!(ready.contains("document.pictureInPictureElement || videos.some(shown) ? 'open'") && ready.contains("return 'ready'"));
        assert!(!ready.contains("requestPictureInPicture") && !ready.contains("webkitSetPresentationMode('picture-in-picture')"));
    }

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
    fn the_engine_decides_whether_anything_matched_and_the_page_how_often() {
        assert_eq!(find_count(false, "3"), 0);
        assert_eq!(find_count(true, "3"), 3);
        assert_eq!(find_count(true, "0"), 1);
        assert_eq!(find_count(true, "junk"), 1);
        assert_eq!(find_count(true, "99999"), FIND_COUNT_MAX);
        assert!(find_count_script("A\"b").contains(r#"const q = "a\"b";"#));
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

    /// Runs `PAGE_KEEP` in JavaScriptCore over form fields given as JS object literals; `user`
    /// marks a field as matching `:user-valid` / `:user-invalid`, `focus` makes it the active element.
    fn keep_of(fields: &str) -> PageKeep {
        keep_after(fields, true)
    }

    /// As `keep_of` on a page the user has (`used`) or has not yet clicked or typed in.
    fn keep_after(fields: &str, used: bool) -> PageKeep {
        let script = format!(
            "const navigator = {{ userActivation: {{ hasBeenActive: {used} }} }};
            const fields = [{fields}].map(f => ({{ localName: 'input', type: 'text', disabled: false, defaultValue: '', value: '', ...f,
                matches: () => !!f.user }}));
            const document = {{ activeElement: fields.find(f => f.focus) || null, querySelectorAll: () => fields }};
            fields.filter(f => f.options).forEach(f => f.options = f.options.map(o => ({{ disabled: false, defaultSelected: false, ...o }})));
            const scrollX = 0, scrollY = 300;
            (function () {{ {PAGE_KEEP} }})()"
        );
        let out = std::process::Command::new("osascript").args(["-l", "JavaScript", "-e", &script]).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        page_keep(String::from_utf8_lossy(&out.stdout).trim())
    }

    #[test]
    fn only_input_the_user_typed_keeps_a_page_awake() {
        assert_eq!(keep_of(""), PageKeep { edited: false, scroll: (0., 300.) });
        assert!(!keep_of("{ value: 'filled by the page' }").edited, "a value the page set is not the user's");
        assert!(keep_of("{ value: 'typed', user: true }").edited);
        assert!(keep_of("{ value: 'typing', focus: true }").edited);
        assert!(!keep_of("{ value: 'x', defaultValue: 'x', user: true }").edited, "an unchanged field");
        assert!(!keep_of("{ type: 'hidden', value: 'token', user: true }").edited);
        assert!(keep_of("{ type: 'checkbox', checked: true, defaultChecked: false, user: true }").edited);
        assert!(!keep_after("{ value: 'autofocused and filled', focus: true }", false).edited, "focus the page gave itself");
        let select = |index: u8, user: bool| {
            format!("{{ localName: 'select', type: 'select-one', options: [{{}}, {{}}], selectedIndex: {index}, user: {user}, focus: true }}")
        };
        assert!(!keep_of(&select(0, false)).edited, "an untouched select shows its first option");
        assert!(keep_of(&select(1, true)).edited);
        assert!(
            !keep_of("{ localName: 'select', type: 'select-one', size: 4, options: [{}, {}], selectedIndex: -1, focus: true }").edited,
            "an untouched list box"
        );
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
    fn find_count_counts_without_case_and_stops_at_the_cap() {
        let count = |text: &str, query: &str| {
            let script =
                format!("const document = {{ body: {{ innerText: {} }} }}; (function () {{ {} }})()", handover::js_string(text), find_count_script(query));
            let out = std::process::Command::new("osascript").args(["-l", "JavaScript", "-e", &script]).output().unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).trim().to_owned()
        };
        assert_eq!(count("Needle needle NEEDLE", "needle"), "3");
        assert_eq!(count("aaaa", "aa"), "2", "matches do not overlap");
        assert_eq!(count("a.b axb", "a.b"), "1", "the query is literal text");
        assert_eq!(count(&"x".repeat(FIND_COUNT_MAX + 50), "x"), FIND_COUNT_MAX.to_string());
        assert_eq!(count(&"y".repeat(500), &"y".repeat(FIND_QUERY_MAX + 1)), "0", "a query past the limit counts none");
    }

    /// Runs `site_styles_script` in JavaScriptCore on a page whose `<body>` and `<html>` have the
    /// given computed backgrounds, then sets the body's to `later`, if any, and reports the
    /// root's style elements in order: `dark`, `off` (dark, disabled) or `css`.
    fn dark_sheets(dark: bool, body: &str, html: &str, scheme: &str, meta: Option<&str>, prefers_dark: bool, later: Option<&str>) -> String {
        let script = site_styles_script(&site_styles(vec![("a.test".into(), "p{}".into())], if dark { vec!["a.test".into()] } else { vec![] })).unwrap();
        let js = |s: &str| handover::js_string(s);
        let state = format!(
            "const D = {dark_css}; const body = {{ bg: {body} }}, root = {{ bg: {html}, kids: [], append(e) {{ this.kids = this.kids.filter(k => k !== e).concat(e); }} }};
            const report = () => root.kids.map(k => k.textContent !== D ? 'css' : k.media === 'not all' ? 'off' : 'dark').join(',');
            const window = {{}}; window.top = window; const location = {{ protocol: 'https:', hostname: 'a.test' }}; const observers = [];
            const document = {{ documentElement: root, body, createElement: () => ({{ dataset: {{}}, media: '' }}), querySelectorAll: () => [],
                querySelector: () => {meta}, addEventListener() {{}} }};
            const getComputedStyle = e => ({{ backgroundColor: e.bg, colorScheme: {scheme} }}); const matchMedia = () => ({{ matches: {prefers_dark}, addEventListener() {{}} }});
            class MutationObserver {{ constructor(f) {{ this.f = f; observers.push(this); }} observe() {{}} disconnect() {{}} }}
            const setTimeout = f => f(), clearTimeout = () => {{}}; let loaded; const addEventListener = (_, f) => {{ loaded = f; }};",
            dark_css = js(DARK_CSS),
            body = js(body),
            html = js(html),
            scheme = js(scheme),
            meta = meta.map_or("null".to_owned(), |m| format!("({{ content: {} }})", js(m))),
        );
        let change = later.map_or(String::new(), |bg| format!("body.bg = {}; observers[0].f([{{ type: 'attributes' }}]);", js(bg)));
        let out = std::process::Command::new("osascript").args(["-l", "JavaScript", "-e", &format!("{state}\n{script}\n{change}\nreport()")]).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    #[test]
    fn forced_dark_leaves_dark_pages_alone() {
        const NONE: &str = "rgba(0, 0, 0, 0)";
        let page = |body, html| dark_sheets(true, body, html, "normal", None, false, None);
        assert_eq!(page("rgb(255, 255, 255)", NONE), "dark,css");
        assert_eq!(page("rgb(13, 17, 23)", "rgb(255, 255, 255)"), "off,css", "the body's background decides first");
        assert_eq!(page(NONE, "rgb(13, 17, 23)"), "off,css");
        assert_eq!(page(NONE, NONE), "dark,css", "a page without a background is white");
        assert_eq!(page("color(srgb 0 0 0)", NONE), "dark,css", "an unreadable colour does not count");
        let scheme = |scheme, meta, prefers_dark| dark_sheets(true, NONE, NONE, scheme, meta, prefers_dark, None);
        assert_eq!(scheme("dark", None, false), "off,css");
        assert_eq!(scheme("light dark", None, true), "off,css");
        assert_eq!(scheme("light dark", None, false), "dark,css");
        assert_eq!(scheme("normal", Some("dark"), false), "off,css");
        assert_eq!(dark_sheets(true, "rgb(255, 255, 255)", NONE, "dark", None, false, None), "off,css", "the page's own dark scheme comes first");
        assert_eq!(dark_sheets(true, "rgb(255, 255, 255)", NONE, "light dark", None, true, None), "off,css");
        assert_eq!(dark_sheets(true, "rgb(255, 255, 255)", NONE, "light dark", None, false, None), "dark,css");
        assert_eq!(dark_sheets(true, "rgb(250, 250, 250)", NONE, "normal", None, false, Some("rgb(13, 17, 23)")), "off,css", "a theme switch");
        assert_eq!(dark_sheets(true, "rgb(13, 17, 23)", NONE, "normal", None, false, Some("rgb(255, 255, 255)")), "dark,css");
        assert_eq!(dark_sheets(false, "rgb(255, 255, 255)", NONE, "normal", None, false, None), "css");
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
