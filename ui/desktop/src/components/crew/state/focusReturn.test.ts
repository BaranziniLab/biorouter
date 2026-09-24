import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  canFocus,
  DIALOG_FOCUS_FALLBACKS,
  focusIsLost,
  nextOpener,
  openerOf,
  restoreFocus,
  restoreFocusSoon,
  SIGN_IN_FOCUS_FALLBACKS,
} from './focusReturn';

/** Replace the body with `html`. */
function mount(html: string): void {
  document.body.innerHTML = html;
}
const byId = (id: string) => document.getElementById(id) as HTMLElement;

afterEach(() => {
  document.body.innerHTML = '';
  vi.useRealTimers();
});

describe('openerOf', () => {
  it('is the element itself outside a menu, and nothing for <body>', () => {
    mount('<button id="add">Add people</button>');
    expect(openerOf(byId('add'))).toBe(byId('add'));
    expect(openerOf(document.body)).toBeNull();
    expect(openerOf(null)).toBeNull();
  });

  it('stands a menu item in for the trigger that controls its menu', () => {
    mount(`
      <button id="trigger" aria-controls="menu-1">Channel menu</button>
      <div role="menu" id="menu-1"><div role="menuitem" id="item" tabindex="-1">Rename…</div></div>
    `);
    expect(openerOf(byId('item'))).toBe(byId('trigger'));
  });

  it('falls back to the element that names the menu, once the trigger no longer controls it', () => {
    // Radix drops `aria-controls` as the menu closes; `aria-labelledby` stays on the content.
    mount(`
      <button id="trigger">Workspace</button>
      <div role="menu" aria-labelledby="trigger"><div role="menuitem" id="item">Keys…</div></div>
    `);
    expect(openerOf(byId('item'))).toBe(byId('trigger'));
  });

  it('climbs from a submenu to the root menu’s trigger', () => {
    mount(`
      <button id="trigger">Workspace</button>
      <div role="menu" aria-labelledby="trigger">
        <div role="menuitem" id="sub-trigger">Add a workspace</div>
      </div>
      <div role="menu" aria-labelledby="sub-trigger"><div role="menuitem" id="item">Join…</div></div>
    `);
    expect(openerOf(byId('item'))).toBe(byId('trigger'));
  });

  it('gives up on a menu whose trigger cannot be found', () => {
    mount('<div role="menu"><div role="menuitem" id="item">Orphan</div></div>');
    expect(openerOf(byId('item'))).toBeNull();
  });
});

describe('nextOpener', () => {
  it('records whatever has focus when nothing was open', () => {
    mount('<button id="add">Add people</button>');
    byId('add').focus();
    expect(nextOpener(null)).toBe(byId('add'));
  });

  it('keeps the first opener when a dialog hands over to another', () => {
    mount(`
      <button id="add">Add people</button>
      <div role="dialog"><button id="invite">Invite people…</button></div>
    `);
    byId('invite').focus();
    expect(nextOpener(byId('add'))).toBe(byId('add'));
  });

  it('takes the control inside the dialog when the first opener is gone', () => {
    mount('<div role="dialog"><button id="invite">Invite people…</button></div>');
    const gone = document.createElement('button');
    byId('invite').focus();
    expect(nextOpener(gone)).toBe(byId('invite'));
  });
});

describe('restoreFocus', () => {
  it('puts focus back on a connected opener when focus was lost', () => {
    mount('<button id="add">Add people</button>');
    (document.activeElement as HTMLElement | null)?.blur();
    expect(focusIsLost()).toBe(true);
    expect(restoreFocus(byId('add'))).toBe(true);
    expect(byId('add')).toHaveFocus();
  });

  it('never takes focus from something that deliberately took it', () => {
    mount('<button id="add">Add people</button><textarea id="composer"></textarea>');
    byId('composer').focus();
    expect(restoreFocus(byId('add'))).toBe(false);
    expect(byId('composer')).toHaveFocus();
  });

  it('treats focus parked on a dialog’s own frame as lost', () => {
    mount('<button id="add">Add people</button><div role="dialog" tabindex="-1" id="frame"></div>');
    byId('frame').focus();
    expect(focusIsLost()).toBe(true);
    expect(restoreFocus(byId('add'))).toBe(true);
  });

  it('falls back to the channel heading, then the composer, when the opener is gone', () => {
    mount(`
      <div class="crew-app">
        <h1><button id="heading">general</button></h1>
        <textarea aria-label="Message #general" id="composer"></textarea>
      </div>
    `);
    const gone = document.createElement('button');
    expect(restoreFocus(gone, DIALOG_FOCUS_FALLBACKS)).toBe(true);
    expect(byId('heading')).toHaveFocus();

    byId('heading').blur();
    byId('heading').setAttribute('disabled', '');
    expect(restoreFocus(gone, DIALOG_FOCUS_FALLBACKS)).toBe(true);
    expect(byId('composer')).toHaveFocus();
  });

  it('sends Sign in’s focus to the workspace switcher first', () => {
    mount(`
      <div class="crew-app">
        <button class="crew-sidebar-switcher" id="switcher">lab</button>
        <h1><button id="heading">general</button></h1>
      </div>
    `);
    expect(restoreFocus(null, SIGN_IN_FOCUS_FALLBACKS)).toBe(true);
    expect(byId('switcher')).toHaveFocus();
  });

  it('skips a hidden, inert or disabled target', () => {
    mount(`
      <div inert><button id="inert">A</button></div>
      <div aria-hidden="true"><button id="hidden">B</button></div>
      <button id="off" disabled>C</button>
    `);
    expect(canFocus(byId('inert'))).toBe(false);
    expect(canFocus(byId('hidden'))).toBe(false);
    expect(canFocus(byId('off'))).toBe(false);
  });

  it('restores on the next frame, after the dialog has left', () => {
    vi.useFakeTimers();
    mount('<button id="add">Add people</button>');
    restoreFocusSoon(byId('add'));
    expect(byId('add')).not.toHaveFocus();
    vi.advanceTimersToNextFrame();
    expect(byId('add')).toHaveFocus();
  });
});
