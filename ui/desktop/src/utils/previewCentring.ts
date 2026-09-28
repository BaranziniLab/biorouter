/**
 * Centres a previewed HTML document whose content is narrower than the frame.
 *
 * The panel frames an HTML artifact the way a browser tab would, and a browser
 * starts normal flow at the left edge. A fixed-size canvas (a 300px chart, an
 * HTML "slide") therefore sat flush against the panel's left edge with all the
 * spare width on its right, at every panel width: 0/59 px at 360 wide, 0/619 px
 * at 920. PowerPoint, Word, PDF and image previews are centred by the panel
 * itself. An HTML document cannot be, because it runs in a sandboxed `srcdoc`
 * frame with an opaque origin the panel may not script. So the panel injects
 * this runtime next to its size reporter, and the document centres its own
 * content.
 *
 * **What moves.** Starting at `<body>`, it looks through every wrapper that
 * draws nothing and holds no text of its own (an `#app` or `<main>` root) to the
 * first boxes that are actually seen, and gives all of them the SAME horizontal
 * `translate`, so their arrangement relative to each other is kept. `translate`
 * never changes layout, so a box sized in percentages keeps its size and the
 * shift cannot feed back into the next measurement.
 *
 * **What never moves.** The runtime leaves the document exactly as a browser
 * lays it out unless it can move the content without changing how any of it is
 * positioned:
 *  - content as wide as the frame, or wider, is left alone (it scrolls from its
 *    own left edge, as in a browser);
 *  - content already centred, to within half a pixel, is left alone;
 *  - text sitting directly in `<body>`, or a bare inline element, cannot be
 *    translated, so nothing moves;
 *  - a box the author already translates is never overwritten;
 *  - a box whose `fixed` or `absolute` descendants would re-anchor to it (a
 *    transform makes an element their containing block) stops the move;
 *  - `fixed` and `absolute` boxes outside the moved content, such as a
 *    navigation bar or a tooltip placed at the pointer's page coordinates, are
 *    neither measured nor moved.
 *
 * **Which width it centres in.** The frame's full width (`innerWidth`), so the
 * space left of the content equals the space right of it up to the frame's
 * edge. That is the rule the document previews follow with `scrollbar-gutter:
 * stable both-edges`. With a classic scrollbar the content is never pushed under
 * the bar.
 *
 * Injected by the panel alone, through `withPreviewSizeReporting`. The standalone
 * window and the open-in-browser page show the document as a browser would.
 * `previewCentring.browser.test.ts` measures it in a real Chromium, because
 * jsdom lays nothing out.
 */
