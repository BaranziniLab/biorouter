/* Biorouter website: shared behaviour.
   Header state, the mobile menu, reveal on scroll, copy buttons, the release
   version, and a small helper the artifacts in art/ use to run only while they
   are on screen. No dependencies. */
(function () {
  'use strict';
  document.documentElement.classList.add('js');

  var reduced = window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  /* Artifacts register here: BR.art('name', function (root, api) { ... return { start, stop } }).
     Each [data-art="name"] element is mounted once; start/stop follow visibility. */
  var registry = {};
  var BR = window.BR = window.BR || {};
  BR.reduced = reduced;
  BR.art = function (name, mount) { registry[name] = mount; mountAll(); };
  BR.sleep = function (ms) { return new Promise(function (r) { setTimeout(r, ms); }); };

  function mountAll() {
    var nodes = document.querySelectorAll('[data-art]');
    for (var i = 0; i < nodes.length; i++) {
      var el = nodes[i];
      var name = el.getAttribute('data-art');
      if (el.__mounted || !registry[name]) continue;
      el.__mounted = true;
      var ctl = registry[name](el, { reduced: reduced }) || {};
      watch(el, ctl);
    }
  }

  function watch(el, ctl) {
    if (!('IntersectionObserver' in window)) { if (ctl.start) ctl.start(); return; }
    var running = false;
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) {
        var visible = e.isIntersecting && e.intersectionRatio >= 0.2;
        if (visible && !running) { running = true; if (ctl.start) ctl.start(); }
        else if (!visible && running) { running = false; if (ctl.stop) ctl.stop(); }
      });
    }, { threshold: 0.2 });
    io.observe(el);
  }

  function ready(fn) { if (document.readyState !== 'loading') fn(); else document.addEventListener('DOMContentLoaded', fn); }

  ready(function () {
    mountAll();

    // Header hairline once the page scrolls
    var header = document.querySelector('.site-header');
    if (header) {
      var onScroll = function () { header.classList.toggle('scrolled', window.scrollY > 4); };
      onScroll();
      window.addEventListener('scroll', onScroll, { passive: true });
    }

    // Mobile menu
    var btn = document.querySelector('.menu-btn');
    var nav = document.querySelector('.nav');
    if (btn && nav) {
      btn.addEventListener('click', function () {
        var open = btn.getAttribute('aria-expanded') !== 'true';
        btn.setAttribute('aria-expanded', String(open));
        nav.classList.toggle('open', open);
      });
      nav.addEventListener('click', function (e) {
        if (e.target.closest('a')) { btn.setAttribute('aria-expanded', 'false'); nav.classList.remove('open'); }
      });
      document.addEventListener('keydown', function (e) {
        if (e.key === 'Escape' && nav.classList.contains('open')) { btn.setAttribute('aria-expanded', 'false'); nav.classList.remove('open'); btn.focus(); }
      });
    }

    // Reveal on scroll
    var reveals = document.querySelectorAll('.reveal');
    if (reduced || !('IntersectionObserver' in window)) {
      reveals.forEach(function (el) { el.classList.add('visible'); });
    } else {
      var rio = new IntersectionObserver(function (entries) {
        entries.forEach(function (e) { if (e.isIntersecting) { e.target.classList.add('visible'); rio.unobserve(e.target); } });
      }, { threshold: 0.08, rootMargin: '0px 0px -40px 0px' });
      reveals.forEach(function (el) { rio.observe(el); });
    }

    // Copy buttons: <button class="copy-btn" data-copy="#id or literal">
    document.addEventListener('click', function (e) {
      var b = e.target.closest('[data-copy]');
      if (!b) return;
      var src = b.getAttribute('data-copy');
      var text = src.charAt(0) === '#' ? (document.querySelector(src) || {}).textContent || '' : src;
      var done = function () {
        var old = b.getAttribute('data-label') || b.textContent;
        b.setAttribute('data-label', old);
        b.textContent = 'Copied';
        setTimeout(function () { b.textContent = old; }, 1600);
      };
      if (navigator.clipboard && navigator.clipboard.writeText) navigator.clipboard.writeText(text.trim()).then(done, function () {});
    });

    // Release version: elements with [data-version] show the latest release tag.
    // The HTML carries a current fallback; this only refreshes it.
    var slots = document.querySelectorAll('[data-version]');
    if (slots.length) {
      fetch('https://api.github.com/repos/BaranziniLab/biorouter/releases/latest', { headers: { Accept: 'application/vnd.github+json' } })
        .then(function (r) { return r.ok ? r.json() : null; })
        .then(function (rel) {
          if (!rel || !rel.tag_name) return;
          var v = String(rel.tag_name).replace(/^v/, '');
          slots.forEach(function (s) { s.textContent = s.getAttribute('data-version') === 'v' ? 'v' + v : v; });
          document.dispatchEvent(new CustomEvent('br:version', { detail: { version: v, release: rel } }));
        })
        .catch(function () {});
    }
  });
})();
