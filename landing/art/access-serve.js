/* Browser access tile (biorouter serve).
   A terminal on a server runs `biorouter serve --open` and prints the lines
   serve.rs's banner_text writes: "Biorouter is serving at", the address with
   its one-time token, and "Press Ctrl-C to stop." (the token, model and
   dictation lines between them are left out for room). The token is random and
   new on every launch; it is shown elided, as docs.html#cli-serve prints it.
   The address then moves to the top of the terminal while the other lines
   clear, and a browser window opens under it, as --open does. The address bar
   reads 127.0.0.1:8765 because opening the address trades the token for a
   cookie and removes it from the bar (docs.html, "The access token"); nothing
   is typed there, since --open opens the page for you.
   The page is the same interface: a sidebar (Home, New chat and Skills from
   AppSidebar.tsx, then Chats) and the composer card with the app's empty
   placeholder ("Ask Biorouter anything…", composer/copy.ts), which turns to
   "Ask a follow-up" after the first message. No model picker is drawn.
   The question, the reply and the two earlier chat names are illustrative.
   One thing moves at a time. */
(function () {
  'use strict';

  var CMD = 'biorouter serve --open';
  var HOST = '127.0.0.1:8765';
  // The address line as serve prints it (six spaces in), then shorter forms
  // for narrow tiles: the elided token shortens first, the indent last.
  var URLS = [
    '      http://127.0.0.1:8765/?t=1f4c9e02…',
    '      http://127.0.0.1:8765/?t=1f4c…',
    '  http://127.0.0.1:8765/?t=1f4c…',
    '  http://127.0.0.1:8765/?t=…'
  ];
  var ASK = 'Summarize cohort.csv';
  var REPLY = 'The file has 5,000 rows and 14 columns.';
  var PH_EMPTY = 'Ask Biorouter anything…';
  var PH_FOLLOW = 'Ask a follow-up';
  var CHATS = ['QC report', 'Variants'];

  // Lucide paths at the app's 1.5 stroke (components/icons/app-icons.tsx):
  // Home is lucide house, New chat is SquarePen, Skills and the chat bubble are
  // the app's own glyphs.
  var HOME = '<path d="M15 21v-8a1 1 0 0 0-1-1h-4a1 1 0 0 0-1 1v8"/><path d="M3 10a2 2 0 0 1 .709-1.528l7-6a2 2 0 0 1 2.582 0l7 6A2 2 0 0 1 21 10v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>';
  var PEN = '<path d="M12 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.375 2.625a1 1 0 0 1 3 3l-9.013 9.014a2 2 0 0 1-.853.505l-2.873.84a.5.5 0 0 1-.62-.62l.84-2.873a2 2 0 0 1 .506-.852z"/>';
  var SKILL = '<rect x="3.5" y="3.5" width="17" height="17" rx="4"/><path d="M12.75 7 9 12.5h6L11.25 17"/>';
  var BUBBLE = '<path d="M3 8a3.5 3.5 0 0 1 3.5-3.5h11A3.5 3.5 0 0 1 21 8v5.5a3.5 3.5 0 0 1-3.5 3.5H8.4a1.5 1.5 0 0 0-.98.37L4.25 20.1A.75.75 0 0 1 3 19.53Z"/>';
  var PLUS = '<path d="M5 12h14"/><path d="M12 5v14"/>';
  var UP = '<path d="m5 12 7-7 7 7"/><path d="M12 19V5"/>';
  function icon(paths) {
    return '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' + paths + '</svg>';
  }
  function el(tag, cls, text) {
    var n = document.createElement(tag);
    if (cls) n.className = cls;
    if (text != null) n.textContent = text;
    return n;
  }

  BR.art('access-serve', function (root, opts) {
    var reduced = !!(opts && opts.reduced);
    var wrap = el('div', 'acs');
    wrap.setAttribute('aria-hidden', 'true');

    // ── The terminal ──────────────────────────────────────
    var term = el('div', 'acs-term');
    var lines = el('div', 'acs-lines');
    function row(cls, text) { var r = el('span', 'acs-row' + (cls ? ' ' + cls : ''), text); lines.appendChild(r); return r; }
    // Every row but the address fades as the browser opens (acs-fade).
    var prompt = row('acs-fade');
    prompt.appendChild(el('span', 'acs-ps', '$'));
    prompt.appendChild(document.createTextNode(' '));
    var cmd = el('span', 'acs-cmd');
    prompt.appendChild(cmd);
    prompt.appendChild(el('i', 'acs-tcur'));
    row('acs-out acs-fade', '');
    row('acs-out acs-fade', '  Biorouter is serving at');
    row('acs-out acs-fade', '');
    var url = row('acs-out acs-url', URLS[0]);
    row('acs-out acs-fade', '');
    row('acs-out acs-fade', '  Press Ctrl-C to stop.');
    term.appendChild(lines);

    // ── The browser window ───────────────────────────────
    var win = el('div', 'acs-win');
    var bar = el('div', 'acs-bar');
    var dots = el('span', 'acs-dots');
    dots.appendChild(el('i')); dots.appendChild(el('i')); dots.appendChild(el('i'));
    bar.appendChild(dots);
    bar.appendChild(el('span', 'acs-addr', HOST));
    win.appendChild(bar);

    var page = el('div', 'acs-page');
    var side = el('div', 'acs-side');
    function nav(glyph, label, cls) {
      var r = el('div', 'acs-nav' + (cls ? ' ' + cls : ''));
      r.innerHTML = icon(glyph);
      r.appendChild(el('span', 'acs-nav-t', label));
      side.appendChild(r);
    }
    nav(HOME, 'Home');
    nav(PEN, 'New chat');
    nav(SKILL, 'Skills');
    side.appendChild(el('div', 'acs-label', 'Chats'));
    CHATS.forEach(function (c) { nav(BUBBLE, c, 'acs-chat'); });

    var main = el('div', 'acs-main');
    var log = el('div', 'acs-log');
    var user = el('div', 'acs-user', ASK);
    var reply = el('p', 'acs-reply');
    var words = REPLY.split(' ').map(function (w, i, all) {
      var s = el('span', 'w', w);
      reply.appendChild(s);
      if (i < all.length - 1) reply.appendChild(document.createTextNode(' '));
      return s;
    });
    log.appendChild(user); log.appendChild(reply);

    // The composer card (R-32): the prose line, then the controls row with +
    // and Send. Narrow, short tiles fold the two rows into one (access-serve.css).
    var card = el('div', 'acs-card');
    var input = el('div', 'acs-input');
    var ph = el('span', 'acs-ph', PH_EMPTY);
    var typedWrap = el('span', 'acs-line');
    var typed = el('span', 'acs-typed');
    typedWrap.appendChild(typed);
    typedWrap.appendChild(el('i', 'acs-caret'));
    input.appendChild(ph); input.appendChild(typedWrap);
    var ctl = el('div', 'acs-ctl');
    var plus = el('span', 'acs-plus'); plus.innerHTML = icon(PLUS);
    var send = el('span', 'acs-send'); send.innerHTML = icon(UP);
    ctl.appendChild(plus); ctl.appendChild(send);
    card.appendChild(input); card.appendChild(ctl);
    main.appendChild(log); main.appendChild(card);

    page.appendChild(side); page.appendChild(main);
    win.appendChild(page);

    // .acs is the size container and carries the state classes; .acs-in holds
    // the geometry, so the size tiers in access-serve.css can restate it.
    var stage = el('div', 'acs-in');
    stage.appendChild(term); stage.appendChild(win);
    wrap.appendChild(stage);
    root.appendChild(wrap);

    function settle(fn) {
      wrap.classList.add('acs-still');
      fn();
      void wrap.offsetWidth;
      wrap.classList.remove('acs-still');
    }

    // Width: the address line is the one long string. Step it through URLS
    // until it fits whole rather than letting it clip.
    function fit() {
      for (var k = 0; k < URLS.length; k++) {
        url.textContent = URLS[k];
        if (url.scrollWidth <= url.clientWidth + 0.5) break;
      }
    }
    if ('ResizeObserver' in window) new ResizeObserver(fit).observe(root);

    var STATES = ['is-run', 'is-printed', 'is-lift', 'is-open', 'is-edge', 'has-text', 'is-press', 'show-user'];
    function draw(done) {
      // done: the answered frame; otherwise the resting prompt.
      STATES.forEach(function (c) { wrap.classList.toggle(c, !!done && c !== 'is-edge' && c !== 'has-text' && c !== 'is-press'); });
      cmd.textContent = done ? CMD : '';
      typed.textContent = '';
      ph.textContent = done ? PH_FOLLOW : PH_EMPTY;
      words.forEach(function (w) { w.classList.toggle('on', !!done); });
    }

    if (reduced) {
      // One still: the browser is open on the answer, the address above it.
      settle(function () { draw(true); });
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
    // Fixed-step timers, so stop() clears a half typed line with the rest.
    function type(node, text, t0, step, first) {
      for (var i = 1; i <= text.length; i++) (function (i) {
        at(t0 + (i - 1) * step, function () {
          if (i === 1 && first) first();
          node.textContent = text.slice(0, i);
        });
      })(i);
    }

    function rest() {
      settle(function () { wrap.classList.remove('is-out'); draw(false); });
    }

    function cycle() {
      // The command, typed at the prompt, then Enter.
      type(cmd, CMD, 400, 66);
      at(2100, function () { wrap.classList.add('is-run'); });
      // serve prints its banner in one go.
      at(2250, function () { wrap.classList.add('is-printed'); });
      // The address moves to the top of the terminal as the other lines clear,
      // then the browser opens under it.
      at(3400, function () { wrap.classList.add('is-lift'); });
      at(3950, function () { wrap.classList.add('is-open'); });
      // The composer takes focus and the question is typed.
      at(4900, function () { wrap.classList.add('is-edge'); });
      type(typed, ASK, 5100, 55, function () { wrap.classList.add('has-text'); });
      // Send.
      at(6500, function () { wrap.classList.add('is-press'); });
      at(6625, function () {
        typed.textContent = '';
        ph.textContent = PH_FOLLOW;
        wrap.classList.remove('is-press', 'has-text', 'is-edge');
        wrap.classList.add('show-user');
      });
      // The reply streams in, a word at a time.
      words.forEach(function (w, i) { at(7300 + i * 130, function () { w.classList.add('on'); }); });
      // Hold, fade, and return to the prompt.
      at(10300, function () { wrap.classList.add('is-out'); });
      at(10650, function () {
        settle(function () { draw(false); });
        wrap.classList.remove('is-out');
      });
      at(11000, cycle);
    }

    rest();
    fit();
    return {
      start: function () { if (active) return; active = true; clear(); rest(); cycle(); },
      stop: function () { active = false; clear(); rest(); }
    };
  });
})();
