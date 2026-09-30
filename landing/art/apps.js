/* apps: Built apps (Agent Drafter).
   A small app window, "Cohort explorer", with one author region
   (data-br-region="results"). The agent drives it with the real appcontrol
   tools: ui_chart draws bars into the region; ui_ask shows a form card (a
   prompt, one checkbox field, and the SDK's real "Skip" plus the submit label)
   and waits for the answer; a second ui_chart replaces the chart with two
   series. One thing moves at a time. */
(function () {
  'use strict';
  if (!window.BR || !BR.art) return;

  var NS = 'http://www.w3.org/2000/svg';
  var AGES = ['20s', '30s', '40s', '50s', '60s'];
  // Patients per age group, then split by sex. Each chart autoscales to its own
  // maximum, as ui_chart does, so heights are percentages of the plot.
  var TOTAL = [58, 92, 110, 84, 50];
  var FEMALE = [30, 50, 52, 46, 28];
  var MALE = [28, 42, 58, 38, 22];

  function pct(v, max) { return (v / (max * 1.1) * 100).toFixed(1) + '%'; }

  function el(tag, cls, text) {
    var n = document.createElement(tag);
    if (cls) n.className = cls;
    if (text != null) n.textContent = text;
    return n;
  }
  function svg(viewBox, paths, attrs) {
    var s = document.createElementNS(NS, 'svg');
    s.setAttribute('viewBox', viewBox);
    s.setAttribute('fill', 'none');
    s.setAttribute('aria-hidden', 'true');
    for (var k in attrs) s.setAttribute(k, attrs[k]);
    paths.forEach(function (d) {
      var p = document.createElementNS(NS, 'path');
      p.setAttribute('d', d);
      s.appendChild(p);
    });
    return s;
  }

  BR.art('apps', function (root, opts) {
    var stage = el('div', 'apps-stage is-paused');
    stage.setAttribute('aria-hidden', 'true');

    var win = el('div', 'apps-win');

    // Title bar
    var bar = el('div', 'apps-bar');
    var dots = el('span', 'apps-dots');
    dots.appendChild(el('i')); dots.appendChild(el('i')); dots.appendChild(el('i'));
    bar.appendChild(dots);
    bar.appendChild(el('span', 'apps-name', 'Cohort explorer'));
    win.appendChild(bar);

    // Page with the results region
    var page = el('div', 'apps-page');
    var region = el('div', 'apps-region');
    var empty = el('div', 'apps-empty');
    empty.appendChild(el('span', null, 'results'));
    region.appendChild(empty);

    var chart = el('div', 'apps-chart');
    var head = el('div', 'apps-head');
    head.appendChild(el('span', 'apps-title', 'Patients by age'));
    var legend = el('span', 'apps-legend');
    var la = el('span'); la.appendChild(el('i', 'f')); la.appendChild(document.createTextNode('Female'));
    var lb = el('span'); lb.appendChild(el('i', 'm')); lb.appendChild(document.createTextNode('Male'));
    legend.appendChild(la); legend.appendChild(lb);
    head.appendChild(legend);
    chart.appendChild(head);

    // Each group holds the first chart's bar (t) and the second chart's pair
    // (f, m). The second ui_chart replaces the first: t fades, f and m grow.
    var plot = el('div', 'apps-plot');
    var axis = el('div', 'apps-axis');
    var maxT = Math.max.apply(null, TOTAL);
    var maxS = Math.max(Math.max.apply(null, FEMALE), Math.max.apply(null, MALE));
    for (var i = 0; i < AGES.length; i++) {
      var g = el('div', 'apps-g');
      g.style.setProperty('--i', i);
      g.style.setProperty('--h', pct(TOTAL[i], maxT));
      g.style.setProperty('--hf', pct(FEMALE[i], maxS));
      g.style.setProperty('--hm', pct(MALE[i], maxS));
      g.appendChild(el('i', 't'));
      g.appendChild(el('i', 'f'));
      g.appendChild(el('i', 'm'));
      plot.appendChild(g);
      var lab = el('span', null, AGES[i]);
      lab.style.setProperty('--i', i);
      axis.appendChild(lab);
    }
    chart.appendChild(plot);
    chart.appendChild(axis);
    region.appendChild(chart);
    page.appendChild(region);

    // ui_ask: prompt, one checkbox field (value "true"), then Skip and the submit label
    page.appendChild(el('div', 'apps-scrim'));
    var ask = el('div', 'apps-ask');
    ask.appendChild(el('p', null, 'Redraw the chart?'));
    var field = el('div', 'apps-check');
    var box = el('span', 'apps-box');
    box.appendChild(svg('0 0 12 12', ['M3 6.2 5.1 8.3 9 4.1'], {
      stroke: 'currentColor', 'stroke-width': '1.6', 'stroke-linecap': 'round', 'stroke-linejoin': 'round'
    }));
    field.appendChild(box);
    field.appendChild(el('span', null, 'Split by sex'));
    ask.appendChild(field);
    var actions = el('div', 'apps-actions');
    actions.appendChild(el('span', 'apps-btn apps-skip', 'Skip'));
    var go = el('span', 'apps-btn apps-go', 'Redraw');
    actions.appendChild(go);
    ask.appendChild(actions);
    page.appendChild(ask);

    var cursor = el('div', 'apps-cursor');
    cursor.appendChild(svg('0 0 13 18', ['M1.5 1.5v13.2l3.4-3.2 2.4 5.2 2.3-1-2.4-5.1h4.6z'], {
      'stroke-width': '1.1', 'stroke-linejoin': 'round'
    }));
    page.appendChild(cursor);
    win.appendChild(page);

    // Agent strip: one tool row, with the app's 2px active rail while it runs
    var strip = el('div', 'apps-strip');
    strip.appendChild(el('span', 'apps-rail'));
    strip.appendChild(svg('0 0 24 24', ['M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z'], {
      stroke: 'currentColor', 'stroke-width': '1.7', 'stroke-linecap': 'round', 'stroke-linejoin': 'round'
    }));
    var rowwrap = el('span', 'apps-rowwrap');
    var row = el('span', 'apps-row');
    var verb = document.createTextNode('Ran ');
    var tool = el('code', null, 'ui_chart');
    row.appendChild(verb); row.appendChild(tool);
    rowwrap.appendChild(row);
    strip.appendChild(rowwrap);
    var chev = svg('0 0 24 24', ['m9 18 6-6-6-6'], {
      class: 'apps-chev', stroke: 'currentColor', 'stroke-width': '2', 'stroke-linecap': 'round', 'stroke-linejoin': 'round'
    });
    strip.appendChild(chev);
    win.appendChild(strip);

    stage.appendChild(win);
    root.appendChild(stage);

    // ── state helpers ──
    var STATES = ['is-quiet', 'is-chart', 'is-grown', 'is-ask', 'is-point', 'is-press', 'is-down', 'is-clear', 'is-split', 'is-out'];
    function on(c) { stage.classList.add(c); }
    function off(c) { stage.classList.remove(c); }

    // running: the tool call is in flight (coral rail); breathe: its text breathes.
    function setRow(v, t, running, breathe) {
      verb.nodeValue = v + ' ';
      tool.textContent = t;
      strip.classList.toggle('is-active', !!running);
      row.classList.toggle('is-running', !!running && breathe !== false);
    }
    var swapTimer = 0;
    function swapRow(v, t, running) {
      clearTimeout(swapTimer);
      rowwrap.classList.add('is-swap');
      swapTimer = setTimeout(function () {
        setRow(v, t, running);
        rowwrap.classList.remove('is-swap');
      }, 150);
    }

    // Place the pointer: resting low on the page, or over the submit button.
    function cursorAt(target) {
      var pr = page.getBoundingClientRect();
      var x, y;
      if (target === 'go') {
        var gr = go.getBoundingClientRect();
        x = gr.left - pr.left + gr.width * 0.84;
        y = gr.top - pr.top + gr.height * 0.62;
      } else {
        x = pr.width * 0.84;
        y = pr.height * 0.9;
      }
      cursor.style.setProperty('--cx', Math.round(x) + 'px');
      cursor.style.setProperty('--cy', Math.round(y) + 'px');
    }
    var aimed = 'rest';
    function aim(target) { aimed = target; cursorAt(target); }

    function jump(fn) {
      stage.classList.add('apps-instant');
      fn();
      void stage.offsetWidth;
      stage.classList.remove('apps-instant');
    }

    // First frame: the empty region, the tool row hidden until it fades in.
    function reset() {
      jump(function () {
        STATES.forEach(off);
        rowwrap.classList.remove('is-swap');
        setRow('Working on', 'ui_chart', true);
        on('is-quiet');
        aim('rest');
      });
    }

    // The still frame for reduced motion: the agent is asking, the answer is chosen.
    function still() {
      jump(function () {
        STATES.forEach(off);
        ['is-chart', 'is-grown', 'is-ask', 'is-point', 'is-press'].forEach(on);
        setRow('Working on', 'ui_ask', true, false);
        aim('go');
      });
    }

    if (window.ResizeObserver) {
      // Keep the pointer on its mark when the tile changes size; only the
      // pointer jumps, everything else keeps its transitions.
      new ResizeObserver(function () {
        cursor.style.transition = 'none';
        cursorAt(aimed);
        void cursor.offsetWidth;
        cursor.style.transition = '';
      }).observe(page);
    }

    if (opts && opts.reduced) {
      still();
      return { start: function () { still(); }, stop: function () {} };
    }

    // ── the loop (about 12.9 s) ──
    var STEPS = [
      [200, function () { off('is-quiet'); }],
      [900, function () { on('is-chart'); }],
      [1300, function () { on('is-grown'); }],
      [2500, function () { swapRow('Ran', 'ui_chart', false); }],
      [3400, function () { swapRow('Working on', 'ui_ask', true); }],
      [3800, function () { on('is-ask'); }],
      [4600, function () { on('is-point'); }],
      [4750, function () { aim('go'); }],
      [5500, function () { on('is-down'); on('is-press'); }],
      [5650, function () { off('is-down'); }],
      [6150, function () { off('is-ask'); off('is-point'); }],
      [6500, function () { swapRow('Ran', 'ui_ask', false); off('is-press'); aim('rest'); }],
      [7200, function () { swapRow('Working on', 'ui_chart', true); }],
      [7500, function () { on('is-clear'); }],
      [7800, function () { on('is-split'); }],
      [9400, function () { swapRow('Ran', 'ui_chart', false); }],
      [11700, function () { on('is-out'); }],
      [12900, null]
    ];

    var timer = 0, idx = 0, t0 = 0, running = false;
    function tick() {
      if (!running) return;
      var now = performance.now() - t0;
      while (idx < STEPS.length && STEPS[idx][0] <= now + 4) {
        var fn = STEPS[idx][1];
        idx++;
        if (!fn) { begin(); return; }
        fn();
      }
      if (idx < STEPS.length) timer = setTimeout(tick, Math.max(0, STEPS[idx][0] - (performance.now() - t0)));
    }
    function begin() {
      reset();
      idx = 0;
      t0 = performance.now();
      timer = setTimeout(tick, STEPS[0][0]);
    }

    reset();
    return {
      start: function () {
        if (running) return;
        running = true;
        off('is-paused');
        begin();
      },
      stop: function () {
        running = false;
        clearTimeout(timer);
        clearTimeout(swapTimer);
        timer = 0;
        on('is-paused');
      }
    };
  });
})();
