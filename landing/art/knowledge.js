/* Knowledge bases tile. A peer-reviewed paper drops into a BioOKF knowledge
   base, the digest writes three typed pages, and the graph grows from the
   source. Vocabulary from the real feature:
   - crates/biorouter-mcp/src/knowledge/schema_biookf.md: every page links to
     its source with `reported_in`; every edge carries knowledge_level,
     agent_type and primary_source (the identifier of a Publication node).
   - biookf/vocabulary.rs + domain_range.rs: `treats` runs Molecule -> Disease.
   - components/knowledge/graph/ForceGraphCanvas.tsx + nodeMark.ts: every node
     is a circle filled by type, except the Provenance & context family
     (Publication among them), which is an open ring in its type colour on the
     ground. A directed edge is a tapered quad, thick at the subject, in ink at
     18%. Labels are always on, 6px from the mark.
   - CredibilityRing.tsx + NodePreview.tsx: the inspector shows a source's tier
     as a 10px four-arc ring beside the words "Peer reviewed".
   - EdgePreview.tsx: "Knowledge level" / "Primary source" with mono values.
   - IngestPanel.tsx: "Digesting…". */
(function () {
  'use strict';

  var SVGNS = 'http://www.w3.org/2000/svg';

  // The paper and the three pages its digest writes. The paper is a file
  // ("PDF") while it is dropped, and a Publication once it joins the graph.
  var NODES = [
    { id: 'p', name: 'Hauser 2017', sub: 'PDF · Peer reviewed', type: 'Publication', tier: 'Peer reviewed', side: 'r', kind: 'pub' },
    { id: 'o', name: 'Ocrelizumab', sub: 'Molecule', side: 'l', kind: 'molecule' },
    { id: 'd', name: 'Multiple sclerosis', sub: 'Disease', side: 'r', kind: 'disease' },
    { id: 'g', name: 'MS4A1', sub: 'Gene', side: 'l', kind: 'gene' }
  ];
  var PAGES = ['o', 'd', 'g']; // the order the digest writes them

  // One loop, 12 s. One thing moves at a time.
  var T = {
    card: 350,      // the paper drops into the base
    status: 1250,   // Digesting…
    source: 2000,   // the file lets go, and the card becomes the source node
    srcNode: 2125,  //   (the node comes in once the file icon has gone)
    page0: 2750,    // each page: its reported_in edge draws in coral, then its node lands
    pageStep: 800,
    nodeLag: 300,
    statusDone: 5250, // Digest complete
    statusOff: 6050,
    claim: 6450,    // the coral treats edge
    pred: 7050,     // its predicate label, once the edge is drawn
    note: 7900,     // its provenance
    out: 11150,     // fade out
    loop: 12000
  };

  function el(tag, cls, text) {
    var e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text != null) e.textContent = text;
    return e;
  }
  function sv(tag, attrs, cls) {
    var e = document.createElementNS(SVGNS, tag);
    if (cls) e.setAttribute('class', cls);
    for (var k in attrs) e.setAttribute(k, attrs[k]);
    return e;
  }
  function icon(paths, cls) {
    var s = sv('svg', { viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', 'stroke-width': '1.5', 'stroke-linecap': 'round', 'stroke-linejoin': 'round' }, cls);
    paths.forEach(function (p) { s.appendChild(sv(p[0], p[1])); });
    return s;
  }
  function on(e, v) { if (e) e.classList.toggle('on', v !== false); }

  // CredibilityRing.tsx at its 10px size: r=4, 1.6px, four arcs from 12
  // o'clock with a 0.5 rad gap after each.
  function credRing() {
    var r = 4, n = 4, gap = 0.5, span = (Math.PI * 2) / n - gap, d = '';
    for (var i = 0; i < n; i++) {
      var a0 = -Math.PI / 2 + ((Math.PI * 2) / n) * i, a1 = a0 + span;
      d += 'M' + (5 + r * Math.cos(a0)).toFixed(2) + ',' + (5 + r * Math.sin(a0)).toFixed(2) +
        'A' + r + ',' + r + ' 0 0 1 ' + (5 + r * Math.cos(a1)).toFixed(2) + ',' + (5 + r * Math.sin(a1)).toFixed(2);
    }
    var s = sv('svg', { viewBox: '0 0 10 10', width: 10, height: 10 }, 'kn-cred');
    s.appendChild(sv('path', { d: d }));
    return s;
  }

  // A directed edge as the app paints it: a quad from the thin end (0,0) to
  // the thick end (len,0), in a group placed and turned onto the edge. The
  // polygon itself carries only the CSS draw (scaleX from its origin).
  function quad(poly, grp, x0, y0, x1, y1, w0, w1) {
    var dx = x1 - x0, dy = y1 - y0, len = Math.hypot(dx, dy);
    grp.setAttribute('transform', 'translate(' + x0.toFixed(2) + ' ' + y0.toFixed(2) + ') rotate(' + (Math.atan2(dy, dx) * 180 / Math.PI).toFixed(3) + ')');
    poly.setAttribute('points', '0,' + (-w0) + ' ' + len.toFixed(2) + ',' + (-w1) + ' ' + len.toFixed(2) + ',' + w1 + ' 0,' + w0);
  }

  BR.art('knowledge', function (root, opts) {
    var reduced = !!opts.reduced;
    var wrap = el('div', 'kn');
    wrap.setAttribute('aria-hidden', 'true');

    // The base's title row: the app's Knowledge glyph, the base, its format.
    var head = el('div', 'kn-head');
    head.appendChild(icon([
      ['circle', { cx: 12, cy: 12, r: 3 }], ['circle', { cx: 5, cy: 6, r: 1.6 }], ['circle', { cx: 19, cy: 6, r: 1.6 }],
      ['circle', { cx: 6, cy: 18, r: 1.6 }], ['circle', { cx: 18, cy: 18, r: 1.6 }],
      ['path', { d: 'M10 10.5L6 7M14 10.5l4-3.5M10 14l-4 3M14 14l4 3' }]
    ], 'kn-head-ico'));
    head.appendChild(el('span', 'kn-base', 'MS literature'));
    head.appendChild(el('span', 'kn-chip', 'BioOKF'));
    wrap.appendChild(head);

    // The dropped source, as a card, before it joins the graph.
    var card = el('div', 'kn-card');
    wrap.appendChild(card);

    var svg = sv('svg', {}, 'kn-svg');
    var gEdges = sv('g', {});
    var gClaim = sv('g', {});
    var claim = sv('polygon', {}, 'kn-claim');
    gClaim.appendChild(claim);
    var gNodes = sv('g', {});
    svg.appendChild(gEdges); svg.appendChild(gClaim); svg.appendChild(gNodes);
    wrap.appendChild(svg);

    var file = icon([
      ['path', { d: 'M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z' }],
      ['path', { d: 'M14 2v4a2 2 0 0 0 2 2h4' }],
      ['path', { d: 'M10 9H8' }], ['path', { d: 'M16 13H8' }], ['path', { d: 'M16 17H8' }]
    ], 'kn-file');
    wrap.appendChild(file);

    var N = {};
    NODES.forEach(function (n) {
      var o = { def: n };
      o.g = sv('g', {}, 'kn-node kn-' + n.kind);
      o.c = sv('circle', {}, 'kn-dot');
      o.g.appendChild(o.c);
      gNodes.appendChild(o.g);
      o.lab = el('div', 'kn-lab kn-' + n.side + (n.id === 'p' ? ' kn-plab' : ''));
      o.lab.appendChild(el('span', 'kn-name', n.name));
      if (n.type) {
        // The file's line gives way to its node type; its tier moves to a line
        // of its own, behind the ring the inspector draws beside it.
        var swap = el('span', 'kn-sub kn-swap');
        swap.appendChild(el('span', 'kn-a', n.sub));
        swap.appendChild(el('span', 'kn-b', n.type));
        o.lab.appendChild(swap);
        var tier = el('span', 'kn-sub kn-tier');
        tier.appendChild(credRing());
        tier.appendChild(el('span', null, n.tier));
        o.lab.appendChild(tier);
      } else {
        o.lab.appendChild(el('span', 'kn-sub', n.sub));
      }
      wrap.appendChild(o.lab);
      if (n.id !== 'p') {
        o.eg = sv('g', {});
        o.edge = sv('polygon', {}, 'kn-edge');
        o.eg.appendChild(o.edge);
        gEdges.appendChild(o.eg);
      }
      N[n.id] = o;
    });

    var pred = el('span', 'kn-pred', 'treats');
    wrap.appendChild(pred);

    var status = el('div', 'kn-status');
    status.appendChild(el('span', 'kn-run', 'Digesting…'));
    status.appendChild(el('span', 'kn-done', 'Digest complete'));
    wrap.appendChild(status);

    var note = el('div', 'kn-note');
    [['Knowledge level', 'knowledge_assertion'], ['Primary source', 'Hauser 2017']].forEach(function (r) {
      note.appendChild(el('span', 'kn-k', r[0]));
      note.appendChild(el('span', 'kn-v', r[1]));
    });
    wrap.appendChild(note);

    root.appendChild(wrap);

    // ── Layout: measured, so labels never collide at any tile size ──
    function layout() {
      var Wr = root.clientWidth, Hr = root.clientHeight;
      if (!Wr || !Hr) return;
      // A large frame (the one column grid) shows the same picture scaled up,
      // not the same marks spread thin.
      var s = Wr > 380 ? Math.min(Wr / 340, Hr / 255) : 1;
      var W = Wr / s, H = Hr / s;
      wrap.style.width = W + 'px'; wrap.style.height = H + 'px';
      wrap.style.transform = s === 1 ? '' : 'scale(' + s.toFixed(4) + ')';
      var sm = W < 300, xs = W < 276;
      wrap.classList.toggle('kn-sm', sm);
      wrap.classList.toggle('kn-xs', xs);
      var pad = xs ? 12 : sm ? 14 : 16, gap = 6;
      var r = xs ? { p: 7, o: 5.5, d: 5.5, g: 4.5 } : { p: sm ? 7.5 : 8, o: sm ? 6 : 6.5, d: sm ? 6 : 6.5, g: sm ? 5 : 5.5 };
      var w = {};
      NODES.forEach(function (n) { w[n.id] = N[n.id].lab.offsetWidth; });
      var nameH = sm ? 14 : 15, subH = sm ? 14 : 15; // the label's line boxes
      var below = nameH / 2 + subH;         // node centre to the bottom of a two line label
      var headB = pad + 20;
      var noteH = note.offsetHeight || 31;
      var noteGap = sm ? 12 : 16, minM = sm ? 16 : 20;

      // Snap a row to the middle of a device pixel row, so the horizontal
      // edges on it render as even lines rather than stepped bands. The
      // browser paints an inline SVG at a pixel snapped offset, so local
      // coordinates are enough.
      var dpr = window.devicePixelRatio || 1;
      function snap(v) { return (Math.floor(v * dpr) + 0.5) / dpr; }

      var x = {}, y = {};
      x.o = pad + w.o + gap + r.o;
      x.d = W - pad - w.d - gap - r.d;
      // Rows: the source and the gene on top, the claim underneath, then the
      // note. The block sits between the title row and the frame's bottom edge,
      // with any spare height going first to the rows and then mostly above.
      var avail = H - headB;
      var fixed = nameH / 2 + below + noteGap + noteH;
      var rowGap = Math.round(Math.min(0.38 * H, avail - fixed - 2 * minM));
      var spare = avail - fixed - rowGap;
      var bottomM = Math.max(minM, spare * 0.4);
      var top = snap(headB + (spare - bottomM) + nameH / 2);
      y.p = y.g = top;
      y.o = y.d = top + rowGap;
      // The source sits above the disease, close enough that its edge to the
      // disease passes its own label on the way down: at the bottom of the
      // tier's ring (the label's lowest, leftmost mark) the edge is still
      // `clearX` left of the label.
      var clearX = 6, ringBottom = nameH / 2 + 1.5 * subH + 5;
      var minP = x.d - rowGap * (r.p + gap - clearX) / ringBottom;
      // Where there is room it also moves right far enough for the gene to sit
      // straight above the molecule.
      var maxP = W - pad - w.p - gap - r.p;
      x.p = Math.min(maxP, Math.max(minP, x.o + 56));
      x.g = Math.max(pad + w.g + gap + r.g, Math.min(x.o, x.p - 56));
      // The predicate sits under its edge, between the edge and its provenance.
      var predX = (x.o + x.d) / 2, predTop = y.o + 5;
      var noteY = y.o + below + noteGap;

      svg.setAttribute('viewBox', '0 0 ' + W + ' ' + H);
      svg.setAttribute('width', W); svg.setAttribute('height', H);

      // Where an edge stops short of a node: the mark's outer edge plus 1.5px.
      var hollowOut = 0.85, outlineOut = 0.5;
      NODES.forEach(function (n) {
        var o = N[n.id], cx = x[n.id], cy = y[n.id], rr = r[n.id];
        o.c.setAttribute('cx', cx); o.c.setAttribute('cy', cy); o.c.setAttribute('r', rr);
        var off = rr + gap;
        o.lab.style.top = (cy - nameH / 2) + 'px';
        if (n.side === 'r') { o.lab.style.left = (cx + off) + 'px'; o.lab.style.right = 'auto'; }
        else { o.lab.style.right = (W - (cx - off)) + 'px'; o.lab.style.left = 'auto'; }
        if (o.edge) {
          // The page's reported_in link: thin at the Publication, thick at the
          // page (its subject), drawn from the source outward.
          var ex = cx - x.p, ey = cy - y.p, len = Math.hypot(ex, ey), ux = ex / len, uy = ey / len;
          var s0 = r.p + hollowOut + 1.5, s1 = rr + outlineOut + 1.5;
          quad(o.edge, o.eg, x.p + ux * s0, y.p + uy * s0, cx - ux * s1, cy - uy * s1, 0.5, 0.85);
        }
      });

      // The claim: Ocrelizumab treats Multiple sclerosis. Drawn from its
      // subject, so the quad starts thick (1.25) and runs down to 0.75; its row
      // is snapped, so both ends land on whole device pixels.
      var sx = x.o + r.o + outlineOut + 1.5, len2 = (x.d - r.d - outlineOut - 1.5) - sx;
      gClaim.setAttribute('transform', 'translate(' + sx.toFixed(2) + ' ' + y.o + ')');
      claim.setAttribute('points', '0,-1.25 ' + len2.toFixed(2) + ',-0.75 ' + len2.toFixed(2) + ',0.75 0,1.25');

      pred.style.left = predX + 'px';
      pred.style.top = predTop + 'px';

      // The card wraps the file icon and the label's first two lines.
      var cl = x.p - 17, ct = y.p - nameH / 2 - 9;
      card.style.left = cl + 'px'; card.style.top = ct + 'px';
      card.style.width = (r.p + gap + w.p + 17 + 11) + 'px';
      card.style.height = (nameH / 2 + below + 18) + 'px';
      file.style.left = (x.p - 8) + 'px'; file.style.top = (y.p - 8) + 'px';

      status.style.left = pad + 'px'; status.style.top = noteY + 'px';
      note.style.left = pad + 'px'; note.style.top = noteY + 'px';
    }

    // ── States ──
    var timers = [];
    function later(ms, fn) { timers.push(setTimeout(fn, ms)); }
    function clearAll() { timers.forEach(clearTimeout); timers = []; }

    function hideAll() {
      [card, file, status, note, pred, claim].forEach(function (e) { on(e, false); });
      status.classList.remove('done');
      wrap.classList.remove('src', 'out');
      NODES.forEach(function (n) {
        var o = N[n.id]; on(o.g, false); on(o.lab, false); on(o.edge, false);
        if (o.edge) o.edge.classList.remove('set');
      });
    }
    function still() {
      // The finished picture: the graph, the claim and its provenance.
      wrap.classList.add('kn-still', 'src');
      [note, pred, claim].forEach(function (e) { on(e); });
      NODES.forEach(function (n) {
        var o = N[n.id]; on(o.g); on(o.lab); on(o.edge);
        if (o.edge) o.edge.classList.add('set');
      });
    }

    function run() {
      clearAll();
      wrap.classList.add('kn-still');
      hideAll();
      void wrap.offsetWidth;
      wrap.classList.remove('kn-still');
      later(T.card, function () { on(card); on(file); on(N.p.lab); });
      later(T.status, function () { on(status); });
      later(T.source, function () { wrap.classList.add('src'); on(card, false); on(file, false); });
      later(T.srcNode, function () { on(N.p.g); });
      PAGES.forEach(function (id, i) {
        var t = T.page0 + i * T.pageStep, o = N[id];
        later(t, function () { on(o.edge); });
        // The node lands and its link settles from coral into the graph's ink.
        later(t + T.nodeLag, function () { on(o.g); on(o.lab); o.edge.classList.add('set'); });
      });
      later(T.statusDone, function () { status.classList.add('done'); });
      later(T.statusOff, function () { on(status, false); });
      later(T.claim, function () { on(claim); });
      later(T.pred, function () { on(pred); });
      later(T.note, function () { on(note); });
      later(T.out, function () { wrap.classList.add('out'); });
      later(T.loop, run);
    }

    layout();
    if (reduced) still(); else { wrap.classList.add('kn-still'); hideAll(); }

    if ('ResizeObserver' in window) {
      var last = '';
      new ResizeObserver(function () {
        var k = root.clientWidth + 'x' + root.clientHeight;
        if (k === last) return;
        last = k;
        var had = wrap.classList.contains('kn-still');
        wrap.classList.add('kn-still');
        layout();
        void wrap.offsetWidth;
        if (!had) wrap.classList.remove('kn-still');
      }).observe(root);
    }

    return {
      start: function () { if (!reduced) { wrap.classList.remove('kn-paused'); layout(); run(); } },
      stop: function () { clearAll(); wrap.classList.add('kn-paused'); }
    };
  });
})();
