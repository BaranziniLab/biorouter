import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import {
  chooseFnComposer,
  focusedLooseField,
  isLiveField,
  modalScope,
  visibleComposer,
  type FieldCandidate,
} from './fieldTarget';

/**
 * jsdom has no `checkVisibility`; this one answers as Chromium does for what these tests use: an
 * element or ancestor with `display: none`, or with `visibility: hidden` when asked about it.
 */
function seen(el: Element, options?: { visibilityProperty?: boolean }): boolean {
  for (let node: Element | null = el; node; node = node.parentElement) {
    const style = (node as HTMLElement).style;
    if (style?.display === 'none') return false;
    if (options?.visibilityProperty && style?.visibility === 'hidden') return false;
  }
  return true;
}

function installCheckVisibility(): () => void {
  const proto = Element.prototype as Element & { checkVisibility?: unknown };
  const had = Object.getOwnPropertyDescriptor(proto, 'checkVisibility');
  Object.defineProperty(proto, 'checkVisibility', {
    configurable: true,
    value(this: Element, options?: { visibilityProperty?: boolean }) {
      return seen(this, options);
    },
  });
  return () => {
    if (had) Object.defineProperty(proto, 'checkVisibility', had);
    else delete (proto as { checkVisibility?: unknown }).checkVisibility;
  };
}

let restoreVisibility: () => void;
beforeEach(() => {
  document.body.innerHTML = '';
  restoreVisibility = installCheckVisibility();
});
afterEach(() => {
  restoreVisibility();
  document.body.innerHTML = '';
});

function el(
  tag: 'textarea' | 'input' | 'div' | 'button' | 'p',
  parent: Element = document.body,
  attrs: Record<string, string> = {}
): HTMLElement {
  const node = document.createElement(tag);
  for (const [name, value] of Object.entries(attrs)) node.setAttribute(name, value);
  parent.append(node);
  return node;
}

const candidate = (key: string, field: () => HTMLElement | null): FieldCandidate => ({
  key,
  field: field as FieldCandidate['field'],
});

describe('a live field', () => {
  it('is in the document, outside inert, visible, writable, and a text kind', () => {
    expect(isLiveField(null)).toBe(false);
    expect(isLiveField(undefined)).toBe(false);
    expect(isLiveField(document.createElement('textarea'))).toBe(false);
    expect(isLiveField(el('textarea'))).toBe(true);
    expect(isLiveField(el('input'))).toBe(true);
    expect(isLiveField(el('input', document.body, { type: 'search' }))).toBe(true);
    expect(isLiveField(el('div', document.body, { contenteditable: 'true' }))).toBe(true);
    expect(isLiveField(el('div'))).toBe(false);
  });

  it('is never a password, a checkbox or a number, nor a disabled or read only control', () => {
    expect(isLiveField(el('input', document.body, { type: 'password' }))).toBe(false);
    expect(isLiveField(el('input', document.body, { type: 'checkbox' }))).toBe(false);
    expect(isLiveField(el('input', document.body, { type: 'number' }))).toBe(false);
    expect(isLiveField(el('textarea', document.body, { disabled: '' }))).toBe(false);
    expect(isLiveField(el('input', document.body, { readonly: '' }))).toBe(false);
    expect(isLiveField(el('div', document.body, { contenteditable: 'false' }))).toBe(false);
  });

  it('is not inside inert, a hidden box, or the terminal', () => {
    const inert = el('div', document.body, { inert: '' });
    expect(isLiveField(el('textarea', inert))).toBe(false);
    const hidden = el('div');
    hidden.style.visibility = 'hidden';
    expect(isLiveField(el('textarea', hidden))).toBe(false);
    const gone = el('div');
    gone.style.display = 'none';
    expect(isLiveField(el('textarea', gone))).toBe(false);
    const terminal = el('div', document.body, { class: 'xterm' });
    expect(isLiveField(el('textarea', terminal, { class: 'xterm-helper-textarea' }))).toBe(false);
  });
});

