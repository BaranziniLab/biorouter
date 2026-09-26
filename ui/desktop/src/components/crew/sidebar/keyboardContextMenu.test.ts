import { describe, expect, it, vi } from 'vitest';
import { isContextMenuKey, openContextMenuFromKeyboard } from './keyboardContextMenu';

const key = (init: Partial<Parameters<typeof isContextMenuKey>[0]> & { key: string }) => ({
  shiftKey: false,
  altKey: false,
  ctrlKey: false,
  metaKey: false,
  ...init,
});

describe('isContextMenuKey', () => {
  it('accepts Shift+F10 and the unmodified Menu key', () => {
    expect(isContextMenuKey(key({ key: 'F10', shiftKey: true }))).toBe(true);
    expect(isContextMenuKey(key({ key: 'ContextMenu' }))).toBe(true);
  });

  it('refuses every other combination, as Chromium does', () => {
    expect(isContextMenuKey(key({ key: 'F10' }))).toBe(false);
    expect(isContextMenuKey(key({ key: 'F10', shiftKey: true, altKey: true }))).toBe(false);
    expect(isContextMenuKey(key({ key: 'F10', shiftKey: true, ctrlKey: true }))).toBe(false);
    expect(isContextMenuKey(key({ key: 'F10', shiftKey: true, metaKey: true }))).toBe(false);
    expect(isContextMenuKey(key({ key: 'ContextMenu', shiftKey: true }))).toBe(false);
    expect(isContextMenuKey(key({ key: 'ContextMenu', ctrlKey: true }))).toBe(false);
    expect(isContextMenuKey(key({ key: 'Enter', shiftKey: true }))).toBe(false);
  });
});

describe('openContextMenuFromKeyboard', () => {
  it('dispatches a bubbling, cancelable contextmenu anchored under the name, at its leading edge', () => {
    const row = document.createElement('button');
    const name = document.createElement('span');
    row.append(name);
    document.body.append(row);
    vi.spyOn(row, 'getBoundingClientRect').mockReturnValue(
      DOMRect.fromRect({ x: 10, y: 100, width: 200, height: 32 })
    );
    vi.spyOn(name, 'getBoundingClientRect').mockReturnValue(
      DOMRect.fromRect({ x: 42, y: 108, width: 60, height: 16 })
    );
    const seen: MouseEvent[] = [];
    document.body.addEventListener('contextmenu', (event) => {
      seen.push(event);
      event.preventDefault();
    });

    expect(openContextMenuFromKeyboard(row, name)).toBe(false);
    expect(seen).toHaveLength(1);
    expect(seen[0].target).toBe(row);
    expect(seen[0].cancelable).toBe(true);
    expect([seen[0].clientX, seen[0].clientY]).toEqual([42, 132]);
    row.remove();
  });
});
