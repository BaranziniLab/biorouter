import { act, render, screen, waitFor } from '@testing-library/react';
import { useState } from 'react';
import { MemoryRouter, type InitialEntry } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { chatAccessRouteState } from '../access/ChatConnectNote';
import { CREW_NOT_SENT } from '../api/errors';
import { composerCopy } from '../composer/copy';
import { CrewHttpError } from '../crewApi';
import { useCrewTransfers } from '../files/useCrewTransfers';
import { MEMBERSHIP_ENDED_CODE } from './connectFailure';
import { crewActionCopy, crewObservationCopy } from './copy';
import { rememberLastChannel, stashedDraft } from './draftStash';
import {
  ARRIVAL_CONNECT_STORAGE_KEY,
  CHAT_ACCESS_INTENT_ROUTE_KEY,
  CREW_CONNECT_ROUTE_KEY,
  forgetConnectionMemory,
  OFFLINE_FOLLOW_WINDOW_MS,
  QUIET_REOBSERVE_GAPS_MS,
  QUIET_REOBSERVE_WINDOW_MS,
  takeQuietReobserve,
} from './useCrewConnections';
import { teamForView } from './useCrewObservation';
import {
  CrewControllerProvider,
  useCrew,
  useCrewErrorSlot,
  useCrewSurfaceReset,
} from './CrewControllerContext';
import { channelForTeam, useCrewController } from './useCrewController';
import type {
  CrewController,
  CrewControllerOptions,
  ErrorSource,
  SurfaceResetReason,
} from './types';

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
const stateFrame = {
  type: 'state',
  connection_id: connection.id,
  connection_mode: 'private',
  connection_policy_epoch: 1,
  connection_institution_id: 'ucsf',
  snapshot,
  runs: [],
  cursor: null,
  labels: { 'person-1': { full: 'Alice (@alice)', short: 'Alice', collides: false } },
};
const messagesFrame = {
  type: 'messages',
  channel_id: channel.id,
  messages: [
    {
      id: 'message-1',
      sequence: 'sequence-1',
      channel_id: channel.id,
      actor_id: actor.id,
      body: 'hello',
      created_at: 1_700_000_000,
      restricted: false,
      source_channels: [channel.id],
      attachments: [],
    },
  ],
  cursor: 'sequence-1',
  reset: true,
};

interface Observation {
  channelId: string | undefined;
  signal: AbortSignal;
  receive: (frame: unknown) => void;
}

/** An observer the test drives: each call stays open until the test sends frames or aborts it. */
function controllableObserver(): Observation[] {
  const sessions: Observation[] = [];
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
  return sessions;
}

/**
 * End the latest observation with `frame`, and the quiet re-observation that follows it the same
 * way at once (Q2-01): a second end within `QUIET_REOBSERVE_GAPS_MS[1]` is shown, not hidden.
 */
async function endTwice(sessions: Observation[], frame: unknown) {
  const before = sessions.length;
  act(() => sessions[sessions.length - 1]!.receive(frame));
  await waitFor(() => expect(sessions.length).toBeGreaterThan(before));
  act(() => sessions[sessions.length - 1]!.receive(frame));
}

/** An observer that answers every call with a verified state and the channel's messages. */
function answeringObserver() {
  mocks.observeCrew.mockImplementation(
    async (
      _connectionId: string,
      channelId: string | undefined,
      _after: string | null,
      signal: AbortSignal,
      receive: (frame: unknown) => void
    ) => {
      if (signal.aborted) return 'terminal';
      receive(stateFrame);
      if (channelId) receive(messagesFrame);
      return 'terminal';
    }
  );
}

let crew: CrewController;
const screens: string[] = [];

function Harness({
  options,
  children,
}: {
  options?: CrewControllerOptions;
  children?: React.ReactNode;
}) {
  const controller = useCrewController(options);
  crew = controller;
  screens.push(controller.screen);
  return <CrewControllerProvider controller={controller}>{children}</CrewControllerProvider>;
}

function renderController(
  options?: CrewControllerOptions,
  children?: React.ReactNode,
  entry: InitialEntry = '/crew'
) {
  return render(
    <MemoryRouter initialEntries={[entry]}>
      <Harness options={options}>{children}</Harness>
    </MemoryRouter>
  );
}

function deferred<T = void>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

async function verifiedChannel() {
  await waitFor(() => expect(crew.snapshot).not.toBeNull());
  await waitFor(() => expect(crew.channelId).toBe(channel.id));
  await waitFor(() => expect(crew.messagesLoaded).toBe(true));
}

beforeEach(() => {
  vi.clearAllMocks();
  screens.length = 0;
  mocks.crewHttp.mockImplementation(async (path: string) => {
    if (path === '/connections') return { connections: [connection] };
    return {};
  });
  mocks.crewRequest.mockResolvedValue({});
  answeringObserver();
});

describe('act(source) routing and pending keys', () => {
  function Slot({ source }: { source: ErrorSource }) {
    const mine = useCrewErrorSlot(source);
    return <div data-testid={`slot-${source}`}>{mine ? 'here' : ''}</div>;
  }
  function OptionalSlot({ source }: { source: ErrorSource }) {
    const [mounted, setMounted] = useState(true);
    return (
      <>
        <button onClick={() => setMounted(false)}>Unmount {source}</button>
        {mounted && <Slot source={source} />}
      </>
    );
  }

  it('records a failure with its source, renders it at the mounted surface, else in the bar', async () => {
    renderController(
      {},
      <>
        <OptionalSlot source="pane:agent" />
        <Slot source="global" />
      </>
    );
    await verifiedChannel();

    let result: string | undefined = 'unset';
    await act(async () => {
      result = await crew.act('pane:agent', 'run.start', async () => {
        throw new CrewHttpError('start failed', 400, 'crew_request_refused');
      });
    });
    expect(result).toBeUndefined();
    expect(crew.error).toEqual({
      message: 'start failed',
      source: 'pane:agent',
      code: 'crew_request_refused',
    });
    await waitFor(() => expect(screen.getByTestId('slot-pane:agent')).toHaveTextContent('here'));
    expect(screen.getByTestId('slot-global')).toHaveTextContent('');
    expect(crew.errorSlotFor('pane:agent')).toBe(true);
    expect(crew.errorSlotFor('global')).toBe(false);

    // The surface that caused it goes away: the connection bar shows it instead, once.
    act(() => screen.getByRole('button', { name: 'Unmount pane:agent' }).click());
    await waitFor(() => expect(screen.getByTestId('slot-global')).toHaveTextContent('here'));
    expect(crew.errorSlotFor('pane:agent')).toBe(false);
  });

  it('returns the value of a successful action and clears the previous error unless asked not to', async () => {
    renderController();
    await verifiedChannel();
    await act(async () => {
      await crew.act('composer', 'send', async () => {
        throw new Error('send failed');
      });
    });
    expect(crew.error?.source).toBe('composer');

    let value: number | undefined;
    await act(async () => {
      value = await crew.act('global', 'refresh', async () => 7, { preserveError: true });
    });
    expect(value).toBe(7);
    expect(crew.error?.message).toBe('send failed');

    await act(async () => {
      await crew.act('global', 'mutate:team.create', async () => undefined);
    });
    expect(crew.error).toBeNull();
  });

  it('routes observer errors to the bar and gives a non-Error failure the fallback text', async () => {
    renderController({}, <Slot source="observer" />);
    await verifiedChannel();
    await act(async () => {
      await crew.act('dialog:create-team', 'mutate:team.create', () => Promise.reject('nope'));
    });
    expect(crew.error).toEqual({
      message: 'Crew could not complete that action.',
      source: 'dialog:create-team',
    });
    expect(crew.errorSlotFor('global')).toBe(true);
    act(() => crew.reportError('observer failure', 'observer'));
    expect(crew.errorSlotFor('observer')).toBe(true);
    act(() => crew.dismissError());
    expect(crew.error).toBeNull();
  });

  it('marks only the running action pending, per key, and keeps busy while any runs', async () => {
    renderController();
    await verifiedChannel();
    const first = deferred();
    const second = deferred();
    let running!: Promise<unknown>;
    act(() => {
      running = Promise.all([
        crew.act('global', 'mutate:team.create', () => first.promise),
        crew.act('global', 'mutate:team.create', () => second.promise),
      ]);
    });
    expect(crew.isPending('mutate:team.create')).toBe(true);
    expect(crew.isPending('send')).toBe(false);
    expect(crew.busy).toBe(true);

    await act(async () => first.resolve());
    expect(crew.isPending('mutate:team.create')).toBe(true);
    await act(async () => {
      second.resolve();
      await running;
    });
    expect(crew.isPending('mutate:team.create')).toBe(false);
    expect(crew.busy).toBe(false);
  });
});

describe('the last verified view', () => {
  it('stays null without the option', async () => {
    renderController();
    await verifiedChannel();
    const sessions = controllableObserver();
    await act(async () => {
      await crew.refresh();
    });
    expect(crew.snapshot).toBeNull();
    expect(crew.lastVerified).toBeNull();
    expect(sessions.length).toBeGreaterThan(0);
    expect(crew.screen).toBe('checking');
  });

  it('is kept through a refresh, then dropped by an observation failure', async () => {
    renderController({ keepLastVerifiedView: true });
    await verifiedChannel();
    await waitFor(() => expect(crew.lastVerified?.messages).toHaveLength(1));
    expect(crew.lastVerified).toMatchObject({
      connectionId: connection.id,
      channelId: channel.id,
      teamId: 'team-1',
      labels: stateFrame.labels,
    });
    expect(crew.labels).toEqual(stateFrame.labels);

    const sessions = controllableObserver();
    await act(async () => {
      await crew.refresh();
    });
    // Presentation only: the verified snapshot is cleared, the copy is not.
    expect(crew.snapshot).toBeNull();
    expect(crew.messages).toEqual([]);
    expect(crew.labels).toBeNull();
    expect(crew.lastVerified?.snapshot.channels[0]?.id).toBe(channel.id);
    expect(crew.lastVerified?.messages).toHaveLength(1);
    expect(crew.screen).toBe('channel');
    expect(crew.status).toBe('checking');
    expect(crew.effectivePrivacy).toBeNull();

    await waitFor(() => expect(sessions.length).toBeGreaterThan(0));
    await endTwice(sessions, {
      type: 'error',
      clear: true,
      code: 'observation_refused',
      error: 'Room observation ended.',
    });
    await waitFor(() => expect(crew.lastVerified).toBeNull());
    // Plain words, never the daemon's sentence; nothing about a draft the composer never held.
    expect(crew.refreshError).toBe(crewObservationCopy.updatesStopped('Fixture'));
    expect(crew.refreshErrorCode).toBe('observation_refused');
    expect(crew.screen).toBe('updates-paused');
    expect(crew.status).toBe('updates-unavailable');
  });

  it('never shows the first-run or join screens while an enrolled connection is checked or re-verified', async () => {
    const sessions = controllableObserver();
    renderController({ keepLastVerifiedView: true });
    await waitFor(() => expect(sessions.length).toBe(1));
    expect(crew.screen).toBe('checking');
    expect(crew.status).toBe('checking');

    act(() => sessions[0]!.receive(stateFrame));
    await waitFor(() => expect(sessions.length).toBeGreaterThan(1));
    act(() => sessions[sessions.length - 1]!.receive(messagesFrame));
    await waitFor(() => expect(crew.screen).toBe('channel'));

    // A dropped bridge: the observer fails (and fails again once observed again quietly), then
    // Retry re-verifies.
    await endTwice(sessions, {
      type: 'error',
      clear: true,
      code: 'observation_refused',
      error: 'Crew SSH failure [ssh_eof; child_before_cleanup=exit_255]',
    });
    await waitFor(() => expect(crew.screen).toBe('updates-paused'));
    await act(async () => {
      await crew.refresh();
    });
    expect(crew.screen).toBe('checking');

    expect(screens).not.toContain('welcome');
    expect(screens).not.toContain('join');
  });
});

