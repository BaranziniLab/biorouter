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

/** The point an action aims at: the centre of the first quad, inside the viewport. */
export function clickPoint(
  quads: number[][],
  viewport: { width: number; height: number }
): { x: number; y: number } | null {
  for (const quad of quads) {
    if (quad.length < 8) continue;
    const xs = [quad[0], quad[2], quad[4], quad[6]];
    const ys = [quad[1], quad[3], quad[5], quad[7]];
    const minX = Math.max(0, Math.min(...xs));
    const maxX = Math.min(viewport.width, Math.max(...xs));
    const minY = Math.max(0, Math.min(...ys));
    const maxY = Math.min(viewport.height, Math.max(...ys));
    if (maxX - minX < 1 || maxY - minY < 1) continue;
    return { x: (minX + maxX) / 2, y: (minY + maxY) / 2 };
  }
  return null;
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
 * A click the observe grant covers (BB-17): following a plain link to an http(s) address that
 * is another page. A link that downloads, a fragment link to the same page (usually a script's
 * button in disguise), and everything that is not a link need the act grant.
 */
export function isPlainLinkClick(
  facts: Pick<ElementFacts, 'href' | 'download' | 'pageUrl'>
): boolean {
  if (!facts.href || facts.download) return false;
  let target: URL;
  try {
    target = new URL(facts.href);
  } catch {
    return false;
  }
  if (target.protocol !== 'http:' && target.protocol !== 'https:') return false;
  if (target.username || target.password) return false;
  try {
    const page = new URL(facts.pageUrl);
    const sameDocument =
      target.origin === page.origin &&
      target.pathname === page.pathname &&
      target.search === page.search;
    if (sameDocument) return false;
  } catch {
    // An unparseable page address: judge the link alone.
  }
  return true;
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
