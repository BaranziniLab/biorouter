/* Scheduling tile.
   One schedule in the Scheduler: its name, its status, cronstrue's sentence for
   `0 0 7 * * 1` ("At 07:00 AM, only on Monday", as SchedulesView renders it) and
   its "Last run" line. The loop opens on Monday 07:00: Monday's dot fills, the
   status reads Running, and a new chat ("Scheduled job: <id>", the name
   scheduler.rs gives every run) joins Recent chats. When the run finishes the
   status returns to Scheduled and Last run takes the new date (scheduler.rs sets
   last_run on a successful finish). Then the week passes a day at a time until
   Monday comes round again. One thing moves at a time. */
(function () {
  'use strict';

  var ID = 'weekly-digest';
  var CRON_WORDS = 'At 07:00 AM, only on Monday';
  var DAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
  var MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  var VISIBLE = 2;
  var MIN_EVEN = 14; // below this even share of the free height, drop the Last run line
  var MIN_PAD = 12;  // the least vertical padding, in px
  var CLEAR = 70;    // ms until the row leaving the top slot has cleared the new row's text

  // Lucide paths at the app's 1.5 stroke (components/icons/app-icons.tsx).
  var CLOCK = '<path d="M12 6v6l4 2"/><circle cx="12" cy="12" r="10"/>';
  var CAL_CLOCK = '<path d="M16 14v2.2l1.6 1"/><path d="M16 2v4"/><path d="M21 7.5V6a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h3.5"/><path d="M3 10h5"/><path d="M8 2v4"/><circle cx="16" cy="16" r="6"/>';
  function icon(paths) {
    return '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' + paths + '</svg>';
  }
  function el(tag, cls, text) {
    var n = document.createElement(tag);
    if (cls) n.className = cls;
    if (text != null) n.textContent = text;
    return n;
  }
  // Mondays, a week apart. Run n of the loop lands on FIRST + 7n days.
  var FIRST = new Date(2026, 8, 14, 7, 0);
  function monday(n) { var d = new Date(FIRST.getTime()); d.setDate(d.getDate() + 7 * n); return d; }
  function short(d) { return MONTHS[d.getMonth()] + ' ' + d.getDate(); }

  BR.art('schedule', function (root, opts) {
    var reduced = !!(opts && opts.reduced);
    var wrap = el('div', 'sch');
    wrap.setAttribute('aria-hidden', 'true');

    // The job row, as SchedulesView draws it.
    var job = el('div', 'sch-job');
    var ico = el('span', 'sch-ico'); ico.innerHTML = icon(CLOCK);
    var line1 = el('div', 'sch-line1');
    line1.appendChild(el('span', 'sch-name', ID));
    var status = el('span', 'sch-status');
    var s1 = el('span', 'sch-state is-scheduled'); s1.appendChild(el('i')); s1.appendChild(document.createTextNode('Scheduled'));
    var s2 = el('span', 'sch-state is-running'); s2.appendChild(el('i')); s2.appendChild(document.createTextNode('Running'));
    status.appendChild(s1); status.appendChild(s2);
    line1.appendChild(status);
    job.appendChild(ico); job.appendChild(line1);
    job.appendChild(el('span', 'sch-cron', CRON_WORDS));
    var last = el('span', 'sch-last', 'Last run ');
    var lastDate = el('span', 'sch-last-date');
    last.appendChild(lastDate);
    job.appendChild(last);

    // The week it waits through.
    var week = el('div', 'sch-week');
    var days = el('div', 'sch-days');
    DAYS.forEach(function (d, i) { var t = el('span', i === 0 ? 'is-on' : '', d); t.style.setProperty('--i', i); days.appendChild(t); });
    var track = el('div', 'sch-track');
    track.appendChild(el('i', 'sch-rail'));
    var progress = el('i', 'sch-progress');
    track.appendChild(progress);
    DAYS.forEach(function (d, i) {
      var dot = el('i', 'sch-dot' + (i === 0 ? ' is-on' : ''));
      dot.style.setProperty('--i', i);
      track.appendChild(dot);
    });
    week.appendChild(days); week.appendChild(track);

    // Recent chats: each run starts a new chat (ScheduleDetailView).
    var chats = el('div', 'sch-chats');
    chats.appendChild(el('div', 'sch-caps', 'Recent chats'));
    var list = el('div', 'sch-list');
    chats.appendChild(list);

    wrap.appendChild(job); wrap.appendChild(week); wrap.appendChild(chats);
    root.appendChild(wrap);

    function runRow(n) {
      var r = el('div', 'sch-run');
      r.innerHTML = icon(CAL_CLOCK);
      r.appendChild(el('span', 'sch-run-name', 'Scheduled job: ' + ID));
      r.appendChild(el('span', 'sch-run-date', short(monday(n))));
      return r;
    }
    function place(r, slot) {
      r.style.setProperty('--slot', slot);
      r.__slot = slot;
      r.classList.toggle('is-last', slot >= VISIBLE - 1);
    }

    // State. `latest` is the newest run already in the list, and its date is
    // the schedule's Last run.
    var latest = 1;
    function seed(n, fresh) {
      list.innerHTML = '';
      for (var k = 0; k < VISIBLE; k++) {
        var r = runRow(n - k);
        place(r, k);
        if (fresh && k === 0) r.classList.add('is-new');
        list.appendChild(r);
      }
      latest = n;
      lastDate.textContent = short(monday(n));
      last.classList.remove('is-swap');
    }
    function settle(fn) {
      wrap.classList.add('sch-still');
      fn();
      void wrap.offsetWidth;
      wrap.classList.remove('sch-still');
    }
    function setStep(k) {
      // k = 0..6 is Monday..Sunday.
      progress.style.transform = 'scaleX(' + (k / 6) + ')';
    }

    // Width: the run name is the one long string; step it down rather than cut it.
    // Height: the free height (everything the job, the week and Recent chats do
    // not fill) goes to the top and bottom padding and the two gaps between the
    // groups. The padding starts at the CSS value, P.
    // - A gap narrower than P: share the free height evenly between padding and
    //   gaps, so the middle is never tighter than the edges. If that even share is
    //   under MIN_EVEN, drop the Last run line first (the top chat carries the same
    //   date).
    // - A gap wider than GMAX (a tall tile): hold the gaps at GMAX and give the
    //   rest to the padding, so the three groups stay together, centred.
    function free() {
      var h = function (n) { return n.getBoundingClientRect().height; };
      return wrap.clientHeight - h(job) - h(week) - h(chats);
    }
    function fit() {
      wrap.classList.remove('fit-sm', 'fit-xs', 'fit-short');
      wrap.style.removeProperty('--sch-pad-y');
      var n = list.querySelector('.sch-run-name');
      if (n) {
        var over = function () { return n.scrollWidth > n.clientWidth + 0.5; };
        if (over()) { wrap.classList.add('fit-sm'); if (over()) wrap.classList.add('fit-xs'); }
      }
      var P = parseFloat(getComputedStyle(wrap).paddingTop);
      if (free() / 4 < MIN_EVEN) wrap.classList.add('fit-short');
      var f = free();
      var gap = (f - 2 * P) / 2;
      var gmax = Math.max(28, Math.round(wrap.clientHeight * 0.08));
      var pad = P;
      if (gap > gmax) pad = Math.floor((f - 2 * gmax) / 2);
      else if (gap < P) pad = Math.max(MIN_PAD, Math.min(P, Math.floor(f / 4)));
      if (pad !== P) wrap.style.setProperty('--sch-pad-y', pad + 'px');
    }
    if ('ResizeObserver' in window) new ResizeObserver(fit).observe(root);

    if (reduced) {
      // One still: Monday's run is done and its chat heads Recent chats.
      settle(function () {
        seed(2, true);
        wrap.classList.add('fired');
        progress.style.opacity = '0';
      });
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

    function rest() {
      settle(function () {
        wrap.classList.remove('running', 'fired');
        progress.style.opacity = '1';
        setStep(0);
        seed(latest, false);
      });
    }

    function addRun() {
      var n = latest + 1;
      var rows = Array.prototype.slice.call(list.children);
      latest = n;
      // Every row moves down one slot in the same frame. The oldest leaves through
      // the list's lower edge while it fades (--dur-fast), so it is almost clear
      // before it reaches the edge, and the two rows move in step, so they never
      // cross. The new chat waits CLEAR ms, until the row leaving the top slot is
      // past its text, then fades in.
      var fresh = runRow(n);
      place(fresh, 0);
      fresh.classList.add('is-in', 'is-new');
      rows.forEach(function (r) {
        r.classList.remove('is-new');
        if (r.__slot + 1 >= VISIBLE) r.classList.add('is-out');
        place(r, r.__slot + 1);
      });
      list.insertBefore(fresh, list.firstChild);
      at(CLEAR, function () { fresh.classList.remove('is-in'); });
      at(900, function () {
        Array.prototype.forEach.call(list.querySelectorAll('.is-out'), function (r) { r.parentNode.removeChild(r); });
      });
    }

    function finish() {
      wrap.classList.remove('running');
      last.classList.add('is-swap');
      at(175, function () { lastDate.textContent = short(monday(latest)); last.classList.remove('is-swap'); });
    }

    function cycle() {
      // Monday 07:00: the run starts, and its chat joins the list.
      at(400, function () { wrap.classList.add('fired'); });
      at(900, function () { wrap.classList.add('running'); });
      at(1400, addRun);
      // It finishes: Scheduled again, and Last run takes the new date.
      at(3800, finish);
      at(4300, function () { var f = list.firstChild; if (f) f.classList.remove('is-new'); });
      // The week goes by, a day at a time. Monday empties as the line leaves it.
      at(4900, function () { wrap.classList.remove('fired'); });
      for (var k = 1; k <= 6; k++) (function (k) { at(4900 + (k - 1) * 900, function () { setStep(k); }); })(k);
      // Sunday: the line fades and returns to Monday, ready for the next run.
      at(10200, function () { progress.style.opacity = '0'; });
      at(10600, function () { settle(function () { setStep(0); progress.style.opacity = '1'; }); });
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
