import { describe, expect, it } from 'vitest';
import type { CrewSessionGrant } from '../api/grants';
import {
  accessRows,
  accessStatusOf,
  agentAccessCount,
  chatTitleOf,
  formatExpiry,
  grantKind,
  splitAccessRows,
} from './accessRows';
import { accessCopy } from './copy';

const NOW = Date.UTC(2026, 8, 23, 20, 0, 0);
const inAnHour = NOW / 1000 + 3600;

function grant(overrides: Partial<CrewSessionGrant> = {}): CrewSessionGrant {
  return {
    session_id: 'chat-1',
    run_id: 'run-1',
    connection_id: 'conn-1',
    channel_id: 'channel-1',
    source_channels: ['channel-1'],
    policy_epoch: 1,
    expired: false,
    kind: 'chat',
    session_name: 'Plot review',
    expires_at: inAnHour,
    ...overrides,
  };
}

const snapshot = {
  teams: [
    { id: 'team-1', name: 'Analysis Lab' },
    { id: 'team-2', name: 'Imaging' },
  ],
  channels: [
    { id: 'channel-1', team_id: 'team-1', name: 'methods' },
    { id: 'channel-2', team_id: 'team-1', name: 'raw-data' },
    { id: 'channel-3', team_id: 'team-2', name: 'methods' },
  ],
};

const run = (overrides: Record<string, string> = {}) => ({
  run_id: 'run-9',
  channel_id: 'channel-1',
  session_id: 'task-1',
  status: 'running',
  ...overrides,
});

describe('grant kind and title', () => {
  it('takes the daemon’s kind, else classifies by an owned run for the same session', () => {
    expect(grantKind(grant({ kind: 'task' }))).toBe('task');
    expect(grantKind(grant({ kind: undefined, session_id: 'task-1' }), [run()])).toBe('task');
    expect(grantKind(grant({ kind: undefined }), [run()])).toBe('chat');
  });

  it('never shows an ID-shaped or empty session name as a title', () => {
    expect(chatTitleOf({ session_name: 'Plot review' })).toBe('Plot review');
    expect(chatTitleOf({ session_name: '0b8f3c2e-4f7a-4c1e-9a55-1d2b3c4d5e6f' })).toBeNull();
    expect(chatTitleOf({ session_name: '   ' })).toBeNull();
    expect(chatTitleOf({ session_name: null })).toBeNull();
  });
});

describe('status', () => {
  it('reads an active grant as Expires {time}, or Active without an expiry', () => {
    expect(accessStatusOf(grant(), NOW)).toEqual({
      status: 'active',
      label: accessCopy.status.expires(formatExpiry(inAnHour, NOW)),
    });
    expect(accessStatusOf(grant({ expires_at: undefined }), NOW)).toEqual({
      status: 'active',
      label: accessCopy.status.active,
    });
  });

  it('reads a grant past its expiry as Expired, and a locally stopped one as Revoked', () => {
    expect(accessStatusOf(grant({ expires_at: NOW / 1000 - 1 }), NOW).status).toBe('expired');
    expect(accessStatusOf(grant({ expired: true }), NOW).status).toBe('revoked');
  });

  it('reads a local stop this window saw unconfirmed as Stopped on this device', () => {
    expect(accessStatusOf(grant({ expired: true }), NOW, true)).toEqual({
      status: 'unconfirmed',
      label: accessCopy.status.unconfirmed,
    });
    // A grant active again (granted anew) is active whatever this window remembers.
    expect(accessStatusOf(grant(), NOW, true).status).toBe('active');
  });

  it('formats a same-day expiry as a time and another day with its date', () => {
    expect(formatExpiry(inAnHour, NOW)).not.toMatch(/Sep/);
    expect(formatExpiry(NOW / 1000 + 3 * 86400, NOW)).toMatch(/Sep/);
  });
});

