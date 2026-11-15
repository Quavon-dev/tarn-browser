(() => {
  'use strict';
  const host = location.hostname;
  const sites = ['youtube.com', 'youtube-nocookie.com', 'youtubekids.com'];
  let matched = false;
  for (let i = 0; i < sites.length; i++) {
    if (host === sites[i] || (host.length > sites[i].length && host.endsWith('.' + sites[i]))) matched = true;
  }
  if (!matched || host === 'accounts.youtube.com') return;

  const KEYS = __TARN_KEYS__;
  // Captured before any page script runs; the hooks below use nothing else.
  const apply = Reflect.apply;
  const defineProperty = Object.defineProperty;
  const descriptor = Object.getOwnPropertyDescriptor;
  const hasOwn = Object.prototype.hasOwnProperty;
  const isArray = Array.isArray;
  const parseJson = JSON.parse;
  const then = Promise.prototype.then;
  const responseJson = Response.prototype.json;
  const xhrResponse = descriptor(XMLHttpRequest.prototype, 'response');
  const xhrResponseType = descriptor(XMLHttpRequest.prototype, 'responseType').get;

  const own = (o, k) => apply(hasOwn, o, [k]);
  const isObject = v => typeof v === 'object' && v !== null;
  const strip = o => {
    if (!isObject(o)) return;
    for (let i = 0; i < KEYS.length; i++) if (own(o, KEYS[i])) delete o[KEYS[i]];
  };
  const isAd = entry => {
    try {
      return !!entry.command.reelWatchEndpoint.adClientParams.isAd;
    } catch (e) {
      return false;
    }
  };
  const prune = v => {
    try {
      if (!isObject(v)) return;
      if (isArray(v)) {
        for (let i = 0; i < v.length && i < 16; i++) {
          strip(v[i]);
          if (isObject(v[i]) && own(v[i], 'playerResponse')) strip(v[i].playerResponse);
        }
        return;
      }
      strip(v);
      if (own(v, 'playerResponse')) strip(v.playerResponse);
      if (own(v, 'entries') && isArray(v.entries)) {
        const kept = [];
        for (let i = 0; i < v.entries.length; i++) if (!isAd(v.entries[i])) kept[kept.length] = v.entries[i];
        v.entries = kept;
      }
    } catch (e) {}
  };

  for (const name of ['ytInitialPlayerResponse', 'playerResponse']) {
    let value;
    try {
      defineProperty(window, name, {
        configurable: true,
        get: () => value,
        set: v => {
          prune(v);
          value = v;
        },
      });
    } catch (e) {}
  }

  JSON.parse = function parse(text, reviver) {
    const value = apply(parseJson, this, arguments);
    prune(value);
    return value;
  };
  Response.prototype.json = function json() {
    return apply(then, apply(responseJson, this, arguments), [value => {
      prune(value);
      return value;
    }]);
  };
  defineProperty(XMLHttpRequest.prototype, 'response', {
    configurable: xhrResponse.configurable,
    enumerable: xhrResponse.enumerable,
    get: function response() {
      const value = apply(xhrResponse.get, this, []);
      if (apply(xhrResponseType, this, []) === 'json') prune(value);
      return value;
    },
  });
})();
