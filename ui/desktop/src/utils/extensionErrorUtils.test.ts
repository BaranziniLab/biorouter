import { describe, it, expect, vi, beforeEach } from 'vitest';
import {
  showExtensionLoadResults,
  setFocusedChatSession,
  resetExtensionToastState,
} from './extensionErrorUtils';
import { resetExtensionLoadFailuresForTests } from './extensionLoadFailures';
import { toastService } from '../toasts';

vi.mock('../toasts', () => ({
  toastService: {
    extensionLoading: vi.fn(),
    error: vi.fn(),
    isExtensionToastActive: vi.fn(() => false),
  },
}));

const ok = (name: string) => ({ name, success: true, error: null });
const bad = (name: string, error = 'spawn ENOENT') => ({ name, success: false, error });

describe('showExtensionLoadResults — multi-chat toast rules', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetExtensionToastState();
    // The standing failure record is persistent BY DESIGN and deliberately
    // survives `resetExtensionToastState` — that is the whole fix — so it has
    // to be cleared explicitly or one case's failure silences the next one's.
    resetExtensionLoadFailuresForTests();
    vi.mocked(toastService.isExtensionToastActive).mockReturnValue(false);
  });

  it('toasts a clean load for the chat the user is focused on', () => {
    setFocusedChatSession('chat-a');
    showExtensionLoadResults([ok('weather-mcp'), ok('notion-mcp')], 'chat-a');

    expect(toastService.extensionLoading).toHaveBeenCalledTimes(1);
  });

  // The regression this whole rule exists for: four splits opening at once must
  // not stack four "all loaded" toasts over the transcript you are reading.
  it('stays silent for a BACKGROUND chat that loads cleanly', () => {
    setFocusedChatSession('chat-a');
    showExtensionLoadResults([ok('weather-mcp')], 'chat-b');

    expect(toastService.extensionLoading).not.toHaveBeenCalled();
  });

  it('four chats opening at once produce exactly one toast — the focused one', () => {
    setFocusedChatSession('chat-2');
    for (const id of ['chat-1', 'chat-2', 'chat-3', 'chat-4']) {
      showExtensionLoadResults([ok('weather-mcp'), ok('notion-mcp')], id);
    }

    expect(toastService.extensionLoading).toHaveBeenCalledTimes(1);
  });

  // A silent failure resurfaces later as a broken tool call. Never swallow it,
  // even from a pane the user is not currently looking at.
  it('toasts a FAILURE from a background chat anyway', () => {
    setFocusedChatSession('chat-a');
    showExtensionLoadResults([ok('weather-mcp'), bad('notion-mcp')], 'chat-b');

    expect(toastService.extensionLoading).toHaveBeenCalledTimes(1);
    const [statuses] = vi.mocked(toastService.extensionLoading).mock.calls[0];
    expect(statuses.find((s) => s.name === 'notion-mcp')?.status).toBe('error');
  });

  it('does not let a later success overwrite a failure still on screen', () => {
    setFocusedChatSession('chat-a');
    showExtensionLoadResults([bad('notion-mcp')], 'chat-b');
    vi.mocked(toastService.extensionLoading).mockClear();

    // That failure toast is still up...
    vi.mocked(toastService.isExtensionToastActive).mockReturnValue(true);
    showExtensionLoadResults([ok('weather-mcp')], 'chat-a');

    expect(toastService.extensionLoading).not.toHaveBeenCalled();
  });

  it('lets a success through once the failure toast has gone', () => {
    setFocusedChatSession('chat-a');
    showExtensionLoadResults([bad('notion-mcp')], 'chat-b');
    vi.mocked(toastService.extensionLoading).mockClear();

    vi.mocked(toastService.isExtensionToastActive).mockReturnValue(false);
    showExtensionLoadResults([ok('weather-mcp')], 'chat-a');

    expect(toastService.extensionLoading).toHaveBeenCalledTimes(1);
  });

  it('toasts when no host has claimed focus (single-chat hosts degrade permissively)', () => {
    showExtensionLoadResults([ok('weather-mcp')], 'chat-a');

    expect(toastService.extensionLoading).toHaveBeenCalledTimes(1);
  });

  it('toasts for non-chat callers, which pass no session id', () => {
    setFocusedChatSession('chat-a');
    showExtensionLoadResults([ok('weather-mcp'), ok('notion-mcp')]);

    expect(toastService.extensionLoading).toHaveBeenCalledTimes(1);
  });

  it('says nothing when there is nothing to report', () => {
    setFocusedChatSession('chat-a');
    showExtensionLoadResults([], 'chat-a');
    showExtensionLoadResults(null, 'chat-a');
    showExtensionLoadResults(undefined, 'chat-a');

    expect(toastService.extensionLoading).not.toHaveBeenCalled();
    expect(toastService.error).not.toHaveBeenCalled();
  });

  it('a lone failed extension still gets the dedicated error toast', () => {
    setFocusedChatSession('chat-a');
    showExtensionLoadResults([bad('notion-mcp', 'connection refused')], 'chat-a');

    expect(toastService.error).toHaveBeenCalledTimes(1);
    expect(vi.mocked(toastService.error).mock.calls[0][0]).toMatchObject({
      title: 'notion-mcp',
      msg: 'connection refused',
    });
  });

  it('counts only the user extensions — built-ins that loaded are not in the tally', () => {
    setFocusedChatSession('chat-a');
    // 5 user extensions (1 failed) + the built-ins that always come up.
    showExtensionLoadResults(
      [
        ok('developer'),
        ok('memory'),
        ok('autovisualiser'),
        ok('knowledge'), // built-ins: excluded
        ok('weather-mcp'),
        ok('notion-mcp'),
        ok('slack-mcp'),
        ok('github-mcp'),
        bad('failing-mcp'), // user: 5 total, 1 failed
      ],
      'chat-a'
    );

    // "4 of 5", not "8 of 9": the total passed to the toast excludes the loaded
    // built-ins, and the statuses list carries only the user extensions.
    expect(toastService.extensionLoading).toHaveBeenCalledTimes(1);
    const [statuses, total] = vi.mocked(toastService.extensionLoading).mock.calls[0];
    expect(total).toBe(5);
    expect(statuses.map((s) => s.name)).toEqual([
      'weather-mcp',
      'notion-mcp',
      'slack-mcp',
      'github-mcp',
      'failing-mcp',
    ]);
  });

  it('a built-in that FAILED is still surfaced, named as a built-in', () => {
    setFocusedChatSession('chat-a');
    showExtensionLoadResults([bad('knowledge', 'index corrupt')], 'chat-a');
    // Surfaced — but through the grouped toast's separate built-in channel, not
    // the lone-failure error toast, which would have presented a shipped
    // capability as though the user had installed it.
    expect(toastService.error).not.toHaveBeenCalled();
    expect(toastService.extensionLoading).toHaveBeenCalledTimes(1);
    const [statuses, total, , builtinFailures] = vi.mocked(toastService.extensionLoading).mock
      .calls[0];
    expect(statuses).toEqual([]);
    expect(total).toBe(0);
    expect(builtinFailures).toEqual(['knowledge']);
  });

  it('stays silent when only built-ins loaded', () => {
    setFocusedChatSession('chat-a');
    showExtensionLoadResults([ok('developer'), ok('memory'), ok('knowledge')], 'chat-a');
    expect(toastService.extensionLoading).not.toHaveBeenCalled();
    expect(toastService.error).not.toHaveBeenCalled();
  });

  /**
   * ⚠ **The property the user actually reads: this toast's denominator and the
   * composer's extension menu must count the same things.**
   *
   * They did not. This filter asked `bundled-extensions.json` (7 entries typed
   * `builtin`); the menu asks the capability catalog (12). The five in the gap
   * ship with Biorouter, are hidden by the menu, and were counted here as the
   * user's own — so a machine with two installed extensions and five shipped
   * capabilities produced a denominator of seven over a menu showing two.
   */
  it('counts only what the composer menu lists — the five capabilities outside bundled-extensions.json are not the user’s', () => {
    setFocusedChatSession('chat-a');
    showExtensionLoadResults(
      [
        // In the capability catalog but NOT in bundled-extensions.json: the
        // exact five that used to leak into the count.
        ok('code_execution'),
        ok('extensionmanager'),
        ok('skills'),
        ok('todo'),
        ok('chatrecall'),
        // In both.
        ok('developer'),
        // The user's own.
        ok('medcp'),
        bad('cdwagent'),
      ],
      'chat-a'
    );

    const [statuses, total] = vi.mocked(toastService.extensionLoading).mock.calls[0];
    expect(total).toBe(2);
    expect(statuses.map((s) => s.name)).toEqual(['medcp', 'cdwagent']);
  });

  /**
   * A failed capability used to sit IN the ratio, making the denominator
   * exactly one larger than the menu — which is how "2 of 3 extensions loaded"
   * came to hang over a menu showing two.
   */
  it('keeps a failed built-in out of the ratio while still reporting it', () => {
    setFocusedChatSession('chat-a');
    showExtensionLoadResults([ok('medcp'), ok('cdwagent'), bad('knowledge')], 'chat-a');

    const [statuses, total, , builtinFailures] = vi.mocked(toastService.extensionLoading).mock
      .calls[0];
    expect(total).toBe(2);
    expect(statuses.map((s) => s.name)).toEqual(['medcp', 'cdwagent']);
    expect(builtinFailures).toEqual(['knowledge']);
  });

  /**
   * The single-error fast path renders ONE extension and returns. Taking it
   * while a capability had also failed would drop that failure entirely.
   */
  it('does not take the lone-failure shortcut when a built-in also failed', () => {
    setFocusedChatSession('chat-a');
    showExtensionLoadResults([bad('cdwagent'), bad('knowledge')], 'chat-a');

    expect(toastService.error).not.toHaveBeenCalled();
    const [statuses, , , builtinFailures] = vi.mocked(toastService.extensionLoading).mock.calls[0];
    expect(statuses.map((s) => s.name)).toEqual(['cdwagent']);
    expect(builtinFailures).toEqual(['knowledge']);
  });
});

/**
 * Defect 1 (2026-09-12). "1 of 2 extensions loaded / Failed: Cdwagent"
 * reappeared after EVERY renderer load once dismissed. The only caller of
 * `showExtensionLoadResults` is `/agent/resume`, which a renderer load always
 * performs — so the toast announced something the user did not do, and
 * announced it again every time. The failure is real news exactly once; after
 * that it belongs on a persistent surface, not in the transient layer.
 */
describe('a failure is announced once, not on every renderer load', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetExtensionToastState();
    resetExtensionLoadFailuresForTests();
  });

  it('stays silent for a failure the user has already been shown', () => {
    showExtensionLoadResults([ok('medcp'), bad('cdwagent')], 'chat-a');
    expect(toastService.extensionLoading).toHaveBeenCalledTimes(1);

    // A renderer reload: every module-level toast latch is fresh, and
    // `/agent/resume` reports the same failure again.
    vi.clearAllMocks();
    resetExtensionToastState();
    showExtensionLoadResults([ok('medcp'), bad('cdwagent')], 'chat-a');

    expect(toastService.extensionLoading).not.toHaveBeenCalled();
  });
});
