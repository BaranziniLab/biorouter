/**
 * DR-33, owner decision 3: a chat's hover note ("Public: its decision model
 * joey-1 runs on Baseten") comes from the sidebar page, so it is only as fresh
 * as the last page read. Saving a chat's decision settings may write or clear
 * the record the note reads, and it is activity (D-3), so the chat also sorts
 * first. Nothing the list already listens to hears that save, so the save asks
 * for a head read with `refreshSidebarChats`. These drive the real list hook
 * against a fake daemon that does what the save does to the row.
 */
import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { SessionSummary } from '../../api';
import useSidebarSessions, { refreshSidebarChats } from './useSidebarSessions';

const daemon = vi.hoisted(() => ({
  rows: [] as SessionSummary[],
  listSidebarSessions: vi.fn(),
}));

vi.mock('../../api', () => ({
  listSidebarSessions: daemon.listSidebarSessions,
}));

vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-proof' }),
}));

/** The daemon's order: `updated_at`, newest first. */
function headOf(limit = 10) {
  const sorted = [...daemon.rows].sort(
    (left, right) => Date.parse(right.updated_at) - Date.parse(left.updated_at)
  );
  return {
    data: {
      sessions: sorted.slice(0, limit).map((row) => ({ ...row })),
      has_more: sorted.length > limit,
      next_cursor: sorted.length > limit ? 'next' : null,
    },
  };
}

function chat(letter: string, minutesAgo: number): SessionSummary {
  const at = new Date(Date.now() - minutesAgo * 60_000).toISOString();
  return {
    id: `session-${letter}`,
    name: `Chat ${letter}`,
    working_dir: '/workspace',
    created_at: at,
    updated_at: at,
    message_count: 2,
    user_set_name: false,
    privacy_tier: 'public',
  };
}

/** What a decision-settings save does to the row, as the next page reads it. */
function saveMonitors(id: string, publicBecause: SessionSummary['public_because']) {
  const row = daemon.rows.find((candidate) => candidate.id === id);
  if (!row) throw new Error(`${id} is not stored`);
  row.updated_at = new Date().toISOString();
  if (publicBecause) row.public_because = publicBecause;
  else delete row.public_because;
}

beforeEach(() => {
  vi.clearAllMocks();
  daemon.rows = [chat('a', 10), chat('b', 20), chat('c', 30)];
  daemon.listSidebarSessions.mockImplementation(async ({ query }: { query: { limit: number } }) =>
    headOf(query.limit)
  );
});

async function mountList() {
  const hook = renderHook(() => useSidebarSessions());
  await waitFor(() => expect(hook.result.current.sessions).toHaveLength(3));
  return hook;
}

describe('refreshSidebarChats', () => {
  it('brings in the note a decision-settings save wrote, and the chat sorts first', async () => {
    const hook = await mountList();
    const reads = daemon.listSidebarSessions.mock.calls.length;

    saveMonitors('session-c', { model: 'joey-1', place: 'Baseten' });
    act(() => refreshSidebarChats());

    await waitFor(() =>
      expect(hook.result.current.sessions.find((row) => row.id === 'session-c')).toMatchObject({
        public_because: { model: 'joey-1', place: 'Baseten' },
      })
    );
    // One debounced head read, with the person's proof like every page.
    expect(daemon.listSidebarSessions).toHaveBeenCalledTimes(reads + 1);
    expect(daemon.listSidebarSessions.mock.calls[reads][0]).toMatchObject({
      headers: { 'X-User-Action': 'test-proof' },
    });
    const row = hook.result.current.sessions.find((candidate) => candidate.id === 'session-c');
    const newest = Math.max(
      ...hook.result.current.sessions.map((candidate) => Date.parse(candidate.updated_at))
    );
    expect(Date.parse(row!.updated_at)).toBe(newest);
  });

  it('takes the note away when a save cleared the record', async () => {
    daemon.rows[1].public_because = { model: 'joey-1', place: 'Baseten' };
    const hook = await mountList();
    expect(
      hook.result.current.sessions.find((row) => row.id === 'session-b')?.public_because
    ).toEqual({ model: 'joey-1', place: 'Baseten' });

    saveMonitors('session-b', null);
    act(() => refreshSidebarChats());

    await waitFor(() =>
      expect(
        hook.result.current.sessions.find((row) => row.id === 'session-b')?.public_because
      ).toBeUndefined()
    );
  });

  it('coalesces a burst into one read, and reads nothing once the list is gone', async () => {
    const hook = await mountList();
    const reads = daemon.listSidebarSessions.mock.calls.length;

    act(() => {
      refreshSidebarChats();
      refreshSidebarChats();
      refreshSidebarChats();
    });
    await waitFor(() => expect(daemon.listSidebarSessions).toHaveBeenCalledTimes(reads + 1));

    hook.unmount();
    act(() => refreshSidebarChats());
    await new Promise((resolve) => setTimeout(resolve, 400));
    expect(daemon.listSidebarSessions).toHaveBeenCalledTimes(reads + 1);
  });
});
