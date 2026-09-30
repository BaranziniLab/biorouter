/* Principle 02: Share the method, not the data.
   Three sites, each with its own records and its own model. A workflow file
   (instructions, parameters, extensions; never records) is written at UCSF,
   run there, then copied to Site B and to Site C. Each site runs it in its own
   private session, fed only by its own records. Nothing flows back.
   Source: docs/workflows/creating-and-sharing-workflows.md ("Each recipient
   gets their own private session ... No data is shared between users"). */
(function () {
  'use strict';
  if (!window.BR || !BR.art) return;

  // Every model here is one of Biorouter's private providers (Versa API Azure is
  // Institutional; Llama Server and Ollama are Local), so every chip wears the padlock.
  var SITES = [
    { label: 'UCSF', model: 'Versa API Azure' },
    { label: 'Site B', model: 'Llama Server' },
    { label: 'Site C', model: 'Ollama' }
  ];
  var SVGNS = 'http://www.w3.org/2000/svg';

  // The BR mark, drawn as the app's boot splash draws it (Inter 800).
  var MARK = '<svg class="fed-mark" viewBox="0 -74 117 115.1" aria-hidden="true">' +
    '<text class="b" x="0" y="0" font-size="100">B</text>' +
    '<text class="r" x="68.14" y="0" font-size="74">R</text>' +
    '<rect class="b" x="0" y="23.36" width="66.64" height="17.7"/>' +
    '<rect class="r" x="66.64" y="23.36" width="49.98" height="17.7"/></svg>';

  // The app's model chip: Brain icon, model name, padlock for a private model.
  var BRAIN = '<svg class="fed-brain" viewBox="0 0 24 24" aria-hidden="true">' +
    '<path d="M12 5a3 3 0 1 0-5.997.125 4 4 0 0 0-2.526 5.77 4 4 0 0 0 .556 6.588A4 4 0 1 0 12 18Z"/>' +
    '<path d="M12 5a3 3 0 1 1 5.997.125 4 4 0 0 1 2.526 5.77 4 4 0 0 1-.556 6.588A4 4 0 1 1 12 18Z"/>' +
    '<path d="M15 13a4.5 4.5 0 0 1-3-4 4.5 4.5 0 0 1-3 4"/></svg>';

  var RECORDS = '<svg class="fed-db" viewBox="0 0 18 22" aria-hidden="true">' +
    '<path d="M1.5 4.5v13c0 1.66 3.36 3 7.5 3s7.5-1.34 7.5-3v-13"/>' +
    '<ellipse cx="9" cy="4.5" rx="7.5" ry="3"/>' +
    '<path d="M1.5 11c0 1.66 3.36 3 7.5 3s7.5-1.34 7.5-3"/></svg>';

  // A finished run: the result stays here.
  var CHECK = '<svg class="fed-check" viewBox="0 0 24 24" aria-hidden="true"><path d="M20 6 9 17l-5-5"/></svg>';

  function el(tag, cls, html) {
    var n = document.createElement(tag);
    if (cls) n.className = cls;
    if (html != null) n.innerHTML = html;
    return n;
  }

  function card() {
    return el('div', 'fed-card',
      '<p class="fed-file">cohort-study.yaml</p>' +
      '<p class="fed-key"><span>instructions:</span><i style="width:100%"></i></p>' +
      '<p class="fed-key"><span>parameters:</span><i style="width:70%"></i></p>' +
      '<p class="fed-key"><span>extensions:</span><i style="width:86%"></i></p>');
  }

  BR.art('federate', function (root, opts) {
    root.classList.add('art-federate');
    var reduced = !!(opts && opts.reduced);

    var wrap = el('div', 'fed');
    wrap.setAttribute('aria-hidden', 'true');
    var sitesEl = el('div', 'fed-sites');
    var routes = document.createElementNS(SVGNS, 'svg');
    routes.setAttribute('class', 'fed-routes');

    var sites = SITES.map(function (s) {
      var site = el('div', 'fed-site');
      site.appendChild(el('p', 'fed-head', MARK + '<span>' + s.label + '</span>'));
      site.appendChild(el('p', 'fed-model', BRAIN + '<span class="fed-mname">' + s.model + '</span><span class="fed-lock"></span>'));
      var c = card();
      site.appendChild(c);
      var run = el('div', 'fed-run', '<span class="privacy is-private">Private session</span><span class="fed-bar"><i></i></span>' + CHECK);
      site.appendChild(run);
      var link = el('div', 'fed-link', '<i></i>');
      site.appendChild(link);
      site.appendChild(el('p', 'fed-rec', RECORDS + '<span>Patient records</span><span class="fed-lock"></span>'));
      sitesEl.appendChild(site);
      return { site: site, card: c, run: run, link: link };
    });

    // One route per hop: a quiet dashed shaft and a solid head that remain, and a
    // coral line (shaft and head in one path) that draws while the file travels.
    function svgPath(cls) {
      var p = document.createElementNS(SVGNS, 'path');
      p.setAttribute('class', cls);
      return p;
    }
    var hops = [0, 1].map(function () {
      var g = document.createElementNS(SVGNS, 'g');
      var rest = svgPath('fed-route-rest'), head = svgPath('fed-route-head'), live = svgPath('fed-route-live');
      live.setAttribute('pathLength', '1');
      g.appendChild(rest); g.appendChild(head); g.appendChild(live);
      routes.appendChild(g);
      return { g: g, rest: rest, head: head, live: live };
    });

    var end = el('p', 'fed-end', 'The workflow travels. The records stay.');
    wrap.appendChild(sitesEl);
    wrap.appendChild(routes);
    wrap.appendChild(end);
    root.appendChild(wrap);

    // Position of an element's layout box inside the root, ignoring transforms.
    function box(n) {
      var x = 0, y = 0, p = n;
      while (p && p !== root) { x += p.offsetLeft; y += p.offsetTop; p = p.offsetParent; }
      return { x: x, y: y, w: n.offsetWidth, h: n.offsetHeight };
    }
    // Matches the container query in federate.css: one grid column means the sites are stacked.
    function stacked() { return getComputedStyle(sitesEl).gridTemplateColumns.trim().split(/\s+/).length === 1; }

    function layoutRoutes() {
      var w = root.clientWidth, h = root.clientHeight;
      routes.setAttribute('viewBox', '0 0 ' + w + ' ' + h);
      routes.setAttribute('width', w);
      routes.setAttribute('height', h);
      var col = stacked();
      hops.forEach(function (hop, i) {
        var a = box(sites[i].card), b = box(sites[i + 1].card), shaft, head;
        if (col) {
          var x = Math.round(a.x + a.w / 2) + 0.5, y1 = a.y + a.h + 5, y2 = b.y - 5;
          shaft = 'M' + x + ' ' + y1 + 'V' + y2;
          head = 'M' + (x - 3.5) + ' ' + (y2 - 4) + 'L' + x + ' ' + y2 + 'L' + (x + 3.5) + ' ' + (y2 - 4);
        } else {
          var y = Math.round(a.y + a.h / 2) + 0.5, x1 = a.x + a.w + 5, x2 = b.x - 5;
          shaft = 'M' + x1 + ' ' + y + 'H' + x2;
          head = 'M' + (x2 - 4) + ' ' + (y - 3.5) + 'L' + x2 + ' ' + y + 'L' + (x2 - 4) + ' ' + (y + 3.5);
        }
        // The dashes belong to the shaft only; the head stays a whole chevron.
        hop.rest.setAttribute('d', shaft);
        hop.head.setAttribute('d', head);
        hop.live.setAttribute('d', shaft + head);
      });
    }

    function set(n, cls, on) { n.classList.toggle(cls, on !== false); }

    // How far a copy must travel before its words are clear of the original
    // card: from the near edge of the card's text to the card's far edge.
    function clearance(card, vertical) {
      var r = document.createRange(), near = Infinity, cb = card.getBoundingClientRect();
      card.querySelectorAll('.fed-file, .fed-key span').forEach(function (n) {
        r.selectNodeContents(n);
        var b = r.getBoundingClientRect();
        near = Math.min(near, vertical ? b.top : b.left);
      });
      return (vertical ? cb.bottom : cb.right) - near;
    }

    // A copy slides out from under the source card (which stays) to its own
    // place in the next site. Its value bars travel with it from the start; its
    // words appear once they are clear of the original card, so no word is ever
    // cut by its edge and no two texts ever sit on top of each other.
    var raf = 0;
    function send(i) {
      var src = sites[i].card, c = sites[i + 1].card;
      var from = box(src), to = box(c), vertical = stacked();
      var reach = clearance(src, vertical) + 1, s0 = src.getBoundingClientRect();
      set(c, 'is-launch'); set(c, 'is-copying');
      c.style.transform = 'translate(' + (from.x - to.x) + 'px,' + (from.y - to.y) + 'px)';
      set(c, 'is-shown'); set(c, 'is-moving');
      void c.offsetWidth;
      c.style.transform = '';
      set(c, 'is-launch', false);
      cancelAnimationFrame(raf);
      (function watch() {
        var b = c.getBoundingClientRect();
        var moved = vertical ? Math.abs(b.top - s0.top) : Math.abs(b.left - s0.left);
        if (moved >= reach) { raf = 0; set(c, 'is-copying', false); return; }
        raf = requestAnimationFrame(watch);
      })();
    }

    function runSteps(i, t) {
      var s = sites[i];
      return [
        [t, function () { set(s.run, 'is-on'); }],
        [t + 250, function () { set(s.link, 'is-feeding'); }],
        [t + 650, function () { set(s.run, 'is-running'); }],
        [t + 2250, function () { set(s.run, 'is-done'); set(s.link, 'is-done'); }]
      ];
    }
    function hopSteps(i, t) {
      var hop = hops[i];
      return [
        [t, function () { layoutRoutes(); set(hop.g, 'is-drawing'); }],
        [t + 250, function () { send(i); }],
        [t + 1450, function () { set(sites[i + 1].card, 'is-moving', false); set(hop.g, 'is-set'); set(hop.g, 'is-drawing', false); }]
      ];
    }

    // UCSF's own run is short (its bar fills in 900ms, see federate.css), so the
    // first copy is on its way by 2.3s and lands in Site B by about 3s.
    function quickRunSteps(i, t) {
      var s = sites[i];
      return [
        [t, function () { set(s.run, 'is-on'); }],
        [t + 150, function () { set(s.link, 'is-feeding'); }],
        [t + 350, function () { set(s.run, 'is-running'); }],
        [t + 1150, function () { set(s.run, 'is-done'); set(s.link, 'is-done'); }]
      ];
    }

    var LOOP = 13500;
    var steps = [
      [300, function () { set(sites[0].card, 'is-shown'); }]
    ].concat(
      quickRunSteps(0, 700),
      hopSteps(0, 2050),
      runSteps(1, 3600),
      hopSteps(1, 6100),
      runSteps(2, 7700),
      // The closing line, and the path the file took, in coral.
      [[10200, function () { set(end, 'is-shown'); hops.forEach(function (h) { set(h.g, 'is-travelled'); }); }],
       [12900, function () { set(wrap, 'is-leaving'); }]]
    );

    function clear() {
      set(wrap, 'no-anim');
      set(wrap, 'is-leaving', false);
      set(end, 'is-shown', false);
      sites.forEach(function (s) {
        ['is-shown', 'is-moving', 'is-launch', 'is-copying'].forEach(function (k) { set(s.card, k, false); });
        ['is-on', 'is-running', 'is-done'].forEach(function (k) { set(s.run, k, false); });
        ['is-feeding', 'is-done'].forEach(function (k) { set(s.link, k, false); });
        s.card.style.transform = '';
      });
      hops.forEach(function (h) { ['is-drawing', 'is-set', 'is-travelled'].forEach(function (k) { set(h.g, k, false); }); });
      void wrap.offsetWidth;
      set(wrap, 'no-anim', false);
    }

    function still() {
      set(wrap, 'no-anim');
      sites.forEach(function (s) {
        set(s.card, 'is-shown');
        set(s.run, 'is-on'); set(s.run, 'is-running'); set(s.run, 'is-done');
        set(s.link, 'is-feeding'); set(s.link, 'is-done');
      });
      hops.forEach(function (h) { set(h.g, 'is-set'); set(h.g, 'is-travelled'); });
      set(end, 'is-shown');
    }

    var timers = [], running = false;
    function cycle() {
      halt();
      clear();
      steps.forEach(function (s) { timers.push(setTimeout(s[1], s[0])); });
      timers.push(setTimeout(cycle, LOOP));
    }
    function halt() { timers.forEach(clearTimeout); timers = []; cancelAnimationFrame(raf); raf = 0; }

    layoutRoutes();
    if ('ResizeObserver' in window) new ResizeObserver(layoutRoutes).observe(root);
    else window.addEventListener('resize', layoutRoutes);

    if (reduced) { still(); return { start: function () {}, stop: function () {} }; }
    clear();

    return {
      start: function () { if (running) return; running = true; layoutRoutes(); cycle(); },
      stop: function () { running = false; halt(); clear(); }
    };
  });
})();
