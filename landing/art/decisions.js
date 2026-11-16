/* Decision models tile: screening many notes with one decide call.
   The person asks which of 30 notes record a relapse. The agent answers with
   one `decisions__decide` tool row, drawn as the transcript draws it: the
   decision glyph (ENTITY_ICONS.decision, Lucide Gauge), the app's summary
   ("Checking 30 chunks against 1 question" while it runs, breathing as a
   running row does, then "Checked 30 chunks against 1 question" with the
   meta "9 yes, 8 escalated"; summarizeDecideCall and decideCallMeta in
   DecideToolSummary.tsx), and its chevron. The row is opened, as a person
   opens it: while it runs the well names only the question it asked. Once
   the batch answers, the well holds what DecideToolSummary draws for a call
   with `escalate`: the first model and where it runs ("joey-1 · Baseten",
   decisionModelDisplay; a Baseten model is public, so no padlock), the
   escalation line ("8 of 30 escalated to elephant-1" and its InfoTip), the
   question in plain words with its counts ("9 yes · 21 no", the final
   answers, decideCountsLine), then the Escalated section, each chunk by its
   number in the call with the final answer beside the first model's
   ("yes 0.94, was no 0.55 by joey-1", DECIDE_TOOL_COPY.escalatedAnswer).
   The two models are real Baseten aliases (providers/decision/catalog.rs);
   the escalation is one call, as docs/agent-loop/decision-models.md
   ("Escalating the unsure answers") describes it.
   The transcript is pinned to the frame's bottom and runs out under a fade
   at the top, as a chat scrolls. The Escalated list runs on past the frame's
   bottom edge under a second fade: the card lists all eight, and the tile
   shows the first. One liberty: the app draws the well's lines at once when
   the result lands; here they arrive one after another. One thing moves at
   a time. Reduced motion shows the answered call. */
