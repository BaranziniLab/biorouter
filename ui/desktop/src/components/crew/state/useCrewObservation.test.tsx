import { act, render, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError } from '../crewApi';
import { crewObservationCopy } from './copy';
import { isLocalHistoryFailure, mergePeople } from './useCrewObservation';
import { useCrewController } from './useCrewController';
import type { CrewController } from './types';

/**
 * The observation's display projections, through the controller that exposes them: the end of
 * the opening backlog, the page size, the authors a page names, the broker's capabilities, and
 * what an older page does when the broker refuses it.
 */

const mocks = vi.hoisted(() => ({
  crewHttp: vi.fn(),
  crewRequest: vi.fn(),
  observeCrew: vi.fn(),
}));

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return {
    ...actual,
    crewHttp: mocks.crewHttp,
    crewRequest: mocks.crewRequest,
    observeCrew: mocks.observeCrew,
  };
});

const connection = {
  id: 'conn-1',
  name: 'Fixture',
  ssh_target: 'alice@hpc',
  port: 22,
  socket_path: '/tmp/socket',
  owner_uid: 1000,
  workspace_id: 'workspace-1',
  workspace_public_key: 'workspace-key',
  public_key: 'device-key',
  device_id: 'device-1',
  cluster_connection_id: 'cluster-1',
  mode: 'private' as const,
  policy_epoch: 1,
  institution_id: 'ucsf',
  status: 'connected' as const,
  remote_execution: false,
};
const actor = { id: 'person-1', uid: 1000, username: 'alice', nickname: 'Alice' };
const channel = {
  id: 'channel-1',
  team_id: 'team-1',
  name: 'general',
  created_by: actor.id,
  owner_id: actor.id,
  members: [actor.id],
  archived: false,
  classification: 'restricted' as const,
};
const snapshot = {
  workspace: { id: 'workspace-1', host_uid: 1000, mode: 'private', policy_epoch: 1 },
  actor,
  principals: [actor],
  teams: [
    {
      id: 'team-1',
      name: 'Lab',
      created_by: actor.id,
      members: [actor.id],
      general_channel_id: 'channel-1',
    },
  ],
  channels: [channel],
  invitations: [],
  runs: [],
};
function stateFrame(extra: Record<string, unknown> = {}) {
  return {
    type: 'state',
    connection_id: connection.id,
    connection_mode: 'private',
    connection_policy_epoch: 1,
    connection_institution_id: 'ucsf',
    snapshot,
    runs: [],
    ...extra,
  };
}
function message(id: string, actorId = actor.id) {
  return {
    id,
    sequence: `sequence-${id}`,
    channel_id: channel.id,
    actor_id: actorId,
    body: `message ${id}`,
    created_at: 1_700_000_000,
    restricted: false,
    source_channels: [channel.id],
    attachments: [],
  };
}
function messagesFrame(id: string, extra: Record<string, unknown> = {}, author = actor.id) {
  return {
    type: 'messages',
    channel_id: channel.id,
    messages: [message(id, author)],
    cursor: `sequence-${id}`,
    reset: false,
    ...extra,
  };
}

interface Observation {
  channelId: string | undefined;
  signal: AbortSignal;
  receive: (frame: unknown) => void;
}

let sessions: Observation[] = [];
let crew: CrewController;

function Harness() {
  crew = useCrewController({ keepLastVerifiedView: true });
  return null;
}

/** Render the controller and wait for an observer of the channel; returns it. */
async function observeChannel(): Promise<Observation> {
  render(
    <MemoryRouter initialEntries={['/crew']}>
      <Harness />
    </MemoryRouter>
  );
  await waitFor(() => expect(sessions.length).toBeGreaterThan(0));
  act(() => sessions[sessions.length - 1].receive(stateFrame()));
  await waitFor(() => expect(crew.channelId).toBe(channel.id));
  return latestChannelObserver();
}

async function latestChannelObserver(): Promise<Observation> {
  let session: Observation | undefined;
  await waitFor(() => {
    const open = sessions.filter((item) => item.channelId === channel.id && !item.signal.aborted);
    session = open[open.length - 1];
    expect(session).toBeDefined();
  });
  act(() => session!.receive(stateFrame()));
  return session!;
}

function send(session: Observation, frame: unknown) {
  act(() => session.receive(frame));
}

