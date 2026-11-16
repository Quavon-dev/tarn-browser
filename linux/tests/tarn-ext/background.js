chrome.runtime.onMessage.addListener((message, sender, respond) => {
  respond({ echo: message.hello + ' via ' + (sender.tab ? 'tab' : 'page') });
});
