chrome.storage.local.get('seen').then(v => { document.getElementById('seen').textContent = 'seen ' + v.seen; });
