/* Download page: offer the installer this computer needs, and refresh every
   row from the latest GitHub release. The HTML carries complete fallbacks,
   so the page works without this script and without GitHub. site.js fetches
   the release once and hands it over in the `br:version` event. */
(function () {
  'use strict';

  // One asset per row. GUI packages start "biorouter_" or "Biorouter-<digit>";
  // the command line only packages start "biorouter-cli", so the two sets never
  // cross-match.
  var ROW_MATCHERS = {
    'mac-arm': /arm64\.dmg$/i,
    'mac-intel': /(x64|x86_64)\.dmg$/i,
    'windows-setup': /Biorouter-Setup-.*\.exe$/i,
    'windows': /win32-x64.*\.zip$/i,
    'linux-deb': /^biorouter_[0-9].*amd64\.deb$/i,
    'linux-rpm': /^biorouter-[0-9].*x86_64\.rpm$/i,
    'linux-cli-deb': /^biorouter-cli_.*amd64\.deb$/i,
    'linux-cli-rpm': /^biorouter-cli-.*x86_64\.rpm$/i
  };

  var get = document.getElementById('get');
  if (!get) return;
  var el = {
    primary: document.getElementById('get-primary'),
    second: document.getElementById('get-second'),
    all: document.getElementById('get-all'),
    file: document.getElementById('get-file'),
    alt: document.getElementById('get-alt'),
    date: document.getElementById('rel-date')
  };

  // ── Rows: read the static fallbacks, then keep them as the source of truth.
  // The static page carries names and links but no sizes: the release script
  // rewrites the version in this file and nothing else, so a size written here
  // would describe an older file. Sizes appear once the release confirms them.
  var table = document.querySelector('.dl-table');
  var rows = {};
  Array.prototype.forEach.call(document.querySelectorAll('tr[data-row]'), function (tr) {
    var name = tr.querySelector('.file-name');
    var link = tr.querySelector('.dl-link');
    if (!name || !link) return;
    rows[tr.getAttribute('data-row')] = { tr: tr, name: name.textContent.trim(), url: link.href, size: 0 };
  });

  function available(row) {
    var r = rows[row];
    return r && !r.tr.hidden ? r : null;
  }

  function mb(bytes) { return bytes ? Math.round(bytes / 1048576) + '\u00a0MB' : ''; }

  function applyAsset(row, a) {
    var r = rows[row];
    r.name = a.name; r.url = a.url; if (a.size) r.size = a.size;
    r.tr.querySelector('.file-name').textContent = a.name;
    r.tr.querySelector('.dl-link').href = a.url;
    var size = r.tr.querySelector('.c-size');
    if (size) size.textContent = mb(a.size);
    var arg = r.tr.querySelector('[data-file]');
    if (arg) arg.textContent = './' + a.name;
    var copy = r.tr.querySelector('.copy-btn');
    if (copy) copy.setAttribute('aria-label', 'Copy the install command for ' + a.name);
  }

  // ── What is this computer?
  function withTimeout(p, ms) {
    return Promise.race([p, new Promise(function (res) { setTimeout(function () { res(null); }, ms); })]);
  }

  // The GPU names the Mac's processor. Intel-era Macs report Intel, AMD or
  // NVIDIA; Apple silicon reports an Apple M chip. Safari reports "Apple GPU"
  // on every Mac, so there only ASTC texture support (Apple GPUs alone have it)
  // settles the question, and anything else stays unknown.
  function archFromWebGL() {
    try {
      var canvas = document.createElement('canvas');
      var gl = canvas.getContext('webgl') || canvas.getContext('experimental-webgl');
      if (!gl) return null;
      var dbg = gl.getExtension('WEBGL_debug_renderer_info');
      var renderer = String((dbg && gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL)) || gl.getParameter(gl.RENDERER) || '').toLowerCase();
      var exts = gl.getSupportedExtensions() || [];
      var lose = gl.getExtension('WEBGL_lose_context');
      if (lose) lose.loseContext();
      if (/intel|amd|radeon|nvidia|geforce/.test(renderer)) return 'intel';
      if (/apple m\d/.test(renderer)) return 'arm';
      if (/apple/.test(renderer)) return exts.indexOf('WEBGL_compressed_texture_astc') !== -1 ? 'arm' : null;
    } catch (e) { /* no WebGL: unknown */ }
    return null;
  }

  // Chromium states the architecture directly, as a high-entropy hint. Its
  // user agent string does not: a reduced UA says "Intel" on every Mac and
  // "Linux x86_64" on every Linux machine.
  function archHint() {
    var uad = navigator.userAgentData;
    if (!uad || !uad.getHighEntropyValues) return Promise.resolve(null);
    var hint;
    try { hint = uad.getHighEntropyValues(['architecture']); } catch (e) { return Promise.resolve(null); }
    return withTimeout(Promise.resolve(hint).then(function (h) {
      var a = String((h && h.architecture) || '').toLowerCase();
      if (a === 'arm') return 'arm';
      if (a === 'x86' || a === 'x86_64' || a === 'x64') return 'intel';
      return null;
    }), 800).then(null, function () { return null; });
  }

  function macArch() {
    return archHint().then(function (a) { return a || archFromWebGL(); });
  }

  function detect() {
    var nav = navigator;
    var uad = nav.userAgentData;
    var ua = nav.userAgent || '';
    var plat = String((uad && uad.platform) || nav.platform || '');
    // Phones and tablets first: Android says "Linux", and iPadOS in desktop
    // mode says "MacIntel", so neither may reach the desktop branches below.
    var mobile = (uad && uad.mobile === true) ||
      /android|iphone|ipad|ipod|mobile/i.test(ua) ||
      /^(android|ios|ipados)$/i.test(plat) ||
      (/mac/i.test(plat) && (nav.maxTouchPoints || 0) > 1);
    if (mobile) return Promise.resolve({ os: 'mobile' });
    if (/mac/i.test(plat) || /macintosh/i.test(ua)) {
      return macArch().then(function (arch) { return { os: 'mac', arch: arch }; });
    }
    if (/win/i.test(plat) || /windows/i.test(ua)) return Promise.resolve({ os: 'windows' });
    // ChromeOS says "Linux" in places but installs none of these packages
    // itself, so it gets the plain list.
    if (/\bcros\b|chrome ?os/i.test(ua + ' ' + plat)) return Promise.resolve({ os: null });
    // "X11" alone is not Linux: the BSDs send it too, and there is no build for them.
    if (!/bsd|sunos/i.test(ua + ' ' + plat) && (/linux/i.test(plat) || /linux/i.test(ua))) {
      var armInUa = /aarch64|arm64|armv\d/i.test(ua + ' ' + plat);
      return archHint().then(function (a) {
        return { os: armInUa || a === 'arm' ? 'linux-arm' : 'linux' };
      });
    }
    return Promise.resolve({ os: null });
  }

  // ── Drawing the offer
  function node(tag, cls, text) {
    var n = document.createElement(tag);
    if (cls) n.className = cls;
    if (text != null) n.textContent = text;
    return n;
  }

  function setButton(btn, label, row) {
    btn.querySelector('span').textContent = label;
    btn.href = row ? row.url : '#platforms';
    btn.hidden = false;
  }

  // "Apple silicon · Biorouter-1.91.2-arm64.dmg · 222 MB"
  function fileLine(variant, row) {
    el.file.textContent = '';
    var parts = [node('span', 'part', variant), node('code', 'part', row.name)];
    if (row.size) parts.push(node('span', 'part', mb(row.size)));
    parts.forEach(function (p, i) {
      if (i) el.file.appendChild(node('span', 'sep', ' · '));
      el.file.appendChild(p);
    });
  }

  function altLine(label, row) {
    el.alt.textContent = label + ' ';
    var a = node('a', null, null);
    a.href = row.url;
    a.appendChild(node('code', null, row.name));
    el.alt.appendChild(a);
    el.alt.hidden = false;
  }

  function markDetected(row) {
    Object.keys(rows).forEach(function (k) {
      var r = rows[k];
      var on = r === row;
      r.tr.classList.toggle('is-detected', on);
      // The rail is visual; a screen reader hears it in the link's name.
      var link = r.tr.querySelector('.dl-link');
      if (on) link.setAttribute('aria-label', r.name + ', for this computer');
      else link.removeAttribute('aria-label');
    });
  }

  var state = null;

  function render() {
    if (!state) return;
    get.classList.remove('is-note');
    el.second.hidden = true;
    el.alt.hidden = true;
    el.all.hidden = false;
    var main = null;

    if (state.os === 'mac' && state.arch) {
      var arm = state.arch === 'arm';
      main = available(arm ? 'mac-arm' : 'mac-intel');
      var other = available(arm ? 'mac-intel' : 'mac-arm');
      if (main) {
        setButton(el.primary, 'Download for macOS', main);
        fileLine(arm ? 'Apple silicon' : 'Intel', main);
        if (other) altLine(arm ? 'For an Intel Mac:' : 'For a Mac with Apple silicon:', other);
      }
    } else if (state.os === 'mac') {
      // The chip is unknown: offer both, Apple silicon first.
      var a = available('mac-arm'), b = available('mac-intel');
      if (a && b) {
        setButton(el.primary, 'macOS, Apple silicon', a);
        setButton(el.second, 'macOS, Intel', b);
        el.file.textContent = 'Apple silicon means an M1 chip or later. About This Mac, in the Apple menu, shows which chip your Mac has.';
        main = null;
      }
    } else if (state.os === 'windows') {
      main = available('windows-setup') || available('windows');
      if (main) {
        setButton(el.primary, 'Download for Windows', main);
        var isSetup = main === rows['windows-setup'];
        fileLine(isSetup ? 'Installer' : 'Archive', main);
        if (isSetup && available('windows')) altLine('Or the archive:', rows.windows);
      }
    } else if (state.os === 'linux') {
      main = available('linux-deb');
      if (main) {
        setButton(el.primary, 'Download for Linux', main);
        fileLine('Debian, Ubuntu', main);
        if (available('linux-rpm')) altLine('For Fedora or RHEL:', rows['linux-rpm']);
      }
    }

    var offered = !!main || !el.second.hidden;
    if (!offered) {
      el.primary.hidden = true;
      get.classList.add('is-note');
      if (state.os === 'mobile') {
        el.file.textContent = 'Biorouter runs on macOS, Windows and Linux computers. Open this page on a computer to download it, or choose a file below.';
      } else if (state.os === 'linux-arm') {
        el.file.textContent = 'The Linux packages are built for x86_64 computers, and this one reports an ARM processor.';
      } else {
        el.file.textContent = 'Biorouter runs on macOS, Windows and Linux. Choose the file for your computer below.';
      }
    }
    markDetected(main);
    get.classList.add('ready');
  }

  // ── The latest release refreshes names, links and sizes.
  document.addEventListener('br:version', function (e) {
    var rel = e.detail && e.detail.release;
    if (!rel || !Array.isArray(rel.assets)) return;
    var sized = false;
    Object.keys(ROW_MATCHERS).forEach(function (row) {
      var r = rows[row];
      if (!r) return;
      var match = null;
      for (var i = 0; i < rel.assets.length; i++) {
        var asset = rel.assets[i];
        if (asset && ROW_MATCHERS[row].test(String(asset.name)) && /^https:\/\/github\.com\//.test(String(asset.browser_download_url))) { match = asset; break; }
      }
      if (match) {
        applyAsset(row, { name: String(match.name), url: String(match.browser_download_url), size: Number(match.size) || 0 });
        if (Number(match.size) > 0) sized = true;
        r.tr.hidden = false;
      } else if (r.tr.hasAttribute('data-optional')) {
        // An optional file is not in every release: hide it rather than link a 404.
        r.tr.hidden = true;
      }
    });
    if (table && sized) table.classList.remove('sizes-pending');
    // The release day as GitHub records it (UTC), so every visitor reads the same date.
    var when = rel.published_at ? new Date(rel.published_at) : null;
    if (el.date && when && !isNaN(when)) {
      el.date.textContent = ' · ' + when.toLocaleDateString('en-US', { month: 'long', day: 'numeric', year: 'numeric', timeZone: 'UTC' });
    }
    render();
  });

  // Never leave the offer hidden, whatever happens to detection.
  var failsafe = setTimeout(function () { get.classList.add('ready'); }, 2000);
  detect().then(function (s) { state = s; }, function () { state = { os: null }; }).then(function () {
    clearTimeout(failsafe);
    render();
  });
})();
