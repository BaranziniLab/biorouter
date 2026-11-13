import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { resetSetupStartedHereForTests } from '../onboarding/setup';
import { CREW_CHANGED_DEBOUNCE_MS, dispatchCrewChanged } from '../state/useCrewChanges';
import { ChatSetupJobNotes, SETUP_JOBS_POLL_MS, setupFailedCopy } from './ChatSetupJobs';
import { setupJobsCopy } from './copy';

/**
 * A setup a chat started follows Crew's change feed (WI-15): the note is read again as soon as the
 * job moves, not at the next poll, and a job it showed that then failed leaves one line saying so.
 */

const mocks = vi.hoisted(() => ({ crewHttp: vi.fn(), jobs: [] as unknown[] }));

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: mocks.crewHttp };
});
vi.mock('../auth/SignInDialog', () => ({ SignInDialog: () => null }));

const job = (overrides: Record<string, unknown> = {}) => ({
  job_id: 'job-chat',
  intent: 'install',
  state: 'running',
  stage: 'install',
  stages: [{ name: 'install', state: 'running' }],
  server: { login: 'wanjun@leo.ucsf.edu', user: 'wanjun' },
  outcome: null,
  error: null,
  ...overrides,
});

const failedJob = (message: string) =>
  job({
    state: 'failed',
    stage: 'install',
    error: { code: 'crew_install_failed', message, detail: null },
  });

const setupChange = (kind: string, revision = 1) => ({
  revision,
  truncated: false,
  changes: [
    {
      revision,
      kind,
      connection_id: null,
      channel_id: null,
      session_id: null,
      run_id: null,
      job_id: 'job-chat',
    },
  ],
});

const setupReads = () => mocks.crewHttp.mock.calls.filter(([path]) => path === '/setup').length;
const quietFor = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

beforeEach(() => {
  mocks.crewHttp.mockReset();
  mocks.jobs = [job()];
  resetSetupStartedHereForTests();
  mocks.crewHttp.mockImplementation(async (path: string) => (path === '/setup' ? mocks.jobs : {}));
});

describe('ChatSetupJobNotes and the change feed', () => {
  it('reads the jobs again when one moves, long before the next poll', async () => {
    expect(SETUP_JOBS_POLL_MS).toBeGreaterThan(2000);
    render(<ChatSetupJobNotes />);
    const note = await screen.findByTestId('crew-outside-setup');
    expect(note).toHaveTextContent(setupJobsCopy.running('wanjun@leo.ucsf.edu'));
    expect(setupReads()).toBe(1);

    mocks.jobs = [job({ state: 'needs_sign_in', stage: 'sign_in' })];
    dispatchCrewChanged(setupChange('setup_needs_sign_in'));
    await waitFor(() =>
      expect(screen.getByTestId('crew-outside-setup')).toHaveTextContent(
        setupJobsCopy.needsSignIn('wanjun@leo.ucsf.edu')
      )
    );
    expect(setupReads()).toBe(2);
  });

  it('keeps one line about a job it showed that then failed, until dismissed', async () => {
    render(<ChatSetupJobNotes />);
    await screen.findByTestId('crew-outside-setup');

    mocks.jobs = [failedJob('The server ran out of disk space.')];
    dispatchCrewChanged(setupChange('setup_failed'));
    const note = await screen.findByTestId('crew-outside-setup-failed');
    expect(note).toHaveTextContent(
      `${setupFailedCopy.failed('wanjun@leo.ucsf.edu')} The server ran out of disk space.`
    );
    expect(screen.queryByTestId('crew-outside-setup')).toBeNull();

    // Read again: the note stays, once.
    dispatchCrewChanged(setupChange('setup_failed', 2));
    await quietFor(CREW_CHANGED_DEBOUNCE_MS * 2);
    expect(screen.getAllByTestId('crew-outside-setup-failed')).toHaveLength(1);

    fireEvent.click(
      within(note).getByRole('button', {
        name: setupFailedCopy.dismissLabel('wanjun@leo.ucsf.edu'),
      })
    );
    expect(screen.queryByTestId('crew-outside-setup-failed')).toBeNull();
  });

  it('says nothing about a failed job it never showed', async () => {
    mocks.jobs = [];
    render(<ChatSetupJobNotes />);
    await waitFor(() => expect(setupReads()).toBe(1));
    mocks.jobs = [failedJob('Refused.')];
    dispatchCrewChanged(setupChange('setup_failed'));
    await waitFor(() => expect(setupReads()).toBe(2));
    expect(screen.queryByTestId('crew-outside-setup-failed')).toBeNull();
  });

  it('reads nothing for a change that is not about a setup', async () => {
    render(<ChatSetupJobNotes />);
    await screen.findByTestId('crew-outside-setup');
    dispatchCrewChanged(setupChange('send'));
    await quietFor(CREW_CHANGED_DEBOUNCE_MS * 2);
    expect(setupReads()).toBe(1);
  });

  it('shows the server text the daemon sent as plain text', async () => {
    render(<ChatSetupJobNotes />);
    await screen.findByTestId('crew-outside-setup');
    mocks.jobs = [failedJob('Stopped\u202E by the server.')];
    dispatchCrewChanged(setupChange('setup_failed'));
    const note = await screen.findByTestId('crew-outside-setup-failed');
    expect(note.textContent).not.toContain('\u202E');
  });
});
