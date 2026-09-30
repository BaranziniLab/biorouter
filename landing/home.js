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
