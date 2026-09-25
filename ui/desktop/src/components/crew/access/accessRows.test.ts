import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { CrewSessionGrant } from '../api/grants';
import {
  accessRows,
  accessStatusOf,
  agentAccessCount,
  chatTitleOf,
  formatExpiry,
  grantKind,
  splitAccessRows,
  taskFirstWords,
  taskStartedAt,
} from './accessRows';
import { accessCopy } from './copy';
import {
  PAST_ACCESS_LIMIT,
  pastAccessStorageKey,
  readPastAccess,
  rememberConfirmedRevoke,
  rememberPastAccess,
  type PastAccessEntry,
} from './pastAccess';

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

  it('names a channel the snapshot does not show as the person saw it when granting', () => {
    const rows = accessRows(
      [
        grant({
          session_id: 'recorded',
          channel_id: 'gone',
          source_channels: ['gone'],
          labels: { destination: { channel_id: 'gone', label: '#old-methods' } },
        }),
        grant({
          session_id: 'shown',
          session_name: 'Shown chat',
          labels: { destination: { channel_id: 'channel-1', label: '#renamed-since' } },
        }),
        grant({
          session_id: 'mislabelled',
          session_name: 'Mislabelled chat',
          channel_id: 'gone',
          source_channels: ['gone'],
          labels: { destination: { channel_id: 'channel-9', label: '#someone-else' } },
        }),
      ],
      { snapshot, now: NOW }
    );
    const destination = (sessionId: string) =>
      rows.find((row) => row.sessionId === sessionId)?.destination;
    expect(destination('recorded')).toBe('#old-methods');
    // The snapshot's current name wins over the name recorded at grant time.
    expect(destination('shown')).toBe('Analysis Lab / #methods');
    expect(destination('mislabelled')).toBe(accessCopy.unknownChannel);
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

/**
 * Q2-09 and Q2-74 (live QA round 2): a task's grant ends with the task, and the Access history
 * listed it as "Revoked", in two rows reading "Your task · #general" that could not be told apart.
 */
describe('task rows in the Access history', () => {
  const TASK_NAME = 'Crew · #general · Please work out the sum and the average of each number…';
  const task = (overrides: Partial<CrewSessionGrant> = {}) =>
    grant({ session_id: 'task-1', kind: 'task', session_name: TASK_NAME, ...overrides });

  it('labels a task whose access is over "Ended", never "Revoked" or "Expired"', () => {
    const rows = accessRows(
      [
        task({ expired: true }),
        task({ session_id: 'task-2', expires_at: NOW / 1000 - 5 }),
        grant({ session_id: 'chat-revoked', expired: true }),
        grant({ session_id: 'chat-expired', expires_at: NOW / 1000 - 5 }),
      ],
      { snapshot, now: NOW }
    );
    const label = (id: string) => rows.find((row) => row.sessionId === id)?.statusLabel;
    expect(label('task-1')).toBe('Ended');
    expect(label('task-2')).toBe(accessCopy.status.ended);
    // A chat's access still reads as what happened to it.
    expect(label('chat-revoked')).toBe(accessCopy.status.revoked);
    expect(label('chat-expired')).toBe(accessCopy.status.expired);
    // The state itself is unchanged: an ended task still folds behind "Show past access".
    expect(splitAccessRows(rows).old.map((row) => row.sessionId)).toContain('task-1');
  });

  it('keeps a task stopped only on this device as a stop to confirm, with Retry', () => {
    const [row] = accessRows([task({ expired: true })], {
      snapshot,
      now: NOW,
      isUnconfirmed: () => true,
    });
    expect(row).toMatchObject({ status: 'unconfirmed', canRetry: true });
    expect(row.statusLabel).toBe(accessCopy.status.unconfirmed);
  });

  it('tells two tasks apart by when they started and their first words', () => {
    const started = NOW - 2 * 3600 * 1000;
    const rows = accessRows(
      [
        task({ expired: true, run_id: 'run-a' }),
        task({
          session_id: 'task-2',
          run_id: 'run-b',
          expired: true,
          session_name: 'Crew · #general · Plot counts',
          expires_at: NOW / 1000 - 1800,
        }),
      ],
      {
        snapshot,
        now: NOW,
        runs: [run({ session_id: 'task-1', run_id: 'run-a', status: 'completed' })].map((item) => ({
          ...item,
          started_at: started,
        })),
      }
    );
    const detail = (id: string) => rows.find((row) => row.sessionId === id)?.detail;
    // The run's own start, when the observer reported it.
    expect(detail('task-1')).toBe(`${formatExpiry(started / 1000, NOW)} · Please work out…`);
    // Else the grant's end less the hour it lasts.
    expect(detail('task-2')).toBe(`${formatExpiry(NOW / 1000 - 5400, NOW)} · Plot counts`);
    expect(detail('task-1')).not.toBe(detail('task-2'));
    for (const row of rows) expect(row.title).toBe(accessCopy.yourTask);
  });

  it('gives a chat row no detail: its title already names it', () => {
    const [row] = accessRows([grant()], { snapshot, now: NOW });
    expect(row.detail).toBeNull();
  });

  it('reads the first words from the task conversation’s title, and nothing else', () => {
    expect(taskFirstWords(TASK_NAME)).toBe('Please work out…');
    expect(taskFirstWords('Crew · #general · Plot counts')).toBe('Plot counts');
    expect(taskFirstWords('Crew · #general · Summarize…')).toBe('Summarize…');
    expect(taskFirstWords('Crew · #general · a · b · c d')).toBe('a · b…');
    // Before admission, without a prompt, renamed since, or ID-shaped: none.
    expect(taskFirstWords('Crew task')).toBeNull();
    expect(taskFirstWords('Crew · #general')).toBeNull();
    expect(taskFirstWords('Plate reader sums')).toBeNull();
    expect(taskFirstWords('Crew · #general · 0b8f3c2e-4f7a-4c1e-9a55-1d2b3c4d5e6f')).toBeNull();
    expect(taskFirstWords(null)).toBeNull();
  });

  it('dates a task by its run, else by its grant, else not at all', () => {
    expect(taskStartedAt({ expires_at: 5000 }, { started_at: 2_000_000 })).toBe(2000);
    expect(taskStartedAt({ expires_at: 5000 }, null)).toBe(1400);
    expect(taskStartedAt({ expires_at: null }, { started_at: undefined })).toBeNull();
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

/**
 * Q4-12 (live QA round 4): the daemon lists one grant per chat, so a chat revoked and then granted
 * again lost its revoked row, and "Show past access" forgot the earlier grant (Jack J6, Gina F9).
 * This device remembers each confirmed revoke and the list merges it in as "Revoked", unless the
 * daemon's list still holds the same run. Display only, bounded, and every storage access wrapped.
 */
describe('past access this device remembers', () => {
  beforeEach(() => {
    window.localStorage.clear();
    vi.restoreAllMocks();
  });

  const remembered = (overrides: Partial<PastAccessEntry> = {}): PastAccessEntry => ({
    session_id: 'chat-1',
    run_id: 'run-1',
    session_name: 'Plot review',
    channel_id: 'channel-1',
    revoked_at: NOW - 60_000,
    kind: 'chat',
    ...overrides,
  });
  const past = (entries: PastAccessEntry[]) => ({ connectionId: 'conn-1', entries });

  it('lists a remembered revoke as Revoked beside the chat’s new grant', () => {
    const rows = accessRows([grant({ run_id: 'run-2' })], {
      snapshot,
      now: NOW,
      pastAccess: past([remembered()]),
    });
    const { current, old } = splitAccessRows(rows);
    expect(current.map((row) => [row.title, row.runId, row.status])).toEqual([
      ['Plot review', 'run-2', 'active'],
    ]);
    expect(old).toHaveLength(1);
    const [revoked] = old;
    expect(revoked).toMatchObject({
      title: 'Plot review',
      runId: 'run-1',
      status: 'revoked',
      // Dated (F5): when this device saw the revoke confirmed.
      statusLabel: accessCopy.status.revokedAt(formatExpiry((NOW - 60_000) / 1000, NOW)),
      revokedAt: NOW - 60_000,
      destination: 'Analysis Lab / #methods',
      canRevoke: false,
      canRetry: false,
      canStop: false,
    });
    // Its own key: the chat is listed too, and React and the list's controls key rows by it.
    expect(new Set(rows.map((row) => row.key)).size).toBe(rows.length);
    expect(showOldLabel(rows)).toBe('Show past access (1)');
  });

  it('dates each revoke, so two revokes of the same chat read as two rows (F5)', () => {
    const first = NOW - 5 * 60_000;
    const second = NOW - 60_000;
    // The daemon still lists the chat's newest grant (run-2, revoked); run-1 only this device
    // remembers.
    const rows = accessRows([grant({ run_id: 'run-2', expired: true })], {
      snapshot,
      now: NOW,
      pastAccess: past([
        remembered({ run_id: 'run-2', revoked_at: second }),
        remembered({ run_id: 'run-1', revoked_at: first }),
      ]),
    });
    const { old } = splitAccessRows(rows);
    expect(old.map((row) => [row.runId, row.revokedAt])).toEqual([
      ['run-2', second],
      ['run-1', first],
    ]);
    const labels = old.map((row) => row.statusLabel);
    expect(labels).toEqual([
      accessCopy.status.revokedAt(formatExpiry(second / 1000, NOW)),
      accessCopy.status.revokedAt(formatExpiry(first / 1000, NOW)),
    ]);
    expect(new Set(labels).size).toBe(2);
    // A revoke this device never saw confirmed stays undated.
    const [plain] = accessRows([grant({ expired: true })], { snapshot, now: NOW });
    expect(plain.statusLabel).toBe(accessCopy.status.revoked);
    expect(plain.revokedAt).toBeNull();
  });

  it('adds nothing for a run the daemon’s list still holds', () => {
    const rows = accessRows([grant({ expired: true })], {
      snapshot,
      now: NOW,
      pastAccess: past([remembered()]),
    });
    expect(rows).toHaveLength(1);
    expect(rows[0].runId).toBe('run-1');
  });

  it('keeps only the channel’s remembered rows, and another connection’s list never hides them', () => {
    const rows = accessRows(
      [grant({ connection_id: 'conn-2', run_id: 'run-1', session_id: 'x' })],
      {
        snapshot,
        now: NOW,
        channelId: 'channel-1',
        pastAccess: past([
          remembered(),
          remembered({ run_id: 'run-3', session_id: 'chat-3', channel_id: 'channel-2' }),
        ]),
      }
    );
    expect(rows.filter((row) => row.connectionId === 'conn-1').map((row) => row.runId)).toEqual([
      'run-1',
    ]);
  });

  it('never reads a remembered revoke as stopped on this device', () => {
    const rows = accessRows([grant({ run_id: 'run-2', expired: true })], {
      snapshot,
      now: NOW,
      isUnconfirmed: () => true,
      pastAccess: past([remembered()]),
    });
    expect(rows.find((row) => row.runId === 'run-2')?.status).toBe('unconfirmed');
    expect(rows.find((row) => row.runId === 'run-1')?.status).toBe('revoked');
  });

  it('records a confirmed revoke once per run, newest first, under the connection’s key', () => {
    rememberConfirmedRevoke('conn-1', 'chat-1', grant(), { run_id: 'run-1' }, NOW);
    rememberConfirmedRevoke(
      'conn-1',
      'chat-2',
      grant({ session_id: 'chat-2', run_id: 'run-2' }),
      null,
      NOW + 1
    );
    rememberConfirmedRevoke('conn-1', 'chat-1', grant(), { run_id: 'run-1' }, NOW + 2);
    expect(readPastAccess('conn-1').map((entry) => [entry.run_id, entry.revoked_at])).toEqual([
      ['run-1', NOW + 2],
      ['run-2', NOW + 1],
    ]);
    expect(readPastAccess('conn-1')[0]).toMatchObject({
      session_id: 'chat-1',
      session_name: 'Plot review',
      channel_id: 'channel-1',
      kind: 'chat',
    });
    expect(window.localStorage.getItem(pastAccessStorageKey('conn-1'))).toContain('run-2');
    expect(pastAccessStorageKey('conn-1')).toBe('crew:pastAccess:v1:conn-1');
    expect(readPastAccess('conn-2')).toEqual([]);
  });

  it('takes the daemon’s run, and records nothing when the row described another run', () => {
    // Just granted in the pane: the row has no run yet, the daemon's answer names it.
    rememberConfirmedRevoke(
      'conn-1',
      'chat-1',
      { channel_id: 'channel-1' },
      { run_id: 'run-7' },
      NOW
    );
    expect(readPastAccess('conn-1').map((entry) => entry.run_id)).toEqual(['run-7']);
    // A listed row of an earlier run: its channel may not be this run's.
    rememberConfirmedRevoke(
      'conn-1',
      'chat-1',
      grant({ run_id: 'run-1' }),
      { run_id: 'run-8' },
      NOW
    );
    rememberConfirmedRevoke(
      'conn-1',
      'chat-1',
      grant(),
      { session_id: 'other', run_id: 'run-1' },
      NOW
    );
    rememberConfirmedRevoke('conn-1', 'chat-1', { run_id: 'run-9' }, null, NOW);
    expect(readPastAccess('conn-1').map((entry) => entry.run_id)).toEqual(['run-7']);
  });

  it(`keeps at most ${PAST_ACCESS_LIMIT} rows, dropping the oldest`, () => {
    for (let index = 0; index < PAST_ACCESS_LIMIT + 5; index += 1)
      rememberPastAccess('conn-1', remembered({ run_id: `run-${index}`, revoked_at: index }));
    const entries = readPastAccess('conn-1');
    expect(entries).toHaveLength(PAST_ACCESS_LIMIT);
    expect(entries[0].run_id).toBe(`run-${PAST_ACCESS_LIMIT + 4}`);
    expect(entries.some((entry) => entry.run_id === 'run-0')).toBe(false);
  });

  it('reads corrupt, malformed or oversized storage as nothing remembered', () => {
    const key = pastAccessStorageKey('conn-1');
    window.localStorage.setItem(key, '{not json');
    expect(readPastAccess('conn-1')).toEqual([]);
    window.localStorage.setItem(key, JSON.stringify({ run_id: 'run-1' }));
    expect(readPastAccess('conn-1')).toEqual([]);
    window.localStorage.setItem(
      key,
      JSON.stringify([
        null,
        { run_id: 'run-1' },
        { ...remembered(), session_id: 'x'.repeat(1000) },
        { ...remembered(), revoked_at: 'yesterday' },
        { ...remembered(), run_id: 'run-ok', session_name: 42, source_channels: ['c', 7] },
      ])
    );
    expect(readPastAccess('conn-1')).toEqual([
      {
        session_id: 'chat-1',
        run_id: 'run-ok',
        session_name: null,
        channel_id: 'channel-1',
        revoked_at: NOW - 60_000,
        kind: 'chat',
        source_channels: ['c'],
      },
    ]);
  });

  it('survives storage that throws on read and on write, or cannot be reached at all', () => {
    const throwing = {
      getItem: () => {
        throw new Error('blocked');
      },
      setItem: () => {
        throw new Error('full');
      },
    } as unknown as Storage;
    const access = vi.spyOn(window, 'localStorage', 'get').mockReturnValue(throwing);
    expect(() => rememberPastAccess('conn-1', remembered())).not.toThrow();
    expect(readPastAccess('conn-1')).toEqual([]);

    access.mockImplementation(() => {
      throw new Error('denied');
    });
    expect(() => rememberPastAccess('conn-1', remembered())).not.toThrow();
    expect(readPastAccess('conn-1')).toEqual([]);
    access.mockRestore();
    expect(readPastAccess('conn-1')).toEqual([]);
  });
});

/** The disclosure's label for a list of rows, as `AccessList` draws it. */
function showOldLabel(rows: ReturnType<typeof accessRows>): string {
  return accessCopy.showOld(splitAccessRows(rows).old.length);
}

/**
 * F3: the daemon's own word on a stopped grant wins over this window's memory of a 503 — it asks
 * the workspace again by itself, so a revoke seen unconfirmed here may since be confirmed.
 */
describe('a revoke waiting for the workspace, as a row (F3)', () => {
  it('follows the daemon’s word over this window’s memory', () => {
    const remembers = () => true;
    const forgets = () => false;
    const [waiting] = accessRows([grant({ expired: true, revocation: 'unconfirmed' })], {
      snapshot,
      now: NOW,
      isUnconfirmed: forgets,
    });
    expect(waiting).toMatchObject({ status: 'unconfirmed', canRetry: true });
    const [confirmed] = accessRows([grant({ expired: true, revocation: 'confirmed' })], {
      snapshot,
      now: NOW,
      isUnconfirmed: remembers,
    });
    expect(confirmed).toMatchObject({ status: 'revoked', canRetry: false });
    // A daemon without the word: this window's memory decides, as before.
    const [older] = accessRows([grant({ expired: true })], {
      snapshot,
      now: NOW,
      isUnconfirmed: remembers,
    });
    expect(older.status).toBe('unconfirmed');
  });
});
