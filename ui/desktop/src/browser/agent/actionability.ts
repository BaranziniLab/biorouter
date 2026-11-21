/**
 * The checks every agent action runs before it touches a page (BB-13), as pure decisions, and
 * the in-page functions that gather their inputs.
 *
 * The rule is agent-browser's most useful trait: an action fails with a reason rather than
 * reporting a success that did not happen. A hidden, disabled, unstable or covered element is
 * refused and the reason names what is in the way.
 *
 * The functions below that are written as source strings run in the page through
 * `Runtime.callFunctionOn` in an isolated world, so the page's own script cannot replace the
 * DOM methods they call.
 */

export type Box = { x: number; y: number; width: number; height: number };

export type ElementFacts = {
  connected: boolean;
  tag: string;
  visibility: string;
  display: string;
  disabled: boolean;
  /** The nearest `<a href>` at or above the element: its resolved address. */
  href: string | null;
  download: boolean;
  type: string;
  autocomplete: string;
  editable: boolean;
  readOnly: boolean;
  role: string;
  /** The page's own address when the facts were read, to tell a fragment link from navigation. */
  pageUrl: string;
};

/**
 * Collected in the page. `this` is the element (or a text node inside one).
 */
export const ELEMENT_FACTS_FUNCTION = `function () {
  const el = this && this.nodeType === 1 ? this : this && this.parentElement;
  if (!el) return { connected: false, tag: '', visibility: 'hidden', display: 'none', disabled: false, href: null, download: false, type: '', autocomplete: '', editable: false, readOnly: false, role: '', pageUrl: location.href };
  const style = getComputedStyle(el);
  const tag = el.tagName.toLowerCase();
  const formControl = /^(button|input|select|textarea|option|optgroup|fieldset)$/.test(tag);
  const disabled = (formControl && (el.disabled === true || !!el.closest('fieldset[disabled]')))
    || el.getAttribute('aria-disabled') === 'true';
  const anchor = el.closest('a[href], area[href]');
  return {
    connected: el.isConnected,
    tag,
    visibility: style.visibility,
    display: style.display,
    disabled,
    href: anchor ? anchor.href : null,
    download: anchor ? anchor.hasAttribute('download') : false,
    type: (el.getAttribute('type') || '').toLowerCase(),
    autocomplete: (el.getAttribute('autocomplete') || '').toLowerCase(),
    editable: el.isContentEditable === true,
    readOnly: el.readOnly === true,
    role: (el.getAttribute('role') || '').toLowerCase(),
    pageUrl: location.href,
  };
}`;

/** Two boxes read two animation frames apart, to see that an element is not moving. */
export const STABLE_BOX_FUNCTION = `function () {
  const el = this && this.nodeType === 1 ? this : this && this.parentElement;
  if (!el) return null;
  const read = () => { const r = el.getBoundingClientRect(); return { x: r.x, y: r.y, width: r.width, height: r.height }; };
  const first = read();
  return new Promise((resolve) => {
    let done = false;
    const finish = () => { if (done) return; done = true; resolve({ first, second: read() }); };
    requestAnimationFrame(() => requestAnimationFrame(finish));
    // A hidden or throttled page may never run animation frames.
    setTimeout(finish, 120);
  });
}`;

/** Whether `this` (the target) is, or contains, the node that was hit, across shadow roots. */
export const CONTAINS_FUNCTION = `function (hit) {
  let node = hit;
  while (node) {
    if (node === this) return true;
    node = node.parentNode || (node instanceof ShadowRoot ? node.host : null);
  }
  return false;
}`;

/** A short description of what covers the target: `[dialog] "Cookie consent"`. */
export const DESCRIBE_COVER_FUNCTION = `function () {
  const start = this && this.nodeType === 1 ? this : this && this.parentElement;
  if (!start) return 'something';
  const named = start.closest('dialog,[role=dialog],[role=alertdialog],[aria-modal=true],[role],button,a,[aria-label],[id]') || start;
  const role = named.getAttribute('role') || (named.tagName === 'DIALOG' ? 'dialog' : named.tagName.toLowerCase());
  const label = (named.getAttribute('aria-label') || named.getAttribute('title') || named.textContent || named.id || '').replace(/\\s+/g, ' ').trim().slice(0, 60);
  return label ? '[' + role + '] "' + label.replace(/"/g, "'") + '"' : '[' + role + ']';
}`;

