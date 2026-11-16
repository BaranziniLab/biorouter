/* Desktop app tile (Ways to use Biorouter).
   The app's window in a narrow size, where the sidebar is collapsed
   (yieldLadder.ts, rung 1, under 1120px): the traffic lights, the sidebar
   toggle and the chat's tab, "New chat" (DEFAULT_SESSION_NAME). The composer
   is the one-line card (ChatInput.structure.test.tsx: +, the text, Send),
   with "Ask Biorouter anything…" in an empty chat and "Ask a follow-up" after
   (composer/copy.ts).
   A question is typed and sent. The agent calls Auto Visualiser: its row reads
   "Rendering chart", then "Rendered chart" (FIGURE_COPY and toolRowLabel in
   toolCallCopy.ts), and the figure arrives as a click-to-open card under the
   row, titled from its ui://bar/chart address ("Bar Chart") with the kind word
   "Figure" (MCPUIResourceRenderer.tsx). In a live chat the side panel opens on
   its own for a new artifact (decideArtifactAutoOpen in BaseChat.tsx): the
   conversation narrows and the panel slides in with the figure's tab and the
   chart, whose title is the one the agent gave it. Then the answer streams.
   The chart's bars and the answer are illustrative, not a recorded run.
   One thing moves at a time. */
(function () {
  'use strict';
  if (!window.BR || !BR.art) return;

  var PROMPT = 'Plot age by diagnosis';
  var ANSWER = 'Here is the figure.';
  var PH_EMPTY = 'Ask Biorouter anything…';
  var PH_FOLLOW = 'Ask a follow-up';
  // Bar heights as a share of the plot. No values are shown.
  var BARS = [0.56, 0.8, 0.64, 0.9, 0.48];
  var MIN_PANEL = 96; // px: the tab and the plot still read

  // Lucide paths at the app's 1.5 stroke (components/icons/app-icons.tsx). The
  // chat glyph is the app's own ChatBubble.
  var ICON = {
    panel: '<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M9 3v18"/>',
    chat: '<path d="M3 8a3.5 3.5 0 0 1 3.5-3.5h11A3.5 3.5 0 0 1 21 8v5.5a3.5 3.5 0 0 1-3.5 3.5H8.4a1.5 1.5 0 0 0-.98.37L4.25 20.1A.75.75 0 0 1 3 19.53Z"/>',
    chart: '<path d="M3 3v16a2 2 0 0 0 2 2h16"/><path d="M18 17V9"/><path d="M13 17V5"/><path d="M8 17v-3"/>',
    plus: '<path d="M5 12h14"/><path d="M12 5v14"/>',
    up: '<path d="m5 12 7-7 7 7"/><path d="M12 19V5"/>',
    chev: '<path d="m9 18 6-6-6-6"/>'
  };
  function svg(name, cls) {
    return '<svg class="' + cls + '" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5"' +
      ' stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">' + ICON[name] + '</svg>';
  }
  function slot(key, inner) {
    return '<div class="acc-gui-slot" data-k="' + key + '"><div class="acc-gui-slot-in">' + inner + '</div></div>';
  }

  BR.art('access-gui', function (root, opts) {
    var reduced = !!(opts && opts.reduced);
    var stage = document.createElement('div');
    stage.className = 'acc-gui';
    stage.setAttribute('aria-hidden', 'true');

    var bars = BARS.map(function (h, i) {
      return '<i class="acc-gui-bar" style="--i:' + i + ';--h:' + (h * 100).toFixed(1) + '%"></i>';
    }).join('');

    stage.innerHTML =
      '<div class="acc-gui-in">' +
        '<div class="acc-gui-chat">' +
          // The chat header band, with the titlebar controls a collapsed
          // sidebar leaves in it (TitlebarControls.tsx).
          '<div class="acc-gui-band">' +
            '<span class="acc-gui-dots"><i></i><i></i><i></i></span>' +
            svg('panel', 'acc-gui-tog') +
            '<span class="acc-gui-tab">' + svg('chat', 'acc-gui-tab-ic') + '<span class="acc-gui-tab-name">New chat</span></span>' +
          '</div>' +
          '<div class="acc-gui-thread">' +
            slot('user', '<div class="acc-gui-user"><span class="acc-gui-bubble">' + PROMPT + '</span></div>') +
            // A tool call is a line, not a card (transcript-row.css): the
            // family glyph, the label, the chevron. The two labels share one
            // cell and hand over when the call finishes.
            slot('row', '<div class="acc-gui-row">' + svg('chart', 'acc-gui-row-ic') +
              '<span class="acc-gui-row-text"><span class="acc-gui-r0">Rendering chart</span><span class="acc-gui-r1">Rendered chart</span></span>' +
              svg('chev', 'acc-gui-chev') + '</div>') +
            slot('card', '<div class="acc-gui-card"><span class="acc-gui-card-tile">' + svg('chart', 'acc-gui-card-ic') + '</span>' +
              '<span class="acc-gui-card-text"><span class="acc-gui-card-title">Bar Chart</span><span class="acc-gui-card-kind">Figure</span></span></div>') +
            slot('ans', '<p class="acc-gui-ans"></p>') +
          '</div>' +
          '<div class="acc-gui-comp">' +
            '<span class="acc-gui-plus">' + svg('plus', 'acc-gui-plus-ic') + '</span>' +
            '<span class="acc-gui-field"><span class="acc-gui-ph">' + PH_EMPTY + '</span><span class="acc-gui-typed"></span><i class="acc-gui-caret"></i></span>' +
            '<span class="acc-gui-send">' + svg('up', 'acc-gui-send-ic') + '</span>' +
          '</div>' +
        '</div>' +
        // The artifact side panel: its tab strip (a third band, flush with the
        // chat's), then the figure on the page ground.
        '<div class="acc-gui-panel">' +
          '<div class="acc-gui-band acc-gui-pband"><span class="acc-gui-tab">' + svg('chart', 'acc-gui-tab-ic') +
            '<span class="acc-gui-tab-name">Bar Chart</span></span></div>' +
          '<div class="acc-gui-fig"><span class="acc-gui-fig-title">Age by diagnosis</span>' +
            '<div class="acc-gui-plot"><i class="acc-gui-grid" style="--y:33.3%"></i><i class="acc-gui-grid" style="--y:66.6%"></i>' + bars + '</div>' +
          '</div>' +
        '</div>' +
      '</div>';
    root.appendChild(stage);

    var q = function (s) { return stage.querySelector(s); };
    var inner = q('.acc-gui-in'), field = q('.acc-gui-field'), comp = q('.acc-gui-comp');
    var ph = q('.acc-gui-ph'), typed = q('.acc-gui-typed'), bubble = q('.acc-gui-bubble');
    var row = q('.acc-gui-row'), ans = q('.acc-gui-ans');
    var slots = {};
    Array.prototype.forEach.call(stage.querySelectorAll('.acc-gui-slot'), function (s) { slots[s.getAttribute('data-k')] = s; });

    function set(cls, on) { stage.classList.toggle(cls, !!on); }
    function setSlot(k, state) {
      var s = slots[k];
      s.classList.toggle('is-open', state !== 'closed');
      s.classList.toggle('is-shown', state === 'shown');
    }
    function setTyped(s) { typed.textContent = s; set('has-text', s.length > 0); }
    function setRow(state) {
      row.classList.toggle('is-run', state === 'run');
      row.classList.toggle('is-done', state === 'done');
    }
    // Suspends transitions for one frame, so a reset never animates.
    function settle(fn) {
      stage.classList.add('acc-gui-still');
      fn();
      void stage.offsetWidth;
      stage.classList.remove('acc-gui-still');
    }

    // Width. The panel takes about 46% of the window, but never so much that the
    // conversation beside it cannot hold the composer with "Ask a follow-up"
    // and the sent message on one line: the conversation narrows, it does not
    // re-wrap, so nothing in it jumps while the panel slides in.
    var ctx = document.createElement('canvas').getContext('2d');
    function textWidth(el, s) {
      var cs = getComputedStyle(el);
      ctx.font = cs.fontStyle + ' ' + cs.fontWeight + ' ' + cs.fontSize + ' ' + cs.fontFamily;
      return ctx.measureText(s).width;
    }
    function fit() {
      var W = inner.clientWidth;
      if (!W) return;
      var pad = parseFloat(getComputedStyle(inner).getPropertyValue('--acc-gui-pad')) || 12;
      var fs = getComputedStyle(field);
      var chrome = comp.offsetWidth - field.clientWidth;
      var need = chrome + parseFloat(fs.paddingLeft) + parseFloat(fs.paddingRight) + textWidth(field, PH_FOLLOW) + 4;
      var chatMin = Math.ceil(Math.max(need, bubble.offsetWidth) + 2 * pad);
      var pw = Math.min(Math.round(W * 0.46), W - chatMin);
      inner.style.setProperty('--acc-gui-pw', Math.max(MIN_PANEL, pw) + 'px');
    }
    if ('ResizeObserver' in window) new ResizeObserver(fit).observe(inner);

    function rest() {
      settle(function () {
        stage.classList.remove('is-focus', 'has-text', 'is-press', 'is-lift', 'is-open', 'is-grown', 'is-clearing');
        setTyped('');
        ph.textContent = PH_EMPTY;
        Object.keys(slots).forEach(function (k) { setSlot(k, 'closed'); });
        setRow('run');
        ans.textContent = '';
      });
    }
    // The answered turn, with the figure open in the panel.
    function finished() {
      settle(function () {
        stage.classList.remove('is-focus', 'has-text', 'is-press', 'is-lift', 'is-clearing');
        setTyped('');
        ph.textContent = PH_FOLLOW;
        Object.keys(slots).forEach(function (k) { setSlot(k, 'shown'); });
        setRow('done');
        ans.textContent = ANSWER;
        set('is-open', true);
        set('is-grown', true);
      });
    }

    if (reduced) {
      finished();
      fit();
      return { start: function () {}, stop: function () {} };
    }

    var timers = [];
    var active = false;
    // Each id leaves the list when its timeout fires, so the list holds only the
    // timeouts still pending and never grows across loops.
    function at(ms, fn) {
      var id = setTimeout(function () {
        var k = timers.indexOf(id);
        if (k >= 0) timers.splice(k, 1);
        if (active) fn();
      }, ms);
      timers.push(id);
    }
    function clear() { timers.forEach(clearTimeout); timers = []; }
    function type(el, text, t0, gap, onChar) {
      for (var k = 1; k <= text.length; k++) (function (k) { at(t0 + (k - 1) * gap, function () { onChar(text.slice(0, k)); }); })(k);
      return t0 + (text.length - 1) * gap;
    }

    function cycle() {
      // The question, typed into the empty chat. The card's edge turns accent.
      at(300, function () { set('is-focus', true); });
      var typedAt = type(typed, PROMPT, 500, 70, setTyped);
      // Send: the press, then the text lifts out of the card and the message
      // takes its place in the conversation.
      at(typedAt + 400, function () { set('is-press', true); });
      at(typedAt + 525, function () {
        set('is-press', false); set('is-focus', false); set('is-lift', true);
        ph.textContent = PH_FOLLOW;
        setSlot('user', 'open');
      });
      at(typedAt + 700, function () { set('is-lift', false); setTyped(''); setSlot('user', 'shown'); });
      // The tool call runs, then finishes, and its figure arrives as a card.
      at(3200, function () { setSlot('row', 'open'); });
      at(3350, function () { setSlot('row', 'shown'); });
      at(4700, function () { setRow('done'); });
      at(5100, function () { setSlot('card', 'open'); });
      at(5250, function () { setSlot('card', 'shown'); });
      // A new artifact in a live chat opens the panel on its own.
      at(5900, function () { set('is-open', true); });
      at(6450, function () { set('is-grown', true); });
      // The answer streams under the card.
      at(7600, function () { setSlot('ans', 'shown'); });
      type(ans, ANSWER, 7650, 35, function (s) { ans.textContent = s; });
      // Hold, fade, close the panel, and start again from an empty chat.
      at(10000, function () { set('is-clearing', true); });
      at(10350, function () { set('is-open', false); });
      at(11000, rest);
      at(11300, cycle);
    }

    rest();
    fit();
    return {
      start: function () { if (active) return; active = true; set('is-live', true); clear(); rest(); cycle(); },
      stop: function () { active = false; set('is-live', false); clear(); rest(); }
    };
  });
})();
