/* Principle 01: Private stays private.
   A drag-to-compare slider in the manner of the hshs before/after slider. The
   two layers are two Biorouter chats; the divider is the wall between them.
   The private chat is the top layer, clipped to the wall at --split; the public
   chat is the base layer. Each chat's column is anchored to the wall, so both
   stay readable at any split.

   While nobody is dragging, a slow loop shows that nothing crosses:
   1. The private chat's model picker opens and the selection tries Claude.
      The app's pre-flight reason appears (SwitchModelModal.tsx,
      PUBLIC_MODEL_IN_PRIVATE_CHAT), and the selection returns to the private
      model, so the chip keeps Versa API Azure.
   2. The public chat asks to read another chat. The request travels to the
      wall and stops there (privacy/refusal.rs workspace_out_of_reach: "Nothing
      was read and nothing was changed").
   Privacy badges never animate.

   Layout. At rest (the wall in the middle) each chat takes the fullest
   arrangement in which its event fits: the picker is compacted, then the
   answer and the tool row are left out, in that order. That reference is
   measured once per frame size, so a drag can only take things away at the
   extremes, never add them. A chat narrower than 210px takes the short
   spellings (wall.css); one whose question still runs too tall takes the
   tiny spelling, and a title that does not fit takes its short form. If the
   visitor leaves the wall where an event has no room, the wall eases back
   to the middle before the next pass. */
