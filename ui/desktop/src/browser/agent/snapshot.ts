/**
 * A page as a compact accessibility tree with `@e` references (BB-12).
 *
 * Input is what CDP gives main: `Accessibility.getFullAXTree` per frame, and a few DOM facts per
 * node from `DOM.getDocument({ pierce: true })` (tag, attributes, the frame an iframe hosts, and
 * short hints from its subtree). Output is one line per kept node:
 *
 *   @e1 [searchbox] "Search" value="multiple sclerosis"
 *   @e6 [button] icon:"filter"
 *   @e7 [textbox] "Password" value="(hidden)"
 *     frame "Cookie settings" (https://consent.example)
 *     @e8 [button] "Accept all"
 *
 * By default it keeps interactive and landmark nodes and drops empty structure (agent-browser's
 * `-i -c`). A control with an empty or duplicated name gets a hint from the DOM, in this order:
 * `icon:`, `img:`, `testid:`, `name:`, `id:`, `in:` (the text of its row). Values of password,
 * verification code and card fields are never shown.
 *
 * Pure: no Electron, so the Vitest suite feeds it recorded trees.
 */

export type AXValue = { type?: string; value?: unknown };
export type AXProperty = { name: string; value?: AXValue };
export type AXNode = {
  nodeId: string;
  ignored?: boolean;
  role?: AXValue;
  name?: AXValue;
  value?: AXValue;
  properties?: AXProperty[];
  childIds?: string[];
  parentId?: string;
  backendDOMNodeId?: number;
};

export type DomFacts = {
  /** Lower case tag name. */
  tag: string;
  attributes: Record<string, string>;
  /** For an iframe element: the frame it hosts. */
  frameId?: string;
  /** The alt text of an image inside the element. */
  imgAlt?: string;
  /** A short name for an icon inside the element: an svg title or an icon class stem. */
  iconHint?: string;
  /** For a select element: its option labels. */
  options?: string[];
};

export type SnapshotFrame = {
  frameKey: string;
  url: string;
  title: string;
  nodes: AXNode[];
  dom: Map<number, DomFacts>;
};

export type SnapshotOptions = {
  interactiveOnly: boolean;
  maxDepth?: number;
  includeUrls: boolean;
  maxChars: number;
  scope?: { frameKey: string; backendNodeId: number };
};

export type SnapshotRefInput = {
  backendNodeId: number;
  frameKey: string;
  role: string;
  name: string;
  unique: boolean;
};

export type SnapshotResult = {
  text: string;
  /** Lines that were not shown because of max_chars. */
  omittedLines: number;
  truncated: boolean;
  refCount: number;
  /** The scope did not resolve to a node in the tree. */
  scopeMissing?: boolean;
};

/** Compared in lower case: CDP spells ARIA roles in lower case and Chromium's own in CamelCase. */
export const INTERACTIVE_ROLES: ReadonlySet<string> = new Set([
  'button',
  'link',
  'textbox',
  'searchbox',
  'combobox',
  'checkbox',
  'radio',
  'switch',
  'slider',
  'spinbutton',
  'menuitem',
  'menuitemcheckbox',
  'menuitemradio',
  'option',
  'tab',
  'treeitem',
  'listbox',
  'textfield',
  'popupbutton',
  'togglebutton',
  'colorwell',
  'date',
  'datetime',
  'inputtime',
]);

/**
 * Controls whose children are the browser's own parts (the month, day and year fields of a date
 * input): the control gets a line and a reference, its parts do not.
 */
const LEAF_CONTROL_ROLES: ReadonlySet<string> = new Set([
  'date',
  'datetime',
  'inputtime',
  'colorwell',
]);

/** Kept in the default snapshot for orientation, beside the controls. */
export const LANDMARK_ROLES: ReadonlySet<string> = new Set([
  'heading',
  'dialog',
  'alertdialog',
  'alert',
  'main',
  'navigation',
  'form',
  'search',
  'banner',
  'contentinfo',
  'complementary',
  'region',
]);

