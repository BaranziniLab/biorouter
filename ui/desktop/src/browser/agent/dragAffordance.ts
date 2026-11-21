/**
 * Which elements of a page a person would drag, and where they would drop them (AD-17), so the
 * snapshot gives them references even when their role is plain structure: a card on a board, a
 * row of a sortable list, an empty box that takes a drop.
 *
 * An element is a drag item when:
 *
 * - it says so: `draggable="true"`, `aria-grabbed`, an `aria-roledescription` of draggable or
 *   sortable, or the marks jQuery UI and react-beautiful-dnd put on their items
 *   (`ui-draggable`, `ui-sortable-handle`, `data-rbd-draggable-id` and the like);
 * - or its cursor is `move`, `grab` or `grabbing`. The cursor is inherited, so only the element
 *   that sets it counts, never the descendants that inherit it; and when that element is a
 *   container whose children share it (a `ul` with `cursor: move`), its children are the items.
 *
 * An element that is not an item is a drop zone when it says so: an inline `ondrop`,
 * `ondragover` or `ondragenter` handler (W3Schools' demo, and most hand-written pages), a
 * `dropzone` attribute, an `aria-dropeffect` other than none, or the marks libraries put on their
 * drop targets (`ui-droppable`, `data-rbd-droppable-id`, a `dropzone` or `droppable` class). A
 * zone often has no role and no words, and without a reference an empty zone inside a frame
 * could not be named at all: a selector looks only in the main document.
 *
 * At most `max` items and `zonesMax` zones per frame, in document order, so a board with
 * hundreds of cards cannot flood the snapshot. Pure: the driver feeds it the DOM it already reads
 * for the snapshot.
 */

/** Cursors that say "drag me", as `getComputedStyle` spells them. */
export const DRAG_CURSORS: readonly string[] = [
  'move',
  'grab',
  'grabbing',
  '-webkit-grab',
  '-webkit-grabbing',
];

export type DragDomNode = {
  backendNodeId: number;
  /** The parent element's backendNodeId; absent for the root or across a shadow boundary. */
  parent?: number;
  tag: string;
  attributes: Record<string, string>;
  /** Element children and text, in document order: text as strings, elements by id. */
  parts: Array<string | number>;
};

export type DragMark = {
  /** The item's own text, collapsed and cut, or empty. */
  text: string;
  /** For an item with no words of its own (a grip handle): the text of the row it sits in. */
  context?: string;
  /** A place to drop on rather than a thing to drag, shown as `[dropzone]`. */
  zone?: true;
};

export const DRAG_MARKS_MAX = 100;
export const DROP_MARKS_MAX = 50;
const TEXT_MAX = 80;
const CONTEXT_MAX = 40;

const LIBRARY_CLASSES = new Set([
  'ui-draggable',
  'ui-draggable-handle',
  'ui-sortable-handle',
  'draggable-item',
  'sortable-item',
]);
const LIBRARY_ATTRIBUTES = [
  'data-rbd-draggable-id',
  'data-rbd-drag-handle-draggable-id',
  'data-rfd-draggable-id',
  'data-rfd-drag-handle-draggable-id',
  'aria-grabbed',
];
const DROP_ATTRIBUTES = [
  'ondrop',
  'ondragover',
  'ondragenter',
  'dropzone',
  'data-rbd-droppable-id',
  'data-rfd-droppable-id',
];
const DROP_CLASSES = new Set(['ui-droppable', 'dropzone', 'drop-zone', 'droppable', 'drop-target']);
const LIST_TAGS = new Set(['ul', 'ol', 'tbody', 'thead', 'table', 'dl', 'menu']);
const LIST_ROLES = new Set([
  'list',
  'listbox',
  'grid',
  'tree',
  'tablist',
  'rowgroup',
  'table',
  'menu',
]);
/** Never an item: the page itself, and content that is not laid out. */
const NEVER_ITEMS = new Set(['html', 'body', 'head', 'script', 'style', 'template', 'noscript']);

function declaresDrag(node: DragDomNode): boolean {
  const a = node.attributes;
  if ((a.draggable ?? '').toLowerCase() === 'true') return true;
  if (/\b(draggable|sortable)\b/i.test(a['aria-roledescription'] ?? '')) return true;
  if (LIBRARY_ATTRIBUTES.some((name) => name in a)) return true;
  return (a.class ?? '').split(/\s+/).some((token) => LIBRARY_CLASSES.has(token));
}

function declaresDrop(node: DragDomNode): boolean {
  const a = node.attributes;
  if (DROP_ATTRIBUTES.some((name) => name in a)) return true;
  const effect = (a['aria-dropeffect'] ?? '').trim().toLowerCase();
  if (effect && effect !== 'none') return true;
  return (a.class ?? '').split(/\s+/).some((token) => DROP_CLASSES.has(token.toLowerCase()));
}

export function collapse(text: string, max: number): string {
  const flat = text.replace(/\s+/g, ' ').trim();
  return flat.length > max ? `${flat.slice(0, max - 1)}…` : flat;
}

