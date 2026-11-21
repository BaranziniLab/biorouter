import { describe, expect, it } from 'vitest';
import { markDragAffordances, textOf, type DragDomNode } from './dragAffordance';

/**
 * AD-17: which elements the snapshot gives drag references to. The trees are built the way the
 * driver builds them from `DOM.getDocument`: elements by backendNodeId, their text and children
 * in document order, and the set of elements whose computed cursor is a drag cursor (which
 * includes every descendant that inherits it).
 */

type Spec = {
  tag: string;
  attributes?: Record<string, string>;
  children?: Array<Spec | string>;
  cursor?: boolean;
};

function tree(root: Spec) {
  const nodes = new Map<number, DragDomNode>();
  const order: number[] = [];
  const cursor = new Set<number>();
  const ids = new Map<string, number>();
  let next = 1;
  const add = (spec: Spec, parent?: number, inherited = false): number => {
    const id = next++;
    order.push(id);
    const node: DragDomNode = {
      backendNodeId: id,
      parent,
      tag: spec.tag,
      attributes: spec.attributes ?? {},
      parts: [],
    };
    nodes.set(id, node);
    const hasCursor = spec.cursor ?? inherited;
    if (hasCursor) cursor.add(id);
    if (spec.attributes?.id) ids.set(spec.attributes.id, id);
    for (const child of spec.children ?? []) {
      node.parts.push(typeof child === 'string' ? child : add(child, id, hasCursor));
    }
    return id;
  };
  add(root);
  const marks = () => markDragAffordances(nodes, order, cursor);
  const textsOf = (found = marks()) => [...found.keys()].map((id) => textOf(nodes, id));
  return { nodes, ids, marks, textsOf };
}

const li = (text: string, extra: Partial<Spec> = {}): Spec => ({
  tag: 'li',
  children: [text],
  ...extra,
});

