import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { MemoryRouter } from 'react-router-dom';
import CrewView from './CrewView';
import { CrewHttpError } from './crewApi';

const mocks = vi.hoisted(() => ({
  crewHttp: vi.fn(),
  crewRequest: vi.fn(),
  observeCrew: vi.fn(),
  navigate: vi.fn(),
  getProviders: vi.fn(),
  read: vi.fn(),
  getProviderModels: vi.fn(),
}));

vi.mock('./crewApi', async () => {
  const actual = await vi.importActual<typeof import('./crewApi')>('./crewApi');
  return {
    ...actual,
    crewHttp: mocks.crewHttp,
    crewRequest: mocks.crewRequest,
    observeCrew: mocks.observeCrew,
  };
});
vi.mock('../ConfigContext', () => ({
  useConfig: () => ({
    getProviders: mocks.getProviders,
    read: mocks.read,
    getProviderModels: mocks.getProviderModels,
  }),
}));
vi.mock('react-router-dom', async () => {
  const actual = await vi.importActual<typeof import('react-router-dom')>('react-router-dom');
  return { ...actual, useNavigate: () => mocks.navigate };
});
vi.mock('./CrewAuthentication', () => ({
  default: ({ onConnected, onClose }: { onConnected: () => void; onClose: () => void }) => (
    <div data-testid="crew-authentication-fixture">
      <button onClick={onConnected}>Simulate authenticated completion</button>
      <button onClick={onClose}>Simulate authentication close</button>
    </div>
  ),
}));
vi.mock('./CrewHostTrust', () => ({ default: () => <div /> }));
vi.mock('./CrewFiles', () => ({
  CrewUpload: () => <div />,
  CrewAttachment: () => <div />,
  CrewRemoteReference: () => <div />,
}));

const connection = {
  id: 'conn-1',
  name: 'Fixture',
  ssh_target: 'fixture',
  port: 22,
  identity_file: '/tmp/key',
  proxy_jump: '',
  socket_path: '/tmp/socket',
  owner_uid: 1000,
  workspace_id: 'workspace-1',
  workspace_public_key: 'workspace-key',
  public_key: 'device-key',
  device_id: 'device-1',
  cluster_connection_id: 'cluster-1',
  mode: 'private' as const,
  policy_epoch: 1,
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
  workspace: { id: 'workspace-1', host_uid: 1000, mode: 'private' as const, policy_epoch: 1 },
  actor,
  principals: [actor],
  teams: [
    {
      id: 'team-1',
      name: 'Lab',
      created_by: actor.id,
      members: [actor.id],
      general_channel_id: channel.id,
    },
  ],
  channels: [channel],
  invitations: [],
  runs: [],
};

function renderCrew(entry = '/crew') {
  return render(
    <MemoryRouter initialEntries={[entry]}>
      <CrewView />
    </MemoryRouter>
  );
}

function observerState(
  nextSnapshot: { workspace: { mode: 'private' | 'public' } } = snapshot,
  connectionMode: 'private' | 'public' = nextSnapshot.workspace.mode
) {
  return {
    type: 'state' as const,
    connection_id: connection.id,
    connection_mode: connectionMode,
    snapshot: nextSnapshot,
    runs: [],
    cursor: null,
  };
}

function installObservation(
  nextSnapshot = snapshot,
  messages: Record<string, unknown>[] = []
): void {
  mocks.observeCrew.mockImplementation(
    async (
      _connectionId: string,
      _channelId: string | undefined,
      _after: string | null,
      signal: AbortSignal,
      receive: (frame: unknown) => void
    ) => {
      if (signal.aborted) return 'terminal';
      receive(observerState(nextSnapshot));
      receive({
        type: 'messages',
        channel_id: channel.id,
        messages,
        cursor: messages.length ? (messages[messages.length - 1]?.sequence ?? null) : null,
        reset: true,
      });
      return 'terminal';
    }
  );
}

