import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError } from '../crewApi';
import { CREW_INVITATION_INVALID } from '../api/errors';
import type { CrewController } from '../state/types';
import { hostCopy, joinCopy } from './copy';
import { HostDialog } from './HostDialog';
import { WORKSPACE_KEY_PATTERN } from './JoinDialog';
import { readJoinContext, resetJoinContextForTests, updateJoinContext } from './joinContext';
import {
  DEVICE_KEY,
  fakeConnection,
  fakeSnapshot,
  makeCrew,
  renderWithCrew,
  WORKSPACE_KEY,
} from './testCrew';

const mocks = vi.hoisted(() => ({
  getProviders: vi.fn(),
  previewInvitation: vi.fn(),
  dockProps: [] as Record<string, unknown>[],
}));

vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({ getProviders: mocks.getProviders }),
  usePrivacyTiersEnabled: () => true,
}));
vi.mock('../api/join', async () => {
  const actual = await vi.importActual<typeof import('../api/join')>('../api/join');
  return { ...actual, previewInvitation: mocks.previewInvitation };
});
vi.mock('../../InAppTerminalDock', () => ({
  default: (props: Record<string, unknown>) => {
    mocks.dockProps.push(props);
    return <div data-testid="in-app-terminal-dock" />;
  },
}));

class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}

const HOSTING_KEY = 'c'.repeat(64);
const PASTE = 'started pid 4242\n{"workspace_id":"w-1","socket":"/tmp/crew-1000-abc/broker.sock"}';
const PREVIEW = {
  workspace_id: '11111111-2222-3333-4444-555555555555',
  workspace_name: 'lab-data',
  workspace_public_key: WORKSPACE_KEY,
  workspace_key_fingerprint: '3f2a9c1e77b0d4e1' + '0'.repeat(48),
  host_username: 'alice',
  host_display_name: null,
  mode: 'private' as const,
  institution_id: null,
  ssh_host: null,
  ssh_port: null,
  proxy_jump: null,
  invitee_username: null,
  socket_path: '/tmp/crew-1000-abc/broker.sock',
  owner_uid: 1000,
};

function deferred<T = void>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((settle) => {
    resolve = settle;
  });
  return { promise, resolve };
}

function renderHost(overrides: Partial<CrewController> = {}) {
  const crew = makeCrew({
    ui: { dialog: { kind: 'host' }, pane: null },
    prepareHostingDevice: vi.fn().mockResolvedValue({
      preparation_id: 'prep-1',
      public_key: HOSTING_KEY,
      device_id: 'device-1',
    }),
    ...overrides,
  });
  return renderWithCrew(<HostDialog />, crew);
}

async function fillName() {
  fireEvent.change(screen.getByLabelText(hostCopy.workspaceName), {
    target: { value: 'Lab Data' },
  });
  fireEvent.change(screen.getByLabelText(hostCopy.serverLogin), {
    target: { value: 'alice@hpc.ucsf.edu' },
  });
  await waitFor(() => expect(screen.getByLabelText(joinCopy.institution)).toHaveValue('ucsf'));
}

async function throughStart() {
  await fillName();
  fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));
  await screen.findByText(hostCopy.startHeading('hpc.ucsf.edu'));
  fireEvent.change(screen.getByLabelText(hostCopy.pasted), { target: { value: PASTE } });
  fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));
  await screen.findByText(hostCopy.createHeading('lab-data', 'hpc.ucsf.edu'));
}

beforeEach(() => {
  vi.stubGlobal('ResizeObserver', ResizeObserverStub);
  resetJoinContextForTests();
  mocks.dockProps.length = 0;
  mocks.previewInvitation.mockReset().mockResolvedValue(PREVIEW);
  mocks.getProviders.mockReset().mockResolvedValue([
    {
      name: 'versa',
      is_configured: true,
      affiliation: { kind: 'institutions', institutions: [{ id: 'ucsf' }] },
    },
    { name: 'openai', is_configured: false },
  ]);
});
afterEach(() => {
  vi.unstubAllGlobals();
});

