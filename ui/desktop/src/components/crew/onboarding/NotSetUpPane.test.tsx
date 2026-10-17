import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError } from '../crewApi';
import { notSetUpCopy, setupCopy } from './copy';
import { resetJoinContextForTests, updateJoinContext } from './joinContext';
import { NotSetUpPane } from './NotSetUpPane';
import { requestMemberSetup, resetMemberSetupRequestsForTests, type SetupRun } from './setup';
import { fakeConnection, makeCrew, renderWithCrew } from './testCrew';
import type { CrewController } from '../state/types';

const mocks = vi.hoisted(() => ({
  readSetupArtifact: vi.fn(),
  startSetup: vi.fn(),
  readSetup: vi.fn(),
  stopSetup: vi.fn(),
  signIn: [] as Record<string, unknown>[],
}));

vi.mock('./setup', async () => {
  const actual = await vi.importActual<typeof import('./setup')>('./setup');
  return {
    ...actual,
    readSetupArtifact: mocks.readSetupArtifact,
    startSetup: mocks.startSetup,
    readSetup: mocks.readSetup,
    stopSetup: mocks.stopSetup,
  };
});

// The setup's Sign in window: its terminal has its own suites. It stands in with its contract.
vi.mock('../auth/SignInDialog', () => ({
  SignInDialog: (props: Record<string, unknown>) => {
    mocks.signIn.push(props);
    if (!props.open) return null;
    return (
      <div role="dialog" aria-label="Sign in" data-testid="setup-sign-in">
        <button onClick={props.onSignedIn as () => void}>Simulate signed in</button>
        <button onClick={props.onClose as () => void}>Simulate close</button>
      </div>
    );
  },
}));

const ARTIFACT = {
  version: '1.94.0',
  target: 'x86_64-unknown-linux-gnu',
  sha256: 'd'.repeat(64),
  size: 3556248,
  source: 'bundled',
};

