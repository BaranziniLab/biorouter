import { renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { CrewTransfer } from '../crewTransfers';
import { CREW_CHANGED_DEBOUNCE_MS, dispatchCrewChanged } from '../state/useCrewChanges';
import { useCrewTransfers } from './useCrewTransfers';

/**
 * Transfer records follow Crew's change feed (WI-15): a file a chat shared or downloaded, or one
 * another window paused, shows here without waiting for a poll that nothing idle would run.
 */

const mocks = vi.hoisted(() => ({ listTransfers: vi.fn() }));

vi.mock('../crewTransfers', () => ({ listTransfers: mocks.listTransfers }));

const transfer = (overrides: Partial<CrewTransfer>): CrewTransfer => ({
  id: 'transfer-1',
  request_id: 'request-1',
  connection_id: 'conn-1',
  channel_id: 'channel-1',
  direction: 'upload',
  name: 'counts.csv',
  size: 2048,
  sha256: 'a'.repeat(64),
  offset: 2048,
  blob_id: 'blob-1',
  state: 'completed',
  error: null,
  ...overrides,
});

const changed = (kind: string, connectionId: string | null, revision = 1) => ({
  revision,
  truncated: false,
  changes: [
    {
      revision,
      kind,
      connection_id: connectionId,
      channel_id: 'channel-1',
      session_id: 'chat-1',
      run_id: null,
      job_id: null,
    },
  ],
});

const reads = (connectionId: string) =>
  mocks.listTransfers.mock.calls.filter(([id]) => id === connectionId).length;
const quietFor = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

beforeEach(() => {
  mocks.listTransfers.mockReset();
});

describe('useCrewTransfers and the change feed', () => {
  it('lists a file a chat shared, with one read for a burst', async () => {
    mocks.listTransfers.mockResolvedValue([]);
    const { result, unmount } = renderHook(() => useCrewTransfers('conn-1'));
    await waitFor(() => expect(result.current.loaded).toBe(true));
    expect(reads('conn-1')).toBe(1);

    mocks.listTransfers.mockResolvedValue([transfer({})]);
    dispatchCrewChanged(changed('files_share', 'conn-1', 1));
    dispatchCrewChanged(changed('send', 'conn-1', 2));
    await waitFor(() => expect(result.current.transfers).toHaveLength(1));
    await quietFor(CREW_CHANGED_DEBOUNCE_MS * 2);
    expect(reads('conn-1')).toBe(2);
    unmount();
  });

  it('reads nothing for a change on a connection it does not show', async () => {
    mocks.listTransfers.mockResolvedValue([]);
    const { result, unmount } = renderHook(() => useCrewTransfers('conn-1'));
    await waitFor(() => expect(result.current.loaded).toBe(true));
    dispatchCrewChanged(changed('files_share', 'conn-9'));
    await quietFor(CREW_CHANGED_DEBOUNCE_MS * 2);
    expect(reads('conn-1')).toBe(1);
    unmount();
  });

  it('stops listening once nothing shows a transfer', async () => {
    mocks.listTransfers.mockResolvedValue([]);
    const { result, unmount } = renderHook(() => useCrewTransfers('conn-1'));
    await waitFor(() => expect(result.current.loaded).toBe(true));
    unmount();
    dispatchCrewChanged(changed('files_share', 'conn-1'));
    await quietFor(CREW_CHANGED_DEBOUNCE_MS * 2);
    expect(reads('conn-1')).toBe(1);
  });
});