export const PREVIEW_CENTRING_INSTALL = `(() => {
  if (window.parent === window) return;
  const key = Symbol.for('biorouter.preview.centre.v1');
  if (window[key]) return;
  window[key] = true;
  const MAX_DEPTH = 8;
  const MAX_BOXES = 200;
  const MAX_WALK = 4000;
  const REPLACED = new Set(['img', 'canvas', 'svg', 'video', 'iframe', 'object', 'embed', 'audio']);
  const DESCEND = new Set(['block', 'flow-root', 'flex', 'grid', 'list-item']);
  const moved = new Map();
  let applied = 0;
  let queued = false;
  let dirty = true;
  const px = (value) => Number.parseFloat(value) || 0;
  const clear = (color) => color === 'transparent' || /^rgba\\(.*,\\s*0\\)$/.test(color);
  const background = (style) => !clear(style.backgroundColor) || style.backgroundImage !== 'none';
  const edges = (style) =>
    style.boxShadow !== 'none' ||
    px(style.borderLeftWidth) + px(style.borderRightWidth) + px(style.borderTopWidth) + px(style.borderBottomWidth) > 0 ||
    (style.outlineStyle !== 'none' && px(style.outlineWidth) > 0);
  const hasText = (el) => {
    for (const node of el.childNodes) if (node.nodeType === 3 && /\\S/.test(node.data)) return true;
    return false;
  };
  const shiftOf = (el) => {
    let total = 0;
    for (let at = el; at; at = at.parentElement) if (moved.has(at)) total += applied;
    return total;
  };
  const ours = (el, style) => style.translate === 'none' || moved.has(el);
  // A body's background paints the canvas, not the body box, unless the root has one of its own.
  const draws = (el, style) => {
    if (el !== document.body) return background(style) || edges(style);
    return edges(style) || (background(style) && background(getComputedStyle(document.documentElement)));
  };
  const descends = (el, style) => {
    if (style.display === 'contents') return true;
    if (REPLACED.has(el.localName) || !DESCEND.has(style.display)) return false;
    if (el !== document.body && style.position !== 'static') return false;
    if (style.overflowX !== 'visible' || style.overflowY !== 'visible') return false;
    if (style.transform !== 'none' || !ours(el, style)) return false;
    return !draws(el, style) && !hasText(el);
  };
  const collect = (el, depth, out) => {
    const style = getComputedStyle(el);
    if (depth < MAX_DEPTH && out.length < MAX_BOXES && descends(el, style)) {
      for (const child of el.children) {
        const cs = getComputedStyle(child);
        if (cs.display === 'none' || cs.position === 'fixed' || cs.position === 'absolute') continue;
        collect(child, depth + 1, out);
      }
      return;
    }
    out.push({ el, style });
  };
  // Would a transform on this box re-anchor one of its positioned descendants?
  const reanchors = (el, style, budget) => {
    const contains = style.position !== 'static' || style.transform !== 'none';
    const all = el.getElementsByTagName('*');
    budget.walked += all.length;
    if (budget.walked > MAX_WALK) return true;
    for (const node of all) {
      const position = getComputedStyle(node).position;
      if (position === 'fixed') return true;
      if (position !== 'absolute' || contains) continue;
      let inner = false;
      for (let at = node.parentElement; at && at !== el; at = at.parentElement) {
        const as = getComputedStyle(at);
        if (as.position !== 'static' || as.transform !== 'none') {
          inner = true;
          break;
        }
      }
      if (!inner) return true;
    }
    return false;
  };
  const apply = (boxes, shift) => {
    const keep = new Set(boxes);
    for (const [el, inline] of moved) {
      if (keep.has(el)) continue;
      el.style.translate = inline;
      moved.delete(el);
    }
    applied = boxes.length ? shift : 0;
    for (const el of boxes) {
      if (!moved.has(el)) moved.set(el, el.style.translate);
      const value = applied + 'px';
      if (el.style.translate !== value) el.style.translate = value;
    }
  };
  const settle = () => {
    queued = false;
    const body = document.body;
    const root = document.documentElement;
    if (!body || !root) return;
    const visible = root.clientWidth;
    if (visible <= 0) return;
    // Wider than the frame: it scrolls from its own left edge, as in a browser.
    // A shift already applied can push the document's scroll width past the frame
    // by exactly that shift, so it is taken off before deciding.
    const width = root.scrollWidth;
    if ((width > visible ? width - Math.max(applied, 0) : width) > visible + 0.5) return apply([], 0);
    const found = [];
    collect(body, 0, found);
    const boxes = [];
    let left = Infinity;
    let right = -Infinity;
    const scroll = window.scrollX || 0;
    for (const { el, style } of found) {
      const rect = el.getBoundingClientRect();
      if (rect.width === 0) continue;
      if (style.display === 'inline' && !REPLACED.has(el.localName)) return apply([], 0);
      const offset = scroll - shiftOf(el);
      let end = rect.right;
      if (style.overflowX === 'visible' && style.transform === 'none' && !REPLACED.has(el.localName)) {
        end = Math.max(end, rect.left + el.clientLeft + el.scrollWidth);
      }
      left = Math.min(left, rect.left + offset);
      right = Math.max(right, end + offset);
      boxes.push({ el, style });
    }
    // Nothing to centre, wider than the frame, or placed partly off its left edge on purpose.
    if (!boxes.length || right > visible + 0.5 || left < -0.5) return apply([], 0);
    const ratio = window.devicePixelRatio || 1;
    const frame = window.innerWidth || visible;
    let shift = Math.round(((frame - right - left) / 2) * ratio) / ratio;
    shift = Math.max(-left, Math.min(shift, visible - right));
    if (Math.abs(shift) < 0.5) return apply([], 0);
    const same = boxes.length === moved.size && boxes.every(({ el }) => moved.has(el));
    if (same && shift === applied && !dirty) return;
    dirty = false;
    const budget = { walked: 0 };
    for (const { el, style } of boxes) {
      if (!ours(el, style) || reanchors(el, style, budget)) return apply([], 0);
    }
    apply(boxes.map(({ el }) => el), shift);
  };
  const queue = () => {
    if (queued) return;
    queued = true;
    setTimeout(settle, 16);
  };
  const watch = () => {
    settle();
    const body = document.body;
    if (!body) return;
    const sizes = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(queue);
    const observe = () => {
      if (!sizes) return;
      sizes.observe(body);
      for (const child of body.children) sizes.observe(child);
      for (const el of moved.keys()) sizes.observe(el);
    };
    observe();
    if (typeof MutationObserver !== 'undefined') {
      new MutationObserver((records) => {
        if (records.some((record) => record.type === 'childList')) dirty = true;
        observe();
        queue();
      }).observe(body, { childList: true, subtree: true, attributes: true, attributeFilter: ['class', 'style', 'hidden'] });
    }
  };
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', watch);
  else watch();
  window.addEventListener('load', queue);
  window.addEventListener('resize', queue);
})()`;