describe('connect and sign in', () => {
  function failConnect(error: Error) {
    mocks.crewHttp.mockImplementation(async (path: string) => {
      if (path === '/connections') return { connections: [connection] };
      if (path === '/connections/conn-1/connect') throw error;
      return {};
    });
  }
  const authRequired = () =>
    new CrewHttpError('Crew SSH failure [ssh_eof]', 400, 'crew_ssh_auth_required');

  it('opens Sign in by itself after a user-initiated connect that needs a password, with the option', async () => {
    renderController({ autoOpenSignIn: true });
    await verifiedChannel();
    failConnect(authRequired());
    await act(async () => {
      await crew.connect({ userInitiated: true });
    });
    expect(crew.signIn).toEqual({ open: true, reason: 'auto' });
    expect(crew.lastConnectFailure).toEqual({
      kind: 'auth_required',
      code: 'crew_ssh_auth_required',
      message: 'Crew SSH failure [ssh_eof]',
    });
    // The raw text is kept for the surface that explains the cause, else the connection bar.
    expect(crew.error).toMatchObject({ source: 'connect', code: 'crew_ssh_auth_required' });
    expect(crew.errorSlotFor('global')).toBe(true);
  });

  it('does not open Sign in for a connect the person did not start', async () => {
    renderController({ autoOpenSignIn: true });
    await verifiedChannel();
    failConnect(authRequired());
    await act(async () => {
      await crew.connect();
    });
    expect(crew.signIn.open).toBe(false);
    expect(crew.lastConnectFailure?.kind).toBe('auth_required');
  });

  it('does not open Sign in without the option (no options given)', async () => {
    renderController();
    await verifiedChannel();
    failConnect(authRequired());
    await act(async () => {
      await crew.connect({ userInitiated: true });
    });
    expect(crew.signIn.open).toBe(false);
  });

  it('opens Sign in for an older daemon whose text says the SSH child ended', async () => {
    renderController({ autoOpenSignIn: true });
    await verifiedChannel();
    failConnect(
      new CrewHttpError(
        'Crew SSH failure [ssh_eof; child_before_cleanup=exit_255]: reconnect.',
        400,
        'crew_request_refused'
      )
    );
    await act(async () => {
      await crew.connect({ userInitiated: true });
    });
    expect(crew.signIn).toEqual({ open: true, reason: 'auto' });
  });

  it('shows a trust failure as its own screen and never opens Sign in for it', async () => {
    mocks.crewHttp.mockImplementation(async (path: string) => {
      if (path === '/connections')
        return { connections: [{ ...connection, status: 'disconnected' }] };
      if (path === '/connections/conn-1/connect')
        throw new CrewHttpError('Host key verification failed.', 400, 'crew_ssh_host_key_unknown');
      return {};
    });
    mocks.observeCrew.mockImplementation(async () => {
      throw new CrewHttpError('Crew is not connected.', 400, 'crew_request_refused');
    });
    renderController({ autoOpenSignIn: true });
    await waitFor(() => expect(crew.connectionsState).toBe('loaded'));
    await act(async () => {
      await crew.connect({ userInitiated: true });
    });
    expect(crew.signIn.open).toBe(false);
    expect(crew.screen).toBe('trust');
    expect(crew.status).toBe('cant-verify');
  });

  it('refreshes after signing in without a second POST connect, and clears the failure', async () => {
    renderController({ autoOpenSignIn: true });
    await verifiedChannel();
    failConnect(authRequired());
    await act(async () => {
      await crew.connect({ userInitiated: true });
    });
    mocks.crewHttp.mockClear();
    const observed = mocks.observeCrew.mock.calls.length;
    act(() => crew.onSignedIn());
    await waitFor(() => expect(mocks.observeCrew.mock.calls.length).toBeGreaterThan(observed));
    expect(crew.signIn.open).toBe(false);
    expect(crew.lastConnectFailure).toBeNull();
    // Reload the connections, then refresh (which reloads them again before observing).
    expect(mocks.crewHttp.mock.calls.map(([path]) => path)).toEqual([
      '/connections',
      '/connections',
    ]);
  });

  it('clears a failure once the connection verifies, and reports one a layout saw', async () => {
    renderController();
    await verifiedChannel();
    act(() =>
      crew.reportConnectFailure(
        new CrewHttpError('Signed in, but Crew could not start.', 400, 'crew_handoff_failed')
      )
    );
    expect(crew.lastConnectFailure?.kind).toBe('handoff_failed');
    await act(async () => {
      await crew.refresh();
    });
    await waitFor(() => expect(crew.snapshot).not.toBeNull());
    expect(crew.lastConnectFailure).toBeNull();
  });

  it('disconnects: stops observing, clears the verified view and reads offline', async () => {
    renderController({ keepLastVerifiedView: true });
    await verifiedChannel();
    mocks.crewHttp.mockImplementation(async (path: string) => {
      if (path === '/connections')
        return { connections: [{ ...connection, status: 'disconnected' }] };
      return {};
    });
    const observed = mocks.observeCrew.mock.calls.length;
    await act(async () => {
      await crew.disconnect();
    });
    expect(mocks.crewHttp).toHaveBeenCalledWith('/connections/conn-1/disconnect', 'POST', {});
    expect(crew.snapshot).toBeNull();
    expect(crew.lastVerified).toBeNull();
    expect(crew.status).toBe('offline');
    expect(crew.screen).toBe('offline');
    expect(mocks.observeCrew.mock.calls.length).toBe(observed);
  });

  it('leaves no stale observation error behind a deliberate disconnect', async () => {
    mocks.observeCrew.mockImplementation(async () => {
      throw new CrewHttpError('Crew updates disconnected.', 400, 'observation_refused');
    });
    renderController();
    await waitFor(() => expect(crew.screen).toBe('updates-paused'));
    mocks.crewHttp.mockImplementation(async (path: string) => {
      if (path === '/connections')
        return { connections: [{ ...connection, status: 'disconnected' }] };
      return {};
    });
    await act(async () => {
      await crew.disconnect();
    });
    expect(crew.refreshError).toBeNull();
    expect(crew.screen).toBe('offline');
  });

  it('offers Connect, not Retry, when a saved-disconnected connection’s observer errors', async () => {
    // After an app restart, or once the idle SSH bridge drops, the list says `disconnected` and
    // the daemon answers the observer with an error frame. The person did not disconnect, so the
    // error stays; the screen and the status row must still both read offline.
    mocks.crewHttp.mockImplementation(async (path: string) => {
      if (path === '/connections')
        return { connections: [{ ...connection, status: 'disconnected' }] };
      return {};
    });
    mocks.observeCrew.mockImplementation(
      async (
        _connectionId: string,
        _channelId: string | undefined,
        _after: string | null,
        signal: AbortSignal,
        receive: (frame: unknown) => void
      ) => {
        if (signal.aborted) return 'terminal';
        receive({
          type: 'error',
          error: 'Crew connection is disconnected; authenticate and connect in Crew',
        });
        return 'terminal';
      }
    );
    renderController();
    await waitFor(() => expect(crew.refreshError).not.toBeNull());
    expect(crew.connection?.status).toBe('disconnected');
    expect(crew.lastConnectFailure).toBeNull();
    expect(crew.status).toBe('offline');
    expect(crew.screen).toBe('offline');
    expect(screens).not.toContain('updates-paused');
  });
});

