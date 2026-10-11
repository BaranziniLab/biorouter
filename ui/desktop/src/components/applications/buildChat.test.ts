import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  BUILD_CHAT_CLOCK_SLACK_SECS,
  buildChatEnabled,
  buildChatLabel,
  buildChatNote,
  buildChatRefusal,
  readBuildChatRow,
  resolveBuildChat,
  resolveBuildChats,
  type BuildChatRowRead,
  type BuildChatState,
} from './buildChat';
import type { Session } from '../../api';

/** Unix seconds for the app's `created_at`; chats are placed around it. */
const APP_CREATED = 1_760_000_000;
const iso = (secs: number) => new Date(secs * 1000).toISOString();

/**
 * A chat row. By default it was last written just after the app was created, as
 * the chat that built it is (the tool's result is a message, and a message
 * stamps `updated_at`), or when it was born, if that is later.
 */
function row(id: string, createdSecs: number, extra: Partial<Session> = {}): Session {
  return {
    id,
    name: `Chat ${id}`,
    created_at: iso(createdSecs),
    updated_at: iso(Math.max(createdSecs, APP_CREATED + 5)),
    working_dir: '/tmp',
    message_count: 1,
    extension_data: {},
    ...extra,
  } as Session;
}

/** A reader over a fixed table; an id it does not hold reads as deleted. */
function tableReader(table: Record<string, BuildChatRowRead>) {
  return vi.fn(async (id: string): Promise<BuildChatRowRead> => table[id] ?? { kind: 'deleted' });
}

