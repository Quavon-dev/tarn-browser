chrome.storage.local.get('last').then(items => {
  document.getElementById('out').textContent = items.last || 'empty';
});