describe('requests, intents and the composer seams', () => {
  it('marks a channel read without refreshing the verified workspace', async () => {
    renderController();
    await verifiedChannel();
    const observed = mocks.observeCrew.mock.calls.length;
    const reads = mocks.crewHttp.mock.calls.length;
    await act(async () => {
      await crew.markRead(channel.id, 'sequence-1');
    });
    expect(mocks.crewRequest).toHaveBeenCalledWith(
      connection.id,
      'channel.read',
      { channel_id: channel.id, sequence: 'sequence-1' },
      true
    );
    expect(mocks.observeCrew.mock.calls.length).toBe(observed);
    expect(mocks.crewHttp.mock.calls.length).toBe(reads);
    expect(crew.snapshot).not.toBeNull();
  });

  it('refreshes after a mutation and closes its dialog, keeping the pane', async () => {
    renderController();
    await verifiedChannel();
    act(() => {
      crew.openPane({ mode: 'agent' });
      crew.openDialog({ kind: 'create-team' });
    });
    const observed = mocks.observeCrew.mock.calls.length;
    await act(async () => {
      await crew.mutate('team.create', { name: 'Imaging' });
    });
    expect(mocks.crewRequest).toHaveBeenCalledWith(
      connection.id,
      'team.create',
      { name: 'Imaging' },
      true
    );
    await waitFor(() => expect(mocks.observeCrew.mock.calls.length).toBeGreaterThan(observed));
    expect(crew.ui).toEqual({ dialog: null, pane: { mode: 'agent' } });
  });

  it('keeps the pane and connection dialogs through a refresh and tells listeners why', async () => {
    const reasons: SurfaceResetReason[] = [];
    function Listener() {
      useCrewSurfaceReset((reason) => reasons.push(reason));
      return null;
    }
    renderController({}, <Listener />);
    await verifiedChannel();
    act(() => {
      crew.openPane({ mode: 'details', tab: 'members' });
      crew.openDialog({ kind: 'connection-settings', connectionId: connection.id });
    });
    reasons.length = 0;
    await act(async () => {
      await crew.refresh();
    });
    expect(reasons).toEqual(['refresh']);
    expect(crew.ui).toEqual({
      dialog: { kind: 'connection-settings', connectionId: connection.id },
      pane: { mode: 'details', tab: 'members' },
    });

    act(() => crew.openDialog({ kind: 'sign-in' }));
    expect(crew.signIn).toEqual({ open: true, reason: 'user' });
    expect(crew.ui.dialog).toEqual({ kind: 'connection-settings', connectionId: connection.id });
    act(() => {
      crew.closeSignIn();
      crew.closeDialog();
      crew.closePane();
    });
    expect(crew.ui).toEqual({ dialog: null, pane: null });
  });

  /** SF2-N7: the reconnect a privacy change causes closed the dialog that made it. */
  it('keeps Workspace settings open while the view is observed again, and closes it when it is gone', async () => {
    const sessions = controllableObserver();
    renderController({ keepLastVerifiedView: true });
    await waitFor(() => expect(sessions.length).toBeGreaterThan(0));
    act(() => sessions[sessions.length - 1]!.receive(stateFrame));
    await waitFor(() => expect(crew.snapshot).not.toBeNull());
    await waitFor(() => expect(sessions[sessions.length - 1]!.channelId).toBe(channel.id));
    act(() => sessions[sessions.length - 1]!.receive(stateFrame));
    act(() => crew.openDialog({ kind: 'workspace-settings', tab: 'privacy' }));

    // The connection's policy moved: the daemon ends the view, and it is observed again.
    act(() =>
      sessions[sessions.length - 1]!.receive({
        type: 'error',
        clear: true,
        code: 'policy_changed',
        error: 'Room observation ended.',
      })
    );
    expect(crew.reverifying).toBe(true);
    expect(crew.ui.dialog).toEqual({ kind: 'workspace-settings', tab: 'privacy' });
    await act(async () => {
      await crew.refresh();
    });
    expect(crew.ui.dialog).toEqual({ kind: 'workspace-settings', tab: 'privacy' });

    // A view that is gone for good takes it with it.
    await act(async () => {
      await crew.disconnect();
    });
    await waitFor(() => expect(crew.ui.dialog).toBeNull());
  });

  it('grants a chat read-and-post access pinned to the verified epochs, and nothing without a chat', async () => {
    renderController();
    await verifiedChannel();
    await act(async () => {
      await crew.grantSession({ contextChannels: [] });
    });
    expect(mocks.crewHttp.mock.calls.some(([path]) => String(path).includes('/grant'))).toBe(false);
    await act(async () => {
      await crew.grantSession({ contextChannels: ['channel-9'], sessionId: 'chat 1' });
    });
    expect(mocks.crewHttp).toHaveBeenCalledWith(
      '/connections/conn-1/sessions/chat%201/grant',
      'POST',
      {
        expected_mode: 'private',
        expected_policy_epoch: 1,
        expected_workspace_policy_epoch: 1,
        channel_id: channel.id,
        context_channels: [channel.id, 'channel-9'],
      }
    );
  });

  it('stops a task at its own route however the broker spelled its run ID (RENDERER-2)', async () => {
    renderController();
    await verifiedChannel();
    // The run ID is the broker's. Interpolated raw, its dot segments sent this POST, with the
    // person's proof, to `/crew/credentials/lock`.
    await act(async () => {
      await crew.cancelRun('../../../credentials/lock?');
    });
    const cancel = mocks.crewHttp.mock.calls.find(([, method]) => method === 'POST');
    expect(cancel).toEqual([
      '/connections/conn-1/runs/..%2F..%2F..%2Fcredentials%2Flock%3F/cancel',
      'POST',
      {},
    ]);
    const url = new URL(`http://127.0.0.1/crew${cancel?.[0]}`);
    expect(url.pathname).toBe(
      '/crew/connections/conn-1/runs/..%2F..%2F..%2Fcredentials%2Flock%3F/cancel'
    );
    expect(url.search).toBe('');
  });

  it('clears the composer only when it still holds the seed', async () => {
    renderController();
    await verifiedChannel();
    act(() => crew.setBody('run the analysis'));
    act(() => crew.clearBodyIfEquals('run the analysis again'));
    expect(crew.draft.body).toBe('run the analysis');
    act(() => crew.clearBodyIfEquals('run the analysis'));
    expect(crew.draft.body).toBe('');
  });

  it('derives host, privacy and names from the verified snapshot only', async () => {
    renderController();
    await verifiedChannel();
    expect(crew.isHost).toBe(true);
    expect(crew.effectivePrivacy).toBe('private');
    expect(crew.status).toBe('connected');
    expect(crew.team?.name).toBe('Lab');
    expect(crew.channel?.name).toBe('general');
    expect(crew.connection).toMatchObject({ id: connection.id, mode: 'private' });
  });

  it('reads a join status other than joined as not a member yet, scoped to the connection', async () => {
    mocks.observeCrew.mockImplementation(async () => {
      throw new CrewHttpError('Room observation ended.', 400, 'observation_refused');
    });
    renderController();
    await waitFor(() => expect(crew.refreshError).not.toBeNull());
    expect(crew.screen).toBe('updates-paused');
    act(() => crew.setJoinStatus('invited'));
    expect(crew.screen).toBe('join');
    expect(crew.status).toBe('not-joined');
    act(() => crew.setJoinStatus('unsupported'));
    expect(crew.screen).toBe('updates-paused');
  });
});

describe('keeping one live observer', () => {
  // Named to sort after "Lab": with nothing chosen, Crew opens the first team by name (setup F6).
  const imaging = {
    id: 'team-2',
    name: 'Microscopy Imaging',
    created_by: actor.id,
    members: [actor.id],
    general_channel_id: 'channel-2',
  };
  const imagingGeneral = { ...channel, id: 'channel-2', team_id: 'team-2' };

  it('starts a new observer when a team is selected whose channel does not change (T-08)', async () => {
    // A workspace with no team yet: the observer watches no channel.
    const empty = { ...snapshot, teams: [], channels: [] };
    const sessions = controllableObserver();
    renderController();
    await waitFor(() => expect(sessions).toHaveLength(1));
    act(() => sessions[0]!.receive({ ...stateFrame, snapshot: empty }));
    await waitFor(() => expect(crew.screen).toBe('no-team'));
    expect(crew.channelId).toBe('');

    // Create Team answers, then selects the team the verified view does not have yet.
    act(() => crew.selectTeam(imaging.id));
    await waitFor(() => expect(sessions.length).toBeGreaterThan(1));
    expect(sessions[0]!.signal.aborted).toBe(true);
    const next = sessions[sessions.length - 1]!;
    act(() =>
      next.receive({
        ...stateFrame,
        snapshot: { ...snapshot, teams: [imaging], channels: [imagingGeneral] },
      })
    );
    await waitFor(() => expect(crew.team?.name).toBe(imaging.name));
    await waitFor(() => expect(crew.channelId).toBe(imagingGeneral.id));
  });

  it('starts one observer, for the new channel, when a team with another channel is selected', async () => {
    const sessions = controllableObserver();
    renderController();
    await waitFor(() => expect(sessions).toHaveLength(1));
    const withImaging = {
      ...snapshot,
      teams: [...snapshot.teams, imaging],
      channels: [channel, imagingGeneral],
    };
    act(() => sessions[0]!.receive({ ...stateFrame, snapshot: withImaging }));
    await waitFor(() => expect(sessions).toHaveLength(2));
    expect(sessions[1]!.channelId).toBe(channel.id);
    await waitFor(() => expect(crew.channelId).toBe(channel.id));

    act(() => crew.selectTeam(imaging.id));
    await waitFor(() => expect(crew.channelId).toBe(imagingGeneral.id));
    await waitFor(() => expect(sessions).toHaveLength(3));
    // Never a restart for the old channel on the way: the channel's own change starts the new one.
    expect(sessions[2]!.channelId).toBe(imagingGeneral.id);
    expect(sessions[1]!.signal.aborted).toBe(true);
    expect(sessions[2]!.signal.aborted).toBe(false);
  });

  it('observes again when the selected connection is selected again', async () => {
    renderController();
    await verifiedChannel();
    const observed = mocks.observeCrew.mock.calls.length;
    act(() => crew.selectConnection(connection.id));
    await waitFor(() => expect(mocks.observeCrew.mock.calls.length).toBeGreaterThan(observed));
    await waitFor(() => expect(crew.snapshot).not.toBeNull());
  });

  it('reads the list again when the window comes back, so a terminal’s connection shows (T-51)', async () => {
    renderController();
    await verifiedChannel();
    const added = { ...connection, id: 'conn-2', name: 'From the terminal' };
    mocks.crewHttp.mockImplementation(async (path: string) => {
      if (path === '/connections') return { connections: [connection, added] };
      return {};
    });
    act(() => {
      window.dispatchEvent(new Event('focus'));
      document.dispatchEvent(new Event('visibilitychange'));
    });
    await waitFor(() =>
      expect(crew.connections.map((item) => item.id)).toEqual(['conn-1', 'conn-2'])
    );
    // Focus and visibility together are read once.
    expect(mocks.crewHttp.mock.calls.filter(([path]) => path === '/connections')).toHaveLength(2);
    expect(crew.connectionId).toBe(connection.id);
  });

  it('says a workspace chosen after it was removed from this computer was removed (MSG2-N9)', async () => {
    const gone = { ...connection, id: 'conn-2', name: 'bob-cap2' };
    mocks.crewHttp.mockImplementation(async (path: string) => {
      if (path === '/connections') return { connections: [connection, gone] };
      return {};
    });
    renderController();
    await verifiedChannel();
    await waitFor(() => expect(crew.connections).toHaveLength(2));
    // Removed from a terminal; the menu still lists it.
    mocks.crewHttp.mockImplementation(async (path: string) => {
      if (path === '/connections') return { connections: [connection] };
      return {};
    });
    act(() => crew.selectConnection(gone.id));
    await waitFor(() =>
      expect(crew.error?.message).toBe(crewActionCopy.workspaceRemoved('bob-cap2'))
    );
    expect(crew.connectionId).toBe(connection.id);
    expect(crew.connections.map((item) => item.id)).toEqual([connection.id]);
  });

  it('reads "Updating…" while it observes again by itself, never "Updates unavailable"', async () => {
    renderController({ keepLastVerifiedView: true });
    await verifiedChannel();
    const sessions = controllableObserver();
    await act(async () => {
      await crew.refresh();
    });
    await waitFor(() => expect(sessions.length).toBeGreaterThan(0));
    act(() => sessions[sessions.length - 1]!.receive(stateFrame));
    await waitFor(() => expect(crew.status).toBe('connected'));
    const ended = sessions.length;
    act(() =>
      sessions[sessions.length - 1]!.receive({
        type: 'error',
        clear: true,
        code: 'policy_changed',
        error: 'Room observation ended.',
      })
    );
    expect(crew.status).toBe('updating');
    expect(crew.reverifying).toBe(true);
    expect(crew.refreshError).toBeNull();
    expect(crew.snapshot).toBeNull();
    expect(crew.lastVerified).toBeNull();
    // It observes again by itself (after 0.3 s), with no Retry.
    await waitFor(() => expect(sessions.length).toBeGreaterThan(ended));
    const again = sessions[sessions.length - 1]!;
    act(() => again.receive(stateFrame));
    await waitFor(() => expect(crew.status).toBe('connected'));
    expect(crew.reverifying).toBe(false);
  });
});

