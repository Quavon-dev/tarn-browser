//! Storage handover on an engine switch: the top-level origin's
//! `localStorage` and `sessionStorage`, read in an isolated world of the old view and written
//! by a one-shot, start-of-document script in an isolated world of the new one.

use std::fmt::Write;

pub const MAX_BYTES: usize = 5 * 1024 * 1024;

/// One origin's storage as JSON. Values stay in memory: never logged, stored or put in an `Event`.
pub struct Storage {
    origin: String,
    json: String,
}

impl std::fmt::Debug for Storage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Storage").field("origin", &self.origin).field("bytes", &self.json.len()).finish()
    }
}

impl Storage {
    pub(crate) fn new(origin: &str, json: String) -> Option<Self> {
        (!json.is_empty() && json.len() <= MAX_BYTES).then(|| Storage { origin: origin.to_owned(), json })
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }
}

/// An expression that yields the storage of the top-level document at `origin` as JSON, or an
/// empty string for a frame, another origin, empty storage or more than `MAX_BYTES`.
pub(crate) fn read_script(origin: &str) -> String {
    format!(
        "(() => {{ try {{ if (window.top !== window || location.origin !== {origin}) return ''; \
        const r = {{ l: [], s: [] }}; let n = 0; \
        for (const [k, area] of [['l', localStorage], ['s', sessionStorage]]) for (let i = 0; i < area.length; i++) {{ \
        const key = area.key(i), value = area.getItem(key); n += key.length + value.length; \
        if (n > {MAX_BYTES}) return ''; r[k].push([key, value]); }} \
        if (!r.l.length && !r.s.length) return ''; const t = JSON.stringify(r); return t.length > {MAX_BYTES} ? '' : t; \
        }} catch {{ return ''; }} }})()",
        origin = js_string(origin)
    )
}

/// Writes `storage` into the top-level document of its origin, into each area only while that
/// area is empty; anything else in the data is ignored.
pub(crate) fn write_script(storage: &Storage) -> String {
    format!(
        "(() => {{ if (window.top !== window || location.origin !== {origin}) return; let d; \
        try {{ d = JSON.parse({json}); }} catch {{ return; }} \
        for (const [k, area] of [['l', () => localStorage], ['s', () => sessionStorage]]) try {{ \
        const target = area(); if (target.length || !Array.isArray(d[k])) continue; \
        for (const e of d[k]) if (Array.isArray(e) && typeof e[0] === 'string' && typeof e[1] === 'string') target.setItem(e[0], e[1]); \
        }} catch {{}} }})();",
        origin = js_string(&storage.origin),
        json = js_string(&storage.json)
    )
}

/// A double-quoted literal that is valid JSON and JavaScript and safe inside HTML: no raw
/// quote, backslash, control character, `<`, `>`, `&`, U+2028 or U+2029.
pub fn js_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '<' | '>' | '&' | '\'' | '\u{2028}' | '\u{2029}' | '\0'..='\u{1f}' | '\u{7f}' => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const NASTY: &str = "</script><script>globalThis.pwned=1</script>\"'\\\u{2028}\u{2029}\n\0&<!--";

    #[test]
    fn literals_carry_nothing_that_ends_a_string_or_a_script() {
        let lit = js_string(NASTY);
        let inner = &lit[1..lit.len() - 1];
        assert!(!inner.contains(['<', '>', '&', '\'', '\n', '\0', '\u{2028}', '\u{2029}']));
        assert!(!inner.replace("\\\\", "").replace("\\\"", "").contains('"'));
        assert_eq!(js_string("a\u{e9}\u{1f600}"), "\"a\u{e9}\u{1f600}\"");
    }

    #[test]
    fn storage_is_capped_and_never_printed() {
        assert!(Storage::new("https://a.test", String::new()).is_none());
        assert!(Storage::new("https://a.test", "x".repeat(MAX_BYTES + 1)).is_none());
        let s = Storage::new("https://a.test", "{\"l\":[[\"k\",\"secret\"]],\"s\":[]}".into()).unwrap();
        assert!(!format!("{s:?}").contains("secret"));
        assert!(write_script(&s).contains(&js_string(&s.json)));
    }

    #[test]
    #[ignore = "needs node: cargo test -p engine handover -- --ignored"]
    fn storage_survives_the_round_trip_and_runs_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let check = dir.path().join("check.mjs");
        std::fs::write(&check, NODE_CHECK.replace("NASTY", &js_string(NASTY))).unwrap();
        let node = |args: &[&str]| std::process::Command::new("node").arg(&check).args(args).output().expect("node");
        let read = node(&["read", &read_script("https://a.test")]);
        let storage = Storage::new("https://a.test", String::from_utf8(read.stdout).unwrap()).expect("read");
        let output = node(&["write", &write_script(&storage), &read_script("https://a.test")]);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success() && stdout.trim_end().ends_with("ALL OK"), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
    }

    const NODE_CHECK: &str = r#"import vm from 'node:vm';
const area = init => { const m = new Map(init); return { get length() { return m.size; }, key: i => [...m.keys()][i] ?? null,
  getItem: k => m.has(k) ? m.get(k) : null, setItem: (k, v) => m.set(k, String(v)), dump: () => [...m] }; };
const page = (origin, top, l = [], s = []) => { const g = { location: { origin }, localStorage: area(l), sessionStorage: area(s) };
  g.window = g; g.top = top ? g : {}; return vm.createContext(g); };
const nasty = NASTY, local = [['k', nasty], [nasty, 'v']], session = [['s', nasty]];
const [mode, script, read] = process.argv.slice(2);
if (mode === 'read') { process.stdout.write(vm.runInContext(script, page('https://a.test', true, local, session))); process.exit(0); }
const run = (origin, top, l) => { const ctx = page(origin, top, l); vm.runInContext(script, ctx); return ctx; };
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
const ok = run('https://a.test', true);
if (!same(ok.localStorage.dump(), local) || !same(ok.sessionStorage.dump(), session)) throw new Error('changed');
if (ok.pwned || globalThis.pwned) throw new Error('ran');
if (run('https://b.test', true).localStorage.length) throw new Error('other origin');
if (run('https://a.test', false).localStorage.length) throw new Error('frame');
if (!same(run('https://a.test', true, [['own', '1']]).localStorage.dump(), [['own', '1']])) throw new Error('overwrote');
if (vm.runInContext(read, page('https://a.test', false, local))) throw new Error('read a frame');
if (vm.runInContext(read, page('https://a.test', true))) throw new Error('read empty storage');
console.log('ALL OK');
"#;
}
