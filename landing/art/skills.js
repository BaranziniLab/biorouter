/* Skills tile: teaching the agent a method once.
   The person types "/skill:" in the composer and the slash picker opens on
   three installed skills in the app's own rows (MentionPopover: `skill:<name>`
   over the bundle count or the skill's description). The arrow moves to
   scientific-research, a standalone skill that loads by its exact name, and it
   becomes the composer's reference chip (ResourceRefChip). The message is sent
   with the chip inline. The agent's reply works through three stages of that
   skill's loop, named as the skill names them (biorouter-skills,
   scientific-research/SKILL.md: Scope, Literature review, Verification).
   Each stage is a tool-call line, as the transcript shows To Do progress:
   the app's own summaries ("Starting “Scope”", then "Marking “Scope”
   complete", ToolCallWithResponse's summarizeTodoCall), 13px muted text, the
   running line breathing as the app's does. One liberty: where the app draws
   the tool's glyph, each line carries the To Do list's status icon (dashed
   circle while it runs, a check once complete), so the stages visibly tick
   off. One thing moves at a time. Reduced motion shows the answered turn. */
(function () {
  'use strict';
  if (!window.BR || !BR.art) return;

  // Lucide paths, as the app draws them (stroke 1.5 on a 24 grid).
  var ICON = {
    layers: '<path d="M12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83z"/><path d="M2 12a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 12"/><path d="M2 17a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 17"/>',
    x: '<path d="M18 6 6 18"/><path d="m6 6 12 12"/>',
    up: '<path d="m5 12 7-7 7 7"/><path d="M12 19V5"/>',
    chev: '<path d="m9 18 6-6-6-6"/>',
    dashed: '<path d="M10.1 2.18a9.93 9.93 0 0 1 3.8 0"/><path d="M17.6 3.71a9.95 9.95 0 0 1 2.69 2.7"/><path d="M21.82 10.1a9.93 9.93 0 0 1 0 3.8"/><path d="M20.29 17.6a9.95 9.95 0 0 1-2.7 2.69"/><path d="M13.9 21.82a9.94 9.94 0 0 1-3.8 0"/><path d="M6.4 20.29a9.95 9.95 0 0 1-2.69-2.7"/><path d="M2.18 13.9a9.93 9.93 0 0 1 0-3.8"/><path d="M3.71 6.4a9.95 9.95 0 0 1 2.7-2.69"/><circle cx="12" cy="12" r="1"/>',
    check: '<path class="sk-tick" d="M20 6 9 17l-5-5" pathLength="1"/>'
  };
  function svg(name, cls, sw) {
    return '<svg class="' + cls + '" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="' + (sw || 1.5) +
      '" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">' + ICON[name] + '</svg>';
  }

  // The picker lists the catalog in name order. Bundles show their size; a
  // standalone skill shows its frontmatter description, truncated. `more`
  // marks a description quoted only up to where the real one continues, so it
  // always ends in an ellipsis.
  var ROWS = [
    { name: 'skill:differential-expression', desc: '7 skills in bundle' },
    { name: 'skill:scientific-research', desc: 'Run a research question end to end', more: true },
    { name: 'skill:single-cell', desc: '14 skills in bundle' }
  ];
  var PICK = 1;
  var SKILL = 'scientific-research';
  var QUERY = '/skill:';
  var PROMPT = 'Survey TREM2';
  // Three stages of the skill's loop, in its own words.
  var STEPS = ['Scope', 'Literature review', 'Verification'];

  function chip(removable) {
    return '<span class="sk-chip">' + svg('layers', 'sk-chip-ic', 1.75) + '<span class="sk-chip-name">' + SKILL + '</span>' +
      (removable ? svg('x', 'sk-chip-x', 2) : '') + '</span>';
  }

  BR.art('skills', function (root, opts) {
    var reduced = !!opts.reduced;
    var stage = document.createElement('div');
    stage.className = 'art-skills';
    stage.setAttribute('aria-hidden', 'true');

    var rowsHtml = ROWS.map(function (r, i) {
      return '<div class="sk-row" data-i="' + i + '">' + svg('layers', 'sk-row-ic') +
        '<span class="sk-row-text"><span class="sk-row-name">' + r.name + '</span><span class="sk-row-desc" data-full="' +
        r.desc + '"' + (r.more ? ' data-more="1"' : '') + '>' + r.desc + (r.more ? '…' : '') + '</span></span></div>';
    }).join('');

    // Each stage is one tool-call line. Its two summaries share one cell and
    // hand over in turn, each followed by the row's chevron.
    var callsHtml = STEPS.map(function (s, i) {
      return '<div class="sk-call" data-i="' + i + '" data-state="hidden">' +
        '<span class="sk-call-ic">' + svg('dashed', 'sk-i-active') + svg('check', 'sk-i-done', 1.75) + '</span>' +
        '<span class="sk-call-text">' +
          '<span class="sk-run"><span class="sk-t">Starting “' + s + '”</span>' + svg('chev', 'sk-chev') + '</span>' +
          '<span class="sk-done"><span class="sk-t">Marking “' + s + '” complete</span>' + svg('chev', 'sk-chev') + '</span>' +
        '</span></div>';
    }).join('');

    // The conversation is one column pinned above the composer: the person's
    // message, then the agent's reply. Room for the whole reply is held from
    // the start, so nothing above it moves as its lines arrive.
    stage.innerHTML =
      '<div class="sk-thread">' +
        '<div class="sk-user"><span class="sk-bubble"><span class="sk-utext">' + PROMPT + '</span> ' + chip(false) + '</span></div>' +
        '<div class="sk-reply">' + callsHtml + '</div>' +
      '</div>' +
      '<div class="sk-composer">' +
        '<div class="sk-picker"><div class="sk-found">' + ROWS.length + ' items found</div>' +
          '<div class="sk-list"><span class="sk-sel"></span>' + rowsHtml + '</div></div>' +
        '<div class="sk-refs"><div class="sk-refs-in">' + chip(true) + '</div></div>' +
        '<div class="sk-line">' +
          '<span class="sk-input"><span class="sk-ph">Ask Biorouter anything…</span><span class="sk-typed"></span><span class="sk-caret"></span></span>' +
          '<span class="sk-send">' + svg('up', 'sk-send-ic', 2) + '</span>' +
        '</div>' +
      '</div>';
    root.appendChild(stage);

    var q = function (s) { return stage.querySelector(s); };
    var qa = function (s) { return stage.querySelectorAll(s); };
    var typed = q('.sk-typed'), rows = qa('.sk-row'), sel = q('.sk-sel'), calls = qa('.sk-call');

    // Truncate the description lines the way the app's `truncate` does, but in
    // the text itself, so no text box ever runs past its row.
    function fitDescs() {
      qa('.sk-row-desc').forEach(function (el) {
        var full = el.getAttribute('data-full');
        var more = el.hasAttribute('data-more');
        el.textContent = full + (more ? '…' : '');
        if (el.scrollWidth <= el.clientWidth + 0.5) return;
        var words = full.split(' '), lo = 1, hi = words.length, best = words[0] + '…';
        while (lo <= hi) {
          var mid = (lo + hi) >> 1, t = words.slice(0, mid).join(' ').replace(/[,.;:]$/, '') + '…';
          el.textContent = t;
          if (el.scrollWidth <= el.clientWidth + 0.5) { best = t; lo = mid + 1; } else hi = mid - 1;
        }
        el.textContent = best;
      });
    }
    function placeSel(i) {
      var r = rows[i];
      sel.style.height = r.offsetHeight + 'px';
      sel.style.transform = 'translateY(' + r.offsetTop + 'px)';
    }
    var selected = 0;
    if ('ResizeObserver' in window) {
      new ResizeObserver(function () { fitDescs(); placeSel(selected); }).observe(stage);
    }

    function set(cls, on) { stage.classList.toggle(cls, !!on); }
    function select(i) { selected = i; placeSel(i); }
    function setTyped(s) { typed.textContent = s; set('has-text', s.length > 0); }
    function call(i, state) { calls[i].setAttribute('data-state', state); }

    // A reset lands with no transitions, then motion is switched back on.
    // `is-live` (the breathing of a running line) survives it.
    function reset() {
      var live = stage.classList.contains('is-live');
      stage.className = 'art-skills sk-instant' + (live ? ' is-live' : '');
      setTyped('');
      select(0);
      for (var i = 0; i < calls.length; i++) call(i, 'hidden');
      void stage.offsetWidth;
      requestAnimationFrame(function () { requestAnimationFrame(function () { stage.classList.remove('sk-instant'); }); });
    }

    // The answered turn: the still frame for reduced motion, and the frame
    // shown before the tile starts. The chip names the skill; the reply has
    // completed two stages and is running the third.
    function still() {
      reset();
      set('show-user', true);
      call(0, 'done'); call(1, 'done'); call(2, 'active');
    }

    // The loop, as a list of [time ms, action].
    function typeAt(t0, text, gap, out) {
      for (var k = 1; k <= text.length; k++) (function (k) { out.push([t0 + (k - 1) * gap, function () { setTyped(text.slice(0, k)); }]); })(k);
    }
    var plan = [];
    plan.push([0, reset]);
    // The focus edge turns accent while the person types the command.
    plan.push([100, function () { set('is-focus', true); set('is-edge', true); }]);
    typeAt(150, QUERY, 60, plan);
    // "/" alone would list every command; the picker is shown once "skill:"
    // has narrowed it to three, with the first row selected.
    plan.push([540, function () { set('show-picker', true); }]);
    plan.push([1500, function () { select(PICK); }]);
    // The selection becomes the chip. The edge returns to its hairline so the
    // chip and the armed Send carry the accent.
    plan.push([2900, function () { set('show-picker', false); setTyped(''); set('has-ref', true); set('is-edge', false); }]);
    typeAt(3150, PROMPT, 30, plan);
    plan.push([3560, function () { set('is-press', true); }]);
    plan.push([3680, function () {
      set('is-press', false); set('is-focus', false); set('has-ref', false); setTyped('');
      set('show-user', true);
    }]);
    // Each stage runs, settles, and the next line arrives a beat later.
    var STEP = 1500, BEAT = 300, FIRST = 3955;
    for (var s = 0; s < STEPS.length; s++) (function (s) {
      var at = FIRST + s * (STEP + BEAT);
      plan.push([at, function () { call(s, 'active'); }]);
      plan.push([at + STEP, function () { call(s, 'done'); }]);
    })(s);
    var LAST_DONE = FIRST + (STEPS.length - 1) * (STEP + BEAT) + STEP;
    var CLEAR_AT = LAST_DONE + 1300;
    plan.push([CLEAR_AT, function () { set('is-clearing', true); }]);
    var LOOP = CLEAR_AT + 500;
    plan.sort(function (a, b) { return a[0] - b[0]; });
    // On scroll-in the still frame holds briefly, then its running stage
    // completes and the loop carries on from there.
    var HOLD = 900;
    var START_AT = LAST_DONE - HOLD;

    var timer = null, t0 = 0, idx = 0, running = false;
    function tick() {
      if (!running) return;
      var now = performance.now() - t0;
      while (idx < plan.length && plan[idx][0] <= now) { plan[idx][1](); idx++; }
      if (idx >= plan.length && now >= LOOP) { t0 += LOOP; idx = 0; tick(); return; }
      var next = idx < plan.length ? plan[idx][0] : LOOP;
      timer = setTimeout(tick, Math.max(0, next - now));
    }

    fitDescs();
    if (reduced) { still(); return { start: function () {}, stop: function () {} }; }
    still();

    return {
      start: function () {
        if (running) return;
        running = true;
        set('is-live', true);
        t0 = performance.now() - START_AT;
        idx = 0;
        while (idx < plan.length && plan[idx][0] <= START_AT) idx++;
        tick();
      },
      stop: function () {
        running = false;
        if (timer) { clearTimeout(timer); timer = null; }
        set('is-live', false);
        still(); // off screen by now; the next start picks up from here
      }
    };
  });
})();