describe('how often a dropped view is observed again quietly (Q2-01, SECURITY-SENSITIVE)', () => {
  const now = 1_000_000;
  const [, second, third] = QUIET_REOBSERVE_GAPS_MS;

  it('grows the gap: the first at once, the second after 20 s, the third after 60 s', () => {
    expect(QUIET_REOBSERVE_GAPS_MS).toEqual([0, 20_000, 60_000]);
    expect(takeQuietReobserve('conn-1', now)).toBe(true);
    expect(takeQuietReobserve('conn-1', now + second! - 1)).toBe(false);
    expect(takeQuietReobserve('conn-1', now + second!)).toBe(true);
    expect(takeQuietReobserve('conn-1', now + second! + third! - 1)).toBe(false);
    expect(takeQuietReobserve('conn-1', now + second! + third!)).toBe(true);
  });

  it('is capped: a bridge that keeps dropping is shown, however far apart the drops are', () => {
    // Every 61 s, as a bridge that fails a minute after each repair would.
    const drops = [0, 61_000, 122_000, 183_000, 244_000, 305_000].map((at) => now + at);
    expect(drops.map((at) => takeQuietReobserve('conn-1', at))).toEqual([
      true,
      true,
      true,
      false,
      false,
      false,
    ]);
    // Only once the window has passed the earlier ones does a drop get one again.
    expect(takeQuietReobserve('conn-1', now + QUIET_REOBSERVE_WINDOW_MS + 122_000)).toBe(true);
  });

  it('counts each connection on its own, forgets a removed one, and never takes an empty id', () => {
    for (const at of [0, 20_000, 80_000]) expect(takeQuietReobserve('conn-1', now + at)).toBe(true);
    expect(takeQuietReobserve('conn-1', now + 500_000)).toBe(false);
    expect(takeQuietReobserve('conn-2', now + 500_000)).toBe(true);
    forgetConnectionMemory('conn-1');
    expect(takeQuietReobserve('conn-1', now + 500_000)).toBe(true);
    expect(takeQuietReobserve('', now)).toBe(false);
  });
});

describe('the channel Crew opens, and the draft each channel keeps (Q2-07, Q2-10, Q2-21)', () => {
  const methods = {
    ...channel,
    id: 'channel-3',
    name: 'methods',
    classification: 'public_safe' as const,
  };
  // Named to sort after "Lab": with nothing chosen, Crew opens the first team by name (setup F6).
  const imaging = {
    id: 'team-2',
    name: 'Microscopy Imaging',
    created_by: actor.id,
    members: [actor.id],
    general_channel_id: 'channel-2',
  };
  const imagingGeneral = { ...channel, id: 'channel-2', team_id: 'team-2', name: 'imaging' };
  const workspace = {
    ...snapshot,
    teams: [...snapshot.teams, imaging],
    channels: [channel, methods, imagingGeneral],
  };
  let view: typeof workspace;
  let mode: 'private' | 'public';

  beforeEach(() => {
    view = workspace;
    mode = 'private';
    mocks.observeCrew.mockImplementation(
      async (
        _connectionId: string,
        channelId: string | undefined,
        _after: string | null,
        signal: AbortSignal,
        receive: (frame: unknown) => void
      ) => {
        if (signal.aborted) return 'terminal';
        receive({
          ...stateFrame,
          connection_mode: mode,
          snapshot: { ...view, workspace: { ...view.workspace, mode } },
        });
        if (channelId)
          receive({
            type: 'messages',
            channel_id: channelId,
            messages: [],
            cursor: null,
            reset: true,
          });
        return 'terminal';
      }
    );
  });

  async function opened(channelId: string) {
    await waitFor(() => expect(crew.channelId).toBe(channelId));
    await waitFor(() => expect(crew.snapshot).not.toBeNull());
    await waitFor(() => expect(crew.messagesLoaded).toBe(true));
  }

  it('picks the team of a channel in another team, so the team does not undo it (Q2-10)', async () => {
    renderController();
    await opened(channel.id);
    act(() => crew.selectChannel(imagingGeneral.id));
    await opened(imagingGeneral.id);
    expect(crew.teamId).toBe(imaging.id);
    expect(crew.team?.name).toBe(imaging.name);
    // Frames that follow keep it there.
    await act(async () => {
      await crew.refresh();
    });
    await opened(imagingGeneral.id);
    expect(crew.teamId).toBe(imaging.id);
  });

  it('reopens on the channel the person last chose, across teams (Q2-21)', async () => {
    const first = renderController();
    await opened(channel.id);
    act(() => crew.selectChannel(imagingGeneral.id));
    await opened(imagingGeneral.id);
    first.unmount();

    renderController();
    await opened(imagingGeneral.id);
    expect(crew.teamId).toBe(imaging.id);
  });

  it('remembers the channel a team selection opens', async () => {
    const first = renderController();
    await opened(channel.id);
    act(() => crew.selectTeam(imaging.id));
    await opened(imagingGeneral.id);
    first.unmount();

    renderController();
    await opened(imagingGeneral.id);
  });

  it('falls back to the first open channel when the remembered one is gone or archived', async () => {
    rememberLastChannel(connection.id, 'channel-gone');
    const first = renderController();
    await opened(channel.id);
    first.unmount();

    rememberLastChannel(connection.id, methods.id);
    view = { ...workspace, channels: [channel, { ...methods, archived: true }, imagingGeneral] };
    renderController();
    await opened(channel.id);
  });

  it('keeps the body through #general → #methods → #general', async () => {
    renderController();
    await opened(channel.id);
    act(() => crew.setBody('for #general'));
    act(() => crew.selectChannel(methods.id));
    await opened(methods.id);
    expect(crew.draft.body).toBe('');
    act(() => crew.setBody('for #methods'));

    act(() => crew.selectChannel(channel.id));
    await opened(channel.id);
    await waitFor(() => expect(crew.draft.body).toBe('for #general'));
    act(() => crew.selectChannel(methods.id));
    await opened(methods.id);
    await waitFor(() => expect(crew.draft.body).toBe('for #methods'));
  });

  it('keeps the body when Crew is left and opened again', async () => {
    const first = renderController();
    await opened(channel.id);
    act(() => crew.setBody('written before leaving'));
    first.unmount();

    renderController();
    await opened(channel.id);
    await waitFor(() => expect(crew.draft.body).toBe('written before leaving'));
    // Handed back once: it is no longer kept aside.
    expect(stashedDraft(connection.id, channel.id)).toBeUndefined();
  });

  it('drops the body when the workspace became public while it was put aside', async () => {
    renderController();
    await opened(channel.id);
    act(() => crew.setBody('sensitive words'));
    act(() => crew.selectChannel(methods.id));
    await opened(methods.id);
    expect(stashedDraft(connection.id, channel.id)?.body).toBe('sensitive words');

    // The next verified view is Public: nothing written under Private may come back.
    mode = 'public';
    await act(async () => {
      await crew.refresh();
    });
    await opened(methods.id);
    expect(stashedDraft(connection.id, channel.id)).toBeUndefined();
    act(() => crew.selectChannel(channel.id));
    await opened(channel.id);
    expect(crew.draft.body).toBe('');
  });

  it('drops the body kept across leaving Crew when the privacy it was written under moved', async () => {
    const first = renderController();
    await opened(channel.id);
    act(() => crew.setBody('sensitive words'));
    first.unmount();

    mode = 'public';
    renderController();
    await opened(channel.id);
    expect(crew.draft.body).toBe('');
    expect(stashedDraft(connection.id, channel.id)).toBeUndefined();
  });

  it('forgets the body of a channel the person can no longer see', async () => {
    renderController();
    await opened(channel.id);
    act(() => crew.setBody('for #general'));
    act(() => crew.selectChannel(methods.id));
    await opened(methods.id);

    // Removed from #general: the next verified view no longer offers it.
    view = { ...workspace, channels: [methods, imagingGeneral] };
    await act(async () => {
      await crew.refresh();
    });
    await opened(methods.id);
    expect(stashedDraft(connection.id, channel.id)).toBeUndefined();

    // Added back later: a fresh start, not the old draft.
    view = workspace;
    act(() => crew.selectChannel(channel.id));
    await opened(channel.id);
    expect(crew.draft.body).toBe('');
  });

  it('never hands back an attachment, a reference or a context channel: the body only', async () => {
    renderController();
    await opened(channel.id);
    act(() => {
      crew.setBody('see the file');
      crew.addAttachment({ id: 'blob-1', name: 'counts.csv' });
      crew.addReference({ id: 'ref-1', label: '/data/run-1' });
      crew.setContextChannels([methods.id]);
    });
    act(() => crew.selectChannel(methods.id));
    await opened(methods.id);
    act(() => crew.selectChannel(channel.id));
    await opened(channel.id);
    await waitFor(() => expect(crew.draft.body).toBe('see the file'));
    expect(crew.draft.attachments).toEqual([]);
    expect(crew.draft.references).toEqual([]);
    expect(crew.contextChannels).toEqual([]);
  });

  describe('a kept draft comes back as its channel is selected, not when it loads (Q4-05)', () => {
    /** Every render's channel, body and whether its history has loaded. */
    let renders: { channelId: string; body: string; loaded: boolean }[];
    function Recorder() {
      const current = useCrew();
      renders.push({
        channelId: current.channelId,
        body: current.draft.body,
        loaded: current.messagesLoaded,
      });
      return null;
    }
    let sessions: Observation[];
    const latest = () => sessions[sessions.length - 1]!;
    const withClassification = (classification: string) => ({
      ...workspace,
      channels: [{ ...channel, classification }, methods, imagingGeneral],
    });

    /** #general verified with 'for #general' written in it, then #methods selected and verified. */
    async function draftLeftInGeneral() {
      renders = [];
      sessions = controllableObserver();
      renderController(undefined, <Recorder />);
      await waitFor(() => expect(sessions.length).toBeGreaterThan(0));
      act(() => latest().receive({ ...stateFrame, snapshot: view }));
      await waitFor(() => expect(latest().channelId).toBe(channel.id));
      act(() => latest().receive({ ...stateFrame, snapshot: view }));
      act(() => crew.setBody('for #general'));
      act(() => crew.selectChannel(methods.id));
      await waitFor(() => expect(latest().channelId).toBe(methods.id));
    }

    /** The first render on `channelId` after `from`. */
    const firstRenderOn = (channelId: string, from: number) =>
      renders.slice(from).find((render) => render.channelId === channelId);

    it('holds the draft in the very render that selects the channel, while its history loads', async () => {
      await draftLeftInGeneral();
      act(() => latest().receive({ ...stateFrame, snapshot: view }));
      expect(stashedDraft(connection.id, channel.id)?.body).toBe('for #general');

      const from = renders.length;
      act(() => crew.selectChannel(channel.id));
      expect(firstRenderOn(channel.id, from)).toEqual({
        channelId: channel.id,
        body: 'for #general',
        loaded: false,
      });

      // Its own first frame keeps it there, and takes it out of the stash.
      await waitFor(() => expect(latest().channelId).toBe(channel.id));
      act(() => latest().receive({ ...stateFrame, snapshot: view }));
      expect(crew.draft.body).toBe('for #general');
      expect(stashedDraft(connection.id, channel.id)).toBeUndefined();
    });

    it('loses nothing when the person leaves again before the channel’s first frame', async () => {
      await draftLeftInGeneral();
      act(() => latest().receive({ ...stateFrame, snapshot: view }));
      act(() => crew.selectChannel(channel.id));
      expect(crew.draft.body).toBe('for #general');
      act(() => crew.selectChannel(methods.id));
      expect(crew.draft.body).toBe('');
      expect(stashedDraft(connection.id, channel.id)?.body).toBe('for #general');
      act(() => crew.selectChannel(channel.id));
      expect(crew.draft.body).toBe('for #general');
    });

    it('never puts it back when the view current at the selection moved its channel’s classification', async () => {
      await draftLeftInGeneral();
      // #methods' verified view says #general is not restricted any more.
      act(() => latest().receive({ ...stateFrame, snapshot: withClassification('public_safe') }));

      const from = renders.length;
      act(() => crew.selectChannel(channel.id));
      expect(firstRenderOn(channel.id, from)?.body).toBe('');
      expect(crew.draft.body).toBe('');
      await waitFor(() => expect(latest().channelId).toBe(channel.id));
      act(() => latest().receive({ ...stateFrame, snapshot: withClassification('public_safe') }));
      expect(crew.draft.body).toBe('');
      expect(stashedDraft(connection.id, channel.id)).toBeUndefined();
    });

    it('clears one put back when the channel’s own first frame shows its classification moved', async () => {
      await draftLeftInGeneral();
      act(() => latest().receive({ ...stateFrame, snapshot: view }));
      act(() => crew.selectChannel(channel.id));
      expect(crew.draft.body).toBe('for #general');

      await waitFor(() => expect(latest().channelId).toBe(channel.id));
      act(() => latest().receive({ ...stateFrame, snapshot: withClassification('public_safe') }));
      expect(crew.draft.body).toBe('');
      expect(crew.error?.message).toBe(crewObservationCopy.scopeChanged);
      expect(stashedDraft(connection.id, channel.id)).toBeUndefined();
    });

    it('puts it back on arrival, when Crew itself opens the channel, before its history', async () => {
      const first = renderController();
      await opened(channel.id);
      act(() => crew.setBody('written before leaving'));
      first.unmount();

      renders = [];
      sessions = controllableObserver();
      renderController(undefined, <Recorder />);
      await waitFor(() => expect(sessions.length).toBeGreaterThan(0));
      // The workspace verifies, and Crew opens #general by itself: the draft is in its first
      // render there, while #general's own frames are still to come.
      act(() => latest().receive({ ...stateFrame, snapshot: view }));
      await waitFor(() => expect(crew.channelId).toBe(channel.id));
      expect(crew.draft.body).toBe('written before leaving');
      expect(crew.messagesLoaded).toBe(false);
    });
  });

  /**
   * MSG2-N4: the stash kept nothing over 64 KB, the message limit itself, so the very draft the
   * composer had just told the person to attach as a file went when they switched channel.
   */
  it('keeps a draft over the message limit, and its note, through a channel switch (MSG2-N4)', async () => {
    renderController();
    await opened(channel.id);
    const long = 'x'.repeat(70_007);
    act(() => crew.setBody(long));
    await act(async () => {
      await crew.send();
    });
    expect(crew.error?.message).toBe(composerCopy.tooLong);
    act(() => crew.selectChannel(methods.id));
    await opened(methods.id);
    expect(crew.draft.body).toBe('');
    act(() => crew.selectChannel(channel.id));
    await opened(channel.id);
    await waitFor(() => expect(crew.draft.body).toBe(long));
    expect(crew.error?.message).toBe(composerCopy.tooLong);
    expect(crew.error?.source).toBe('composer');
  });

  /**
   * MSG2-N3: a draft was kept only under the scope recorded from its own channel's first frame. The
   * words of a post sent in the second after the channel was selected had none, and were dropped
   * when the person moved on, so a post refused after that lost its text.
   */
  it('keeps the words of a post sent before its channel verified, when it fails after the person moved on (MSG2-N3)', async () => {
    const sessions = controllableObserver();
    const latest = () => sessions[sessions.length - 1]!;
    const post = deferred<unknown>();
    mocks.crewRequest.mockImplementation(async (_connection: string, method: string) =>
      method === 'message.post' ? post.promise : {}
    );
    renderController();
    await waitFor(() => expect(sessions.length).toBeGreaterThan(0));
    act(() => latest().receive({ ...stateFrame, snapshot: view }));
    await waitFor(() => expect(latest().channelId).toBe(channel.id));
    act(() => latest().receive({ ...stateFrame, snapshot: view }));

    // #methods is selected, and the person writes and sends before its first frame arrives…
    act(() => crew.selectChannel(methods.id));
    await waitFor(() => expect(latest().channelId).toBe(methods.id));
    act(() => crew.setBody('for #methods'));
    let sending: Promise<void> = Promise.resolve();
    act(() => {
      sending = crew.send();
    });
    await waitFor(() =>
      expect(mocks.crewRequest.mock.calls.some(([, method]) => method === 'message.post')).toBe(
        true
      )
    );
    // …then moves on while it is out: the words wait for #methods.
    act(() => crew.selectChannel(channel.id));
    expect(crew.draft.body).toBe('');
    expect(stashedDraft(connection.id, methods.id)?.body).toBe('for #methods');

    // The connection drops before the post is written.
    await act(async () => {
      post.reject(new CrewHttpError('Nothing was sent', 503, CREW_NOT_SENT));
      await sending;
    });
    expect(crew.error?.message).toContain('#methods');

    // Back in #methods: its draft, and what was said about it.
    act(() => crew.selectChannel(methods.id));
    expect(crew.draft.body).toBe('for #methods');
    await waitFor(() => expect(crew.error?.source).toBe('composer'));
    expect(crew.error?.message).toBe(composerCopy.notSent);
  });

  it('never overwrites a newer draft the person typed before the channel verified again', async () => {
    const first = renderController();
    await opened(channel.id);
    act(() => crew.setBody('older'));
    first.unmount();

    const sessions = controllableObserver();
    renderController();
    await waitFor(() => expect(sessions).toHaveLength(1));
    act(() => sessions[0]!.receive({ ...stateFrame, snapshot: view }));
    await waitFor(() => expect(sessions).toHaveLength(2));
    expect(sessions[1]!.channelId).toBe(channel.id);
    // The person types before #general's own view arrives: theirs wins, the older is dropped.
    act(() => crew.setBody('newer'));
    act(() => sessions[1]!.receive({ ...stateFrame, snapshot: view }));
    await waitFor(() => expect(crew.snapshot).not.toBeNull());
    expect(crew.draft.body).toBe('newer');
    expect(stashedDraft(connection.id, channel.id)).toBeUndefined();
  });
});