beforeEach(() => {
  vi.clearAllMocks();
  sessions = [];
  mocks.crewHttp.mockImplementation(async (path: string) => {
    if (path === '/connections') return { connections: [connection] };
    return {};
  });
  mocks.crewRequest.mockResolvedValue({});
  mocks.observeCrew.mockImplementation(
    (
      _connectionId: string,
      channelId: string | undefined,
      _after: string | null,
      signal: AbortSignal,
      receive: (frame: unknown) => void
    ) =>
      new Promise((resolve) => {
        sessions.push({ channelId, signal, receive });
        signal.addEventListener('abort', () => resolve('terminal'));
      })
  );
});

describe('the end of the opening backlog', () => {
  it('follows the observer’s count down to zero', async () => {
    const session = await observeChannel();
    expect(crew.backlogComplete).toBeUndefined();
    send(session, messagesFrame('a', { reset: true, remaining: 2 }));
    expect(crew.backlogComplete).toBe(false);
    send(session, messagesFrame('b', { remaining: 1 }));
    expect(crew.backlogComplete).toBe(false);
    send(session, messagesFrame('c', { remaining: 0 }));
    expect(crew.backlogComplete).toBe(true);
    expect(crew.messages.map((item) => item.id)).toEqual(['a', 'b', 'c']);
    // A later page's count does not reopen the backlog.
    send(session, messagesFrame('d', { remaining: 3 }));
    expect(crew.backlogComplete).toBe(true);
  });

  it('ends at once for an empty channel', async () => {
    const session = await observeChannel();
    send(session, {
      type: 'messages',
      channel_id: channel.id,
      messages: [],
      cursor: null,
      reset: true,
      remaining: 0,
    });
    expect(crew.backlogComplete).toBe(true);
    expect(crew.messagesLoaded).toBe(true);
  });

  it('says nothing for a daemon that does not count, so the timeline times the stream', async () => {
    const session = await observeChannel();
    send(session, messagesFrame('a', { reset: true }));
    send(session, messagesFrame('b'));
    expect(crew.backlogComplete).toBeUndefined();
    expect(crew.pageSize).toBe(200);
  });
});

describe('the page size', () => {
  it('is the full page until the observer says it asks for less', async () => {
    const session = await observeChannel();
    expect(crew.pageSize).toBe(200);
    send(session, messagesFrame('a', { reset: true, remaining: 0, page_size: 50 }));
    expect(crew.pageSize).toBe(50);
  });
});

describe('the authors a page names', () => {
  const dan = { username: 'dan', display_name: 'Dan Wu', active: false };

  it('keeps every author the frames named, and starts over when the list does', async () => {
    const session = await observeChannel();
    send(
      session,
      messagesFrame('a', { reset: true, remaining: 1, people: { 'person-dan': dan } }, 'person-dan')
    );
    const first = crew.people;
    expect(first?.['person-dan']).toEqual(dan);
    // A frame naming no one new keeps the same map, so a directory memoized on it is not rebuilt.
    send(session, messagesFrame('b', { remaining: 0, people: { 'person-dan': dan } }));
    expect(crew.people).toBe(first);
    send(session, messagesFrame('c', { people: { [actor.id]: { username: 'alice' } } }));
    expect(Object.keys(crew.people ?? {}).sort()).toEqual([actor.id, 'person-dan'].sort());

    send(session, messagesFrame('d', { reset: true, remaining: 0 }));
    expect(crew.people).toBeNull();
  });

  it('merges without letting a key reach a prototype', () => {
    const base = mergePeople(null, { a: { username: 'a' } });
    const merged = mergePeople(base, JSON.parse('{"__proto__":{"username":"m"}}'));
    expect(Object.getPrototypeOf(merged)).toBeNull();
    expect(({} as Record<string, unknown>).username).toBeUndefined();
    expect(mergePeople(base, undefined)).toBe(base);
  });
});

describe('the broker’s capabilities', () => {
  it('are the last state frame’s, and unknown when it names none', async () => {
    const session = await observeChannel();
    expect(crew.capabilities).toBeNull();
    send(session, stateFrame({ capabilities: ['unique_names_v1'] }));
    expect(crew.capabilities).toEqual(['unique_names_v1']);
    const same = crew.capabilities;
    send(session, stateFrame({ capabilities: ['unique_names_v1'] }));
    expect(crew.capabilities).toBe(same);
    send(session, stateFrame());
    expect(crew.capabilities).toBeNull();
  });
});