export type ActionCheck = { ok: true } | { ok: false; reason: string };

/** Visible and enabled (BB-13 step 5). `needsEnabled` is false for a hover. */
export function checkElement(
  facts: ElementFacts,
  box: Box | null,
  needsEnabled: boolean,
  label: string
): ActionCheck {
  if (!facts.connected) {
    return { ok: false, reason: `${label} is no longer on the page. Take a new browser_snapshot.` };
  }
  if (
    facts.display === 'none' ||
    facts.visibility === 'hidden' ||
    facts.visibility === 'collapse'
  ) {
    return { ok: false, reason: `${label} is hidden, so it cannot be used.` };
  }
  if (!box || box.width <= 0 || box.height <= 0) {
    return { ok: false, reason: `${label} has no size on the page (it is hidden or collapsed).` };
  }
  if (needsEnabled && facts.disabled) {
    return { ok: false, reason: `${label} is disabled.` };
  }
  return { ok: true };
}

/** The element did not move between two frames (BB-13 step 5, "stable"). */
export function isStable(first: Box | null, second: Box | null): boolean {
  if (!first || !second) return false;
  const close = (a: number, b: number) => Math.abs(a - b) < 1;
  return (
    close(first.x, second.x) &&
    close(first.y, second.y) &&
    close(first.width, second.width) &&
    close(first.height, second.height)
  );
}

/** The part of the first quad inside the viewport that is at least a pixel each way. */
function visibleBox(
  quads: number[][],
  viewport: { width: number; height: number }
): { minX: number; maxX: number; minY: number; maxY: number } | null {
  for (const quad of quads) {
    if (quad.length < 8) continue;
    const xs = [quad[0], quad[2], quad[4], quad[6]];
    const ys = [quad[1], quad[3], quad[5], quad[7]];
    const minX = Math.max(0, Math.min(...xs));
    const maxX = Math.min(viewport.width, Math.max(...xs));
    const minY = Math.max(0, Math.min(...ys));
    const maxY = Math.min(viewport.height, Math.max(...ys));
    if (maxX - minX < 1 || maxY - minY < 1) continue;
    return { minX, maxX, minY, maxY };
  }
  return null;
}

/** The point an action aims at: the centre of the first quad, inside the viewport. */
export function clickPoint(
  quads: number[][],
  viewport: { width: number; height: number }
): { x: number; y: number } | null {
  const box = visibleBox(quads, viewport);
  return box ? { x: (box.minX + box.maxX) / 2, y: (box.minY + box.maxY) / 2 } : null;
}

/**
 * Where a drag drops on its destination (AD-17): inside the destination's visible part, a
 * quarter of its size past the centre in the main direction of travel. A person moving an item
 * onto another goes past its middle, and a sortable that inserts before or after by halves
 * (jQuery UI's default tolerance, and most hand-written lists) then puts the item on the far
 * side, where it was taken; a drop zone gets a point well inside it. `from` is where the drag
 * starts, in the same coordinates as the quads.
 */
export function dropPointIn(
  quads: number[][],
  viewport: { width: number; height: number },
  from: { x: number; y: number }
): { x: number; y: number } | null {
  const box = visibleBox(quads, viewport);
  if (!box) return null;
  const x = (box.minX + box.maxX) / 2;
  const y = (box.minY + box.maxY) / 2;
  const dx = x - from.x;
  const dy = y - from.y;
  if (Math.abs(dx) < 1 && Math.abs(dy) < 1) return { x, y };
  if (Math.abs(dy) >= Math.abs(dx)) {
    return { x, y: y + (Math.sign(dy) * (box.maxY - box.minY)) / 4 };
  }
  return { x: x + (Math.sign(dx) * (box.maxX - box.minX)) / 4, y };
}