describe('resolveBuildChat', () => {
  it('is none when the app recorded no chat, and reads nothing', async () => {
    const read = tableReader({});
    expect(await resolveBuildChat({ created_at: APP_CREATED }, read)).toEqual({ kind: 'none' });
    expect(read).not.toHaveBeenCalled();
  });

  it('is available, named, when the chat exists and predates the app', async () => {
    const read = tableReader({
      's-1': { kind: 'row', row: row('s-1', APP_CREATED - 60, { user_set_name: true }) },
    });
    expect(await resolveBuildChat({ session_id: 's-1', created_at: APP_CREATED }, read)).toEqual({
      kind: 'available',
      sessionId: 's-1',
      title: 'Chat s-1',
      userSetName: true,
      via: 'self',
    });
  });

  it('allows the clock slack, and no more', async () => {
    const within = tableReader({
      's-1': { kind: 'row', row: row('s-1', APP_CREATED + BUILD_CHAT_CLOCK_SLACK_SECS) },
    });
    expect(
      (await resolveBuildChat({ session_id: 's-1', created_at: APP_CREATED }, within)).kind
    ).toBe('available');

    const beyond = tableReader({
      's-1': { kind: 'row', row: row('s-1', APP_CREATED + BUILD_CHAT_CLOCK_SLACK_SECS + 1) },
    });
    expect(
      (await resolveBuildChat({ session_id: 's-1', created_at: APP_CREATED }, beyond)).kind
    ).toBe('replaced');
  });

  // A chat created after the app cannot be the one that built it: the id names
  // another conversation (issued again after a backup restore, or another
  // machine's id carried by an export made before exports dropped it).
  it('is replaced when the id now names a chat newer than the app', async () => {
    const read = tableReader({ 's-1': { kind: 'row', row: row('s-1', APP_CREATED + 86_400) } });
    expect(await resolveBuildChat({ session_id: 's-1', created_at: APP_CREATED }, read)).toEqual({
      kind: 'replaced',
    });
  });

  // The other end of the same rule: the chat that built the app was written to
  // when it did, so a chat that had gone quiet before the app was created names
  // somebody else's conversation (an older export's id on this machine).
  it('is replaced when the id names a chat that went quiet before the app', async () => {
    const quiet = (updatedSecs: number) =>
      tableReader({
        's-1': {
          kind: 'row',
          row: row('s-1', APP_CREATED - 6 * 3_600, { updated_at: iso(updatedSecs) }),
        },
      });
    const app = { session_id: 's-1', created_at: APP_CREATED };
    expect((await resolveBuildChat(app, quiet(APP_CREATED - 3_600))).kind).toBe('replaced');
    expect(
      (await resolveBuildChat(app, quiet(APP_CREATED - BUILD_CHAT_CLOCK_SLACK_SECS - 1))).kind
    ).toBe('replaced');
    expect(
      (await resolveBuildChat(app, quiet(APP_CREATED - BUILD_CHAT_CLOCK_SLACK_SECS))).kind
    ).toBe('available');
    // ⚠ The limit, pinned so nobody reads more into the check: a chat that was
    // born before the app and still active after it passes, whoever built it.
    expect((await resolveBuildChat(app, quiet(APP_CREATED + 3_600))).kind).toBe('available');
  });

  // ⚠ The daemon serves a manifest without `created_at` as 0, never as absent.
  // Read as a time, every real chat was newer than 1970 and the row grayed out.
  it('treats an app created_at of 0 as unknown, not as 1970', async () => {
    const read = tableReader({
      's-1': { kind: 'row', row: row('s-1', APP_CREATED - 60, { updated_at: iso(APP_CREATED) }) },
    });
    expect(await resolveBuildChat({ session_id: 's-1', created_at: 0 }, read)).toMatchObject({
      kind: 'available',
      sessionId: 's-1',
      via: 'self',
    });
  });

  it('cannot check identity without times, and does not invent a missing chat', async () => {
    const read = tableReader({ 's-1': { kind: 'row', row: row('s-1', APP_CREATED + 86_400) } });
    expect((await resolveBuildChat({ session_id: 's-1' }, read)).kind).toBe('available');

    const unparsable = tableReader({
      's-1': {
        kind: 'row',
        row: { ...row('s-1', 0), created_at: 'not a date', updated_at: 'not a date either' },
      },
    });
    expect(
      (await resolveBuildChat({ session_id: 's-1', created_at: APP_CREATED }, unparsable)).kind
    ).toBe('available');
  });

  it.each(['deleted', 'unreachable', 'unknown'] as const)(
    'passes a %s read through',
    async (kind) => {
      const read = tableReader({ 's-1': { kind } });
      expect(await resolveBuildChat({ session_id: 's-1', created_at: APP_CREATED }, read)).toEqual({
        kind,
      });
    }
  );

  describe('an app built by a subagent run', () => {
    const child = row('child', APP_CREATED - 30, {
      session_type: 'sub_agent',
      parent_session_id: 'parent',
    });

    it('leads to the conversation that delegated the run', async () => {
      const read = tableReader({
        child: { kind: 'row', row: child },
        parent: { kind: 'row', row: row('parent', APP_CREATED - 600) },
      });
      expect(
        await resolveBuildChat({ session_id: 'child', created_at: APP_CREATED }, read)
      ).toMatchObject({
        kind: 'available',
        sessionId: 'parent',
        title: 'Chat parent',
        via: 'parent',
      });
    });

    // ⚠ Deleting a parent does not delete its runs, so the row must not read
    // "deleted" while the run itself can still be opened.
    it('falls back to the run itself when the parent is gone', async () => {
      const read = tableReader({ child: { kind: 'row', row: child } });
      expect(
        await resolveBuildChat({ session_id: 'child', created_at: APP_CREATED }, read)
      ).toMatchObject({ kind: 'available', sessionId: 'child', via: 'subagent' });
    });

    it('does not trust a parent id that now names a newer chat', async () => {
      const read = tableReader({
        child: { kind: 'row', row: child },
        parent: { kind: 'row', row: row('parent', APP_CREATED + 3_600) },
      });
      expect(
        await resolveBuildChat({ session_id: 'child', created_at: APP_CREATED }, read)
      ).toMatchObject({ kind: 'available', sessionId: 'child', via: 'subagent' });
    });
  });
});

describe('resolveBuildChats', () => {
  it('reads each distinct chat once, however many apps were built in it', async () => {
    const read = tableReader({
      shared: { kind: 'row', row: row('shared', APP_CREATED - 60) },
    });
    const states = await resolveBuildChats(
      [
        { id: 'a', session_id: 'shared', created_at: APP_CREATED },
        { id: 'b', session_id: 'shared', created_at: APP_CREATED + 10 },
        { id: 'c', session_id: 'gone', created_at: APP_CREATED },
        { id: 'd', created_at: APP_CREATED },
      ],
      read
    );
    expect(read).toHaveBeenCalledTimes(2);
    expect(states.a.kind).toBe('available');
    expect(states.b.kind).toBe('available');
    expect(states.c).toEqual({ kind: 'deleted' });
    expect(states.d).toEqual({ kind: 'none' });
  });
});

