import { act, createEvent, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { MemoryRouter } from 'react-router-dom';
import CrewApp from './CrewApp';
import { CrewHttpError } from './crewApi';
import { layoutCopy } from './layout/copy';
import { emptyCopy } from './onboarding/copy';
import { crewObservationCopy } from './state/copy';
import {
  chooseModel,
  channelAction,
  installResizeObserverStub,
  workspaceAction,
} from './test/crewTestUtils';

/**
 * The Crew route's behavioral regressions, driven through the redesigned layout (`CrewApp`).
 *
 * Migrated row by row from the legacy layout's suite as ui-redesign-spec's "CVT, query by query"
 * lists: every behavioral assertion is kept, and a query moved only where its control deliberately
 * left the resting screen for a menu (C5: Reconnect, Sign in…, Connection settings…, Refresh
 * channel) or changed kind (C3: privacy radios and the status-row chip; C4: the workspace
 * switcher; one model picker for the provider and model selects).
 */

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
vi.mock('../ConfigContext', async () => {
  // The actual module keeps `usePrivacyTiersEnabled`, which `PrivacyBadge` reads (enforcing,
  // outside a provider).
  const actual = await vi.importActual<typeof import('../ConfigContext')>('../ConfigContext');
  return {
    ...actual,
    useConfig: () => ({
      getProviders: mocks.getProviders,
      read: mocks.read,
      getProviderModels: mocks.getProviderModels,
    }),
  };
});
vi.mock('react-router-dom', async () => {
  const actual = await vi.importActual<typeof import('react-router-dom')>('react-router-dom');
  return { ...actual, useNavigate: () => mocks.navigate };
});
// The Sign in dialog's SSH terminal. `CrewHostTrust` is reached only through it, so it needs no
// mock of its own.
vi.mock('./CrewAuthentication', () => ({
  default: ({ onConnected, onClose }: { onConnected: () => void; onClose: () => void }) => (
    <div data-testid="crew-authentication-fixture">
      <button onClick={onConnected}>Simulate authenticated completion</button>
      <button onClick={onClose}>Simulate authentication close</button>
    </div>
  ),
}));
// Nothing here uploads; a picker must never open from a regression test.
vi.mock('./files/useCrewUpload', async () => {
  const actual =
    await vi.importActual<typeof import('./files/useCrewUpload')>('./files/useCrewUpload');
  return {
    ...actual,
    useCrewUpload: () => ({
      upload: async () => undefined,
      choosing: false,
      error: '',
      reportError: () => undefined,
      dismissError: () => undefined,
      chips: [],
      pause: async () => undefined,
      resume: async () => undefined,
      forget: () => undefined,
    }),
  };
});

installResizeObserverStub();

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
  workspace: {
    id: 'workspace-1',
    host_uid: 1000,
    mode: 'private' as const,
    policy_epoch: 1,
    institution_id: 'ucsf',
  },
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
      <CrewApp />
    </MemoryRouter>
  );
}

