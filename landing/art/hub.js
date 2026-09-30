/* Hero artifact: one interface, every model.
   The person opens the composer's model chip, picks a model, types and sends.
   A coral pulse carries the turn from the composer through Biorouter to the
   chosen row of the app's provider catalogue, and the reply streams back into
   the chat. Three turns in ONE chat, each on a model the person picked:
   Public (Claude Code), Institutional (Versa API Azure), Local (Llama Server).
   Public comes first because a chat that has run a turn on a private model
   stays private: in the third turn the picker shows Claude Code as
   unavailable, as the app's model picker does. The loop then starts a new chat
   (the transcript clears and "New chat" shows) before any public turn runs.
   Groups, names and tiers follow crates/biorouter/src/providers/factory.rs,
   providerOrdering.ts and SwitchModelModal.tsx. */
(function () {
  'use strict';
  if (!window.BR || !BR.art) return;

  var NS = 'http://www.w3.org/2000/svg';
  var BRAIN = '<path d="M12 18V5"/><path d="M15 13a4.17 4.17 0 0 1-3-4 4.17 4.17 0 0 1-3 4"/><path d="M17.598 6.5A3 3 0 1 0 12 5a3 3 0 1 0-5.598 1.5"/><path d="M17.997 5.125a4 4 0 0 1 2.526 5.77"/><path d="M18 18a4 4 0 0 0 2-7.464"/><path d="M19.967 17.483A4 4 0 1 1 12 18a4 4 0 1 1-7.967-.517"/><path d="M6 18a4 4 0 0 1-2-7.464"/><path d="M6.003 5.125a4 4 0 0 0-2.526 5.77"/>';
  var UP = '<path d="m5 12 7-7 7 7"/><path d="M12 19V5"/>';
  function svgIcon(body) {
    return '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">' + body + '</svg>';
  }

  /* The catalogue, in the app's order. 28 providers ship (23 built in plus 5
     bundled declarative ones); nine are named here, so 19 more are Public.
     "Your subscription" shortens the app's "AI agents · your subscription",
     which does not fit a half column. In the narrow layout "UCSF" moves
     beside its group (`aside`, so its sublabel is wide only) and the two
     Public sublabels stay, one above each line of rows. */
  var GROUPS = [
    { key: 'local', label: 'Local', cols: [
      { rows: [
        { id: 'llama', name: 'Llama Server', note: 'Gemma, Qwen', lock: true },
        { id: 'ollama', name: 'Ollama', lock: true }
      ] }
    ] },
    { key: 'inst', label: 'Institutional', aside: 'UCSF', cols: [
      { sub: 'UCSF', wideOnly: true, rows: [
        { id: 'azure', name: 'Versa API Azure', lock: true },
        { id: 'bedrock', name: 'Versa API Bedrock', lock: true }
      ] }
    ] },
    { key: 'public', label: 'Public', cols: [
      { sub: 'Your subscription', rows: [
        { id: 'claude', name: 'Claude Code' },
        { id: 'codex', name: 'Codex' }
      ] },
      { sub: 'API providers', rows: [
        { name: 'Anthropic' },
        { name: 'OpenAI' },
        { name: 'Google Gemini' },
        { name: '+ 19 more', more: true }
      ] }
    ] }
  ];

  var TURNS = [
    { model: 'Claude Code', lock: false, row: 'claude',
      prompt: 'Summarize these 40 abstracts',
      reply: 'Three themes recur: treatment response, imaging markers and disability.' },
    { model: 'Versa API Azure', lock: true, row: 'azure',
      prompt: 'Find relapses in these notes',
      reply: 'Two admissions for relapse. Each was treated with steroids and improved.' },
    { model: 'Llama Server', lock: true, row: 'llama',
      prompt: 'Draft a methods section',
      reply: 'We enrolled 212 participants at two sites and drew blood at baseline.' }
  ];
  // The chip's menu: the models this chat uses, in the catalogue's order.
  var PICK = [
    { row: 'llama', name: 'Llama Server', lock: true },
    { row: 'azure', name: 'Versa API Azure', lock: true },
    { row: 'claude', name: 'Claude Code' }
  ];
  var ORDER = PICK.map(function (p) { return p.row; });
  // A new chat opens on the default model. It has no tier until a turn runs,
  // so the person may move it to a public model before the first message.
  var NEW_CHAT = { model: 'Llama Server', lock: true, row: 'llama' };
  var STILL = 1; // the reduced-motion picture: the institutional turn, one public turn above it

  function el(tag, cls, html) {
    var e = document.createElement(tag);
    if (cls) e.className = cls;
    if (html != null) e.innerHTML = html;
    return e;
  }
  function clamp(v, a, b) { return Math.max(a, Math.min(b, v)); }
  function easeInOut(p) { return 0.5 - 0.5 * Math.cos(Math.PI * p); }
  function easeOut(p) { return 1 - Math.pow(1 - p, 3); }
  // A turn is private once any earlier turn in the chat ran on a private model.
  function chatIsPrivate(ti) { for (var i = 0; i < ti; i++) if (TURNS[i].lock) return true; return false; }

  BR.art('hub', function (root, opts) {
    var reduced = !!(opts && opts.reduced);

    /* ── Build ─────────────────────────────────────────────────── */
    var hub = el('div', 'hub');
    hub.setAttribute('aria-hidden', 'true');

    var svg = document.createElementNS(NS, 'svg');
    svg.setAttribute('class', 'hub-lines');
    function mkPath(cls) { var p = document.createElementNS(NS, 'path'); if (cls) p.setAttribute('class', cls); svg.appendChild(p); return p; }
    var linkPath = mkPath();
    var spokePaths = GROUPS.map(function () { return mkPath(); });
    var trunkPath = mkPath();
    var stubPaths = GROUPS.map(function () { return mkPath(); });
    var spinePaths = GROUPS.map(function () { return mkPath(); });
    var routePath = mkPath('hub-route');
    var pulsePath = mkPath('hub-pulse');
    hub.appendChild(svg);

    // The chat: a transcript that scrolls, the composer card, the model chip.
    var log = el('div', 'hub-log');
    var stack = el('div', 'hub-stack');
    var fresh = el('div', 'hub-new', 'New chat');
    log.appendChild(stack); log.appendChild(fresh);
    var card = el('div', 'hub-card');
    var input = el('div', 'hub-input');
    var ph = el('span', 'hub-ph', 'Ask Biorouter anything…');
    var typed = el('span', 'hub-typed');
    var caret = el('span', 'hub-caret');
    input.appendChild(ph); input.appendChild(typed); input.appendChild(caret);
    var send = el('div', 'hub-send', svgIcon(UP));
    card.appendChild(input); card.appendChild(send);

    // The chip's menu, opening above the chip as the app's does.
    var pick = el('div', 'hub-pick');
    var prows = {};
    PICK.forEach(function (p) {
      var r = el('div', 'hub-prow');
      r.appendChild(el('span', 'label', p.name));
      if (p.lock) r.appendChild(el('span', 'hub-lock'));
      else r.appendChild(el('span', 'note', 'Unavailable'));
      pick.appendChild(r);
      prows[p.row] = r;
    });

    var controls = el('div', 'hub-controls');
    var chip = el('div', 'hub-chip', svgIcon(BRAIN));
    var chipModel = el('span', 'model');
    var chipName = el('span', 'name');
    var chipLock = el('span', 'hub-lock');
    chipModel.appendChild(chipName); chipModel.appendChild(chipLock);
    chip.appendChild(chipModel);
    controls.appendChild(chip);

    var mark = el('div', 'hub-mark',
      '<svg viewBox="-24.5235 -110.806 165.672 165.672">' +
      '<text class="navy" x="0" y="0" font-size="100">B</text>' +
      '<text class="coral" x="68.140625" y="0" font-size="74">R</text>' +
      '<rect class="navy" x="0" y="23.36" width="66.640625" height="17.7"/>' +
      '<rect class="coral" x="66.640625" y="23.36" width="49.984375" height="17.7"/></svg>');

    var cat = el('div', 'hub-cat');
    var rows = {}, groupEls = [], dotEls = [];
    GROUPS.forEach(function (g) {
      var ge = el('div', 'hub-group' + (g.cols.length > 1 ? ' two' : ''));
      var gh = el('div', 'hub-gh');
      var dot = el('i', 'dot ' + g.key);
      gh.appendChild(dot);
      gh.appendChild(el('span', null, g.label));
      if (g.aside) gh.appendChild(el('span', 'aside', g.aside));
      ge.appendChild(gh);
      var cols = el('div', 'hub-cols');
      g.cols.forEach(function (c) {
        var ce = el('div', 'hub-col');
        if (c.sub) ce.appendChild(el('div', 'hub-sl' + (c.wideOnly ? ' wo' : ''), c.sub));
        var items = el('div', 'hub-items');
        c.rows.forEach(function (r) {
          var re = el('div', 'hub-row' + (r.more ? ' more' : ''));
          re.appendChild(el('span', 'label', r.name));
          if (r.note) re.appendChild(el('span', 'note', r.note));
          if (r.lock) re.appendChild(el('span', 'hub-lock'));
          items.appendChild(re);
          if (r.id) rows[r.id] = re;
        });
        ce.appendChild(items);
        cols.appendChild(ce);
      });
      ge.appendChild(cols);
      cat.appendChild(ge);
      groupEls.push(ge); dotEls.push(dot);
    });

    var measure = el('div', 'hub-measure');
    var probe = el('span', 'hub-probe');
    measure.appendChild(probe);

    hub.appendChild(log); hub.appendChild(card); hub.appendChild(pick); hub.appendChild(controls);
    hub.appendChild(mark); hub.appendChild(cat); hub.appendChild(measure);
    root.appendChild(hub);

    function makeTurn(t, shown) {
      var turn = el('div', 'hub-turn');
      turn.appendChild(el('div', 'hub-user', t.prompt));
      var p = el('p', 'hub-reply');
      t.reply.split(' ').forEach(function (w, i, a) {
        var s = el('span', 'w' + (shown ? ' on' : ''), w);
        p.appendChild(s);
        if (i < a.length - 1) p.appendChild(document.createTextNode(' '));
      });
      turn.appendChild(p);
      return turn;
    }

    /* ── Layout ───────────────────────────────────────────────── */
    var geo = null, logGap = 14;
    function box(e) {
      var r = e.getBoundingClientRect(), R = root.getBoundingClientRect();
      return { x: r.left - R.left - root.clientLeft, y: r.top - R.top - root.clientTop, w: r.width, h: r.height };
    }
    function place(e, x, y, w, h) {
      e.style.left = Math.round(x) + 'px'; e.style.top = Math.round(y) + 'px';
      if (w != null) e.style.width = Math.round(w) + 'px';
      if (h != null) e.style.height = Math.round(h) + 'px';
    }
    function f(n) { return Math.round(n * 10) / 10; }
    function textW(s, size) { probe.style.fontSize = size + 'px'; probe.textContent = s; return probe.getBoundingClientRect().width; }

    // A message leaves the view whole: a bubble or a reply whose top has
    // passed the transcript's top edge fades out as one block, so no half
    // paragraph is ever left at the top. `dy` is how far the stack still sits
    // below its resting place while a scroll runs; units are judged where
    // they will come to rest, so they fade as the scroll starts.
    function clipLog(dy) {
      var L = log.getBoundingClientRect(), R = root.getBoundingClientRect();
      var edge = Math.max(L.top - 1, R.top + 2);
      var units = stack.querySelectorAll('.hub-user, .hub-reply');
      for (var i = 0; i < units.length; i++) {
        var r = units[i].getBoundingClientRect();
        units[i].classList.toggle('gone', r.top - (dy || 0) < edge);
      }
    }

    function layout() {
      var w = root.clientWidth, h = root.clientHeight;
      if (!w || !h) return;
      // Side by side needs room for the composer, the mark, the catalogue and
      // the connectors between them; below that everything stacks.
      var narrow = w < 670 || (w < 760 && h > w * 0.7);
      hub.classList.toggle('narrow', narrow);
      hub.classList.remove('tight');
      hub.style.setProperty('--hub-row', (narrow ? 24 : h >= 440 ? 28 : h >= 390 ? 26 : 24) + 'px');
      hub.style.setProperty('--hub-gap', (narrow ? 10 : h >= 440 ? 20 : h >= 390 ? 14 : 10) + 'px');

      var pad, chatW, chatX, markS, markX, markY, catX, catW, cardH, cardY, G, tm;
      var longest = 0;
      TURNS.forEach(function (t) { longest = Math.max(longest, textW(t.prompt, 13.5)); });

      function sizeCard(width) {
        var inputW = width - 2 - 14 - 10 - 32 - 8;
        var lines = longest + 3 > inputW ? 2 : 1;
        input.style.height = (lines * 20 + 12) + 'px';
        return lines * 20 + 12 + 20 + 2;
      }
      // Heights at this width: the tallest turn (`one`), and the tallest turn
      // together with the whole reply before it (`two`), so the chat can show
      // the previous answer above the current exchange when there is room.
      function turnMetrics(width) {
        var th = [], rh = [];
        TURNS.forEach(function (t) {
          var tt = makeTurn(t, true);
          tt.style.width = width + 'px';
          measure.appendChild(tt);
          th.push(tt.getBoundingClientRect().height);
          rh.push(tt.querySelector('.hub-reply').getBoundingClientRect().height);
          measure.removeChild(tt);
        });
        var one = 0, two = 0;
        for (var i = 0; i < th.length; i++) {
          one = Math.max(one, th[i]);
          if (i) two = Math.max(two, th[i] + 14 + rh[i - 1]);
        }
        return { one: Math.ceil(one), two: Math.ceil(two) };
      }
      // The menu is as wide as its widest state (Claude Code unavailable) and
      // at least wide enough to cover the placeholder it opens over.
      var wasOff = prows.claude.classList.contains('off');
      pick.style.width = '';
      prows.claude.classList.add('off');
      var pb = pick.getBoundingClientRect();
      prows.claude.classList.toggle('off', wasOff);
      var pickW = Math.ceil(pb.width), pickH = Math.ceil(pb.height);
      var phW = textW(ph.textContent, 13.5);
      function placePick() {
        var px = chatX + 6;
        var pw = Math.max(pickW, Math.ceil(chatX + 15 + phW + 10 - px));
        // sits over the composer, its bottom a little inside the card's
        place(pick, px, cardY + cardH - 5 - pickH, pw);
      }
      // The transcript ends far enough above the card that the open menu
      // keeps a clear 14px from the last line of the chat.
      function gapFor(ch) { return Math.max(14, pickH + 5 + 14 - ch); }

      if (!narrow) {
        pad = clamp(Math.round(w * 0.04), 24, 44);
        markS = w >= 900 ? 72 : w >= 700 ? 64 : 56;
        catW = clamp(Math.round(w * 0.3), 262, 316);
        chatW = clamp(Math.round(w * 0.31), 228, 300);
        // widen the composer to hold a prompt on one line when there is room,
        // letting the catalogue give up to 22px for it
        var oneLine = Math.ceil(longest) + 3 + 2 + 14 + 10 + 32 + 8;
        if (oneLine > chatW) {
          var room = w - 2 * pad - markS - 80;
          var give = clamp(oneLine - (room - catW), 0, 22);
          catW -= give;
          chatW = Math.min(300, Math.max(chatW, Math.min(oneLine, room - catW)));
        }
        var rem = w - 2 * pad - catW - chatW - markS;
        chatX = pad;
        markX = pad + chatW + rem * 0.42;
        catX = w - pad - catW;
        cardH = sizeCard(chatW);
        G = gapFor(cardH);
        tm = turnMetrics(chatW);
        // The composer sits a little below the middle; it moves lower, as far
        // as the chip allows, when that lets the whole previous reply show.
        var half = cardH / 2;
        var cy = Math.round(h * 0.52);
        var cyMax = Math.round(h - pad - 32 - half);
        var cyTwo = Math.round(pad + tm.two + 4 + G + half);
        var cyOne = Math.round(pad + tm.one + 14 + G + half);
        cy = Math.min(Math.max(cy, cyTwo <= cyMax ? cyTwo : cyOne), cyMax);
        cardY = cy - half;
        place(card, chatX, cardY, chatW, cardH);
        place(controls, chatX, cardY + cardH + 4);
        place(log, chatX, pad, chatW, cardY - G - pad);
        hub.style.setProperty('--hub-fade', '26px');
        markY = cy - markS / 2;
        place(mark, markX, markY, markS, markS);
        cat.style.width = catW + 'px';
        var catH = cat.getBoundingClientRect().height;
        place(cat, catX, Math.max(12, (h - catH) / 2));
      } else {
        pad = w < 320 ? 14 : 18;
        // one centred column; on a wide phone frame it stops at a readable width
        chatW = Math.min(w - 2 * pad, 440);
        chatX = (w - chatW) / 2;
        cardH = sizeCard(chatW);
        G = gapFor(cardH);
        tm = turnMetrics(chatW);
        // the catalogue shares the composer's left edge: its dots sit under
        // the chip's icon, and the trunk runs in the gutter left of them
        catX = chatX + 13;
        catW = chatW - 13;
        cat.style.width = catW + 'px';
        var catH2, g1, g2, padB, L;
        var fixed = function () { return pad + G + cardH + 4 + 28 + g1 + markS + g2 + catH2 + padB; };
        // The transcript holds the current exchange below its top fade, or
        // also the reply before it when there is room. Small frames tighten
        // the rows and gaps, step by step, before anything is cut.
        var needOne = tm.one + 14, needTwo = tm.two + 4;
        var steps = [
          { tight: false, row: 24, gap: 10, markS: 48, g1: 12, g2: 26, padB: 18 },
          { tight: false, row: 24, gap: 10, markS: 48, g1: 8, g2: 20, padB: 14 },
          { tight: true, row: 22, gap: 6, markS: 44, g1: 10, g2: 22, padB: 16 },
          { tight: true, row: 22, gap: 6, markS: 40, g1: 6, g2: 16, padB: 12 }
        ];
        var Ls = steps.map(function (s) {
          hub.classList.toggle('tight', s.tight);
          hub.style.setProperty('--hub-row', s.row + 'px');
          hub.style.setProperty('--hub-gap', s.gap + 'px');
          catH2 = cat.getBoundingClientRect().height;
          markS = s.markS; g1 = s.g1; g2 = s.g2; padB = s.padB;
          return { s: s, cat: catH2, L: h - fixed() };
        });
        // The previous reply is worth slimmer gaps, not slimmer rows.
        var pickStep = null;
        for (var si = 0; si < 2 && !pickStep; si++) if (Ls[si].L >= needTwo) pickStep = Ls[si];
        for (si = 0; si < Ls.length && !pickStep; si++) if (Ls[si].L >= needOne) pickStep = Ls[si];
        pickStep = pickStep || Ls[Ls.length - 1];
        var s = pickStep.s;
        hub.classList.toggle('tight', s.tight);
        hub.style.setProperty('--hub-row', s.row + 'px');
        hub.style.setProperty('--hub-gap', s.gap + 'px');
        catH2 = pickStep.cat; markS = s.markS; g1 = s.g1; g2 = s.g2; padB = s.padB;
        L = pickStep.L;
        var two = L >= needTwo;
        var need = two ? needTwo : Math.min(needOne, Math.max(tm.one, L));
        var extra = Math.max(0, L - need);
        var logH = need + extra * 0.45;
        g1 += extra * 0.2; g2 += extra * 0.25;
        // The top fade dims only history, never the current exchange.
        hub.style.setProperty('--hub-fade', (two ? 18 : clamp(Math.floor(logH - tm.one), 4, 18)) + 'px');
        place(log, chatX, pad, chatW, logH);
        cardY = pad + logH + G;
        place(card, chatX, cardY, chatW, cardH);
        place(controls, chatX, cardY + cardH + 4);
        markX = (w - markS) / 2;
        markY = cardY + cardH + 4 + 28 + g1;
        place(mark, markX, markY, markS, markS);
        place(cat, catX, markY + markS + g2);
      }
      placePick();
      logGap = G;

      // Connectors, measured from what was placed
      svg.setAttribute('width', w); svg.setAttribute('height', h);
      svg.setAttribute('viewBox', '0 0 ' + w + ' ' + h);
      var cb = box(card), mb = box(mark);
      var dots = dotEls.map(function (d) { var b = box(d); return { x: b.x + b.w / 2, y: b.y + b.h / 2 }; });
      function lastRowY(ge, allCols) {
        var scope = allCols ? ge : ge.querySelector('.hub-col');
        var y = 0;
        scope.querySelectorAll('.hub-row').forEach(function (r) { var b = box(r); y = Math.max(y, b.y + b.h / 2); });
        return y;
      }
      var link, spokes = ['', '', ''], stubs = ['', '', ''], spines = [], trunk = '', curve = '', tree = false, tx = 0, ym = 0;
      if (!narrow) {
        var x1 = cb.x + cb.w, x2 = mb.x;
        ym = mb.y + mb.h / 2;
        link = 'M' + f(x1) + ' ' + f(ym) + 'H' + f(x2);
        var sx = mb.x + mb.w;
        // With a wide span the groups fan out on curves. When the mark sits
        // close to the catalogue (tablet widths) the curves would climb on top
        // of each other, so the groups hang off one trunk instead.
        tree = dots[0].x - sx < 110;
        if (!tree) {
          dots.forEach(function (d, i) {
            var k = (d.x - sx) * 0.5;
            spokes[i] = 'M' + f(sx) + ' ' + f(ym) + 'C' + f(sx + k) + ' ' + f(ym) + ' ' + f(d.x - k) + ' ' + f(d.y) + ' ' + f(d.x) + ' ' + f(d.y);
            spines.push('M' + f(d.x) + ' ' + f(d.y) + 'V' + f(lastRowY(groupEls[i], false)));
          });
        } else {
          tx = dots[0].x - 16;
          spokes[0] = 'M' + f(sx) + ' ' + f(ym) + 'H' + f(tx);
          trunk = 'M' + f(tx) + ' ' + f(Math.min(dots[0].y, ym)) + 'V' + f(Math.max(dots[2].y, ym));
          dots.forEach(function (d, i) {
            stubs[i] = 'M' + f(tx) + ' ' + f(d.y) + 'H' + f(d.x - 3);
            spines.push('M' + f(d.x) + ' ' + f(d.y + 3) + 'V' + f(lastRowY(groupEls[i], false)));
          });
        }
      } else {
        // Each group hangs off its own short branch of a trunk in the gutter,
        // so a turn never runs past another group's rows on its way.
        var cx = mb.x + mb.w / 2;
        link = 'M' + f(cx) + ' ' + f(cb.y + cb.h) + 'V' + f(mb.y);
        var sy = mb.y + mb.h;
        tx = chatX + 2.5;
        var yT = dots[0].y - 12;
        var k2 = (yT - sy) * 0.6;
        curve = 'M' + f(cx) + ' ' + f(sy) + 'C' + f(cx) + ' ' + f(sy + k2) + ' ' + f(tx) + ' ' + f(yT - k2) + ' ' + f(tx) + ' ' + f(yT);
        spokes[0] = curve;
        trunk = 'M' + f(tx) + ' ' + f(yT) + 'V' + f(dots[2].y);
        dots.forEach(function (d, i) {
          stubs[i] = 'M' + f(tx) + ' ' + f(d.y) + 'H' + f(d.x - 3);
          spines.push('M' + f(d.x) + ' ' + f(d.y + 3) + 'V' + f(lastRowY(groupEls[i], true)));
        });
      }
      geo = { narrow: narrow, tree: tree, link: link, spokes: spokes, curve: curve, dots: dots, tx: tx, ym: ym, sx: mb.x + mb.w };
      linkPath.setAttribute('d', link);
      spokePaths.forEach(function (p, i) { p.setAttribute('d', spokes[i]); p.style.display = spokes[i] ? '' : 'none'; });
      trunkPath.setAttribute('d', trunk); trunkPath.style.display = trunk ? '' : 'none';
      stubPaths.forEach(function (p, i) { p.setAttribute('d', stubs[i]); p.style.display = stubs[i] ? '' : 'none'; });
      spinePaths.forEach(function (p, i) { p.setAttribute('d', spines[i] || ''); });
      if (current != null) routePath.setAttribute('d', route(current));
      clipLog(0);
    }

    // The path a turn takes: composer → mark → its group → the row. One
    // subpath, so the dash stays one segment; the stretch behind the plate is hidden.
    function route(ti) {
      var t = TURNS[ti];
      var row = rows[t.row];
      var gi = groupEls.indexOf(row.closest('.hub-group'));
      var rb = box(row), ry = rb.y + rb.h / 2;
      var d = geo.dots[gi];
      if (geo.narrow) {
        return geo.link + geo.curve.replace(/^M/, 'L') + 'V' + f(d.y) + 'H' + f(d.x) + 'V' + f(ry);
      }
      if (geo.tree) {
        return geo.link + 'L' + f(geo.sx) + ' ' + f(geo.ym) + 'H' + f(geo.tx) + 'V' + f(d.y) + 'H' + f(d.x) + 'V' + f(ry);
      }
      return geo.link + geo.spokes[gi].replace(/^M/, 'L') + 'V' + f(ry);
    }

    /* ── State helpers ────────────────────────────────────────── */
    var current = null;
    function setChip(t) {
      chipName.textContent = t.model;
      chipLock.hidden = !t.lock;
    }
    function setActive(id) {
      Object.keys(rows).forEach(function (k) { rows[k].classList.toggle('on', k === id); });
    }
    function still(ti) {
      current = ti;
      setChip(TURNS[ti]);
      stack.innerHTML = ''; stack.style.transform = '';
      for (var i = 0; i <= ti; i++) stack.appendChild(makeTurn(TURNS[i], true));
      fresh.classList.remove('on');
      ph.classList.remove('off'); typed.textContent = ''; caret.classList.remove('on');
      send.classList.remove('armed');
      setActive(TURNS[ti].row);
      layout();
      routePath.setAttribute('d', route(ti));
      routePath.classList.add('on');
    }

    still(STILL);
    var ro = ('ResizeObserver' in window) ? new ResizeObserver(function () { layout(); }) : null;
    if (ro) ro.observe(root); else window.addEventListener('resize', layout);
    if (document.fonts && document.fonts.ready) document.fonts.ready.then(layout);

    if (reduced) return { start: function () {}, stop: function () {} };

    /* ── Timeline ─────────────────────────────────────────────── */
    var alive = 0, timers = [], raf = 0;
    function wait(ms) {
      return new Promise(function (res) {
        var id = setTimeout(function () { timers.splice(timers.indexOf(id), 1); res(); }, ms);
        timers.push(id);
      });
    }
    function tween(ms, fn) {
      return new Promise(function (res) {
        var t0 = performance.now();
        function step(now) {
          var p = Math.min(1, (now - t0) / ms);
          fn(p);
          if (p < 1) raf = requestAnimationFrame(step); else res();
        }
        raf = requestAnimationFrame(step);
      });
    }
    function halt() {
      alive++;
      timers.forEach(clearTimeout); timers = [];
      cancelAnimationFrame(raf);
    }

    function runPulse(ti) {
      var d = route(ti);
      pulsePath.setAttribute('d', d);
      var total = pulsePath.getTotalLength();
      var seg = Math.min(40, total * 0.2);
      var dur = clamp(total * 2, 1000, 1200);
      return tween(dur, function (p) {
        // mostly even speed, softened at both ends
        var q = 0.45 * p + 0.55 * easeInOut(p);
        var s = q * (total + seg);
        var a = clamp(s - seg, 0, total), b = clamp(s, 0, total);
        var len = b - a;
        if (len < 0.5) { pulsePath.style.opacity = '0'; return; }
        pulsePath.style.opacity = '1';
        pulsePath.style.strokeDasharray = len + ' ' + (total * 2 + 10);
        pulsePath.style.strokeDashoffset = String(-a);
      }).then(function () { pulsePath.style.opacity = '0'; });
    }

    // The sent message joins the chat; everything above scrolls up, and a
    // message that will pass the top edge fades out whole as the scroll starts.
    function addTurn(t) {
      var h0 = stack.offsetHeight;
      var nt = makeTurn(t, false);
      nt.classList.add('enter');
      stack.appendChild(nt);
      var d = stack.offsetHeight - h0;
      stack.style.transform = 'translateY(' + d + 'px)';
      clipLog(d);
      return { turn: nt, done: tween(340, function (p) {
        stack.style.transform = p < 1 ? 'translateY(' + f(d * (1 - easeOut(p))) + 'px)' : '';
      }) };
    }

    function swapChip(t) {
      if (chipName.textContent === t.model && chipLock.hidden === !t.lock) return Promise.resolve();
      chipModel.classList.add('out');
      return wait(150).then(function () {
        setChip(t);
        chipModel.classList.remove('out'); chipModel.classList.add('in');
        void chipModel.offsetWidth;
        chipModel.classList.remove('in');
      });
    }

    async function reset(my) {
      // A new chat: the whole transcript goes at once, the lit row goes quiet,
      // the chip shows the new chat's model, and "New chat" shows before
      // anything is sent. A composer left mid-turn by stop() is put to rest.
      chipModel.classList.remove('out', 'in');
      ph.classList.remove('hide');
      pick.classList.remove('open');
      PICK.forEach(function (p) { prows[p.row].classList.remove('on'); });
      chip.classList.remove('tint');
      send.classList.remove('armed', 'press');
      caret.classList.remove('on');
      if (typed.textContent) typed.classList.add('off');
      pulsePath.style.opacity = '0';
      routePath.classList.remove('on');
      setActive(null);
      fresh.classList.remove('on');
      log.classList.add('leave');
      current = null;
      var swapping = swapChip(NEW_CHAT);
      await wait(320); if (my !== alive) return;
      await swapping; if (my !== alive) return;
      stack.innerHTML = ''; stack.style.transform = '';
      typed.textContent = ''; typed.classList.remove('off');
      ph.classList.remove('off');
      log.classList.remove('leave');
      fresh.classList.add('on');
      await wait(400);
    }

    async function turn(ti, my) {
      var t = TURNS[ti];
      var priv = chatIsPrivate(ti);
      var from = ti ? TURNS[ti - 1].row : NEW_CHAT.row;
      var a = ORDER.indexOf(from), b = ORDER.indexOf(t.row), dir = b > a ? 1 : -1;

      // 1. The person opens the model chip. The menu opens on the chat's
      //    current model, and the highlight moves row by row to the one they
      //    pick. In a chat that is already private, the public model is
      //    shown as unavailable. The last turn's route and row go quiet.
      PICK.forEach(function (p) {
        prows[p.row].classList.toggle('off', priv && !p.lock);
        prows[p.row].classList.toggle('on', p.row === from);
      });
      routePath.classList.remove('on');
      setActive(null);
      chip.classList.add('tint');
      ph.classList.add('hide');
      await wait(100); if (my !== alive) return;
      pick.classList.add('open');
      // about 1.1 to 1.3 s open: a beat on the current model, a step per
      // row, then a hold on the choice before the menu closes
      await wait(175 + (Math.abs(b - a) > 1 ? 225 : 320)); if (my !== alive) return;
      for (var k = a; k !== b; k += dir) {
        prows[ORDER[k]].classList.remove('on');
        prows[ORDER[k + dir]].classList.add('on');
        await wait(k + dir === b ? 600 : 260); if (my !== alive) return;
      }
      current = ti;
      pick.classList.remove('open');
      chip.classList.remove('tint');
      swapChip(t);   // the chip takes the chosen model while the menu fades
      await wait(185); if (my !== alive) return;

      // 2. The prompt types; Send arms with the first character.
      ph.classList.add('off'); ph.classList.remove('hide');
      typed.classList.remove('off');
      caret.classList.add('on');
      for (var i = 1; i <= t.prompt.length; i++) {
        typed.textContent = t.prompt.slice(0, i);
        if (i === 1) send.classList.add('armed');
        await wait(15); if (my !== alive) return;
      }
      await wait(130); if (my !== alive) return;

      // 3. Send. The composer clears and the message joins the chat.
      send.classList.add('press');
      fresh.classList.remove('on');
      await wait(125); if (my !== alive) return;
      send.classList.remove('press', 'armed');
      caret.classList.remove('on');
      typed.classList.add('off');
      await wait(110); if (my !== alive) return;
      typed.textContent = ''; typed.classList.remove('off');
      var added = addTurn(t);
      added.turn.classList.remove('enter');
      // The placeholder returns at once, unless the transcript sits so close
      // above the card that the arriving message would still cross it.
      var early = logGap >= 24;
      if (early) ph.classList.remove('off');
      await added.done; if (my !== alive) return;
      if (!early) ph.classList.remove('off');

      // 4. The pulse carries the turn through Biorouter to the chosen model.
      //    Its path stays lit, with the row, until the next choice.
      await runPulse(ti); if (my !== alive) return;
      routePath.setAttribute('d', route(ti));
      routePath.classList.add('on');
      setActive(t.row);
      await wait(120); if (my !== alive) return;

      // 5. The reply streams back.
      var ws = added.turn.querySelectorAll('.w');
      for (var j = 0; j < ws.length; j++) {
        ws[j].classList.add('on');
        await wait(32); if (my !== alive) return;
      }
      await wait(ti === TURNS.length - 1 ? 300 : 220);
    }

    async function loop(my) {
      while (my === alive) {
        await reset(my); if (my !== alive) return;
        for (var i = 0; i < TURNS.length; i++) {
          await turn(i, my); if (my !== alive) return;
        }
      }
    }

    hub.classList.add('live');
    return {
      start: function () { halt(); loop(alive); },
      stop: function () { halt(); pulsePath.style.opacity = '0'; }
    };
  });
})();
