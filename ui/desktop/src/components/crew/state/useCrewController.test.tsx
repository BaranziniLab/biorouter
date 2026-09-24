import { act, render, screen, waitFor } from '@testing-library/react';
import { useState } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError } from '../crewApi';
import {
  CrewControllerProvider,
  useCrewErrorSlot,
  useCrewSurfaceReset,
} from './CrewControllerContext';
import { useCrewController } from './useCrewController';
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

function renderController(options?: CrewControllerOptions, children?: React.ReactNode) {
  return render(
    <MemoryRouter initialEntries={['/crew']}>
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
    act(() =>
      sessions[sessions.length - 1]!.receive({
        type: 'error',
        clear: true,
        code: 'observation_refused',
        error: 'Room observation ended.',
      })
    );
    await waitFor(() => expect(crew.lastVerified).toBeNull());
    expect(crew.refreshError).toMatch(/^Room observation ended\. .*unsent draft is retained/);
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

    // A dropped bridge: the observer fails, then Retry re-verifies.
    act(() =>
      sessions[sessions.length - 1]!.receive({
        type: 'error',
        clear: true,
        code: 'observation_refused',
        error: 'Crew SSH failure [ssh_eof; child_before_cleanup=exit_255]',
      })
    );
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

  it('does not open Sign in without the option (the legacy layout)', async () => {
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
