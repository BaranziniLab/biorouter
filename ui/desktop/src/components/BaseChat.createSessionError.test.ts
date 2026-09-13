import { describe, it, expect, vi, beforeEach } from 'vitest';

// Capture the toast without rendering it.
const mockToastError = vi.fn();
vi.mock('../toasts', () => ({
  toastError: (...args: unknown[]) => mockToastError(...args),
  toastSuccess: vi.fn(),
}));

import { handleCreateSessionError } from './BaseChat';

const restoreEventFrom = (dispatch: ReturnType<typeof vi.spyOn>): CustomEvent | undefined =>
  dispatch.mock.calls
    .map((c: unknown[]) => c[0] as CustomEvent)
    .find((e: CustomEvent) => e?.type === 'restore-chat-input');

describe('handleCreateSessionError', () => {
  beforeEach(() => vi.clearAllMocks());

  it('preserves the typed message and shows a disconnected toast on a connection error', () => {
    const dispatch = vi.spyOn(window, 'dispatchEvent');

    const keep = vi.fn();
    handleCreateSessionError(new TypeError('Failed to fetch'), {
      textValue: 'analyze my cohort',
      attachments: [],
      sessionId: null,
      keep,
    });

    // (1) the text the composer already cleared is restored, not lost
    const restore = restoreEventFrom(dispatch);
    expect(restore).toBeTruthy();
    expect(restore!.detail).toMatchObject({ value: 'analyze my cohort', sessionId: null });

    // (2) and the surface, which outlives the composer, is given the durable
    // copy — the one a rebuilt composer reads. Without this the toast below
    // would be claiming something only the doomed composer could honour.
    expect(keep).toHaveBeenCalledWith({ sessionId: '', value: 'analyze my cohort' });

    // (3) a visible, connection-specific toast surfaces (no silent swallow)
    expect(mockToastError).toHaveBeenCalledTimes(1);
    expect(mockToastError).toHaveBeenCalledWith(
      expect.objectContaining({ title: 'Backend disconnected' })
    );

    dispatch.mockRestore();
  });

  it('still preserves text but shows a generic toast on a non-connection error', () => {
    const dispatch = vi.spyOn(window, 'dispatchEvent');

    const keep = vi.fn();
    handleCreateSessionError(new Error('HTTP 500 Internal Server Error'), {
      textValue: 'keep me',
      attachments: [],
      sessionId: 'sess-1',
      keep,
    });

    const restore = restoreEventFrom(dispatch);
    expect(restore!.detail).toMatchObject({ value: 'keep me', sessionId: 'sess-1' });
    // Named for the chat it was typed into, so the surface can refuse to hand
    // it to a composer for any other chat.
    expect(keep).toHaveBeenCalledWith({ sessionId: 'sess-1', value: 'keep me' });
    expect(mockToastError).toHaveBeenCalledWith(
      expect.objectContaining({ title: 'Failed to start chat' })
    );

    dispatch.mockRestore();
  });
});