/** An element's text, in document order, to a few levels and a few dozen characters. */
export function textOf(nodes: Map<number, DragDomNode>, id: number, max = TEXT_MAX): string {
  let out = '';
  const walk = (current: number, depth: number) => {
    const node = nodes.get(current);
    if (!node || depth > 6 || out.length > max * 2) return;
    if (NEVER_ITEMS.has(node.tag)) return;
    for (const part of node.parts) {
      if (typeof part === 'string') out += ` ${part}`;
      else walk(part, depth + 1);
    }
  };
  walk(id, 0);
  return collapse(out, max);
}

/** Words, as opposed to a grip glyph such as ⋮⋮ or ☰. */
function hasWords(text: string): boolean {
  return /[\p{L}\p{N}]/u.test(text);
}

/**
 * A container of drag items rather than an item: a list (by tag or role) with two or more
 * element children, or three or more children that share a tag and class and make up most of
 * them. A card with a heading and two paragraphs stays an item.
 */
function itemsOfContainer(nodes: Map<number, DragDomNode>, node: DragDomNode): number[] | null {
  if (node.parts.some((part) => typeof part === 'string' && part.trim())) return null;
  const children = node.parts
    .filter((part): part is number => typeof part === 'number')
    .map((id) => nodes.get(id))
    .filter((child): child is DragDomNode => !!child && !NEVER_ITEMS.has(child.tag));
  if (children.length < 2) return null;
  const list =
    LIST_TAGS.has(node.tag) || LIST_ROLES.has((node.attributes.role ?? '').toLowerCase());
  const signature = (child: DragDomNode) =>
    list
      ? child.tag
      : `${child.tag}.${(child.attributes.class ?? '').split(/\s+/).sort().join('.')}`;
  const counts = new Map<string, number>();
  for (const child of children)
    counts.set(signature(child), (counts.get(signature(child)) ?? 0) + 1);
  const [top, count] = [...counts.entries()].sort((a, b) => b[1] - a[1])[0];
  if (list ? count < 2 : count < 3 || count / children.length < 0.8) return null;
  return children.filter((child) => signature(child) === top).map((child) => child.backendNodeId);
}

/** For a mark with no words of its own: the text of the nearest ancestor with words. */
function contextOf(nodes: Map<number, DragDomNode>, id: number): string | undefined {
  let at = nodes.get(id)?.parent;
  for (let depth = 0; at !== undefined && depth < 3; depth += 1) {
    const around = textOf(nodes, at, CONTEXT_MAX);
    if (hasWords(around)) return around;
    at = nodes.get(at)?.parent;
  }
  return undefined;
}

/**
 * Mark the drag items and drop zones of one frame. `order` is every element in document order;
 * `cursor` the elements whose computed cursor is one of `DRAG_CURSORS`.
 */
export function markDragAffordances(
  nodes: Map<number, DragDomNode>,
  order: readonly number[],
  cursor: ReadonlySet<number>,
  max = DRAG_MARKS_MAX,
  zonesMax = DROP_MARKS_MAX
): Map<number, DragMark> {
  const items = new Set<number>();
  const position = new Map(order.map((id, index) => [id, index]));
  for (const id of order) {
    const node = nodes.get(id);
    if (!node || NEVER_ITEMS.has(node.tag)) continue;
    if (declaresDrag(node)) {
      items.add(id);
      continue;
    }
    if (!cursor.has(id)) continue;
    // Inherited from an ancestor that sets it: that ancestor (or its children) is the item.
    if (node.parent !== undefined && cursor.has(node.parent)) continue;
    const children = itemsOfContainer(nodes, node);
    if (children) {
      for (const child of children) if (cursor.has(child)) items.add(child);
    } else {
      items.add(id);
    }
  }
  const marks = new Map<number, DragMark>();
  const sorted = [...items].sort((a, b) => (position.get(a) ?? 0) - (position.get(b) ?? 0));
  for (const id of sorted.slice(0, max)) {
    const text = textOf(nodes, id);
    const mark: DragMark = { text: hasWords(text) ? text : '' };
    if (!mark.text) {
      // A grip handle: name it by its row, the nearest ancestor with words, a few levels up.
      const context = contextOf(nodes, id);
      if (context) mark.context = context;
    }
    marks.set(id, mark);
  }
  // Drop zones: an item is already reachable, so only elements that are not items.
  let zones = 0;
  for (const id of order) {
    if (zones >= zonesMax) break;
    const node = nodes.get(id);
    if (!node || NEVER_ITEMS.has(node.tag) || items.has(id) || !declaresDrop(node)) continue;
    const text = textOf(nodes, id);
    const mark: DragMark = { text: hasWords(text) ? text : '', zone: true };
    if (!mark.text) {
      const context = contextOf(nodes, id);
      if (context) mark.context = context;
    }
    marks.set(id, mark);
    zones += 1;
  }
  return marks;
}
