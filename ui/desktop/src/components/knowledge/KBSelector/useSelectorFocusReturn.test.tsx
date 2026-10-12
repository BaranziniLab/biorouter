import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { useSelectorFocusReturn } from './useSelectorFocusReturn';

/**
 * The hand-back rules, one at a time (V3-R7-3, V3-R9-1). The close decides, as the dialog
 * primitive's own rule does: how the dialog was opened does not matter.
 * `KnowledgeView.focusReturn.test.tsx` runs the real view and
 * `KnowledgeView.focusReturn.browser.test.tsx` runs it in Chromium, where the picker's close
 * motion is what makes the dialog read a stale opener; here the timing is the test's, so each
 * rule is pinned without leaning on either.
 */

let trigger: HTMLButtonElement;
let content: HTMLDivElement;

beforeEach(() => {
  trigger = document.createElement('button');
  content = document.createElement('div');
  document.body.append(trigger);
});

afterEach(() => {
  document.body.replaceChildren();
});

/** The last input the window saw, as `inputTrail` records it. */
function press(kind: 'key' | 'pointer') {
  window.dispatchEvent(new Event(kind === 'key' ? 'keydown' : 'pointerdown'));
}

/** A close report from Radix: the event is dispatched on the content that unmounted. */
function closeEvent(): Event {
  return new Event('closeautofocus', { cancelable: true });
}

function setup() {
  const hook = renderHook(() => useSelectorFocusReturn());
  hook.result.current.triggerRef.current = trigger;
  return hook;
}

describe('useSelectorFocusReturn', () => {
  it('gives focus to the trigger on a real close after a key pick closed with a key', () => {
    const { result } = setup();
    press('key');
    act(() => result.current.opened());
    act(() => result.current.closed());

    press('key');
    const event = closeEvent();
    result.current.onCloseAutoFocus(event);

    expect(event.defaultPrevented).toBe(true);
    expect(trigger).toHaveFocus();
  });

  // V3-R9-1: the trigger clicked open, "Manage bases..." clicked, Escape. The open used to be the
  // gate, so this left focus on <body> and the next Tab began at the top of the window.
  it('gives focus to the trigger after a pointer pick closed with a key', () => {
    const { result } = setup();
    press('pointer');
    act(() => result.current.opened());
    act(() => result.current.closed());

    press('key');
    const event = closeEvent();
    result.current.onCloseAutoFocus(event);

    expect(event.defaultPrevented).toBe(true);
    expect(trigger).toHaveFocus();
  });

  it('gives nothing back when a key pick is closed with the pointer', () => {
    const { result } = setup();
    press('key');
    act(() => result.current.opened());
    act(() => result.current.closed());

    press('pointer');
    const event = closeEvent();
    result.current.onCloseAutoFocus(event);

    expect(event.defaultPrevented).toBe(false);
    expect(trigger).not.toHaveFocus();
  });

  // A script focus after a mouse click can wear a keyboard ring, so a pointer in, pointer out
  // visit never moves focus.
  it('gives nothing back when a pointer pick is closed with the pointer', () => {
    const { result } = setup();
    press('pointer');
    act(() => result.current.opened());
    act(() => result.current.closed());

    press('pointer');
    const event = closeEvent();
    result.current.onCloseAutoFocus(event);

    expect(event.defaultPrevented).toBe(false);
    expect(trigger).not.toHaveFocus();
  });

  // The manager unmounts while the chooser takes its place and reports a close of its own. That
  // is not the person leaving, so it neither moves focus nor spends the hand-back for the real
  // close that follows the chooser.
  it('ignores the close the manager reports when the chooser takes its place, and still hands back later', () => {
    const { result } = setup();
    press('key');
    act(() => result.current.opened());

    const swap = closeEvent();
    result.current.onCloseAutoFocus(swap);
    expect(swap.defaultPrevented).toBe(false);
    expect(trigger).not.toHaveFocus();

    act(() => result.current.closed());
    press('key');
    const real = closeEvent();
    result.current.onCloseAutoFocus(real);
    expect(real.defaultPrevented).toBe(true);
    expect(trigger).toHaveFocus();
  });

  // The Create row reached with the pointer: the chooser takes the manager's place, Escape
  // returns to the manager, and the second Escape is the real close.
  it('hands back after a pointer pick through the chooser swap', () => {
    const { result } = setup();
    press('pointer');
    act(() => result.current.opened());

    press('key');
    const swap = closeEvent();
    result.current.onCloseAutoFocus(swap);
    expect(swap.defaultPrevented).toBe(false);

    act(() => result.current.closed());
    press('key');
    const real = closeEvent();
    result.current.onCloseAutoFocus(real);
    expect(real.defaultPrevented).toBe(true);
    expect(trigger).toHaveFocus();
  });

  it('leaves focus alone when it already went somewhere outside the dialog', () => {
    const { result } = setup();
    const elsewhere = document.createElement('input');
    document.body.append(elsewhere);
    press('key');
    act(() => result.current.opened());
    act(() => result.current.closed());

    elsewhere.focus();
    const event = closeEvent();
    Object.defineProperty(event, 'target', { value: content });
    result.current.onCloseAutoFocus(event);

    expect(event.defaultPrevented).toBe(false);
    expect(elsewhere).toHaveFocus();
  });

  it('does nothing once the trigger has left the page', () => {
    const { result } = setup();
    press('key');
    act(() => result.current.opened());
    act(() => result.current.closed());
    trigger.remove();

    const event = closeEvent();
    result.current.onCloseAutoFocus(event);

    expect(event.defaultPrevented).toBe(false);
  });

  it('hands back once per open', () => {
    const { result } = setup();
    press('key');
    act(() => result.current.opened());
    act(() => result.current.closed());
    press('key');
    result.current.onCloseAutoFocus(closeEvent());
    trigger.blur();

    const again = closeEvent();
    result.current.onCloseAutoFocus(again);

    expect(again.defaultPrevented).toBe(false);
    expect(trigger).not.toHaveFocus();
  });

  // `pending` is the open's debt: a close report with no open in front of it (a stray report from
  // the primitive after the hand-back was spent) never moves focus, even after a key press.
  it('hands back nothing for a close report that no open asked for', () => {
    const { result } = setup();
    press('key');

    const event = closeEvent();
    result.current.onCloseAutoFocus(event);

    expect(event.defaultPrevented).toBe(false);
    expect(trigger).not.toHaveFocus();
  });
});