describe('markDragAffordances', () => {
  it('takes the items of a list whose cursor is inherited, not the list or their insides', () => {
    const page = tree({
      tag: 'body',
      children: [
        {
          tag: 'ul',
          cursor: true,
          children: [
            li('Item 1', { children: [{ tag: 'span', children: ['Item 1'] }] }),
            li('Item 2'),
            li('Item 3'),
          ],
        },
      ],
    });
    expect(page.textsOf()).toEqual(['Item 1', 'Item 2', 'Item 3']);
  });

  it('keeps a card that sets its own cursor as one item, whatever is inside it', () => {
    const card = (title: string): Spec => ({
      tag: 'div',
      cursor: true,
      attributes: { class: 'card' },
      children: [
        { tag: 'h3', children: [title] },
        { tag: 'p', children: ['First line'] },
        { tag: 'p', children: ['Second line'] },
      ],
    });
    const page = tree({ tag: 'body', children: [card('Alpha'), card('Beta')] });
    expect(page.textsOf()).toEqual(['Alpha First line Second line', 'Beta First line Second line']);
  });

  it('takes what declares a drag: draggable, aria and the marks of drag libraries', () => {
    const page = tree({
      tag: 'body',
      children: [
        { tag: 'div', attributes: { draggable: 'true' }, children: ['Native'] },
        { tag: 'div', attributes: { draggable: 'false' }, children: ['Not this'] },
        {
          tag: 'li',
          attributes: { class: 'ui-state-default ui-sortable-handle' },
          children: ['jQuery'],
        },
        { tag: 'div', attributes: { 'data-rbd-draggable-id': 'x' }, children: ['Beautiful'] },
        { tag: 'div', attributes: { 'aria-roledescription': 'sortable' }, children: ['Kit'] },
        { tag: 'img', attributes: { alt: 'A picture' } },
      ],
    });
    expect(page.textsOf()).toEqual(['Native', 'jQuery', 'Beautiful', 'Kit']);
  });

  it('names a grip with no words by its row', () => {
    const row = (task: string): Spec => ({
      tag: 'div',
      children: [
        { tag: 'span', cursor: true, children: ['⋮⋮'] },
        { tag: 'span', children: [task] },
      ],
    });
    const page = tree({ tag: 'body', children: [row('Alpha task'), row('Beta task')] });
    const marks = [...page.marks().values()];
    expect(marks).toEqual([
      { text: '', context: '⋮⋮ Alpha task' },
      { text: '', context: '⋮⋮ Beta task' },
    ]);
  });

  it('marks a place that declares it takes a drop, named by its words or its row', () => {
    const page = tree({
      tag: 'body',
      children: [
        { tag: 'p', children: ['Drag the image into the rectangle:'] },
        // W3Schools' demo: an empty box whose only marks are its inline handlers.
        { tag: 'div', attributes: { id: 'div1', ondrop: 'drop(event)', ondragover: 'a(event)' } },
        { tag: 'div', attributes: { class: 'col dropzone' }, children: ['Done'] },
        { tag: 'div', attributes: { 'data-rbd-droppable-id': 'later' }, children: ['Later'] },
        { tag: 'div', attributes: { class: 'ui-droppable' }, children: ['Bin'] },
        { tag: 'div', attributes: { 'aria-dropeffect': 'move' }, children: ['Aria'] },
        { tag: 'div', attributes: { 'aria-dropeffect': 'none' }, children: ['Not this'] },
        { tag: 'section', children: [{ tag: 'div', attributes: { ondragenter: 'x()' } }] },
        // Both an item and a zone: it stays an item.
        { tag: 'div', attributes: { draggable: 'true', ondragover: 'x()' }, children: ['Card'] },
        { tag: 'img', attributes: { draggable: 'true', id: 'drag1' } },
      ],
    });
    const marks = page.marks();
    const zone = (id: string) => marks.get(page.ids.get(id) ?? -1);
    expect(zone('div1')).toEqual({ text: '', zone: true });
    expect([...marks.values()].filter((mark) => mark.zone).map((mark) => mark.text)).toEqual([
      '',
      'Done',
      'Later',
      'Bin',
      'Aria',
      '',
    ]);
    expect(page.textsOf().filter((text) => text === 'Card')).toHaveLength(1);
    expect([...marks.values()].find((mark) => mark.text === 'Card')?.zone).toBeUndefined();
    expect(page.textsOf()).not.toContain('Not this');
  });

  it('never takes the page itself as a drop zone, and caps zones apart from items', () => {
    const whole = tree({ tag: 'body', attributes: { ondragover: 'x()' }, children: ['Hi'] });
    expect(whole.marks().size).toBe(0);
    const many = tree({
      tag: 'body',
      children: [
        { tag: 'div', attributes: { draggable: 'true' }, children: ['Item'] },
        ...Array.from({ length: 80 }, (_, i) => ({
          tag: 'div',
          attributes: { ondrop: 'x()' },
          children: [`Z${i + 1}`],
        })),
      ],
    });
    const marks = [...many.marks().values()];
    expect(marks.filter((mark) => mark.zone)).toHaveLength(50);
    expect(marks.filter((mark) => !mark.zone).map((mark) => mark.text)).toEqual(['Item']);
  });

  it('never marks the page itself, and caps a flood in document order', () => {
    const whole = tree({ tag: 'body', cursor: true, children: [{ tag: 'p', children: ['Hi'] }] });
    expect(whole.marks().size).toBe(0);
    const tiles = tree({
      tag: 'body',
      children: [
        { tag: 'div', attributes: { draggable: 'true' }, children: ['First'] },
        {
          tag: 'div',
          children: Array.from({ length: 300 }, (_, i) => ({
            tag: 'div',
            cursor: true,
            children: [`T${i + 1}`],
          })),
        },
      ],
    });
    const texts = tiles.textsOf();
    expect(texts).toHaveLength(100);
    expect(texts[0]).toBe('First');
    expect(texts[1]).toBe('T1');
  });
});
