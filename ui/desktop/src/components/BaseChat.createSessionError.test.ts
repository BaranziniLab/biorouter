import { readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, it, expect, vi, beforeEach } from 'vitest';

// Capture the toast without rendering it.
const mockToastError = vi.fn();
vi.mock('../toasts', () => ({
  toastError: (...args: unknown[]) => mockToastError(...args),
  toastSuccess: vi.fn(),
}));

import { handleCreateSessionError } from './BaseChat';

const restoreEvents = (dispatch: ReturnType<typeof vi.spyOn>): CustomEvent[] =>
  dispatch.mock.calls
    .map((c: unknown[]) => c[0] as CustomEvent)
    .filter((e: CustomEvent) => e?.type === 'restore-chat-input');

describe('handleCreateSessionError', () => {
  beforeEach(() => vi.clearAllMocks());

  it('says the backend is unreachable, and answers false so the composer hands it back', () => {
    const dispatch = vi.spyOn(window, 'dispatchEvent');

    // `false` is ChatInput's "not taken": the composer gives the whole message
    // back through its own identity. That is what the toast's claim rests on.
    expect(handleCreateSessionError(new TypeError('Failed to fetch'))).toBe(false);

    expect(mockToastError).toHaveBeenCalledTimes(1);
    expect(mockToastError).toHaveBeenCalledWith(
      expect.objectContaining({ title: 'Backend disconnected' })
    );
    expect(mockToastError.mock.calls[0][0].msg).toContain('Your message was kept');

    dispatch.mockRestore();
  });

  it('broadcasts NOTHING — a new chat has no id, and every new tab heard `""`', () => {
    // Measured on 1.90.4: this function's `restore-chat-input` with `sessionId:
    // ''` filled BOTH panes' new-tab composers after a failure in one of them,
    // replacing the other's unsent draft.
    const dispatch = vi.spyOn(window, 'dispatchEvent');

    handleCreateSessionError(new Error('HTTP 500 Internal Server Error'));

    expect(restoreEvents(dispatch)).toHaveLength(0);
    expect(mockToastError).toHaveBeenCalledWith(
      expect.objectContaining({ title: 'Failed to start chat' })
    );

    dispatch.mockRestore();
  });
});

/**
 * `BaseChatContent` cannot be mounted here (react-router plus a dozen contexts),
 * so the two wiring facts the fix depends on are asserted AT THE SOURCE, the
 * idiom `BaseChat.initialMessage.test.ts` already uses.
 */
describe('the pre-session submit is wired to the give-back', () => {
  const source = readFileSync(path.join(process.cwd(), 'src/components/BaseChat.tsx'), 'utf8');

  it('resolves the failure with handleCreateSessionError’s false, not true', () => {
    const start = source.indexOf('const handleFormSubmit = async');
    const end = source.indexOf('submitAndReturnToBottom(', start);
    expect(start).toBeGreaterThan(-1);
    expect(end).toBeGreaterThan(start);
    const preSession = source.slice(start, end);
    expect(preSession).toMatch(/catch \(err\) \{[\s\S]*?return handleCreateSessionError\(err\);/);
  });

  it('addresses new-chat drafts by tab and existing-chat drafts by tab plus session', () => {
    expect(source).toContain('draftKey={inputDraftKey}');
    expect(source).toContain('existingChatComposerDraftKey(terminalKey, sessionId)');
    expect(source).toMatch(
      /const composerDraftKey = terminalKey \? composerDraftKeyForTab\(terminalKey\) : undefined;/
    );
  });
});