/** Structure that never gets a line of its own; its children are walked. */
const SKIPPED_ROLES: ReadonlySet<string> = new Set([
  'generic',
  'none',
  'presentation',
  'InlineTextBox',
  'LineBreak',
  'ListMarker',
  'RootWebArea',
  'WebArea',
  'Ignored',
  'IgnoredRole',
  'LayoutTable',
  'LayoutTableRow',
  'LayoutTableCell',
]);

/** A collapsed select's popup: its options are listed on the select line instead. */
const HIDDEN_SUBTREE_ROLES: ReadonlySet<string> = new Set(['MenuListPopup']);

/** Containers whose name tells two identical controls apart (`in:`). */
const ROW_ROLES: ReadonlySet<string> = new Set([
  'row',
  'listitem',
  'article',
  'gridcell',
  'cell',
  'group',
  'region',
  'form',
  'dialog',
  'treeitem',
]);

const PROTECTED_AUTOCOMPLETE = new Set([
  'current-password',
  'new-password',
  'one-time-code',
  'cc-number',
  'cc-csc',
  'cc-exp',
  'cc-exp-month',
  'cc-exp-year',
  'cc-name',
  'cc-given-name',
  'cc-family-name',
  'cc-type',
]);

const NAME_MAX = 200;
const VALUE_MAX = 200;
const HINT_MAX = 60;
const URL_MAX = 300;

/**
 * Whether a field holds a credential the agent may never type into or read (BB-13): a password
 * input, or a field whose `autocomplete` names a password, a one time code or a card value.
 */
export function isProtectedField(facts: Pick<DomFacts, 'tag' | 'attributes'> | undefined): boolean {
  if (!facts) return false;
  const type = (facts.attributes.type ?? '').toLowerCase();
  if (facts.tag === 'input' && type === 'password') return true;
  const tokens = (facts.attributes.autocomplete ?? '').toLowerCase().split(/\s+/).filter(Boolean);
  return tokens.some((token) => PROTECTED_AUTOCOMPLETE.has(token));
}

function str(value: AXValue | undefined): string {
  const raw = value?.value;
  if (raw === undefined || raw === null) return '';
  return String(raw);
}

/** One line of page-chosen text: whitespace collapsed, quotes escaped, length capped. */
export function clean(text: string, max: number): string {
  const collapsed = text.replace(/\s+/g, ' ').trim();
  const cut = collapsed.length > max ? `${collapsed.slice(0, max - 1)}…` : collapsed;
  return cut.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
}

function property(node: AXNode, name: string): unknown {
  return node.properties?.find((prop) => prop.name === name)?.value?.value;
}

function roleOf(node: AXNode): string {
  return str(node.role) || 'unknown';
}

/** The DOM hint for a control whose name is empty or shared (BB-12). */
function hintFor(
  facts: DomFacts | undefined,
  rowName: string | null
): { kind: string; value: string } | null {
  if (facts?.iconHint) return { kind: 'icon', value: facts.iconHint };
  if (facts?.attributes.title) return { kind: 'icon', value: facts.attributes.title };
  if (facts?.imgAlt) return { kind: 'img', value: facts.imgAlt };
  const testid = facts?.attributes['data-testid'] ?? facts?.attributes['data-test-id'];
  if (testid) return { kind: 'testid', value: testid };
  if (facts?.attributes.name) return { kind: 'name', value: facts.attributes.name };
  if (facts?.attributes.id) return { kind: 'id', value: facts.attributes.id };
  if (rowName) return { kind: 'in', value: rowName };
  return null;
}

type Item =
  | {
      kind: 'node';
      depth: number;
      frameKey: string;
      node: AXNode;
      role: string;
      name: string;
      facts: DomFacts | undefined;
      rowName: string | null;
      interactive: boolean;
    }
  | { kind: 'text'; depth: number; text: string }
  | { kind: 'frame'; depth: number; title: string; url: string };

/**
 * Format `frames` (keyed by frame key; `main` is the top document) as snapshot lines, assigning
 * references through `assign`.
 */