function job(overrides: Partial<SetupRun> = {}): SetupRun {
  return {
    jobId: 'job-1',
    intent: 'member',
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

const connection = fakeConnection();

function renderPane(overrides: Partial<CrewController> = {}) {
  const crew = makeCrew({
    connectionId: 'conn-1',
    connection,
    connections: [connection],
    lastConnectFailure: { kind: 'bridge_missing', message: 'exit_127' },
    ...overrides,
  });
  return renderWithCrew(<NotSetUpPane />, crew);
}

beforeEach(() => {
  resetJoinContextForTests();
  resetMemberSetupRequestsForTests();
  mocks.signIn.length = 0;
  mocks.readSetupArtifact.mockReset().mockResolvedValue(ARTIFACT);
  mocks.startSetup.mockReset();
  mocks.readSetup.mockReset();
  mocks.stopSetup.mockReset().mockResolvedValue(undefined);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('NotSetUpPane', () => {
  it('leads with Set up Crew for my account and folds the older ways', async () => {
    updateJoinContext('conn-1', { hostUsername: 'alice', hostDisplayName: 'Alice Chen' });
    const view = renderPane();

    expect(
      screen.getByRole('heading', { name: notSetUpCopy.title('hpc.ucsf.edu') })
    ).toBeInTheDocument();
    expect(screen.getByText(notSetUpCopy.body)).toBeInTheDocument();
    const setUp = screen.getByRole('button', { name: notSetUpCopy.setUp });
    expect(setUp).toHaveAccessibleDescription(notSetUpCopy.setUpHint('hpc.ucsf.edu', 'bob'));
    // Nothing starts before the person asks.
    expect(mocks.startSetup).not.toHaveBeenCalled();

    // The message to the host is folded, and still the same message.
    const message =
      'Hi Alice, Crew isn’t set up for my account (@bob) on hpc.ucsf.edu yet. Could you or IT install biorouter-crew in ~/.local/bin for me?';
    expect(screen.queryByText(message)).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: notSetUpCopy.askHost }));
    fireEvent.click(screen.getByRole('button', { name: `Copy ${notSetUpCopy.messageLabel}` }));
    await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenCalledWith(message));

    // The manual install is generated from the broker this build carries: no placeholder path.
    fireEvent.click(screen.getByRole('button', { name: notSetUpCopy.installYourself }));
    const manual = await screen.findByTestId('crew-manual-install');
    expect(within(manual).getByText(/sha256sum -c -/)).toBeInTheDocument();
    expect(manual.textContent).toContain(`${'d'.repeat(64)}  biorouter-crew-1.94.0-linux-x86_64`);
    expect(manual.textContent).toContain('bob@hpc.ucsf.edu:biorouter-crew-1.94.0-linux-x86_64');
    expect(manual.textContent).not.toMatch(/replace\/with|\/path\/to/);

    // Try again still connects without setting anything up.
    fireEvent.click(screen.getByRole('button', { name: notSetUpCopy.tryAgain }));
    expect(view.crew().connect).toHaveBeenCalledWith({ userInitiated: true });
    expect(view.crew().registerErrorSlot).toHaveBeenCalledWith('connect');
  });

  it('sets Crew up for the account, in plain steps, and connects once it has', async () => {
    mocks.startSetup.mockResolvedValue(job());
    mocks.readSetup.mockResolvedValueOnce(job({ stage: 'transfer' })).mockResolvedValueOnce(
      job({
        state: 'finished',
        stage: 'done',
        outcome: {
          installedVersion: '1.94.0',
          reused: false,
          host: null,
          hostCommand: null,
          connectionId: 'conn-1',
          workFolder: '/home/bob/crew-work/lab',
          remoteAccess: 'full',
          contextDraft: null,
        },
      })
    );
    const view = renderPane();
    const setUp = screen.getByRole('button', { name: notSetUpCopy.setUp });
    fireEvent.click(setUp);

    // The work folder is named for the workspace: the connection's name, `lab`.
    await waitFor(() =>
      expect(mocks.startSetup).toHaveBeenCalledWith({
        target: { kind: 'connection', connection_id: 'conn-1' },
        intent: 'member',
        workspace_name: 'lab',
      })
    );
    const steps = await screen.findByRole('list', { name: setupCopy.stepsLabel });
    // No sign-in was asked for: four steps, the member's ending in Connecting and Ready.
    expect(
      within(steps)
        .getAllByRole('listitem')
        .map((item) => item.textContent)
    ).toEqual([
      setupCopy.stages.checking,
      setupCopy.stages.installing,
      setupCopy.stages.connecting,
      setupCopy.stages.ready,
    ]);
    // Busy, not disabled: focus stays on the button, and a second press starts nothing.
    expect(setUp).toHaveAttribute('aria-disabled', 'true');
    fireEvent.click(setUp);
    expect(mocks.startSetup).toHaveBeenCalledOnce();

    await waitFor(
      () =>
        expect(within(steps).getByText(setupCopy.stages.installing).closest('li')).toHaveAttribute(
          'aria-current',
          'step'
        ),
      { timeout: 3000 }
    );
    await waitFor(() => expect(view.crew().connect).toHaveBeenCalledWith({ userInitiated: true }), {
      timeout: 3000,
    });
    expect(mocks.readSetup).toHaveBeenCalledWith('job-1', expect.any(AbortSignal));
    expect(screen.getByText(setupCopy.installed('1.94.0'))).toBeInTheDocument();
  });

  it('opens the Sign in window for a job that asks, and goes on once signed in', async () => {
    mocks.startSetup.mockResolvedValue(job({ state: 'needs_sign_in', stage: 'sign_in' }));
    mocks.readSetup.mockResolvedValue(job({ state: 'needs_sign_in', stage: 'sign_in' }));
    renderPane();
    fireEvent.click(screen.getByRole('button', { name: notSetUpCopy.setUp }));

    const dialog = await screen.findByTestId('setup-sign-in');
    const props = mocks.signIn[mocks.signIn.length - 1];
    expect(props.target).toEqual({ kind: 'setup', jobId: 'job-1' });
    expect(props.host).toBe('hpc.ucsf.edu');
    const steps = screen.getByRole('list', { name: setupCopy.stepsLabel });
    expect(within(steps).getByText(setupCopy.stages['signing-in']).closest('li')).toHaveAttribute(
      'aria-current',
      'step'
    );
    fireEvent.click(within(dialog).getByText('Simulate signed in'));
    await waitFor(() => expect(screen.queryByTestId('setup-sign-in')).toBeNull());
    // Signed in: the job is the daemon's to finish, so it is not stopped.
    expect(mocks.stopSetup).not.toHaveBeenCalled();
  });

  it('stops the job when the Sign in window is closed, and says so', async () => {
    mocks.startSetup.mockResolvedValue(job({ state: 'needs_sign_in', stage: 'sign_in' }));
    mocks.readSetup
      .mockResolvedValueOnce(job({ state: 'needs_sign_in', stage: 'sign_in' }))
      .mockResolvedValue(job({ state: 'cancelled', stage: 'sign_in' }));
    renderPane();
    fireEvent.click(screen.getByRole('button', { name: notSetUpCopy.setUp }));
    const dialog = await screen.findByTestId('setup-sign-in');
    fireEvent.click(within(dialog).getByText('Simulate close'));
    expect(mocks.stopSetup).toHaveBeenCalledWith('job-1');
    expect(
      await screen.findByTestId('crew-setup-problem', undefined, { timeout: 3000 })
    ).toHaveTextContent(setupCopy.signInEnded);
  });

  it.each([
    [
      'crew_install_checksum_tool_missing',
      setupCopy.error('crew_install_checksum_tool_missing', 'hpc.ucsf.edu'),
    ],
    ['crew_install_no_space', setupCopy.error('crew_install_no_space', 'hpc.ucsf.edu')],
    ['crew_ssh_key_refused', 'The server refused this computer’s SSH key.'],
  ])('says what %s means, in the pane, with Try again', async (code, sentence) => {
    mocks.startSetup.mockResolvedValue(
      job({
        state: 'failed',
        stage: 'check_installation',
        error: { code, message: 'The server refused this computer’s SSH key.', detail: null },
        output: 'BRCREW_PROBE begin 1',
      })
    );
    renderPane();
    fireEvent.click(screen.getByRole('button', { name: notSetUpCopy.setUp }));
    const problem = await screen.findByTestId('crew-setup-problem');
    expect(problem).toHaveTextContent(sentence as string);
    // The step it stopped at is marked, and what the server printed is kept, folded.
    const steps = screen.getByRole('list', { name: setupCopy.stepsLabel });
    expect(within(steps).getByText(setupCopy.stages.checking).closest('li')).toHaveAttribute(
      'data-state',
      'failed'
    );
    expect(screen.getByRole('button', { name: setupCopy.output })).toBeInTheDocument();
    mocks.startSetup.mockResolvedValue(job({ state: 'running' }));
    mocks.readSetup.mockReturnValue(new Promise(() => {}));
    fireEvent.click(within(problem).getByRole('button', { name: setupCopy.tryAgain }));
    await waitFor(() => expect(mocks.startSetup).toHaveBeenCalledTimes(2));
  });

  it('offers to replace an install that does not work with this Biorouter', async () => {
    mocks.startSetup.mockResolvedValue(
      job({
        state: 'failed',
        stage: 'check_installation',
        error: { code: 'crew_install_incompatible', message: 'Too old.', detail: null },
      })
    );
    renderPane();
    fireEvent.click(screen.getByRole('button', { name: notSetUpCopy.setUp }));
    const problem = await screen.findByTestId('crew-setup-problem');
    mocks.startSetup.mockResolvedValue(job());
    mocks.readSetup.mockReturnValue(new Promise(() => {}));
    fireEvent.click(within(problem).getByRole('button', { name: setupCopy.replace }));
    await waitFor(() =>
      expect(mocks.startSetup).toHaveBeenLastCalledWith({
        target: { kind: 'connection', connection_id: 'conn-1' },
        intent: 'member',
        workspace_name: 'lab',
        replace_existing: true,
      })
    );
  });

  it('starts by itself after Join, once, and never on its own otherwise', async () => {
    mocks.startSetup.mockResolvedValue(job());
    mocks.readSetup.mockReturnValue(new Promise(() => {}));
    requestMemberSetup('conn-1');
    const view = renderPane();
    await waitFor(() => expect(mocks.startSetup).toHaveBeenCalledOnce());
    view.unmount();
    // The request was taken: the pane shown again waits for the person.
    renderPane();
    await act(async () => {
      await Promise.resolve();
    });
    expect(mocks.startSetup).toHaveBeenCalledOnce();
  });

  it('lets a request from Join lapse', async () => {
    requestMemberSetup('conn-1', Date.now() - 11 * 60 * 1000);
    renderPane();
    await act(async () => {
      await Promise.resolve();
    });
    expect(mocks.startSetup).not.toHaveBeenCalled();
  });

  it('says a newer background service is needed when the daemon predates setup', async () => {
    mocks.readSetupArtifact.mockRejectedValue(new CrewHttpError('Crew request failed (404)', 404));
    renderPane();
    expect(await screen.findByText(setupCopy.outdated)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: notSetUpCopy.setUp })).toBeNull();
    // The older ways still work.
    expect(screen.getByRole('button', { name: notSetUpCopy.askHost })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: notSetUpCopy.tryAgain })).toBeEnabled();
  });

  it('still reads as a message without a known host', async () => {
    renderPane({ lastConnectFailure: { kind: 'handoff_failed', message: 'handoff' } });
    // The broker this build carries is read as the pane opens.
    await act(async () => {
      await Promise.resolve();
    });
    fireEvent.click(screen.getByRole('button', { name: notSetUpCopy.askHost }));
    expect(
      screen.getByText(
        'Crew isn’t set up for my account (@bob) on hpc.ucsf.edu yet. Could you or IT install biorouter-crew in ~/.local/bin for me?'
      )
    ).toBeInTheDocument();
  });
});
