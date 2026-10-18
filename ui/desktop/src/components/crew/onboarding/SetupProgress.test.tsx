import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError } from '../crewApi';
import { setupCopy } from './copy';
import { setupRunFrom, type SetupInput, type SetupRun } from './setup';
import { HOST_STORAGE_UNSAFE_CODE, SetupProgress, useSetupJob } from './SetupProgress';

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
// A setup job's Sign in window has its own suites.
vi.mock('../auth/SignInDialog', () => ({ SignInDialog: () => null }));

const HOST: SetupInput = {
  target: {
    kind: 'pending',
    preparation_id: 'prep-1',
    ssh_target: 'wanjun@leo.ucsf.edu',
  },
  intent: 'host',
  workspace_name: 'lab',
};

/** The daemon's sentence and detail for L3's leo account: `~/.local` is group-writable. */
const UNSAFE_MESSAGE =
  'Other people can change /pool1/home/wanjun/.local on leo.ucsf.edu, so Crew won’t keep a workspace under it. Run chmod go-w /pool1/home/wanjun/.local on the server, then try again.';
const UNSAFE_DETAIL = '/pool1/home/wanjun/.local';

function failedHost(error: SetupRun['error']): SetupRun {
  return {
    jobId: 'job-1',
    intent: 'host',
    state: 'failed',
    stage: 'probe',
    stages: [],
    server: null,
    artifact: null,
    outcome: null,
    error,
    output: '',
  };
}

function Harness({ input }: { input: SetupInput }) {
  const job = useSetupJob({ server: 'leo.ucsf.edu', onFinished: () => {} });
  return (
    <>
      <button type="button" onClick={() => void job.start(input)}>
        Start
      </button>
      <SetupProgress job={job} intent="host" server="leo.ucsf.edu" />
    </>
  );
}

beforeEach(() => {
  mocks.startSetup.mockReset();
  mocks.readSetup.mockReset().mockReturnValue(new Promise(() => {}));
  mocks.stopSetup.mockReset().mockResolvedValue(undefined);
});

describe('a host setup stopped by unsafe storage (L3)', () => {
  it('says which folders, offers Fix it for me, and starts again asking for the fix', async () => {
    mocks.startSetup.mockResolvedValueOnce(
      failedHost({ code: HOST_STORAGE_UNSAFE_CODE, message: UNSAFE_MESSAGE, detail: UNSAFE_DETAIL })
    );
    render(<Harness input={HOST} />);
    fireEvent.click(screen.getByRole('button', { name: 'Start' }));
    const problem = await screen.findByTestId('crew-setup-problem');
    // The daemon's own sentence, naming the folders and the command, and what the button does.
    expect(problem).toHaveTextContent(UNSAFE_MESSAGE);
    expect(problem).toHaveTextContent(setupCopy.fixStorageHint);
    const fix = within(problem).getByRole('button', { name: setupCopy.fixStorage });
    expect(fix).toHaveAccessibleDescription(setupCopy.fixStorageHint);
    // The folders it named, under Show details.
    fireEvent.click(screen.getByRole('button', { name: setupCopy.output }));
    expect(screen.getByLabelText(setupCopy.outputLabel)).toHaveTextContent(UNSAFE_DETAIL);

    mocks.startSetup.mockResolvedValueOnce({ ...failedHost(null), state: 'running' });
    fireEvent.click(fix);
    await waitFor(() =>
      expect(mocks.startSetup).toHaveBeenLastCalledWith({ ...HOST, fix_storage_permissions: true })
    );
    // Never on its own: the first start did not ask for it.
    expect(mocks.startSetup.mock.calls[0][0]).toEqual(HOST);
  });

  it('offers the same when the daemon refuses the start outright', async () => {
    mocks.startSetup.mockRejectedValueOnce(
      new CrewHttpError(UNSAFE_MESSAGE, 409, HOST_STORAGE_UNSAFE_CODE, UNSAFE_DETAIL)
    );
    render(<Harness input={HOST} />);
    fireEvent.click(screen.getByRole('button', { name: 'Start' }));
    const problem = await screen.findByTestId('crew-setup-problem');
    expect(problem).toHaveTextContent(UNSAFE_MESSAGE);
    fireEvent.click(screen.getByRole('button', { name: setupCopy.output }));
    expect(screen.getByLabelText(setupCopy.outputLabel)).toHaveTextContent(UNSAFE_DETAIL);
    mocks.startSetup.mockResolvedValueOnce({ ...failedHost(null), state: 'running' });
    fireEvent.click(within(problem).getByRole('button', { name: setupCopy.fixStorage }));
    await waitFor(() =>
      expect(mocks.startSetup).toHaveBeenLastCalledWith({ ...HOST, fix_storage_permissions: true })
    );
  });

  it('offers no fix for any other stop', async () => {
    mocks.startSetup.mockResolvedValueOnce(
      failedHost({ code: 'crew_install_no_space', message: 'No room.', detail: null })
    );
    render(<Harness input={HOST} />);
    fireEvent.click(screen.getByRole('button', { name: 'Start' }));
    const problem = await screen.findByTestId('crew-setup-problem');
    expect(within(problem).queryByRole('button', { name: setupCopy.fixStorage })).toBeNull();
    expect(problem).not.toHaveTextContent(setupCopy.fixStorageHint);
  });
});