export function buildSnapshot(
  frames: Map<string, SnapshotFrame>,
  options: SnapshotOptions,
  assign: (target: SnapshotRefInput) => string
): SnapshotResult {
  const items: Item[] = [];
  const visitedFrames = new Set<string>();
  let scopeFound = !options.scope;

  const walkFrame = (frame: SnapshotFrame, depth: number, startBackendId?: number) => {
    if (visitedFrames.has(frame.frameKey)) return;
    visitedFrames.add(frame.frameKey);
    const byId = new Map(frame.nodes.map((node) => [node.nodeId, node]));
    let start: AXNode | undefined;
    if (startBackendId !== undefined) {
      start = frame.nodes.find((node) => node.backendDOMNodeId === startBackendId);
      if (start) scopeFound = true;
    } else {
      start = frame.nodes.find((node) => !node.parentId) ?? frame.nodes[0];
    }
    if (!start) return;
    const rowNames: Array<string | null> = [];

    const visit = (node: AXNode, level: number) => {
      if (options.maxDepth !== undefined && level > options.maxDepth) return;
      const role = roleOf(node);
      if (HIDDEN_SUBTREE_ROLES.has(role)) return;
      const facts =
        node.backendDOMNodeId !== undefined ? frame.dom.get(node.backendDOMNodeId) : undefined;
      const name = str(node.name).replace(/\s+/g, ' ').trim();
      const ignored = node.ignored === true;
      const interactive = !ignored && INTERACTIVE_ROLES.has(role.toLowerCase());
      const landmark =
        !ignored &&
        LANDMARK_ROLES.has(role) &&
        // An unnamed region is just a box.
        (role !== 'region' || name !== '');
      let printed = false;

      if (!ignored && facts?.frameId && frames.has(facts.frameId)) {
        const child = frames.get(facts.frameId)!;
        items.push({
          kind: 'frame',
          depth: level,
          title: facts.attributes.title || child.title || name,
          url: child.url,
        });
        walkFrame(child, level + 1);
        return;
      }

      if (interactive || landmark) {
        items.push({
          kind: 'node',
          depth: level,
          frameKey: frame.frameKey,
          node,
          role,
          name,
          facts,
          rowName: [...rowNames].reverse().find((row) => row) ?? null,
          interactive,
        });
        printed = true;
      } else if (!options.interactiveOnly && !ignored && !SKIPPED_ROLES.has(role)) {
        if (role === 'StaticText') {
          const text = name;
          // Text already carried by the parent's name is noise.
          const parent = node.parentId ? byId.get(node.parentId) : undefined;
          const parentName = parent ? str(parent.name).replace(/\s+/g, ' ').trim() : '';
          if (text && text !== parentName) items.push({ kind: 'text', depth: level, text });
        } else if (name || node.backendDOMNodeId !== undefined) {
          items.push({
            kind: 'node',
            depth: level,
            frameKey: frame.frameKey,
            node,
            role,
            name,
            facts,
            rowName: null,
            interactive: false,
          });
          printed = true;
        }
      }

      if (interactive && LEAF_CONTROL_ROLES.has(role.toLowerCase())) return;
      const isRow = ROW_ROLES.has(role) && name !== '';
      rowNames.push(isRow ? clean(name, 40) : null);
      for (const childId of node.childIds ?? []) {
        const child = byId.get(childId);
        if (child) visit(child, printed ? level + 1 : level);
      }
      rowNames.pop();
    };
    visit(start, depth);
  };

  const main = options.scope ? frames.get(options.scope.frameKey) : frames.get('main');
  if (main) walkFrame(main, 0, options.scope?.backendNodeId);

  // Uniqueness of (role, name) among the snapshot's references.
  const counts = new Map<string, number>();
  for (const item of items) {
    if (item.kind !== 'node') continue;
    const key = `${item.role}\u0000${item.name}`;
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }

  const lines: string[] = [];
  let used = 0;
  let omitted = 0;
  let refCount = 0;
  for (const item of items) {
    let line: string;
    const indent = '  '.repeat(item.depth);
    if (item.kind === 'text') {
      line = `${indent}- "${clean(item.text, NAME_MAX)}"`;
    } else if (item.kind === 'frame') {
      line = `${indent}frame "${clean(item.title, NAME_MAX)}" (${clean(item.url, URL_MAX)})`;
    } else {
      const unique = (counts.get(`${item.role}\u0000${item.name}`) ?? 0) <= 1;
      line = `${indent}${formatNode(item, unique, options)}`;
    }
    if (used + line.length + 1 > options.maxChars) {
      omitted += 1;
      continue;
    }
    if (item.kind === 'node' && item.node.backendDOMNodeId !== undefined) {
      const unique = (counts.get(`${item.role}\u0000${item.name}`) ?? 0) <= 1;
      const ref = assign({
        backendNodeId: item.node.backendDOMNodeId,
        frameKey: item.frameKey,
        role: item.role,
        name: item.name,
        unique,
      });
      line = line.replace('\u0001REF\u0001', ref);
      refCount += 1;
    } else {
      line = line.replace('\u0001REF\u0001 ', '');
    }
    lines.push(line);
    used += line.length + 1;
  }
  if (omitted > 0) {
    lines.push(`(${omitted} more lines. Pass scope, max_depth or max_chars to see more.)`);
  }
  if (lines.length === 0 && scopeFound) {
    lines.push(
      options.interactiveOnly
        ? '(No interactive elements. Pass interactive_only: false, or use browser_read for the text.)'
        : '(The page is empty.)'
    );
  }
  return {
    text: lines.join('\n'),
    omittedLines: omitted,
    truncated: omitted > 0,
    refCount,
    scopeMissing: !scopeFound,
  };
}

