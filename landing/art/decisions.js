/* Decision models tile: screening many notes with one decide call.
   The person asks which of 30 notes record a relapse. The agent answers with
   one `decisions__decide` tool row, drawn as the transcript draws it: the
   decision glyph (ENTITY_ICONS.decision, Lucide Gauge), the app's summary
   ("Checking 30 chunks against 1 question" while it runs, breathing as a
   running row does, then "Checked 30 chunks against 1 question" with the
   meta "9 yes, 1 escalated"; summarizeDecideCall and decideCallMeta in
   DecideToolSummary.tsx), and its chevron. The row is opened, as a person
   opens it: while it runs the well names only the question it asked. Once
   the batch answers, the well holds what DecideToolSummary draws for a call
   with `escalate`: the first model and where it runs ("joey-1 · Baseten",
   decisionModelDisplay; a Baseten model is public, so no padlock), the
   escalation line ("1 of 30 escalated to elephant-1" and its InfoTip), the
   question in plain words with its counts ("9 yes · 21 no", the final
   answers, decideCountsLine), then the Escalated section: the one chunk
   asked again, by its number in the call, with the final answer beside the
   first model's ("yes 0.94, was no 0.55 by joey-1",
   DECIDE_TOOL_COPY.escalatedAnswer). The card lists every escalated chunk up
   to ten (DECIDE_CARD_ESCALATED_SHOWN), so a call that escalated one shows
   one row, and the whole well fits the frame.
   The two models are real Baseten aliases (providers/decision/catalog.rs);
   the escalation is one call, as docs/agent-loop/decision-models.md
   ("Escalating the unsure answers") describes it.
   The transcript is pinned to the frame's bottom and runs out under a fade
   at the top, as a chat scrolls. One liberty: the app draws the well's lines
   at once when the result lands; here they arrive one after another. One
   thing moves at a time. Reduced motion shows the answered call. */
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

  // The call: 30 chunks, one yes or no question, joey-1 first, its one
  // unsure answer (a top probability under 0.80) asked again of elephant-1.
  var PROMPT = 'Which of 30 notes record a relapse?';
  var QUESTION = 'Does the note record a relapse?';
  var CHUNKS = 30;
  var YES = 9;
  var ESCALATED = 1;
  var FIRST = 'joey-1';
  var SECOND = 'elephant-1';
  // The chunk asked again: joey-1 said no at 0.55, elephant-1 says yes, which
  // makes the ninth yes.
  // [place in the call, final answer, its P, first answer, its P]
  var ASKED_AGAIN = [
    [4, 'yes', '0.94', 'no', '0.55']
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
    var metaIn = q('.dm-meta-in');

    // Width: the row's meta gives way first, then its chevron, so the summary
    // keeps every word. Only on the narrowest tile, where the summary alone is
    // wider than the line, does it truncate as the app's row does (CSS); its
    // full width is read from scrollWidth there.
    // Height: on the shorter tiles the answered column is taller than the
    // frame, and its top runs out under the fade. settle() keeps every line
    // of text either whole or gone.
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
      settle();
    }

    // The lines of the answered column, top to bottom: each is shown whole or
    // not at all.
    var LINES = '.dm-user, .dm-line, .dm-head .dm-l, .dm-esc .dm-l, .dm-qtext, .dm-counts .dm-l, .dm-sec-label, .dm-ask';
    // The answered column is laid out in a hidden copy, whatever the loop is
    // showing, and each line's ink (its text, glyphs and the bubble) is read
    // from it. A line half under the top fade is the problem. The column may
    // move down by about half the frame's margin to show that line whole;
    // when that is not enough, the line and any above it fade out as the
    // last part opens, as a chat scrolled to its end shows its newest lines.
    // On the larger tiles the column fits, and nothing moves or fades.
    function settle() {
      var copy = stage.cloneNode(true);
      copy.className = 'art-decisions dm-instant show-user is-open show-head show-esc show-counts show-sec';
      copy.style.removeProperty('--dm-shift');
      copy.style.visibility = 'hidden';
      copy.querySelector('.dm-row').setAttribute('data-state', 'done');
      root.appendChild(copy);
      // The fade is drawn on the thread, which fills the frame.
      var top0 = copy.querySelector('.dm-thread').getBoundingClientRect().top;
      var pad = parseFloat(getComputedStyle(copy.querySelector('.dm-col')).bottom);
      var range = document.createRange();
      var spans = Array.prototype.map.call(copy.querySelectorAll(LINES), function (el) {
        var boxes = [];
        if (el.classList.contains('dm-user')) boxes.push(el.firstChild.getBoundingClientRect());
        var walk = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
        for (var t = walk.nextNode(); t; t = walk.nextNode()) {
          range.selectNodeContents(t);
          Array.prototype.push.apply(boxes, range.getClientRects());
        }
        Array.prototype.forEach.call(el.querySelectorAll('svg'), function (g) { boxes.push(g.getBoundingClientRect()); });
        var top = Infinity, bot = -Infinity;
        boxes.forEach(function (b) {
          if (b.width > 0 && b.height > 0) { top = Math.min(top, b.top - top0); bot = Math.max(bot, b.bottom - top0); }
        });
        return [top, bot];
      });
      root.removeChild(copy);
      function cut(d, i) { return spans[i][1] + d > pad / 2 + 0.5 && spans[i][0] + d < pad - 0.5; }
      function clear(d) {
        for (var i = 0; i < spans.length; i++) if (cut(d, i)) return false;
        return true;
      }
      var most = Math.ceil(pad / 2 + 1), shift = 0, away = -1;
      while (shift <= most && !clear(shift)) shift++;
      if (shift > most) {
        shift = 0;
        for (var i = 0; i < spans.length; i++) if (spans[i][0] < pad - 0.5) away = i;
      }
      stage.style.setProperty('--dm-shift', shift + 'px');
      Array.prototype.forEach.call(stage.querySelectorAll(LINES), function (el, i) {
        el.classList.toggle('dm-away', i <= away);
      });
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
