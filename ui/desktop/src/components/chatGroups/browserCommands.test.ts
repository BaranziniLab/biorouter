import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { BROWSER_NO_DESKTOP, isSessionOnScreen, runBrowserCommand } from './browserCommands';
import {
  BROWSER_CHAT_NOT_ON_SCREEN,
  applyWorkspaceCommand,
  hasPendingWorkspaceCommands,
  resetWorkspaceCommandRegistry,
  type WorkspaceCommand,
} from './workspaceCommandRegistry';
import type { ChatGroupsState } from './chatGroupsTypes';

/**
 * BB-10 and BB-15 in the renderer: a browser frame runs only for a chat on screen in this
 * window, is forwarded to main as it came, and is never queued for later.
 */

const frame = (extra: Partial<WorkspaceCommand> = {}): WorkspaceCommand => ({
  type: 'workspace',
  cmd: 'browser',
  session_id: 'chat-a',
  request_id: 'wsreq-7',
  op: 'click',
  args: { ref: '@e3' },
  grant: 'observe',
  tier: 'private',
  deadline_ms: 30_000,
  ...extra,
});

const state = (active: string, background: string): Pick<ChatGroupsState, 'layout' | 'groups'> =>
  ({
    layout: { kind: 'leaf', groupId: 'g1' },
    groups: {
      g1: {
        groupId: 'g1',
        activeTabId: 't1',
        tabs: [
          { tabId: 't1', sessionId: active, title: 'A' },
          { tabId: 't2', sessionId: background, title: 'B' },
        ],
      },
    },
  }) as unknown as Pick<ChatGroupsState, 'layout' | 'groups'>;

const run = vi.fn();
const cancel = vi.fn();

beforeEach(() => {
  run
    .mockReset()
    .mockResolvedValue({ ok: true, detail: 'clicked [button] "Go"', data: { tab: 't1' } });
  cancel.mockReset().mockResolvedValue(true);
  (window as unknown as { electron: unknown }).electron = { browserAgent: { run, cancel } };
});
afterEach(() => {
  resetWorkspaceCommandRegistry();
  delete (window as unknown as { electron?: unknown }).electron;
});

describe('runBrowserCommand', () => {
  it('forwards an operation for a chat on screen, as the daemon sent it', async () => {
    const result = await runBrowserCommand(frame({ download_dir: '/w/downloads' }), () => true);
    expect(run).toHaveBeenCalledWith({
      requestId: 'wsreq-7',
      sessionId: 'chat-a',
      op: 'click',
      args: { ref: '@e3' },
      grant: 'observe',
      tier: 'private',
      deadlineMs: 30_000,
      downloadDir: '/w/downloads',
    });
    expect(result).toEqual({ ok: true, detail: 'clicked [button] "Go"', data: { tab: 't1' } });
  });

  it('refuses a chat that is not on screen, without touching the browser', async () => {
    const result = await runBrowserCommand(frame(), () => false);
    expect(result.ok).toBe(false);
    expect(result.detail).toBe(BROWSER_CHAT_NOT_ON_SCREEN);
    expect(run).not.toHaveBeenCalled();
  });

  it('reads a missing tier as public, and refuses a frame with no grant or a bad op', async () => {
    await runBrowserCommand(frame({ tier: undefined }), () => true);
    expect(run.mock.calls[0][0].tier).toBe('public');
    expect((await runBrowserCommand(frame({ grant: undefined }), () => true)).ok).toBe(false);
    expect((await runBrowserCommand(frame({ op: 'eval' }), () => true)).ok).toBe(false);
    expect(run).toHaveBeenCalledTimes(1);
  });

  it('says so in a web browser, where there is no desktop to host the page', async () => {
    (window as unknown as { electron: unknown }).electron = {};
    const result = await runBrowserCommand(frame(), () => true);
    expect(result.detail).toBe(BROWSER_NO_DESKTOP);
  });

  it('abandons the operation a cancel frame names', async () => {
    await runBrowserCommand(
      {
        type: 'workspace',
        cmd: 'browser_cancel',
        session_id: 'chat-a',
        cancel_request_id: 'wsreq-7',
      },
      () => true
    );
    expect(cancel).toHaveBeenCalledWith('wsreq-7');
  });
});

describe('isSessionOnScreen', () => {
  it('is the active tab of a visible group, not a tab behind it', () => {
    expect(isSessionOnScreen(state('chat-a', 'chat-b'), 'chat-a')).toBe(true);
    expect(isSessionOnScreen(state('chat-a', 'chat-b'), 'chat-b')).toBe(false);
    expect(isSessionOnScreen(state('chat-a', 'chat-b'), 'chat-c')).toBe(false);
  });
});

describe('applyWorkspaceCommand', () => {
  it('never queues a browser operation while no chat surface is mounted', () => {
    const result = applyWorkspaceCommand(frame());
    expect(result).toMatchObject({ ok: false, detail: BROWSER_CHAT_NOT_ON_SCREEN });
    expect(hasPendingWorkspaceCommands()).toBe(false);
  });
});
