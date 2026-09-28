import { createElement } from 'react';
import { render } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { MessageBody } from '../timeline/MessageBody';
import {
  ATTENTION_POLL_MS,
  AttentionThrottle,
  CrewAttentionWatcher,
  NOTIFY_FETCH_LIMIT,
  attentionBadgeText,
  attentionNotice,
  mentionsUser,
  namesFrom,
  unreadCounts,
  unreadRises,
  unreadTotal,
  type AttentionWatcherDeps,
} from './crewAttention';

describe('unread counts', () => {
  it('keeps only real counts and sums them', () => {
    const counts = unreadCounts({ a: 3, b: 0, c: -1, d: 2.5, e: '4', f: 2 });
    expect([...counts]).toEqual([
      ['a', 3],
      ['f', 2],
    ]);
    expect(unreadTotal(counts)).toBe(5);
    expect(unreadCounts(null).size).toBe(0);
    expect(unreadCounts([1, 2]).size).toBe(0);
  });

  it('finds the channels that rose, and says nothing about a first look', () => {
    const before = new Map([
      ['a', 2],
      ['b', 5],
    ]);
    const after = new Map([
      ['a', 4],
      ['b', 1],
      ['c', 3],
    ]);
    expect(unreadRises(before, after)).toEqual([
      { channelId: 'a', added: 2 },
      { channelId: 'c', added: 3 },
    ]);
    expect(unreadRises(undefined, after)).toEqual([]);
  });

  it('caps the badge as the Crew sidebar does', () => {
    expect(attentionBadgeText(7)).toBe('7');
    expect(attentionBadgeText(99)).toBe('99');
    expect(attentionBadgeText(100)).toBe('99+');
  });
});

/**
 * One rule for "mentions you" (W2-SHL-2): every case runs through `mentionsUser` AND through the
 * timeline's `MessageBody`, and the two must agree with the expected answer. A notification that
 * says "mentioned you" while the channel marks nothing, or the reverse, fails here.
 */
describe('mentionsUser agrees with the timeline', () => {
  const USERNAME = 'crew_bob';
  const table: [string, boolean][] = [
    ['@crew_bob can you look?', true],
    ['thanks @Crew_Bob.', true],
    ['(@crew_bob)', true],
    ['ends with @crew_bob...', true],
    ['> @crew_bob in a quote', true],
    ['**@crew_bob** in bold', true],
    ['```\ncode\n```\n@crew_bob after the block', true],
    ['cc @crew_bobby', false],
    ['cc @crew_bob-x', false],
    // A longer name that happens to start with this one (usernames may hold a dot).
    ['@crew_bob.lee see this', false],
    ['mail crew_bob@lab.org', false],
    ['x@crew_bob', false],
    ['@@crew_bob', false],
    // A hidden character against the name makes it another name.
    ['@crew_bob\u200Bx', false],
    ['@cre\u200Bw_bob', false],
    ['\u202E@crew_bob', false],
    ['run `@crew_bob` literally', false],
    ['```\n@crew_bob in a block\n```\nno mention here', false],
    ['~~~\n@crew_bob in a block\n~~~', false],
    ['    @crew_bob in an indented block', false],
    ['[@crew_bob](https://www.ucsf.edu)', false],
    ['see https://example.org/@crew_bob for it', false],
    ['@crew_alice only', false],
  ];

  it.each(table)('%j mentions crew_bob: %s', (body, expected) => {
    expect(mentionsUser(body, USERNAME)).toBe(expected);
    const { container, unmount } = render(
      createElement(MessageBody, { body, mention: USERNAME, mentionLabelId: 'timeline-mention' })
    );
    expect(container.querySelector('#timeline-mention') !== null).toBe(expected);
    unmount();
  });

  it('never mentions an empty or invalid name, or a body that is not text', () => {
    expect(mentionsUser('@ hello', '')).toBe(false);
    expect(mentionsUser('@crew bob', 'crew bob')).toBe(false);
    expect(mentionsUser('@crew_bob', null)).toBe(false);
    expect(mentionsUser(42, 'crew_bob')).toBe(false);
  });
});