function formatNode(
  item: Extract<Item, { kind: 'node' }>,
  unique: boolean,
  options: SnapshotOptions
): string {
  const parts = ['\u0001REF\u0001', `[${item.role}]`];
  const name = clean(item.name, NAME_MAX);
  if (name) parts.push(`"${name}"`);
  if (item.interactive && (!name || !unique)) {
    const hint = hintFor(item.facts, item.rowName);
    if (hint) parts.push(`${hint.kind}:"${clean(hint.value, HINT_MAX)}"`);
  }
  const node = item.node;
  const protectedField = isProtectedField(item.facts);
  const valueRoles = new Set([
    'textbox',
    'searchbox',
    'combobox',
    'spinbutton',
    'slider',
    'textfield',
    'date',
    'datetime',
    'inputtime',
  ]);
  if (valueRoles.has(item.role.toLowerCase())) {
    const value = str(node.value);
    if (protectedField) parts.push('value="(hidden)"');
    else if (value) parts.push(`value="${clean(value, VALUE_MAX)}"`);
  }
  if (item.facts?.tag === 'select' && item.facts.options?.length) {
    const shown = item.facts.options.slice(0, 20).map((option) => clean(option, 40));
    const more = item.facts.options.length > 20 ? `|+${item.facts.options.length - 20}` : '';
    parts.push(`options="${shown.join('|')}${more}"`);
  }
  const level = property(node, 'level');
  if (item.role === 'heading' && typeof level === 'number') parts.push(`[level=${level}]`);
  const checked = property(node, 'checked');
  if (checked === 'true' || checked === true) parts.push('[checked]');
  else if (checked === 'mixed') parts.push('[checked=mixed]');
  const pressed = property(node, 'pressed');
  if (pressed === 'true' || pressed === true) parts.push('[pressed]');
  const expanded = property(node, 'expanded');
  if (expanded === true) parts.push('[expanded]');
  else if (expanded === false) parts.push('[collapsed]');
  if (property(node, 'selected') === true) parts.push('[selected]');
  if (property(node, 'disabled') === true) parts.push('[disabled]');
  if (property(node, 'required') === true) parts.push('[required]');
  if (property(node, 'readonly') === true) parts.push('[readonly]');
  if (property(node, 'focused') === true) parts.push('[focused]');
  if (options.includeUrls && item.role === 'link') {
    const href = item.facts?.attributes.href ?? (property(node, 'url') as string | undefined);
    if (href) parts.push(`href="${clean(href, URL_MAX)}"`);
  }
  return parts.join(' ');
}
