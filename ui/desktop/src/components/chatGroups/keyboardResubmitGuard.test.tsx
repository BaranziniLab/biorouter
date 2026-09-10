import React from 'react';
import { render, act, waitFor } from '@testing-library/react';
import { describe, expect, it, vi, beforeEach } from 'vitest';
import { MemoryRouter } from 'react-router-dom';
import { requestNewTab, resetNewTabRegistry } from './newTabRegistry';
import { closeActiveTab, resetCloseActiveTabRegistry } from './closeActiveTabRegistry';

/**
 * REGRESSION GATE (keyboard half) — duplicate submission on Cmd+T / Cmd+W.
 *
 * GitHub issue #19: "send a message, Cmd+T Cmd+T, Cmd+W Cmd+W, and the latest
 * message is sent AGAIN". Same defect as the close-BUTTON repro that
 * duplicateSubmissionWiring.test.tsx pins, but reached by a different road, and
 * the difference is not cosmetic:
 *
 *   the "+" and "x" buttons  -> onClick -> dispatch
 *   Cmd+T / Cmd+W            -> Electron MENU accelerator -> IPC -> App.tsx
 *                            -> newTabRegistry / closeActiveTabRegistry
 *                            -> the handler ChatGroupsProvider registered
 *                            -> dispatch
 *
 * The keystrokes never reach the DOM at all (main.ts owns them as menu items),
 * so no amount of key-event testing exercises this; the registries ARE the
 * keyboard's entry point and this drives them directly. The two roads converge
 * on the same reducer, which is precisely the claim under test — a claim that
 * was previously assumed rather than proven.
 *
 * The fix being gated is ChatGroupsShell dispatching `consumePending` through
 * `onInitialMessageConsumed` (commit f1f1d6b6). Remove that dispatch and the
 * SANITY case below documents what comes back.
 */

const MARKER = 'ISSUE19PROBE name one metal';

/** Every text the stand-in BaseChat actually submitted, across all mounts. */
let submitLog: string[] = [];
/** How many times a BaseChat mounted — the SANITY case's non-vacuity meter. */
let baseChatMounts = 0;

vi.mock('../BaseChat', () => {
  function BaseChatStub({
    initialMessage,
    onInitialMessageConsumed,
    renderSessionTitle,
  }: {
    initialMessage?: string;
    onInitialMessageConsumed?: () => void;
    renderSessionTitle?: () => React.ReactNode;
  }) {
    // Mirrors BaseChat's `hasAutoSubmittedRef`: per-mount, so a remount forgets.
    // That amnesia is the bug's engine — only a group's ACTIVE tab is mounted,
    // so closing tabs remounts the survivor with a fresh, empty ref.
    const hasAutoSubmitted = React.useRef(false);
    React.useEffect(() => {
      baseChatMounts += 1;
    }, []);
    React.useEffect(() => {
      if (hasAutoSubmitted.current) return;
      if (!initialMessage) return;
      hasAutoSubmitted.current = true;
      submitLog.push(initialMessage);
      onInitialMessageConsumed?.();
    }, [initialMessage, onInitialMessageConsumed]);
    return <div data-testid="basechat">{renderSessionTitle?.()}</div>;
  }
  return { default: BaseChatStub };
});

vi.mock('../InAppTerminalDock', () => ({ default: () => <div data-testid="dock" /> }));

vi.mock('../../contexts/TerminalDockContext', () => ({
  useTerminalDock: () => ({
    isOpenFor: () => false,
    setOpen: vi.fn(),
    remove: vi.fn(),
    retain: vi.fn(),
    terminals: [],
  }),
}));

vi.mock('../ui/sidebar', () => ({
  useSidebar: () => ({ state: 'expanded', isMobile: false }),
}));

// Closing a tab detaches that session's observer stream (§4.3), so the provider
// reads the registry as well as the hook — on ANY close, not only a daemon
// frame. A mock that supplies only `useRunningChats` throws on the first close.
vi.mock('../../hooks/chatStreamStore', () => ({
  useRunningChats: () => [],
  defaultChatStreamRegistry: { peekController: () => undefined },
  // The shell reads the live per-chat privacy tiers from the registry (finding
  // M8). Nothing here is about privacy, so an empty map is the whole stub —
  // but it has to be PRESENT: this file replaces the module wholesale, and a
  // missing export is a render-time throw, not an `undefined`.
  useLiveSessionTiers: () => ({}),
}));
vi.mock('../../utils/sessionNameSync', () => ({
  subscribeSessionNameChanges: () => () => undefined,
}));

import { ChatGroupsProvider, useChatGroups } from '../../contexts/ChatGroupsContext';
import ChatGroupsShell from './ChatGroupsShell';
import { ChatGroupsState, leafGroupIds } from './chatGroupsTypes';

let latestState: ChatGroupsState | null = null;

/** The `openTab` a Home-screen submit dispatches: a session plus route cargo. */
const seedAction = (message: string) =>
  ({
    type: 'openTab',
    payload: { sessionId: 'session-A', title: 'session-A', pendingInitialMessage: message },
  }) as const;

/**
 * Seeds the tab the way the app does, and publishes the live state and dispatch
 * so assertions can read the cargo directly rather than inferring it from a DOM
 * that lies across tab switches.
 */
function Seeder() {
  const ctx = useChatGroups();
  latestState = ctx?.state ?? null;
  const seeded = React.useRef(false);
  React.useEffect(() => {
    if (seeded.current || !ctx) return;
    seeded.current = true;
    ctx.dispatch(seedAction(MARKER));
  }, [ctx]);
  return null;
}