describe('HostDialog', () => {
  it('names the workspace with a live preview and the one configured institution', async () => {
    renderHost();
    expect(screen.getByText(hostCopy.stepOf(1, 3, 'Name'))).toBeInTheDocument();
    const name = screen.getByLabelText(hostCopy.workspaceName);
    expect(name).toHaveFocus();
    fireEvent.change(name, { target: { value: 'Lab Data' } });
    expect(screen.getByText(/Your workspace: lab-data/)).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: /^Private/ })).toBeChecked();
    await waitFor(() => expect(screen.getByLabelText(joinCopy.institution)).toHaveValue('ucsf'));
    expect(screen.getByLabelText(joinCopy.institution)).toBeRequired();
    expect(screen.getByText(hostCopy.advancedSummary)).toBeInTheDocument();
  });

  it('prepares the hosting identity on Continue and shows the start command with its key', async () => {
    const view = renderHost();
    await fillName();
    fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));

    expect(await screen.findByText(hostCopy.startHeading('hpc.ucsf.edu'))).toBeInTheDocument();
    expect(view.crew().prepareHostingDevice).toHaveBeenCalledOnce();
    expect(screen.getByText(hostCopy.stepOf(2, 3, 'Start'))).toBeInTheDocument();
    expect(screen.getByText(hostCopy.runThis('hpc.ucsf.edu', 'alice'))).toBeInTheDocument();
    expect(screen.getByText(/--name lab-data --bootstrap-key c{64}/)).toBeInTheDocument();
    expect(screen.getByText(hostCopy.consequence)).toBeInTheDocument();

    // The embedded terminal is a plain shell: nothing is typed or run for the person.
    fireEvent.click(screen.getByRole('button', { name: hostCopy.openTerminal }));
    expect(screen.getByTestId('in-app-terminal-dock')).toBeInTheDocument();
    expect(Object.keys(mocks.dockProps[0]).sort()).toEqual(['onClose', 'onEmptied', 'open']);

    fireEvent.click(screen.getByRole('button', { name: hostCopy.notSignedIn }));
    expect(screen.getByText('ssh alice@hpc.ucsf.edu')).toBeInTheDocument();
    expect(screen.getByText(hostCopy.confirmServer)).toBeInTheDocument();
  });

  it('says what went wrong when the paste is not what Crew prints', async () => {
    mocks.previewInvitation.mockRejectedValue(
      new CrewHttpError('not an invitation', 400, CREW_INVITATION_INVALID)
    );
    renderHost();
    await fillName();
    fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));
    await screen.findByText(hostCopy.startHeading('hpc.ucsf.edu'));
    fireEvent.change(screen.getByLabelText(hostCopy.pasted), { target: { value: 'oops' } });
    fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));
    expect(await screen.findByText(hostCopy.bad)).toBeInTheDocument();
    expect(mocks.previewInvitation).toHaveBeenCalledWith('oops');
  });

  it('asks for the details a preview lacks, prefilled with the ones it had, instead of refusing the paste', async () => {
    // A daemon whose preview carries only the display fields: no socket path, no host user ID.
    mocks.previewInvitation.mockResolvedValue({ ...PREVIEW, socket_path: null, owner_uid: null });
    const saved = fakeConnection({ id: 'conn-host', status: 'disconnected' });
    const view = renderHost({ saveConnection: vi.fn().mockResolvedValue(saved) });
    await fillName();
    fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));
    await screen.findByText(hostCopy.startHeading('hpc.ucsf.edu'));
    fireEvent.change(screen.getByLabelText(hostCopy.pasted), { target: { value: PASTE } });
    fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));

    expect(await screen.findByText(hostCopy.detailsMissing)).toBeInTheDocument();
    expect(screen.queryByText(hostCopy.bad)).toBeNull();
    expect(screen.getByLabelText(hostCopy.pasted)).not.toHaveAttribute('aria-invalid', 'true');
    expect(screen.getByLabelText(joinCopy.workspaceId)).toHaveValue(PREVIEW.workspace_id);
    expect(screen.getByLabelText(joinCopy.workspaceKey)).toHaveValue(WORKSPACE_KEY);
    const socket = screen.getByLabelText(joinCopy.socketPath);
    const uid = screen.getByLabelText(joinCopy.hostUserId);
    expect(socket).toHaveValue('');
    expect(socket).toBeRequired();
    expect(uid).toBeRequired();

    // Still empty: the form refuses Continue and stays on Start.
    fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));
    expect(screen.getByText(hostCopy.stepOf(2, 3, 'Start'))).toBeInTheDocument();

    fireEvent.change(socket, { target: { value: '/tmp/crew-1000-abc/broker.sock' } });
    fireEvent.change(uid, { target: { value: '1000' } });
    fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));
    await screen.findByText(hostCopy.createHeading('lab-data', 'hpc.ucsf.edu'));
    // The key is the one the daemon read, so its fingerprint still describes it.
    expect(screen.getByText('3F2A 9C1E 77B0 D4E1')).toBeInTheDocument();
    expect(mocks.previewInvitation).toHaveBeenCalledOnce();

    fireEvent.click(screen.getByRole('button', { name: hostCopy.create }));
    await waitFor(() =>
      expect(view.crew().saveConnection).toHaveBeenCalledWith(
        expect.objectContaining({
          socket_path: '/tmp/crew-1000-abc/broker.sock',
          owner_uid: 1000,
          workspace_id: PREVIEW.workspace_id,
          workspace_public_key: WORKSPACE_KEY,
          preparation_id: 'prep-1',
        })
      )
    );
  });

  it('reads a new paste again after asking for missing details', async () => {
    mocks.previewInvitation
      .mockResolvedValueOnce({ ...PREVIEW, socket_path: null, owner_uid: null })
      .mockResolvedValueOnce(PREVIEW);
    renderHost();
    await fillName();
    fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));
    await screen.findByText(hostCopy.startHeading('hpc.ucsf.edu'));
    fireEvent.change(screen.getByLabelText(hostCopy.pasted), { target: { value: PASTE } });
    fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));
    await screen.findByText(hostCopy.detailsMissing);

    fireEvent.change(screen.getByLabelText(hostCopy.pasted), {
      target: {
        value: `${PASTE}
`,
      },
    });
    expect(screen.queryByText(hostCopy.detailsMissing)).toBeNull();
    expect(screen.queryByLabelText(joinCopy.socketPath)).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));
    await screen.findByText(hostCopy.createHeading('lab-data', 'hpc.ucsf.edu'));
    expect(mocks.previewInvitation).toHaveBeenCalledTimes(2);
  });

  it('takes the details by hand, with the restart hint, when the background service is older', async () => {
    mocks.previewInvitation.mockRejectedValue(new CrewHttpError('Crew request failed (404)', 404));
    const saved = fakeConnection({ id: 'conn-host', status: 'disconnected' });
    const view = renderHost({ saveConnection: vi.fn().mockResolvedValue(saved) });
    await fillName();
    fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));
    await screen.findByText(hostCopy.startHeading('hpc.ucsf.edu'));
    fireEvent.change(screen.getByLabelText(hostCopy.pasted), { target: { value: PASTE } });
    fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));

    expect(await screen.findByText(hostCopy.staleDaemon)).toBeInTheDocument();
    expect(screen.getByLabelText(joinCopy.workspaceKey)).toHaveAttribute(
      'pattern',
      WORKSPACE_KEY_PATTERN
    );
    fireEvent.change(screen.getByLabelText(joinCopy.socketPath), {
      target: { value: '/tmp/crew-1000-abc/broker.sock' },
    });
    fireEvent.change(screen.getByLabelText(joinCopy.workspaceId), {
      target: { value: PREVIEW.workspace_id },
    });
    fireEvent.change(screen.getByLabelText(joinCopy.hostUserId), { target: { value: '1000' } });
    fireEvent.change(screen.getByLabelText(joinCopy.workspaceKey), {
      target: { value: WORKSPACE_KEY.toUpperCase() },
    });
    fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));
    await screen.findByText(hostCopy.createHeading('lab-data', 'hpc.ucsf.edu'));
    // Typed by hand: no daemon read the key, so no fingerprint is claimed for it.
    expect(screen.queryByText('3F2A 9C1E 77B0 D4E1')).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: hostCopy.create }));
    await waitFor(() =>
      expect(view.crew().saveConnection).toHaveBeenCalledWith(
        expect.objectContaining({
          socket_path: '/tmp/crew-1000-abc/broker.sock',
          owner_uid: 1000,
          workspace_id: PREVIEW.workspace_id,
          workspace_public_key: WORKSPACE_KEY,
        })
      )
    );
  });

  it('creates the workspace: save with the prepared identity, connect, bootstrap, then label', async () => {
    const connectDone = deferred();
    const connect = vi.fn(() => connectDone.promise);
    const saved = fakeConnection({
      id: 'conn-host',
      ssh_target: 'alice@hpc.ucsf.edu',
      status: 'disconnected',
    });
    const view = renderHost({ connect, saveConnection: vi.fn().mockResolvedValue(saved) });
    await throughStart();

    expect(screen.getByText('3F2A 9C1E 77B0 D4E1')).toBeInTheDocument();
    expect(screen.getByText(hostCopy.createBody('lab-data'))).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: hostCopy.create }));

    const crew = view.crew();
    await waitFor(() =>
      expect(crew.saveConnection).toHaveBeenCalledWith({
        name: 'lab-data',
        ssh_target: 'alice@hpc.ucsf.edu',
        port: undefined,
        identity_file: undefined,
        proxy_jump: undefined,
        socket_path: PREVIEW.socket_path,
        owner_uid: 1000,
        workspace_id: PREVIEW.workspace_id,
        workspace_public_key: WORKSPACE_KEY,
        remote_root: undefined,
        remote_execution: false,
        mode: 'private',
        institution_id: 'ucsf',
        preparation_id: 'prep-1',
      })
    );
    expect(readJoinContext('conn-host')).toMatchObject({ hostSetup: true });

    // The controller selects and lists the saved connection; the dialog then connects it.
    view.update({ connectionId: 'conn-host', connections: [saved], connection: saved });
    await waitFor(() => expect(connect).toHaveBeenCalledWith({ userInitiated: true }));
    view.update({ connection: { ...saved, status: 'connected' } });
    await act(async () => connectDone.resolve());

    await waitFor(() =>
      expect(crew.request).toHaveBeenCalledWith(
        'auth.bootstrap',
        { public_key: DEVICE_KEY },
        { mutation: true }
      )
    );
    await waitFor(() => expect(crew.setJoinStatus).toHaveBeenCalledWith('joined'));
    expect(crew.refresh).toHaveBeenCalled();
    expect(readJoinContext('conn-host')).toMatchObject({ hostSetup: false });

    // Verified, Private and unlabelled: ask once for the host's institution.
    const snapshot = fakeSnapshot({
      workspace: {
        id: 'workspace-1',
        host_uid: 1000,
        mode: 'private',
        institution_id: null,
        policy_epoch: 1,
        host_principal_id: 'p-alice',
        name: 'lab-data',
      },
    });
    view.update({
      snapshot,
      observedPrivacy: {
        connectionId: 'conn-host',
        mode: 'private',
        institutionId: 'ucsf',
        policyEpoch: 1,
      },
    });
    expect(await screen.findByText(hostCopy.labelTitle('lab-data', 'ucsf'))).toBeInTheDocument();
    expect(screen.getByText(hostCopy.labelBody)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: hostCopy.labelSet('ucsf') }));
    await waitFor(() =>
      expect(crew.mutate).toHaveBeenCalledWith('policy.set', {
        mode: 'private',
        institution_id: 'ucsf',
      })
    );
    await waitFor(() => expect(crew.closeDialog).toHaveBeenCalled());
  });

  it('waits for Sign in, and returns to Create when it ends without signing in', async () => {
    const connectDone = deferred();
    const connect = vi.fn(() => connectDone.promise);
    const saved = fakeConnection({ id: 'conn-host', status: 'disconnected' });
    const view = renderHost({ connect, saveConnection: vi.fn().mockResolvedValue(saved) });
    await throughStart();
    fireEvent.click(screen.getByRole('button', { name: hostCopy.create }));
    await waitFor(() => expect(view.crew().saveConnection).toHaveBeenCalled());
    view.update({ connectionId: 'conn-host', connections: [saved], connection: saved });
    await waitFor(() => expect(connect).toHaveBeenCalled());

    // The server wanted a password: the controller opened Sign in by itself.
    view.update({
      signIn: { open: true, reason: 'auto' },
      lastConnectFailure: { kind: 'auth_required', message: 'Permission denied' },
    });
    await act(async () => connectDone.resolve());
    expect(await screen.findByRole('button', { name: hostCopy.signingIn })).toBeDisabled();

    view.update({ signIn: { open: false, reason: null } });
    expect(await screen.findByText(hostCopy.signInEnded)).toBeInTheDocument();
    expect(view.crew().request).not.toHaveBeenCalledWith(
      'auth.bootstrap',
      expect.anything(),
      expect.anything()
    );
    expect(screen.getByRole('button', { name: hostCopy.create })).toBeEnabled();
  });

  it('resumes at Create for a workspace this computer saved but never created', async () => {
    updateJoinContext('conn-1', { hostSetup: true, workspaceName: 'lab' });
    const connection = fakeConnection({ ssh_target: 'alice@hpc.ucsf.edu', status: 'connected' });
    const view = renderHost({ connectionId: 'conn-1', connection, connections: [connection] });

    expect(await screen.findByText(hostCopy.stepOf(3, 3, 'Create'))).toBeInTheDocument();
    expect(screen.getByText(hostCopy.createHeading('lab', 'hpc.ucsf.edu'))).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: hostCopy.create }));
    await waitFor(() =>
      expect(view.crew().request).toHaveBeenCalledWith(
        'auth.bootstrap',
        { public_key: DEVICE_KEY },
        { mutation: true }
      )
    );
    expect(view.crew().saveConnection).not.toHaveBeenCalled();
  });
});