describe('which composer an fn dictation goes into', () => {
  it('never one whose field is null, disconnected, inert or hidden', () => {
    const detached = document.createElement('textarea');
    const inert = el('textarea', el('div', document.body, { inert: '' }));
    const hiddenBox = el('div');
    hiddenBox.style.visibility = 'hidden';
    const hidden = el('textarea', hiddenBox);
    for (const field of [null, detached, inert, hidden]) {
      expect(
        chooseFnComposer({ candidates: [candidate('a', () => field)], lastFocused: 'a' })
      ).toBeNull();
    }
  });

  it('the composer, after a message editor that had focus closed and its field went away', () => {
    const composer = el('textarea');
    const editor = el('textarea');
    let editorField: HTMLElement | null = editor;
    const candidates = [
      candidate('tab:a', () => composer),
      candidate('editor:m1', () => editorField),
    ];
    editor.focus();
    expect(chooseFnComposer({ candidates, lastFocused: 'editor:m1' })).toBe('editor:m1');
    // The editor closes: its textarea unmounts and focus goes to the Edit button.
    editor.remove();
    editorField = null;
    el('button').focus();
    expect(chooseFnComposer({ candidates, lastFocused: 'editor:m1' })).toBe('tab:a');
  });

  it('three message editors and the composer, with focus on the page, give the composer', () => {
    const composer = el('textarea');
    // Closed editors have no field.
    const closed = [1, 2, 3].map((n) => candidate(`editor:m${n}`, () => null));
    expect(
      chooseFnComposer({
        candidates: [...closed, candidate('tab:a', () => composer)],
        lastFocused: 'editor:m2',
      })
    ).toBe('tab:a');

    // Open ones in the active pane: the chat's own composer still comes first.
    document.body.innerHTML = '';
    const pane = el('div', document.body, { 'data-active-group': 'true' });
    const open = [1, 2, 3].map((n) => {
      const field = el('textarea', pane);
      return candidate(`editor:m${n}`, () => field);
    });
    const chat = el('textarea', pane);
    (document.activeElement as HTMLElement | null)?.blur();
    expect(
      chooseFnComposer({ candidates: [...open, candidate('tab:a', () => chat)], lastFocused: null })
    ).toBe('tab:a');
  });

  it('the focused composer first, then the active pane’s, then the one focused last, then the only one', () => {
    const left = el('div', document.body, { 'data-active-group': 'false' });
    const right = el('div', document.body, { 'data-active-group': 'true' });
    const a = el('textarea', left);
    const b = el('textarea', right);
    const candidates = [candidate('a', () => a), candidate('b', () => b)];
    a.focus();
    expect(chooseFnComposer({ candidates, lastFocused: null })).toBe('a');
    a.blur();
    expect(chooseFnComposer({ candidates, lastFocused: null })).toBe('b');
    // In the active pane, the composer focused last there wins over the chat's own.
    right.setAttribute('data-active-group', 'false');
    expect(chooseFnComposer({ candidates, lastFocused: 'a' })).toBe('a');
    expect(chooseFnComposer({ candidates, lastFocused: null })).toBeNull();
    b.remove();
    expect(chooseFnComposer({ candidates, lastFocused: null })).toBe('a');
  });

  it('inside an open modal dialog only its own fields count', () => {
    const composer = el('textarea');
    const dialog = el('div', document.body, { role: 'dialog', 'aria-modal': 'true' });
    const button = el('button', dialog);
    button.focus();
    expect(modalScope()).toBe(dialog);
    const candidates = [candidate('tab:a', () => composer)];
    expect(chooseFnComposer({ candidates, lastFocused: 'tab:a' })).toBeNull();
    // The dialog's own field, focused, is the one.
    const name = el('input', dialog);
    name.focus();
    expect(focusedLooseField(candidates)).toBe(name);
    // A composer inside the dialog is chosen.
    const inner = el('textarea', dialog);
    button.focus();
    expect(
      chooseFnComposer({
        candidates: [...candidates, candidate('dialog:c', () => inner)],
        lastFocused: null,
      })
    ).toBe('dialog:c');
    // Closed: the composer comes back.
    dialog.remove();
    expect(modalScope()).toBeNull();
    expect(chooseFnComposer({ candidates, lastFocused: null })).toBe('tab:a');
  });
});

describe('a focused field no composer owns', () => {
  it('a rename box or a search field is one; a password never is', () => {
    const composer = el('textarea');
    const candidates = [candidate('tab:a', () => composer)];
    const rename = el('input');
    rename.focus();
    expect(focusedLooseField(candidates)).toBe(rename);
    const secret = el('input', document.body, { type: 'password' });
    secret.focus();
    expect(focusedLooseField(candidates)).toBeNull();
    // A composer's own field is the composer's, not a loose one.
    composer.focus();
    expect(focusedLooseField(candidates)).toBeNull();
    // Nothing focused.
    composer.blur();
    expect(focusedLooseField(candidates)).toBeNull();
  });

  it('an editable region is named by its editing host', () => {
    const host = el('div', document.body, { contenteditable: 'true', tabindex: '0' });
    const inner = el('p', host, { tabindex: '-1' });
    inner.focus();
    expect(focusedLooseField([])).toBe(host);
  });
});

describe('the composer that shows a notice for a field that is gone', () => {
  it('the active pane’s chat composer, preferring the ones that can show its actions', () => {
    const left = el('div', document.body, { 'data-active-group': 'false' });
    const right = el('div', document.body, { 'data-active-group': 'true' });
    const a = el('textarea', left);
    const b = el('textarea', right);
    const editor = el('textarea', right);
    const candidates = [
      candidate('a', () => a),
      candidate('editor:m1', () => editor),
      candidate('b', () => b),
    ];
    expect(visibleComposer(candidates)).toBe('b');
    expect(visibleComposer(candidates, (key) => key === 'a')).toBe('a');
    expect(visibleComposer([candidate('gone', () => null)])).toBeNull();
    // A dialog over them does not move the notice: it is there when the dialog closes.
    const dialog = el('div', document.body, { role: 'dialog', 'aria-modal': 'true' });
    el('button', dialog).focus();
    expect(visibleComposer(candidates)).toBe('b');
  });
});
