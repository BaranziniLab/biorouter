/* Workflows tile. A workflow file is written key by key, its parameter is
   filled in, Start is pressed, and its three steps complete in order. Then the
   same file runs again, the same way. One thing moves at a time. */
(function () {
  'use strict';
  if (!window.BR || !BR.art) return;

  var NS = 'http://www.w3.org/2000/svg';

  // Real schema keys (docs/workflows/workflow-schema-reference.md). Each entry
  // is [className, text]; k = key, p = punctuation, v = value, c = comment.
  // The prompt is what starts the run (without one the agent waits for
  // input), and its one parameter is declared and used, as
  // validate_parameters_in_template requires. The extension is CDWAgent, a
  // private UCSF data extension, and the provider is Llama Server, a private
  // Local model, so the extension is allowed to it. With no model named it
  // runs that provider's own default.
  // The file is an excerpt, and says so in a comment on its first line: the
  // required description, and the parameter's and extension's other fields
  // (input_type, requirement, type and the like) are left out, so the tile
  // stays seven short lines. The longest line is 32 characters, so every
  // line fits every tile at 11px.
  var LINES = [
    [['k', 'title'], ['p', ': '], ['v', 'Statin cohort'], ['c', '  # excerpt']],
    [['k', 'prompt'], ['p', ': '], ['v', 'Plot LDL in {{ '], ['v wf-tpl', 'cohort'], ['v', ' }}']],
    [['k', 'parameters'], ['p', ': [{'], ['k', 'key'], ['p', ': '], ['v', 'cohort'], ['p', '}]']],
    [['k', 'extensions'], ['p', ': [{'], ['k', 'name'], ['p', ': '], ['v', 'cdwagent'], ['p', '}]']],
    [['k', 'skills'], ['p', ': ['], ['v', 'clinical-biostatistics'], ['p', ']']],
    [['k', 'settings'], ['p', ':']],
    [['p', '  '], ['k', 'biorouter_provider'], ['p', ': '], ['v', 'llamacpp']]
  ];
  var STEPS = ['Query', 'Analysis', 'Report'];
  // Lucide CircleDotDashed, the app's In progress icon.
  var DASHED = [
    ['path', { d: 'M10.1 2.18a9.93 9.93 0 0 1 3.8 0' }],
    ['path', { d: 'M17.6 3.71a9.95 9.95 0 0 1 2.69 2.7' }],
    ['path', { d: 'M21.82 10.1a9.93 9.93 0 0 1 0 3.8' }],
    ['path', { d: 'M20.29 17.6a9.95 9.95 0 0 1-2.7 2.69' }],
    ['path', { d: 'M13.9 21.82a9.94 9.94 0 0 1-3.8 0' }],
    ['path', { d: 'M6.4 20.29a9.95 9.95 0 0 1-2.69-2.7' }],
    ['path', { d: 'M2.18 13.9a9.93 9.93 0 0 1 0-3.8' }],
    ['path', { d: 'M3.71 6.4a9.95 9.95 0 0 1 2.7-2.69' }],
    ['circle', { cx: '12', cy: '12', r: '1' }]
  ];
  var VALUE = 'T2D';

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
    s.setAttribute('stroke', 'currentColor');
    s.setAttribute('stroke-linecap', 'round');
    s.setAttribute('stroke-linejoin', 'round');
    s.setAttribute('stroke-width', (attrs && attrs.sw) || '1.5');
    s.setAttribute('aria-hidden', 'true');
    paths.forEach(function (p) {
      var node = document.createElementNS(NS, p[0]);
      for (var k in p[1]) node.setAttribute(k, p[1][k]);
      s.appendChild(node);
    });
    return s;
  }

  BR.art('workflows', function (root, opts) {
    var stage = el('div', 'wf');
    stage.setAttribute('aria-hidden', 'true');
    var inner = el('div', 'wf-in');
    stage.appendChild(inner);

    // The file: a header with the app's Workflow icon and the file name.
    var file = el('div', 'wf-file');
    var head = el('div', 'wf-head');
    head.appendChild(svg('0 0 24 24', [
      ['rect', { width: '8', height: '8', x: '3', y: '3', rx: '2' }],
      ['path', { d: 'M7 11v4a2 2 0 0 0 2 2h4' }],
      ['rect', { width: '8', height: '8', x: '13', y: '13', rx: '2' }]
    ]));
    head.appendChild(el('span', 'wf-name', 'statin-cohort.yaml'));
    file.appendChild(head);

    var body = el('div', 'wf-body');
    var linesBox = el('div', 'wf-lines');
    var rail = el('i', 'wf-rail');
    linesBox.appendChild(rail);
    var lines = LINES.map(function (parts) {
      var line = el('span', 'wf-line');
      parts.forEach(function (p) { line.appendChild(el('span', p[0], p[1])); });
      linesBox.appendChild(line);
      return line;
    });
    // The name inside {{ cohort }}: underlined while its value is typed, so
    // the field below reads as this file's parameter.
    var tpl = linesBox.querySelector('.wf-tpl');
    body.appendChild(linesBox);
    file.appendChild(body);
    inner.appendChild(file);

    // The run: the parameter field and Start on one row, the steps below.
    var run = el('div', 'wf-run');
    var row = el('div', 'wf-row');
    row.appendChild(el('span', 'wf-key', 'cohort'));
    var field = el('span', 'wf-field');
    var val = el('span', 'wf-val', '');
    var caret = el('i', 'wf-caret');
    field.appendChild(val);
    field.appendChild(caret);
    row.appendChild(field);
    var btn = el('span', 'wf-btn');
    btn.appendChild(svg('0 0 12 12', [['path', { d: 'M3.5 2.2v7.6L9.6 6z', fill: 'currentColor' }]], { sw: '1.2' }));
    // The app's parameter dialog button reads "Start workflow".
    btn.appendChild(el('span', null, 'Start'));
    row.appendChild(btn);
    run.appendChild(row);

    var stepsBox = el('div', 'wf-steps');
    var steps = [], links = [];
    STEPS.forEach(function (label, i) {
      if (i > 0) {
        var link = el('span', 'wf-link');
        link.appendChild(el('b'));
        stepsBox.appendChild(link);
        links.push(link);
      }
      var step = el('span', 'wf-step');
      var ico = el('span', 'wf-ico');
      var pending = svg('0 0 24 24', [['circle', { cx: '12', cy: '12', r: '10' }]]);
      var active = svg('0 0 24 24', DASHED);
      var done = svg('0 0 24 24', [['path', { d: 'M20 6 9 17l-5-5' }]], { sw: '2' });
      pending.setAttribute('class', 'i-pending');
      active.setAttribute('class', 'i-active');
      done.setAttribute('class', 'i-done');
      ico.appendChild(pending); ico.appendChild(active); ico.appendChild(done);
      step.appendChild(ico);
      step.appendChild(el('span', null, label));
      stepsBox.appendChild(step);
      steps.push(step);
    });
    run.appendChild(stepsBox);
    inner.appendChild(run);
    root.appendChild(stage);

    function cls(node, name, on) { node.classList.toggle(name, !!on); }
    // The rail follows the line's own box.
    function setRail(line) { rail.style.transform = 'translateY(' + (line ? line.offsetTop : 0) + 'px)'; }

    // Instant resets. `still` suspends transitions for one frame.
    function still(fn) {
      stage.classList.add('is-still');
      fn();
      void stage.offsetWidth;
      stage.classList.remove('is-still');
    }
    function clearRun() {
      cls(stage, 'is-clearing', false);
      val.textContent = '';
      cls(field, 'is-focus', false);
      cls(tpl, 'is-on', false);
      cls(btn, 'is-armed', false);
      cls(btn, 'is-pressed', false);
      steps.forEach(function (s) { cls(s, 'is-active', false); cls(s, 'is-done', false); });
      links.forEach(function (l) { cls(l, 'is-filled', false); });
    }
    function clearFile() {
      lines.forEach(function (l) { cls(l, 'is-in', false); });
      cls(rail, 'is-on', false);
      setRail(null);
    }
    function finished() {
      lines.forEach(function (l) { cls(l, 'is-in', true); });
      val.textContent = VALUE;
      steps.forEach(function (s) { cls(s, 'is-done', true); });
      links.forEach(function (l) { cls(l, 'is-filled', true); });
    }

    if (opts && opts.reduced) {
      still(finished);
      stage.classList.add('is-still');
      return { start: function () {}, stop: function () {} };
    }

    var timers = [];
    function at(ms, fn) { timers.push(setTimeout(fn, ms)); }
    function cancel() { timers.forEach(clearTimeout); timers = []; }

    var LINE_MS = 420;

    // Write the file, one key at a time, with the rail beside the new line.
    function write(t0) {
      lines.forEach(function (line, i) {
        at(t0 + i * LINE_MS, function () {
          setRail(line);
          if (i === 0) cls(rail, 'is-on', true);
          cls(line, 'is-in', true);
        });
      });
      var end = t0 + (lines.length - 1) * LINE_MS + 380;
      at(end, function () { cls(rail, 'is-on', false); });
      return end + 520;
    }

    // Fill the parameter, press Start, complete the steps. Returns the end time.
    function runOnce(t) {
      at(t, function () { cls(field, 'is-focus', true); cls(tpl, 'is-on', true); });
      for (var c = 1; c <= VALUE.length; c++) {
        (function (c) { at(t + 260 + (c - 1) * 170, function () { val.textContent = VALUE.slice(0, c); }); })(c);
      }
      at(t + 1050, function () { cls(field, 'is-focus', false); cls(tpl, 'is-on', false); });
      at(t + 1350, function () { cls(btn, 'is-armed', true); });
      at(t + 1950, function () { cls(btn, 'is-pressed', true); });
      at(t + 2075, function () { cls(btn, 'is-pressed', false); });
      at(t + 2300, function () { cls(btn, 'is-armed', false); });

      var s = t + 2550, STEP = 750, LINK = 480;
      steps.forEach(function (step, i) {
        var a = s + i * (STEP + LINK);
        at(a, function () { cls(step, 'is-active', true); });
        at(a + STEP, function () { cls(step, 'is-active', false); cls(step, 'is-done', true); });
        if (links[i]) at(a + STEP, function () { cls(links[i], 'is-filled', true); });
      });
      return s + steps.length * STEP + (steps.length - 1) * LINK;
    }

    function cycle(t) {
      var end = runOnce(t);
      var hold = end + 2200;
      at(hold, function () { cls(stage, 'is-clearing', true); });
      at(hold + 600, function () { still(clearRun); });
      at(hold + 1000, function () { cancel(); cycle(0); });
    }

    function shown() {
      return !!val.textContent || lines.some(function (l) { return l.classList.contains('is-in'); });
    }
    function start() {
      cancel();
      if (!shown()) { still(clearRun); cycle(write(350)); return; }
      // Coming back to a tile that was left mid-loop: fade it out, then begin.
      cls(stage, 'is-clearing', true);
      cls(rail, 'is-on', false);
      cls(field, 'is-focus', false);
      cls(tpl, 'is-on', false);
      cls(btn, 'is-armed', false);
      cls(btn, 'is-pressed', false);
      steps.forEach(function (s) { cls(s, 'is-active', false); });
      lines.forEach(function (l) { cls(l, 'is-in', false); });
      at(600, function () {
        still(function () { clearRun(); clearFile(); });
        cycle(write(300));
      });
    }
    function stop() {
      cancel();
      cls(field, 'is-focus', false);
      cls(tpl, 'is-on', false);
    }

    // Before the tile is first seen it rests on the empty file, which is where
    // the loop begins, so the first start does not jump.
    still(function () { clearRun(); clearFile(); });
    return { start: start, stop: stop };
  });
})();
