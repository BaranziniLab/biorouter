/**
 * ExtensionUpdateReporter.listeners.test.tsx
 *
 * The reporter's *lifecycle* contract, kept apart from its behaviour tests in
 * ExtensionUpdateReporter.test.tsx so the two stubs cannot drift into each
 * other: that file's stub captures the last callback so a test can emit events,
 * this one tracks the live set so a test can count them.
 *
 * PR #189 measured `MaxListenersExceededWarning: 11 extension-update-event
 * listeners` after a handful of route changes. The reporter subscribed in a
 * `useEffect(..., [])` and returned no cleanup, and the claim that justified it
 * — "mounted once, at the app shell, for the life of the window" — was false on
 * two counts: the reporter sits inside AppLayout, under ProviderGuard >
 * ChatProvider, so that subtree remounts; and React.StrictMode is on in dev, so
 * every mount runs the effect twice regardless.
 */
import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render } from '@testing-library/react';
import type { ExtensionUpdateEvent } from '../utils/extensionUpdater';

vi.mock('../toasts', () => ({
  toastError: vi.fn(),
  toastService: { success: vi.fn() },
}));

import ExtensionUpdateReporter from './ExtensionUpdateReporter';

/**
 * Stands in for the preload bridge, which registers a real `ipcRenderer.on`
 * listener and hands back a disposer that removes it. Membership of this set is
 * the renderer-side proxy for "how many listeners are on the IPC channel".
 */
let live: Set<(event: ExtensionUpdateEvent) => void>;
/**
 * Totals, not just the live set. `live.size` alone cannot tell "subscribed
 * twice and cleaned up once" apart from "only ever subscribed once", and those
 * are exactly the two worlds the StrictMode test below has to separate.
 */
let subscribes: number;
let disposes: number;

beforeEach(() => {
  live = new Set();
  subscribes = 0;
  disposes = 0;
  // @ts-expect-error — partial stub, only what the reporter touches.
  window.electron = {
    onExtensionUpdateEvent: (cb: (event: ExtensionUpdateEvent) => void) => {
      live.add(cb);
      subscribes++;
      return () => {
        live.delete(cb);
        disposes++;
      };
    },
  };
});

describe('ExtensionUpdateReporter listener lifecycle', () => {
  it('leaves no listener behind across repeated mounts', () => {
    // 20 cycles, not 2: the warning fires at 11, so a leak that only shows up
    // after a handful of remounts has to be visible here.
    for (let i = 0; i < 20; i++) {
      const { unmount } = render(<ExtensionUpdateReporter />);
      expect(live.size, `one live listener while mounted (cycle ${i + 1})`).toBe(1);
      unmount();
      expect(live.size, `no live listener after unmount (cycle ${i + 1})`).toBe(0);
    }

    expect(live.size).toBe(0);
  });

  it('registers exactly one listener under StrictMode', () => {
    // React 19 StrictMode runs effect → cleanup → effect on the first mount, so
    // a component whose effect has no cleanup ends up subscribed twice. This is
    // the dev-mode half of #189, and it needs no route change to reproduce.
    render(
      <React.StrictMode>
        <ExtensionUpdateReporter />
      </React.StrictMode>
    );

    expect(live.size).toBe(1);
    // Assert the mechanism, not just the total. A harness where StrictMode had
    // stopped double-invoking would also leave one live listener, and would
    // then pass this test while proving nothing about the cleanup.
    expect({ subscribes, disposes }).toEqual({ subscribes: 2, disposes: 1 });
  });
});
