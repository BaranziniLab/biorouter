import { fireEvent, render, screen, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { openCopy, setupCopy } from './copy';
import type { SetupInput, SetupRun, SetupWorkspace } from './setup';
import { SetupProgress, useSetupJob } from './SetupProgress';

/**
 * Setup progress for account ownership (AO-8, AO-9): the Looking for workspaces and Updating
 * running workspaces rows, the workspaces a host's or member's job found (the Host and Join
 * dialogs), the note when they could not be listed, and the caller's own answer to a refusal
 * (`crew_host_name_in_use`'s Open).
 */

const mocks = vi.hoisted(() => ({
  startSetup: vi.fn(),
  readSetup: vi.fn(),
  stopSetup: vi.fn(),
}));

vi.mock('./setup', async () => {
  const actual = await vi.importActual<typeof import('./setup')>('./setup');
  return {
    ...actual,
    startSetup: mocks.startSetup,
    readSetup: mocks.readSetup,
    stopSetup: mocks.stopSetup,
  };
});
vi.mock('../auth/SignInDialog', () => ({ SignInDialog: () => null }));

const HOST: SetupInput = {
  target: { kind: 'pending', preparation_id: 'prep-1', ssh_target: 'wanjun@leo.ucsf.edu' },
  intent: 'host',
  workspace_name: 'baranzini-lab',
};

const FOUND: SetupWorkspace = {
  workspaceId: '11111111-2222-4333-8444-555555555555',
  name: 'chen-lab',
  role: 'member',
  state: 'running',
  hostUsername: 'chen',
  fingerprint: null,
  mode: 'private',
  institutionId: 'ucsf',
  version: '1.95.0',
  binaryCurrent: true,
  othersConnected: null,
  invitation: 'brcrew1:abc',
  savedConnectionId: 'conn-1',
  thisComputer: 'connected',
  error: null,
};

function run(overrides: Partial<SetupRun>): SetupRun {
  return {
    jobId: 'job-1',
    intent: 'host',
    state: 'running',
    stage: 'workspaces',
    stages: [
      { name: 'check', state: 'done', detail: null },
      { name: 'install', state: 'done', detail: null },
      { name: 'workspaces', state: 'running', detail: null },
      { name: 'restart', state: 'pending', detail: null },
      { name: 'start', state: 'pending', detail: null },
    ],
    server: null,
    artifact: null,
    outcome: null,
    error: null,
    output: '',
    ...overrides,
  };
}

function Harness({ action }: { action?: ReactNode }) {
  const job = useSetupJob({ server: 'leo.ucsf.edu', onFinished: () => {} });
  return (
    <>
      <button type="button" onClick={() => void job.start(HOST)}>
        Start
      </button>
      <SetupProgress job={job} intent="host" server="leo.ucsf.edu" problemAction={action} />
    </>
  );
}

beforeEach(() => {
  mocks.startSetup.mockReset();
  mocks.readSetup.mockReset().mockReturnValue(new Promise(() => {}));
  mocks.stopSetup.mockReset().mockResolvedValue(undefined);
});

describe('SetupProgress for account ownership', () => {
  it('adds the workspaces and restart rows a host job reports, and lists what it found', async () => {
    mocks.startSetup.mockResolvedValueOnce(run({ workspaces: [FOUND] }));
    render(<Harness />);
    fireEvent.click(screen.getByRole('button', { name: 'Start' }));
    const steps = await screen.findByRole('list', { name: setupCopy.stepsLabel });
    const names = within(steps)
      .getAllByRole('listitem')
      .map((item) => item.textContent);
    expect(names).toEqual([
      setupCopy.stages.checking,
      setupCopy.stages.installing,
      setupCopy.stages.finding,
      setupCopy.stages.updating,
      setupCopy.stages.starting,
      setupCopy.stages.ready,
    ]);
    expect(within(steps).getByText(setupCopy.stages.finding).closest('li')).toHaveAttribute(
      'aria-current',
      'step'
    );
    const found = screen.getByRole('region', { name: setupCopy.foundLabel('leo.ucsf.edu') });
    expect(within(found).getByText(openCopy.hostedBy('chen-lab', 'chen'))).toBeVisible();
    expect(within(found).getByText(/Open on this computer/)).toBeVisible();
  });

  it('says why the workspaces could not be listed, and goes on', async () => {
    mocks.startSetup.mockResolvedValueOnce(run({ workspaces: [], workspacesNote: 'timed out' }));
    render(<Harness />);
    fireEvent.click(screen.getByRole('button', { name: 'Start' }));
    expect(await screen.findByTestId('crew-setup-workspaces-note')).toHaveTextContent(
      setupCopy.workspacesNote('timed out')
    );
  });

  it('offers the caller’s own way on beside Try again', async () => {
    mocks.startSetup.mockResolvedValueOnce(
      run({
        state: 'failed',
        stage: 'check',
        error: {
          code: 'crew_host_name_in_use',
          message:
            'You already host baranzini-lab on leo.ucsf.edu. Open it on this computer instead.',
          detail: null,
        },
      })
    );
    const open = vi.fn();
    render(
      <Harness
        action={
          <button type="button" onClick={open}>
            {setupCopy.openExisting}
          </button>
        }
      />
    );
    fireEvent.click(screen.getByRole('button', { name: 'Start' }));
    const problem = await screen.findByTestId('crew-setup-problem');
    expect(problem).toHaveTextContent(
      'You already host baranzini-lab on leo.ucsf.edu. Open it on this computer instead.'
    );
    fireEvent.click(within(problem).getByRole('button', { name: setupCopy.openExisting }));
    expect(open).toHaveBeenCalled();
    expect(within(problem).getByRole('button', { name: setupCopy.tryAgain })).toBeVisible();
  });

  it('shows no workspace rows from a daemon before account ownership', async () => {
    mocks.startSetup.mockResolvedValueOnce(
      run({
        stage: 'probe',
        stages: [{ name: 'probe', state: 'running', detail: null }],
      })
    );
    render(<Harness />);
    fireEvent.click(screen.getByRole('button', { name: 'Start' }));
    const steps = await screen.findByRole('list', { name: setupCopy.stepsLabel });
    expect(within(steps).queryByText(setupCopy.stages.finding)).toBeNull();
    expect(within(steps).queryByText(setupCopy.stages.updating)).toBeNull();
    expect(screen.queryByTestId('crew-setup-workspaces')).toBeNull();
  });
});
