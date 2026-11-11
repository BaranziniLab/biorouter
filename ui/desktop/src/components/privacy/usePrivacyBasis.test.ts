import { renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

/** DR-33: why a chat is public is read from the single-session read, only while it is public. */

const mocks = vi.hoisted(() => ({ getSession: vi.fn() }));
vi.mock('../../api', () => ({ getSession: mocks.getSession }));
vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'proof' }),
}));

import { usePrivacyBasis } from './usePrivacyBasis';

const BASIS = {
  kind: 'decisionModel',
  source: 'monitor',
  label: 'Done',
  model: 'joey-1',
  host: { kind: 'baseten' },
  place: 'Baseten',
  summary: 'monitor "Done" uses joey-1, a Baseten model',
};

beforeEach(() => {
  mocks.getSession.mockReset().mockResolvedValue({ data: { id: 's1', privacyBasis: BASIS } });
});

describe('usePrivacyBasis', () => {
  it('reads it with the person’s proof for a public chat', async () => {
    const { result } = renderHook(() => usePrivacyBasis('s1', 'public', 0));
    await waitFor(() => expect(result.current?.model).toBe('joey-1'));
    expect(mocks.getSession).toHaveBeenCalledWith({
      path: { session_id: 's1' },
      headers: { 'X-User-Action': 'proof' },
    });
  });

  it('asks nothing of a private chat, or of a chat that does not exist yet', async () => {
    renderHook(() => usePrivacyBasis('s1', 'private', 0));
    renderHook(() => usePrivacyBasis(undefined, 'public', 0));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(mocks.getSession).not.toHaveBeenCalled();
  });

  it('reads again when what can move it changes, and a failed read shows none', async () => {
    const { result, rerender } = renderHook(
      ({ revision }) => usePrivacyBasis('s1', 'public', revision),
      {
        initialProps: { revision: 1 },
      }
    );
    await waitFor(() => expect(result.current).not.toBeNull());
    mocks.getSession.mockRejectedValueOnce(new Error('offline'));
    rerender({ revision: 2 });
    await waitFor(() => expect(result.current).toBeNull());
    expect(mocks.getSession).toHaveBeenCalledTimes(2);
  });
});
