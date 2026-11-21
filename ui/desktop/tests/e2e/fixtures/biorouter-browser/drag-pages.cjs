// Drag and drop pages for the Biorouter Browser's agent (AD-17), served by `site.cjs` under
// /agent/drag/. Each page reproduces how a real drag library behaves, inline and without the
// network, and writes what happened where the spec can read it:
//
// - /agent/drag/zone: native HTML5 drop zones. The card being dragged is hidden on the tick
//   after `dragstart`, as many kanban pages do, so everything below it moves up while the
//   drag is open: a drop point measured before the press lands one zone too low.
// - /agent/drag/native: a list in the style of SortableJS's native mode. An item becomes
//   draggable on `pointerdown`, drags are accepted only from the tick after `dragstart`, and
//   the list reorders live on `dragover` by index. A second list repeats the same words.
// - /agent/drag/mouse: a list in the style of jQuery UI sortable: mouse events only, a start
//   threshold, `which` checked on every move, and the "intersect" rule (moving down, an item
//   is passed only once the pointer is in its lower half).
// - /agent/drag/affordances: what the snapshot should give references to, and a flood of
//   draggable tiles it must cap.
//
// Every value is synthetic.

'use strict';

const ZONE = `<style>
  .zone{border:1px dashed #888;padding:6px 10px;margin:6px 0;min-height:44px;width:420px}
  .zone h2{margin:0;font-size:15px}
  .card{height:34px;line-height:34px;margin:6px 0;padding:0 10px;background:#eef;width:380px}
  .lifted{display:none}
</style>
<h1>Board</h1>
<section id="todo" class="zone" aria-label="To do"><h2>To do</h2>
  <div class="card" draggable="true" id="card-a">Write the report</div>
  <div class="card" draggable="true" id="card-b">Review the data</div>
  <div class="card" draggable="true" id="card-c">Send the samples</div>
</section>
<section id="done" class="zone" aria-label="Done"><h2>Done</h2></section>
<section id="later" class="zone" aria-label="Later"><h2>Later</h2></section>
<label>Password <input id="pw" type="password" autocomplete="current-password"></label>
<script>
  let dragged = null;
  document.addEventListener('dragstart', (e) => {
    const card = e.target.closest && e.target.closest('.card');
    if (!card) return;
    dragged = card;
    e.dataTransfer.setData('text/plain', card.id);
    setTimeout(() => card.classList.add('lifted'), 0);
  });
  document.addEventListener('dragend', () => {
    if (dragged) dragged.classList.remove('lifted');
    dragged = null;
  });
  for (const zone of document.querySelectorAll('.zone')) {
    zone.addEventListener('dragover', (e) => e.preventDefault());
    zone.addEventListener('drop', (e) => {
      e.preventDefault();
      const id = e.dataTransfer.getData('text/plain');
      const card = document.getElementById(id);
      if (card) zone.appendChild(card);
      document.title = 'dropped ' + id + ' in ' + zone.id;
    });
  }
</script>`;

const NATIVE = `<style>
  .list{width:300px;margin:8px 0}
  .item{padding:8px 12px;margin:0;border:1px solid #ccc;background:#fff}
  .ghost{opacity:.4}
</style>
<h1>Native list</h1>
<div id="native-list" class="list">
  <div class="item">Item 1</div><div class="item">Item 2</div><div class="item">Item 3</div><div class="item">Item 4</div>
</div>
<h2>Another list with the same words</h2>
<div id="other-list" class="list">
  <div class="item">Item 1</div><div class="item">Item 2</div><div class="item">Item 3</div><div class="item">Item 4</div>
</div>
<script>
  for (const list of document.querySelectorAll('.list')) {
    let dragEl = null;
    let started = false;
    const finish = () => {
      if (!dragEl) return;
      dragEl.classList.remove('ghost');
      dragEl.draggable = false;
      dragEl = null;
      started = false;
      list.dataset.done = String(Number(list.dataset.done || 0) + 1);
    };
    list.addEventListener('pointerdown', (e) => {
      const item = e.target.closest('.item');
      if (!item || e.button !== 0 || item.parentNode !== list) return;
      dragEl = item;
      item.draggable = true;
    });
    list.addEventListener('pointerup', () => { if (dragEl && !started) finish(); });
    list.addEventListener('dragstart', (e) => {
      if (!dragEl) return;
      e.dataTransfer.effectAllowed = 'move';
      e.dataTransfer.setData('Text', dragEl.textContent);
      // Like SortableJS: the drag counts from the next tick, and dragovers before it do nothing.
      setTimeout(() => { if (dragEl) { started = true; dragEl.classList.add('ghost'); } }, 0);
    });
    list.addEventListener('dragover', (e) => {
      if (!dragEl) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = 'move';
      if (!started) return;
      const target = e.target.closest && e.target.closest('.item');
      if (!target || target === dragEl || target.parentNode !== list) return;
      const items = [...list.children];
      if (items.indexOf(dragEl) < items.indexOf(target)) target.after(dragEl);
      else target.before(dragEl);
    });
    list.addEventListener('drop', (e) => e.preventDefault());
    list.addEventListener('dragend', finish);
  }
</script>`;

