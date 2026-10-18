/* Introduction page: name the visitor's platform on the hero download button. */
(function () {
  var btn = document.getElementById('hero-download');
  if (!btn) return;
  var label = btn.querySelector('span');
  var p = ((navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || '').toLowerCase();
  var ua = navigator.userAgent.toLowerCase();
  var mobile = /android|iphone|ipad|ipod/.test(ua) || (p === 'macintel' && navigator.maxTouchPoints > 1);
  if (mobile) return;
  if (/mac/.test(p) || /macintosh/.test(ua)) label.textContent = 'Download for macOS';
  else if (/win/.test(p) || /windows/.test(ua)) label.textContent = 'Download for Windows';
  else if (/linux/.test(p) || /linux/.test(ua)) label.textContent = 'Download for Linux';
})();

/* The hero's "New" line names the newest entry in the About page's Updates
   list (its first .news-list, newest first) and links to that list. The HTML
   carries a current fallback; this only refreshes it, so a new row in
   about.html reaches the hero with no edit here. */
(function () {
  var slot = document.querySelector('[data-latest-update]');
  if (!slot || !window.fetch || !window.DOMParser) return;
  fetch('about.html')
    .then(function (r) { return r.ok ? r.text() : null; })
    .then(function (html) {
      if (!html) return;
      var doc = new DOMParser().parseFromString(html, 'text/html');
      var h = doc.querySelector('.news-list:not(.news-list-earlier) .news-row h3');
      var t = h && h.textContent.replace(/\s+/g, ' ').trim();
      if (t) slot.textContent = t;
    })
    .catch(function () {});
})();
