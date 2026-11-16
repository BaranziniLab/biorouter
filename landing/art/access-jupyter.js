/* JupyterLab tile (Ways to use Biorouter).
   JupyterLab's file browser at the left (the chat is a .chat file, as Jupyter
   AI 3 saves one) and the Jupyter AI chat it has open. The person types "@Bi",
   the mention menu offers "@Biorouter" (the persona's name and avatar,
   integrations/jupyter-ai/jupyter_ai_biorouter/persona.py), and choosing it
   puts the mention in the message. The rest of the question is typed and sent.
   The reply carries the persona's name, one tool row titled the way
   `biorouter acp` titles a call (format_tool_name in
   crates/biorouter-acp/src/server.rs: code_execution__execute_code reads
   "Code Execution: Execute Code"), then the answer. The placeholder is Jupyter
   AI's own, as the recording shows it. The answer is an illustration: no real
   patient or cohort figures. One thing moves at a time. Reduced motion shows
   the answered turn. */
(function () {
  'use strict';
  if (!window.BR || !BR.art) return;

  var FILES = [
    { name: 'cohort.csv', icon: 'sheet' },
    { name: 'notes.md', icon: 'text' },
    { name: 'qc.chat', icon: 'chat', current: true },
    { name: 'qc.ipynb', icon: 'notebook' }
  ];
  var PLACEHOLDER = 'Type a chat message, @ to mention...';
  var QUERY = '@Bi';
  var MENTION = '@Biorouter';
  var QUESTION = ' how many rows in cohort.csv have no age?';
  var PERSONA = 'Biorouter';
  var TOOL = 'Code Execution: Execute Code';
  // Illustrative model output, with the figure set bold as Jupyter AI renders it.
  var ANSWER = [['412', 1], ['of', 1], ['5,000', 1], ['rows', 1], ['have', 0], ['no', 0], ['age.', 0]];

  // Lucide paths at stroke 1.5, the site's icon weight.
  var ICON = {
    sheet: '<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M3 9h18"/><path d="M3 15h18"/><path d="M9 9v12"/><path d="M15 9v12"/>',
    text: '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M16 13H8"/><path d="M16 17H8"/><path d="M10 9H8"/>',
    chat: '<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>',
    notebook: '<path d="M2 6h4"/><path d="M2 10h4"/><path d="M2 14h4"/><path d="M2 18h4"/><rect width="16" height="20" x="4" y="2" rx="2"/><path d="M16 2v20"/>',
    up: '<path d="m5 12 7-7 7 7"/><path d="M12 19V5"/>',
    dashed: '<path d="M10.1 2.18a9.93 9.93 0 0 1 3.8 0"/><path d="M17.6 3.71a9.95 9.95 0 0 1 2.69 2.7"/><path d="M21.82 10.1a9.93 9.93 0 0 1 0 3.8"/><path d="M20.29 17.6a9.95 9.95 0 0 1-2.7 2.69"/><path d="M13.9 21.82a9.94 9.94 0 0 1-3.8 0"/><path d="M6.4 20.29a9.95 9.95 0 0 1-2.69-2.7"/><path d="M2.18 13.9a9.93 9.93 0 0 1 0-3.8"/><path d="M3.71 6.4a9.95 9.95 0 0 1 2.7-2.69"/><circle cx="12" cy="12" r="1"/>',
    check: '<path class="ajl-tick" d="M20 6 9 17l-5-5" pathLength="1"/>'
  };
  function svg(name, cls, sw) {
    return '<svg class="' + cls + '" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="' + (sw || 1.5) +
      '" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">' + ICON[name] + '</svg>';
  }
  // The persona's avatar is the app icon on its plate (biorouter.svg), which the
  // site serves as icon.svg. The box crops the icon's safe-area margin so the
  // plate fills it, as Jupyter AI draws the avatar.
  var AVATAR = '<span class="ajl-av"><img src="icon.svg" alt="" width="1024" height="1024"></span>';

  BR.art('access-jupyter', function (root, opts) {
    var reduced = !!(opts && opts.reduced);
    var wrap = document.createElement('div');
    wrap.className = 'ajl';
    wrap.setAttribute('aria-hidden', 'true');

    var files = FILES.map(function (f) {
      return '<div class="ajl-file' + (f.current ? ' is-current' : '') + '">' + svg(f.icon, 'ajl-file-ic') +
        '<span class="ajl-file-name">' + f.name + '</span></div>';
    }).join('');

    var words = ANSWER.map(function (w) {
      return '<span class="ajl-w' + (w[1] ? ' is-strong' : '') + '">' + w[0] + '</span>';
    }).join(' ');

    // The conversation is pinned above the message box, and room for the whole
    // turn is held from the start, so nothing moves as its parts arrive.
    wrap.innerHTML =
      '<div class="ajl-files">' + files + '</div>' +
      '<div class="ajl-chat">' +
        '<div class="ajl-log"><div class="ajl-stack">' +
          '<div class="ajl-user"><span class="ajl-bubble"><b>' + MENTION + '</b>' + QUESTION + '</span></div>' +
          '<div class="ajl-reply">' +
            '<div class="ajl-who">' + AVATAR + '<span class="ajl-name">' + PERSONA + '</span></div>' +
            '<div class="ajl-tool" data-state="hidden">' +
              '<span class="ajl-tool-ic">' + svg('dashed', 'ajl-i-run') + svg('check', 'ajl-i-done', 1.75) + '</span>' +
              '<span class="ajl-tool-t">' + TOOL + '</span>' +
            '</div>' +
            '<p class="ajl-ans">' + words + '</p>' +
          '</div>' +
        '</div></div>' +
        '<div class="ajl-field">' +
          // Named acc-pick: the floating menu keeps its hairline and shadow, and
          // the site's outline allowlist names it.
          '<div class="acc-pick"><div class="ajl-pick-row">' + AVATAR + '<span class="ajl-pick-name">' + MENTION + '</span></div></div>' +
          '<div class="ajl-input">' +
            // The placeholder and the typed text share one cell, so the box is
            // as tall as the longer of the two, and wraps as a textarea does.
            '<span class="ajl-ph">' + PLACEHOLDER + '</span>' +
            '<span class="ajl-typed"><span class="ajl-mention"></span><span class="ajl-rest"></span><span class="ajl-caret"></span></span>' +
          '</div>' +
          '<span class="ajl-send">' + svg('up', 'ajl-send-ic', 2) + '</span>' +
        '</div>' +
      '</div>';
    root.appendChild(wrap);

    var q = function (s) { return wrap.querySelector(s); };
    var chat = q('.ajl-chat'), log = q('.ajl-log'), stack = q('.ajl-stack');
    var mention = q('.ajl-mention'), rest_ = q('.ajl-rest'), tool = q('.ajl-tool');
    var ws = wrap.querySelectorAll('.ajl-w');
    var names = wrap.querySelectorAll('.ajl-file-name');

    function set(cls, on) { wrap.classList.toggle(cls, !!on); }
    function typed(m, r) {
      mention.textContent = m;
      rest_.textContent = r;
      set('has-text', (m + r).length > 0);
    }
    function settle(fn) {
      wrap.classList.add('ajl-still');
      fn();
      void wrap.offsetWidth;
      wrap.classList.remove('ajl-still');
    }

    // Width: the chat keeps at least CHAT_MIN px. Where the file browser would
    // squeeze it below that, the browser keeps only its icons; elsewhere a name
    // that does not fit steps down a size before it is ever cut. Height: the
    // turn is held whole. Where it is taller than the room above the message
    // box, its rhythm tightens, then the reply drops its name row (the message
    // already names @Biorouter), and only then does its top fade under the edge.
    var CHAT_MIN = 236;
    function fit() {
      wrap.classList.remove('fit-sm', 'fit-icons', 'fit-tight', 'fit-short', 'fit-over');
      if (chat.clientWidth < CHAT_MIN) wrap.classList.add('fit-icons');
      else {
        var over = function () {
          for (var i = 0; i < names.length; i++) if (names[i].scrollWidth > names[i].clientWidth + 0.5) return true;
          return false;
        };
        if (over()) wrap.classList.add('fit-sm');
      }
      var steps = ['fit-tight', 'fit-short', 'fit-over'];
      for (var k = 0; k < steps.length && stack.scrollHeight > log.clientHeight + 0.5; k++) wrap.classList.add(steps[k]);
    }
    if ('ResizeObserver' in window) new ResizeObserver(fit).observe(root);

    function answered() {
      wrap.classList.remove('is-focus', 'is-edge', 'show-pick', 'is-mention', 'is-press', 'is-clearing');
      typed('', '');
      set('is-sent', true);
      set('show-who', true);
      tool.setAttribute('data-state', 'done');
      for (var i = 0; i < ws.length; i++) ws[i].classList.add('on');
    }

    if (reduced) {
      settle(answered);
      fit();
      return { start: function () {}, stop: function () {} };
    }

    var timers = [];
    var active = false;
    function at(ms, fn) {
      var id = setTimeout(function () {
        var k = timers.indexOf(id);
        if (k >= 0) timers.splice(k, 1);
        if (active) fn();
      }, ms);
      timers.push(id);
    }
    function clear() { timers.forEach(clearTimeout); timers = []; }

    function rest() {
      settle(function () {
        wrap.classList.remove('is-focus', 'is-edge', 'show-pick', 'is-mention', 'is-press', 'is-sent', 'show-who', 'is-clearing');
        typed('', '');
        tool.setAttribute('data-state', 'hidden');
        for (var i = 0; i < ws.length; i++) ws[i].classList.remove('on');
      });
    }

    function typeAt(t0, gap, text, put) {
      for (var k = 1; k <= text.length; k++) (function (k) { at(t0 + (k - 1) * gap, function () { put(text.slice(0, k)); }); })(k);
    }

    function cycle() {
      // The box takes focus and "@Bi" narrows the mention menu to one persona.
      // The focus edge is coral while the mention is being typed.
      at(300, function () { set('is-focus', true); set('is-edge', true); });
      typeAt(500, 60, QUERY, function (s) { typed(s, ''); });
      at(760, function () { set('show-pick', true); });
      // Chosen: the menu goes, and the mention stands in the message. The edge
      // returns to rest, so the mention and the armed Send carry the accent.
      at(1900, function () { set('show-pick', false); set('is-edge', false); set('is-mention', true); typed(MENTION, ''); });
      typeAt(2300, 55, QUESTION, function (s) { typed(MENTION, s); });
      // Sent: the box empties and the message joins the chat.
      at(4800, function () { set('is-press', true); });
      at(4925, function () {
        wrap.classList.remove('is-press', 'is-focus', 'is-edge', 'is-mention');
        typed('', '');
        set('is-sent', true);
      });
      // The reply: the persona, its tool call running, then done.
      at(5500, function () { set('show-who', true); tool.setAttribute('data-state', 'active'); });
      at(6600, function () { tool.setAttribute('data-state', 'done'); });
      // The answer streams in, a word at a time.
      for (var i = 0; i < ws.length; i++) (function (i) { at(7000 + i * 90, function () { ws[i].classList.add('on'); }); })(i);
      // Hold, then the chat clears for the next loop.
      at(10000, function () { set('is-clearing', true); });
      at(10400, function () { rest(); cycle(); });
    }

    rest();
    fit();
    return {
      start: function () { if (active) return; active = true; set('is-live', true); clear(); rest(); cycle(); },
      stop: function () { active = false; set('is-live', false); clear(); rest(); }
    };
  });
})();