describe('rows', () => {
  it('names chats, tasks and destinations, never IDs, and counts extra sources', () => {
    const rows = accessRows(
      [
        grant({ source_channels: ['channel-1', 'channel-2', 'channel-2'] }),
        grant({ session_id: 'task-1', kind: 'task', session_name: 'Crew task', run_id: 'run-9' }),
        grant({
          session_id: 'chat-2',
          session_name: null,
          channel_id: 'gone',
          source_channels: ['gone'],
        }),
      ],
      { snapshot, runs: [run()], now: NOW }
    );
    expect(rows.map((row) => [row.title, row.destination, row.extraSources])).toEqual([
      ['Plot review', 'Analysis Lab / #methods', 1],
      ['Untitled chat', accessCopy.unknownChannel, 0],
      ['Your task', 'Analysis Lab / #methods', 0],
    ]);
    for (const row of rows) {
      expect(row.title).not.toMatch(/chat-|task-|run-/);
      expect(row.destination).not.toMatch(/channel-/);
    }
  });

  it('offers Revoke on active chats, Stop on running tasks, and Retry on an unconfirmed stop', () => {
    const rows = accessRows(
      [
        grant({ session_id: 'a', session_name: 'A', expires_at: inAnHour + 50 }),
        grant({ session_id: 'task-1', kind: 'task', expires_at: inAnHour + 40 }),
        grant({ session_id: 'task-2', kind: 'task', expires_at: inAnHour + 30 }),
        grant({ session_id: 'b', session_name: 'B', expired: true }),
        grant({ session_id: 'c', session_name: 'C', expired: true }),
      ],
      {
        snapshot,
        runs: [run(), run({ session_id: 'task-2', run_id: 'run-10', status: 'completed' })],
        now: NOW,
        isUnconfirmed: (_connection, session) => session === 'c',
      }
    );
    const bySession = (id: string) => rows.find((row) => row.sessionId === id);
    expect(bySession('a')).toMatchObject({ canRevoke: true, canStop: false, canRetry: false });
    expect(bySession('task-1')).toMatchObject({ canRevoke: false, canStop: true });
    expect(bySession('task-2')).toMatchObject({ canRevoke: false, canStop: false });
    expect(bySession('b')).toMatchObject({ status: 'revoked', canRevoke: false, canRetry: false });
    expect(bySession('c')).toMatchObject({ status: 'unconfirmed', canRetry: true });
  });

  it('puts unconfirmed stops first, then active (newest first), then the rest', () => {
    const rows = accessRows(
      [
        grant({ session_id: 'old', expired: true }),
        grant({ session_id: 'older-active', expires_at: inAnHour - 600 }),
        grant({ session_id: 'newer-active', expires_at: inAnHour }),
        grant({ session_id: 'stopped', expired: true }),
      ],
      { snapshot, now: NOW, isUnconfirmed: (_c, session) => session === 'stopped' }
    );
    expect(rows.map((row) => row.sessionId)).toEqual([
      'stopped',
      'newer-active',
      'older-active',
      'old',
    ]);
    const { current, old } = splitAccessRows(rows);
    expect(current.map((row) => row.sessionId)).toEqual([
      'stopped',
      'newer-active',
      'older-active',
    ]);
    expect(old.map((row) => row.sessionId)).toEqual(['old']);
  });

  it('keeps a channel’s rows: those posting in it and those reading it', () => {
    const rows = accessRows(
      [
        grant({ session_id: 'posts' }),
        grant({
          session_id: 'reads',
          channel_id: 'channel-2',
          source_channels: ['channel-2', 'channel-1'],
        }),
        grant({ session_id: 'elsewhere', channel_id: 'channel-2', source_channels: ['channel-2'] }),
      ],
      { snapshot, now: NOW, channelId: 'channel-1' }
    );
    expect(rows.map((row) => row.sessionId).sort()).toEqual(['posts', 'reads']);
  });
});

describe('the header chip’s count', () => {
  it('counts active chats posting here, and says chats', () => {
    expect(
      agentAccessCount({
        grants: [
          grant(),
          grant({ session_id: 'chat-2' }),
          grant({ session_id: 'x', expired: true }),
        ],
        channelId: 'channel-1',
        now: NOW,
      })
    ).toEqual({
      chats: 2,
      tasks: 0,
      total: 2,
      label: '2 chats',
      accessibleName: '2 chats or agents can post here',
    });
  });

  it('counts a running task once, from its run or its grant, and says tasks', () => {
    const count = agentAccessCount({
      grants: [
        grant({ session_id: 'task-1', kind: 'task' }),
        grant({ session_id: 'task-3', kind: 'task' }),
      ],
      runs: [run(), run({ session_id: 'task-4', run_id: 'r4', status: 'waiting_for_approval' })],
      channelId: 'channel-1',
      now: NOW,
    });
    expect(count).toMatchObject({ chats: 0, tasks: 3, label: '3 tasks' });
  });

  it('leaves out finished tasks, other channels and ended grants, and says agents when mixed', () => {
    const count = agentAccessCount({
      grants: [
        grant(),
        grant({ session_id: 'task-2', kind: 'task' }),
        grant({ session_id: 'late', expires_at: NOW / 1000 - 5 }),
        grant({ session_id: 'other', channel_id: 'channel-2' }),
      ],
      runs: [run(), run({ session_id: 'task-2', run_id: 'r2', status: 'completed' })],
      channelId: 'channel-1',
      now: NOW,
    });
    expect(count).toMatchObject({ chats: 1, tasks: 1, total: 2, label: '2 agents' });
    expect(count.accessibleName).toBe('2 chats or agents can post here');
  });

  it('is empty when nothing can post', () => {
    expect(agentAccessCount({ grants: [], channelId: 'channel-1', now: NOW })).toEqual({
      chats: 0,
      tasks: 0,
      total: 0,
      label: '',
      accessibleName: '',
    });
    expect(agentAccessCount({ grants: [grant()], channelId: 'channel-1', now: NOW }).label).toBe(
      '1 chat'
    );
    expect(
      agentAccessCount({ grants: [grant()], channelId: 'channel-1', now: NOW }).accessibleName
    ).toBe('1 chat or agent can post here');
  });
});