describe('the team and channel a verified view picks (pure)', () => {
  const view = {
    teams: [{ id: 'team-1' }, { id: 'team-2' }],
    channels: [
      { id: 'general', team_id: 'team-1', archived: false },
      { id: 'methods', team_id: 'team-1', archived: false },
      { id: 'old', team_id: 'team-1', archived: true },
      { id: 'imaging', team_id: 'team-2', archived: false },
    ],
  } as unknown as Parameters<typeof teamForView>[0];

  it('teamForView: the selected channel’s team, then the team shown, then the last channel’s', () => {
    expect(teamForView(view, 'team-1', 'imaging', null)).toBe('team-2');
    expect(teamForView(view, 'team-2', '', 'general')).toBe('team-2');
    expect(teamForView(view, '', '', 'imaging')).toBe('team-2');
    expect(teamForView(view, '', '', 'old')).toBe('team-1');
    expect(teamForView(view, '', '', 'missing')).toBe('team-1');
    expect(teamForView({ ...view, teams: [] }, '', '', 'imaging')).toBe('');
  });

  it('channelForTeam: the current channel, then the remembered one, then the first open one', () => {
    expect(channelForTeam(view, 'team-1', 'methods', 'general')).toBe('methods');
    expect(channelForTeam(view, 'team-1', '', 'methods')).toBe('methods');
    expect(channelForTeam(view, 'team-1', '', 'old')).toBe('general');
    expect(channelForTeam(view, 'team-1', '', 'imaging')).toBe('general');
    expect(channelForTeam(view, 'team-2', 'general', null)).toBe('imaging');
  });

  /**
   * Setup F6: channels and teams arrive in the order of their random IDs, so a new member with
   * nothing remembered landed on whichever sorted first: #random, for Bob, Henry and Mallory.
   */
  describe('with nothing chosen yet (setup F6)', () => {
    const byId = {
      teams: [
        { id: '4c78-zeta', name: 'Zeta Core', general_channel_id: 'zeta-general' },
        { id: '9f00-chen', name: 'chen-lab', general_channel_id: '630e-general' },
      ],
      channels: [
        { id: '4c78-random', team_id: '9f00-chen', name: 'random', archived: false },
        { id: '5ee2-methods', team_id: '9f00-chen', name: 'methods', archived: false },
        { id: '630e-general', team_id: '9f00-chen', name: 'general', archived: false },
        { id: 'zeta-general', team_id: '4c78-zeta', name: 'general', archived: false },
      ],
    } as unknown as Parameters<typeof teamForView>[0];

    it('opens the team’s #general, not the channel whose ID sorts first', () => {
      expect(channelForTeam(byId, '9f00-chen', '', null)).toBe('630e-general');
      // A remembered channel still wins, and an archived #general gives way to the first open one.
      expect(channelForTeam(byId, '9f00-chen', '', '5ee2-methods')).toBe('5ee2-methods');
      const archived = {
        ...byId,
        channels: byId.channels.map((item) =>
          item.id === '630e-general' ? { ...item, archived: true } : item
        ),
      };
      expect(channelForTeam(archived, '9f00-chen', '', null)).toBe('4c78-random');
    });

    it('opens the first team by name, not the team whose ID sorts first', () => {
      expect(teamForView(byId, '', '', null)).toBe('9f00-chen');
    });
  });
});