const MOUSE = `<style>
  #mouse-list{list-style:none;margin:8px 0;padding:0;width:320px;position:relative;cursor:move}
  #mouse-list li{height:22px;line-height:22px;margin:0 0 3px 0;padding:4px 8px 4px 24px;border:1px solid #bbb;background:#f6f6f6}
  #mouse-list li .ui-icon{display:inline-block;width:12px;height:12px;margin-left:-16px;margin-right:4px;background:#999}
  #mouse-list li.placeholder{background:#ffd;border:1px dashed #aa8}
  #mouse-list li.helper{position:absolute;left:0;width:288px;z-index:5;opacity:.85}
</style>
<h1>Mouse list</h1>
<ul id="mouse-list">
  <li><span class="ui-icon"></span>Item 1</li><li><span class="ui-icon"></span>Item 2</li><li><span class="ui-icon"></span>Item 3</li><li><span class="ui-icon"></span>Item 4</li><li><span class="ui-icon"></span>Item 5</li>
</ul>
<script>
  // In the style of jQuery UI sortable: mouse events only, a 3 px start threshold, \`which\`
  // checked on every move, and the "intersect" tolerance.
  const list = document.getElementById('mouse-list');
  let drag = null;
  list.addEventListener('mousedown', (e) => {
    const item = e.target.closest('li');
    if (!item || e.which !== 1) return;
    e.preventDefault();
    const box = item.getBoundingClientRect();
    drag = { item, x: e.pageX, y: e.pageY, lastY: e.pageY, grabY: e.clientY - box.top, started: false };
  });
  const stop = () => {
    if (drag && drag.started) {
      drag.placeholder.replaceWith(drag.item);
      drag.item.classList.remove('helper');
      drag.item.style.top = '';
      list.dataset.done = String(Number(list.dataset.done || 0) + 1);
    }
    drag = null;
  };
  document.addEventListener('mousemove', (e) => {
    if (!drag) return;
    if (!e.which) return stop();
    if (!drag.started) {
      if (Math.max(Math.abs(e.pageX - drag.x), Math.abs(e.pageY - drag.y)) < 3) return;
      drag.placeholder = document.createElement('li');
      drag.placeholder.className = 'placeholder';
      drag.item.before(drag.placeholder);
      drag.item.classList.add('helper');
      drag.started = true;
    }
    const listTop = list.getBoundingClientRect().top;
    drag.item.style.top = (e.clientY - listTop - drag.grabY) + 'px';
    const dir = e.pageY - drag.lastY;
    drag.lastY = e.pageY;
    if (!dir) return;
    for (const other of [...list.children].reverse()) {
      if (other === drag.item || other === drag.placeholder) continue;
      const r = other.getBoundingClientRect();
      if (e.clientY < r.top || e.clientY >= r.bottom) continue;
      const half = r.top + r.height / 2;
      if (dir > 0 && e.clientY >= half) { other.after(drag.placeholder); break; }
      if (dir < 0 && e.clientY < half) { other.before(drag.placeholder); break; }
    }
  });
  document.addEventListener('mouseup', () => stop());
</script>`;

const AFFORDANCES = `<style>
  .card{cursor:grab;border:1px solid #ccc;padding:6px;margin:6px 0;width:300px}
  .card h3{margin:0;font-size:14px} .card p{margin:2px 0}
  #moves{cursor:move;list-style:none;padding:0} #moves li{padding:4px;border-bottom:1px solid #eee}
  .row{display:flex;gap:8px;padding:4px} .handle{cursor:grab;width:16px}
  #tiles{display:flex;flex-wrap:wrap;gap:2px} #tiles div{cursor:grab;width:30px;height:20px;background:#eef;font-size:9px}
</style>
<h1>Affordances</h1>
<div class="card"><h3>Card Alpha</h3><p>First line</p><p>Second line</p></div>
<div class="card"><h3>Card Beta</h3><p>First line</p><p>Second line</p></div>
<ul id="moves"><li>Move one</li><li>Move two</li><li>Move three</li></ul>
<div draggable="true" id="native-box">Native box</div>
<div draggable="true" style="display:none">Hidden box</div>
<div class="row"><span class="handle">⋮⋮</span><span>Alpha task</span></div>
<div class="row"><span class="handle">⋮⋮</span><span>Beta task</span></div>
<button style="cursor:move">Move button</button>
<div id="tiles">${Array.from({ length: 300 }, (_, i) => `<div>T${i + 1}</div>`).join('')}</div>`;

/** The page for `pathname`, or null. */
function dragPage(pathname) {
  switch (pathname) {
    case '/agent/drag/zone':
      return { title: 'Drag zone', body: ZONE };
    case '/agent/drag/native':
      return { title: 'Drag native', body: NATIVE };
    case '/agent/drag/mouse':
      return { title: 'Drag mouse', body: MOUSE };
    case '/agent/drag/affordances':
      return { title: 'Drag affordances', body: AFFORDANCES };
    default:
      return null;
  }
}

module.exports = { dragPage };
