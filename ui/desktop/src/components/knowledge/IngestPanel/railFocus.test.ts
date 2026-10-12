import { describe, expect, it } from 'vitest';
import { neighbourOf, railFocusElement } from './railFocus';

/**
 * V3-R6-1: the rule that says where focus goes when the control that held it leaves the rail.
 * The components' own cases (`IngestPanel.test.tsx`) press real controls; these pin the two small
 * pieces under them, including the one the cases cannot reach: ids are free text for a warning
 * (title, message and path joined), so a target is compared with the attribute and never put in a
 * selector.
 */
describe('neighbourOf', () => {
  const list = [{ id: 'a' }, { id: 'b' }, { id: 'c' }];

  it('takes the next entry, else the previous, else nothing', () => {
    expect(neighbourOf(list, 'a')).toBe(list[1]);
    expect(neighbourOf(list, 'b')).toBe(list[2]);
    expect(neighbourOf(list, 'c')).toBe(list[1]);
    expect(neighbourOf([{ id: 'only' }], 'only')).toBeNull();
  });

  it('names nothing for an id that is not in the list', () => {
    expect(neighbourOf(list, 'gone')).toBeNull();
    expect(neighbourOf([], 'a')).toBeNull();
  });
});

describe('railFocusElement', () => {
  function rail() {
    const root = document.createElement('div');
    root.innerHTML = `
      <button data-testid="knowledge-ingest-paste-text">Paste text</button>
      <div data-staged-id="s1"><button data-testid="knowledge-staged-remove">x</button></div>
      <div data-staged-id="s2"><button data-testid="knowledge-staged-remove">x</button></div>
      <button data-warning-id='odd "id" with [brackets] and a quote'>Dismiss</button>
      <button data-testid="knowledge-digest-button">Digest</button>`;
    return root;
  }

  it('finds each kind of target', () => {
    const root = rail();
    expect(railFocusElement(root, { to: 'paste' })).toBe(
      root.querySelector('[data-testid="knowledge-ingest-paste-text"]')
    );
    expect(railFocusElement(root, { to: 'digest' })).toBe(
      root.querySelector('[data-testid="knowledge-digest-button"]')
    );
    const row = root.querySelector('[data-staged-id="s2"]');
    expect(railFocusElement(root, { to: 'staged-row', id: 's2' })).toBe(row);
    expect(railFocusElement(root, { to: 'staged-remove', id: 's2' })).toBe(
      row!.querySelector('button')
    );
  });

  it('finds a warning whose id would break a selector', () => {
    const root = rail();
    const id = 'odd "id" with [brackets] and a quote';
    expect(railFocusElement(root, { to: 'warning-dismiss', id })).toBe(
      root.querySelector('[data-warning-id]')
    );
  });

  it('answers null for a target that is no longer on screen', () => {
    const root = rail();
    expect(railFocusElement(root, { to: 'staged-row', id: 'gone' })).toBeNull();
    expect(railFocusElement(root, { to: 'staged-remove', id: 'gone' })).toBeNull();
    expect(railFocusElement(root, { to: 'warning-dismiss', id: 'gone' })).toBeNull();
    expect(railFocusElement(document.createElement('div'), { to: 'paste' })).toBeNull();
  });
});
