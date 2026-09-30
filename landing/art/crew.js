/* crew: one Crew channel, seen by the person who asks their agent.
   Gina Rossi shares counts.csv in #methods. The viewer chooses Ask my agent
   on an empty message box, and the Ask my agent pane opens in the composer's
   place: the task is typed in its Task field, the pane says what the agent
   may do ("Your agent can read #methods and post there, for this task
   only."), and nothing starts until the viewer chooses "Start my agent and
   allow posting here". Then the pane closes, Crew posts the task in the
   channel as "Task: …" from "Your agent", and the viewer's task row sits
   under that post reading "Your agent · Working…". The result joins the same
   post group, the row moves under it, and the row reads "Done".
   Vocabulary from ui/desktop/src/components/crew (ChannelHeader, MessageRow,
   TaskStatusRow, AttachmentCard, Composer, pane/AgentTaskPane and
   pane/copy.ts) and docs/crew/agents-and-chat-access.md. */
(function () {
  'use strict';
  if (!window.BR || !BR.art) return;

  var SVG = 'xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false"';
  var BOT = '<svg ' + SVG + '><path d="M12 8V4H8"/><rect width="16" height="12" x="4" y="8" rx="2"/><path d="M2 14h2"/><path d="M20 14h2"/><path d="M15 13v2"/><path d="M9 13v2"/></svg>';
  var FILE = '<svg ' + SVG + '><path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/></svg>';
  var UP = '<svg ' + SVG + '><path d="m5 12 7-7 7 7"/><path d="M12 19V5"/></svg>';
  var CHEV = '<svg ' + SVG + '><path d="m6 9 6 6 6-6"/></svg>';

  /* The task as the viewer types it in the pane. It names the shared file,
     and it fits one line of the Task field at every tile size. */
  var TASK = 'QC on counts.csv';

  var HTML =
    '<div class="crew-stage" aria-hidden="true">' +
      '<div class="crew-head">' +
        '<span class="crew-chan"><span class="crew-hash">#</span>methods' + CHEV + '</span>' +
        '<span class="crew-class">Restricted</span>' +
      '</div>' +
      '<div class="crew-log">' +
        '<div class="crew-msg crew-gina">' +
          '<span class="crew-av crew-av-person">GR</span>' +
          '<div class="crew-main">' +
            '<div class="crew-meta"><span class="crew-name">Gina Rossi</span><span class="crew-time">10:02 AM</span></div>' +
            '<div class="crew-file">' + FILE + '<span class="crew-file-name">counts.csv</span><span class="crew-file-size">55 KB</span></div>' +
          '</div>' +
        '</div>' +
        /* The agent's post group: "Task: …" first, the result joins it later. */
        '<div class="crew-slot crew-slot-post"><div class="crew-slot-in"><div class="crew-slot-pad">' +
          '<div class="crew-msg crew-post">' +
            '<span class="crew-av crew-av-agent">' + BOT + '</span>' +
            '<div class="crew-main">' +
              '<div class="crew-meta"><span class="crew-name">Your agent</span><span class="crew-badge">Agent</span><span class="crew-time">10:03 AM</span></div>' +
              '<p class="crew-body">Task: ' + TASK + '</p>' +
              '<div class="crew-slot crew-slot-result"><div class="crew-slot-in"><div class="crew-result-pad">' +
                '<p class="crew-body">All 24 samples pass QC.</p>' +
              '</div></div></div>' +
            '</div>' +
          '</div>' +
        '</div></div></div>' +
        /* The viewer's task row, under the group, never inside it. */
        '<div class="crew-slot crew-slot-task"><div class="crew-slot-in"><div class="crew-slot-pad">' +
          '<div class="crew-task">' +
            '<span class="crew-av crew-av-agent">' + BOT + '</span>' +
            '<p class="crew-task-status">Your agent · <span class="crew-word"><span class="crew-w-run">Working…</span><span class="crew-w-done">Done</span></span></p>' +
          '</div>' +
        '</div></div></div>' +
      '</div>' +
      '<div class="crew-compose"><div class="crew-compose-in"><div class="crew-box">' +
        '<div class="crew-card">' +
          '<span class="crew-field"><span class="crew-ph">Message #methods</span></span>' +
          '<span class="crew-ask">' + BOT + '<span class="crew-ask-label">Ask my agent</span></span>' +
          '<span class="crew-send">' + UP + '</span>' +
        '</div>' +
        /* Ask my agent, in the composer's place: the composer's box grows
           into the pane, which holds Task, the footer's scope line, and
           Start. */
        '<div class="crew-sheet">' +
          '<div class="crew-sheet-task">' +
            '<span class="crew-sheet-label">Task</span>' +
            '<span class="crew-input"><span class="crew-input-ph">What should your agent do?</span><span class="crew-draft"><span class="crew-draft-text"></span><i class="crew-caret"></i></span></span>' +
          '</div>' +
          '<p class="crew-scope">Your agent can read #methods and post there, for this task only.</p>' +
          '<div class="crew-sheet-foot"><span class="crew-start">Start my agent and allow posting here</span></div>' +
        '</div>' +
      '</div></div></div>' +
    '</div>';

  BR.art('crew', function (root, opts) {
    var reduced = !!(opts && opts.reduced);
    root.classList.add('art-crew');
    root.innerHTML = HTML;

    var q = function (s) { return root.querySelector(s); };
    var stage = q('.crew-stage');
    var gina = q('.crew-gina');
    var post = q('.crew-slot-post');
    var result = q('.crew-slot-result');
    var task = q('.crew-slot-task');
    var field = q('.crew-field');
    var ph = q('.crew-ph');
    var composeIn = q('.crew-compose-in');
    var sheet = q('.crew-sheet');
    var input = q('.crew-input');
    var draftBox = q('.crew-draft');
    var draft = q('.crew-draft-text');
    var foot = q('.crew-sheet-foot');
    var start = q('.crew-start');
    var slots = [post, result, task];
    var timers = [];

    function set(el, cls, on) { el.classList.toggle(cls, on); }
    function at(ms, fn) { timers.push(setTimeout(fn, ms)); }
    function clear() { timers.forEach(clearTimeout); timers = []; }
    function show(el, open, shown) { set(el, 'is-open', open); set(el, 'is-shown', shown); }

    /* A one line field: while the draft is wider than the field, keep its end
       and the caret in view, as an input does. */
    function setDraft(text) {
      draft.textContent = text;
      set(input, 'crew-over', text !== '' && draftBox.scrollWidth > input.clientWidth);
    }

    /* Put everything back to the first frame without animating the way back. */
    function base() {
      stage.classList.add('crew-still');
      slots.forEach(function (s) { show(s, false, false); });
      stage.classList.remove('is-asking', 'is-pane', 'has-draft', 'is-starting', 'is-running', 'is-done', 'is-hl');
      setDraft('');
      void stage.offsetWidth;
      stage.classList.remove('crew-still');
    }

    /* The picture a reduced-motion visitor gets: the running moment. Gina's
       file, Crew's "Task: …" post, and the row "Your agent · Working…" with
       its rail. */
    function running() {
      stage.classList.add('crew-still');
      show(post, true, true);
      show(task, true, true);
      stage.classList.add('is-running');
    }

    /* The composer drops the button's words when it is too narrow for them,
       and Start tightens before it would overflow the pane. The pane covers
       the empty log under Gina's post; where it would reach her post, the
       scope line goes first (Start already says what is allowed). */
    function fit() {
      stage.classList.remove('crew-narrow', 'crew-tight', 'crew-noscope');
      if (ph.scrollWidth > field.clientWidth + 1) stage.classList.add('crew-narrow');
      if (start.offsetWidth > foot.clientWidth + 1) stage.classList.add('crew-tight');
      var room = composeIn.getBoundingClientRect().bottom - gina.getBoundingClientRect().bottom - 8;
      if (sheet.offsetHeight + 2 > room) stage.classList.add('crew-noscope');
      // The height the box grows to: the pane's own, plus the box's border.
      stage.style.setProperty('--crew-sheet-h', (sheet.offsetHeight + 2) + 'px');
      setDraft(draft.textContent);
    }
    fit();
    if ('ResizeObserver' in window) new ResizeObserver(fit).observe(root);
    if (document.fonts && document.fonts.ready) document.fonts.ready.then(fit);

    function loop() {
      base();
      // 1. Ask my agent, pressed on an empty message box (a ghost button: it
      //    greys, it never turns coral). Send stays idle.
      at(500, function () { stage.classList.add('is-asking'); });
      // 2. The composer's box grows into the pane.
      at(800, function () { stage.classList.remove('is-asking'); stage.classList.add('is-pane'); });
      // 3. The task is typed in Task. The caret arrives with the first
      //    character, so it never sits on the placeholder.
      for (var i = 1; i <= TASK.length; i++) {
        (function (n) {
          at(1100 + (n - 1) * 45, function () {
            if (n === 1) stage.classList.add('has-draft');
            setDraft(TASK.slice(0, n));
          });
        })(i);
      }
      // 4. Start my agent and allow posting here: the one control that starts
      //    anything, and the frame's one coral element. The pane holds long
      //    enough to read it.
      at(2900, function () { stage.classList.add('is-starting'); });
      // 5. The pane shrinks back into the empty composer; Crew posts
      //    "Task: …", and the viewer's row arrives under it, highlighted,
      //    working.
      at(3150, function () { stage.classList.remove('is-pane', 'is-starting'); });
      at(3300, function () {
        set(post, 'is-open', true);
        set(task, 'is-open', true);
        stage.classList.add('is-running', 'is-hl');
      });
      at(3450, function () { set(post, 'is-shown', true); set(task, 'is-shown', true); });
      at(4100, function () { stage.classList.remove('is-hl'); });
      // 6. The result joins the post group, and the row moves down under it.
      at(4800, function () { set(result, 'is-open', true); });
      at(5000, function () { set(result, 'is-shown', true); });
      // 7. The row reads Done. Its rail stays, the frame's one accent.
      at(6200, function () { stage.classList.remove('is-running'); stage.classList.add('is-done'); });
      // 8. Rest, then clear the agent's rows (the rail goes with them) and
      //    start again.
      at(8800, function () { slots.forEach(function (s) { set(s, 'is-shown', false); }); });
      at(9200, function () { slots.forEach(function (s) { set(s, 'is-open', false); }); });
      at(10200, loop);
    }

    if (reduced) {
      running();
      fit();
      return { start: function () {}, stop: function () {} };
    }

    return {
      start: function () { clear(); stage.classList.remove('crew-paused'); loop(); },
      // Halt: no timers left, and the breathing word stops too.
      stop: function () { clear(); stage.classList.add('crew-paused'); }
    };
  });
})();
