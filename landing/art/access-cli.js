/* Command line tile (Ways to use Biorouter).
   A plain terminal: no banner, a $ prompt. Two one-shot runs, each printing
   only its answer, and after each the shell reports exit code 0.
   The commands are the ones docs.html#cli-run gives (`-t` for the request,
   `-i -` to read it from stdin), with `-q`, which prints only the model
   response on stdout (crates/biorouter-cli/src/cli.rs). Exit code 0 means
   Finished (docs.html#cli-exit-codes). The two answers are illustrative model
   output, not a recorded run.
   The tile wraps long commands itself, at whole words, with a hanging indent,
   so a word never jumps to the next line while it is typed. When the second
   run would not fit under the first, the screen scrolls up once, before it is
   typed. One thing moves at a time. */
(function () {
  'use strict';
  if (!window.BR || !BR.art) return;

  var PROMPT = '$ ';
  var INDENT = 2; // a wrapped command continues under its first character
  var ROWS = [
    { cmd: 'biorouter run -q -t "Check samples.csv for duplicate sample IDs"' },
    { out: 'No duplicate sample IDs in samples.csv.' },
    { cmd: 'echo $?' },
    { out: '0' },
    { cmd: 'cat plan.md | biorouter run -q -i -' },
    { out: 'Plan checked: 3 steps, no conflicts.' },
    { cmd: 'echo $?' },
    { out: '0' },
    { cmd: '' }
  ];
  var RUN2 = 4; // the row the second run is typed on

  function el(tag, cls, text) {
    var n = document.createElement(tag);
    if (cls) n.className = cls;
    if (text != null) n.textContent = text;
    return n;
  }

  // Greedy word wrap into [start, end) ranges of `text`. The space a line
  // breaks at belongs to neither line. Lines after the first are `indent`
  // columns narrower.
  function wrap(text, cols, indent) {
    var segs = [];
    var start = 0;
    var first = true;
    while (start < text.length) {
      var width = first ? cols : cols - indent;
      if (text.length - start <= width) { segs.push([start, text.length]); break; }
      var cut = text.lastIndexOf(' ', start + width);
      if (cut <= start + (first ? PROMPT.length : 0)) { segs.push([start, start + width]); start += width; }
      else { segs.push([start, cut]); start = cut + 1; }
      first = false;
    }
    if (!segs.length) segs.push([0, 0]);
    return segs;
  }

  BR.art('access-cli', function (root, opts) {
    var reduced = !!(opts && opts.reduced);
    var wrapEl = el('div', 'acli');
    wrapEl.setAttribute('aria-hidden', 'true');
    var pane = el('div', 'acli-pane');
    var view = el('div', 'acli-view');
    var lines = el('div', 'acli-lines');
    var probe = el('span', 'acli-probe', '0000000000000000000000000000000000000000');
    var caret = el('i', 'acli-caret');
    view.appendChild(lines);
    view.appendChild(probe);
    pane.appendChild(view);
    wrapEl.appendChild(pane);
    root.appendChild(wrapEl);

    // Where the cursor is: rows before `row` are done; on `row` the command has
    // `typed` characters, or the output row is still empty while the run works.
    var st = { row: 0, typed: 0 };
    var cols = 40;
    var lh = 16;
    var capacity = 12;
    var off = 0;        // lines scrolled off the top
    var room = false;   // the second run has been given room

    function segsOf(r) {
      var row = ROWS[r];
      return row.cmd != null ? wrap(PROMPT + row.cmd, cols, INDENT) : wrap(row.out, cols, 0);
    }
    // The line each row starts on, when every row is complete.
    function startLine(r) {
      var n = 0;
      for (var k = 0; k < r; k++) n += segsOf(k).length;
      return n;
    }

    // Draw the transcript up to the cursor. Rows from `fresh` on fade in.
    function render(fresh) {
      lines.textContent = '';
      var count = 0;
      var host = null;
      for (var r = 0; r <= st.row && r < ROWS.length; r++) {
        var row = ROWS[r];
        var cur = r === st.row;
        var rowEl = el('div', 'acli-row ' + (row.cmd != null ? 'is-cmd' : 'is-out'));
        if (fresh != null && r >= fresh) rowEl.classList.add('is-new');
        if (row.cmd != null) {
          var full = PROMPT + row.cmd;
          var n = PROMPT.length + (cur ? st.typed : row.cmd.length);
          var segs = wrap(full, cols, INDENT);
          for (var k = 0; k < segs.length; k++) {
            var s = segs[k];
            if (k > 0 && s[0] > n) break;
            if (k > 0 && s[0] === n && !cur) break;
            var ln = el('div', 'acli-ln' + (k > 0 ? ' is-cont' : ''));
            if (k === 0) {
              ln.appendChild(el('span', 'acli-p', '$'));
              ln.appendChild(el('span', 'acli-t', ' ' + full.slice(PROMPT.length, Math.min(s[1], n))));
            } else {
              ln.appendChild(el('span', 'acli-t', full.slice(s[0], Math.min(s[1], n))));
            }
            rowEl.appendChild(ln);
            count++;
            if (cur) host = ln;
          }
        } else if (cur) {
          // The run is working: the cursor waits on an empty line.
          host = el('div', 'acli-ln');
          rowEl.appendChild(host);
          count++;
        } else {
          wrap(row.out, cols, 0).forEach(function (s) {
            rowEl.appendChild(el('div', 'acli-ln', row.out.slice(s[0], s[1])));
            count++;
          });
        }
        rowEl.__end = count;
        if (count <= off) rowEl.classList.add('is-gone');
        lines.appendChild(rowEl);
      }
      if (host) host.appendChild(caret);
      return count - 1; // the cursor's line
    }

    // Scroll by whole lines. A row that leaves the screen fades quickly while
    // the block moves, so it is almost clear before it reaches the top edge.
    function scrollTo(lineCount, animate) {
      if (lineCount === off && !animate) return;
      off = lineCount;
      lines.style.transform = off ? 'translateY(' + (-off * lh) + 'px)' : '';
      Array.prototype.forEach.call(lines.children, function (r) {
        r.classList.toggle('is-gone', r.__end <= off);
      });
    }
    // Keep the cursor's line in view, as a terminal does.
    function follow(cursorLine) {
      if (cursorLine >= off + capacity) scrollTo(cursorLine - capacity + 1);
    }
    // Before the second run: scroll once so all of it, and the prompt after
    // it, fits under what stays of the first. The top line is then the start
    // of a row, never the tail of a wrapped answer.
    function roomLine() {
      var over = startLine(ROWS.length) - capacity;
      if (over <= 0) return 0;
      for (var r = 0; r <= RUN2; r++) {
        var s = startLine(r);
        if (s >= over) return s;
      }
      return startLine(RUN2);
    }
    function makeRoom() {
      room = true;
      var need = roomLine();
      if (need > off) scrollTo(need);
    }

    function draw(fresh) { follow(render(fresh)); }

    function settle(fn) {
      wrapEl.classList.add('acli-still');
      fn();
      void wrapEl.offsetWidth;
      wrapEl.classList.remove('acli-still');
    }

    // Columns and rows from the tile's own size. Mono, so one glyph's advance
    // sets the columns; a hair is kept free for the cursor after a full line.
    function fit() {
      var w = view.clientWidth;
      var h = view.clientHeight;
      if (w < 40 || h < 20) return;
      var cw = probe.getBoundingClientRect().width / probe.textContent.length;
      lh = parseFloat(getComputedStyle(view).lineHeight) || 16;
      cols = Math.max(16, Math.floor((w - 2) / cw));
      capacity = Math.max(3, Math.floor(h / lh));
      settle(function () {
        var cursorLine = render();
        off = 0;
        var target = Math.max(0, cursorLine - capacity + 1);
        if (room) target = Math.max(target, roomLine());
        scrollTo(target, true);
      });
    }
    if ('ResizeObserver' in window) new ResizeObserver(fit).observe(root);
    if (document.fonts && document.fonts.ready) document.fonts.ready.then(fit);

    if (reduced) {
      // One still: both runs done, the second in full, and the shell waiting
      // at its prompt. The first run stays above it where the height allows.
      st.row = ROWS.length - 1;
      room = true;
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
        wrapEl.classList.remove('is-out');
        st.row = 0; st.typed = 0; room = false;
        off = 0; lines.style.transform = '';
        render(0);
      });
    }

    // Type row r's command from `from` ms, one character every `per` ms.
    // Returns the time of the last character.
    function type(r, from, per) {
      var text = ROWS[r].cmd;
      for (var i = 1; i <= text.length; i++) (function (i) {
        at(from + (i - 1) * per, function () { st.typed = i; draw(); });
      })(i);
      return from + (text.length - 1) * per;
    }
    // Enter: the cursor drops to the empty line under the command.
    function enter(ms, r) { at(ms, function () { st.row = r + 1; st.typed = 0; draw(); }); }
    // The answer prints whole, and the next prompt with it.
    function print(ms, r) { at(ms, function () { st.row = r + 1; st.typed = 0; draw(r); }); }

    function cycle() {
      // First run: a request on the command line. The run works for a beat
      // with nothing on screen, as -q prints nothing but the answer.
      type(0, 300, 34);
      enter(2600, 0);
      print(3100, 1);
      // The shell reports how it ended.
      type(2, 3800, 55);
      enter(4300, 2);
      print(4550, 3);
      // Second run: the request comes from a file, through stdin. Any scroll
      // it needs ends before the first character.
      at(5500, makeRoom);
      type(4, 6150, 36);
      enter(7550, 4);
      print(8050, 5);
      type(6, 8700, 55);
      enter(9200, 6);
      print(9450, 7);
      // Hold on the finished screen, then clear it and start again.
      at(10500, function () { wrapEl.classList.add('is-out'); });
      at(10850, rest);
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
