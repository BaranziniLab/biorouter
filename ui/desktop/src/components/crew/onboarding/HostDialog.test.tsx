import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError } from '../crewApi';
import { CREW_INVITATION_INVALID } from '../api/errors';
import type { CrewController } from '../state/types';
import { hostCopy, joinCopy, setupCopy } from './copy';
import { HostDialog } from './HostDialog';
import type { SetupRun } from './setup';
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
  startHostRun: vi.fn(),
  readHostRun: vi.fn(),
  stopHostRun: vi.fn(),
  startSetup: vi.fn(),
  readSetup: vi.fn(),
  stopSetup: vi.fn(),
  signIn: [] as Record<string, unknown>[],
  /** What the daemon says of automatic setup; its own read has its suite (`setup.test.ts`). */
  setupSupport: { supported: true as boolean | null, artifact: null as unknown },
}));

vi.mock('./SetupProgress', async () => {
  const actual = await vi.importActual<typeof import('./SetupProgress')>('./SetupProgress');
  return { ...actual, useSetupArtifact: () => mocks.setupSupport };
});

vi.mock('./hostStart', async () => {
  const actual = await vi.importActual<typeof import('./hostStart')>('./hostStart');
  return {
    ...actual,
    startHostRun: mocks.startHostRun,
    readHostRun: mocks.readHostRun,
    stopHostRun: mocks.stopHostRun,
  };
});

vi.mock('./setup', async () => {
  const actual = await vi.importActual<typeof import('./setup')>('./setup');
  return {
    ...actual,
    startSetup: mocks.startSetup,
    readSetup: mocks.readSetup,
    stopSetup: mocks.stopSetup,
  };
});

// A setup job's Sign in window: its terminal has its own suites. It stands in with its contract.
vi.mock('../auth/SignInDialog', () => ({
  SignInDialog: (props: Record<string, unknown>) => {
    mocks.signIn.push(props);
    if (!props.open) return null;
    return (
      <div data-testid="setup-sign-in">
        <button onClick={props.onSignedIn as () => void}>Simulate signed in</button>
      </div>
    );
  },
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
/** The broker this build carries (`GET /crew/setup/artifact`). */
const ARTIFACT = {
  version: '1.94.0',
  target: 'x86_64-unknown-linux-gnu',
  sha256: 'd'.repeat(64),
  size: 3556248,
  source: 'bundled',
};
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

/** Take the manual path: "Run it yourself in a terminal" (D-HOST keeps it, folded). */
function runItYourself() {
  fireEvent.click(screen.getByRole('button', { name: hostCopy.runYourself }));
}

async function toStart() {
  await fillName();
  fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));
  await screen.findByText(hostCopy.startHeading('hpc.ucsf.edu'));
}

async function throughStart() {
  await toStart();
  runItYourself();
  fireEvent.change(screen.getByLabelText(hostCopy.pasted), { target: { value: PASTE } });
  fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));
  await screen.findByText(hostCopy.createHeading('lab-data', 'hpc.ucsf.edu'));
}

