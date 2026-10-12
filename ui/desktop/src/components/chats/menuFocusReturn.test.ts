import { afterEach, describe, expect, it } from 'vitest';

import * as inputTrail from '../ui/inputTrail';
import {
  keepFocusOffAfterPointerClose,
  lastInputWasKey,
  returnFocusAfterKeyClose,
} from './menuFocusReturn';

const close = () => {
  const event = new Event('menuCloseAutoFocus', { cancelable: true });
  keepFocusOffAfterPointerClose(event);
  return event;
};

describe('where focus goes when a row menu closes', () => {
  it('does not hand focus back after a pointer press', () => {
    window.dispatchEvent(new Event('pointerdown'));
    expect(lastInputWasKey()).toBe(false);
    expect(close().defaultPrevented).toBe(true);
  });

  it('hands focus back after a key press (Escape, or Enter on an item)', () => {
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    expect(lastInputWasKey()).toBe(true);
    expect(close().defaultPrevented).toBe(false);
  });

  it('follows whichever input came last', () => {
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'F10', shiftKey: true }));
    window.dispatchEvent(new Event('pointerdown'));
    expect(close().defaultPrevented).toBe(true);
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown' }));
    expect(close().defaultPrevented).toBe(false);
  });

  it('reads the app’s one input tracker, so a menu and the dialog it opens agree', () => {
    expect(lastInputWasKey).toBe(inputTrail.lastInputWasKey);
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
    expect(inputTrail.lastInputWasKey()).toBe(true);
    expect(close().defaultPrevented).toBe(false);
    window.dispatchEvent(new Event('pointerdown'));
    expect(inputTrail.lastInputWasKey()).toBe(false);
    expect(close().defaultPrevented).toBe(true);
  });
});

/** V1-R8-1: a dialog a row's menu opened gives focus back to that row when a key closed it. */
describe('where focus goes when a dialog a row menu opened closes', () => {
  afterEach(() => {
    document.body.replaceChildren();
  });

  const setUp = () => {
    const row = document.createElement('button');
    document.body.append(row);
    return { row, event: new Event('dialogCloseAutoFocus', { cancelable: true }) };
  };

  it('focuses the row after a key close, and stops the dialog placing focus itself', () => {
    const { row, event } = setUp();
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    returnFocusAfterKeyClose(event, row);
    expect(document.activeElement).toBe(row);
    expect(event.defaultPrevented).toBe(true);
  });

  it('leaves focus alone after a pointer close', () => {
    const { row, event } = setUp();
    window.dispatchEvent(new Event('pointerdown'));
    returnFocusAfterKeyClose(event, row);
    expect(document.activeElement).not.toBe(row);
    expect(event.defaultPrevented).toBe(false);
  });

  it('leaves it to the dialog when the row has gone or a caller already placed focus', () => {
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    const gone = setUp();
    gone.row.remove();
    returnFocusAfterKeyClose(gone.event, gone.row);
    expect(gone.event.defaultPrevented).toBe(false);
    returnFocusAfterKeyClose(gone.event, null);
    expect(gone.event.defaultPrevented).toBe(false);

    const placed = setUp();
    placed.event.preventDefault();
    returnFocusAfterKeyClose(placed.event, placed.row);
    expect(document.activeElement).not.toBe(placed.row);
  });
});
