import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { useSelectorFocusReturn } from './useSelectorFocusReturn';

/**
 * The hand-back rules, one at a time (V3-R7-3). `KnowledgeView.focusReturn.test.tsx` runs the real
 * view and `KnowledgeView.focusReturn.browser.test.tsx` runs it in Chromium, where the picker's
 * close motion is what makes the dialog read a stale opener; here the timing is the test's, so each
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

  it('gives nothing back for a pointer pick', () => {
    const { result } = setup();
    press('pointer');
    act(() => result.current.opened());
    act(() => result.current.closed());

    press('key');
    const event = closeEvent();
    result.current.onCloseAutoFocus(event);

    expect(event.defaultPrevented).toBe(false);
    expect(trigger).not.toHaveFocus();
  });

  it('gives nothing back when the dialog is closed with the pointer', () => {
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
});