describe('attentionNotice', () => {
  const people = { p1: { username: 'crew_alice', display_name: 'Alice Chen' } };

  it('names who mentioned the person, and where, and never the message itself', () => {
    const notice = attentionNotice({
      workspace: 'chen-lab',
      channel: '#general',
      username: 'crew_bob',
      added: 2,
      messages: [
        { actor_id: 'p2', body: 'unrelated' },
        { actor_id: 'p1', body: '@crew_bob the secret results are in' },
      ],
      people,
    });
    expect(notice).toEqual({ title: 'Alice Chen mentioned you in #general', body: 'chen-lab' });
    expect(JSON.stringify(notice)).not.toContain('secret');
  });

  it('counts new messages otherwise, with the channel under the workspace', () => {
    expect(
      attentionNotice({
        workspace: 'chen-lab',
        channel: '#general',
        username: 'crew_bob',
        added: 3,
        messages: [],
      })
    ).toEqual({ title: '3 new messages in chen-lab', body: '#general' });
    expect(
      attentionNotice({
        workspace: 'chen-lab',
        channel: '#general',
        username: 'crew_bob',
        added: 1,
        messages: [{ actor_id: 'p1', body: 'hi' }],
      }).title
    ).toBe('1 new message in chen-lab');
  });

  it('looks for a mention only among the messages that are new', () => {
    const notice = attentionNotice({
      workspace: 'w',
      channel: '#c',
      username: 'crew_bob',
      added: 1,
      messages: [
        { actor_id: 'p1', body: '@crew_bob an old mention, already counted' },
        { actor_id: 'p1', body: 'the one new message' },
      ],
      people,
    });
    expect(notice.title).toBe('1 new message in w');
  });

  it("never reads the viewer's own words as mentioning them, as the timeline does not", () => {
    const own = { actor_id: 'p-bob', body: 'note to self @crew_bob' };
    const notice = (message: { actor_id: string; body: string; run_id?: string }) =>
      attentionNotice({
        workspace: 'w',
        channel: '#c',
        username: 'crew_bob',
        viewerId: 'p-bob',
        added: 1,
        messages: [message],
        people: { 'p-bob': { username: 'crew_bob', display_name: 'Bob' } },
      }).title;
    expect(notice(own)).toBe('1 new message in w');
    // Their agent's words may mention them.
    expect(notice({ ...own, run_id: 'run-1' })).toBe('Bob mentioned you in #c');
  });

  it('takes the viewer as the timeline does', () => {
    const names = namesFrom({
      workspace: { name: 'w' } as never,
      channels: [],
      actor: { id: 'p-bob', username: 'crew_bob' } as never,
    });
    expect([names.username, names.viewerId]).toEqual(['crew_bob', 'p-bob']);
    const nameless = namesFrom({
      workspace: { name: 'w' } as never,
      channels: [],
      actor: { username: '' } as never,
    });
    expect([nameless.username, nameless.viewerId]).toEqual([null, null]);
  });

  it('shows no hidden character a member put in a name', () => {
    const names = namesFrom({
      workspace: { name: 'chen\u202Elab' } as never,
      channels: [{ id: 'c1', name: 'gen\u200Beral' } as never],
      actor: { username: 'crew_bob' } as never,
    });
    expect(names.workspace).toBe('chenlab');
    expect(names.channel('c1')).toBe('#general');
    expect(names.username).toBe('crew_bob');
    expect(
      attentionNotice({
        workspace: names.workspace,
        channel: names.channel('c1'),
        username: 'crew_bob',
        added: 1,
        messages: [{ actor_id: 'p9', body: '@crew_bob' }],
        people: { p9: { username: 'mallory', display_name: 'Mal\u202Elory' } },
      }).title
    ).toBe('Mallory mentioned you in #general');
  });
});