describe('CrewView', () => {
  it('provides the controller to an injected layout and passes the options through', async () => {
    const { default: CrewView } = await import('../CrewView');
    const { useCrew } = await import('./CrewControllerContext');
    function Probe() {
      const controller = useCrew();
      return <p>screen: {controller.screen}</p>;
    }
    render(
      <MemoryRouter initialEntries={['/crew']}>
        <CrewView layout={Probe} controllerOptions={{ keepLastVerifiedView: true }} />
      </MemoryRouter>
    );
    expect(await screen.findByText('screen: channel')).toBeInTheDocument();
    expect(screen.queryByTestId('crew-view')).toBeNull();
  });

  it('refuses to be read outside a CrewView', async () => {
    const { useCrew } = await import('./CrewControllerContext');
    function Orphan() {
      useCrew();
      return null;
    }
    vi.spyOn(console, 'error').mockImplementation(() => {});
    expect(() => render(<Orphan />)).toThrow('useCrew() must be called inside a CrewView.');
  });
});

describe('after a post (Q3-10, Q3-03)', () => {
  const posted = {
    id: 'message-2',
    sequence: 'message-2',
    channel_id: channel.id,
    actor_id: actor.id,
    body: 'hi',
    created_at: 1_700_000_100,
    restricted: false,
    source_channels: [channel.id],
    attachments: [],
  };
  const reads = () =>
    mocks.crewRequest.mock.calls.filter(([, method]) => method === 'channel.read');

  it('reads the channel up to the posted message, without refreshing the view', async () => {
    mocks.crewRequest.mockImplementation(async (_connection: string, method: string) =>
      method === 'message.post' ? posted : {}
    );
    renderController();
    await verifiedChannel();
    const observed = mocks.observeCrew.mock.calls.length;
    act(() => crew.setBody('hi'));
    await act(async () => {
      await crew.send();
    });
    await waitFor(() => expect(reads()).toHaveLength(1));
    expect(mocks.crewRequest).toHaveBeenCalledWith(
      connection.id,
      'channel.read',
      { channel_id: channel.id, sequence: 'message-2' },
      true
    );
    expect(mocks.observeCrew.mock.calls.length).toBe(observed);
    expect(crew.draft.body).toBe('');
  });

  it('reads nothing when the post answers without a sequence', async () => {
    renderController();
    await verifiedChannel();
    act(() => crew.setBody('hi'));
    await act(async () => {
      await crew.send();
    });
    expect(mocks.crewRequest.mock.calls.some(([, method]) => method === 'message.post')).toBe(true);
    expect(reads()).toHaveLength(0);
  });

  it('says nothing when that read fails: the message was posted', async () => {
    mocks.crewHttp.mockImplementation(async (path: string) => {
      if (path === '/connections') return { connections: [connection] };
      if (path.startsWith('/transfers?')) return { transfers: [] };
      return {};
    });
    mocks.crewRequest.mockImplementation(async (_connection: string, method: string) => {
      if (method === 'channel.read') throw new Error('read failed');
      return method === 'message.post' ? posted : {};
    });
    renderController();
    await verifiedChannel();
    act(() => crew.setBody('hi'));
    await act(async () => {
      await crew.send();
    });
    await waitFor(() => expect(reads()).toHaveLength(1));
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20));
    });
    expect(crew.error).toBeNull();
    expect(crew.draft.body).toBe('');
  });

  it('lists the transfers again at once, so a sent file’s forgotten record leaves the Files tab', async () => {
    const record = {
      id: 'transfer-1',
      request_id: 'request-1',
      connection_id: connection.id,
      channel_id: channel.id,
      direction: 'upload',
      name: 'counts.csv',
      size: 10,
      sha256: 'a'.repeat(64),
      offset: 10,
      blob_id: 'blob-1',
      state: 'completed',
      error: null,
    };
    let records = [record];
    mocks.crewHttp.mockImplementation(async (path: string, method = 'GET') => {
      if (path === '/connections') return { connections: [connection] };
      if (path.startsWith('/transfers?')) return { transfers: records };
      if (path === '/transfers/transfer-1' && method === 'GET') return record;
      if (path === '/transfers/transfer-1' && method === 'DELETE') {
        records = [];
        return {};
      }
      return {};
    });
    function Transfers() {
      const { transfers } = useCrewTransfers(connection.id);
      return (
        <ul aria-label="Transfers">
          {transfers.map((item) => (
            <li key={item.id}>{item.name}</li>
          ))}
        </ul>
      );
    }
    renderController({}, <Transfers />);
    await verifiedChannel();
    expect(await screen.findByText('counts.csv')).toBeInTheDocument();
    act(() => {
      crew.setBody('the table');
      crew.addAttachment({ id: 'blob-1', name: 'counts.csv' });
    });
    await act(async () => {
      await crew.send();
    });
    // Nothing is moving, so the shared poller would not have asked again by itself.
    await waitFor(() => expect(screen.queryByText('counts.csv')).toBeNull());
    expect(crew.draft.attachments).toEqual([]);
  });
});

describe('connect on arrival from a chat (Q3-08)', () => {
  const CONNECT = '/connections/conn-1/connect';
  const connects = () =>
    mocks.crewHttp.mock.calls.filter(([path, method]) => path === CONNECT && method === 'POST')
      .length;
  let saved: Record<string, unknown>[];

  function serve() {
    mocks.crewHttp.mockImplementation(async (path: string, method = 'GET') => {
      if (path === '/connections') return { connections: saved };
      if (path === CONNECT && method === 'POST') {
        saved = saved.map((item) =>
          item.id === 'conn-1' ? { ...item, status: 'connected', last_error_code: undefined } : item
        );
        return {};
      }
      return {};
    });
  }
  function arrival(intentId = 'intent-1', connectionId = 'conn-1'): InitialEntry {
    return {
      pathname: '/crew',
      search: '?sessionId=session-1',
      state: { [CHAT_ACCESS_INTENT_ROUTE_KEY]: intentId, [CREW_CONNECT_ROUTE_KEY]: connectionId },
    };
  }

  /** The daemon observes only a connection it calls connected, as the real one does. */
  function observeWhenConnected() {
    mocks.observeCrew.mockImplementation(
      async (
        connectionId: string,
        channelId: string | undefined,
        _after: string | null,
        signal: AbortSignal,
        receive: (frame: unknown) => void
      ) => {
        if (signal.aborted) return 'terminal';
        const record = saved.find((item) => item.id === connectionId);
        if (record?.status !== 'connected') {
          receive({
            type: 'error',
            code: 'observation_refused',
            clear: true,
            error: 'Crew connection is not connected',
          });
          return 'terminal';
        }
        receive({ ...stateFrame, connection_id: connectionId });
        if (channelId) receive(messagesFrame);
        return 'terminal';
      }
    );
  }

  beforeEach(() => {
    saved = [{ ...connection, status: 'disconnected' }];
    serve();
    observeWhenConnected();
  });

  it('reads the intent id under the key the chat’s route state uses', () => {
    expect(Object.keys(chatAccessRouteState())).toEqual([CHAT_ACCESS_INTENT_ROUTE_KEY]);
  });

  it('connects a disconnected connection once, as the person, and not again on a remount', async () => {
    const first = renderController({ autoOpenSignIn: true }, null, arrival());
    await verifiedChannel();
    expect(connects()).toBe(1);
    expect(crew.status).toBe('connected');

    first.unmount();
    saved = [{ ...connection, status: 'disconnected' }];
    renderController({ autoOpenSignIn: true }, null, arrival());
    await waitFor(() => expect(crew.connectionsState).toBe('loaded'));
    await waitFor(() => expect(crew.screen).toBe('offline'));
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20));
    });
    expect(connects()).toBe(1);
  });

  it('does not connect again on a reload of the same history entry', async () => {
    // A reload keeps the history entry and this window's session storage, not this module's memory.
    window.sessionStorage.setItem(ARRIVAL_CONNECT_STORAGE_KEY, JSON.stringify(['intent-1']));
    renderController({ autoOpenSignIn: true }, null, arrival());
    await waitFor(() => expect(crew.screen).toBe('offline'));
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20));
    });
    expect(connects()).toBe(0);
  });

  it('connects nothing when the connection is already connected, and spends the intent', async () => {
    saved = [connection];
    const first = renderController({}, null, arrival());
    await verifiedChannel();
    expect(connects()).toBe(0);

    // It drops later: coming back to the same entry still never connects it.
    first.unmount();
    saved = [{ ...connection, status: 'disconnected' }];
    renderController({}, null, arrival());
    await waitFor(() => expect(crew.screen).toBe('offline'));
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20));
    });
    expect(connects()).toBe(0);
  });

  it('connects nothing without an intent id, or for a connection it does not know', async () => {
    renderController({}, null, {
      pathname: '/crew',
      state: { [CREW_CONNECT_ROUTE_KEY]: 'conn-1' },
    });
    await waitFor(() => expect(crew.screen).toBe('offline'));
    const other = renderController({}, null, arrival('intent-2', 'conn-unknown'));
    await waitFor(() => expect(crew.connectionsState).toBe('loaded'));
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20));
    });
    expect(connects()).toBe(0);
    other.unmount();
  });

  it('selects the named connection first, then connects it', async () => {
    saved = [
      { ...connection, id: 'conn-0', name: 'Other' },
      { ...connection, status: 'disconnected' },
    ];
    renderController({}, null, arrival());
    await waitFor(() => expect(connects()).toBe(1));
    expect(crew.connectionId).toBe('conn-1');
    expect(
      mocks.crewHttp.mock.calls.some(
        ([path, method]) => path === '/connections/conn-0/connect' && method === 'POST'
      )
    ).toBe(false);
  });

  it('opens Sign in when the server asks for a password, and connects nothing more', async () => {
    mocks.crewHttp.mockImplementation(async (path: string, method = 'GET') => {
      if (path === '/connections') return { connections: saved };
      if (path === CONNECT && method === 'POST')
        throw new CrewHttpError('Crew SSH failure [ssh_eof]', 400, 'crew_ssh_auth_required');
      return {};
    });
    renderController({ autoOpenSignIn: true }, null, arrival());
    await waitFor(() => expect(crew.signIn).toEqual({ open: true, reason: 'auto' }));
    act(() => crew.closeSignIn());
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20));
    });
    expect(connects()).toBe(1);
  });

  it('never connects a connection whose membership the workspace ended', async () => {
    saved = [{ ...connection, status: 'disconnected', last_error_code: MEMBERSHIP_ENDED_CODE }];
    renderController({ autoOpenSignIn: true }, null, arrival());
    await waitFor(() => expect(crew.screen).toBe('offline'));
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20));
    });
    expect(connects()).toBe(0);
    // A person's own Connect still may.
    await act(async () => {
      await crew.connect({ userInitiated: true });
    });
    expect(connects()).toBe(1);
  });

  it('never connects a connection that waits for Sign in', async () => {
    saved = [{ ...connection, status: 'authentication_required' }];
    renderController({ autoOpenSignIn: true }, null, arrival());
    await waitFor(() => expect(crew.connectionsState).toBe('loaded'));
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20));
    });
    expect(connects()).toBe(0);
  });
});