(function () {
  'use strict';
  if (!window.BR || !BR.art) return;

  // Lucide paths, as the app draws them (stroke 1.5 on a 24 grid).
  var ICON = {
    // The decision glyph: Lucide Gauge (components/icons/entity-icons.ts).
    gauge: '<path d="m12 14 4-4"/><path d="M3.34 19a10 10 0 1 1 17.32 0"/>',
    chev: '<path d="m9 18 6-6-6-6"/>',
    // The InfoTip's glyph (components/ui/info-tip.tsx).
    info: '<circle cx="12" cy="12" r="10"/><path d="M12 16v-4"/><path d="M12 8h.01"/>'
  };
  function svg(name, cls, sw) {
    return '<svg class="' + cls + '" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="' + (sw || 1.5) +
      '" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">' + ICON[name] + '</svg>';
  }

  // The call: 30 chunks, one yes or no question, joey-1 first, its unsure
  // answers (a top probability under 0.80) asked again of elephant-1.
  var PROMPT = 'Which of 30 notes record a relapse?';
  var QUESTION = 'Does the note record a relapse?';
  var CHUNKS = 30;
  var YES = 9;
  var ESCALATED = 8;
  var FIRST = 'joey-1';
  var SECOND = 'elephant-1';
  // Three of the eight chunks asked again; the first is the one the tile shows.
  // [place in the call, final answer, its P, first answer, its P]
  var ASKED_AGAIN = [
    [4, 'yes', '0.94', 'no', '0.55'],
    [11, 'no', '0.91', 'yes', '0.58'],
    [17, 'yes', '0.88', 'yes', '0.64']
  ];

  // The app's words for these numbers (DECIDE_TOOL_COPY and the functions
  // beside it in DecideToolSummary.tsx; the settled verb from toolCallCopy.ts).
  var RUNNING = 'Checking ' + CHUNKS + ' chunks against 1 question';
  var SETTLED = 'Checked ' + CHUNKS + ' chunks against 1 question';
  var META = '· ' + YES + ' yes, ' + ESCALATED + ' escalated';
  var HEAD = FIRST + ' · Baseten';
  var ESCALATION = ESCALATED + ' of ' + CHUNKS + ' escalated to ' + SECOND;
  var COUNTS = YES + ' yes · ' + (CHUNKS - YES) + ' no';
  function answerText(a) { return a[1] + ' ' + a[2] + ', was ' + a[3] + ' ' + a[4] + ' by ' + FIRST; }

  BR.art('decisions', function (root, opts) {
    var reduced = !!opts.reduced;
    var stage = document.createElement('div');
    stage.className = 'art-decisions';
    stage.setAttribute('aria-hidden', 'true');

    var rowsHtml = ASKED_AGAIN.map(function (a) {
      return '<div class="dm-ask"><span class="dm-chunk">Chunk ' + a[0] + '</span><span class="dm-ans">' + answerText(a) + '</span></div>';
    }).join('');
    // A part of the well that arrives with the result: its room opens, then
    // its words fade in.
    function part(cls, inner) { return '<div class="dm-open ' + cls + '"><div class="dm-in">' + inner + '</div></div>'; }

    stage.innerHTML =
      '<div class="dm-thread"><div class="dm-col">' +
        '<div class="dm-user"><span class="dm-bubble">' + PROMPT + '</span></div>' +
        '<div class="dm-row" data-state="hidden">' +
          // The row's head: glyph, the two summaries sharing one cell, the
          // meta once answered, the chevron.
          '<div class="dm-line">' +
            '<span class="dm-glyph">' + svg('gauge', 'dm-glyph-ic') + '</span>' +
            // The meta rides with the settled summary, so it sits one word
            // space after that text whatever the running one measured.
            '<span class="dm-label"><span class="dm-run"><span class="dm-t">' + RUNNING + '</span></span>' +
              '<span class="dm-done"><span class="dm-dt">' + SETTLED + '</span>' +
              '<span class="dm-meta"><span class="dm-meta-in">' + META + '</span></span></span></span>' +
            svg('chev', 'dm-chev') +
          '</div>' +
          '<div class="dm-body"><div class="dm-body-in"><div class="dm-well">' +
            part('dm-head', '<div class="dm-l">' + HEAD + '</div>') +
            part('dm-esc', '<div class="dm-l">' + ESCALATION + svg('info', 'dm-info') + '</div>') +
            '<div class="dm-q"><div class="dm-qtext">' + QUESTION + '</div>' +
              part('dm-counts', '<div class="dm-l">' + COUNTS + '</div>') +
            '</div>' +
            part('dm-sec', '<div class="dm-sec-label">Escalated</div><div class="dm-asks">' + rowsHtml + '</div>') +
          '</div></div></div>' +
        '</div>' +
      '</div></div>';
    root.appendChild(stage);

    var q = function (s) { return stage.querySelector(s); };
    var row = q('.dm-row'), line = q('.dm-line'), glyph = q('.dm-glyph');
    var runText = q('.dm-run .dm-t'), doneText = q('.dm-dt');
    var metaIn = q('.dm-meta-in'), well = q('.dm-well'), asks = q('.dm-asks');

    // Width: the row's meta gives way first, then its chevron, so the summary
    // keeps every word. Only on the narrowest tile, where the summary alone is
    // wider than the line, does it truncate as the app's row does (CSS); its
    // full width is read from scrollWidth there.
    // Height: the Escalated list hangs below the frame by all but its first
    // chunk, so that chunk sits on the bottom edge and the next one runs out
    // under the fade.
    function fit() {
      line.classList.remove('dm-fit-meta', 'dm-fit-chev');
      var avail = line.clientWidth;
      var lead = glyph.offsetWidth + 8;
      var run = Math.max(runText.offsetWidth, runText.scrollWidth);
      var done = Math.max(doneText.offsetWidth, doneText.scrollWidth);
      var chev = 22; // 16px glyph and its 6px gap
      var showChev = lead + Math.max(run, done) + chev <= avail + 0.5;
      var showMeta = lead + Math.max(run, done + metaIn.scrollWidth) + (showChev ? chev : 0) <= avail + 0.5;
      line.classList.toggle('dm-fit-chev', !showChev);
      line.classList.toggle('dm-fit-meta', !showMeta);
      var first = asks.firstChild;
      var cs = getComputedStyle(well);
      var hang = asks.offsetHeight - (first.offsetTop + first.offsetHeight) +
        parseFloat(cs.paddingBottom) + parseFloat(cs.borderBottomWidth);
      stage.style.setProperty('--dm-hang', Math.max(0, Math.round(hang)) + 'px');
    }
    if ('ResizeObserver' in window) new ResizeObserver(fit).observe(stage);

    function set(cls, on) { stage.classList.toggle(cls, !!on); }
    function state(s) { row.setAttribute('data-state', s); }

    // A reset lands with no transitions, then motion is switched back on.
    // `is-live` (the breathing of a running row) survives it.
    function reset() {
      var live = stage.classList.contains('is-live');
      stage.className = 'art-decisions dm-instant' + (live ? ' is-live' : '');
      state('hidden');
      void stage.offsetWidth;
      requestAnimationFrame(function () { requestAnimationFrame(function () { stage.classList.remove('dm-instant'); }); });
    }
    function answered() {
      set('show-head', true); set('show-esc', true); set('show-counts', true); set('show-sec', true);
    }

    // The answered call: the still frame for reduced motion, and the frame
    // shown before the tile starts.
    function still() {
      reset();
      set('show-user', true);
      state('done');
      set('is-open', true);
      answered();
    }

    // The loop, as a list of [time ms, action].
    var plan = [];
    plan.push([0, reset]);
    plan.push([300, function () { set('show-user', true); }]);
    plan.push([1000, function () { state('running'); }]);
    // The person opens the running row: the well names the question it asked.
    plan.push([1800, function () { set('is-open', true); }]);
    // The batch answers: the summary takes its settled form and the meta.
    plan.push([3500, function () { state('done'); }]);
    // The well's lines, one after another.
    plan.push([4100, function () { set('show-head', true); }]);
    plan.push([4800, function () { set('show-esc', true); }]);
    plan.push([5600, function () { set('show-counts', true); }]);
    var LAST_PART = 6500;
    plan.push([LAST_PART, function () { set('show-sec', true); }]);
    var CLEAR_AT = 10000;
    plan.push([CLEAR_AT, function () { set('is-clearing', true); }]);
    var LOOP = CLEAR_AT + 600;
    plan.sort(function (a, b) { return a[0] - b[0]; });
    // On scroll-in the answered call holds while it can be read, then the
    // column clears and the loop starts over.
    var HOLD = 1500;
    var START_AT = CLEAR_AT - HOLD;

    var timer = null, t0 = 0, idx = 0, running = false;
    function tick() {
      if (!running) return;
      var now = performance.now() - t0;
      while (idx < plan.length && plan[idx][0] <= now) { plan[idx][1](); idx++; }
      if (idx >= plan.length && now >= LOOP) { t0 += LOOP; idx = 0; tick(); return; }
      var next = idx < plan.length ? plan[idx][0] : LOOP;
      timer = setTimeout(tick, Math.max(0, next - now));
    }

    fit();
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
