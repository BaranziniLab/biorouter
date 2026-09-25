import { describe, expect, it } from 'vitest';
import type { PendingJoin, Snapshot } from '../crewApi';
import { knownUsername, loginLabel, serverLabel, waitingToJoin } from './sidebarView';

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

describe('the server by the person’s own name for it (D-ALIAS)', () => {
  it('prefers the daemon’s label, and keeps the raw address out of both', () => {
    const saved = { ssh_target: 'crew_alice@52.33.141.141', server_label: 'lab-server' };
    expect(serverLabel(saved)).toBe('lab-server');
    expect(loginLabel(saved)).toBe('crew_alice@lab-server');
  });

  it('falls back to the saved login, exactly as saved, from a daemon that sends no label', () => {
    const saved = { ssh_target: 'alice@hpc.ucsf.edu' };
    expect(serverLabel(saved)).toBe('hpc.ucsf.edu');
    expect(loginLabel(saved)).toBe('alice@hpc.ucsf.edu');
    expect(serverLabel({ ssh_target: 'alice@hpc.ucsf.edu', server_label: '' })).toBe(
      'hpc.ucsf.edu'
    );
    expect(serverLabel({ ssh_target: 'alice@hpc.ucsf.edu', server_label: 42 })).toBe(
      'hpc.ucsf.edu'
    );
  });

  it('names an alias login by the alias alone', () => {
    const saved = { ssh_target: 'lab-server', server_label: 'lab-server' };
    expect(serverLabel(saved)).toBe('lab-server');
    expect(loginLabel(saved)).toBe('lab-server');
  });

  it('says nothing for no connection', () => {
    expect(serverLabel(null)).toBe('');
    expect(loginLabel(undefined)).toBe('');
  });
});

describe('knownUsername', () => {
  it('takes the verified person first, then the login, then what the join remembered', () => {
    const login = { ssh_target: 'crew_frank@52.33.141.141' };
    expect(knownUsername({ username: 'frank' }, login, 'other')).toBe('frank');
    expect(knownUsername(null, login, 'other')).toBe('crew_frank');
    expect(knownUsername(null, { ssh_target: 'lab-server' }, 'crew_frank')).toBe('crew_frank');
    expect(knownUsername(null, { ssh_target: 'lab-server' }, null)).toBeNull();
  });
});