function observerState(
  nextSnapshot: { workspace: { mode: 'private' | 'public' } } = snapshot,
  connectionMode: 'private' | 'public' = nextSnapshot.workspace.mode,
  connectionPolicyEpoch = 1
) {
  return {
    type: 'state' as const,
    connection_id: connection.id,
    connection_mode: connectionMode,
    connection_policy_epoch: connectionPolicyEpoch,
    connection_institution_id: connection.institution_id,
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
    if (path === '/connections/conn-1/grants' && method === 'GET') return { grants: [] };
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

/**
 * The daemon calls the connection disconnected, refuses to observe it, and asks for credentials
 * on connect, until the returned `signedIn()`: then it reads connected and observes as the
 * test's own observer does. Install before rendering.
 */
function offlineUntilSignedIn(): () => void {
  let status: 'connected' | 'disconnected' = 'disconnected';
  const normalHttp = mocks.crewHttp.getMockImplementation();
  mocks.crewHttp.mockImplementation(async (path: string, method = 'GET', body?: unknown) => {
    if (path === '/connections') return { connections: [{ ...connection, status }] };
    if (path === '/connections/conn-1/connect' && method === 'POST')
      throw new CrewHttpError('Crew SSH failure [ssh_eof]', 400, 'crew_ssh_auth_required');
    return normalHttp?.(path, method, body);
  });
  const normalObserve = mocks.observeCrew.getMockImplementation();
  mocks.observeCrew.mockImplementation(
    async (
      connectionId: string,
      channelId: string | undefined,
      after: string | null,
      signal: AbortSignal,
      receive: (frame: unknown) => void
    ) => {
      if (signal.aborted) return 'terminal';
      if (status === 'disconnected') {
        receive({
          type: 'error',
          clear: true,
          code: 'observation_refused',
          error: 'Crew connection is not connected',
        });
        return 'terminal';
      }
      return normalObserve?.(connectionId, channelId, after, signal, receive);
    }
  );
  return () => {
    status = 'connected';
  };
}

/**
 * Open Sign in the way it opens now (T-40, Q3-57): the offline screen's Connect meets a server
 * that asks for credentials, and the dialog opens by itself. A connected, verified workspace
 * offers no Reconnect to reach it from.
 */
async function openSignInFromOffline() {
  fireEvent.click(await screen.findByRole('button', { name: emptyCopy.offlineAction('Fixture') }));
  return screen.findByRole('button', { name: 'Simulate authenticated completion' });
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

  it('does not label a cached connected connection as verified without a current snapshot', async () => {
    mocks.observeCrew.mockImplementation(async () => 'terminal');
    renderCrew();
    await screen.findByText('fixture');
    expect(screen.getByText('Checking connection')).toBeInTheDocument();
    expect(screen.queryByText('Connected · identity verified')).toBeNull();
  });

  it('requires the snapshot connection identity to match before showing verified status', async () => {
    mocks.observeCrew.mockImplementation(
      async (
        _connectionId: string,
        _channelId: string | undefined,
        _after: string | null,
        _signal: AbortSignal,
        receive: (frame: unknown) => void
      ) => {
        receive({ ...observerState(), connection_id: 'different-connection' });
        return 'terminal';
      }
    );
    renderCrew();
    await screen.findByText('fixture');
    expect(screen.getByText(/^(Checking connection|Updates unavailable)$/)).toBeInTheDocument();
    expect(screen.queryByText('Connected · identity verified')).toBeNull();
  });

  it('requires an institution for private connection saves and preserves it across mode edits', async () => {
    renderCrew();
    await screen.findByText('Connected · identity verified');
    await waitFor(() =>
      expect(
        mocks.observeCrew.mock.calls.some(([, observedChannel]) => observedChannel === channel.id)
      ).toBe(true)
    );
    await workspaceAction('Connection settings…');
    const institution = await screen.findByPlaceholderText('For example, ucsf or sdsc');
    expect(institution).toBeRequired();
    fireEvent.change(institution, { target: { value: '' } });
    fireEvent.click(screen.getByRole('radio', { name: /^Public/ }));
    // Public needs no institution, so the field is not merely optional: it is gone.
    expect(screen.queryByPlaceholderText('For example, ucsf or sdsc')).toBeNull();
    fireEvent.click(screen.getByRole('radio', { name: /^Private/ }));
    const required = await screen.findByPlaceholderText('For example, ucsf or sdsc');
    expect(required).toBeRequired();
    expect(required).toHaveValue('');
    fireEvent.click(screen.getByRole('button', { name: 'Save connection' }));
    expect(required).toBeInvalid();
    expect(
      mocks.crewHttp.mock.calls.some(
        ([path, method]) => path === '/connections/conn-1' && method === 'PATCH'
      )
    ).toBe(false);
  });

  it('keeps a typed institution across switching the connection to Public and back', async () => {
    renderCrew();
    await screen.findByText('Connected · identity verified');
    await workspaceAction('Connection settings…');
    fireEvent.change(await screen.findByPlaceholderText('For example, ucsf or sdsc'), {
      target: { value: 'sdsc' },
    });
    fireEvent.click(screen.getByRole('radio', { name: /^Public/ }));
    expect(screen.queryByPlaceholderText('For example, ucsf or sdsc')).toBeNull();
    fireEvent.click(screen.getByRole('radio', { name: /^Private/ }));
    expect(await screen.findByPlaceholderText('For example, ucsf or sdsc')).toHaveValue('sdsc');
  });

  it('offers host institution confirmation before the workspace label is set', async () => {
    const workspace = snapshot.workspace as unknown as { institution_id: string | null };
    const originalInstitution = workspace.institution_id;
    workspace.institution_id = null;
    renderCrew();
    try {
      expect(
        await screen.findByText(layoutCopy.institution.title('Fixture', 'ucsf'))
      ).toBeInTheDocument();
      const offer = screen.getByRole('button', { name: 'Set institution to ucsf…' });
      expect(offer).toBeEnabled();
      fireEvent.click(offer);
      // The label is permanent, so it now asks first; nothing is set before the confirmation.
      const confirm = await screen.findByRole('button', { name: 'Set ucsf permanently' });
      expect(mocks.crewRequest.mock.calls.some(([, method]) => method === 'policy.set')).toBe(
        false
      );
      fireEvent.click(confirm);
      await waitFor(() =>
        expect(
          mocks.crewRequest.mock.calls.some(
            ([, method, params]) =>
              method === 'policy.set' &&
              params.institution_id === 'ucsf' &&
              params.mode === 'private'
          )
        ).toBe(true)
      );
    } finally {
      workspace.institution_id = originalInstitution;
    }
  });

  it('clears a draft when the observed connection policy epoch changes', async () => {
    let connectionPolicyEpoch = 1;
    mocks.observeCrew.mockImplementation(
      async (
        _connectionId: string,
        _channelId: string | undefined,
        _after: string | null,
        _signal: AbortSignal,
        receive: (frame: unknown) => void
      ) => {
        receive(observerState(snapshot, 'private', connectionPolicyEpoch));
        return 'terminal';
      }
    );
    renderCrew();
    const composer = await screen.findByLabelText('Message #general');
    fireEvent.change(composer, { target: { value: 'clear after policy change' } });
    connectionPolicyEpoch = 2;
    // Observed again (Refresh channel; Reconnect is offered only while not connected, Q3-57).
    await channelAction('Refresh channel');
    await waitFor(() => expect(screen.getByLabelText('Message #general')).toHaveValue(''));
    expect(screen.getByText(/privacy or selected channel access changed/)).toBeInTheDocument();
  });

  it('keeps a draft across leaving Crew, unless the connection policy epoch moved meanwhile (Q2-07)', async () => {
    let connectionPolicyEpoch = 1;
    mocks.observeCrew.mockImplementation(
      async (
        _connectionId: string,
        _channelId: string | undefined,
        _after: string | null,
        _signal: AbortSignal,
        receive: (frame: unknown) => void
      ) => {
        receive(observerState(snapshot, 'private', connectionPolicyEpoch));
        return 'terminal';
      }
    );
    const first = renderCrew();
    fireEvent.change(await screen.findByLabelText('Message #general'), {
      target: { value: 'kept while away' },
    });
    first.unmount();
    const second = renderCrew();
    await waitFor(() =>
      expect(screen.getByLabelText('Message #general')).toHaveValue('kept while away')
    );

    second.unmount();
    // Away again, and this computer's connection privacy binding moved meanwhile.
    connectionPolicyEpoch = 2;
    renderCrew();
    const composer = await screen.findByLabelText('Message #general');
    await waitFor(() => expect(mocks.observeCrew).toHaveBeenCalled());
    expect(composer).toHaveValue('');
    // Dropped silently: nothing was in the composer to clear.
    expect(screen.queryByText(/privacy or selected channel access changed/)).toBeNull();
  });

  it('sends on Enter, while preserving Shift+Enter and IME composition', async () => {
    renderCrew();
    await screen.findByText('Connected · identity verified');
    const composer = await screen.findByLabelText('Message #general');
    fireEvent.change(composer, { target: { value: 'line one' } });
    const shiftEnter = createEvent.keyDown(composer, {
      key: 'Enter',
      code: 'Enter',
      keyCode: 13,
      shiftKey: true,
    });
    fireEvent(composer, shiftEnter);
    expect(shiftEnter.defaultPrevented).toBe(false);
    expect(mocks.crewRequest).not.toHaveBeenCalledWith(
      expect.anything(),
      'message.post',
      expect.anything(),
      expect.anything()
    );
    expect(composer).toHaveValue('line one');

    fireEvent.keyDown(composer, { key: 'Enter', code: 'Enter', keyCode: 13, isComposing: true });
    fireEvent.keyDown(composer, { key: 'Enter', code: 'Enter', keyCode: 229 });
    expect(mocks.crewRequest).not.toHaveBeenCalledWith(
      expect.anything(),
      'message.post',
      expect.anything(),
      expect.anything()
    );

    fireEvent.keyDown(composer, { key: 'Enter', code: 'Enter', keyCode: 13 });
    await waitFor(() =>
      expect(mocks.crewRequest.mock.calls.some(([, method]) => method === 'message.post')).toBe(
        true
      )
    );
    expect(composer).toHaveValue('');
  });

  it('keeps sending single-flight and does not refresh the verified workspace', async () => {
    let resolvePost!: () => void;
    const post = new Promise<void>((resolve) => {
      resolvePost = resolve;
    });
    mocks.crewRequest.mockImplementation(async (_id: string, method: string) => {
      if (method === 'message.post') return post;
      return {};
    });
    renderCrew();
    await screen.findByText('Connected · identity verified');
    const composer = await screen.findByLabelText('Message #general');
    fireEvent.change(composer, { target: { value: 'send once' } });
    const observedCalls = mocks.observeCrew.mock.calls.length;
    fireEvent.keyDown(composer, { key: 'Enter', code: 'Enter', keyCode: 13 });
    fireEvent.keyDown(composer, { key: 'Enter', code: 'Enter', keyCode: 13 });
    await waitFor(() =>
      expect(
        mocks.crewRequest.mock.calls.filter(([, method]) => method === 'message.post')
      ).toHaveLength(1)
    );
    resolvePost();
    await waitFor(() => expect(composer).toHaveValue(''));
    expect(mocks.observeCrew.mock.calls.length).toBe(observedCalls);
    expect(screen.getByText('Connected · identity verified')).toBeInTheDocument();
  });

  it('retains the idempotency key when observation fails during an in-flight send', async () => {
    let releasePost!: () => void;
    let failObserver: (() => void) | undefined;
    let failing = false;
    let attempts = 0;
    const requestIds: string[] = [];
    const post = new Promise<void>((resolve) => {
      releasePost = resolve;
    });
    const failure = { type: 'error', error: 'temporary observation failure', code: 'temporary' };
    mocks.observeCrew.mockImplementation(
      async (
        _connectionId: string,
        _channelId: string | undefined,
        _after: string | null,
        _signal: AbortSignal,
        receive: (frame: unknown) => void
      ) => {
        // Once it fails it keeps failing, so the quiet re-observation (Q2-01) ends the same way.
        if (failing) {
          receive(failure);
          return 'terminal';
        }
        receive(observerState());
        failObserver = () => {
          failing = true;
          receive(failure);
        };
        return 'terminal';
      }
    );
    mocks.crewRequest.mockImplementation(
      async (_id: string, method: string, params?: { idempotency_key?: string }) => {
        if (method !== 'message.post') return {};
        attempts += 1;
        requestIds.push(params?.idempotency_key ?? '');
        if (attempts === 1) return post;
        return {};
      }
    );
    renderCrew();
    await screen.findByText('Connected · identity verified');
    const composer = await screen.findByLabelText('Message #general');
    fireEvent.change(composer, { target: { value: 'survive observation loss' } });
    fireEvent.keyDown(composer, { key: 'Enter', code: 'Enter', keyCode: 13 });
    await waitFor(() => expect(attempts).toBe(1));
    await act(async () => {
      failObserver?.();
    });
    // Plain words, and the draft (still in the composer while the post is in flight) is kept.
    expect(
      await screen.findByText(
        `${crewObservationCopy.updatesStopped('Fixture')} ${crewObservationCopy.draftRetained}`
      )
    ).toBeInTheDocument();
    expect(screen.queryByText(/temporary observation failure/)).toBeNull();
    releasePost();
    await waitFor(() => expect(screen.queryByLabelText('Message #general')).toBeNull());

    installObservation();
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Retry Crew updates' })).toBeEnabled()
    );
    fireEvent.click(screen.getByRole('button', { name: 'Retry Crew updates' }));
    const recoveredComposer = await screen.findByLabelText('Message #general');
    expect(recoveredComposer).toHaveValue('survive observation loss');
    fireEvent.click(screen.getByRole('button', { name: 'Send message' }));
    await waitFor(() => expect(attempts).toBe(2));
    expect(requestIds[0]).not.toBe('');
    expect(requestIds[1]).toBe(requestIds[0]);
  });

  it('retains the draft and idempotency key for a failed send retry', async () => {
    let attempts = 0;
    const requestIds: string[] = [];
    mocks.crewRequest.mockImplementation(
      async (_id: string, method: string, params?: { idempotency_key?: string }) => {
        if (method !== 'message.post') return {};
        attempts += 1;
        requestIds.push(params?.idempotency_key ?? '');
        if (attempts === 1) throw new Error('send failed');
        return {};
      }
    );
    renderCrew();
    await screen.findByText('Connected · identity verified');
    const composer = await screen.findByLabelText('Message #general');
    fireEvent.change(composer, { target: { value: 'retry this' } });
    fireEvent.keyDown(composer, { key: 'Enter', code: 'Enter', keyCode: 13 });
    expect(await screen.findAllByText('send failed')).not.toHaveLength(0);
    expect(composer).toHaveValue('retry this');
    fireEvent.click(screen.getByRole('button', { name: 'Send message' }));
    await waitFor(() => expect(attempts).toBe(2));
    expect(requestIds[1]).toBe(requestIds[0]);
    await waitFor(() => expect(composer).toHaveValue(''));
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
    await waitFor(() =>
      expect(
        mocks.observeCrew.mock.calls.some(([, observedChannel]) => observedChannel === channel.id)
      ).toBe(true)
    );
    const askButton = await screen.findByRole('button', { name: 'Ask my agent' });
    await waitFor(() => expect(askButton).toBeEnabled());
    fireEvent.click(askButton);
    fireEvent.change(await screen.findByLabelText('Task'), { target: { value: 'run it' } });
    await chooseModel('fixture-model');
    fireEvent.click(screen.getByRole('button', { name: 'Start my agent and allow posting here' }));
    expect(await screen.findAllByText('start failed')).toHaveLength(1);
    // A successful manual refresh must not erase the action error that still needs attention.
    // The pane stays open through it, and the channel menu stays reachable beside it (C1).
    const beforeRefresh = mocks.observeCrew.mock.calls.length;
    await channelAction('Refresh channel');
    await waitFor(() => expect(mocks.observeCrew.mock.calls.length).toBeGreaterThan(beforeRefresh));
    expect(screen.getAllByText('start failed')).toHaveLength(1);
  });

  it('refreshes after authenticated completion without issuing a second manual connect request', async () => {
    const signedIn = offlineUntilSignedIn();
    renderCrew();
    const complete = await openSignInFromOffline();
    // The sign-in terminal connected it.
    signedIn();
    mocks.crewHttp.mockClear();
    mocks.crewRequest.mockClear();
    const observed = mocks.observeCrew.mock.calls.length;

    fireEvent.click(complete);

    await waitFor(() => expect(mocks.observeCrew.mock.calls.length).toBeGreaterThan(observed));
    expect(await screen.findByText('Welcome to #general')).toBeInTheDocument();
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
    let latest: ((frame: unknown) => void) | null = null;
    mocks.observeCrew.mockImplementation(
      async (
        _connectionId: string,
        _channelId: string | undefined,
        _after: string | null,
        signal: AbortSignal,
        receive: (frame: unknown) => void
      ) => {
        if (signal.aborted) return 'terminal';
        latest = receive;
        receive(observerState(activeSnapshot));
        return 'terminal';
      }
    );
    renderCrew();
    const composer = await screen.findByLabelText('Message #general');
    fireEvent.change(composer, { target: { value: 'discard after revocation' } });
    activeSnapshot = { ...snapshot, channels: [], teams: [] };
    const observationsBeforeRefresh = mocks.observeCrew.mock.calls.length;
    await channelAction('Refresh channel');
    await waitFor(() =>
      expect(mocks.observeCrew.mock.calls.length).toBeGreaterThan(observationsBeforeRefresh)
    );
    await waitFor(() => expect(screen.queryByLabelText('Message #general')).toBeNull());
    // Access comes back with the daemon's next state frame: the draft does not.
    activeSnapshot = snapshot;
    act(() => latest?.(observerState(activeSnapshot)));
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
    // Observed again (Refresh channel; Reconnect is offered only while not connected, Q3-57).
    await channelAction('Refresh channel');
    await waitFor(() =>
      expect(
        screen.getByText(
          `${crewObservationCopy.updatesStopped('Fixture')} ${crewObservationCopy.draftRetained}`
        )
      ).toBeInTheDocument()
    );
    expect(screen.queryByText(/observer temporarily unavailable/)).toBeNull();
    // The failed observation, and the one quiet re-observation that ended the same way (Q2-01).
    expect(mocks.observeCrew.mock.calls.length).toBe(beforeFailure + 2);

    observerMode = 'success';
    fireEvent.click(screen.getByRole('button', { name: 'Retry Crew updates' }));
    await waitFor(() => expect(mocks.observeCrew.mock.calls.length).toBeGreaterThan(2));
    await waitFor(() =>
      expect(screen.getByLabelText('Message #general')).toHaveValue('retain while reconnecting')
    );
  });

  it('recovers from a policy terminal by itself, reloading changed connection metadata first', async () => {
    // Live QA round 1, P0-1: the broker moves the workspace policy epoch for every accepted
    // invitation, and the daemon ends every member's observation with `policy_changed`. That
    // used to wipe the view, clear the draft and wait for Retry; now it observes again by itself.
    const refreshedConnection = {
      ...connection,
      name: 'Renamed workspace',
      ssh_target: 'alice@new-host',
      remote_root: '/srv/new-workspace',
    };
    let connectionReads = 0;
    let observationCalls = 0;
    const events: string[] = [];
    const observedChannels: (string | undefined)[] = [];
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
        observedChannels.push(_channelId);
        if (signal.aborted) return 'terminal';
        receive(observerState());
        if (observationCalls === 1) {
          receive({
            type: 'error',
            clear: true,
            code: 'policy_changed',
            error:
              'Room observation ended. Clear cached room content and refresh authorized access; a stale cursor requires an explicit fresh history selection.',
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

    // The reloaded server shows: in the You row's login (`alice@new-host`), or as the place the row
    // says the person is ("on new-host", Q4-50).
    await waitFor(() => expect(screen.getAllByText(/new-host/).length).toBeGreaterThan(0));
    expect(screen.getByRole('button', { name: /^Renamed workspace/ })).toBeInTheDocument();
    // No banner, no Retry, and never the daemon's sentence.
    expect(screen.queryByRole('button', { name: 'Retry Crew updates' })).toBeNull();
    expect(screen.queryByText(/Room observation ended/)).toBeNull();
    expect(screen.queryByText(/stopped/)).toBeNull();
    // The reload lands before the observation that recovers, and nothing reloads again after it.
    // Once it verifies the workspace, the unchanged controller may restart it for the channel it
    // just selected; how soon that lands against this assertion is timing, not order, so it is
    // checked for what it is rather than counted.
    expect(events.slice(0, 4)).toEqual([
      'connections:1',
      'observe:1',
      'connections:2',
      'observe:2',
    ]);
    events.slice(4).forEach((event, index) => {
      expect(event).toMatch(/^observe:/);
      expect(observedChannels[index + 2]).toBe(channel.id);
    });
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
    // Observed again, with the saved record still reading private.
    await channelAction('Refresh channel');
    // The status row's chip is the effective mode (it replaced the privacy select and its
    // "Effective: …" line); the payload assertions below still prove the wire.
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /^Privacy: Public/ })).toBeInTheDocument()
    );
    expect(await screen.findByLabelText('Message #general')).toHaveValue('');

    // The mocked daemon holds no grant for this chat, so `/crew` opened its consent by itself
    // (Q3-28); while it is open the note's "Review access" is not drawn beside it (Q4-14).
    fireEvent.click(
      await screen.findByRole('button', { name: 'Allow this conversation to read and post here' })
    );
    await waitFor(() =>
      expect(
        mocks.crewHttp.mock.calls.some(
          ([path, method, params]) =>
            path === '/connections/conn-1/sessions/agent-1/grant' &&
            method === 'POST' &&
            params.expected_mode === 'public' &&
            params.expected_policy_epoch === 1 &&
            params.expected_workspace_policy_epoch === 1
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
    await chooseModel('fixture-model');
    fireEvent.click(screen.getByRole('button', { name: 'Start my agent and allow posting here' }));
    await waitFor(() =>
      expect(
        mocks.crewHttp.mock.calls.some(
          ([path, method, params]) =>
            path === '/connections/conn-1/runs' &&
            method === 'POST' &&
            params.expected_mode === 'public' &&
            params.expected_policy_epoch === 1 &&
            params.expected_workspace_policy_epoch === 1
        )
      ).toBe(true)
    );

    observedMode = 'private';
    await channelAction('Refresh channel');
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /^Privacy: Private/ })).toBeInTheDocument()
    );
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
    await chooseModel('fixture-model');
    const submit = screen.getByRole('button', { name: 'Start my agent and allow posting here' });
    fireEvent.click(submit);
    expect(await screen.findAllByText('temporary start failure')).toHaveLength(1);
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
    await chooseModel('fixture-model');
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
    await chooseModel('fixture-model');
    fireEvent.click(screen.getByRole('checkbox'));
    const restart = screen.getByRole('button', { name: /Start a new task/ });
    expect(restart).toBeEnabled();
    fireEvent.click(restart);
    await waitFor(() => expect(starts).toBe(2));
    expect(requestIds[1]).not.toBe(requestIds[0]);
    expect(screen.queryByText('Inspect the previous task before starting again')).toBeNull();
  });
});