describe('the control a state renders', () => {
  const cases: [BuildChatState, boolean, string, string | undefined][] = [
    [
      { kind: 'available', sessionId: 's', via: 'self' },
      true,
      'Open the chat where Atlas was built',
      undefined,
    ],
    [
      { kind: 'available', sessionId: 's', via: 'subagent' },
      true,
      'Open the subagent run that built Atlas',
      undefined,
    ],
    [{ kind: 'checking' }, true, 'Open the chat where Atlas was built', undefined],
    [{ kind: 'unknown' }, true, 'Open the chat where Atlas was built', undefined],
    [
      { kind: 'deleted' },
      false,
      'The chat where Atlas was built was deleted',
      'Build chat deleted',
    ],
    [
      { kind: 'replaced' },
      false,
      'The chat where Atlas was built no longer exists',
      'Build chat no longer exists',
    ],
    [
      { kind: 'unreachable' },
      false,
      "The chat where Atlas was built can't be opened here",
      "Build chat can't be opened here",
    ],
    [{ kind: 'none' }, false, 'No chat was recorded for Atlas', 'No build chat recorded'],
  ];

  it.each(cases)('%o', (state, enabled, label, note) => {
    expect(buildChatEnabled(state)).toBe(enabled);
    expect(buildChatLabel(state, 'Atlas')).toBe(label);
    expect(buildChatNote(state)).toBe(note);
    // Every state that does not open a chat says why.
    expect(buildChatRefusal(state) === undefined).toBe(state.kind === 'available');
  });

  // A daemon that did not answer must never be reported as a deleted chat.
  it('never calls an unanswered check a deletion', () => {
    expect(buildChatRefusal({ kind: 'unknown' })).toBe(
      "Couldn't reach Biorouter to check that chat. Try again."
    );
  });
});

describe('readBuildChatRow', () => {
  let fetchMock: ReturnType<typeof vi.fn>;
  let respond: () => Response | Promise<Response>;

  const urlOf = (input: unknown) => (input instanceof Request ? input.url : String(input));

  beforeEach(() => {
    respond = () => new Response('', { status: 404 });
    fetchMock = vi.fn(async () => respond());
    vi.stubGlobal('fetch', fetchMock);
    window.electron = {
      getUserActionKey: vi.fn().mockResolvedValue('proof-of-person'),
    } as unknown as typeof window.electron;
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  const json = (body: unknown, status = 200) =>
    new Response(JSON.stringify(body), {
      status,
      headers: { 'Content-Type': 'application/json' },
    });

  it('asks the gated metadata read, carrying the proof of a person', async () => {
    respond = () => json(row('s-1', APP_CREATED));
    const read = await readBuildChatRow('s-1');

    expect(read.kind).toBe('row');
    const request = fetchMock.mock.calls[0][0] as Request;
    const url = new URL(urlOf(request));
    expect(url.pathname).toBe('/sessions/s-1');
    expect(url.searchParams.get('metadata_only')).toBe('true');
    // ⚠ Without it the reach gate answers 403 for a deleted chat too.
    expect(request.headers.get('X-User-Action')).toBe('proof-of-person');
  });

  it.each([
    [404, 'deleted'],
    [400, 'deleted'],
    [403, 'unreachable'],
    [500, 'unknown'],
    [503, 'unknown'],
    [408, 'unknown'],
    [429, 'unknown'],
    [401, 'unknown'],
  ])('maps %i to %s', async (status, kind) => {
    respond = () => new Response('', { status });
    expect((await readBuildChatRow('s-1')).kind).toBe(kind);
  });

  it('is unknown when nothing answered at all', async () => {
    fetchMock.mockRejectedValue(new TypeError('Failed to fetch'));
    expect(await readBuildChatRow('s-1')).toEqual({ kind: 'unknown' });
  });

  // A 200 that is not this chat's row proves nothing (a proxy, an older daemon).
  it('is unknown for a 200 that is not this chat', async () => {
    respond = () => json([{ id: 'cohort-explorer' }]);
    expect(await readBuildChatRow('s-1')).toEqual({ kind: 'unknown' });
    respond = () => json(row('someone-else', APP_CREATED));
    expect(await readBuildChatRow('s-1')).toEqual({ kind: 'unknown' });
  });
});
