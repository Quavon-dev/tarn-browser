chrome.runtime.onMessage.addListener((message, sender, respond) => {
  chrome.action.setBadgeText({ text: 'ok' });
  respond({ echo: message.hello + ' via ' + (sender.tab ? 'tab' : 'page') });
});
