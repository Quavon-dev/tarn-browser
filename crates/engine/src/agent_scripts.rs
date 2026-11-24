//! What an AI agent does on a page (S31), as async function bodies for the engine's isolated
//! world, the same on every engine. Element numbers live only in that world and only for the
//! latest snapshot. Values of password, payment and one-time-code fields are never read out,
//! also once a page switches such a field to plain text, password fields are never typed into, and a click, submit or Enter while such a field holds a
//! value answers `confirm` until the user allows it. Fields inside cross-origin frames (hosted
//! card fields) are out of reach of the isolated world and so not seen.

use base64::Engine as _;

use crate::handover::js_string;

pub const MAX_ELEMENTS: usize = 400;

/// Whether a page-started main-frame navigation of an agent's tab may go to a URL; a refused one
/// is cancelled before any request and reported as `Event::AgentHeld`.
pub type Gate = std::rc::Rc<dyn Fn(&url::Url) -> bool>;

#[derive(Clone, Debug, PartialEq)]
pub enum Act<'a> {
    Click,
    Type { text: &'a str, submit: bool },
    Select(&'a str),
    Key(&'a str),
    Scroll { down: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub enum AgentScript<'a> {
    Snapshot,
    /// `target` 0 is the focused element, or the page for `Scroll`.
    Act {
        target: u32,
        act: Act<'a>,
        confirmed: bool,
    },
    /// Answers "1" once the document has loaded and, when given, shows `text`.
    Wait {
        text: Option<&'a str>,
    },
    /// Answers "1" while a password, payment or one-time-code field holds a value, else "0".
    Sensitive,
}

const COMMON: &str = r"const st = window.__tarnAgent || (window.__tarnAgent = { next: 1, map: new Map() });
st.secret = st.secret || new WeakSet();
st.password = st.password || new WeakSet();
const sensitive = el => {
    if (!el || !/^(input|select)$/.test(el.localName)) return false;
    if (el.type === 'password') st.password.add(el);
    if (st.secret.has(el)) return true;
    const auto = el.getAttribute('autocomplete') || '', named = (el.getAttribute('name') || '') + ' ' + el.id;
    const password = el.type === 'password' || /(^|\s)(current|new)-password/i.test(auto) || /pass|pwd/i.test(named);
    const hit = password || /(^|\s)(cc-|one-time-code)/i.test(auto) || /cvc|cvv|otp/i.test(named);
    if (hit) st.secret.add(el);
    if (password) st.password.add(el);
    return hit;
};
const filledSensitive = () => {
    let seen = false;
    const walk = root => {
        for (const el of root.querySelectorAll('input,select')) if (sensitive(el) && el.value) seen = true;
        for (const host of root.querySelectorAll('*')) if (host.shadowRoot) walk(host.shadowRoot);
        for (const frame of root.querySelectorAll('iframe,frame')) { try { if (frame.contentDocument) walk(frame.contentDocument); } catch (e) {} }
    };
    walk(document);
    return seen;
};";

const SNAPSHOT: &str = r"st.map = new Map();
const SEL = 'a[href],button,input:not([type=hidden]),select,textarea,summary,[contenteditable=true],[contenteditable=\x22\x22],[role=button],[role=link],[role=checkbox],[role=radio],[role=tab],[role=menuitem],[role=option],[role=switch],[role=combobox],[role=textbox],[role=searchbox],[onclick],[tabindex]:not([tabindex=\x22-1\x22])';
const found = [];
const walk = root => { for (const el of root.querySelectorAll(SEL)) found.push(el); for (const host of root.querySelectorAll('*')) if (host.shadowRoot) walk(host.shadowRoot); };
walk(document);
const onScreen = el => { const r = el.getBoundingClientRect(); return r.bottom >= 0 && r.top <= innerHeight && r.right >= 0 && r.left <= innerWidth; };
found.sort((a, b) => onScreen(b) - onScreen(a));
const clean = (s, n) => (s || '').replace(/[\u0000-\u001f\u007f\u200b-\u200f\u2028\u2029\u202a-\u202e\u2066-\u2069]/g, ' ').replace(/\s+/g, ' ').trim().slice(0, n);
const roles = { a: 'link', button: 'button', select: 'combobox', textarea: 'textbox', summary: 'button' };
const inputRoles = { checkbox: 'checkbox', radio: 'radio', submit: 'button', button: 'button', reset: 'button', image: 'button', range: 'slider', file: 'file' };
const roleOf = el => el.getAttribute('role') || roles[el.localName] || (el.localName === 'input' ? (inputRoles[el.type] || 'textbox') : (el.isContentEditable ? 'textbox' : 'clickable'));
const nameOf = el => clean(el.getAttribute('aria-label') || (el.labels && el.labels[0] && el.labels[0].innerText) || el.getAttribute('placeholder') || el.getAttribute('title') || el.getAttribute('alt') || (el.localName === 'input' && /^(submit|button|reset)$/.test(el.type) ? el.value : '') || el.innerText || el.textContent, 120);
const lines = ['Address: ' + location.href, 'Title: ' + clean(document.title, 200), 'Scrolled: ' + Math.round(scrollY) + ' of ' + Math.max(0, document.documentElement.scrollHeight - innerHeight), 'Elements:'];
let n = 0;
for (const el of found) {
    if (n >= MAX) { lines.push('[more elements not listed; scroll and snapshot again]'); break; }
    const r = el.getBoundingClientRect();
    if (r.width <= 0 || r.height <= 0 || (el.checkVisibility && !el.checkVisibility({ visibilityProperty: true }))) continue;
    const id = st.next++;
    st.map.set(id, new WeakRef(el));
    n++;
    const role = roleOf(el);
    let line = '[' + id + '] ' + role + ' \x22' + nameOf(el) + '\x22';
    if (sensitive(el)) line += el.value ? ' value=(hidden)' : ' empty';
    else if (role === 'checkbox' || role === 'radio') line += el.checked || el.getAttribute('aria-checked') === 'true' ? ' checked' : ' unchecked';
    else if (el.localName === 'select') line += ' value=\x22' + clean(el.selectedOptions[0] ? el.selectedOptions[0].label : '', 80) + '\x22 options=' + [...el.options].slice(0, 20).map(o => clean(o.label, 40)).join('|');
    else if (role === 'textbox' && el.localName !== 'div') line += ' value=\x22' + clean(el.value, 200) + '\x22';
    if (el.localName === 'a') line += ' -> ' + clean(el.href, 200);
    if (el.disabled || el.getAttribute('aria-disabled') === 'true') line += ' disabled';
    if (r.bottom < 0 || r.top > innerHeight || r.right < 0 || r.left > innerWidth) line += ' offscreen';
    lines.push(line);
}
if (filledSensitive()) lines.push('Note: a password or payment field holds a value; Tarn asks the user before any click, submit or Enter here.');
return lines.join('\n');";

const ACT: &str = r"const el = target === 0 ? (document.activeElement || document.body) : (st.map.get(target) && st.map.get(target).deref());
if (target !== 0 && (!el || !el.isConnected)) return 'stale';
const secretFilled = filledSensitive();
const guard = () => !confirmed && secretFilled;
const flash = el => { if (!el.style) return; const before = el.style.outline; el.style.outline = '3px solid #ff9500'; setTimeout(() => { el.style.outline = before; }, 700); };
const center = el => { const r = el.getBoundingClientRect(); return { clientX: r.left + r.width / 2, clientY: r.top + r.height / 2 }; };
const fire = (el, type, init) => el.dispatchEvent(new (type.startsWith('pointer') ? PointerEvent : type.startsWith('key') ? KeyboardEvent : MouseEvent)(type, Object.assign({ bubbles: true, cancelable: true, composed: true, view: window }, init)));
const setValue = (el, value) => {
    const proto = el.localName === 'textarea' ? HTMLTextAreaElement.prototype : el.localName === 'select' ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, value);
    el.dispatchEvent(new Event('input', { bubbles: true, composed: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
};
const submit = el => { const form = el.form || el.closest('form'); if (!form) return false; if (form.requestSubmit) form.requestSubmit(); else form.submit(); return true; };
const editable = el => el.isContentEditable || el.localName === 'textarea' || (el.localName === 'input' && !/^(checkbox|radio|submit|button|reset|image|file|range|color|hidden)$/.test(el.type));
const focusables = () => [...document.querySelectorAll('a[href],button,input:not([type=hidden]),select,textarea,[tabindex]:not([tabindex=\x22-1\x22]),[contenteditable=true]')].filter(e => !e.disabled && e.getBoundingClientRect().width > 0);
switch (act.kind) {
    case 'click': {
        if (guard()) return 'confirm';
        el.scrollIntoView({ block: 'center', inline: 'center' });
        flash(el);
        const at = center(el);
        fire(el, 'pointerdown', Object.assign({ isPrimary: true, button: 0, buttons: 1 }, at));
        fire(el, 'mousedown', Object.assign({ button: 0, buttons: 1 }, at));
        if (el.focus) el.focus();
        fire(el, 'pointerup', Object.assign({ isPrimary: true, button: 0 }, at));
        fire(el, 'mouseup', Object.assign({ button: 0 }, at));
        el.click();
        return 'ok';
    }
    case 'type': {
        if (sensitive(el) && st.password.has(el)) return 'refused:password';
        if (!editable(el) || el.readOnly || el.disabled) return 'error:not a text field';
        if (act.submit && guard()) return 'confirm';
        el.scrollIntoView({ block: 'center' });
        flash(el);
        el.focus();
        if (el.isContentEditable) { document.execCommand('selectAll'); document.execCommand('insertText', false, act.text); }
        else setValue(el, act.text);
        if (act.submit && !confirmed && filledSensitive()) return 'confirm';
        if (act.submit && !submit(el)) { fire(el, 'keydown', { key: 'Enter', code: 'Enter' }); fire(el, 'keyup', { key: 'Enter', code: 'Enter' }); }
        return 'ok';
    }
    case 'select': {
        if (el.localName !== 'select') return 'error:not a select element';
        if (guard()) return 'confirm';
        const want = act.value.trim().toLowerCase();
        const option = [...el.options].find(o => o.value === act.value) || [...el.options].find(o => o.label.trim().toLowerCase() === want);
        if (!option) return 'error:no such option';
        flash(el);
        setValue(el, option.value);
        return 'ok';
    }
    case 'key': {
        const key = act.key;
        if (key === 'Enter' && guard()) return 'confirm';
        if (sensitive(el) && st.password.has(el) && key === 'Backspace') return 'refused:password';
        const init = { key, code: key };
        const go = fire(el, 'keydown', init);
        if (go) {
            if (key === 'Enter' && el.localName === 'input') submit(el);
            else if (key === 'Enter' && el.localName !== 'textarea' && !el.isContentEditable && el.click) el.click();
            else if (key === 'Tab') { const all = focusables(); const next = all[(all.indexOf(el) + 1) % Math.max(all.length, 1)]; if (next) next.focus(); }
            else if (key === 'Backspace' && editable(el) && !el.isContentEditable) setValue(el, el.value.slice(0, -1));
            else if (key === 'PageDown' || key === 'PageUp') scrollBy(0, (key === 'PageDown' ? 1 : -1) * innerHeight * 0.9);
        }
        fire(el, 'keyup', init);
        return 'ok';
    }
    case 'scroll': {
        if (target === 0) scrollBy(0, (act.down ? 1 : -1) * innerHeight * 0.8);
        else { el.scrollIntoView({ block: 'center' }); flash(el); }
        return 'ok:' + Math.round(scrollY) + ' of ' + Math.max(0, document.documentElement.scrollHeight - innerHeight);
    }
}
return 'error:unknown action';";

impl AgentScript<'_> {
    pub fn source(&self) -> String {
        match self {
            AgentScript::Snapshot => format!("const MAX = {MAX_ELEMENTS};\n{COMMON}\n{SNAPSHOT}"),
            AgentScript::Act { target, act, confirmed } => {
                let act = match act {
                    Act::Click => "{ kind: 'click' }".to_string(),
                    Act::Type { text, submit } => format!("{{ kind: 'type', text: {}, submit: {submit} }}", js_string(text)),
                    Act::Select(value) => format!("{{ kind: 'select', value: {} }}", js_string(value)),
                    Act::Key(key) => format!("{{ kind: 'key', key: {} }}", js_string(key)),
                    Act::Scroll { down } => format!("{{ kind: 'scroll', down: {down} }}"),
                };
                format!("const target = {target}, confirmed = {confirmed}, act = {act};\n{COMMON}\n{ACT}")
            }
            AgentScript::Sensitive => format!("{COMMON}\nreturn filledSensitive() ? '1' : '0';"),
            AgentScript::Wait { text } => format!(
                "const want = {};\nreturn document.readyState === 'complete' && (!want || (document.body && document.body.innerText.includes(want))) ? '1' : '';",
                text.map_or("null".to_string(), js_string)
            ),
        }
    }
}

pub fn base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_text_stays_a_string_literal() {
        let hostile = "'; alert(1); //\"\n</script>\u{2028}";
        let typed = AgentScript::Act { target: 3, act: Act::Type { text: hostile, submit: false }, confirmed: false }.source();
        assert!(typed.starts_with("const target = 3, confirmed = false, act = { kind: 'type', text: \""));
        assert!(!typed.contains("'; alert(1)"));
        let wait = AgentScript::Wait { text: Some(hostile) }.source();
        assert!(!wait.contains("'; alert(1)"));
        assert!(AgentScript::Wait { text: None }.source().starts_with("const want = null;"));
    }

    #[test]
    fn the_snapshot_is_capped() {
        assert!(AgentScript::Snapshot.source().starts_with(&format!("const MAX = {MAX_ELEMENTS};")));
    }
}
