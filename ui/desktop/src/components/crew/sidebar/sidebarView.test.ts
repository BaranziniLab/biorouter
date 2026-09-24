import { describe, expect, it } from 'vitest';
import type { PendingJoin, Snapshot } from '../crewApi';
import { waitingToJoin } from './sidebarView';

describe('waitingToJoin', () => {
  const snapshot = (pending_joins: unknown): Pick<Snapshot, 'pending_joins'> => ({
    pending_joins: pending_joins as PendingJoin[],
  });

  it('marks a join whose invitation ran out as expired, from the broker’s flag alone', () => {
    const rows = waitingToJoin(
      snapshot([
        { username: 'bob', full_name: 'Bob Lee', approved: true, expired: true },
        // The local clock is not asked: an `expires_at` in the past is the broker's to judge.
        { username: 'erin', expires_at: 1 },
        { username: 'finn', expired: 'yes' },
      ])
    );
    expect(rows.map((row) => [row.username, row.expired])).toEqual([
      ['bob', true],
      ['erin', false],
      ['finn', false],
    ]);
    expect(rows[0]).toMatchObject({ approved: true, serverName: 'Bob Lee' });
  });

  it('keeps only joins it can name, and nothing without the host’s list', () => {
    expect(waitingToJoin(null)).toEqual([]);
    expect(waitingToJoin(snapshot(undefined))).toEqual([]);
    expect(
      waitingToJoin(snapshot([null, { full_name: 'No username' }, { username: 'dee' }]))
    ).toEqual([
      {
        username: 'dee',
        serverName: null,
        approved: false,
        expired: false,
        otherDeviceTried: false,
      },
    ]);
  });
});
