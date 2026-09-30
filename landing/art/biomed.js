/* Principle 03: Built for biomedical research.
   A calm grid of research connectors from BAAM (landing/registry.json), one line
   glyph and one short name each. The two UCSF data connectors (CDWAgent and
   UCSFOMOPAgent) carry the padlock and never animate. The highlight moves across
   five connectors that span the domains (knowledge graph, literature, variants,
   chemistry, the lab), one every 2.8s, so the loop is 14s. Under the grid one
   tool call row does what the app's ToolCallWithResponse does: "Working on …"
   while it breathes, then "Ran …", then one line of result. Each row names the
   source the call reaches (the target the extension's call tool is given), and
   `tool` records the real read only tool behind it; the results are
   illustrative. The footer counts are read from registry.json at runtime.
   PrimeKG is left out on purpose: twelve tiles make a clean grid, its own
   description says it has no graph query engine, and SPOKE covers knowledge
   graphs. */
(function () {
  'use strict';

  // Grid order: UCSF clinical, knowledge graph, literature, genomics, cells,
  // structure and chemistry, imaging, the lab. The five with a `tool` take turns.
  var CONNECTORS = [
    { glyph: 'cdw', name: 'CDW', ucsf: true },
    { glyph: 'omop', name: 'OMOP', ucsf: true },
    { glyph: 'spoke', name: 'SPOKE', tool: 'query_spoke', what: 'Cypher query',
      result: '3 paths link metformin to multiple sclerosis.' },
    { glyph: 'literature', name: 'Literature', tool: 'call_literatureagent_api', what: 'Crossref',
      result: 'All 12 DOIs resolve.' },
    { glyph: 'ncbi', name: 'NCBI' },
    { glyph: 'variants', name: 'Clinical variants', short: 'Variants', tool: 'call_clinicalvariantagent_api', what: 'ClinVar',
      result: 'rs80357914: 2 records, with review status.' },
    { glyph: 'cell', name: 'Single cell' },
    { glyph: 'depmap', name: 'DepMap' },
    { glyph: 'protein', name: 'Protein structure', short: 'Protein' },
    { glyph: 'chem', name: 'Chemistry', tool: 'call_chemoinformaticsagent_api', what: 'PubChem',
      result: 'Caffeine: CID 2519, 194.19 g/mol.' },
    { glyph: 'imaging', name: 'Imaging' },
    { glyph: 'opentrons', name: 'Opentrons', tool: 'call_opentronsagent_api', what: 'Health check',
      result: 'Robot online: name, model and API version.' }
  ];

  var FALLBACK = { extensions: 38, skills: 129 };

  var FIRST_HOLD = 1400; // the opening frame is already a finished result
  var FADE = 150;        // the old result leaves before the highlight moves
  var WORK = 1100;       // "Working on …": long enough for the breath to reach 0.7
  var STEP = 2800;       // one connector; five of them make the 14s loop

  function num(v) { return (+v.toFixed(2)).toString(); }

  // The protein structure glyph: a chain that winds into three helical turns,
  // seen a little from the side, laid on the diagonal and fitted to an 18 unit box.
  // The pitch is open enough that the turns stay apart at 18px.
  function helix() {
    var turns = 3, p = 5, e = 3, R = 7, tail = 1.2, rot = -30 * Math.PI / 180, ext = 18, n = 240;
    var T = turns * 2 * Math.PI, pts = [], i;
    for (i = 0; i <= n; i++) {
      var t = (T * i) / n;
      pts.push([p * t / (2 * Math.PI) + e * Math.sin(t), R * Math.cos(t)]);
    }
    pts.unshift([pts[0][0] - tail, pts[0][1]]);
    pts.push([pts[pts.length - 1][0] + tail, pts[pts.length - 1][1]]);
    var c = Math.cos(rot), s = Math.sin(rot);
    pts = pts.map(function (q) { return [q[0] * c - q[1] * s, q[0] * s + q[1] * c]; });
    var minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
    pts.forEach(function (q) {
      minX = Math.min(minX, q[0]); maxX = Math.max(maxX, q[0]);
      minY = Math.min(minY, q[1]); maxY = Math.max(maxY, q[1]);
    });
    var k = ext / Math.max(maxX - minX, maxY - minY), cx = (minX + maxX) / 2, cy = (minY + maxY) / 2;
    return '<path d="M' + pts.map(function (q) { return num(12 + (q[0] - cx) * k) + ' ' + num(12 + (q[1] - cy) * k); }).join('L') + '"/>';
  }

  function spoke() {
    var s = '<circle cx="12" cy="12" r="2.4"/>';
    for (var k = 0; k < 5; k++) {
      var a = (-90 + 72 * k) * Math.PI / 180, c = Math.cos(a), si = Math.sin(a);
      s += '<circle cx="' + num(12 + 8.3 * c) + '" cy="' + num(12 + 8.3 * si) + '" r="1.7"/>';
      s += '<path d="M' + num(12 + 2.4 * c) + ' ' + num(12 + 2.4 * si) + 'L' + num(12 + 6.6 * c) + ' ' + num(12 + 6.6 * si) + '"/>';
    }
    return s;
  }

  // Line glyphs on a 24 grid, each about 17 to 18 units across, drawn at a true 1.5px stroke.
  var GLYPHS = {
    cdw: '<ellipse cx="12" cy="5.5" rx="7.5" ry="2.75"/><path d="M4.5 5.5v13c0 1.52 3.36 2.75 7.5 2.75s7.5-1.23 7.5-2.75v-13"/><path d="M4.5 12c0 1.52 3.36 2.75 7.5 2.75s7.5-1.23 7.5-2.75"/>',
    omop: '<rect x="3.5" y="4.5" width="17" height="15" rx="2"/><path d="M3.5 9.5h17M3.5 14.5h17M9.5 9.5v10"/>',
    spoke: spoke(),
    literature: '<path d="M14 3.5H7.5a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2h9a2 2 0 0 0 2-2V8z"/><path d="M14 3.5V8h4.5"/><path d="M8.75 12.5h6.5M8.75 16h4.5"/>',
    ncbi: '<path d="M6 3c0 4.5 12 4.5 12 9s-12 4.5-12 9"/><path d="M18 3c0 4.5-12 4.5-12 9s12 4.5 12 9"/><path d="M8.2 5.2h7.6M6.9 12h10.2M8.2 18.8h7.6"/>',
    // A variant lollipop plot: a gene track with two marked positions.
    variants: '<path d="M3 19.5h18"/><path d="M8 19.5V10M16 19.5v-5"/><circle cx="8" cy="7.5" r="2.5"/><circle cx="16" cy="12" r="2.5"/>',
    cell: '<circle cx="12" cy="12" r="8.75"/><circle cx="13" cy="10.75" r="3"/><circle cx="7.9" cy="14.9" r="0.9"/><circle cx="15.9" cy="16.6" r="0.9"/>',
    depmap: '<path d="M3.5 20.5h17"/><path d="M6.5 20.5V5.5M10.5 20.5V9.5M14.5 20.5V13M18.5 20.5V16"/>',
    protein: helix(),
    // A benzene ring (alternating double bonds) with one substituent.
    chem: '<path d="M10.9 3.8L18.69 8.3L18.69 17.3L10.9 21.8L3.11 17.3L3.11 8.3z"/><path d="M16.36 10.66L16.36 14.94M10.03 18.6L6.32 16.45M6.32 9.15L10.03 7"/><path d="M18.69 8.3l2.94-1.7"/>',
    imaging: '<path d="M3.5 8V5.5a2 2 0 0 1 2-2H8M16 3.5h2.5a2 2 0 0 1 2 2V8M20.5 16v2.5a2 2 0 0 1-2 2H16M8 20.5H5.5a2 2 0 0 1-2-2V16"/><circle cx="12" cy="12" r="4.5"/><circle cx="13.6" cy="10.6" r="1.3"/>',
    // A pipette: plunger, a body with a grip line, a tapered tip.
    opentrons: '<g transform="rotate(40 12 12)"><path d="M8.75 1.5h6.5M12 1.5v2.5"/><rect x="8.25" y="4" width="7.5" height="8.75" rx="1.75"/><path d="M8.25 8.4h7.5"/><path d="M10.25 12.75l1.25 7.75h1l1.25-7.75"/></g>'
  };

  // The app's tool row: lucide Wrench (the icon every extension tool gets) and
  // ChevronRight at 60%, both at the app's 1.5 stroke. Lock is the one privacy mark.
  var WRENCH = '<path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/>';
  var CHEVRON = '<path d="m9 18 6-6-6-6"/>';
  var LOCK = '<rect x="4" y="11" width="16" height="10" rx="2"/><path d="M8 11V7a4 4 0 0 1 8 0v4"/>';

  function svg(paths, cls, stroke) {
    return '<svg class="' + cls + '" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="' + (stroke || 1.5) +
      '" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">' + paths + '</svg>';
  }
  function el(tag, cls, text) {
    var n = document.createElement(tag);
    if (cls) n.className = cls;
    if (text != null) n.textContent = text;
    return n;
  }

  // The BAAM affiliation badge: a padlock and "UCSF data", navy on its wash.
  // In a tile the words drop out when the tile is small; the footer then says them.
  function ucsfBadge(inTile) {
    var b = el('span', 'bm-badge');
    b.innerHTML = svg(LOCK, 'bm-lock', 2.2);
    if (inTile) b.appendChild(el('span', 'bm-badge-word', 'UCSF data'));
    return b;
  }

  BR.art('biomed', function (root, opts) {
    var reduced = !!(opts && opts.reduced);
    var wrap = el('div', 'bm');
    wrap.setAttribute('aria-hidden', 'true');

    // The grid
    var grid = el('div', 'bm-grid');
    var shorts = [];
    var tiles = CONNECTORS.map(function (c, i) {
      var t = el('div', 'bm-tile' + (c.ucsf ? ' is-ucsf' : ''));
      var top = el('span', 'bm-top');
      var g = el('span', 'bm-glyph');
      g.innerHTML = svg(GLYPHS[c.glyph], 'bm-glyph-svg');
      top.appendChild(g);
      if (c.ucsf) top.appendChild(ucsfBadge(true));
      t.appendChild(top);
      var name = el('span', 'bm-name');
      if (c.short) {
        name.appendChild(el('span', 'bm-full', c.name));
        shorts[i] = name.appendChild(el('span', 'bm-short', c.short));
      } else {
        name.textContent = c.name;
      }
      t.appendChild(name);
      grid.appendChild(t);
      return t;
    });
    wrap.appendChild(grid);

    // One tool call, as a line, and one line of result under it
    var call = el('div', 'bm-call');
    var row = el('div', 'bm-row');
    row.insertAdjacentHTML('beforeend', svg(WRENCH, 'bm-tool-ico'));
    var rowText = el('span', 'bm-row-text');
    row.appendChild(rowText);
    row.insertAdjacentHTML('beforeend', svg(CHEVRON, 'bm-chev'));
    var result = el('p', 'bm-result');
    call.appendChild(row);
    call.appendChild(result);
    wrap.appendChild(call);

    // Live counts from the marketplace catalog
    var foot = el('p', 'bm-foot');
    var counts = el('span', 'bm-counts');
    var nExt = el('span', 'bm-n', String(FALLBACK.extensions));
    var nSkills = el('span', 'bm-n', String(FALLBACK.skills));
    counts.appendChild(nExt);
    counts.appendChild(document.createTextNode(' extensions · '));
    counts.appendChild(nSkills);
    counts.appendChild(document.createTextNode(' skills'));
    counts.appendChild(el('span', 'bm-in', ' in BAAM'));
    foot.appendChild(counts);
    var legend = el('span', 'bm-legend');
    legend.appendChild(ucsfBadge(false));
    legend.appendChild(document.createTextNode('UCSF data'));
    foot.appendChild(legend);
    wrap.appendChild(foot);

    root.appendChild(wrap);

    if (window.fetch) {
      fetch('registry.json')
        .then(function (r) { return r.ok ? r.json() : null; })
        .then(function (reg) {
          if (!reg) return;
          if (Array.isArray(reg.extensions) && reg.extensions.length) nExt.textContent = String(reg.extensions.length);
          if (Array.isArray(reg.skills) && reg.skills.length) nSkills.textContent = String(reg.skills.length);
        })
        .catch(function () {});
    }

    // The connectors that take a turn, in grid order. The UCSF pair stays still.
    var order = [];
    CONNECTORS.forEach(function (c, i) { if (c.tool) order.push(i); });

    var pos = 0;          // index into order
    var phase = 'done';   // 'done' | 'leaving' | 'working'
    var compact = false;  // true when some row would not fit with its source
    var timer = 0;
    var running = false;

    function later(fn, ms) { clearTimeout(timer); timer = setTimeout(fn, ms); }

    // The name the tile shows right now, so the row and the tile agree.
    function visibleName(i) {
      var s = shorts[i];
      return s && s.getClientRects().length ? CONNECTORS[i].short : CONNECTORS[i].name;
    }
    function label(verb, i) {
      var name = visibleName(i);
      return compact ? verb + ' ' + name : verb + ' ' + name + ' · ' + CONNECTORS[i].what;
    }
    function verb() { return phase === 'working' ? 'Working on' : 'Ran'; }

    // One decision per layout: if any row, in either state, would not fit with
    // its source, every row drops the source, so neighbours never differ.
    function measure() {
      compact = false;
      for (var k = 0; k < order.length && !compact; k++) {
        for (var v = 0; v < 2 && !compact; v++) {
          rowText.textContent = label(v ? 'Working on' : 'Ran', order[k]);
          if (rowText.scrollWidth > rowText.clientWidth + 0.5) compact = true;
        }
      }
      rowText.textContent = label(verb(), order[pos]);
    }

    function show(idx, state) {
      var c = CONNECTORS[idx];
      tiles.forEach(function (t, i) { t.classList.toggle('is-active', i === idx); });
      phase = state;
      rowText.textContent = label(verb(), idx);
      if (state === 'working') {
        row.classList.add('is-running');
        result.classList.remove('is-shown');
        result.textContent = '';
      } else {
        row.classList.remove('is-running');
        result.textContent = c.result;
        result.classList.add('is-shown');
      }
    }

    // One step: the old result fades out, the highlight moves and the row
    // works, then the row reads "Ran …" and the result fades in.
    function next() {
      if (!running) return;
      phase = 'leaving';
      result.classList.remove('is-shown');
      later(advance, FADE);
    }
    function advance() {
      if (!running) return;
      pos = (pos + 1) % order.length;
      show(order[pos], 'working');
      later(finish, WORK);
    }
    function finish() {
      if (!running) return;
      show(order[pos], 'done');
      later(next, STEP - FADE - WORK);
    }

    show(order[0], 'done');
    measure();

    if (typeof ResizeObserver !== 'undefined') {
      new ResizeObserver(measure).observe(root);
    }

    if (reduced) return { start: function () {}, stop: function () {} };

    return {
      start: function () {
        if (running) return;
        running = true;
        if (phase === 'working') { row.classList.add('is-running'); later(finish, WORK); }
        else if (phase === 'leaving') later(advance, FADE);
        else later(next, FIRST_HOLD);
      },
      stop: function () {
        running = false;
        clearTimeout(timer);
        timer = 0;
        row.classList.remove('is-running');
      }
    };
  });
})();