describe('AttentionThrottle', () => {
  it('lets one notification per key through a minute', () => {
    const throttle = new AttentionThrottle(60_000);
    expect(throttle.allow('a', 0)).toBe(true);
    expect(throttle.allow('a', 30_000)).toBe(false);
    expect(throttle.allow('b', 30_000)).toBe(true);
    expect(throttle.allow('a', 60_000)).toBe(true);
  });
});

describe('CrewAttentionWatcher (M2)', () => {
  const flush = async () => {
    for (let index = 0; index < 10; index += 1) await Promise.resolve();
  };
  const snapshot = (unread: Record<string, number>) => ({
    workspace: { name: 'chen-lab' },
    channels: [
      { id: 'c-general', name: 'general' },
      { id: 'c-jobs', name: 'jobs' },
    ],
    actor: { username: 'crew_bob' },
    unread,
  });

  function harness(overrides: Partial<AttentionWatcherDeps> = {}) {
    const sleeps: (() => void)[] = [];
    let now = 0;
    let unread: Record<string, number> = { 'c-general': 1 };
    const base = {
      listConnections: vi.fn(async () => [
        { id: 'conn-1', status: 'connected' },
        { id: 'conn-2', status: 'disconnected' },
      ]),
      readSnapshot: vi.fn(async () => snapshot(unread)),
      readLatest: vi.fn(async () => ({
        messages: [{ actor_id: 'p1', body: '@crew_bob please check #jobs' }],
        people: { p1: { username: 'crew_alice', display_name: 'Alice Chen' } },
      })),
      notify: vi.fn(),
      onTotal: vi.fn(),
      attended: vi.fn(() => false),
      sleep: vi.fn(
        (ms: number) =>
          new Promise<void>((resolve) => {
            expect(ms).toBe(ATTENTION_POLL_MS);
            sleeps.push(resolve);
          })
      ),
      now: vi.fn(() => now),
    };
    // An override stands in for its mock at run time; the tests read it as the same mock.
    const deps = { ...base, ...overrides } as unknown as typeof base;
    const watcher = new CrewAttentionWatcher(deps);
    return {
      deps,
      watcher,
      setUnread: (next: Record<string, number>) => (unread = next),
      advance: async (ms = ATTENTION_POLL_MS) => {
        now += ms;
        sleeps.shift()?.();
        await flush();
      },
    };
  }

  it('reports the total of connected workspaces only, and announces nothing on the first look', async () => {
    const { deps, watcher } = harness();
    watcher.start();
    await flush();
    expect(deps.readSnapshot).toHaveBeenCalledTimes(1);
    expect(deps.readSnapshot).toHaveBeenCalledWith('conn-1', expect.anything());
    expect(deps.onTotal).toHaveBeenLastCalledWith(1);
    expect(deps.notify).not.toHaveBeenCalled();
    watcher.stop();
  });

  it('announces a mention that arrives while the person is elsewhere, reading only the new messages', async () => {
    const { deps, watcher, setUnread, advance } = harness();
    watcher.start();
    await flush();
    setUnread({ 'c-general': 3 });
    await advance();
    expect(deps.onTotal).toHaveBeenLastCalledWith(3);
    expect(deps.readLatest).toHaveBeenCalledWith('conn-1', 'c-general', 2, expect.anything());
    expect(deps.notify).toHaveBeenCalledWith({
      title: 'Alice Chen mentioned you in #general',
      body: 'chen-lab',
      key: 'conn-1:c-general',
      connectionId: 'conn-1',
      channelId: 'c-general',
    });
    watcher.stop();
  });

  it("does not call the viewer's own words a mention of them", async () => {
    const { deps, watcher, advance } = harness({
      readSnapshot: vi.fn(async () => ({
        ...snapshot({ 'c-general': 2 }),
        actor: { id: 'p-bob', username: 'crew_bob' },
      })),
      readLatest: vi.fn(async () => ({
        messages: [{ actor_id: 'p-bob', body: 'reminder for @crew_bob' }],
        people: { 'p-bob': { username: 'crew_bob', display_name: 'Bob' } },
      })),
    });
    watcher.start();
    await flush();
    deps.readSnapshot.mockImplementation(async () => ({
      ...snapshot({ 'c-general': 3 }),
      actor: { id: 'p-bob', username: 'crew_bob' },
    }));
    await advance();
    expect(deps.notify).toHaveBeenCalledWith(
      expect.objectContaining({ title: '1 new message in chen-lab', body: '#general' })
    );
    watcher.stop();
  });

  it('reads at most the bounded number of new messages', async () => {
    const { deps, watcher, setUnread, advance } = harness();
    watcher.start();
    await flush();
    setUnread({ 'c-general': 500 });
    await advance();
    expect(deps.readLatest).toHaveBeenCalledWith(
      'conn-1',
      'c-general',
      NOTIFY_FETCH_LIMIT,
      expect.anything()
    );
    expect(deps.notify).toHaveBeenCalledTimes(1);
    watcher.stop();
  });

  it('says nothing while the person is looking at Crew, but keeps the count', async () => {
    const { deps, watcher, setUnread, advance } = harness({ attended: vi.fn(() => true) });
    watcher.start();
    await flush();
    setUnread({ 'c-general': 4 });
    await advance();
    expect(deps.onTotal).toHaveBeenLastCalledWith(4);
    expect(deps.readLatest).not.toHaveBeenCalled();
    expect(deps.notify).not.toHaveBeenCalled();
    watcher.stop();
  });

  it('announces a channel at most once a minute', async () => {
    const { deps, watcher, setUnread, advance } = harness();
    watcher.start();
    await flush();
    setUnread({ 'c-general': 2 });
    await advance();
    setUnread({ 'c-general': 3 });
    await advance();
    expect(deps.notify).toHaveBeenCalledTimes(1);
    setUnread({ 'c-general': 4, 'c-jobs': 1 });
    await advance();
    // Another channel is its own.
    expect(deps.notify).toHaveBeenCalledTimes(2);
    expect(deps.notify.mock.calls[1][0]).toMatchObject({
      key: 'conn-1:c-jobs',
      title: 'Alice Chen mentioned you in #jobs',
    });
    await advance(60_000);
    setUnread({ 'c-general': 5, 'c-jobs': 1 });
    await advance();
    expect(deps.notify).toHaveBeenCalledTimes(3);
    watcher.stop();
  });

  it('still says something arrived when the new messages cannot be read', async () => {
    const { deps, watcher, setUnread, advance } = harness({
      readLatest: vi.fn(async () => {
        throw new Error('bridge down');
      }),
    });
    watcher.start();
    await flush();
    setUnread({ 'c-general': 2 });
    await advance();
    expect(deps.notify).toHaveBeenCalledWith(
      expect.objectContaining({ title: '1 new message in chen-lab', body: '#general' })
    );
    watcher.stop();
  });

  it('drops a workspace that is no longer connected from the total', async () => {
    let connected = true;
    const { deps, watcher, advance } = harness({
      listConnections: vi.fn(async () => [
        { id: 'conn-1', status: connected ? 'connected' : 'disconnected' },
      ]),
    });
    watcher.start();
    await flush();
    expect(deps.onTotal).toHaveBeenLastCalledWith(1);
    connected = false;
    watcher.refresh();
    await flush();
    await advance(0);
    expect(deps.onTotal).toHaveBeenLastCalledWith(0);
    watcher.stop();
  });

  it('keeps the last count when a read fails, and stops for good', async () => {
    const { deps, watcher, advance } = harness();
    watcher.start();
    await flush();
    deps.readSnapshot.mockRejectedValueOnce(new Error('offline'));
    await advance();
    expect(deps.onTotal).toHaveBeenLastCalledWith(1);
    watcher.stop();
    const reads = deps.readSnapshot.mock.calls.length;
    await advance();
    expect(deps.readSnapshot).toHaveBeenCalledTimes(reads);
  });
});
