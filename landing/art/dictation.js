/* Voice dictation tile: speaking into the composer.
   The composer is the app's one line card (owner, 2026-11-13; ChatInput's
   `.br-composer-row`): the `+`, the text, the microphone and Send, with the
   microphone immediately left of Send (DictationButton.tsx, DD-60). Above it
   sits a slice of the Dictionary page in its own rows (DictionaryRow.tsx: the
   term, "Said as <spoken form>", the "<n> uses" meta).
   The person clicks into the field and presses the microphone. It becomes the
   recording pill: five accent level bars (driven here by a scripted level, as
   the app drives them from each 100 ms chunk) and the m:ss timer. The empty
   field reads "Listening…" (DICTATION_LABELS.listening). What has been heard
   so far shows as the muted italic line in the card's notice row
   (DictationNotices, `.br-dictation-partial`), never in the field. On stop the
   pill shows the spinner (Transcribing), and the final text goes into the
   field at the caret as one edit, with TREM2 spelled as the dictionary spells
   it. Then the Dictionary slice gives way to the app's own disclosure,
   MSG-DISCLOSE-LOCAL (docs/dictation/surfaces.md, message catalog).
   One liberty: the inserted TREM2 and its dictionary row take a brief accent
   wash together, so the link between them reads; the app marks no inserted
   word. Times are the tile's own, and the stop to text wait is drawn longer
   than the 150 to 330 ms the docs measured. One thing moves at a time.
   Reduced motion shows the inserted text under the dictionary. */
