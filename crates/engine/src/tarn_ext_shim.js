(function (config) {
    'use strict';
    const post = globalThis.__tarnExtPost;
    try { delete globalThis.__tarnExtPost; } catch (e) {}
    if (typeof post !== 'function' || Object.prototype.hasOwnProperty.call(globalThis, '__tarnExtReceive')) return;
    const pending = new Map();
    const listeners = { message: new Set(), changed: new Set(), clicked: new Set() };
    let next = 1, lastError;

    const send = (call, args) => new Promise((resolve, reject) => {
        const id = next++;
        pending.set(id, { resolve, reject });
        try {
            post(JSON.stringify({ id, call, args }));
        } catch (e) {
            pending.delete(id);
            reject(new Error('The message could not be sent.'));
        }
    });

    const api = call => (...args) => {
        const callback = typeof args[args.length - 1] === 'function' ? args.pop() : null;
        const promise = send(call, args);
        if (!callback) return promise;
        promise.then(value => callback(value), error => {
            lastError = { message: String((error && error.message) || error) };
            try { callback(); } finally { lastError = undefined; }
        });
    };

    const event = set => ({
        addListener: f => { if (typeof f === 'function') set.add(f); },
        removeListener: f => { set.delete(f); },
        hasListener: f => set.has(f),
    });

    const deliver = m => {
        let answered = false, kept = false;
        const respond = response => {
            if (answered) return;
            answered = true;
            post(JSON.stringify({ id: null, call: 'reply', args: [m.delivery, response === undefined ? null : response] }));
        };
        for (const f of [...listeners.message]) {
            let out;
            try { out = f(m.message, m.sender, respond); } catch (e) { continue; }
            if (out === true) kept = true;
            else if (out && typeof out.then === 'function') {
                kept = true;
                out.then(respond, () => respond(undefined));
            }
        }
        if (!kept) respond(undefined);
    };

    const fire = (set, ...args) => {
        for (const f of [...set]) {
            try { f(...args); } catch (e) {}
        }
    };

    const receive = json => {
        let m;
        try { m = JSON.parse(json); } catch (e) { return; }
        if (!m || typeof m !== 'object') return;
        if (m.event === 'runtime.onMessage') return deliver(m);
        if (m.event === 'storage.onChanged') return fire(listeners.changed, m.changes, 'local');
        if (m.event === 'action.onClicked') return fire(listeners.clicked, m.tab);
        const p = pending.get(m.id);
        if (!p) return;
        pending.delete(m.id);
        if (typeof m.error === 'string') p.reject(new Error(m.error));
        else p.resolve(m.result === null ? undefined : m.result);
    };
    Object.defineProperty(globalThis, '__tarnExtReceive', { value: receive });

    const getMessage = (name, substitutions) => {
        const text = config.messages[String(name).toLowerCase()];
        if (typeof text !== 'string') return '';
        const list = substitutions == null ? [] : Array.isArray(substitutions) ? substitutions.map(String) : [String(substitutions)];
        return text.replace(/\$(\$|[1-9])/g, (_, n) => n === '$' ? '$' : (list[n - 1] ?? ''));
    };

    const runtime = {
        id: config.id,
        getURL: path => config.base + String(path).replace(/^\/+/, ''),
        getManifest: () => JSON.parse(config.manifest),
        sendMessage: api('runtime.sendMessage'),
        onMessage: event(listeners.message),
        get lastError() { return lastError; },
    };
    const chrome = { runtime, i18n: { getMessage, getUILanguage: () => config.locale } };
    if (config.storage) {
        const local = {};
        for (const name of ['get', 'set', 'remove', 'clear', 'getBytesInUse']) local[name] = api('storage.' + name);
        chrome.storage = { local, onChanged: event(listeners.changed) };
    }
    if (config.page) {
        chrome.tabs = { query: api('tabs.query'), create: api('tabs.create'), sendMessage: api('tabs.sendMessage') };
        chrome.action = {
            setBadgeText: api('action.setBadgeText'),
            setBadgeBackgroundColor: api('action.setBadgeBackgroundColor'),
            setTitle: api('action.setTitle'),
            onClicked: event(listeners.clicked),
        };
        globalThis.addEventListener('load', () => post(JSON.stringify({ id: null, call: 'ready', args: [] })), { once: true });
    }
    globalThis.chrome = chrome;
    globalThis.browser = chrome;
})
