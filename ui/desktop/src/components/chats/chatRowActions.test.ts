import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  chatRowActions,
  copyConversationId,
  openChatInNewWindow,
  type ChatRowActionTarget,
} from './chatRowActions';

const mocks = vi.hoisted(() => ({
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock('../../toasts', () => ({
  toastSuccess: mocks.toastSuccess,
  toastError: mocks.toastError,
}));

function target(overrides: Partial<ChatRowActionTarget> = {}): ChatRowActionTarget {
  return {
    sessionId: '20260823_2',
    workingDir: '/Users/x/project',
    openInNewTab: vi.fn(),
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  Object.assign(navigator, { clipboard: { writeText: vi.fn(() => Promise.resolve()) } });
  Object.assign(window, { electron: { createChatWindow: vi.fn() } });
  // The two failure cases below log deliberately; `src/test/setup.ts` mocks
  // `console.log` only, so without this the suite's own stderr carries two
  // stack traces that look like real failures.
  vi.spyOn(console, 'error').mockImplementation(() => {});
});

describe('chatRowActions', () => {
  /**
   * The order is the issue's, and the reason to pin it is that four menus render
   * this array — History's `⋯` overflow and three right-click menus. A surface
   * that quietly reordered its own copy would be a menu whose second item does
   * something different depending on where you opened it.
   */
  it('offers the three actions in the specified order, with the labels the issue names', () => {
    expect(chatRowActions(target()).map((action) => [action.key, action.label])).toEqual([
      ['open-tab', 'Open in new tab'],
      ['open-window', 'Open in new window'],
      ['copy-id', 'Copy conversation ID'],
    ]);
  });

  /**
   * "Reuse the existing tab/window paths, do not create another lifecycle."
   * `openInNewTab` is the surface's own opener, handed in, so this asserts the
   * action calls it rather than reaching for a second one.
   */
  it('opens a tab through the surface’s own opener', () => {
    const openInNewTab = vi.fn();
    chatRowActions(target({ openInNewTab }))
      .find((a) => a.key === 'open-tab')!
      .run();
    expect(openInNewTab).toHaveBeenCalledTimes(1);
  });

  /**
   * The window path is asserted argument-for-argument, not just "was called":
   * these five positional arguments were History's, and `resumeSessionId` in
   * slot four plus `'pair'` in slot five are what make the new window load THAT
   * conversation as a tabbed surface. A call that dropped either would still
   * open a window, which is why a laxer assertion would not catch it.
   */
  it('opens a window with the same five arguments History always used', () => {
    chatRowActions(target())
      .find((a) => a.key === 'open-window')!
      .run();
    expect(window.electron.createChatWindow).toHaveBeenCalledWith(
      undefined,
      '/Users/x/project',
      undefined,
      '20260823_2',
      'pair'
    );
  });

  it('passes no directory when the surface does not have one', () => {
    openChatInNewWindow('20260823_9');
    expect(window.electron.createChatWindow).toHaveBeenCalledWith(
      undefined,
      undefined,
      undefined,
      '20260823_9',
      'pair'
    );
  });
});

describe('copyConversationId', () => {
  /**
   * The raw id and NOTHING else. The whole point is that the clipboard contents
   * can be pasted straight into a chat, where Chat Recall's exact-ID load and
   * every `workspace_*` id argument already accept exactly this string — so a
   * prefix, a label or a URL would have to be stripped back off by hand.
   */
  it('copies the id undecorated and confirms it', async () => {
    await copyConversationId('20260823_2');

    expect(navigator.clipboard.writeText).toHaveBeenCalledWith('20260823_2');
    expect(mocks.toastSuccess).toHaveBeenCalledWith(
      expect.objectContaining({ title: 'Conversation ID copied', msg: '20260823_2' })
    );
    expect(mocks.toastError).not.toHaveBeenCalled();
  });

  it('reports a rejected write instead of failing silently', async () => {
    Object.assign(navigator, {
      clipboard: { writeText: vi.fn(() => Promise.reject(new Error('denied'))) },
    });

    await expect(copyConversationId('20260823_2')).resolves.toBe(false);
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
    // The failure names the id, so the user can still get it out by hand.
    expect(mocks.toastError).toHaveBeenCalledWith(
      expect.objectContaining({ msg: expect.stringContaining('20260823_2') })
    );
  });

  /**
   * An absent `navigator.clipboard` is a different failure from a rejected one —
   * an insecure context has no API at all — and it used to be the one that read
   * as "the menu item does nothing".
   */
  it('reports a missing clipboard API the same way', async () => {
    Object.assign(navigator, { clipboard: undefined });

    await expect(copyConversationId('20260823_2')).resolves.toBe(false);
    expect(mocks.toastError).toHaveBeenCalledTimes(1);
  });
});

describe('native conversation ID clipboard', () => {
  it('uses the app bridge without requesting browser clipboard permission', async () => {
    const nativeCopy = vi.fn().mockResolvedValue(undefined);
    Object.assign(window.electron, { copyConversationId: nativeCopy });
    await copyConversationId('20260921_2');
    expect(nativeCopy).toHaveBeenCalledWith('20260921_2');
    expect(navigator.clipboard.writeText).not.toHaveBeenCalled();
    expect(mocks.toastSuccess).toHaveBeenCalled();
  });
  it('shows native write failures rather than reporting false success', async () => {
    Object.assign(window.electron, {
      copyConversationId: vi.fn().mockRejectedValue(new Error('denied')),
    });
    await copyConversationId('20260921_2');
    expect(mocks.toastError).toHaveBeenCalled();
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
  });
});