describe('an older page', () => {
  const refused = (brokerCode: string, status = 400) =>
    new CrewHttpError(
      `${brokerCode}: refused`,
      status,
      'crew_request_refused',
      undefined,
      brokerCode
    );

  async function openFullTail(pageSize: number) {
    const session = await observeChannel();
    send(session, messagesFrame('a', { reset: true, remaining: 0, page_size: pageSize }));
    return session;
  }

  it('asks at the observer’s size and halves it while the broker says the answer is too large', async () => {
    await openFullTail(100);
    const limits: number[] = [];
    mocks.crewRequest.mockImplementation(
      async (_connection: string, method: string, params: Record<string, unknown>) => {
        if (method !== 'messages.history') return {};
        limits.push(params.limit as number);
        if ((params.limit as number) > 25) throw refused('response_too_large');
        return {
          messages: [message('older', 'person-dan')],
          cursor: 'sequence-older',
          people: { 'person-dan': { username: 'dan', display_name: 'Dan Wu', active: false } },
        };
      }
    );
    act(() => crew.loadOlder());
    await waitFor(() => expect(crew.messagesLoaded).toBe(true));
    expect(limits).toEqual([100, 50, 25]);
    expect(crew.historyBefore).toBe('sequence-a');
    expect(crew.pageSize).toBe(25);
    expect(crew.messages.map((item) => item.id)).toEqual(['older']);
    expect(crew.people?.['person-dan']?.display_name).toBe('Dan Wu');
    expect(crew.snapshot).not.toBeNull();
  });

  it('keeps the verified view when it fails for a reason that is not about access', async () => {
    await openFullTail(200);
    const observers = mocks.observeCrew.mock.calls.length;
    mocks.crewRequest.mockRejectedValue(
      new CrewHttpError('Crew unavailable', 503, 'crew_unavailable')
    );
    act(() => crew.loadOlder());
    await waitFor(() => expect(crew.error?.source).toBe('observer'));
    expect(crew.error?.message).toContain(crewObservationCopy.historyFailed);
    expect(crew.snapshot).not.toBeNull();
    expect(crew.refreshError).toBeNull();
    expect(crew.historyBefore).toBeNull();
    // The live tail is observed afresh.
    await waitFor(() => expect(mocks.observeCrew.mock.calls.length).toBeGreaterThan(observers));
  });

  it.each([
    ['a broker access refusal', refused('forbidden')],
    ['a stale cursor', refused('stale_cursor')],
    [
      'a missing proof of a person',
      new CrewHttpError(
        'Verified human Crew authority is required',
        403,
        'crew_user_action_required'
      ),
    ],
    ['a refusal the broker did not code', new CrewHttpError('x', 400, 'crew_request_refused')],
    ['a failure it cannot classify', new Error('something the renderer cannot classify')],
  ])('still clears the view after %s', async (_name, failure) => {
    await openFullTail(200);
    mocks.crewRequest.mockRejectedValue(failure);
    act(() => crew.loadOlder());
    await waitFor(() => expect(crew.refreshError).not.toBeNull());
    expect(crew.snapshot).toBeNull();
    expect(crew.lastVerified).toBeNull();
    expect(crew.messages).toEqual([]);
  });

  it('tells a failure about the page from one about access', () => {
    expect(isLocalHistoryFailure(new CrewHttpError('x', 503))).toBe(true);
    expect(isLocalHistoryFailure(refused('rate_limited'))).toBe(true);
    expect(isLocalHistoryFailure(refused('response_too_large'))).toBe(true);
    expect(isLocalHistoryFailure(refused('forbidden'))).toBe(false);
    expect(isLocalHistoryFailure(refused('privacy_denied', 503))).toBe(false);
    expect(isLocalHistoryFailure(refused('stale_cursor'))).toBe(false);
    expect(isLocalHistoryFailure(new CrewHttpError('x', 400, 'crew_request_refused'))).toBe(false);
    expect(isLocalHistoryFailure(new CrewHttpError('x', 403, 'crew_user_action_required'))).toBe(
      false
    );
    expect(isLocalHistoryFailure(new TypeError('Failed to fetch'))).toBe(false);
  });
});
