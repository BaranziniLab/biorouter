/* Terminal chat tile (download page, "Ways to use Biorouter").
   `biorouter session`, or a bare `biorouter` in a terminal, opens the full
   screen chat in crates/biorouter-cli/src/session/tui. Every string in the
   tile is the TUI's own: the BIOROUTER_ASCII banner and the greeting
   (output.rs, greeting_into in tui/mod.rs), the person's turn as a coral bar
   on a shaded line (push_user in tui/app.rs), the input box with its
   "Send a message…" placeholder and, while a turn runs, a spinner and one of
   thinking.rs's labels in its top edge (draw_input), the status line
   (draw_status) and the key hints (hint_text). The status line and the hints
   choose their form by the width the way the TUI does, so a narrow tile shows
   what a terminal that narrow shows. The model line above the counts, the
   version after the tagline and the working directory line are left out to
   keep the tile short. The question, the answer, the counts and the 4% are
   illustrative. One thing moves at a time. */
(function () {
  'use strict';
  if (!window.BR || !BR.art) return;

  var BANNER = [
    '█▀▄ █ █▀█ █▀▄ █▀█ █ █ ▀█▀ █▀▀ █▀▄',
    '█▀▄ █ █ █ █▀▄ █ █ █ █  █  █▀▀ █▀▄',
    '▀▀  ▀ ▀▀▀ ▀ ▀ ▀▀▀ ▀▀▀  ▀  ▀▀▀ ▀ ▀'
  ];
  var TAGLINE = 'Biorouter: integrated biomedical research environment';
  var ENTER = 'Enter your instructions, or type /help for commands';
  var PLACEHOLDER = 'Send a message…';
  var SPINNER = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
  var THINKING = 'Aligning reads';
  var QUESTION = 'Which genes in genes.tsv are on chr6?';
  var ANSWER = 'Two genes are on chr6: HLA-DRB1 and TNF.';
  // draw_status: the long counts when they fit beside the meter, else the
  // compact ones, else the compact ones beside "ctx N%".
  var COUNTS_FULL = '8 skills · 3 extensions · 1 knowledge base';
  var COUNTS_COMPACT = '8 skills · 3 ext · 1 KB';
  var METER_CELLS = 12;
  // hint_text with no subagents and no dictation key: the longest that fits.
  var HINTS = [
    '↵ send · ^J newline · / commands · ↑↓ history · ^C stop · ^B subagents · type anytime',
    '↵ send · ^J newline · / commands · ↑↓ history · ^C stop · ^B subagents',
    '↵ send · ^J newline · / commands · ↑↓ history · ^C stop'
  ];
  var SPIN_MS = 110; // the TUI's tick

  function el(tag, cls, text) {
    var n = document.createElement(tag);
    if (cls) n.className = cls;
    if (text != null) n.textContent = text;
    return n;
  }
  function row(cls, text) { return el('div', 'acc-tui-row' + (cls ? ' ' + cls : ''), text); }

  // The banner, cell for cell. A terminal draws each half block as a solid
  // half cell; a font's glyphs leave seams between cells and differ from
  // font to font, so the tile reads BANNER and draws the same cells: a cell
  // is one column wide and one row tall, and its upper half is filled for
  // '█' and '▀', its lower half for '█' and '▄'.
  function bannerArt() {
    var box = el('div', 'acc-tui-banner');
    var NS = 'http://www.w3.org/2000/svg';
    var s = document.createElementNS(NS, 'svg');
    s.setAttribute('viewBox', '0 0 ' + BANNER[0].length + ' ' + BANNER.length * 2);
    s.setAttribute('preserveAspectRatio', 'none');
    s.setAttribute('shape-rendering', 'crispEdges');
    s.setAttribute('fill', 'currentColor');
    s.setAttribute('aria-hidden', 'true');
    BANNER.forEach(function (line, r) {
      [['█▀', 0], ['█▄', 1]].forEach(function (half) {
        var run = -1;
        for (var c = 0; c <= line.length; c++) {
          var on = c < line.length && half[0].indexOf(line.charAt(c)) >= 0;
          if (on && run < 0) run = c;
          if (!on && run >= 0) {
            var rect = document.createElementNS(NS, 'rect');
            rect.setAttribute('x', run); rect.setAttribute('y', r * 2 + half[1]);
            rect.setAttribute('width', c - run); rect.setAttribute('height', 1);
            s.appendChild(rect);
            run = -1;
          }
        }
      });
    });
    box.appendChild(s);
    return box;
  }

  BR.art('access-tui', function (root, opts) {
    var reduced = !!(opts && opts.reduced);
    var wrap = el('div', 'acc-tui');
    wrap.setAttribute('aria-hidden', 'true');
    var pane = el('div', 'acc-tui-pane');
    wrap.appendChild(pane);

    // The scrollback. It sits at the top while it fits; once it is taller
    // than the pane the newest line stays in view, as in the TUI (see scroll).
    var hist = el('div', 'acc-tui-hist');
    var lines = el('div', 'acc-tui-lines');
    var banner = bannerArt();
    lines.appendChild(banner);
    lines.appendChild(row('acc-tui-blank'));
    lines.appendChild(row('acc-tui-tag', TAGLINE));
    lines.appendChild(row('acc-tui-dim', ENTER));
    // The turn: push_user adds a blank, the barred line and a blank; the
    // reply follows.
    var turn = el('div', 'acc-tui-turn');
    var you = row('acc-tui-you');
    you.appendChild(el('b', 'acc-tui-bar', '▌ '));
    you.appendChild(el('span', 'acc-tui-said', QUESTION + ' '));
    var reply = row('acc-tui-reply');
    turn.appendChild(row('acc-tui-blank'));
    turn.appendChild(you);
    turn.appendChild(row('acc-tui-blank'));
    turn.appendChild(reply);
    lines.appendChild(turn);
    hist.appendChild(lines);
    pane.appendChild(hist);

    // Status: counts on the left, the context meter on the right.
    var status = el('div', 'acc-tui-status');
    var counts = el('span', 'acc-tui-counts');
    var right = el('span', 'acc-tui-right');
    status.appendChild(counts);
    status.appendChild(right);
    pane.appendChild(status);

    // The input box.
    var box = el('div', 'acc-tui-box');
    var legend = el('span', 'acc-tui-legend');
    var field = el('div', 'acc-tui-field');
    var typed = el('span', 'acc-tui-typed');
    var caret = el('i', 'acc-tui-caret');
    var ph = el('span', 'acc-tui-ph', PLACEHOLDER);
    var draft = el('span', 'acc-tui-draft');
    draft.appendChild(typed); draft.appendChild(caret); draft.appendChild(ph);
    field.appendChild(draft);
    field.appendChild(el('span', 'acc-tui-spacer'));
    box.appendChild(el('b', 'acc-tui-prompt', '❯ '));
    box.appendChild(field);
    box.appendChild(legend);
    pane.appendChild(box);

    var hints = el('div', 'acc-tui-hints');
    pane.appendChild(hints);

    // A probe for one cell's width in the pane's own font.
    var probe = el('span', 'acc-tui-probe', 'xxxxxxxxxxxxxxxxxxxx');
    pane.appendChild(probe);
    root.appendChild(wrap);

    // ── State ───────────────────────────────────────────────
    var pct = 0;
    var cols = 40;

    function meter(p) {
      var filled = Math.min(METER_CELLS, Math.floor(p * METER_CELLS / 100));
      var f = document.createDocumentFragment();
      f.appendChild(document.createTextNode('context ▕'));
      if (filled) f.appendChild(el('span', 'acc-tui-cells', new Array(filled + 1).join('█')));
      f.appendChild(document.createTextNode(new Array(METER_CELLS - filled + 1).join('░') + '▏ ' + p + '%'));
      return f;
    }
    function drawStatus() {
      var long = 'context ▕' + new Array(METER_CELLS + 1).join('░') + '▏ ' + pct + '%';
      var left = COUNTS_FULL.length + long.length < cols ? COUNTS_FULL : COUNTS_COMPACT;
      var summary = left.length + long.length >= cols;
      var rightLen = summary ? ('ctx ' + pct + '%').length : long.length;
      counts.textContent = left;
      right.textContent = '';
      if (summary) right.textContent = 'ctx ' + pct + '%';
      else right.appendChild(meter(pct));
      right.hidden = left.length + rightLen >= cols;
    }
    // Where no hint line fits whole, the terminal cuts the shortest at its
    // edge; the tile leaves the line out instead, as it leaves out the model
    // line, rather than draw a cut one.
    function drawHints() {
      var pick = '';
      for (var i = 0; i < HINTS.length; i++) if (HINTS[i].length <= cols) { pick = HINTS[i]; break; }
      hints.textContent = pick;
      wrap.classList.toggle('hints-cut', !pick);
    }
    function setDraft(text) {
      typed.textContent = text;
      box.classList.toggle('has-draft', text !== '');
    }
    function setPct(p) { pct = p; drawStatus(); }
    function setBusy(on, k) {
      box.classList.toggle('is-busy', on);
      if (on) legend.textContent = ' ' + SPINNER[k % SPINNER.length] + ' ' + THINKING + ' ';
    }
    function showTurn(n) {
      // n = 0: no turn; 1: the question; 2: the question and the reply row.
      wrap.classList.toggle('has-question', n >= 1);
      wrap.classList.toggle('has-reply', n >= 2);
      scroll();
    }
    function setReply(text) { reply.textContent = text; scroll(); }
    // Keep the newest line in view. The banner leaves whole, with the blank
    // row under it, rather than a row at a time: its last rows alone do not
    // read as letters.
    function scroll() {
      var over = lines.offsetHeight - hist.clientHeight;
      var y = 0;
      if (over > 0) {
        var block = banner.offsetHeight + banner.nextSibling.offsetHeight;
        y = over < block ? block : over;
      }
      lines.style.transform = y ? 'translateY(' + -y + 'px)' : '';
    }

    function settle(fn) {
      wrap.classList.add('acc-tui-still');
      fn();
      void wrap.offsetWidth;
      wrap.classList.remove('acc-tui-still');
    }

    // ── Fit ─────────────────────────────────────────────────
    // Width: count the columns the pane holds, then let the status line and
    // the hints choose their form for that many.
    // Height: the scrollback shows whole rows. The finished turn should leave
    // the banner in view: tighten the spacing first, then drop the status
    // line, then the hints. Where even that is not enough, keep both and let
    // the scrollback scroll, as the TUI's does.
    var LADDER = [[], ['fit-tight'], ['fit-tight', 'no-status'], ['fit-tight', 'no-status', 'no-hints']];
    var ALL = ['fit-tight', 'no-status', 'no-hints'];
    function fit() {
      hist.style.removeProperty('height');
      hist.style.removeProperty('flex');
      var had = [wrap.classList.contains('has-question'), wrap.classList.contains('has-reply'), reply.textContent];
      wrap.classList.add('has-question', 'has-reply');
      reply.textContent = ANSWER;
      var chosen = LADDER[1];
      for (var i = 0; i < LADDER.length; i++) {
        ALL.forEach(function (c) { wrap.classList.remove(c); });
        LADDER[i].forEach(function (c) { wrap.classList.add(c); });
        measure();
        if (need() <= avail() + 0.5) { chosen = LADDER[i]; break; }
      }
      ALL.forEach(function (c) { wrap.classList.remove(c); });
      chosen.forEach(function (c) { wrap.classList.add(c); });
      measure();
      var L = parseFloat(getComputedStyle(pane).lineHeight) || 14;
      hist.style.flex = 'none';
      hist.style.height = Math.max(L, Math.floor((avail() + 0.5) / L) * L) + 'px';
      wrap.classList.toggle('has-question', had[0]);
      wrap.classList.toggle('has-reply', had[1]);
      reply.textContent = had[2];
      setDraft(typed.textContent);
      settle(scroll);
    }
    function measure() {
      var cs = getComputedStyle(pane);
      var inner = pane.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight);
      var cell = probe.getBoundingClientRect().width / 20 || 7.2;
      cols = Math.max(1, Math.floor((inner + 0.5) / cell));
      drawStatus();
      drawHints();
    }
    function need() { return lines.getBoundingClientRect().height; }
    function avail() {
      var cs = getComputedStyle(pane);
      var h = pane.clientHeight - parseFloat(cs.paddingTop) - parseFloat(cs.paddingBottom);
      [status, box, hints].forEach(function (n) {
        var s = getComputedStyle(n);
        if (s.display === 'none') return;
        h -= n.getBoundingClientRect().height + parseFloat(s.marginTop) + parseFloat(s.marginBottom);
      });
      return h;
    }
    if ('ResizeObserver' in window) new ResizeObserver(fit).observe(root);

    if (reduced) {
      // One still: the reply is in and the meter has moved.
      settle(function () {
        reply.textContent = ANSWER;
        showTurn(2);
        setPct(4);
        setDraft('');
        setBusy(false);
      });
      fit();
      return { start: function () {}, stop: function () {} };
    }

    var timers = [];
    var active = false;
    // Each id leaves the list when its timeout fires, so the list holds only the
    // timeouts still pending.
    function at(ms, fn) {
      var id = setTimeout(function () {
        var k = timers.indexOf(id);
        if (k >= 0) timers.splice(k, 1);
        if (active) fn();
      }, ms);
      timers.push(id);
    }
    function clear() { timers.forEach(clearTimeout); timers = []; }

    // The first frame: the greeting, an empty input with its caret blinking.
    function base() {
      wrap.classList.remove('is-clearing');
      reply.textContent = '';
      showTurn(0);
      setDraft('');
      setBusy(false);
      box.classList.add('is-idle');
      right.classList.remove('is-swap');
      setPct(0);
    }
    function rest() { settle(base); }

    var TYPE_AT = 700, TYPE_STEP = 52;
    var ENTER_AT = 2900, BUSY_AT = 3300, REPLY_AT = 4300, STREAM_STEP = 15;
    var METER_AT = 5800, IDLE_AT = 6200, CLEAR_AT = 10000, LOOP = 10300;

    function cycle() {
      // The question is typed at the prompt.
      at(TYPE_AT, function () { box.classList.remove('is-idle'); });
      for (var c = 1; c <= QUESTION.length; c++) {
        (function (c) { at(TYPE_AT + (c - 1) * TYPE_STEP, function () { setDraft(QUESTION.slice(0, c)); }); })(c);
      }
      // Enter: the input clears and the turn joins the scrollback.
      at(ENTER_AT, function () { setDraft(''); showTurn(1); });
      // The turn runs: the spinner turns in the box's top edge.
      for (var t = BUSY_AT, k = 0; t < REPLY_AT; t += SPIN_MS, k++) {
        (function (k) { at(t, function () { setBusy(true, k); }); })(k);
      }
      // The reply streams in and the spinner stops.
      at(REPLY_AT, function () { setBusy(false); showTurn(2); });
      for (var i = 1; i <= ANSWER.length; i++) {
        (function (i) { at(REPLY_AT + i * STREAM_STEP, function () { setReply(ANSWER.slice(0, i)); }); })(i);
      }
      // The meter takes the turn's tokens.
      at(METER_AT, function () { right.classList.add('is-swap'); });
      at(METER_AT + 175, function () { setPct(4); right.classList.remove('is-swap'); });
      at(IDLE_AT, function () { box.classList.add('is-idle'); });
      // The turn fades, the meter returns to 0%, and the next loop begins.
      at(CLEAR_AT, function () { wrap.classList.add('is-clearing'); });
      at(LOOP, function () {
        settle(function () { wrap.classList.remove('is-clearing'); reply.textContent = ''; showTurn(0); });
        right.classList.add('is-swap');
        at(175, function () { setPct(0); right.classList.remove('is-swap'); });
        cycle();
      });
    }

    rest();
    fit();
    return {
      start: function () {
        if (active) return;
        active = true; clear(); rest();
        wrap.classList.remove('acc-tui-paused');
        cycle();
      },
      stop: function () {
        active = false; clear(); rest();
        wrap.classList.add('acc-tui-paused');
      }
    };
  });
})();