beforeEach(() => {
  vi.stubGlobal('ResizeObserver', ResizeObserverStub);
  resetJoinContextForTests();
  mocks.dockProps.length = 0;
  mocks.signIn.length = 0;
  mocks.previewInvitation.mockReset().mockResolvedValue(PREVIEW);
  mocks.startHostRun.mockReset();
  mocks.readHostRun.mockReset();
  mocks.stopHostRun.mockReset().mockResolvedValue(undefined);
  mocks.setupSupport = { supported: true, artifact: ARTIFACT };
  mocks.startSetup.mockReset();
  mocks.readSetup.mockReset();
  mocks.stopSetup.mockReset().mockResolvedValue(undefined);
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
    // An example, marked as one, never a value that looks typed already (T-42).
    expect(name).toHaveAttribute('placeholder', 'e.g. lab');
    expect(screen.getByLabelText(hostCopy.serverLogin)).toHaveAttribute(
      'placeholder',
      'e.g. alice@hpc.example.edu'
    );
    fireEvent.change(name, { target: { value: 'Lab Data' } });
    expect(screen.getByText(/Your workspace: lab-data/)).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: /^Private/ })).toBeChecked();
    await waitFor(() => expect(screen.getByLabelText(joinCopy.institution)).toHaveValue('ucsf'));
    expect(screen.getByLabelText(joinCopy.institution)).toBeRequired();
    // The host is the one others match, so the helper says so rather than "as your host uses it".
    expect(screen.getByLabelText(joinCopy.institution)).toHaveAccessibleDescription(
      hostCopy.institutionHelper
    );
    expect(screen.getByText(hostCopy.advancedSummary)).toBeInTheDocument();
  });

  it('shows the steps as one numbered row in the header, not a second list in the body', async () => {
    renderHost();
    const steps = screen.getByTestId('crew-host-steps');
    expect(steps.closest('form')).toBeNull();
    const current = steps.querySelector('[data-state="current"]');
    expect(current).toHaveTextContent('1Name');
    expect(steps.querySelectorAll('[data-state="next"]')).toHaveLength(2);
    // A screen reader hears the position once, as the dialog's description.
    expect(screen.getByRole('dialog')).toHaveAccessibleDescription(hostCopy.stepOf(1, 3, 'Name'));
    expect(screen.queryByRole('list', { name: hostCopy.stepsLabel })).toBeNull();

    await fillName();
    fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));
    await screen.findByText(hostCopy.startHeading('hpc.ucsf.edu'));
    expect(steps.querySelector('[data-state="done"]')).toHaveTextContent('Name');
    expect(steps.querySelector('[data-state="current"]')).toHaveTextContent('2Start');
  });

  it('answers a blank required field under it instead of with a native bubble', async () => {
    const view = renderHost();
    const name = screen.getByLabelText(hostCopy.workspaceName);
    expect(name.closest('form')).toHaveAttribute('novalidate');
    fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));
    const message = await screen.findAllByText(joinCopy.fieldRequired);
    expect(message[0]).toHaveClass('text-text-danger');
    expect(name).toHaveAttribute('aria-invalid', 'true');
    expect(name).toHaveFocus();
    expect(view.crew().prepareHostingDevice).not.toHaveBeenCalled();
  });

  it('prepares the hosting identity on Continue and shows the start command with its key', async () => {
    const view = renderHost();
    await fillName();
    fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));

    expect(await screen.findByText(hostCopy.startHeading('hpc.ucsf.edu'))).toBeInTheDocument();
    expect(view.crew().prepareHostingDevice).toHaveBeenCalledOnce();
    expect(screen.getByText(hostCopy.stepOf(2, 3, 'Start'))).toBeInTheDocument();
    expect(screen.getByText(hostCopy.consequence)).toBeInTheDocument();

    // The step leads with its action, Set up and start Crew (OW-9), and no shell block (U1): the
    // commands are the manual path's, folded under it.
    const start = screen.getByRole('button', { name: hostCopy.setUpAndStart });
    expect(start).toHaveFocus();
    expect(start).toHaveAccessibleDescription(hostCopy.setUpAndStartHint('hpc.ucsf.edu', 'alice'));
    expect(screen.queryByText(hostCopy.runThis('hpc.ucsf.edu', 'alice'))).toBeNull();
    expect(screen.queryByText(/--bootstrap-key/)).toBeNull();
    expect(screen.queryByLabelText(hostCopy.pasted)).toBeNull();
    expect(screen.queryByRole('button', { name: hostCopy.continue })).toBeNull();
    runItYourself();
    expect(screen.getByText(hostCopy.runThis('hpc.ucsf.edu', 'alice'))).toBeVisible();
    const command = screen.getByText(/--name lab-data --bootstrap-key c{64}/);
    // One command per line, scrolling sideways rather than wrapping mid-flag (T-42).
    expect(command.closest('[data-slot="copy-field"]')).toHaveClass('crew-onboard-command');
    expect(screen.getByText(hostCopy.runYourselfBody('hpc.ucsf.edu', 'alice'))).toBeVisible();
    expect(screen.getByRole('button', { name: hostCopy.continue })).toBeInTheDocument();

    // The embedded terminal is a plain shell: nothing is typed or run for the person.
    fireEvent.click(screen.getByRole('button', { name: hostCopy.openTerminal }));
    expect(screen.getByTestId('in-app-terminal-dock')).toBeInTheDocument();
    expect(Object.keys(mocks.dockProps[0]).sort()).toEqual(['onClose', 'onEmptied', 'open']);

    expect(screen.getByText(hostCopy.notSignedIn)).toBeInTheDocument();
    expect(screen.getByText('ssh alice@hpc.ucsf.edu')).toBeInTheDocument();
    expect(screen.getByText(hostCopy.confirmServer)).toBeInTheDocument();
  });

  it('says what went wrong when the paste is not what Crew prints', async () => {
    mocks.previewInvitation.mockRejectedValue(
      new CrewHttpError('not an invitation', 400, CREW_INVITATION_INVALID)
    );
    renderHost();
    await toStart();
    runItYourself();
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
    await toStart();
    runItYourself();
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
    await toStart();
    runItYourself();
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
    await toStart();
    runItYourself();
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
    // An answer this computer once recorded for the id does not stand for the new workspace.
    updateJoinContext('conn-host', { suggestName: false });
    await throughStart();

    expect(screen.getByText('3F2A 9C1E 77B0 D4E1')).toBeInTheDocument();
    // The fingerprint says what it is for (T-34).
    expect(screen.getByText(hostCopy.fingerprintHelper)).toBeInTheDocument();
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
    // The host never joins, so Create makes the server-account name offer the join flow would
    // have (Q3-51): the checklist asks before the first invitation goes out.
    expect(readJoinContext('conn-host')).toMatchObject({ hostSetup: false, suggestName: true });

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
    // It says what the label does — only what the broker enforces — and what Not now leaves open
    // (Q3-45), instead of "Label lab-data as ucsf? This can't be changed later."
    expect(
      await screen.findByRole('dialog', { name: 'Mark lab-data as a ucsf workspace?' })
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        'Agents working in lab-data can then use only models approved for ucsf. This can’t be undone.'
      )
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: hostCopy.labelLater })).toHaveAccessibleDescription(
      'Until then, people can chat and share files in lab-data, but agents can’t work there. You can do this later from Get lab-data ready, or from Privacy… in the workspace menu.'
    );
    expect(document.body.textContent).not.toMatch(/\bLabel\b|changed later/);
    // A separate, explicit confirmation: nothing is set until the person presses it.
    expect(crew.mutate).not.toHaveBeenCalledWith('policy.set', expect.anything());
    fireEvent.click(screen.getByRole('button', { name: 'Mark as ucsf' }));
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
  it('reads the paste as soon as it is pasted, through the terminal text around it (T-27)', async () => {
    renderHost();
    await toStart();
    runItYourself();

    // A prompt before the JSON, and a copy that broke the long line inside the socket path.
    const noisy = [
      'alice@hpc:~$ "$HOME/.local/bin/biorouter-crew" status --state-dir lab-data',
      'alice@hpc:~$ {"workspace_id":"w-1","socket":"/tmp/crew-1000-abc/bro',
      'ker.sock","host_uid":1000}',
      'alice@hpc:~$ ',
    ].join('\n');
    fireEvent.change(screen.getByLabelText(hostCopy.pasted), { target: { value: noisy } });

    const found = await screen.findByTestId('crew-host-paste-found');
    expect(found).toHaveTextContent(hostCopy.found('lab-data', 'hpc.ucsf.edu'));
    expect(found.closest('[aria-live="polite"]')).not.toBeNull();
    expect(mocks.previewInvitation).toHaveBeenCalledWith(
      '{"workspace_id":"w-1","socket":"/tmp/crew-1000-abc/broker.sock","host_uid":1000}',
      {},
      expect.any(AbortSignal)
    );

    // Continue moves on with what was read, without asking the daemon again.
    fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));
    await screen.findByText(hostCopy.createHeading('lab-data', 'hpc.ucsf.edu'));
    expect(mocks.previewInvitation).toHaveBeenCalledOnce();
  });

  it.each([
    [
      'Crew was still starting',
      '{"started_pid":4242,"state":"starting","status_command":"status"}',
      hostCopy.pasteStarting,
    ],
    [
      'the copy stopped partway',
      'alice@hpc:~$ {"workspace_id":"w-1","socket":"/tmp/crew-1000-',
      hostCopy.pasteCutOff,
    ],
    [
      'biorouter-crew is missing',
      'bash: /home/alice/.local/bin/biorouter-crew: No such file or directory',
      hostCopy.pasteNotInstalled('hpc.ucsf.edu'),
    ],
  ])('says exactly what is wrong when %s, before Continue', async (_case, paste, message) => {
    renderHost();
    await toStart();
    runItYourself();
    const box = screen.getByLabelText(hostCopy.pasted);
    fireEvent.change(box, { target: { value: paste } });

    expect(await screen.findByText(message)).toBeInTheDocument();
    expect(box).toHaveAttribute('aria-invalid', 'true');
    // Nothing the daemon could read, so it was not asked; Continue stays on Start.
    expect(mocks.previewInvitation).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));
    expect(await screen.findByText(message)).toBeInTheDocument();
    expect(screen.getByText(hostCopy.stepOf(2, 3, 'Start'))).toBeInTheDocument();
  });

  it('opens the workspace it just created connected, and ignores an error from before it existed (T-09)', async () => {
    const connect = vi.fn().mockResolvedValue(undefined);
    const refresh = vi.fn().mockResolvedValue(undefined);
    const bootstrap = deferred<unknown>();
    const request = vi.fn((method: string) =>
      method === 'auth.bootstrap' ? bootstrap.promise : Promise.resolve({})
    );
    const saved = fakeConnection({
      id: 'conn-host',
      ssh_target: 'alice@hpc.ucsf.edu',
      status: 'disconnected',
    });
    const view = renderHost({
      connect,
      refresh,
      request: request as unknown as CrewController['request'],
      saveConnection: vi.fn().mockResolvedValue(saved),
    });
    await throughStart();
    fireEvent.click(screen.getByRole('button', { name: hostCopy.create }));
    await waitFor(() => expect(view.crew().saveConnection).toHaveBeenCalled());
    view.update({
      connectionId: 'conn-host',
      connections: [saved],
      connection: { ...saved, status: 'connected' },
    });
    await waitFor(() =>
      expect(request).toHaveBeenCalledWith(
        'auth.bootstrap',
        { public_key: DEVICE_KEY },
        { mutation: true }
      )
    );

    // Before the workspace existed, its observer gave up and the connection dropped.
    view.update({
      connection: { ...saved, status: 'disconnected' },
      refreshError: 'Room observation ended.',
    });
    await act(async () => bootstrap.resolve({}));

    // Connected again first, then verified.
    await waitFor(() => expect(connect).toHaveBeenCalledWith({ userInitiated: true }));
    await waitFor(() => expect(refresh).toHaveBeenCalled());
    expect(connect.mock.invocationCallOrder[0]).toBeLessThan(
      refresh.mock.invocationCallOrder[refresh.mock.invocationCallOrder.length - 1]
    );
    // The error from before is not a verdict on the new workspace: the dialog waits.
    expect(view.crew().closeDialog).not.toHaveBeenCalled();

    view.update({
      refreshError: null,
      connection: { ...saved, status: 'connected' },
      snapshot: fakeSnapshot({
        workspace: {
          id: 'workspace-1',
          host_uid: 1000,
          mode: 'private',
          institution_id: null,
          policy_epoch: 1,
          host_principal_id: 'p-alice',
          name: 'lab-data',
        },
      }),
      observedPrivacy: {
        connectionId: 'conn-host',
        mode: 'private',
        institutionId: 'ucsf',
        policyEpoch: 1,
      },
    });
    expect(await screen.findByText(hostCopy.labelTitle('lab-data', 'ucsf'))).toBeInTheDocument();
  });

  it('asks to mark the workspace in the institution’s own name, and says why it asks again (Q4-47)', async () => {
    // The configured provider publishes "UCSF" for `ucsf`.
    mocks.getProviders.mockResolvedValue([
      {
        name: 'versa',
        is_configured: true,
        affiliation: { kind: 'institutions', institutions: [{ id: 'ucsf', display_name: 'UCSF' }] },
      },
    ]);
    const saved = fakeConnection({ id: 'conn-host', status: 'connected' });
    const view = renderHost({ saveConnection: vi.fn().mockResolvedValue(saved) });
    await throughStart();
    fireEvent.click(screen.getByRole('button', { name: hostCopy.create }));
    await waitFor(() => expect(view.crew().saveConnection).toHaveBeenCalled());
    view.update({ connectionId: 'conn-host', connections: [saved], connection: saved });
    await waitFor(() => expect(view.crew().setJoinStatus).toHaveBeenCalledWith('joined'));
    view.update({
      snapshot: fakeSnapshot({
        workspace: {
          id: 'workspace-1',
          host_uid: 1000,
          mode: 'private',
          institution_id: null,
          policy_epoch: 1,
          host_principal_id: 'p-alice',
          name: 'lab-data',
        },
      }),
      observedPrivacy: {
        connectionId: 'conn-host',
        mode: 'private',
        institutionId: 'ucsf',
        policyEpoch: 1,
      },
    });
    // Title, body and button all say "UCSF", as every chip does, never `ucsf`.
    expect(
      await screen.findByRole('dialog', { name: 'Mark lab-data as a UCSF workspace?' })
    ).toBeInTheDocument();
    // The Host dialog already asked: say what that set, and what this sets that it did not. That
    // dialog has closed, so this never names its steps ("Step 1 set UCSF…", F9).
    expect(screen.getByTestId('crew-host-label-why')).toHaveTextContent(
      'You set UCSF as this computer’s institution when you created lab-data. Marking lab-data sets it for the workspace itself, for everyone who works there.'
    );
    expect(screen.getByTestId('crew-host-label-why')).not.toHaveTextContent(/\bStep\b/);
    expect(
      screen.getByText(
        'Agents working in lab-data can then use only models approved for UCSF. This can’t be undone.'
      )
    ).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/\bucsf\b/);
    fireEvent.click(screen.getByRole('button', { name: 'Mark as UCSF' }));
    // The label written is still the canonical ID.
    await waitFor(() =>
      expect(view.crew().mutate).toHaveBeenCalledWith('policy.set', {
        mode: 'private',
        institution_id: 'ucsf',
      })
    );
  });

  it('closes on an observation error that arose while verifying, for the connection bar to explain', async () => {
    const saved = fakeConnection({ id: 'conn-host', status: 'connected' });
    const view = renderHost({ saveConnection: vi.fn().mockResolvedValue(saved) });
    await throughStart();
    fireEvent.click(screen.getByRole('button', { name: hostCopy.create }));
    await waitFor(() => expect(view.crew().saveConnection).toHaveBeenCalled());
    view.update({ connectionId: 'conn-host', connections: [saved], connection: saved });
    await waitFor(() => expect(view.crew().setJoinStatus).toHaveBeenCalledWith('joined'));
    expect(view.crew().closeDialog).not.toHaveBeenCalled();

    view.update({ refreshError: 'The workspace refused this computer.' });
    await waitFor(() => expect(view.crew().closeDialog).toHaveBeenCalled());
  });
  describe('the dialog itself', () => {
    it('opens at the top, as a Crew dialog, with a secondary Cancel (Q2-25, Q2-26)', () => {
      renderHost();
      const dialog = screen.getByRole('dialog');
      expect(dialog).toHaveAttribute('data-anchor', 'top');
      expect(dialog).toHaveClass('crew-dialog');
      expect(screen.getByRole('button', { name: joinCopy.cancel })).toHaveClass('br-fill-rest');
    });

    it('keeps focus on the workspace name while the menu that opened it closes (Q2-27)', async () => {
      renderHost();
      const name = screen.getByLabelText(hostCopy.workspaceName);
      expect(name).toHaveFocus();
      act(() => name.blur());
      expect(document.activeElement).toBe(document.body);
      await waitFor(() => expect(name).toHaveFocus());
    });

    it('gives the agent’s permissions their own row, out of Advanced, as me by default (Q2-37, OW-6)', async () => {
      const view = renderHost({
        saveConnection: vi.fn().mockResolvedValue(fakeConnection({ id: 'conn-host' })),
      });
      await fillName();
      // Advanced, folded, says what it holds in plain words, the same as Join's (Q3-49).
      expect(screen.getByRole('button', { name: 'Advanced' })).toHaveAccessibleDescription(
        'server connection details'
      );
      expect(hostCopy.advancedSummary).toBe(joinCopy.advancedSummary);
      expect(hostCopy.advancedSummary).not.toMatch(/agent|Port|SSH/);
      const row = screen.getByRole('button', { name: joinCopy.agentHeading('hpc.ucsf.edu') });
      // A new connection made by the setup flow defaults to As me on the server (OW-6).
      expect(row).toHaveAccessibleDescription('runs commands as you');
      fireEvent.click(screen.getByRole('button', { name: 'Advanced' }));
      expect(screen.queryByRole('switch', { name: joinCopy.remoteExecution })).toBeNull();
      fireEvent.click(row);
      // Three levels, the default with its one warning; the host's own account says more.
      expect(screen.getByRole('radio', { name: /^As me on the server/ })).toBeChecked();
      expect(screen.getByTestId('crew-onboard-full-warning')).toHaveTextContent(
        joinCopy.fullWarning('alice', 'hpc.ucsf.edu')
      );
      expect(screen.getByTestId('crew-onboard-full-warning')).toHaveTextContent(
        joinCopy.fullWarningHost('lab-data')
      );
      fireEvent.click(screen.getByRole('radio', { name: /^In a folder/ }));
      expect(screen.queryByTestId('crew-onboard-full-warning')).toBeNull();
      // The work folder in plain words, not "an absolute path on the server" (Q3-49).
      expect(screen.getByLabelText(joinCopy.remoteFolder)).toHaveAccessibleDescription(
        'Optional. A folder on hpc.ucsf.edu, starting with /. Your agent can read and write files there.'
      );
      fireEvent.change(screen.getByLabelText(joinCopy.remoteFolder), {
        target: { value: '/srv/lab' },
      });
      fireEvent.click(screen.getByRole('switch', { name: joinCopy.remoteExecution }));

      fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));
      await screen.findByText(hostCopy.startHeading('hpc.ucsf.edu'));
      runItYourself();
      fireEvent.change(screen.getByLabelText(hostCopy.pasted), { target: { value: PASTE } });
      fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));
      await screen.findByText(hostCopy.createHeading('lab-data', 'hpc.ucsf.edu'));
      fireEvent.click(screen.getByRole('button', { name: hostCopy.create }));
      await waitFor(() =>
        expect(view.crew().saveConnection).toHaveBeenCalledWith(
          expect.objectContaining({ remote_root: '/srv/lab', remote_execution: true })
        )
      );
    });

    it('names a resumed setup’s server by the host’s own SSH alias (D-ALIAS)', async () => {
      updateJoinContext('conn-1', { hostSetup: true, workspaceName: 'lab' });
      const connection = {
        ...fakeConnection({ ssh_target: 'alice@52.33.141.141', status: 'connected' }),
        server_label: 'lab-server',
      };
      renderHost({ connectionId: 'conn-1', connection, connections: [connection] });
      expect(await screen.findByText(hostCopy.createHeading('lab', 'lab-server'))).toBeVisible();
      // One name from the start: nothing to explain.
      expect(screen.queryByTestId('crew-host-server-alias')).toBeNull();
    });

    it('keeps the server word step 1 showed through Create, and explains the alias (Q4-34)', async () => {
      const connectDone = deferred();
      const connect = vi.fn(() => connectDone.promise);
      // The host typed the address; their SSH config calls it lab-server.
      const saved = {
        ...fakeConnection({
          id: 'conn-host',
          ssh_target: 'iris@52.33.141.141',
          status: 'disconnected',
        }),
        server_label: 'lab-server',
      };
      const view = renderHost({ connect, saveConnection: vi.fn().mockResolvedValue(saved) });
      fireEvent.change(screen.getByLabelText(hostCopy.workspaceName), {
        target: { value: 'Wong Lab' },
      });
      fireEvent.change(screen.getByLabelText(hostCopy.serverLogin), {
        target: { value: 'iris@52.33.141.141' },
      });
      await waitFor(() => expect(screen.getByLabelText(joinCopy.institution)).toHaveValue('ucsf'));
      expect(
        screen.getByRole('button', { name: joinCopy.agentHeading('52.33.141.141') })
      ).toBeInTheDocument();
      fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));
      await screen.findByText(hostCopy.startHeading('52.33.141.141'));
      // The step-2 fold names the situation and the server, not a program (Q4-37).
      expect(
        screen.getByRole('button', { name: hostCopy.notInstalled('52.33.141.141') })
      ).toHaveTextContent('Crew isn’t on 52.33.141.141 yet?');
      expect(document.body.textContent).not.toMatch(/biorouter-crew isn’t installed/);
      runItYourself();
      fireEvent.change(screen.getByLabelText(hostCopy.pasted), { target: { value: PASTE } });
      fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));
      const heading = await screen.findByText(hostCopy.createHeading('wong-lab', '52.33.141.141'));
      expect(screen.queryByTestId('crew-host-server-alias')).toBeNull();

      fireEvent.click(screen.getByRole('button', { name: hostCopy.create }));
      await waitFor(() => expect(view.crew().saveConnection).toHaveBeenCalled());
      // Saved: the daemon now calls the server lab-server. While Create runs, the heading keeps
      // the word it had (it flipped to "on lab-server" mid-Create), and one line relates the two.
      view.update({ connectionId: 'conn-host', connections: [saved], connection: saved });
      await waitFor(() => expect(connect).toHaveBeenCalled());
      expect(heading).toHaveTextContent('wong-lab on 52.33.141.141');
      expect(screen.queryByText(hostCopy.createHeading('wong-lab', 'lab-server'))).toBeNull();
      expect(screen.getByTestId('crew-host-server-alias')).toHaveTextContent(
        'Your SSH settings call this server lab-server.'
      );
    });

    it('follows the login again after Back to step 1', async () => {
      renderHost();
      await toStart();
      fireEvent.click(screen.getByRole('button', { name: hostCopy.back }));
      fireEvent.change(screen.getByLabelText(hostCopy.serverLogin), {
        target: { value: 'alice@lab-server' },
      });
      expect(
        screen.getByRole('button', { name: joinCopy.agentHeading('lab-server') })
      ).toBeInTheDocument();
      fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));
      expect(await screen.findByText(hostCopy.startHeading('lab-server'))).toBeInTheDocument();
    });
  });

  describe('Set up and start Crew (OW-9)', () => {
    const SHOWN = [
      'umask 077',
      'mkdir -p "$HOME/.local/share/biorouter-crew"',
      `"$HOME/.local/bin/biorouter-crew" start --state-dir "$HOME/.local/share/biorouter-crew/lab-data" --name lab-data --bootstrap-key ${HOSTING_KEY}`,
      '"$HOME/.local/bin/biorouter-crew" status --state-dir "$HOME/.local/share/biorouter-crew/lab-data"',
    ].join('\n');
    const LINE = 'brcrew1:eyJ2IjoxfQ';
    const DRAFT = '# hpc.ucsf.edu\n\nUbuntu 22.04, 16 CPUs, no job scheduler.';

    function job(overrides: Partial<SetupRun> = {}): SetupRun {
      return {
        jobId: 'setup-1',
        intent: 'host',
        state: 'running',
        stage: 'probe',
        stages: [],
        server: null,
        artifact: ARTIFACT,
        outcome: null,
        error: null,
        output: '',
        ...overrides,
      };
    }
    function finished(outcome: Partial<NonNullable<SetupRun['outcome']>> = {}): SetupRun {
      return job({
        state: 'finished',
        stage: 'done',
        outcome: {
          installedVersion: '1.94.0',
          reused: false,
          host: { kind: 'found', text: LINE },
          hostCommand: SHOWN,
          connectionId: null,
          workFolder: '/home/alice/crew-work/lab-data',
          remoteAccess: null,
          contextDraft: DRAFT,
          ...outcome,
        },
      });
    }

    it('sets Crew up as the login typed, in plain steps, then moves on to Create', async () => {
      mocks.startSetup.mockResolvedValue(job());
      mocks.readSetup
        .mockResolvedValueOnce(job({ stage: 'transfer' }))
        .mockResolvedValueOnce(job({ stage: 'start' }))
        .mockResolvedValue(finished());
      renderHost();
      await toStart();
      const start = screen.getByRole('button', { name: hostCopy.setUpAndStart });
      fireEvent.click(start);

      // Only what the route takes: this computer's host setup, the login typed and the name.
      await waitFor(() => expect(mocks.startSetup).toHaveBeenCalledOnce());
      const [request] = mocks.startSetup.mock.calls[0];
      expect(request).toEqual({
        target: {
          kind: 'pending',
          preparation_id: 'prep-1',
          ssh_target: 'alice@hpc.ucsf.edu',
          port: null,
          identity_file: null,
          proxy_jump: null,
        },
        intent: 'host',
        workspace_name: 'lab-data',
      });
      expect(JSON.stringify(request)).not.toMatch(/biorouter-crew|umask|bootstrap/);
      expect(mocks.startHostRun).not.toHaveBeenCalled();

      // The steps in plain words; Signing in only once a job asks for it.
      const steps = await screen.findByRole('list', { name: setupCopy.stepsLabel });
      expect(
        within(steps)
          .getAllByRole('listitem')
          .map((item) => item.textContent)
      ).toEqual([
        setupCopy.stages.checking,
        setupCopy.stages.installing,
        setupCopy.stages.starting,
        setupCopy.stages.ready,
      ]);
      // Busy, not disabled: focus stays on it, and pressing again starts nothing.
      expect(start).toHaveAttribute('aria-disabled', 'true');
      expect(start).toHaveFocus();
      fireEvent.click(start);
      expect(mocks.startSetup).toHaveBeenCalledOnce();
      expect(screen.getByRole('button', { name: hostCopy.back })).toBeDisabled();

      await screen.findByText(hostCopy.createHeading('lab-data', 'hpc.ucsf.edu'), undefined, {
        timeout: 4000,
      });
      expect(mocks.readSetup).toHaveBeenCalledWith('setup-1', expect.any(AbortSignal));
      // Read as a paste is read, exactly as the older start's output is.
      expect(mocks.previewInvitation).toHaveBeenCalledWith(LINE);
      expect(screen.getByRole('button', { name: hostCopy.create })).toHaveFocus();
    });

    it('creates with the folder setup made, as me, then opens the workspace and saves its context', async () => {
      mocks.startSetup.mockResolvedValue(finished());
      const saved = fakeConnection({
        id: 'prep-1',
        ssh_target: 'alice@hpc.ucsf.edu',
        status: 'connected',
      });
      const view = renderHost({ saveConnection: vi.fn().mockResolvedValue(saved) });
      await toStart();
      fireEvent.click(screen.getByRole('button', { name: hostCopy.setUpAndStart }));
      await screen.findByText(hostCopy.createHeading('lab-data', 'hpc.ucsf.edu'));
      fireEvent.click(screen.getByRole('button', { name: hostCopy.create }));

      const crew = view.crew();
      await waitFor(() =>
        expect(crew.saveConnection).toHaveBeenCalledWith(
          expect.objectContaining({
            remote_root: '/home/alice/crew-work/lab-data',
            remote_execution: true,
            remote_access: 'full',
            preparation_id: 'prep-1',
          })
        )
      );
      view.update({ connectionId: 'prep-1', connections: [saved], connection: saved });
      await waitFor(() => expect(crew.setJoinStatus).toHaveBeenCalledWith('joined'));
      // Verified, with a broker that takes both: open governance, then the context draft.
      view.update({
        capabilities: ['presence_v1', 'open_governance_v1', 'workspace_context_v1'],
        snapshot: fakeSnapshot({
          workspace: {
            id: 'workspace-1',
            host_uid: 1000,
            mode: 'private',
            institution_id: null,
            policy_epoch: 1,
            host_principal_id: 'p-alice',
            name: 'lab-data',
          },
        }),
        observedPrivacy: {
          connectionId: 'prep-1',
          mode: 'private',
          institutionId: 'ucsf',
          policyEpoch: 1,
        },
      });
      await waitFor(() =>
        expect(crew.request).toHaveBeenCalledWith(
          'workspace.set_governance',
          { governance: 'open' },
          { mutation: true }
        )
      );
      await waitFor(() =>
        expect(crew.request).toHaveBeenCalledWith(
          'workspace.context.set',
          { text: DRAFT, expected_revision: 0 },
          { mutation: true }
        )
      );
      // Then the institution, as before: its own explicit confirmation.
      expect(await screen.findByText(hostCopy.labelTitle('lab-data', 'ucsf'))).toBeInTheDocument();
      expect(crew.mutate).not.toHaveBeenCalledWith('policy.set', expect.anything());
    });

    it('asks a broker for neither setting when it does not advertise them', async () => {
      mocks.startSetup.mockResolvedValue(finished());
      const saved = fakeConnection({ id: 'prep-1', status: 'connected' });
      const view = renderHost({ saveConnection: vi.fn().mockResolvedValue(saved) });
      await toStart();
      fireEvent.click(screen.getByRole('button', { name: hostCopy.setUpAndStart }));
      await screen.findByText(hostCopy.createHeading('lab-data', 'hpc.ucsf.edu'));
      fireEvent.click(screen.getByRole('button', { name: hostCopy.create }));
      await waitFor(() => expect(view.crew().saveConnection).toHaveBeenCalled());
      view.update({ connectionId: 'prep-1', connections: [saved], connection: saved });
      await waitFor(() => expect(view.crew().setJoinStatus).toHaveBeenCalledWith('joined'));
      view.update({
        capabilities: ['presence_v1'],
        snapshot: fakeSnapshot(),
        observedPrivacy: {
          connectionId: 'prep-1',
          mode: 'private',
          institutionId: 'ucsf',
          policyEpoch: 1,
        },
      });
      await waitFor(() => expect(view.crew().closeDialog).toHaveBeenCalled());
      expect(view.crew().request).not.toHaveBeenCalledWith(
        'workspace.set_governance',
        expect.anything(),
        expect.anything()
      );
      expect(view.crew().request).not.toHaveBeenCalledWith(
        'workspace.context.set',
        expect.anything(),
        expect.anything()
      );
    });

    it('never reads a setup whose start command is not the one shown', async () => {
      mocks.startSetup.mockResolvedValue(finished({ hostCommand: `${SHOWN}\nrm -rf ~` }));
      renderHost();
      await toStart();
      fireEvent.click(screen.getByRole('button', { name: hostCopy.setUpAndStart }));
      expect(await screen.findByTestId('crew-host-start-problem')).toHaveTextContent(
        hostCopy.startCommandChanged
      );
      expect(mocks.previewInvitation).not.toHaveBeenCalled();
      // The manual path opens: it always works.
      expect(screen.getByLabelText(hostCopy.pasted)).toBeInTheDocument();
    });

    it('opens the Sign in window for the setup job when the server asks', async () => {
      mocks.startSetup.mockResolvedValue(job({ state: 'needs_sign_in', stage: 'sign_in' }));
      mocks.readSetup.mockResolvedValue(job({ state: 'needs_sign_in', stage: 'sign_in' }));
      renderHost();
      await toStart();
      fireEvent.click(screen.getByRole('button', { name: hostCopy.setUpAndStart }));
      await screen.findByTestId('setup-sign-in');
      expect(mocks.signIn[mocks.signIn.length - 1]).toMatchObject({
        target: { kind: 'setup', jobId: 'setup-1' },
        open: true,
        host: 'hpc.ucsf.edu',
      });
      const steps = screen.getByRole('list', { name: setupCopy.stepsLabel });
      expect(within(steps).getByText(setupCopy.stages['signing-in'])).toBeInTheDocument();
    });

    it('says what a refusal means and opens the manual path, which always works', async () => {
      mocks.startSetup.mockResolvedValue(
        job({
          state: 'failed',
          stage: 'verify_installed',
          error: {
            code: 'crew_host_home_unsafe',
            message: 'Home is group-writable.',
            detail: null,
          },
        })
      );
      renderHost();
      await toStart();
      fireEvent.click(screen.getByRole('button', { name: hostCopy.setUpAndStart }));
      expect(await screen.findByTestId('crew-setup-problem')).toHaveTextContent(
        setupCopy.error('crew_host_home_unsafe', 'hpc.ucsf.edu') as string
      );
      expect(screen.getByLabelText(hostCopy.pasted)).toBeInTheDocument();
    });

    it('keeps the manual install generated from the broker this build carries', async () => {
      renderHost();
      await toStart();
      fireEvent.click(screen.getByRole('button', { name: hostCopy.notInstalled('hpc.ucsf.edu') }));
      const manual = await screen.findByTestId('crew-manual-install');
      expect(manual.textContent).toContain(
        'scp ~/Downloads/biorouter-crew-1.94.0-linux-x86_64 alice@hpc.ucsf.edu:biorouter-crew-1.94.0-linux-x86_64'
      );
      expect(manual.textContent).toContain(`${'d'.repeat(64)}  biorouter-crew-1.94.0-linux-x86_64`);
      expect(manual.textContent).not.toMatch(/replace\/with|\/path\/to/);
      expect(screen.getByRole('button', { name: setupCopy.saveBroker })).toBeInTheDocument();
    });

    it('takes the older start straight away when the daemon is known to predate setup', async () => {
      // A daemon before automatic setup answers its routes 404 (`useSetupArtifact`).
      mocks.setupSupport = { supported: false, artifact: null };
      mocks.startHostRun.mockResolvedValue({
        jobId: 'job-1',
        command: SHOWN,
        state: 'running',
        output: '',
        result: null,
        error: null,
      });
      mocks.readHostRun.mockReturnValue(new Promise(() => {}));
      renderHost();
      await fillName();
      // As me on the server is not offered by a daemon that would refuse it.
      expect(
        screen.getByRole('button', { name: joinCopy.agentHeading('hpc.ucsf.edu') })
      ).toHaveAccessibleDescription('is off');
      fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));
      await screen.findByText(hostCopy.startHeading('hpc.ucsf.edu'));
      const start = screen.getByRole('button', { name: hostCopy.setUpAndStart });
      expect(start).toHaveAccessibleDescription(hostCopy.startForMeHint('hpc.ucsf.edu', 'alice'));
      fireEvent.click(start);
      await waitFor(() => expect(mocks.startHostRun).toHaveBeenCalledOnce());
      expect(mocks.startSetup).not.toHaveBeenCalled();
    });
  });

  // A daemon before automatic setup answers its route 404: the same click takes the older start,
  // which runs the commands shown and nothing else (D-HOST).
  describe('the older start, on a daemon before setup (D-HOST)', () => {
    beforeEach(() => {
      mocks.startSetup.mockRejectedValue(new CrewHttpError('Crew request failed (404)', 404));
    });
    const SHOWN = [
      'umask 077',
      'mkdir -p "$HOME/.local/share/biorouter-crew"',
      `"$HOME/.local/bin/biorouter-crew" start --state-dir "$HOME/.local/share/biorouter-crew/lab-data" --name lab-data --bootstrap-key ${HOSTING_KEY}`,
      '"$HOME/.local/bin/biorouter-crew" status --state-dir "$HOME/.local/share/biorouter-crew/lab-data"',
    ].join('\n');
    const LINE = 'brcrew1:eyJ2IjoxfQ';

    function run(overrides: Record<string, unknown> = {}) {
      return {
        jobId: 'job-1',
        command: SHOWN,
        state: 'running',
        output: '',
        result: null,
        error: null,
        ...overrides,
      };
    }

    it('runs exactly the commands shown, as the login typed, then reads the result and moves on', async () => {
      mocks.startHostRun.mockResolvedValue(run({ output: 'starting Crew…\n' }));
      mocks.readHostRun.mockResolvedValue(
        run({
          state: 'finished',
          output: `starting Crew…\n{"invitation":"${LINE}"}\n`,
          exit_code: 0,
          result: { kind: 'found', text: LINE },
        })
      );
      renderHost();
      await toStart();

      const start = screen.getByRole('button', { name: hostCopy.setUpAndStart });
      fireEvent.click(start);
      // Only what the route accepts: the host setup, the name and the login. Never a command.
      await waitFor(() => expect(mocks.startHostRun).toHaveBeenCalledOnce());
      const [request] = mocks.startHostRun.mock.calls[0];
      expect(request).toEqual({
        preparation_id: 'prep-1',
        workspace_name: 'lab-data',
        ssh_target: 'alice@hpc.ucsf.edu',
        port: null,
        identity_file: null,
        proxy_jump: null,
      });
      expect(JSON.stringify(request)).not.toMatch(/biorouter-crew|umask|bootstrap/);

      // Its output streams in, in a region a keyboard can reach, while it runs.
      const output = await screen.findByTestId('crew-host-start-output');
      expect(output).toHaveTextContent('starting Crew…');
      expect(output).toHaveAttribute('tabindex', '0');
      expect(screen.getByText(hostCopy.startRunning('hpc.ucsf.edu'))).toBeInTheDocument();
      // Busy, not disabled: focus stays on it.
      expect(start).toHaveAttribute('aria-disabled', 'true');
      expect(start).toHaveFocus();
      // The text Run it yourself shows is the one the daemon reports running.
      runItYourself();
      expect(screen.getByText(/--name lab-data --bootstrap-key c{64}/).textContent).toBe(SHOWN);

      // Read as a paste is read, then on to Create with nothing to paste; focus follows.
      await screen.findByText(hostCopy.createHeading('lab-data', 'hpc.ucsf.edu'), undefined, {
        timeout: 3000,
      });
      expect(mocks.readHostRun).toHaveBeenCalledWith('job-1', expect.any(AbortSignal));
      expect(mocks.previewInvitation).toHaveBeenCalledWith(LINE);
      expect(screen.getByRole('button', { name: hostCopy.create })).toHaveFocus();
      expect(screen.getByText('3F2A 9C1E 77B0 D4E1')).toBeInTheDocument();
    });

    it('stops a run whose commands are not the ones shown, and reads nothing from it', async () => {
      mocks.startHostRun.mockResolvedValue(run({ command: `${SHOWN}\nrm -rf ~` }));
      renderHost();
      await toStart();
      fireEvent.click(screen.getByRole('button', { name: hostCopy.setUpAndStart }));

      expect(await screen.findByTestId('crew-host-start-problem')).toHaveTextContent(
        hostCopy.startCommandChanged
      );
      expect(mocks.stopHostRun).toHaveBeenCalledWith('job-1');
      expect(mocks.readHostRun).not.toHaveBeenCalled();
      expect(mocks.previewInvitation).not.toHaveBeenCalled();
      // The manual path opens: it always works.
      expect(screen.getByLabelText(hostCopy.pasted)).toBeInTheDocument();
    });

    it.each([
      [
        'the server wants a password',
        {
          state: 'failed',
          error: {
            code: 'crew_ssh_auth_required',
            message:
              'The server asks for a password or a code, so Biorouter can’t sign in for you. Run the commands yourself in a terminal.',
          },
        },
        'The server asks for a password or a code, so Biorouter can’t sign in for you. Run the commands yourself in a terminal.',
      ],
      [
        'biorouter-crew is missing',
        { state: 'finished', result: { kind: 'problem', problem: 'not_installed' } },
        hostCopy.pasteNotInstalled('hpc.ucsf.edu'),
      ],
      [
        'Crew was still starting',
        { state: 'finished', result: { kind: 'problem', problem: 'starting' } },
        hostCopy.startStarting,
      ],
      [
        'Crew printed an error',
        {
          state: 'finished',
          result: { kind: 'problem', problem: 'server_error', detail: 'state dir is locked' },
        },
        hostCopy.pasteServerError('state dir is locked'),
      ],
      [
        'nothing Crew printed',
        { state: 'finished', result: { kind: 'problem', problem: 'unreadable' } },
        hostCopy.startUnreadable,
      ],
    ])('says what to do when %s, and opens the manual path', async (_case, answer, message) => {
      mocks.startHostRun.mockResolvedValue(run({ ...answer, output: 'ssh: …' }));
      renderHost();
      await toStart();
      const start = screen.getByRole('button', { name: hostCopy.setUpAndStart });
      fireEvent.click(start);

      expect(await screen.findByTestId('crew-host-start-problem')).toHaveTextContent(message);
      expect(screen.getByTestId('crew-host-start-output')).toHaveTextContent('ssh: …');
      expect(screen.getByLabelText(hostCopy.pasted)).toBeInTheDocument();
      expect(screen.getByText(hostCopy.stepOf(2, 3, 'Start'))).toBeInTheDocument();
      // It can be tried again, and focus never left it.
      expect(start).not.toHaveAttribute('aria-disabled');
      expect(start).toHaveFocus();
    });

    it('shows the daemon’s refusal, such as a missing proof that a person asked', async () => {
      mocks.startHostRun.mockRejectedValue(
        new CrewHttpError(
          'Confirm this in Biorouter to continue.',
          403,
          'crew_user_action_required'
        )
      );
      renderHost();
      await toStart();
      fireEvent.click(screen.getByRole('button', { name: hostCopy.setUpAndStart }));
      expect(await screen.findByTestId('crew-host-start-problem')).toHaveTextContent(
        'Confirm this in Biorouter to continue.'
      );
      expect(mocks.readHostRun).not.toHaveBeenCalled();
    });

    it('says a newer background service is needed when the daemon has no such route', async () => {
      mocks.startHostRun.mockRejectedValue(new CrewHttpError('Crew request failed (404)', 404));
      renderHost();
      await toStart();
      fireEvent.click(screen.getByRole('button', { name: hostCopy.setUpAndStart }));
      expect(await screen.findByTestId('crew-host-start-problem')).toHaveTextContent(
        hostCopy.startStaleDaemon
      );
      expect(screen.getByLabelText(hostCopy.pasted)).toBeInTheDocument();
    });

    it('asks for the details the output lacked, prefilled, instead of moving on', async () => {
      mocks.startHostRun.mockResolvedValue(
        run({ state: 'finished', result: { kind: 'found', text: LINE } })
      );
      mocks.previewInvitation.mockResolvedValue({ ...PREVIEW, socket_path: null, owner_uid: null });
      renderHost();
      await toStart();
      fireEvent.click(screen.getByRole('button', { name: hostCopy.setUpAndStart }));

      expect(await screen.findByText(hostCopy.detailsMissing)).toBeInTheDocument();
      expect(screen.getByLabelText(joinCopy.workspaceKey)).toHaveValue(WORKSPACE_KEY);
      expect(screen.getByLabelText(joinCopy.socketPath)).toHaveValue('');
      // Continue takes the typed details.
      fireEvent.change(screen.getByLabelText(joinCopy.socketPath), {
        target: { value: '/tmp/crew-1000-abc/broker.sock' },
      });
      fireEvent.change(screen.getByLabelText(joinCopy.hostUserId), { target: { value: '1000' } });
      fireEvent.click(screen.getByRole('button', { name: hostCopy.continue }));
      await screen.findByText(hostCopy.createHeading('lab-data', 'hpc.ucsf.edu'));
    });

    it('stops a run on Stop, and keeps Back closed while it runs', async () => {
      mocks.startHostRun.mockResolvedValue(run());
      mocks.readHostRun.mockReturnValue(new Promise(() => {}));
      renderHost();
      await toStart();
      fireEvent.click(screen.getByRole('button', { name: hostCopy.setUpAndStart }));
      const stop = await screen.findByRole('button', { name: hostCopy.stop });
      expect(screen.getByRole('button', { name: hostCopy.back })).toBeDisabled();
      fireEvent.click(stop);
      expect(mocks.stopHostRun).toHaveBeenCalledWith('job-1');
    });

    it.each([
      [
        'it was stopped',
        {
          state: 'failed',
          error: {
            code: 'crew_host_start_cancelled',
            message:
              'Stopped. Crew may have started on the server; run the commands yourself to check.',
          },
        },
      ],
      [
        'the server refused the login',
        {
          state: 'failed',
          error: { code: 'crew_ssh_failed', message: 'Biorouter couldn’t reach the server.' },
        },
      ],
    ])(
      'hands focus from Stop to Start it for me when the run ends because %s (Q2-20)',
      async (_case, answer) => {
        mocks.startHostRun.mockResolvedValue(run({ output: 'starting Crew…\n' }));
        let respond: (status: unknown) => void = () => {};
        mocks.readHostRun.mockReturnValue(
          new Promise((resolve) => {
            respond = resolve;
          })
        );
        renderHost();
        await toStart();
        const start = screen.getByRole('button', { name: hostCopy.setUpAndStart });
        fireEvent.click(start);
        // A keyboard user moves to Stop and presses it.
        const stop = await screen.findByRole('button', { name: hostCopy.stop });
        act(() => stop.focus());
        fireEvent.click(stop);
        expect(mocks.stopHostRun).toHaveBeenCalledWith('job-1');

        // The next read answers that the run ended: Stop leaves with it, focus does not.
        await waitFor(() => expect(mocks.readHostRun).toHaveBeenCalled(), { timeout: 3000 });
        act(() => respond(run({ ...answer, output: 'starting Crew…\n' })));
        await screen.findByTestId('crew-host-start-problem');
        expect(screen.queryByRole('button', { name: hostCopy.stop })).toBeNull();
        expect(start).toHaveFocus();
        expect(start).not.toHaveAttribute('aria-disabled');
      }
    );

    it('carries focus from Stop through the read to Create workspace when the run finishes', async () => {
      mocks.startHostRun.mockResolvedValue(run({ output: 'starting Crew…\n' }));
      let respond: (status: unknown) => void = () => {};
      mocks.readHostRun.mockReturnValue(
        new Promise((resolve) => {
          respond = resolve;
        })
      );
      let preview: (value: unknown) => void = () => {};
      mocks.previewInvitation.mockReturnValue(
        new Promise((resolve) => {
          preview = resolve;
        })
      );
      renderHost();
      await toStart();
      const start = screen.getByRole('button', { name: hostCopy.setUpAndStart });
      fireEvent.click(start);
      const stop = await screen.findByRole('button', { name: hostCopy.stop });
      act(() => stop.focus());

      await waitFor(() => expect(mocks.readHostRun).toHaveBeenCalled(), { timeout: 3000 });
      act(() =>
        respond(run({ state: 'finished', exit_code: 0, result: { kind: 'found', text: LINE } }))
      );
      // Reading what it printed: Stop is gone, and focus waits on Start it for me.
      expect(await screen.findByText(hostCopy.startReading)).toBeInTheDocument();
      expect(screen.queryByRole('button', { name: hostCopy.stop })).toBeNull();
      expect(start).toHaveFocus();

      await act(async () => preview(PREVIEW));
      await screen.findByText(hostCopy.createHeading('lab-data', 'hpc.ucsf.edu'));
      expect(screen.getByRole('button', { name: hostCopy.create })).toHaveFocus();
    });

    it('starts nothing while a run is under way, however often it is pressed', async () => {
      mocks.startHostRun.mockResolvedValue(run());
      mocks.readHostRun.mockReturnValue(new Promise(() => {}));
      renderHost();
      await toStart();
      const start = screen.getByRole('button', { name: hostCopy.setUpAndStart });
      fireEvent.click(start);
      await screen.findByRole('button', { name: hostCopy.stop });
      fireEvent.click(start);
      fireEvent.click(start);
      expect(mocks.startHostRun).toHaveBeenCalledOnce();
    });
  });
});