(function () {
  'use strict';
  if (!window.BR || !BR.art) return;

  var MIN = 12, MAX = 88;
  var SLOP = 8;       // px; a touch moves the wall only once it travels this far sideways
  var EVT_MIN = 96;   // px; an event needs a column at least this wide
  var GAP = 10;       // px; clear space kept between messages, events and the composer
  var Q_SPELL = 3;    // a question that needs more lines than this takes a shorter spelling
  var Q_LINES = 4;    // and one that still needs more than this steps aside
  var T_LINES = 2;    // a tool row that needs more lines than this steps aside
  var PH = ['l', 'm', 's', '0'];  // placeholder spellings, longest first; 0 hides it

  var PATHS = {
    lock: '<rect x="4" y="11" width="16" height="10" rx="2"/><path d="M8 11V7a4 4 0 0 1 8 0v4"/>',
    wrench: '<path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/>',
    search: '<circle cx="11" cy="11" r="7"/><path d="m20 20-4-4"/>',
    chat: '<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>',
    brain: '<path d="M12 5a3 3 0 1 0-5.997.125 4 4 0 0 0-2.526 5.77 4 4 0 0 0 .556 6.588A4 4 0 1 0 12 18Z"/><path d="M12 5a3 3 0 1 1 5.997.125 4 4 0 0 1 2.526 5.77 4 4 0 0 1-.556 6.588A4 4 0 1 1 12 18Z"/><path d="M15 13a4.5 4.5 0 0 1-3-4 4.5 4.5 0 0 1-3 4"/>',
    up: '<path d="m6 11 6-6 6 6"/><path d="M12 19V5"/>',
    left: '<path d="m15 18-6-6 6-6"/>',
    right: '<path d="m9 18 6-6-6-6"/>',
    check: '<path d="M20 6 9 17l-5-5"/>',
    bank: '<path d="M3 21h18"/><path d="M6 17v-6"/><path d="M10 17v-6"/><path d="M14 17v-6"/><path d="M18 17v-6"/><path d="M12 3 20 8H4z"/>'
  };
  function icon(name, cls) {
    return '<svg class="wall-i ' + (cls || '') + '" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' + PATHS[name] + '</svg>';
  }
  // One line in four spellings: desktop, short, large and tiny. The frame
  // picks among the first three (wall.css); a chat too narrow for its
  // question takes the short one, then the tiny one (wall.js).
  function say(d, p, g, n) {
    return '<span class="wall-d">' + d + '</span><span class="wall-p">' + (p || d) + '</span><span class="wall-g">' + (g || d) + '</span><span class="wall-n">' + (n || p || d) + '</span>';
  }
  // A title and its short form.
  function title(long, short) {
    return '<span class="wall-title"><span class="wall-tl">' + long + '</span><span class="wall-ts">' + short + '</span></span>';
  }

  function composer(model, lock) {
    return '<div class="wall-comp">' +
      '<div class="wall-card"><span class="wall-ph"><span class="wall-phl">Ask Biorouter anything…</span><span class="wall-phm">Ask anything…</span><span class="wall-phs">Ask…</span></span><span class="wall-send">' + icon('up') + '</span></div>' +
      '<p class="wall-chip">' + icon('brain', 'wall-brain') + '<span class="wall-cn">' + model + '</span>' + (lock ? icon('lock', 'wall-chiplock') : '') + '</p>' +
    '</div>';
  }

  var PRIVATE =
    '<div class="wall-layer wall-priv"><div class="wall-pane">' +
      '<div class="wall-head">' +
        '<span class="privacy is-private wall-badge">Private</span>' +
        '<span class="wall-lockonly">' + icon('lock') + '</span>' +
        title('Statin exposure, T2D cohort', 'Statin exposure') +
      '</div>' +
      '<div class="wall-body">' +
        '<div class="wall-msgs">' +
          '<p class="wall-user wall-m-user">' + say('How many patients with type 2 diabetes take a statin?', 'How many T2D patients take a statin?', 0, 'T2D patients on a statin?') + '</p>' +
          '<p class="wall-tool wall-m-tool">' + icon('wrench') + '<span class="wall-tx"><span class="wall-tt">' + say('Ran CDWAgent', 'Ran CDWAgent', 'Ran CDWAgent · build cohort') + '</span><span class="wall-aff">' + icon('bank') + 'UCSF data</span></span></p>' +
          '<p class="wall-bot wall-m-bot">412 patients match.</p>' +
        '</div>' +
        '<div class="wall-evt">' +
          '<div class="wall-pop">' +
            '<i class="wall-hl"></i>' +
            '<p class="wall-opt wall-keep">' + icon('lock', 'wall-navy') + '<span>Versa API Azure</span>' + icon('check', 'wall-end wall-check') + '</p>' +
            '<div class="wall-barred">' +
              '<p class="wall-opt"><span class="wall-gap"></span><span>Claude</span><span class="wall-quiet wall-end">Public</span></p>' +
              '<div class="wall-why"><p>' + say('Only private models may run in a private chat.', 0, 'Unavailable: this is a private chat, so only private models may run in it.') + '</p></div>' +
            '</div>' +
          '</div>' +
        '</div>' +
        composer('Versa API Azure', true) +
      '</div>' +
    '</div></div>';

  var PUBLIC =
    '<div class="wall-layer wall-pub"><div class="wall-pane">' +
      '<div class="wall-head">' +
        '<span class="privacy is-public wall-badge">Public</span>' +
        title('Statin literature scan', 'Literature scan') +
      '</div>' +
      '<div class="wall-body">' +
        '<div class="wall-msgs">' +
          '<p class="wall-user wall-m-user">' + say('Find recent trials on statins and cognition.', 'Find trials on statins and cognition.', 0, 'Statins and cognition?') + '</p>' +
          '<p class="wall-tool wall-m-tool">' + icon('search') + '<span class="wall-tx"><span class="wall-tt">' + say('Ran LiteratureAgent', 'Ran PubMed search', 'Ran LiteratureAgent · PubMed search') + '</span></span></p>' +
          '<p class="wall-bot wall-m-bot">23 trials since 2020.</p>' +
        '</div>' +
        '<div class="wall-evt">' +
          '<div class="wall-req">' +
            '<p class="wall-reqrow">' + icon('chat') + '<span>Read another chat</span></p>' +
            '<i class="wall-lane"><i class="wall-ray"><svg class="wall-tip" viewBox="0 0 6 10" aria-hidden="true"><path d="M5 1 1 5l4 4"/></svg></i></i>' +
          '</div>' +
          '<p class="wall-note">' + say('Public models cannot read private chats. Nothing was read.', 'Public models cannot read private chats.') + '</p>' +
        '</div>' +
        composer('Claude', false) +
      '</div>' +
    '</div></div>';

  var WALL = '<div class="wall-line"></div><div class="wall-handle">' + icon('left', 'wall-chev') + icon('lock', 'wall-hlock') + icon('right', 'wall-chev') + '</div>';

  // cubic-bezier(0.24, 1, 0.4, 1), the site's one curve, for the wall's own moves.
  function ease(t) {
    var lo = 0, hi = 1, s = t;
    for (var i = 0; i < 24; i++) {
      s = (lo + hi) / 2;
      var x = 3 * (1 - s) * (1 - s) * s * 0.24 + 3 * (1 - s) * s * s * 0.4 + s * s * s;
      if (x < t) lo = s; else hi = s;
    }
    return 3 * (1 - s) * (1 - s) * s + 3 * (1 - s) * s * s + s * s * s;
  }

  BR.art('wall', function (root, opts) {
    var reduced = !!(opts && opts.reduced);

    // The frame holds a real control, so it is a labelled group rather than an
    // image: an img role would make the slider inside it presentational.
    root.setAttribute('role', 'group');
    if (reduced) root.classList.add('wall-still');
    root.innerHTML =
      '<div class="wall-stage" aria-hidden="true">' + PUBLIC + PRIVATE + WALL + '</div>' +
      '<input class="wall-input" type="range" min="0" max="100" value="50" step="1" ' +
      'aria-valuemin="' + MIN + '" aria-valuemax="' + MAX + '" ' +
      'aria-label="Move the wall between the private chat and the public chat">';

    var q = function (s) { return root.querySelector(s); };
    var stage = q('.wall-stage');
    var input = q('.wall-input');
    var chip = q('.wall-priv .wall-chip');
    var pop = q('.wall-pop');
    var why = q('.wall-why');
    var req = q('.wall-req');
    var note = q('.wall-note');
    var lane = q('.wall-lane');
    var handle = q('.wall-handle');

    function paneParts(side) {
      var pane = q('.wall-' + side + ' .wall-pane');
      return {
        side: side, pane: pane,
        head: pane.querySelector('.wall-head'),
        title: pane.querySelector('.wall-title'),
        body: pane.querySelector('.wall-body'),
        msgs: pane.querySelector('.wall-msgs'),
        user: pane.querySelector('.wall-m-user'),
        tool: pane.querySelector('.wall-m-tool'),
        evt: pane.querySelector('.wall-evt'),
        rows: pane.querySelectorAll('.wall-opt, .wall-reqrow'),
        comp: pane.querySelector('.wall-comp'),
        ph: pane.querySelector('.wall-ph'),
        // The reference arrangement (see the note at the top), and geometry
        // that only changes with the frame's size; both set by reference().
        refEvt: 'full', refDrops: 0, refPh: 0,
        headH: 0, headOuter: 0, padB: 0, phI: 0,
        phLevel: -1, drops: -1, lv: -1
      };
    }
    var panes = [paneParts('priv'), paneParts('pub')];
    var W = 0, H = 0, hr = 0;

    function shown(el) { return el.offsetParent !== null; }
    function bottomOf(el) { return el.offsetTop + el.offsetHeight; }
    function overflows(el) { return el.scrollWidth > el.clientWidth + 1; }
    // Lines of an element whose text sits in one visible inline span.
    function lines(el) {
      var kids = el.children.length ? el.children : [el];
      for (var i = 0; i < kids.length; i++) { var n = kids[i].getClientRects().length; if (n) return n; }
      return 0;
    }
    function setClass(P, prefix, value, key) {
      if (P[key] === value) return;
      if (P[key] !== -1) P.pane.classList.remove(prefix + P[key]);
      P.pane.classList.add(prefix + value);
      P[key] = value;
    }
    function setPh(P, i) { setClass(P, 'wall-phx-', PH[i], 'phLevel'); P.phI = i; }
    function setDrops(P, d) { setClass(P, 'wall-drop', d, 'drops'); }
    function setLevel(P, v) { setClass(P, 'wall-sp', v, 'lv'); }
    function tallTool(P) {
      return shown(P.tool) && P.tool.offsetHeight > (T_LINES + 0.5) * parseFloat(getComputedStyle(P.tool).lineHeight);
    }
    // A title fits in at most two lines, clear of the header band's edges by
    // 6px, inside the header's padding (a narrow chat's is smaller: wall.css).
    function titleFits(P) {
      var t = P.title, tt = t.offsetTop, tb = tt + t.offsetHeight;
      if (lines(t) > 2 || tt < 6 || tb > P.headH - 6 || overflows(t)) return false;
      var hs = getComputedStyle(P.head);
      return t.offsetLeft >= parseFloat(hs.paddingLeft) - 0.5 &&
        t.offsetLeft + t.offsetWidth <= P.head.clientWidth - parseFloat(hs.paddingRight) + 0.5;
    }
    function msgsOverflow(P) {
      var w = P.msgs.clientWidth, kids = P.msgs.children;
      for (var i = 0; i < kids.length; i++) {
        if (shown(kids[i]) && (kids[i].offsetWidth > w + 1 || overflows(kids[i]))) return true;
      }
      return false;
    }

    // Lay one chat out for the wall's current position.
    function fitPane(P) {
      var cl = P.pane.classList;

      // Title: the long one, else the short one, else none.
      cl.remove('wall-no-title', 'wall-title-s');
      if (shown(P.title) && !titleFits(P)) {
        cl.add('wall-title-s');
        if (!titleFits(P)) cl.add('wall-no-title');
      }

      // Placeholder: the reference spelling, or a shorter one where it does not fit.
      var i = P.refPh;
      setPh(P, i);
      while (i < PH.length - 1 && shown(P.ph) && overflows(P.ph)) setPh(P, ++i);

      // Messages: the reference drops, and more only where this width needs
      // them (the answer, then the tool row, then the question). A narrow
      // chat first takes shorter words: the short spelling, then the tiny
      // one. A tool row still too tall steps aside with the answer; a
      // question still too tall takes the rest with it.
      var compOn = shown(P.comp);
      var limit = compOn ? P.comp.offsetTop - GAP : P.body.clientHeight - P.padB;
      var d = P.refDrops;
      setDrops(P, d);
      var lv = 0;
      setLevel(P, 0);
      while (lv < 2 && ((shown(P.user) && lines(P.user) > Q_SPELL) || tallTool(P))) setLevel(P, ++lv);
      if (d < 2 && tallTool(P)) setDrops(P, d = 2);
      if (shown(P.user) && lines(P.user) > Q_LINES) setDrops(P, d = 3);
      while (d < 3 && (bottomOf(P.msgs) > limit || msgsOverflow(P))) setDrops(P, ++d);

      // The event keeps the reference's form and plays only where it has room.
      cl.toggle('wall-compact', P.refEvt === 'compact');
      cl.toggle('wall-no-evt', !(P.refEvt !== 'none' && compOn && roomAt(P, limit)));
    }

    // Place the event and say whether it fits: clear of the messages, the
    // composer and the handle.
    function roomAt(P, limit) {
      var evt = P.evt;
      if (evt.clientWidth < EVT_MIN) return false;
      for (var i = 0; i < P.rows.length; i++) if (shown(P.rows[i]) && overflows(P.rows[i])) return false;
      if (overflows(evt)) return false;
      var h = evt.offsetHeight;
      var msgEnd = P.msgs.offsetHeight ? bottomOf(P.msgs) : P.msgs.offsetTop;
      // The handle, in this chat's body coordinates.
      var paneLeft = P.side === 'priv' ? 0 : split / 100 * W;
      var hx = split / 100 * W - 1 - paneLeft;
      var hy = H / 2 - P.headOuter;
      var top;
      if (P.side === 'priv') {
        // The picker rises from the model chip, above the composer, and grows
        // upward when the reason opens (wall.css), so it is measured open.
        var shut = why.scrollHeight - why.clientHeight;
        h += shut;
        top = limit - h;
        evt.style.bottom = Math.round(P.body.clientHeight - limit) + 'px';
        if (top < msgEnd + GAP) return false;
        var x0 = evt.offsetLeft, x1 = x0 + pop.offsetWidth, rad = 12;
        var dx = Math.max(x0 + rad - hx, 0, hx - (x1 - rad));
        var dy = Math.max(top + rad - hy, 0, hy - (top + pop.offsetHeight + shut - rad));
        return Math.sqrt(dx * dx + dy * dy) - rad - hr >= 4;
      }
      // The request is the next row of its own transcript. Its ray ends at the
      // wall, so it (and a request that starts close to the wall, all of it)
      // passes above or below the handle, never under it.
      top = msgEnd + GAP + 4;
      var near = evt.offsetLeft - hx < hr + 8;
      var z0 = near ? 0 : req.offsetTop + lane.offsetTop;
      var z1 = near ? h : z0 + lane.offsetHeight;
      var b0 = hy - hr - 6, b1 = hy + hr + 6;
      if (top + z0 < b1 && top + z1 > b0) top = b1 - z0;
      evt.style.top = Math.round(top) + 'px';
      return top + h <= limit;
    }

    function hasRoom(i) { return !panes[i].pane.classList.contains('wall-no-evt'); }

    // Measure the frame, then choose each chat's reference arrangement with
    // the wall in the middle, and the placeholder that fits across the
    // first-view hint (44 to 56). Runs at mount and when the frame resizes.
    var LADDER = {
      priv: [['full', 0], ['compact', 0], ['compact', 1], ['compact', 2]],
      pub: [['full', 0], ['full', 1], ['full', 2]]
    };
    function reference() {
      W = stage.clientWidth; H = stage.clientHeight;
      hr = handle.offsetWidth / 2;
      var keep = split;
      panes.forEach(function (P) {
        var bs = getComputedStyle(P.body);
        P.headH = P.head.clientHeight; P.headOuter = P.head.offsetHeight;
        P.padB = parseFloat(bs.paddingBottom);
      });
      place(50);
      panes.forEach(function (P, n) {
        P.refPh = 0;
        var steps = LADDER[P.side], found = false;
        for (var i = 0; i < steps.length && !found; i++) {
          P.refEvt = steps[i][0]; P.refDrops = steps[i][1];
          fitPane(P);
          found = P.drops === P.refDrops && hasRoom(n);
        }
        if (!found) { P.refEvt = 'none'; P.refDrops = 0; }
      });
      // Both composers share one spelling: the longest that fits either chat
      // at its narrowest point in the hint.
      var ph = 0;
      [[44, 0], [56, 1]].forEach(function (s) {
        place(s[0]);
        fitPane(panes[s[1]]);
        ph = Math.max(ph, panes[s[1]].phI);
      });
      panes.forEach(function (P) { P.refPh = ph; });
      place(keep);
      fitted = -1;
      fit();
    }
    function place(v) { split = v; root.style.setProperty('--split', v + '%'); }

    function fit() {
      if (fitted === split) return;
      fitted = split;
      panes.forEach(fitPane);
    }
    var fitted = -1, fitRaf = 0;
    function flushFit() { if (fitRaf) { cancelAnimationFrame(fitRaf); fitRaf = 0; } fit(); }

    var split = 50;
    // The wall moves at once; the chats are laid out once per frame, before
    // it is painted. With `now` they are laid out at once (the wall's own
    // moves, which already run once per frame).
    function setSplit(v, now) {
      v = Math.max(MIN, Math.min(MAX, v));
      place(v);
      var r = Math.round(v);
      if (+input.value !== r) input.value = String(r);
      input.setAttribute('aria-valuetext', 'Private chat ' + r + ' percent, public chat ' + (100 - r) + ' percent');
      if (now) flushFit();
      else if (!fitRaf) fitRaf = requestAnimationFrame(function () { fitRaf = 0; fit(); });
    }

    setSplit(50, false);
    reference();
    if ('ResizeObserver' in window) {
      var size = W + 'x' + H;
      new ResizeObserver(function () {
        var s = stage.clientWidth + 'x' + stage.clientHeight;
        if (s !== size) { size = s; reference(); }
      }).observe(stage);
    }

    function on(el, cls, v) { el.classList.toggle(cls, v); }
    function reset() {
      on(chip, 'is-open', false); on(pop, 'is-on', false); on(pop, 'is-try', false); on(pop, 'is-why', false);
      on(req, 'is-on', false); on(req, 'is-run', false); on(req, 'is-go', false);
      on(note, 'is-on', false);
    }

    // The visitor can always move the wall; with reduced motion nothing else moves.
    var paused = false, hinted = false, running = false;
    var timers = [], idle = 0, tw = null, tp = null;
    function at(ms, fn) { timers.push(setTimeout(fn, ms)); }
    function clear() { timers.forEach(clearTimeout); timers = []; }

    // The wall's own moves: keys are [split, ms] pairs, eased between.
    function tween(keys, done, isHint) {
      stopTween();
      var t0 = 0, me = { hint: !!isHint, raf: 0 };
      tw = me;
      me.raf = requestAnimationFrame(function frame(now) {
        if (tw !== me) return;
        if (!t0) t0 = now;
        var t = now - t0;
        for (var i = 1; i < keys.length; i++) {
          if (t < keys[i][1]) {
            var a = keys[i - 1], b = keys[i];
            setSplit(a[0] + (b[0] - a[0]) * ease((t - a[1]) / (b[1] - a[1])), true);
            me.raf = requestAnimationFrame(frame);
            return;
          }
        }
        tw = null;
        setSplit(keys[keys.length - 1][0], true);
        done();
      });
    }
    function stopTween() { if (tw) { cancelAnimationFrame(tw.raf); tw = null; } }

    function touched() {
      hinted = true;
      stopTween();
      if (reduced) return;
      if (!paused) { paused = true; clear(); reset(); }
      clearTimeout(idle);
      idle = setTimeout(function () { paused = false; if (running) at(300, cycle); }, 4000);
    }
    function fromX(x) {
      var r = stage.getBoundingClientRect();
      if (r.width) setSplit((x - r.left) / r.width * 100);
    }
    // Chromium's range jumps to a finger the moment it lands on the track,
    // before anyone knows whether the finger is scrolling the page. While a
    // touch is still undecided (tp, below) that jump is undone, so the wall
    // stays where it was.
    input.addEventListener('input', function () {
      if (tp && !tp.drag) { input.value = String(Math.round(split)); return; }
      setSplit(+input.value); touched();
    });
    input.addEventListener('keydown', function (e) { if (/^(Arrow|Page|Home|End)/.test(e.key)) touched(); });
    // Mouse and pen: the range itself follows the pointer.
    // Touch: a finger is watched first, wherever it lands. A swipe that goes
    // up or down is the page scrolling (the browser takes it and cancels the
    // pointer) and leaves the wall alone; only one that goes sideways moves
    // the wall. A tap moves the wall to the finger. The range's thumb is as
    // wide as the handle and sits under it (wall.css), so where the browser
    // drags a range only from its thumb (iOS Safari), a finger on the wall
    // still drags it.
    input.addEventListener('pointerdown', function (e) {
      if (e.pointerType !== 'touch') { touched(); return; }
      tp = { id: e.pointerId, x: e.clientX, y: e.clientY, drag: false };
    });
    input.addEventListener('pointermove', function (e) {
      if (!tp || e.pointerId !== tp.id) return;
      if (!tp.drag) {
        var dx = Math.abs(e.clientX - tp.x), dy = Math.abs(e.clientY - tp.y);
        if (dx <= SLOP || dx <= dy) return;
        tp.drag = true;
      }
      touched();
      fromX(e.clientX);
    });
    input.addEventListener('pointerup', function (e) {
      if (!tp || e.pointerId !== tp.id) return;
      var p = tp; tp = null;
      if (p.drag) return;
      if (Math.abs(e.clientX - p.x) < SLOP && Math.abs(e.clientY - p.y) < SLOP) { touched(); fromX(e.clientX); }
      else input.value = String(Math.round(split));
    });
    input.addEventListener('pointercancel', function (e) {
      if (!tp || e.pointerId !== tp.id) return;
      tp = null;
      input.value = String(Math.round(split));
    });

    if (reduced) {
      // One still: the refused model on the left, the stopped request on the right.
      on(pop, 'is-on', true); on(pop, 'is-try', true); on(pop, 'is-why', true); on(chip, 'is-open', true);
      on(req, 'is-on', true); on(req, 'is-go', true); on(note, 'is-on', true);
      return {};
    }

    // One pass of the loop: about 12.5s with both events. One thing moves at a time.
    function cycle() {
      clear();
      if (!running || paused) return;
      flushFit();
      // An event has no room where the visitor left the wall: ease it back to
      // the middle first, where both do.
      var want0 = panes[0].refEvt !== 'none', want1 = panes[1].refEvt !== 'none';
      if (Math.abs(split - 50) > 0.5 && ((want0 && !hasRoom(0)) || (want1 && !hasRoom(1)))) {
        tween([[split, 0], [50, 525]], function () { at(300, cycle); });
        return;
      }
      var t = 0;
      if (hasRoom(0)) {
        at(700, function () { on(chip, 'is-open', true); on(pop, 'is-on', true); });
        at(1700, function () { on(pop, 'is-try', true); });       // the selection tries Claude
        at(2250, function () { on(pop, 'is-why', true); });       // and the reason appears
        at(3900, function () { on(pop, 'is-try', false); });      // back to Versa API Azure
        at(4800, function () { on(pop, 'is-on', false); on(chip, 'is-open', false); });
        at(5100, function () { on(pop, 'is-why', false); });
        t = 5100;
      }
      if (hasRoom(1)) {
        var b = t + 700;
        at(b, function () { on(req, 'is-on', true); on(req, 'is-run', true); });
        at(b + 600, function () { on(req, 'is-go', true); });              // 1.1s, linear
        at(b + 1700, function () { on(req, 'is-run', false); });           // it has hit the wall
        at(b + 1900, function () { on(note, 'is-on', true); });
        at(b + 4900, function () { on(req, 'is-on', false); on(note, 'is-on', false); });
        at(b + 5500, function () { on(req, 'is-go', false); });
        t = b + 5500;
      }
      at(Math.max(t + 1200, 8600), cycle);
    }

    return {
      start: function () {
        if (running) return;
        running = true;
        if (paused) return;
        // A one-time hint at first view: the wall moves 50, 44, 56, 50.
        if (!hinted) at(500, function () {
          hinted = true;
          tween([[50, 0], [44, 650], [56, 1600], [50, 2250]], function () { at(500, cycle); }, true);
        });
        else at(300, cycle);
      },
      stop: function () {
        running = false; paused = false;
        clear(); clearTimeout(idle);
        if (tw) { if (tw.hint) hinted = false; stopTween(); setSplit(50, true); }
        reset();
      }
    };
  });
})();