describe('a start that failed on the server', () => {
  it('shows what the server’s Crew said under Show details', async () => {
    const said = 'Error: unsafe_storage: writable non-sticky ancestor /pool1/home/wanjun/.local';
    mocks.startSetup.mockResolvedValueOnce({
      ...failedHost({
        code: 'crew_setup_start_failed',
        message: 'Crew on leo.ucsf.edu didn’t start the workspace.',
        detail: said,
      }),
      stage: 'start',
    });
    render(<Harness input={HOST} />);
    fireEvent.click(screen.getByRole('button', { name: 'Start' }));
    await screen.findByTestId('crew-setup-problem');
    // Folded until asked for, and then the broker's own words, as the daemon bounded them.
    expect(screen.queryByLabelText(setupCopy.outputLabel)).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: setupCopy.output }));
    expect(screen.getByLabelText(setupCopy.outputLabel)).toHaveTextContent(said);
  });
});

describe('an install that replaced a program', () => {
  /** A finished job as the daemon answers it, with `outcome` laid over a fresh install's. */
  const finished = (outcome: Record<string, unknown>) =>
    setupRunFrom({
      job_id: 'job-1',
      intent: 'host',
      state: 'finished',
      stage: 'done',
      stages: [
        { name: 'probe', state: 'done', detail: null },
        {
          name: 'install',
          state: 'done',
          detail: 'Replaced Crew 1.93.0 with this app’s build of it.',
        },
      ],
      outcome: { installed_version: '1.93.0', reused: false, ...outcome },
    });
  const sameVersion = {
    previous_kept: true,
    upgraded_from: {
      version: '1.93.0',
      sha256: 'fe'.repeat(32),
      installed_version: '1.93.0',
      installed_sha256: 'f4'.repeat(32),
    },
  };
  /** What the "Installing Crew in your account" step says once the job finished. */
  const installStep = async (outcome: Record<string, unknown>) => {
    mocks.startSetup.mockResolvedValueOnce(finished(outcome));
    const view = render(<Harness input={HOST} />);
    fireEvent.click(screen.getByRole('button', { name: 'Start' }));
    const step = (await screen.findByText(setupCopy.stages.installing)).closest('li')!;
    await waitFor(() => expect(step).toHaveAttribute('data-state', 'done'));
    const said = step.textContent;
    view.unmount();
    return said;
  };

  it('says it replaced a same-version copy, or updated an older one, as the command line does', async () => {
    const replaced = await installStep(sameVersion);
    expect(replaced).toContain(setupCopy.replaced('1.93.0'));
    expect(replaced).not.toContain(setupCopy.installed('1.93.0'));
    expect(
      await installStep({
        ...sameVersion,
        upgraded_from: { ...sameVersion.upgraded_from, version: '1.92.1' },
      })
    ).toContain('Updated Crew from 1.92.1 to 1.93.0.');
    expect(await installStep({ previous_kept: true, upgraded_from: null })).toContain(
      setupCopy.replacedProgram('1.93.0')
    );
  });

  it('still says installed for a fresh install, and already installed for a reuse', async () => {
    expect(await installStep({ previous_kept: false, upgraded_from: null })).toContain(
      'Installed Crew 1.93.0.'
    );
    expect(await installStep({ reused: true })).toContain('Crew 1.93.0 was already installed.');
  });
});