function mount() {
  return render(
    <MemoryRouter initialEntries={['/pair']}>
      <ChatGroupsProvider>
        <Seeder />
        <ChatGroupsShell onChatChange={() => {}} />
      </ChatGroupsProvider>
    </MemoryRouter>
  );
}

const allTabs = (s: ChatGroupsState | null) =>
  s ? leafGroupIds(s.layout).flatMap((id) => s.groups[id].tabs) : [];
const cargo = (s: ChatGroupsState | null) =>
  allTabs(s)
    .map((t) => t.pendingInitialMessage)
    .filter((m) => m !== undefined);

/** Cmd+T, through the registry the menu's IPC handler calls. */
const cmdT = () => act(() => void requestNewTab());
/** Cmd+W, likewise. `true` means a tab was claimed; `false` closes the window. */
const cmdW = () => {
  let claimed = false;
  act(() => {
    claimed = closeActiveTab();
  });
  return claimed;
};

describe('Cmd+T / Cmd+W do not re-submit a tab’s initial message (issue #19)', () => {
  beforeEach(() => {
    localStorage.clear();
    submitLog = [];
    latestState = null;
    baseChatMounts = 0;
    resetNewTabRegistry();
    resetCloseActiveTabRegistry();
  });

  it('the seeded tab submits its message exactly once', async () => {
    mount();
    await waitFor(() => expect(submitLog).toEqual([MARKER]));
    expect(cargo(latestState)).toEqual([]);
  });

  /** The reporter's repro, verbatim: Cmd+T Cmd+T, then Cmd+W Cmd+W. */
  it('survives Cmd+T Cmd+T then Cmd+W Cmd+W', async () => {
    mount();
    await waitFor(() => expect(submitLog).toEqual([MARKER]));

    cmdT();
    cmdT();
    await waitFor(() => expect(allTabs(latestState)).toHaveLength(3));

    expect(cmdW()).toBe(true);
    expect(cmdW()).toBe(true);
    await waitFor(() => expect(allTabs(latestState)).toHaveLength(1));

    // Back on the original tab, which has just remounted. It must stay quiet.
    expect(submitLog).toEqual([MARKER]);
    expect(allTabs(latestState).map((t) => t.sessionId)).toEqual(['session-A']);
    expect(cargo(latestState)).toEqual([]);
  });

  it('survives three Cmd+T/Cmd+W cycles', async () => {
    mount();
    await waitFor(() => expect(submitLog).toEqual([MARKER]));

    for (let i = 0; i < 3; i++) {
      cmdT();
      await waitFor(() => expect(allTabs(latestState)).toHaveLength(2));
      expect(cmdW()).toBe(true);
      await waitFor(() => expect(allTabs(latestState)).toHaveLength(1));
    }

    expect(submitLog).toEqual([MARKER]);
    expect(cargo(latestState)).toEqual([]);
  });

  /**
   * Interleaved rather than nested: open, open, close, open, close, close. The
   * reducer picks a different survivor to activate depending on which tab left,
   * so this walks remount paths the strictly-nested order never reaches.
   */
  it('survives an interleaved open/close order', async () => {
    mount();
    await waitFor(() => expect(submitLog).toEqual([MARKER]));

    cmdT();
    cmdT();
    await waitFor(() => expect(allTabs(latestState)).toHaveLength(3));
    cmdW();
    cmdT();
    await waitFor(() => expect(allTabs(latestState)).toHaveLength(3));
    cmdW();
    cmdW();
    await waitFor(() => expect(allTabs(latestState)).toHaveLength(1));

    expect(submitLog).toEqual([MARKER]);
    expect(cargo(latestState)).toEqual([]);
  });

  /**
   * Cmd+W past the last tab. The registry must report `false` so App.tsx falls
   * through to closing the WINDOW — and the extra keystrokes must not resurrect
   * anything on the way out.
   */
  it('reports false once the strip is empty, without re-submitting', async () => {
    mount();
    await waitFor(() => expect(submitLog).toEqual([MARKER]));

    expect(cmdW()).toBe(true);
    await waitFor(() => expect(allTabs(latestState)).toHaveLength(0));
    expect(cmdW()).toBe(false);
    expect(submitLog).toEqual([MARKER]);
  });

  /**
   * Proves the gate is not vacuous.
   *
   * Everything above asserts an ABSENCE — that nothing was re-submitted — and an
   * absence is exactly what a hollow test also reports. The gate is only
   * meaningful while the mechanism the bug rides on is still present: the
   * keyboard churn must genuinely UNMOUNT and REMOUNT the surviving tab's
   * BaseChat, resetting the per-mount `hasAutoSubmittedRef` that used to be the
   * only thing standing between the user and a duplicate turn.
   *
   * So count the mounts. If a future change kept every tab mounted (or memoised
   * the pane across a close), this case fails and tells you the silence above
   * has stopped meaning anything — rather than letting it quietly pass forever.
   */
  it('SANITY: the Cmd+T/Cmd+W drive really does remount BaseChat', async () => {
    mount();
    await waitFor(() => expect(submitLog).toEqual([MARKER]));
    const mountsAfterSeed = baseChatMounts;

    cmdT();
    cmdT();
    await waitFor(() => expect(allTabs(latestState)).toHaveLength(3));
    cmdW();
    cmdW();
    await waitFor(() => expect(allTabs(latestState)).toHaveLength(1));

    // The survivor came back from scratch — a fresh ref, which is precisely why
    // clearing the cargo (and not the ref) is what fixes this.
    expect(baseChatMounts).toBeGreaterThan(mountsAfterSeed);
    expect(submitLog).toEqual([MARKER]);
  });
});
