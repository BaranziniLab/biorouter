import { renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { resolveDialogOpener, useReturnFocusToOpener } from './dialog';
import { lastFocusOutsideMenu, lastInputWasKey } from './inputTrail';

/**
 * The logic behind "a dialog opened from a menu item" (V4-03, V4-04). The real focus behaviour,
 * `:focus-visible` and the menus' focus traps are measured in Chromium
 * (`dialogFromMenu.browser.test.tsx`); jsdom has none of them, so this file holds the decisions
 * that do not need them: which control stands for a menu item, what the modality tracker says, and
 * when the hook acts.
 */
const press = () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
const point = () => window.dispatchEvent(new Event('pointerdown'));

function mount(html: string): HTMLElement {
  const host = document.createElement('div');
  host.innerHTML = html;
  document.body.appendChild(host);
  return host;
}

afterEach(() => {
  document.body.innerHTML = '';
});

describe('inputTrail', () => {
  it('reads the last input: a key press, then a pointer press, and the pointer before any', () => {
    press();
    expect(lastInputWasKey()).toBe(true);
    point();
    expect(lastInputWasKey()).toBe(false);
  });

  it('remembers the last control that held focus outside a menu, and not the menu’s own', () => {
    const host = mount(`
      <button id="row">Row</button>
      <div role="menu" id="m"><button role="menuitem" id="item">Item</button></div>
    `);
    host.querySelector<HTMLElement>('#row')!.focus();
    expect(lastFocusOutsideMenu()?.id).toBe('row');
    host.querySelector<HTMLElement>('#item')!.focus();
    expect(lastFocusOutsideMenu()?.id, 'a focus inside a menu is not an opener').toBe('row');
    host.remove();
    expect(lastFocusOutsideMenu(), 'a control that left the document is not one either').toBeNull();
  });
});

describe('resolveDialogOpener', () => {
  it('is the element itself outside a menu, and nothing for <body> or null', () => {
    const host = mount('<button id="plain">Plain</button>');
    const plain = host.querySelector<HTMLElement>('#plain')!;
    expect(resolveDialogOpener(plain)).toEqual({ opener: plain, viaMenu: false });
    expect(resolveDialogOpener(document.body)).toEqual({ opener: null, viaMenu: false });
    expect(resolveDialogOpener(null)).toEqual({ opener: null, viaMenu: false });
  });

  it('is the trigger a DropdownMenu is labelled by, for an item in it', () => {
    const host = mount(`
      <button id="trigger">More</button>
      <div role="menu" aria-labelledby="trigger"><div role="menuitem" id="item">Info</div></div>
    `);
    const result = resolveDialogOpener(host.querySelector('#item'));
    expect(result.viaMenu).toBe(true);
    expect(result.opener?.id).toBe('trigger');
  });

  it('climbs a submenu to the root menu’s trigger', () => {
    const host = mount(`
      <button id="trigger">More</button>
      <div role="menu" aria-labelledby="trigger">
        <div role="menuitem" id="sub-trigger">Share</div>
      </div>
      <div role="menu" aria-labelledby="sub-trigger"><div role="menuitem" id="deep">Link</div></div>
    `);
    expect(resolveDialogOpener(host.querySelector('#deep')).opener?.id).toBe('trigger');
  });

  it('finds the trigger by aria-controls when a caller’s own id replaced the one Radix names', () => {
    const host = mount(`
      <button id="own-id" aria-controls="menu-1">More</button>
      <div role="menu" id="menu-1" aria-labelledby="radix-gone"><div role="menuitem" id="item">Info</div></div>
    `);
    expect(resolveDialogOpener(host.querySelector('#item')).opener?.id).toBe('own-id');
  });

  it('is the control that held focus before an unlabelled menu (a ContextMenu) took it', () => {
    const host = mount(`
      <div id="row" tabindex="0">Row</div>
      <div role="menu"><div role="menuitem" id="item" tabindex="-1">Info</div></div>
    `);
    host.querySelector<HTMLElement>('#row')!.focus();
    host.querySelector<HTMLElement>('#item')!.focus();
    const result = resolveDialogOpener(document.activeElement);
    expect(result.viaMenu).toBe(true);
    expect(result.opener?.id).toBe('row');
  });

  it('is nothing when an unlabelled menu has no remembered control', () => {
    const host = mount(
      '<div role="menu"><div role="menuitem" id="item" tabindex="-1">Info</div></div>'
    );
    // The only focus the tracker ever sees is inside the menu, which it does not count.
    const stale = document.createElement('button');
    document.body.appendChild(stale);
    stale.focus();
    stale.remove();
    host.querySelector<HTMLElement>('#item')!.focus();
    expect(resolveDialogOpener(document.activeElement)).toEqual({ opener: null, viaMenu: true });
  });
});

describe('useReturnFocusToOpener, for a dialog a menu item opened', () => {
  /** Fire Radix's mount event on a content box, as its focus scope does, and report what it did. */
  function openOn(content: HTMLElement, hook: ReturnType<typeof useReturnFocusToOpener>) {
    const event = new CustomEvent('focusScope.autoFocusOnMount', { cancelable: true });
    content.addEventListener(event.type, hook.onOpenAutoFocus);
    content.dispatchEvent(event);
    content.removeEventListener(event.type, hook.onOpenAutoFocus);
    return event;
  }
  function closeOn(content: HTMLElement, hook: ReturnType<typeof useReturnFocusToOpener>) {
    const event = new CustomEvent('focusScope.autoFocusOnUnmount', { cancelable: true });
    content.addEventListener(event.type, hook.onCloseAutoFocus);
    content.dispatchEvent(event);
    content.removeEventListener(event.type, hook.onCloseAutoFocus);
    return event;
  }
  /** `field`: no text field, one before the Cancel button (the first stop), or one after it. */
  function scene(field: 'none' | 'first' | 'after') {
    const input = '<input id="field" aria-label="Name" />';
    const host = mount(`
      <button id="trigger">More</button>
      <div role="menu" id="menu" aria-labelledby="trigger"><button role="menuitem" id="item">Info</button></div>
      <div id="content" tabindex="-1">
        ${field === 'first' ? input : ''}
        <button id="cancel">Cancel</button>
        ${field === 'after' ? input : ''}
      </div>
    `);
    return {
      trigger: host.querySelector<HTMLElement>('#trigger')!,
      item: host.querySelector<HTMLElement>('#item')!,
      content: host.querySelector<HTMLElement>('#content')!,
      menu: host.querySelector<HTMLElement>('#menu')!,
    };
  }

  it('after a keyboard pick: leaves the open to Radix, and gives focus to the trigger on a keyboard close', () => {
    const { result } = renderHook(() => useReturnFocusToOpener({}));
    const { trigger, item, content, menu } = scene('none');
    press();
    item.focus();
    expect(openOn(content, result.current).defaultPrevented).toBe(false);
    menu.remove();
    press();
    const close = closeOn(content, result.current);
    expect(close.defaultPrevented).toBe(true);
    expect(trigger).toHaveFocus();
  });

  it('after a pointer pick: focuses the dialog box, and gives nothing back', () => {
    const { result } = renderHook(() => useReturnFocusToOpener({}));
    const { trigger, item, content, menu } = scene('none');
    point();
    item.focus();
    const open = openOn(content, result.current);
    expect(open.defaultPrevented, 'Radix is told not to focus its first stop').toBe(true);
    expect(content).toHaveFocus();
    menu.remove();
    press();
    expect(closeOn(content, result.current).defaultPrevented).toBe(false);
    expect(trigger).not.toHaveFocus();
  });

  it('after a pointer pick of a dialog whose first stop is a text field: leaves the field to Radix', () => {
    const { result } = renderHook(() => useReturnFocusToOpener({}));
    const { item, content } = scene('first');
    point();
    item.focus();
    expect(openOn(content, result.current).defaultPrevented).toBe(false);
  });

  it('after a pointer pick of a dialog with a text field AFTER a button: focuses the box (V4-R5-1)', () => {
    const { result } = renderHook(() => useReturnFocusToOpener({}));
    const { item, content } = scene('after');
    point();
    item.focus();
    expect(openOn(content, result.current).defaultPrevented).toBe(true);
    expect(content).toHaveFocus();
  });

  it('after a keyboard pick closed with the pointer: gives nothing back', () => {
    const { result } = renderHook(() => useReturnFocusToOpener({}));
    const { trigger, item, content, menu } = scene('none');
    press();
    item.focus();
    openOn(content, result.current);
    menu.remove();
    point();
    expect(closeOn(content, result.current).defaultPrevented).toBe(false);
    expect(trigger).not.toHaveFocus();
  });

  it('leaves a caller that cancelled the open event to place focus itself', () => {
    const { result } = renderHook(() =>
      useReturnFocusToOpener({
        onOpenAutoFocus: (event) => {
          event.preventDefault();
          document.getElementById('cancel')?.focus();
        },
      })
    );
    const { item, content } = scene('none');
    point();
    item.focus();
    openOn(content, result.current);
    expect(document.getElementById('cancel')).toHaveFocus();
  });

  it('a dialog that no menu opened: a pointer open rests on the box, a key open is Radix’s; both give focus back (V4-R5-1)', () => {
    const { result } = renderHook(() => useReturnFocusToOpener({}));
    const { trigger, content } = scene('none');
    for (const input of [point, press]) {
      input();
      trigger.focus();
      const open = openOn(content, result.current);
      expect(open.defaultPrevented).toBe(input === point);
      if (input === point) expect(content).toHaveFocus();
      trigger.blur();
      input();
      const close = closeOn(content, result.current);
      expect(close.defaultPrevented).toBe(true);
      expect(trigger).toHaveFocus();
    }
  });
});
