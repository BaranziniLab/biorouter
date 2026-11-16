/* Agents tile: handing parts of a task to subagents that run at the same time.
   The agent delegates three papers, one per subagent. Each delegation is a
   tool-call line in the transcript, labelled as the app labels a `subagent`
   call when the parent gives a role ("Delegating to Reader 1: Read paper 1",
   ToolCallWithResponse's summarizeDelegation), with the app's Agent glyph
   (icons/app-icons.tsx, the `agent` entry toolGlyph.ts gives
   workspace__subagent), 13px muted text, the running line breathing as the
   app's does.
   Under it, the Chat summary's Agents section (summary/SummarySubagents.tsx):
   the heading "Agents" with its count ("1 running · 0 of 1", ListCount over
   summaryCopy.todoCount), and with four agents or fewer one row per agent
   (agentGrid.ts, AGENT_ROWS_MAX): the agent's mark (AgentMark.tsx, a square
   for a language model, 14px: hollow while it waits to start, a core in the
   info ink while it runs, solid in the calm sage once done, never red), its
   role, the run time on the right (formatElapsed), and one line of what it is
   doing: "Waiting to start", "Working…", then the first line of its result
   (subagentLine). The section appears with the first subagent, as the app's
   does, and the count grows as each one starts. The agent's answer follows.
   Liberties, all in the cause of fitting a small tile:
   1. The three delegation lines swap in place in one slot; the transcript
      stacks them.
   2. The run times are time compressed: each 400ms counts a second.
   3. The rows leave out the model name. The app writes the model's short
      name before the second line ("<model> · Working…"), and the site names
      no model versions; a row whose model is not recorded reads exactly as
      these do.
   4. The heading is in sentence case. The app sets the section label in
      caps; the site has no caps labels.
   One thing moves at a time. Reduced motion shows the answered turn. */