describe('a membership the workspace ended (Q3-12, Q3-50)', () => {
  const ended = { ...connection, last_error_code: MEMBERSHIP_ENDED_CODE };

  it('offers no Retry for an observation end on such a connection', async () => {
    const sessions = controllableObserver();
    mocks.crewHttp.mockImplementation(async (path: string) => {
      if (path === '/connections') return { connections: [ended] };
      return {};
    });
    renderController();
    await waitFor(() => expect(sessions.length).toBeGreaterThan(0));
    act(() => sessions[sessions.length - 1]!.receive({ ...stateFrame }));
    await waitFor(() => expect(crew.snapshot).not.toBeNull());
    act(() =>
      sessions[sessions.length - 1]!.receive({
        type: 'error',
        code: 'access_denied',
        clear: true,
        error: 'Room observation ended.',
      })
    );
    await waitFor(() => expect(crew.refreshError).not.toBeNull());
    expect(crew.refreshErrorRetryable).toBe(false);
  });

  it('never observes it again quietly, nor follows it, after a loss', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const sessions = controllableObserver();
      let reads = 0;
      let list: Record<string, unknown>[] = [connection];
      mocks.crewHttp.mockImplementation(async (path: string) => {
        if (path === '/connections') {
          reads += 1;
          return { connections: list };
        }
        return {};
      });
      renderController();
      await waitFor(() => expect(sessions.length).toBeGreaterThan(0));
      act(() => sessions[sessions.length - 1]!.receive({ ...stateFrame }));
      await waitFor(() => expect(crew.snapshot).not.toBeNull());

      // The daemon found the membership ended and stopped the bridge.
      list = [{ ...ended, status: 'disconnected' }];
      const observed = sessions.length;
      act(() =>
        sessions[sessions.length - 1]!.receive({
          type: 'error',
          code: 'observation_refused',
          clear: true,
          error: 'Room observation ended.',
        })
      );
      await waitFor(() => expect(crew.refreshError).not.toBeNull());
      const after = reads;
      await act(async () => {
        await vi.advanceTimersByTimeAsync(OFFLINE_FOLLOW_WINDOW_MS * 2);
      });
      expect(reads).toBe(after);
      expect(sessions.length).toBe(observed);
      expect(mocks.crewHttp.mock.calls.some(([path]) => String(path).endsWith('/connect'))).toBe(
        false
      );
    } finally {
      vi.useRealTimers();
    }
  });
});

