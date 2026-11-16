const root = document.documentElement;
root.dataset.tarnExt = typeof chrome.runtime.sendMessage;
chrome.storage.local.set({ seen: location.pathname })
  .then(() => chrome.storage.local.get('seen'))
  .then(v => { root.dataset.tarnStored = v.seen; });
chrome.runtime.sendMessage({ hello: 'content' }).then(
  r => { root.dataset.tarnReply = r && r.echo; },
  e => { root.dataset.tarnReply = 'error: ' + e.message; },
);
