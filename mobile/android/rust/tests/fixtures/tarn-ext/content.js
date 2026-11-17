document.documentElement.dataset.tarnFixture = 'content';
chrome.runtime.sendMessage({ from: 'content', url: location.href }).then(reply => {
  document.documentElement.dataset.tarnFixtureReply = String(reply && reply.ok);
});