describe('a post still on its way when the person moves to another channel (RENDERER-4)', () => {
  const methods = { ...channel, id: 'channel-3', name: 'methods' };
  const analysis = { ...channel, id: 'channel-4', name: 'analysis' };
  const workspace = { ...snapshot, channels: [channel, methods, analysis] };

  beforeEach(() => {
    forgetConnectionMemory(connection.id);
    mocks.observeCrew.mockImplementation(
      async (
        _connectionId: string,
        channelId: string | undefined,
        _after: string | null,
        signal: AbortSignal,
        receive: (frame: unknown) => void
      ) => {
        if (signal.aborted) return 'terminal';
        receive({ ...stateFrame, snapshot: workspace });
        if (channelId)
          receive({
            type: 'messages',
            channel_id: channelId,
            messages: [],
            cursor: null,
            reset: true,
          });
        return 'terminal';
      }
    );
  });

  async function opened(channelId: string) {
    await waitFor(() => expect(crew.channelId).toBe(channelId));
    await waitFor(() => expect(crew.snapshot).not.toBeNull());
    await waitFor(() => expect(crew.messagesLoaded).toBe(true));
  }

  /** Sends "for #methods" from #methods and moves to #analysis while the broker answers. */
  async function sendThenMove() {
    const post = deferred<unknown>();
    let posts = 0;
    mocks.crewRequest.mockImplementation(async (_connection: string, method: string) => {
      if (method !== 'message.post') return {};
      posts += 1;
      // Only the first post, #methods', waits for the test to answer it.
      return posts === 1 ? post.promise : { sequence: `m-${posts}` };
    });
    renderController();
    await opened(channel.id);
    act(() => crew.selectChannel(methods.id));
    await opened(methods.id);
    act(() => crew.setBody('for #methods'));
    let sent!: Promise<void>;
    act(() => {
      sent = crew.send();
    });
    await waitFor(() => expect(crew.isPending('send')).toBe(true));
    act(() => crew.selectChannel(analysis.id));
    await opened(analysis.id);
    return { post, sent };
  }

  it('leaves the new channel’s composer free while the old post is on its way', async () => {
    const { post, sent } = await sendThenMove();
    // The post is #methods', not #analysis': nothing here is sending.
    expect(crew.isPending('send')).toBe(false);
    act(() => crew.setBody('for #analysis'));
    expect(crew.draft.body).toBe('for #analysis');
    await act(async () => {
      post.resolve({ sequence: 'm-1' });
      await sent;
    });
    // The #methods post changed nothing in #analysis' composer.
    expect(crew.draft.body).toBe('for #analysis');
    expect(crew.error).toBeNull();
  });

  it('sends in the new channel without waiting for the old post', async () => {
    const { post, sent } = await sendThenMove();
    act(() => crew.setBody('for #analysis'));
    await act(async () => {
      await crew.send();
    });
    const posts = mocks.crewRequest.mock.calls.filter(([, method]) => method === 'message.post');
    expect(posts.map(([, , params]) => (params as { channel_id: string }).channel_id)).toEqual([
      methods.id,
      analysis.id,
    ]);
    await act(async () => {
      post.resolve({ sequence: 'm-1' });
      await sent;
    });
  });

  it('reports a refusal in the connection bar, naming its channel, not in the new composer', async () => {
    const { post, sent } = await sendThenMove();
    act(() => crew.setBody('for #analysis'));
    await act(async () => {
      post.reject(new CrewHttpError('Slow down', 429, 'crew_request_refused'));
      await sent;
    });
    expect(crew.error?.source).toBe('global');
    expect(crew.error?.message).toBe(crewActionCopy.sendFailedIn('#methods', 'Slow down'));
    expect(crew.draft.body).toBe('for #analysis');
    // The unsent text waits in #methods, where the person left it.
    act(() => crew.selectChannel(methods.id));
    await opened(methods.id);
    await waitFor(() => expect(crew.draft.body).toBe('for #methods'));
  });

  /** Every `message.post` so far: where it went and the idempotency key it carried. */
  function messagePosts() {
    return mocks.crewRequest.mock.calls
      .filter(([, method]) => method === 'message.post')
      .map(
        ([, , params]) => params as { channel_id: string; body: string; idempotency_key: string }
      );
  }

  /** From `sendThenMove`: send in #analysis, then come back to #methods, whose post is still out. */
  async function postHereThenReturn() {
    const moved = await sendThenMove();
    act(() => crew.setBody('for #analysis'));
    await act(async () => {
      await crew.send();
    });
    act(() => crew.selectChannel(methods.id));
    await opened(methods.id);
    // The kept draft is back, and #methods' own post still holds its Send.
    await waitFor(() => expect(crew.draft.body).toBe('for #methods'));
    expect(crew.isPending('send')).toBe(true);
    return moved;
  }

  it('retries #methods’ post under its own key after a post in #analysis', async () => {
    const { post, sent } = await postHereThenReturn();
    // The outcome is unknown (the broker may have committed it): the retry must reuse its key.
    await act(async () => {
      post.reject(new CrewHttpError('The computer did not answer in time', 504));
      await sent;
    });
    expect(crew.error?.source).toBe('composer');
    expect(crew.draft.body).toBe('for #methods');
    await act(async () => {
      await crew.send();
    });
    const posts = messagePosts();
    expect(posts.map((item) => item.channel_id)).toEqual([methods.id, analysis.id, methods.id]);
    expect(posts[2].body).toBe('for #methods');
    // The same message to the same channel: the broker's deduplication covers the retry.
    expect(posts[2].idempotency_key).toBe(posts[0].idempotency_key);
    expect(posts[1].idempotency_key).not.toBe(posts[0].idempotency_key);
  });

  it('keeps #methods’ key while #analysis’ post is still on its way', async () => {
    const methodsPost = deferred<unknown>();
    const analysisPost = deferred<unknown>();
    let posts = 0;
    mocks.crewRequest.mockImplementation(async (_connection: string, method: string) => {
      if (method !== 'message.post') return {};
      posts += 1;
      if (posts === 1) return methodsPost.promise;
      if (posts === 2) return analysisPost.promise;
      return { sequence: `m-${posts}` };
    });
    renderController();
    await opened(channel.id);
    act(() => crew.selectChannel(methods.id));
    await opened(methods.id);
    act(() => crew.setBody('for #methods'));
    let methodsSent!: Promise<void>;
    act(() => {
      methodsSent = crew.send();
    });
    await waitFor(() => expect(crew.isPending('send')).toBe(true));
    act(() => crew.selectChannel(analysis.id));
    await opened(analysis.id);
    act(() => crew.setBody('for #analysis'));
    let analysisSent!: Promise<void>;
    act(() => {
      analysisSent = crew.send();
    });
    await waitFor(() => expect(crew.isPending('send')).toBe(true));
    act(() => crew.selectChannel(methods.id));
    await opened(methods.id);
    await waitFor(() => expect(crew.draft.body).toBe('for #methods'));
    await act(async () => {
      methodsPost.reject(new CrewHttpError('The computer did not answer in time', 504));
      await methodsSent;
    });
    await act(async () => {
      await crew.send();
    });
    const sentPosts = messagePosts();
    expect(sentPosts.map((item) => item.channel_id)).toEqual([methods.id, analysis.id, methods.id]);
    expect(sentPosts[2].idempotency_key).toBe(sentPosts[0].idempotency_key);
    await act(async () => {
      analysisPost.resolve({ sequence: 'm-2' });
      await analysisSent;
    });
  });

  it('keeps #methods’ text and key when the answer comes after the person came back', async () => {
    const { post, sent } = await postHereThenReturn();
    await act(async () => {
      post.resolve({ sequence: 'm-1' });
      await sent;
    });
    // The answer came to a view that has moved on: the composer keeps the text it put back, so
    // sending it again must be the same message to the broker, not a second one.
    expect(crew.draft.body).toBe('for #methods');
    await act(async () => {
      await crew.send();
    });
    const posts = messagePosts();
    expect(posts.map((item) => item.channel_id)).toEqual([methods.id, analysis.id, methods.id]);
    expect(posts[2].idempotency_key).toBe(posts[0].idempotency_key);
  });

  it('forgets #methods’ attempt when its answer comes while the person is elsewhere', async () => {
    const { post, sent } = await sendThenMove();
    await act(async () => {
      post.resolve({ sequence: 'm-1' });
      await sent;
    });
    act(() => crew.selectChannel(methods.id));
    await opened(methods.id);
    // Sent while away: its kept draft is gone, and so is its attempt.
    expect(crew.draft.body).toBe('');
    // The same words written again are a new message, under a key the broker will not take for
    // the first one.
    act(() => crew.setBody('for #methods'));
    await act(async () => {
      await crew.send();
    });
    const posts = messagePosts();
    expect(posts.map((item) => item.channel_id)).toEqual([methods.id, methods.id]);
    expect(posts[1].idempotency_key).not.toBe(posts[0].idempotency_key);
  });

  it('still reports a refusal in the composer when the person stayed', async () => {
    mocks.crewRequest.mockImplementation(async (_connection: string, method: string) => {
      if (method === 'message.post') throw new CrewHttpError('Slow down', 429);
      return {};
    });
    renderController();
    await opened(channel.id);
    act(() => crew.setBody('stay'));
    await act(async () => {
      await crew.send();
    });
    // It belongs to this channel's draft (QA M5): no other channel's composer shows it.
    expect(crew.error).toEqual({
      message: 'Slow down',
      source: 'composer',
      destination: `${connection.id}\n${channel.id}`,
    });
    expect(crew.draft.body).toBe('stay');
  });

  it('keeps a failed post’s key through an edit that is undone', async () => {
    let posts = 0;
    mocks.crewRequest.mockImplementation(async (_connection: string, method: string) => {
      if (method !== 'message.post') return {};
      posts += 1;
      if (posts === 1) throw new CrewHttpError('The computer did not answer in time', 504);
      return { sequence: `m-${posts}` };
    });
    renderController();
    await opened(channel.id);
    act(() => crew.setBody('for #general'));
    await act(async () => {
      await crew.send();
    });
    act(() => crew.setBody('for #general, and more'));
    act(() => crew.setBody('for #general'));
    await act(async () => {
      await crew.send();
    });
    const sent = messagePosts();
    expect(sent).toHaveLength(2);
    // The same payload to the same channel: the first may have been committed.
    expect(sent[1].idempotency_key).toBe(sent[0].idempotency_key);
  });

  it('lets a taken post’s key go at the first edit, so the same words written anew are new', async () => {
    const { post, sent } = await postHereThenReturn();
    await act(async () => {
      post.resolve({ sequence: 'm-1' });
      await sent;
    });
    expect(crew.draft.body).toBe('for #methods');
    act(() => crew.setBody('for #methods!'));
    act(() => crew.setBody('for #methods'));
    await act(async () => {
      await crew.send();
    });
    const posts = messagePosts();
    expect(posts.map((item) => item.channel_id)).toEqual([methods.id, analysis.id, methods.id]);
    // Reused, the broker would answer with the first message and post nothing.
    expect(posts[2].idempotency_key).not.toBe(posts[0].idempotency_key);
  });

  describe('when Crew is left and opened again (U1, U2)', () => {
    /** In #methods, the post of "for #methods" is sent; `answer` decides each post's answer. */
    async function sendInMethods(answer: (posts: number) => Promise<unknown>) {
      let posts = 0;
      mocks.crewRequest.mockImplementation(async (_connection: string, method: string) => {
        if (method !== 'message.post') return {};
        posts += 1;
        return answer(posts);
      });
      const first = renderController();
      await opened(channel.id);
      act(() => crew.selectChannel(methods.id));
      await opened(methods.id);
      act(() => crew.setBody('for #methods'));
      let sent!: Promise<void>;
      act(() => {
        sent = crew.send();
      });
      await waitFor(() => expect(crew.isPending('send')).toBe(true));
      return { first, sent };
    }

    /** Crew opened again, on #methods (the channel it remembers). */
    async function openedAgain() {
      renderController();
      await opened(methods.id);
    }

    it('sends a refused post again under its key (U2)', async () => {
      const { first, sent } = await sendInMethods(async (posts) => {
        if (posts === 1) throw new CrewHttpError('The computer did not answer in time', 504);
        return { sequence: `m-${posts}` };
      });
      await act(async () => {
        await sent;
      });
      expect(crew.error?.source).toBe('composer');
      first.unmount();

      await openedAgain();
      await waitFor(() => expect(crew.draft.body).toBe('for #methods'));
      await act(async () => {
        await crew.send();
      });
      const posts = messagePosts();
      expect(posts.map((item) => item.channel_id)).toEqual([methods.id, methods.id]);
      // The 504 may have come after the broker committed it: the same message, not a second.
      expect(posts[1].idempotency_key).toBe(posts[0].idempotency_key);
    });

    it('holds Send while the post is still on its way, then sends under its key (U1)', async () => {
      const post = deferred<unknown>();
      const { first, sent } = await sendInMethods((posts) =>
        posts === 1 ? post.promise : Promise.resolve({ sequence: `m-${posts}` })
      );
      first.unmount();

      await openedAgain();
      await waitFor(() => expect(crew.draft.body).toBe('for #methods'));
      // The first post is on its way still: this screen holds Send, and a press posts nothing.
      expect(crew.isPending('send')).toBe(true);
      await act(async () => {
        await crew.send();
      });
      expect(messagePosts()).toHaveLength(1);

      await act(async () => {
        post.reject(new CrewHttpError('The computer did not answer in time', 504));
        await sent;
      });
      // Told to the screen open now, in its composer, since the words are its. A gateway timeout
      // is the link's failure: stale once the connection verifies again (QA R-4).
      expect(crew.error).toEqual({
        message: 'The computer did not answer in time',
        source: 'composer',
        destination: `${connection.id}\n${methods.id}`,
        transport: true,
      });
      expect(crew.isPending('send')).toBe(false);
      await act(async () => {
        await crew.send();
      });
      const posts = messagePosts();
      expect(posts).toHaveLength(2);
      expect(posts[1].idempotency_key).toBe(posts[0].idempotency_key);
    });

    it('keeps the words and their key when the post is taken after Crew was opened again', async () => {
      const post = deferred<unknown>();
      const { first, sent } = await sendInMethods((posts) =>
        posts === 1 ? post.promise : Promise.resolve({ sequence: `m-${posts}` })
      );
      first.unmount();

      await openedAgain();
      await waitFor(() => expect(crew.draft.body).toBe('for #methods'));
      await act(async () => {
        post.resolve({ sequence: 'm-1' });
        await sent;
      });
      expect(crew.error).toBeNull();
      expect(crew.draft.body).toBe('for #methods');
      await act(async () => {
        await crew.send();
      });
      const posts = messagePosts();
      expect(posts).toHaveLength(2);
      // Sending the kept words again is the message already taken: the broker answers with it.
      expect(posts[1].idempotency_key).toBe(posts[0].idempotency_key);
      expect(crew.draft.body).toBe('');
    });

    it('keeps the words and their key when the post is refused while Crew is closed', async () => {
      const post = deferred<unknown>();
      const { first, sent } = await sendInMethods((posts) =>
        posts === 1 ? post.promise : Promise.resolve({ sequence: `m-${posts}` })
      );
      first.unmount();
      await act(async () => {
        post.reject(new CrewHttpError('The computer did not answer in time', 504));
        await sent;
      });

      await openedAgain();
      await waitFor(() => expect(crew.draft.body).toBe('for #methods'));
      expect(crew.isPending('send')).toBe(false);
      // What the refusal said comes back with the words: it was told nowhere while Crew was shut.
      expect(crew.error).toMatchObject({
        message: 'The computer did not answer in time',
        source: 'composer',
        destination: `${connection.id}\n${methods.id}`,
      });
      await act(async () => {
        await crew.send();
      });
      const posts = messagePosts();
      expect(posts).toHaveLength(2);
      expect(posts[1].idempotency_key).toBe(posts[0].idempotency_key);
    });

    it('forgets the words and their key when the post is taken while Crew is closed', async () => {
      const post = deferred<unknown>();
      const { first, sent } = await sendInMethods((posts) =>
        posts === 1 ? post.promise : Promise.resolve({ sequence: `m-${posts}` })
      );
      first.unmount();
      await act(async () => {
        post.resolve({ sequence: 'm-1' });
        await sent;
      });
      expect(stashedDraft(connection.id, methods.id)).toBeUndefined();

      await openedAgain();
      expect(crew.draft.body).toBe('');
      act(() => crew.setBody('for #methods'));
      await act(async () => {
        await crew.send();
      });
      const posts = messagePosts();
      expect(posts).toHaveLength(2);
      // The same words written again are a new message, under a key the broker has not seen.
      expect(posts[1].idempotency_key).not.toBe(posts[0].idempotency_key);
    });

    it('keeps with the words a digest of the post and its key, never its files or references', async () => {
      // Every post goes unanswered in time, so the words and their attempt stay.
      const { first, sent } = await sendInMethods(async () => {
        throw new CrewHttpError('The computer did not answer in time', 504);
      });
      await act(async () => {
        await sent;
      });
      first.unmount();
      mocks.crewRequest.mockClear();
      await openedAgain();
      await waitFor(() => expect(crew.draft.body).toBe('for #methods'));
      act(() => {
        crew.addAttachment({ id: 'blob-1', name: 'counts.csv' });
        crew.addReference({ id: 'ref-1', label: '/data/run-1' });
      });
      await act(async () => {
        await crew.send();
      });
      // Another payload (files and a reference now): another key.
      const withFiles = messagePosts()[0];
      act(() => crew.selectChannel(analysis.id));
      await opened(analysis.id);

      const kept = stashedDraft(connection.id, methods.id);
      // The composer's note about the words goes aside with them (QA M5): words, never an ID.
      expect(Object.keys(kept ?? {}).sort()).toEqual(['attempt', 'body', 'note', 'scope']);
      expect(JSON.stringify(kept?.note)).not.toMatch(/blob-1|ref-1/);
      const attempt = kept?.attempt?.current;
      expect(Object.keys(attempt ?? {}).sort()).toEqual(['digest', 'key']);
      expect(attempt?.digest).toMatch(/^[0-9a-f]{64}$/);
      expect(attempt?.key).toBe(withFiles.idempotency_key);
      expect(JSON.stringify(kept?.attempt)).not.toMatch(/blob-1|ref-1|for #methods/);
    });
  });
});