export function boxOfQuads(quads: number[][]): Box | null {
  const quad = quads.find((candidate) => candidate.length >= 8);
  if (!quad) return null;
  const xs = [quad[0], quad[2], quad[4], quad[6]];
  const ys = [quad[1], quad[3], quad[5], quad[7]];
  return {
    x: Math.min(...xs),
    y: Math.min(...ys),
    width: Math.max(...xs) - Math.min(...xs),
    height: Math.max(...ys) - Math.min(...ys),
  };
}

/**
 * A click the observe grant covers (BB-17): following a plain link to another http(s) page of
 * the page's own origin. A link that leaves the origin (a page that injected instructions could
 * aim the agent at its own server), a link that downloads, a fragment link to the same page
 * (usually a script's button in disguise), and everything that is not a link need the act grant.
 */
export function isPlainLinkClick(
  facts: Pick<ElementFacts, 'href' | 'download' | 'pageUrl'>
): boolean {
  if (!facts.href || facts.download) return false;
  let target: URL;
  let page: URL;
  try {
    target = new URL(facts.href);
    page = new URL(facts.pageUrl);
  } catch {
    return false;
  }
  if (target.protocol !== 'http:' && target.protocol !== 'https:') return false;
  if (target.username || target.password) return false;
  if (target.origin !== page.origin) return false;
  const sameDocument = target.pathname === page.pathname && target.search === page.search;
  return !sameDocument;
}

const PROTECTED_AUTOCOMPLETE =
  /(^|\s)(current-password|new-password|one-time-code|cc-[a-z-]+)(\s|$)/;

/** A field the agent never types into (BB-13 step 2). */
export function isProtectedInput(
  facts: Pick<ElementFacts, 'tag' | 'type' | 'autocomplete'>
): boolean {
  if (facts.tag === 'input' && facts.type === 'password') return true;
  return PROTECTED_AUTOCOMPLETE.test(facts.autocomplete);
}

export const PROTECTED_FIELD_REFUSAL =
  'This is a password, verification code or card field. The Biorouter Browser never types into one: ask the person to enter it in the panel themselves.';

/** A drop types the dragged text into a field, so a drag never starts or ends on one. */
export const PROTECTED_DRAG_REFUSAL =
  'That is a password, verification code or card field. The Biorouter Browser never drags into or out of one: ask the person to do it in the panel themselves.';

/**
 * Finds the one visible element a `browser_drag` locator names (AD-17), in the main document
 * and the open shadow roots under it, never inside a frame. Runs in the isolated world through
 * `Runtime.callFunctionOn` with the locator as arguments, so nothing the model wrote becomes
 * script. Returns the element, or a JSON string saying why there is not exactly one:
 * `{ problem, count, hidden, near, frames? }`, `near` naming up to five places the matches are
 * in and `frames`, when nothing visible matched, how many visible frames the page has, whose
 * content no locator reaches.
 *
 * - `selector`: `querySelectorAll` in each root.
 * - `text`: the smallest element whose whole text is the given text (case and spacing
 *   ignored); if none, the elements holding a piece of text that contains it. `scope`, a CSS
 *   selector, keeps only matches inside one of its visible elements.
 */
