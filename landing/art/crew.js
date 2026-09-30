/* crew: one Crew channel, seen by the person who asks their agent.
   Gina Rossi shares counts.csv in #methods. The viewer chooses Ask my agent
   in the composer, and the Ask my agent pane covers the channel under its
   header, as the app's pane does in a narrow window: a title row with its ×,
   the Task field, the Model row with its "Private · UCSF" tier mark, and a
   footer holding the scope line ("Your agent can read #methods and post
   there, for this task only.") and "Start my agent and allow posting here".
   Nothing starts until Start. Then the pane closes, Crew posts the task in
   the channel as "Task: …" from "Your agent", and the viewer's task row
   arrives under that post reading "Your agent · Starting…", then
   "Working…". The result joins the same post group and ends with Crew's own
   Source line, and the row reads "Done".
   Vocabulary from ui/desktop/src/components/crew (ChannelHeader, MessageRow,
   TaskStatusRow, AttachmentCard, Composer, pane/DetailsPane,
   pane/AgentTaskPane, pane/CrewModelPicker, pane/copy.ts,
   state/crewStatus.ts), crates/biorouter/src/crew/source_line.rs and
   docs/crew/agents-and-chat-access.md. */
(function () {
  'use strict';
  if (!window.BR || !BR.art) return;

  var SVG = 'xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false"';
  var BOT = '<svg ' + SVG + '><path d="M12 8V4H8"/><rect width="16" height="12" x="4" y="8" rx="2"/><path d="M2 14h2"/><path d="M20 14h2"/><path d="M15 13v2"/><path d="M9 13v2"/></svg>';
  var FILE = '<svg ' + SVG + '><path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/></svg>';
  var UP = '<svg ' + SVG + '><path d="m5 12 7-7 7 7"/><path d="M12 19V5"/></svg>';
  var CHEV = '<svg ' + SVG + '><path d="m6 9 6 6 6-6"/></svg>';
  var X = '<svg ' + SVG + '><path d="M18 6 6 18"/><path d="m6 6 12 12"/></svg>';

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
              /* The result, then the line Crew itself adds to every result:
                 the shared file the run read, and who shared it. */
              '<div class="crew-slot crew-slot-result"><div class="crew-slot-in"><div class="crew-result-pad">' +
                '<p class="crew-body">All 24 samples pass QC.</p>' +
                '<p class="crew-source">Source: counts.csv, shared by Gina Rossi.</p>' +
              '</div></div></div>' +
            '</div>' +
          '</div>' +
        '</div></div></div>' +
        /* The viewer's task row, under the group, never inside it. */
        '<div class="crew-slot crew-slot-task"><div class="crew-slot-in"><div class="crew-slot-pad">' +
          '<div class="crew-task">' +
            '<span class="crew-av crew-av-agent">' + BOT + '</span>' +
            '<p class="crew-task-status">Your agent · <span class="crew-word">' +
              '<span class="crew-w crew-w-start"><span class="crew-br">Starting…</span></span>' +
              '<span class="crew-w crew-w-run"><span class="crew-br">Working…</span></span>' +
              '<span class="crew-w crew-w-done">Done</span>' +
            '</span></p>' +
          '</div>' +
        '</div></div></div>' +
      '</div>' +
      '<div class="crew-compose"><div class="crew-card">' +
        '<span class="crew-field"><span class="crew-ph">Message #methods</span></span>' +
        '<span class="crew-ask">' + BOT + '<span class="crew-ask-label">Ask my agent</span></span>' +
        '<span class="crew-send">' + UP + '</span>' +
      '</div></div>' +
      /* Ask my agent, covering the channel below its header. Its ground and
         hairlines cover first; its contents follow. */
      '<div class="crew-pane">' +
        '<div class="crew-pane-top"><div class="crew-pane-row crew-fade">' +
          '<span class="crew-pane-title">Ask my agent</span>' +
          '<span class="crew-pane-x">' + X + '</span>' +
        '</div></div>' +
        '<div class="crew-pane-body crew-fade">' +
          '<span class="crew-pane-label">Task</span>' +
          '<span class="crew-input"><span class="crew-input-ph">What should your agent do?</span><span class="crew-draft"><span class="crew-draft-text"></span><i class="crew-caret"></i></span></span>' +
          '<span class="crew-pane-label">Model</span>' +
          '<span class="crew-tier"><span class="privacy is-private">Private · UCSF</span></span>' +
        '</div>' +
        '<div class="crew-pane-foot"><div class="crew-pane-foot-in crew-fade">' +
          '<p class="crew-scope">Your agent can read #methods and post there, for this task only.</p>' +
          '<div class="crew-start-row"><span class="crew-start">Start my agent and allow posting here</span></div>' +
        '</div></div>' +
      '</div>' +
    '</div>';

  BR.art('crew', function (root, opts) {
    var reduced = !!(opts && opts.reduced);
    root.classList.add('art-crew');
    root.innerHTML = HTML;

    var q = function (s) { return root.querySelector(s); };
    var stage = q('.crew-stage');
    var post = q('.crew-slot-post');
    var result = q('.crew-slot-result');
    var task = q('.crew-slot-task');
    var field = q('.crew-field');
    var ph = q('.crew-ph');
    var pane = q('.crew-pane');
    var paneTop = q('.crew-pane-top');
    var paneBody = q('.crew-pane-body');
    var paneFoot = q('.crew-pane-foot');
    var startRow = q('.crew-start-row');
    var start = q('.crew-start');
    var input = q('.crew-input');
    var draftBox = q('.crew-draft');
    var draft = q('.crew-draft-text');
    var slots = [post, result, task];
    var STATES = ['is-asking', 'is-pane', 'is-pane-in', 'has-draft', 'is-starting', 'is-task', 'is-working', 'is-done', 'is-hl'];
    var timers = [];

    function set(el, cls, on) { el.classList.toggle(cls, on); }
    function on(cls) { stage.classList.add(cls); }
    function off(cls) { stage.classList.remove(cls); }
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
      STATES.forEach(off);
      setDraft('');
      void stage.offsetWidth;
      stage.classList.remove('crew-still');
    }

    /* The picture a reduced-motion visitor gets: the finished task. Gina's
       file, Crew's "Task: …" post, the result with Crew's Source line, and
       the row reading Done. Nothing in it moves. */
    function finished() {
      stage.classList.add('crew-still');
      slots.forEach(function (s) { show(s, true, true); });
      on('is-done');
    }

    /* The composer drops the button's words when it is too narrow for them,
       and Start tightens before it would overflow the pane. Where the pane is
       too short for all it holds, the scope line goes first: Start already
       says what is allowed, and the Model row stays. */
    function fit() {
      stage.classList.remove('crew-narrow', 'crew-tight', 'crew-noscope');
      if (ph.scrollWidth > field.clientWidth + 1) on('crew-narrow');
      if (start.offsetWidth > startRow.clientWidth + 1) on('crew-tight');
      var need = paneTop.offsetHeight + paneBody.scrollHeight + paneFoot.offsetHeight;
      if (need > pane.clientHeight + 1) on('crew-noscope');
      setDraft(draft.textContent);
    }
    fit();
    if ('ResizeObserver' in window) new ResizeObserver(fit).observe(root);
    if (document.fonts && document.fonts.ready) document.fonts.ready.then(fit);

    function loop() {
      base();
      // 1. Ask my agent, pressed on an empty message box (a ghost button: it
      //    greys, it never turns coral). Send stays idle.
      at(300, function () { on('is-asking'); });
      // 2. The pane covers the channel under its header: its ground first,
      //    then its contents, 8px in from the right.
      at(750, function () { off('is-asking'); on('is-pane'); });
      at(875, function () { on('is-pane-in'); });
      // 3. The task is typed in Task. The caret arrives with the first
      //    character, so it never sits on the placeholder.
      for (var i = 1; i <= TASK.length; i++) {
        (function (n) {
          at(1300 + (n - 1) * 45, function () {
            if (n === 1) on('has-draft');
            setDraft(TASK.slice(0, n));
          });
        })(i);
      }
      // 4. Start my agent and allow posting here: the one control that starts
      //    anything, and the frame's one coral element. The pane holds long
      //    enough to read the Model row and the scope line.
      at(3000, function () { on('is-starting'); });
      // 5. The pane goes: its contents, then its ground.
      at(3250, function () { off('is-pane-in'); off('is-starting'); });
      at(3400, function () { off('is-pane'); off('has-draft'); setDraft(''); });
      // 6. Crew posts "Task: …" in the channel.
      at(3750, function () { set(post, 'is-open', true); });
      at(4050, function () { set(post, 'is-shown', true); });
      // 7. The viewer's row arrives under it, washed once, reading
      //    Starting…, with the coral rail.
      at(4400, function () { set(task, 'is-open', true); on('is-task'); on('is-hl'); });
      at(4650, function () { set(task, 'is-shown', true); });
      at(4900, function () { off('is-hl'); });
      // 8. Working…
      at(5250, function () { on('is-working'); });
      // 9. The result joins the post group, with Crew's Source line, and the
      //    row moves down under it.
      at(6150, function () { set(result, 'is-open', true); });
      at(6450, function () { set(result, 'is-shown', true); });
      // 10. Done: the word changes and the rail goes with the running state.
      at(7450, function () { off('is-task'); off('is-working'); on('is-done'); });
      // 11. Rest on the finished channel, then clear the agent's rows and
      //     start again.
      at(9650, function () { slots.forEach(function (s) { set(s, 'is-shown', false); }); });
      at(10050, function () { slots.forEach(function (s) { set(s, 'is-open', false); }); });
      at(10700, loop);
    }

    if (reduced) {
      finished();
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
