import { act, render, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError } from '../crewApi';
import { crewObservationCopy } from './copy';
import {
  draftScope,
  draftScopeChanged,
  observationFailureOutcome,
  observationFrameText,
  type ScopeFrame,
} from './observationFailure';
import {
  isLocalHistoryFailure,
  mergePeople,
  REOBSERVE_BACKOFF_MS,
  REOBSERVE_CEILING,
  takeRecoveryDelay,
} from './useCrewObservation';
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
  /** The cursor the observer resumed after, or null for a fresh opening. */
  after: string | null;
  signal: AbortSignal;
  receive: (frame: unknown) => void;
  /** End this observer session the way the daemon does: at its lifetime, or for good. */
  end: (outcome: 'reconnect' | 'terminal') => void;
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
      after: string | null,
      signal: AbortSignal,
      receive: (frame: unknown) => void
    ) =>
      new Promise((resolve) => {
        sessions.push({ channelId, after, signal, receive, end: resolve });
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

  it('keeps the smaller opening size when a reconnected observer starts at the full page', async () => {
    const first = await observeChannel();
    send(first, messagesFrame('a', { reset: true, remaining: 0, page_size: 100 }));
    expect(crew.pageSize).toBe(100);

    // The daemon ends an observer at its lifetime; the renderer resumes after the cursor.
    send(first, { type: 'reconnect', cursor: 'sequence-a' });
    act(() => first.end('reconnect'));
    let second: Observation | undefined;
    await waitFor(() => {
      second = sessions.find(
        (item) => item !== first && item.channelId === channel.id && !item.signal.aborted
      );
      expect(second).toBeDefined();
    });
    expect(second!.after).toBe('sequence-a');

    // Its first frame extends the tail and names the full page it now asks for: the tail it
    // extends was still loaded at 100, so "Older messages" and the channel start stay right.
    send(second!, {
      type: 'messages',
      channel_id: channel.id,
      messages: [],
      cursor: 'sequence-a',
      reset: false,
      remaining: 0,
      page_size: 200,
    });
    expect(crew.pageSize).toBe(100);
    expect(crew.messages.map((item) => item.id)).toEqual(['a']);

    // A new opening is loaded at the size it names.
    send(second!, messagesFrame('b', { reset: true, remaining: 0, page_size: 200 }));
    expect(crew.pageSize).toBe(200);
  });

  it('shrinks when a frame that extends the tail was loaded at a smaller size', async () => {
    const session = await observeChannel();
    send(session, messagesFrame('a', { reset: true, remaining: 0, page_size: 100 }));
    expect(crew.pageSize).toBe(100);
    send(session, messagesFrame('b', { page_size: 50 }));
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

describe('a recoverable end of observation (live QA round 1, P0-1)', () => {
  const ended = (code: string) => ({
    type: 'error',
    clear: true,
    code,
    error: 'Room observation ended. Clear cached room content and refresh authorized access.',
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  async function nextChannelObserver(after: number): Promise<Observation> {
    let next: Observation | undefined;
    await waitFor(() => {
      next = sessions
        .slice(after)
        .find((item) => item.channelId === channel.id && !item.signal.aborted);
      expect(next).toBeDefined();
    });
    return next!;
  }

  it('clears the view, keeps the draft, and observes again by itself with no cursor', async () => {
    const first = await observeChannel();
    send(first, messagesFrame('a', { reset: true, remaining: 0 }));
    act(() => crew.setBody('keep this'));
    const before = sessions.length;

    send(first, ended('stale_cursor'));
    // `clear: true`: nothing verified stays; nothing is wrong yet either.
    expect(crew.snapshot).toBeNull();
    expect(crew.lastVerified).toBeNull();
    expect(crew.messages).toEqual([]);
    expect(crew.refreshError).toBeNull();
    expect(crew.error).toBeNull();
    expect(crew.status).toBe('updating');
    expect(crew.draft.body).toBe('keep this');

    const second = await nextChannelObserver(before);
    expect(second.after).toBeNull();
    send(second, stateFrame());
    expect(crew.status).toBe('connected');
    expect(crew.draft.body).toBe('keep this');
    expect(crew.error).toBeNull();
    // It reloaded the saved connection before observing again.
    expect(mocks.crewHttp.mock.calls.filter(([path]) => path === '/connections').length).toBe(2);
  });

  it('keeps the draft when only the workspace policy epoch moved (an invitation was accepted)', async () => {
    const first = await observeChannel();
    act(() => crew.setBody('still mine'));
    const before = sessions.length;
    send(first, ended('policy_changed'));
    const second = await nextChannelObserver(before);
    send(
      second,
      stateFrame({
        snapshot: { ...snapshot, workspace: { ...snapshot.workspace, policy_epoch: 2 } },
      })
    );
    expect(crew.draft.body).toBe('still mine');
    expect(crew.error).toBeNull();
    expect(crew.refreshError).toBeNull();
  });

  it('closes a channel the daemon says is gone, and says so plainly', async () => {
    const first = await observeChannel();
    act(() => crew.setBody('for #general'));
    send(first, ended('channel_access_changed'));
    expect(crew.channelId).toBe('');
    expect(crew.draft.body).toBe('');
    expect(crew.error?.message).toBe(
      `${crewObservationCopy.channelAccessLostNamed('#general')} ${crewObservationCopy.draftDiscarded}`
    );
    expect(crew.refreshError).toBeNull();
    // It observes the workspace again rather than the channel it just lost.
    await waitFor(() =>
      expect(sessions.some((item) => item.channelId === undefined && !item.signal.aborted)).toBe(
        true
      )
    );
  });

  it('says plainly that updates stopped once three attempts in a minute have failed', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const first = await observeChannel();
    act(() => crew.setBody('keep me'));
    send(first, ended('policy_changed'));
    for (const wait of REOBSERVE_BACKOFF_MS) {
      const before = sessions.length;
      await act(async () => {
        await vi.advanceTimersByTimeAsync(wait);
      });
      const next = await nextChannelObserver(before);
      expect(crew.refreshError).toBeNull();
      send(next, ended('policy_changed'));
    }
    expect(crew.refreshError).toBe(
      `${crewObservationCopy.updatesStopped('Fixture')} ${crewObservationCopy.draftRetained}`
    );
    expect(crew.refreshError).not.toMatch(/Room observation|cursor|policy/i);
    expect(crew.refreshErrorCode).toBe('policy_changed');
    expect(crew.status).toBe('updates-unavailable');
    expect(crew.draft.body).toBe('keep me');
  });

  it('stops at once, in plain words, on a connection the daemon does not call connected', async () => {
    const first = await observeChannel();
    mocks.crewHttp.mockImplementation(async (path: string) => {
      if (path === '/connections')
        return { connections: [{ ...connection, status: 'disconnected' }] };
      return {};
    });
    await act(async () => {
      await crew.refresh();
    });
    const open = sessions.filter((item) => !item.signal.aborted && item !== first);
    send(open[open.length - 1]!, ended('policy_changed'));
    expect(crew.refreshError).toBe(crewObservationCopy.updatesStopped('Fixture'));
    expect(crew.status).toBe('offline');
  });
});

describe('the recovery budget', () => {
  it('waits 0.3, 1 and 3 s, then stops until a verified frame or the window passes', () => {
    const budget = { attempts: [] as number[], all: [] as number[] };
    expect([0, 1, 2, 3].map(() => takeRecoveryDelay(budget, 1_000))).toEqual([
      300,
      1000,
      3000,
      null,
    ]);
    // A verified frame starts the count again.
    budget.attempts = [];
    expect(takeRecoveryDelay(budget, 2_000)).toBe(300);
    // So does a minute passing.
    const later = { attempts: [0, 1, 2], all: [0, 1, 2] };
    expect(takeRecoveryDelay(later, 61_000)).toBe(300);
  });

  it('has a ceiling a stream that verifies and ends at once cannot reset', () => {
    const budget = { attempts: [] as number[], all: [] as number[] };
    const delays: (number | null)[] = [];
    for (let index = 0; index <= REOBSERVE_CEILING; index += 1) {
      delays.push(takeRecoveryDelay(budget, 5_000 + index));
      budget.attempts = []; // each one verified
    }
    expect(delays.slice(0, REOBSERVE_CEILING).every((delay) => delay === 300)).toBe(true);
    expect(delays[REOBSERVE_CEILING]).toBeNull();
  });
});

describe('when an unsent draft must go (SECURITY-SENSITIVE)', () => {
  const base: ScopeFrame = {
    connection_id: 'conn-1',
    connection_mode: 'private',
    connection_policy_epoch: 1,
    connection_institution_id: 'ucsf',
    snapshot: {
      workspace: { mode: 'private', institution_id: 'ucsf' },
      channels: [
        { id: 'general', classification: 'restricted' },
        { id: 'methods', classification: 'public_safe' },
      ],
    },
  };
  const scope = draftScope(base, 'general', ['methods']);
  const changed = (frame: ScopeFrame, sources: string[] = ['methods'], channelId = 'general') =>
    draftScopeChanged(scope, frame, channelId, sources);
  const withWorkspace = (workspace: ScopeFrame['snapshot']['workspace']) => ({
    ...base,
    snapshot: { ...base.snapshot, workspace },
  });
  const withChannels = (channels: ScopeFrame['snapshot']['channels']) => ({
    ...base,
    snapshot: { ...base.snapshot, channels },
  });

  it('keeps it when nothing it was written under moved, whatever the workspace epoch did', () => {
    expect(changed(base)).toBe(false);
    // The broker's workspace policy epoch is not part of the scope at all.
    expect(changed({ ...base, snapshot: { ...base.snapshot } })).toBe(false);
    // Someone joined the channel: membership is not privacy.
    expect(
      changed(
        withChannels([
          { id: 'general', classification: 'restricted' },
          { id: 'methods', classification: 'public_safe' },
          { id: 'new-channel', classification: 'restricted' },
        ])
      )
    ).toBe(false);
  });

  it.each([
    ['the workspace mode', withWorkspace({ mode: 'public', institution_id: 'ucsf' })],
    ['the workspace institution', withWorkspace({ mode: 'private', institution_id: 'other' })],
    ['the connection mode', { ...base, connection_mode: 'public' }],
    ['the connection policy epoch', { ...base, connection_policy_epoch: 2 }],
    ['the connection institution', { ...base, connection_institution_id: null }],
    [
      'the selected channel’s classification',
      withChannels([
        { id: 'general', classification: 'public_safe' },
        { id: 'methods', classification: 'public_safe' },
      ]),
    ],
    [
      'a source channel’s classification',
      withChannels([
        { id: 'general', classification: 'restricted' },
        { id: 'methods', classification: 'restricted' },
      ]),
    ],
    [
      'a source channel disappearing',
      withChannels([{ id: 'general', classification: 'restricted' }]),
    ],
  ])('clears it when %s changed', (_what, frame) => {
    expect(changed(frame as ScopeFrame)).toBe(true);
  });

  it('checks a source chosen after the last view only for still being there', () => {
    expect(changed(base, ['methods', 'general'])).toBe(false);
    expect(changed(base, ['methods', 'gone'])).toBe(true);
  });

  it('compares nothing across connections, or before a first view', () => {
    expect(draftScopeChanged(null, base, 'general', [])).toBe(false);
    expect(changed({ ...base, connection_id: 'conn-2', connection_mode: 'public' })).toBe(false);
  });

  it('leaves the selected channel’s disappearance to the channel-revoked path', () => {
    expect(changed(withChannels([{ id: 'methods', classification: 'public_safe' }]), [])).toBe(
      false
    );
  });
});

describe('what the connection bar is told', () => {
  const names = { workspace: 'lab', channel: '#general' };

  it('never repeats the daemon’s sentence, whatever the code', () => {
    for (const code of [
      undefined,
      'observation_refused',
      'policy_changed',
      'stale_cursor',
      'scope_changed',
      'channel_access_changed',
      'forbidden',
      'unauthorized',
      'human_authority_required',
      'observer_capacity_reached',
      'response_too_large',
      'something_new',
    ]) {
      const text = observationFrameText(code, names);
      expect(text).not.toMatch(/observation|cursor|daemon|policy/i);
      expect(text.length).toBeGreaterThan(0);
    }
    expect(observationFrameText('channel_access_changed', names)).toBe(
      crewObservationCopy.channelAccessChanged('#general')
    );
    expect(observationFrameText('unauthorized', names)).toBe(
      crewObservationCopy.unknownComputer('lab')
    );
  });

  it('mentions the draft only when the composer held something', () => {
    expect(observationFailureOutcome('Stopped.', 'forbidden', { draftHasContent: false })).toEqual({
      clearDraft: true,
      text: 'Stopped.',
    });
    expect(observationFailureOutcome('Stopped.', 'forbidden').text).toBe(
      `Stopped. ${crewObservationCopy.draftCleared}`
    );
    expect(observationFailureOutcome('Stopped.', 'observation_refused').text).toBe(
      `Stopped. ${crewObservationCopy.draftRetained}`
    );
  });

  it('leaves a recoverable code’s draft to the next verified view, but not lost access', () => {
    const defer = { deferRecoverableToReverification: true };
    for (const code of [
      'policy_changed',
      'channel_access_changed',
      'scope_changed',
      'stale_cursor',
    ])
      expect(observationFailureOutcome('x', code, defer).clearDraft).toBe(false);
    for (const code of [
      'access_denied',
      'principal_revoked',
      'forbidden',
      'privacy_denied',
      'human_authority_required',
    ])
      expect(observationFailureOutcome('x', code, defer).clearDraft).toBe(true);
  });
});