(function () {
  'use strict';
  if (!window.BR || !BR.art) return;

  // Lucide paths, as the app draws them (stroke 1.5 on a 24 grid).
  var ICON = {
    // The Dictionary glyph (components/icons/app-icons.tsx: Dictionary = BookA).
    book: '<path d="M4 19.5v-15A2.5 2.5 0 0 1 6.5 2H19a1 1 0 0 1 1 1v18a1 1 0 0 1-1 1H6.5a1 1 0 0 1 0-5H20"/><path d="m8 13 4-7 4 7"/><path d="M9.1 11h5.7"/>',
    mic: '<path d="M12 19v3"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><rect x="9" y="2" width="6" height="13" rx="3"/>',
    // The one busy glyph (ui/spinner.tsx: Loader2).
    spin: '<path d="M21 12a9 9 0 1 1-6.219-8.56"/>',
    plus: '<path d="M5 12h14"/><path d="M12 5v14"/>',
    up: '<path d="m5 12 7-7 7 7"/><path d="M12 19V5"/>'
  };
  function svg(name, cls, sw) {
    return '<svg class="' + cls + '" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="' + (sw || 1.5) +
      '" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">' + ICON[name] + '</svg>';
  }

  // Dictionary rows: the written form, its second line, the uses meta. The
  // third row shows only where the tile is tall enough.
  var TERMS = [
    { term: 'TREM2', line: 'Said as trem two', uses: '12 uses' },
    { term: 'EPAS1', line: 'Said as epas one', uses: '4 uses' },
    { term: 'UCSF', line: 'Added by you', uses: '31 uses' }
  ];
  // What the person says, word by word, as the preview line grows.
  var WORDS = ['Survey', 'TREM2', 'variants'];
  var PH_EMPTY = 'Ask Biorouter anything…';
  var PH_LISTEN = 'Listening…';
  var NOTE = 'Speech is transcribed by Biorouter on this computer. Audio is not saved.';

  // The scripted level: one swell per syllable, in ms after recording starts,
  // over a quiet floor. Read every 100 ms, as the app reads each chunk's RMS.
  var SYL = [[260, 0.85], [470, 0.6], [860, 1], [1090, 0.75], [1480, 0.8], [1690, 0.6], [1910, 0.7]];
  function levelAt(ms) {
    var v = 0.05 + 0.04 * (1 + Math.sin(ms / 41)) / 2;
    for (var i = 0; i < SYL.length; i++) {
      var d = (ms - SYL[i][0]) / 120;
      v = Math.max(v, SYL[i][1] * Math.exp(-d * d) * (0.88 + 0.12 * Math.sin(ms / 23 + i)));
    }
    return Math.min(1, v);
  }

  function esc(s) { return s.replace(/&/g, '&amp;').replace(/</g, '&lt;'); }

  BR.art('dictation', function (root, opts) {
    var reduced = !!(opts && opts.reduced);
    var stage = document.createElement('div');
    stage.className = 'art-dictation';
    stage.setAttribute('aria-hidden', 'true');

    var termsHtml = TERMS.map(function (t, i) {
      return '<div class="dx-term" data-i="' + i + '"><span class="dx-term-text">' +
        '<span class="dx-term-name"><span class="dx-mark">' + t.term + '</span></span>' +
        '<span class="dx-term-line">' + t.line + '</span></span>' +
        '<span class="dx-term-uses">' + t.uses + '</span></div>';
    }).join('');

    var barsHtml = '';
    for (var b = 0; b < 5; b++) barsHtml += '<i class="dx-bar" data-bar="' + b + '"></i>';

    var inserted = WORDS.map(function (w, i) {
      return i === 1 ? '<span class="dx-hit">' + esc(w) + '</span>' : esc(w);
    }).join(' ');

    stage.innerHTML =
      '<div class="dx-top">' +
        '<div class="dx-dict">' +
          '<div class="dx-dict-head">' + svg('book', 'dx-head-ic') + '<span>Dictionary</span></div>' +
          '<div class="dx-terms">' + termsHtml + '</div>' +
        '</div>' +
        '<div class="dx-note">' + NOTE + '</div>' +
      '</div>' +
      '<div class="dx-composer">' +
        // The notice row: the muted preview of what has been heard.
        '<div class="dx-heard"><div class="dx-heard-in"><p class="dx-partial"></p></div></div>' +
        // ONE ROW: [+] [text] [mic] [Send].
        '<div class="dx-row">' +
          '<span class="dx-plus">' + svg('plus', 'dx-row-ic') + '</span>' +
          '<span class="dx-field"><span class="dx-text"></span><span class="dx-caret"></span><span class="dx-ph">' + PH_EMPTY + '</span></span>' +
          '<span class="dx-mic">' +
            '<span class="dx-mic-ic">' + svg('mic', 'dx-row-ic') + '</span>' +
            '<span class="dx-pill">' +
              '<span class="dx-meter"><span class="dx-bars">' + barsHtml + '</span>' + svg('spin', 'dx-spin') + '</span>' +
              '<span class="dx-timer">0:00</span>' +
            '</span>' +
          '</span>' +
          '<span class="dx-send">' + svg('up', 'dx-send-ic', 2) + '</span>' +
        '</div>' +
      '</div>';
    root.appendChild(stage);

    var q = function (s) { return stage.querySelector(s); };
    var text = q('.dx-text'), ph = q('.dx-ph'), partial = q('.dx-partial'), timer = q('.dx-timer'), mic = q('.dx-mic');

    function set(cls, on) { stage.classList.toggle(cls, !!on); }
    function setPartial(n) {
      // The newest word fades in; the words before it stay as they were.
      var head = WORDS.slice(0, n - 1).map(esc).join(' ');
      partial.innerHTML = (head ? head + ' ' : '') + '<span class="dx-new">' + esc(WORDS[n - 1]) + '</span>';
    }
    function setClock(s) { timer.textContent = '0:0' + s; }
    function setLevel(v) { mic.style.setProperty('--dx-level', v.toFixed(3)); }

    // The level is read every 100 ms while recording, on the loop's own clock.
    var levelTimer = null;
    function stopLevel() { if (levelTimer) { clearInterval(levelTimer); levelTimer = null; } setLevel(0); }
    function startLevel() {
      stopLevel();
      levelTimer = setInterval(function () {
        if (!running) { stopLevel(); return; }
        setLevel(levelAt(performance.now() - t0 - REC_AT));
      }, 100);
    }

    // A reset lands with no transitions, then motion is switched back on.
    // `is-live` (the bars and the spinner) survives it.
    function reset() {
      var live = stage.classList.contains('is-live');
      stage.className = 'art-dictation dx-instant' + (live ? ' is-live' : '');
      stopLevel();
      text.innerHTML = '';
      partial.innerHTML = '';
      ph.textContent = PH_EMPTY;
      setClock(0);
      void stage.offsetWidth;
      requestAnimationFrame(function () { requestAnimationFrame(function () { stage.classList.remove('dx-instant'); }); });
    }

    function insert() {
      text.innerHTML = inserted;
      set('has-text', true);
    }

    // The dictated draft: the still frame for reduced motion, and the frame
    // shown before the tile starts. The text is in the field, TREM2 spelled as
    // the dictionary row above it, Send armed, the microphone back at rest.
    function still() {
      reset();
      set('show-dict', true);
      set('is-focus', true);
      insert();
    }

    // The loop, as a list of [time ms, action].
    var REC_AT = 1150;
    var plan = [];
    plan.push([0, reset]);
    plan.push([150, function () { set('show-dict', true); }]);
    // The person clicks into the field: the caret, and the card's focus edge.
    plan.push([600, function () { set('is-focus', true); set('is-edge', true); }]);
    // The press; the pill takes the microphone's place.
    plan.push([1000, function () { set('is-press', true); }]);
    plan.push([REC_AT, function () {
      set('is-press', false); set('is-rec', true);
      ph.textContent = PH_LISTEN;
      startLevel();
    }]);
    plan.push([REC_AT + 1000, function () { setClock(1); }]);
    plan.push([REC_AT + 2000, function () { setClock(2); }]);
    // The preview line opens with the first word and grows as words are heard.
    plan.push([REC_AT + 800, function () { setPartial(1); set('has-heard', true); }]);
    plan.push([REC_AT + 1450, function () { setPartial(2); }]);
    plan.push([REC_AT + 2250, function () { setPartial(3); }]);
    // Stop: the spinner takes the bars' place and the preview clears.
    var STOP_AT = REC_AT + 2850;
    plan.push([STOP_AT, function () { set('is-press', true); }]);
    plan.push([STOP_AT + 100, function () {
      set('is-press', false); set('is-busy', true); set('has-heard', false);
      stopLevel();
    }]);
    // The transcript lands as one edit. The edge returns to its hairline so the
    // armed Send and the dictionary's spelling carry the accent.
    var INSERT_AT = STOP_AT + 600;
    plan.push([INSERT_AT, function () {
      set('is-rec', false); set('is-busy', false);
      ph.textContent = PH_EMPTY;
      set('is-edge', false);
      insert();
      set('is-hit', true);
    }]);
    plan.push([INSERT_AT + 1000, function () { set('is-hit', false); }]);
    // The Dictionary slice gives way to the disclosure.
    var NOTE_AT = INSERT_AT + 2600;
    plan.push([NOTE_AT, function () { set('show-note', true); }]);
    var CLEAR_AT = NOTE_AT + 2700;
    plan.push([CLEAR_AT, function () { set('is-clearing', true); }]);
    var LOOP = CLEAR_AT + 600;
    plan.sort(function (a, b) { return a[0] - b[0]; });
    // On scroll-in the still frame holds briefly, then the disclosure arrives
    // and the loop carries on from there.
    var HOLD = 900;
    var START_AT = NOTE_AT - HOLD;

    var tid = null, t0 = 0, idx = 0, running = false;
    function tick() {
      if (!running) return;
      var now = performance.now() - t0;
      while (idx < plan.length && plan[idx][0] <= now) { plan[idx][1](); idx++; }
      if (idx >= plan.length && now >= LOOP) { t0 += LOOP; idx = 0; tick(); return; }
      var next = idx < plan.length ? plan[idx][0] : LOOP;
      tid = setTimeout(tick, Math.max(0, next - now));
    }

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
        if (tid) { clearTimeout(tid); tid = null; }
        stopLevel();
        set('is-live', false);
        still(); // off screen by now; the next start picks up from here
      }
    };
  });
})();
