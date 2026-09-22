import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { MemoryRouter } from 'react-router-dom';
import CrewView from './CrewView';
import { CrewHttpError } from './crewApi';

const mocks = vi.hoisted(() => ({
  crewHttp: vi.fn(),
  crewRequest: vi.fn(),
  navigate: vi.fn(),
  getProviders: vi.fn(),
  read: vi.fn(),
  getProviderModels: vi.fn(),
}));

vi.mock('./crewApi', async () => {
  const actual = await vi.importActual<typeof import('./crewApi')>('./crewApi');
  return { ...actual, crewHttp: mocks.crewHttp, crewRequest: mocks.crewRequest };
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
vi.mock('./CrewAuthentication', () => ({ default: () => <div /> }));
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

function renderCrew() {
  return render(
    <MemoryRouter initialEntries={['/crew']}>
      <CrewView />
    </MemoryRouter>
  );
}

function defaultHttp() {
  mocks.crewHttp.mockImplementation(async (path: string, method = 'GET') => {
    if (path === '/connections') return { connections: [connection] };
    if (path === '/connections/conn-1/runs' && method === 'GET') return { runs: [] };
    if (path === '/connections/conn-1/runs' && method === 'POST')
      return { run_id: 'run-1', session_id: 'session-1' };
    return {};
  });
  mocks.crewRequest.mockImplementation(async (_id: string, method: string) => {
    if (method === 'workspace.snapshot') return snapshot;
    if (method === 'messages.history') return { messages: [], cursor: 0 };
    return {};
  });
  mocks.getProviders.mockResolvedValue([{ name: 'fixture-provider', is_configured: true }]);
  mocks.getProviderModels.mockResolvedValue(['fixture-model']);
  mocks.read.mockResolvedValue('');
}

describe('CrewView action and uncertain-start regressions', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    defaultHttp();
    let next = 1;
    vi.spyOn(globalThis.crypto, 'randomUUID').mockImplementation(() => `request-${next++}`);
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
    fireEvent.click(screen.getByRole('button', { name: 'Refresh channel' }));
    await waitFor(() =>
      expect(mocks.crewRequest).toHaveBeenCalledWith(
        'conn-1',
        'messages.history',
        expect.anything()
      )
    );
    expect(screen.getAllByText('start failed').length).toBeGreaterThan(0);
  });

  it('retains the action error across failed, recovered, and successful background polls', async () => {
    let pollMode: 'success' | 'failure' = 'success';
    renderCrew();
    await screen.findByText('Welcome to #general');
    mocks.crewHttp.mockImplementation(async (path: string, method = 'GET') => {
      if (path === '/connections/conn-1/runs' && method === 'POST') {
        throw new Error('start failed');
      }
      if (path === '/connections/conn-1/runs' && method === 'GET') return { runs: [] };
      if (path === '/connections') return { connections: [connection] };
      return {};
    });
    mocks.crewRequest.mockImplementation(async (_id: string, method: string) => {
      if (method === 'workspace.snapshot') {
        if (pollMode === 'failure') throw new Error('poll failed');
        return snapshot;
      }
      if (method === 'messages.history') return { messages: [], cursor: 0 };
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

    const beforeFailure = mocks.crewRequest.mock.calls.length;
    pollMode = 'failure';
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 4200));
    });
    expect(mocks.crewRequest.mock.calls.length).toBeGreaterThan(beforeFailure);
    expect(screen.getAllByText('start failed').length).toBeGreaterThan(0);

    const beforeRecovery = mocks.crewRequest.mock.calls.length;
    pollMode = 'success';
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 4200));
    });
    expect(mocks.crewRequest.mock.calls.length).toBeGreaterThan(beforeRecovery);
    expect(screen.getAllByText('start failed').length).toBeGreaterThan(0);
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