(function () {
  'use strict';
  if (!window.BR || !BR.art) return;

  // Lucide paths and the app's hand-drawn glyphs (stroke 1.5 on a 24 grid).
  var ICON = {
    // The app's Agent glyph (components/icons/app-icons.tsx).
    agent: '<rect x="4.5" y="8.5" width="15" height="11" rx="3.5"/><path d="M12 8.5V5.75"/><circle cx="12" cy="4.25" r="1.25"/><path d="M9.5 13v1.5M14.5 13v1.5"/>',
    chev: '<path d="m9 18 6-6-6-6"/>',
    // The section's open chevron (ChevronRight turned a quarter, as the app turns it).
    down: '<path d="m6 9 6 6 6-6"/>'
  };
  function svg(name, cls, sw) {
    return '<svg class="' + cls + '" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="' + (sw || 1.5) +
      '" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">' + ICON[name] + '</svg>';
  }

  // The agent mark, as AgentMark.tsx draws a square in its 12-unit box: the
  // ring (waiting to start), the ring with a core (running), the solid square
  // (done). All three are drawn; the row's state shows one.
  var MARK =
    '<svg class="ag-mark" viewBox="0 0 12 12" aria-hidden="true" focusable="false">' +
      '<rect class="ag-ring" x="0.75" y="0.75" width="10.5" height="10.5" rx="2"/>' +
      '<rect class="ag-core" x="3.5" y="3.5" width="5" height="5" rx="1"/>' +
      '<path class="ag-solid" d="M2.5 0H9.5A2.5 2.5 0 0 1 12 2.5V9.5A2.5 2.5 0 0 1 9.5 12H2.5A2.5 2.5 0 0 1 0 9.5V2.5A2.5 2.5 0 0 1 2.5 0Z"/>' +
    '</svg>';

  // Three readers, one paper each. Each result is the first line the
  // subagent's result would carry, short enough for the row at every size.
  var AGENTS = [
    { role: 'Reader 1', task: 'Read paper 1', result: 'Longer survival in 1,204 patients.' },
    { role: 'Reader 2', task: 'Read paper 2', result: '31% response in 412 patients.' },
    { role: 'Reader 3', task: 'Read paper 3', result: 'No clear benefit in 88 patients.' }
  ];
  var ANSWER = 'Two of the three studies favor the drug.';

  // The timeline (ms). Each subagent is delegated, appears waiting to start,
  // then runs; they finish in their own order.
  var DELEGATE = [400, 1700, 3000];
  var OPEN = [900, 2200, 3500];
  var RUN = [1300, 2600, 3900];
  var DONE = [6900, 5800, 7900];
  var ANSWER_AT = 8700;
  var CLEAR_AT = 10200;
  var LOOP = 10800;
  var TICK = 400; // one second of run time

  BR.art('agents', function (root, opts) {
    var reduced = !!opts.reduced;
    var stage = document.createElement('div');
    stage.className = 'art-agents';
    stage.setAttribute('aria-hidden', 'true');

    var linesHtml = AGENTS.map(function (a, i) {
      return '<div class="ag-line" data-i="' + i + '">' + svg('agent', 'ag-line-ic') +
        '<span class="ag-line-text"><span class="ag-t">Delegating to ' + a.role + ': ' + a.task + '</span>' +
        svg('chev', 'ag-chev') + '</span></div>';
    }).join('') +
      '<div class="ag-line ag-answer" data-i="' + AGENTS.length + '"><span class="ag-t">' + ANSWER + '</span></div>';

    var rowsHtml = AGENTS.map(function (a, i) {
      return '<div class="ag-row" data-i="' + i + '" data-state="hidden"><div class="ag-row-in"><div class="ag-row-box">' +
        '<span class="ag-glyph">' + MARK + '</span>' +
        '<span class="ag-text">' +
          '<span class="ag-name-line"><span class="ag-name">' + a.role + '</span><span class="ag-time"></span></span>' +
          '<span class="ag-meta">' +
            '<span class="ag-m ag-m-wait">Waiting to start</span>' +
            '<span class="ag-m ag-m-work">Working…</span>' +
            '<span class="ag-m ag-m-res">' + a.result + '</span>' +
          '</span>' +
        '</span>' +
      '</div></div></div>';
    }).join('');

    // One column: the transcript line on top, the Agents section under it.
    stage.innerHTML =
      '<div class="ag-col">' +
        '<div class="ag-slot">' + linesHtml + '</div>' +
        '<div class="ag-card">' +
          '<div class="ag-head">' +
            '<span class="ag-head-chev">' + svg('down', 'ag-head-ic') + '</span>' +
            '<span class="ag-label">Agents</span>' +
            '<span class="ag-count"></span>' +
          '</div>' +
          '<div class="ag-rows">' + rowsHtml + '</div>' +
        '</div>' +
      '</div>';
    root.appendChild(stage);

    var q = function (s) { return stage.querySelector(s); };
    var qa = function (s) { return stage.querySelectorAll(s); };
    var lines = qa('.ag-line'), rows = qa('.ag-row'), times = qa('.ag-time'), count = q('.ag-count');

    function set(cls, on) { stage.classList.toggle(cls, !!on); }

    // The slot shows one line at a time; `run` makes it breathe.
    function line(i, run) {
      for (var k = 0; k < lines.length; k++) {
        lines[k].classList.toggle('is-on', k === i);
        lines[k].classList.toggle('is-run', k === i && !!run);
      }
    }
    function settle() { for (var k = 0; k < lines.length; k++) lines[k].classList.remove('is-run'); }

    // The section's count, as ListCount writes it: "2 running · 1 of 3".
    function recount() {
      var total = 0, running = 0, done = 0;
      for (var k = 0; k < rows.length; k++) {
        var s = rows[k].getAttribute('data-state');
        if (s === 'hidden') continue;
        total++;
        if (s === 'running') running++;
        if (s === 'done') done++;
      }
      count.textContent = (running ? running + ' running · ' : '') + done + ' of ' + total;
      set('has-agents', total > 0);
    }
    function row(i, state) { rows[i].setAttribute('data-state', state); recount(); }
    function elapsed(i, s) { times[i].textContent = s; }

    // A reset lands with no transitions, then motion is switched back on.
    // `is-live` (the breathing and the pulse) survives it.
    function reset() {
      var live = stage.classList.contains('is-live');
      stage.className = 'art-agents ag-instant' + (live ? ' is-live' : '');
      line(-1);
      for (var k = 0; k < rows.length; k++) { rows[k].setAttribute('data-state', 'hidden'); elapsed(k, ''); }
      recount();
      void stage.offsetWidth;
      requestAnimationFrame(function () { requestAnimationFrame(function () { stage.classList.remove('ag-instant'); }); });
    }

    // The loop, as a list of [time ms, action].
    var plan = [];
    plan.push([0, reset]);
    for (var a = 0; a < AGENTS.length; a++) (function (i) {
      plan.push([DELEGATE[i], function () { line(i, true); }]);
      plan.push([OPEN[i], function () { row(i, 'queued'); }]);
      // The call returns once the subagent has started, so its line settles.
      plan.push([RUN[i], function () { row(i, 'running'); settle(); }]);
      // The run time counts up while it runs and stops when it ends.
      var ticks = Math.floor((DONE[i] - RUN[i]) / TICK);
      for (var k = 0; k <= ticks; k++) (function (k) {
        plan.push([RUN[i] + k * TICK, function () { elapsed(i, k + 's'); }]);
      })(k);
      plan.push([DONE[i], function () { row(i, 'done'); }]);
    })(a);
    plan.push([ANSWER_AT, function () { line(AGENTS.length, false); }]);
    plan.push([CLEAR_AT, function () { set('is-clearing', true); }]);
    // Ties keep their written order, so a row starts before its first second shows.
    plan.forEach(function (p, n) { p.push(n); });
    plan.sort(function (x, y) { return x[0] - y[0] || x[2] - y[2]; });

    // Draw the frame at time t at once: the reset, then every action up to t,
    // all under the instant class.
    function seek(t) {
      for (var n = 0; n < plan.length && plan[n][0] <= t; n++) plan[n][1]();
    }
    // On scroll-in the frame holds briefly, then the last reader finishes and
    // the loop carries on from there. Reduced motion shows the answered turn.
    var HOLD = 900;
    var START_AT = DONE[2] - HOLD;
    function still() { seek(START_AT); }
    function answered() { seek(CLEAR_AT - 1); }

    var timer = null, t0 = 0, idx = 0, running = false;
    function tick() {
      if (!running) return;
      var now = performance.now() - t0;
      while (idx < plan.length && plan[idx][0] <= now) { plan[idx][1](); idx++; }
      if (idx >= plan.length && now >= LOOP) { t0 += LOOP; idx = 0; tick(); return; }
      var next = idx < plan.length ? plan[idx][0] : LOOP;
      timer = setTimeout(tick, Math.max(0, next - now));
    }

    if (reduced) { answered(); return { start: function () {}, stop: function () {} }; }
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
