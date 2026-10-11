import { describe, expect, it } from 'vitest';

import * as inputTrail from '../ui/inputTrail';
import { keepFocusOffAfterPointerClose, lastInputWasKey } from './menuFocusReturn';

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
