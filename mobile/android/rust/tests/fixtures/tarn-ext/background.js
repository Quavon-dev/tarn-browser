chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  chrome.storage.local.set({ last: message.url }).then(() => sendResponse({ ok: true }));
  return true;
});