export const LOCATE_FUNCTION = `function (kind, value, scope) {
  const SKIP = /^(SCRIPT|STYLE|NOSCRIPT|TEMPLATE|HEAD|TITLE)$/;
  const norm = (s) => String(s || '').replace(/\\s+/g, ' ').trim().toLowerCase();
  const visible = (el) => {
    if (!el.isConnected) return false;
    if (typeof el.checkVisibility === 'function' && !el.checkVisibility({ visibilityProperty: true, checkVisibilityCSS: true })) return false;
    const r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0;
  };
  const roots = [];
  const addRoots = (root) => { roots.push(root); for (const el of root.querySelectorAll('*')) if (el.shadowRoot) addRoots(el.shadowRoot); };
  addRoots(document);
  const queryAll = (selector) => { const out = []; for (const root of roots) for (const el of root.querySelectorAll(selector)) out.push(el); return out; };
  const near = (els) => {
    const out = [];
    for (const el of els) {
      let at = el.parentElement;
      while (at && !at.id) at = at.parentElement;
      const name = at && at.id ? '#' + CSS.escape(at.id).slice(0, 40) : el.tagName.toLowerCase();
      if (!out.includes(name)) out.push(name);
      if (out.length >= 5) break;
    }
    return out;
  };
  const withFrames = (out) => { const count = queryAll('iframe, frame, object, embed').filter(visible).length; if (count) out.frames = count; return JSON.stringify(out); };
  const report = (problem, all, shown) => { const out = { problem, count: shown.length, hidden: all.length - shown.length, near: near(shown.length ? shown : []) }; return shown.length ? JSON.stringify(out) : withFrames(out); };
  if (kind === 'selector') {
    let all;
    try { all = queryAll(value); } catch (e) { return JSON.stringify({ problem: 'syntax', count: 0, hidden: 0, near: [] }); }
    const shown = all.filter(visible);
    return shown.length === 1 ? shown[0] : report(shown.length ? 'several' : 'none', all, shown);
  }
  let scopes = null;
  if (scope) {
    let all;
    try { all = queryAll(scope); } catch (e) { return JSON.stringify({ problem: 'scope-syntax', count: 0, hidden: 0, near: [] }); }
    scopes = new Set(all.filter(visible));
    if (scopes.size === 0) return withFrames({ problem: 'scope-none', count: 0, hidden: all.length, near: [] });
  }
  const inScope = (node) => {
    if (!scopes) return true;
    for (let at = node; at; at = at.parentNode || (at instanceof ShadowRoot ? at.host : null)) if (scopes.has(at)) return true;
    return false;
  };
  const needle = norm(value);
  const exact = new Set();
  const partial = new Set();
  for (const root of roots) {
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    for (let t = walker.nextNode(); t; t = walker.nextNode()) {
      const parent = t.parentElement;
      if (!parent || SKIP.test(parent.tagName) || !inScope(t)) continue;
      const text = norm(t.data);
      if (!text) continue;
      if (text.includes(needle)) partial.add(parent);
      if (needle.includes(text)) {
        for (let el = parent, depth = 0; el && depth < 8; el = el.parentElement, depth += 1) {
          const whole = norm(el.textContent);
          if (whole === needle) { exact.add(el); break; }
          if (whole.length > needle.length) break;
        }
      }
    }
  }
  const all = exact.size ? [...exact] : [...partial];
  const shown = all.filter(visible);
  if (shown.length === 1) return shown[0];
  if (shown.length === 0 && exact.size) {
    const loose = [...partial].filter(visible);
    if (loose.length === 1) return loose[0];
    if (loose.length > 1) return report('several', [...partial], loose);
  }
  return report(shown.length ? 'several' : 'none', all, shown);
}`;

/** A short label for an element a locator found: `[listitem] "Item 1"`. */
export const DESCRIBE_TARGET_FUNCTION = `function () {
  const el = this;
  const implicit = { li: 'listitem', a: el.hasAttribute('href') ? 'link' : 'a', button: 'button', img: 'img', tr: 'row', td: 'cell', th: 'columnheader', h1: 'heading', h2: 'heading', h3: 'heading', h4: 'heading', h5: 'heading', h6: 'heading', input: 'textbox', textarea: 'textbox', select: 'combobox', ul: 'list', ol: 'list', table: 'table', section: 'region', article: 'article' };
  const tag = el.tagName.toLowerCase();
  const role = (el.getAttribute('role') || implicit[tag] || tag).toLowerCase().slice(0, 40);
  const name = (el.getAttribute('aria-label') || el.getAttribute('alt') || el.textContent || '').replace(/\\s+/g, ' ').trim().slice(0, 80);
  return { role, name };
}`;

/** Inputs whose value is chosen, not typed: `browser_fill` sets them directly. */
export const SET_VALUE_INPUT_TYPES: ReadonlySet<string> = new Set([
  'date',
  'datetime-local',
  'month',
  'week',
  'time',
  'color',
  'range',
]);

/** Inputs `browser_fill` cannot fill. */
export const UNFILLABLE_INPUT_TYPES: ReadonlySet<string> = new Set([
  'checkbox',
  'radio',
  'file',
  'submit',
  'button',
  'reset',
  'image',
  'hidden',
]);
