const data = document.documentElement.dataset;
data.tarnFrame = location.hostname;
chrome.runtime.sendMessage({ hello: 'frame' }).then(
  r => { data.tarnFrameReply = r && r.echo; },
  e => { data.tarnFrameReply = 'error: ' + e.message; },
);