function defaultHttp() {
  mocks.crewHttp.mockImplementation(async (path: string, method = 'GET') => {
    if (path === '/connections') return { connections: [connection] };
    if (path.startsWith('/transfers?')) return { transfers: [] };
    if (path === '/connections/conn-1/runs' && method === 'GET') return { runs: [] };
    if (path === '/connections/conn-1/runs' && method === 'POST')
      return { run_id: 'run-1', session_id: 'session-1' };
    return {};
  });
  mocks.crewRequest.mockImplementation(async (_id: string, method: string) => {
    if (method === 'workspace.snapshot') return snapshot;
    if (method === 'messages.history') return { messages: [], cursor: null };
    return {};
  });
  mocks.getProviders.mockResolvedValue([{ name: 'fixture-provider', is_configured: true }]);
  mocks.getProviderModels.mockResolvedValue(['fixture-model']);
  mocks.read.mockResolvedValue('');
  installObservation();
}

describe('CrewView action and uncertain-start regressions', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    defaultHttp();
    let next = 1;
    vi.spyOn(globalThis.crypto, 'randomUUID').mockImplementation(
      () => `00000000-0000-4000-8000-${String(next++).padStart(12, '0')}`
    );
  });

  it('retains a start action error after a successful manual refresh', async () => {
    renderCrew();
    await screen.findByText('Welcome to #general');
    mocks.crewHttp.mockImplementation(async (path: string, method = 'GET') => {
      if (path === '/connections/conn-1/runs' && method === 'POST') throw new Error('start failed');
      if (path === '/connections') return { connections: [connection] };
      if (path === '/connections/conn-1/runs' && method === 'GET') return { runs: [] };
      return {};
    });
    fireEvent.click(screen.getByRole('button', { name: 'Ask my agent' }));
    fireEvent.change(await screen.findByLabelText('Task'), { target: { value: 'run it' } });
    fireEvent.change(screen.getByLabelText('Configured provider'), {
      target: { value: 'fixture-provider' },
    });
    fireEvent.change(screen.getByLabelText('Model'), { target: { value: 'fixture-model' } });
    fireEvent.click(screen.getByRole('button', { name: 'Start my agent and allow posting here' }));
    expect(await screen.findAllByText('start failed')).not.toHaveLength(0);
    // A successful manual refresh must not erase the action error that still needs attention.
    const beforeRefresh = mocks.observeCrew.mock.calls.length;
    fireEvent.click(screen.getByRole('button', { name: 'Refresh channel' }));
    await waitFor(() => expect(mocks.observeCrew.mock.calls.length).toBeGreaterThan(beforeRefresh));
    expect(screen.getAllByText('start failed').length).toBeGreaterThan(0);
  });

  it('refreshes after authenticated completion without issuing a second manual connect request', async () => {
    renderCrew();
    await screen.findByText('Welcome to #general');
    mocks.crewHttp.mockClear();
    mocks.crewRequest.mockClear();

    fireEvent.click(screen.getByRole('button', { name: 'Authenticate' }));
    fireEvent.click(
      await screen.findByRole('button', { name: 'Simulate authenticated completion' })
    );

    await waitFor(() => expect(mocks.observeCrew.mock.calls.length).toBeGreaterThan(1));
    expect(
      mocks.crewHttp.mock.calls.some(
        ([path, method]) => path === '/connections/conn-1/connect' && method === 'POST'
      )
    ).toBe(false);
    expect(mocks.crewHttp.mock.calls.some(([path]) => path === '/connections')).toBe(true);
  });

  it('preserves a draft when an older history cursor becomes stale', async () => {
    const historyRequests: Record<string, unknown>[] = [];
    const olderMessages = Array.from({ length: 200 }, (_, index) => ({
      id: `message-${index}`,
      sequence: `00000000-0000-4000-8000-${String(index + 1).padStart(12, '0')}`,
      channel_id: channel.id,
      actor_id: actor.id,
      body: `message ${index}`,
      created_at: 1_700_000_000 + index,
      restricted: false,
      source_channels: [channel.id],
      attachments: [],
    }));
    mocks.crewRequest.mockImplementation(async (_id: string, method: string, params = {}) => {
      if (method === 'messages.history') {
        historyRequests.push(params);
        if ('before' in params) throw new Error('stale_cursor');
        return {
          messages: olderMessages,
          cursor: olderMessages[olderMessages.length - 1]?.sequence ?? null,
        };
      }
      return {};
    });
    installObservation(snapshot, olderMessages);
    renderCrew();
    const composer = await screen.findByLabelText('Message #general');
    fireEvent.change(composer, { target: { value: 'keep this unsent draft' } });
    fireEvent.click(await screen.findByRole('button', { name: 'Older messages' }));
    await waitFor(() => expect(historyRequests.some((params) => 'before' in params)).toBe(true));
    await waitFor(() => expect(screen.queryByText('Viewing earlier messages')).toBeNull());
    expect(screen.getByText(/unsent draft is retained/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Retry Crew updates' }));
    await waitFor(() =>
      expect(screen.getByLabelText('Message #general')).toHaveValue('keep this unsent draft')
    );
  });

  it('clears the composer after the selected channel is revoked', async () => {
    let activeSnapshot = snapshot;
    mocks.observeCrew.mockImplementation(
      async (
        _connectionId: string,
        _channelId: string | undefined,
        _after: string | null,
        signal: AbortSignal,
        receive: (frame: unknown) => void
      ) => {
        if (signal.aborted) return 'terminal';
        receive(observerState(activeSnapshot));
        return 'terminal';
      }
    );
    renderCrew();
    const composer = await screen.findByLabelText('Message #general');
    fireEvent.change(composer, { target: { value: 'discard after revocation' } });
    activeSnapshot = { ...snapshot, channels: [], teams: [] };
    const observationsBeforeRefresh = mocks.observeCrew.mock.calls.length;
    fireEvent.click(screen.getByRole('button', { name: 'Refresh channel' }));
    await waitFor(() =>
      expect(mocks.observeCrew.mock.calls.length).toBeGreaterThan(observationsBeforeRefresh)
    );
    await waitFor(() => expect(screen.queryByLabelText('Message #general')).toBeNull());
    activeSnapshot = snapshot;
    await waitFor(() => expect(screen.getByRole('button', { name: 'Reconnect' })).toBeEnabled());
    fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }));
    await waitFor(() => expect(screen.getByLabelText('Message #general')).toHaveValue(''));
  });

  it('retains an unsent draft across a transient observer failure and manual recovery', async () => {
    let observerMode: 'success' | 'failure' = 'success';
    renderCrew();
    const composer = await screen.findByLabelText('Message #general');
    fireEvent.change(composer, { target: { value: 'retain while reconnecting' } });
    mocks.observeCrew.mockImplementation(
      async (
        _connectionId: string,
        _channelId: string | undefined,
        _after: string | null,
        signal: AbortSignal,
        receive: (frame: unknown) => void
      ) => {
        if (signal.aborted) return 'terminal';
        if (observerMode === 'failure') {
          receive({
            type: 'error',
            clear: true,
            code: 'temporary_observer_error',
            error: 'observer temporarily unavailable',
          });
        } else {
          receive(observerState());
        }
        return 'terminal';
      }
    );
    observerMode = 'failure';
    const beforeFailure = mocks.observeCrew.mock.calls.length;
    fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }));
    await waitFor(() =>
      expect(screen.getByText(/observer temporarily unavailable/)).toBeInTheDocument()
    );
    expect(mocks.observeCrew.mock.calls.length).toBe(beforeFailure + 1);

    observerMode = 'success';
    fireEvent.click(screen.getByRole('button', { name: 'Retry Crew updates' }));
    await waitFor(() => expect(mocks.observeCrew.mock.calls.length).toBeGreaterThan(2));
    await waitFor(() =>
      expect(screen.getByLabelText('Message #general')).toHaveValue('retain while reconnecting')
    );
  });

  it('reloads changed connection metadata before retrying observation after a policy terminal', async () => {
    const refreshedConnection = {
      ...connection,
      name: 'Renamed workspace',
      ssh_target: 'alice@new-host',
      remote_root: '/srv/new-workspace',
    };
    let connectionReads = 0;
    let observationCalls = 0;
    const events: string[] = [];
    mocks.crewHttp.mockImplementation(async (path: string, method = 'GET') => {
      if (path === '/connections') {
        connectionReads += 1;
        events.push(`connections:${connectionReads}`);
        return { connections: [connectionReads > 1 ? refreshedConnection : connection] };
      }
      if (path === '/connections/conn-1/runs' && method === 'GET') return { runs: [] };
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
        observationCalls += 1;
        events.push(`observe:${observationCalls}`);
        if (signal.aborted) return 'terminal';
        receive(observerState());
        if (observationCalls === 1) {
          receive({
            type: 'error',
            clear: true,
            code: 'policy_changed',
            error: 'Workspace policy changed while observing.',
          });
          // A late frame from the retired observer must not restore the old metadata.
          receive(observerState());
        } else {
          receive(observerState());
        }
        return 'terminal';
      }
    );
    renderCrew();
    await screen.findByText(/Workspace policy changed while observing/);
    fireEvent.click(screen.getByRole('button', { name: 'Retry Crew updates' }));

    await waitFor(() => expect(screen.getByText('alice@new-host')).toBeInTheDocument());
    expect(screen.getByRole('option', { name: 'Renamed workspace' })).toBeInTheDocument();
    expect(events).toEqual(['connections:1', 'observe:1', 'connections:2', 'observe:2']);
  });

  it('uses observer privacy across stale connection refreshes and clears drafts on mode changes', async () => {
    const publicSnapshot = {
      ...snapshot,
      workspace: { ...snapshot.workspace, mode: 'public' as const },
    };
    let observedMode: 'private' | 'public' = 'private';
    mocks.observeCrew.mockImplementation(
      async (
        _connectionId: string,
        _channelId: string | undefined,
        _after: string | null,
        signal: AbortSignal,
        receive: (frame: unknown) => void
      ) => {
        if (signal.aborted) return 'terminal';
        receive(observerState(observedMode === 'public' ? publicSnapshot : snapshot, observedMode));
        return 'terminal';
      }
    );
    renderCrew('/crew?sessionId=agent-1');
    const privateComposer = await screen.findByLabelText('Message #general');
    fireEvent.change(privateComposer, { target: { value: 'private draft to clear' } });

    observedMode = 'public';
    fireEvent.click(screen.getByRole('button', { name: 'Authenticate' }));
    fireEvent.click(
      await screen.findByRole('button', { name: 'Simulate authenticated completion' })
    );
    await waitFor(() =>
      expect(screen.getByRole('combobox', { name: 'Connection privacy' })).toHaveValue('public')
    );
    expect(screen.getByText(/Effective: public/)).toBeInTheDocument();
    expect(screen.getByLabelText('Message #general')).toHaveValue('');

    fireEvent.click(screen.getByRole('button', { name: 'Review access and posting permission' }));
    fireEvent.click(
      screen.getByRole('button', { name: 'Allow this conversation to read and post here' })
    );
    await waitFor(() =>
      expect(
        mocks.crewHttp.mock.calls.some(
          ([path, method, params]) =>
            path === '/connections/conn-1/sessions/agent-1/grant' &&
            method === 'POST' &&
            params.expected_mode === 'public'
        )
      ).toBe(true)
    );

    fireEvent.change(screen.getByLabelText('Message #general'), {
      target: { value: 'public message' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Send message' }));
    await waitFor(() =>
      expect(
        mocks.crewRequest.mock.calls.some(
          ([, method, params]) => method === 'message.post' && params.personal_mode === 'public'
        )
      ).toBe(true)
    );
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Ask my agent' })).toBeInTheDocument()
    );

    fireEvent.click(screen.getByRole('button', { name: 'Ask my agent' }));
    fireEvent.change(await screen.findByLabelText('Task'), {
      target: { value: 'public task' },
    });
    fireEvent.change(screen.getByLabelText('Configured provider'), {
      target: { value: 'fixture-provider' },
    });
    fireEvent.change(screen.getByLabelText('Model'), {
      target: { value: 'fixture-model' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Start my agent and allow posting here' }));
    await waitFor(() =>
      expect(
        mocks.crewHttp.mock.calls.some(
          ([path, method, params]) =>
            path === '/connections/conn-1/runs' &&
            method === 'POST' &&
            params.expected_mode === 'public'
        )
      ).toBe(true)
    );

    observedMode = 'private';
    fireEvent.click(screen.getByRole('button', { name: 'Authenticate' }));
    fireEvent.click(
      await screen.findByRole('button', { name: 'Simulate authenticated completion' })
    );
    await waitFor(() =>
      expect(screen.getByRole('combobox', { name: 'Connection privacy' })).toHaveValue('private')
    );
    expect(screen.getByText(/Effective: private/)).toBeInTheDocument();
    await waitFor(() => expect(screen.getByLabelText('Message #general')).toHaveValue(''));
  });

  it('keeps one request id across a retry with the same payload', async () => {
    let starts = 0;
    const requestIds: string[] = [];
    mocks.crewHttp.mockImplementation(
      async (path: string, method = 'GET', body?: { request_id?: string }) => {
        if (path === '/connections') return { connections: [connection] };
        if (path === '/connections/conn-1/runs' && method === 'GET') return { runs: [] };
        if (path === '/connections/conn-1/runs' && method === 'POST') {
          starts += 1;
          requestIds.push(body?.request_id ?? '');
          if (starts === 1) throw new Error('temporary start failure');
          return {};
        }
        return {};
      }
    );
    renderCrew();
    await screen.findByText('Welcome to #general');
    fireEvent.click(screen.getByRole('button', { name: 'Ask my agent' }));
    fireEvent.change(await screen.findByLabelText('Task'), { target: { value: 'retry me' } });
    fireEvent.change(screen.getByLabelText('Configured provider'), {
      target: { value: 'fixture-provider' },
    });
    fireEvent.change(screen.getByLabelText('Model'), { target: { value: 'fixture-model' } });
    const submit = screen.getByRole('button', { name: 'Start my agent and allow posting here' });
    fireEvent.click(submit);
    await screen.findAllByText('temporary start failure');
    fireEvent.click(submit);
    await waitFor(() => expect(starts).toBe(2));
    expect(requestIds[1]).toBe(requestIds[0]);
  });

  it('requires inspection before a typed unknown outcome can be restarted and then rotates the request id', async () => {
    let starts = 0;
    const requestIds: string[] = [];
    mocks.crewHttp.mockImplementation(
      async (path: string, method = 'GET', body?: { request_id?: string }) => {
        if (path === '/connections') return { connections: [connection] };
        if (path === '/connections/conn-1/runs' && method === 'GET') return { runs: [] };
        if (path === '/connections/conn-1/runs' && method === 'POST') {
          starts += 1;
          requestIds.push(body?.request_id ?? '');
          if (starts === 1) {
            throw new CrewHttpError('outcome unknown', 502, 'crew_start_outcome_unknown');
          }
          return {};
        }
        return {};
      }
    );
    const view = renderCrew();
    await screen.findByText('Welcome to #general');
    fireEvent.click(screen.getByRole('button', { name: 'Ask my agent' }));
    fireEvent.change(await screen.findByLabelText('Task'), { target: { value: 'uncertain' } });
    fireEvent.change(screen.getByLabelText('Configured provider'), {
      target: { value: 'fixture-provider' },
    });
    fireEvent.change(screen.getByLabelText('Model'), { target: { value: 'fixture-model' } });
    fireEvent.click(screen.getByRole('button', { name: 'Start my agent and allow posting here' }));
    expect(
      await screen.findByText('Inspect the previous task before starting again')
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Start my agent and allow posting here' })
    ).toBeDisabled();
    fireEvent.click(screen.getByRole('button', { name: 'Start my agent and allow posting here' }));
    expect(starts).toBe(1);
    fireEvent.change(screen.getByLabelText('Task'), {
      target: { value: 'edited after uncertainty' },
    });
    expect(starts).toBe(1);
    view.unmount();
    renderCrew();
    await screen.findByText('Welcome to #general');
    fireEvent.click(screen.getByRole('button', { name: 'Ask my agent' }));
    expect(
      await screen.findByText('Inspect the previous task before starting again')
    ).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText('Task'), { target: { value: 'edited after remount' } });
    fireEvent.change(screen.getByLabelText('Configured provider'), {
      target: { value: 'fixture-provider' },
    });
    fireEvent.change(screen.getByLabelText('Model'), { target: { value: 'fixture-model' } });
    fireEvent.click(screen.getByRole('checkbox'));
    const restart = screen.getByRole('button', { name: /Start a new task/ });
    expect(restart).toBeEnabled();
    fireEvent.click(restart);
    await waitFor(() => expect(starts).toBe(2));
    expect(requestIds[1]).not.toBe(requestIds[0]);
    expect(screen.queryByText('Inspect the previous task before starting again')).toBeNull();
  });
});
