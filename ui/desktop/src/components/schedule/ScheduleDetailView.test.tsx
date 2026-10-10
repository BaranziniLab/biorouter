import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { MemoryRouter } from 'react-router-dom';
import type { ReactNode } from 'react';
import ScheduleDetailView from './ScheduleDetailView';
import { scheduleCopy } from './copy';

const mocks = vi.hoisted(() => ({
  getScheduleSessions: vi.fn(),
  listSchedules: vi.fn(),
  runScheduleNow: vi.fn(),
  pauseSchedule: vi.fn(),
  deleteSchedule: vi.fn(),
  inspectRunningJob: vi.fn(),
  getSession: vi.fn(),
  toastError: vi.fn(),
  toastSuccess: vi.fn(),
}));

vi.mock('../../schedule', () => ({
  ...mocks,
  unpauseSchedule: vi.fn(),
  updateSchedule: vi.fn(),
  killRunningJob: vi.fn(),
}));
vi.mock('../../toasts', () => mocks);
vi.mock('../../api', () => ({ getSession: mocks.getSession }));
vi.mock('../../utils/userAction', () => ({ userActionHeaders: async () => ({ proof: 'x' }) }));
// The session list is where a run's privacy tier comes from (its own endpoint
// carries none). `null` is "not fetched yet", as the real cache starts.
let cachedSessionList: Array<{ id: string; privacy_tier?: string }> | null = null;
vi.mock('../../utils/sessionListCache', () => ({
  getCachedSessionList: () => cachedSessionList,
  subscribeSessionList: () => () => {},
  preloadSessionList: () => {},
}));
vi.mock('../sessions/SessionHistoryView', () => ({
  default: ({ session }: { session: { id: string } }) => (
    <div data-testid="opened-run">{session.id}</div>
  ),
}));
vi.mock('./ScheduleModal', () => ({ ScheduleModal: () => null }));
vi.mock('../ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: ReactNode }) => <div>{children}</div>,
}));

const schedule = {
  id: 'daily-meditation',
  source: '/tmp/daily-meditation.yaml',
  cron: '0 0 3 * * *',
  last_run: null,
  currently_running: false,
  paused: false,
};

function renderDetails(onNavigateBack = vi.fn()) {
  return render(
    <MemoryRouter>
      <ScheduleDetailView scheduleId={schedule.id} onNavigateBack={onNavigateBack} />
    </MemoryRouter>
  );
}

/** Radix opens a dropdown on pointerdown, not click. */
function openBandMenu() {
  fireEvent.pointerDown(screen.getByRole('button', { name: 'More actions' }), {
    button: 0,
    ctrlKey: false,
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  cachedSessionList = null;
  mocks.listSchedules.mockResolvedValue([schedule]);
  mocks.getScheduleSessions.mockResolvedValue([]);
  mocks.pauseSchedule.mockResolvedValue(undefined);
});

describe('manual schedule run feedback', () => {
  it('announces a pending run without claiming success or submitting twice', async () => {
    let finish!: (id: string) => void;
    mocks.runScheduleNow.mockReturnValue(new Promise<string>((resolve) => (finish = resolve)));
    renderDetails();
    const run = await screen.findByRole('button', { name: 'Run now' });
    fireEvent.click(run);
    fireEvent.click(run);

    expect(mocks.runScheduleNow).toHaveBeenCalledTimes(1);
    expect(run).toBeDisabled();
    // Spec 3.10: the pending run is a status word in the band beside Run now,
    // not a note under an Actions section.
    const status = screen.getByRole('status');
    expect(status).toHaveTextContent(scheduleCopy.detail.running);
    expect(screen.getByTestId('page-header')).toContainElement(status);
    expect(mocks.toastSuccess).not.toHaveBeenCalled();

    await act(async () => finish('finished-session'));
    await waitFor(() => expect(run).not.toBeDisabled());
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
    expect(mocks.getScheduleSessions).toHaveBeenCalledTimes(2);
  });

  it('clears pending feedback and allows a retry after a failed run', async () => {
    let fail!: (reason: Error) => void;
    mocks.runScheduleNow.mockReturnValue(new Promise<string>((_, reject) => (fail = reject)));
    renderDetails();
    const run = await screen.findByRole('button', { name: 'Run now' });
    fireEvent.click(run);
    expect(screen.getByRole('status')).toBeInTheDocument();

    await act(async () => fail(new Error('Provider unavailable')));
    await waitFor(() => expect(run).not.toBeDisabled());
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
    expect(mocks.toastError).toHaveBeenCalledWith(
      expect.objectContaining({ msg: 'Provider unavailable' })
    );
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
  });

  it('does not describe another pending action as a schedule run', async () => {
    let finish!: () => void;
    mocks.pauseSchedule.mockReturnValue(new Promise<void>((resolve) => (finish = resolve)));
    renderDetails();
    fireEvent.click(await screen.findByRole('button', { name: 'Pause' }));
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
    expect(mocks.runScheduleNow).not.toHaveBeenCalled();
    await act(async () => finish());
  });
});

/**
 * A run's glyph must not call a private run Public.
 *
 * `GET /schedule/{id}/sessions` lists a schedule's private runs to the desktop
 * (it sends the proof) but its rows carry no `privacy_tier`, and until
 * 2026-09-14 the row passed the glyph no tier at all — which the glyph drew as
 * `data-privacy="public"` on every run. The tier now comes from the session
 * list; a run that list does not carry is drawn as not yet known.
 */
describe('a run’s privacy glyph', () => {
  const glyphFor = async (runName: string) => {
    const row = (await screen.findByRole('button', { name: `Open run ${runName}` })) as HTMLElement;
    const glyph = row.querySelector('[data-testid="chat-kind-icon"]');
    expect(glyph).not.toBeNull();
    return glyph as HTMLElement;
  };

  it('marks a private run private and a public run public, from the session list', async () => {
    cachedSessionList = [
      { id: 'run-private', privacy_tier: 'private' },
      { id: 'run-public', privacy_tier: 'public' },
    ];
    mocks.getScheduleSessions.mockResolvedValue([
      { id: 'run-private', name: 'Nightly cohort pull', messageCount: 4 },
      { id: 'run-public', name: 'Nightly news digest', messageCount: 2 },
    ]);
    renderDetails();

    const privateGlyph = await glyphFor('Nightly cohort pull');
    expect(privateGlyph).toHaveAttribute('data-chat-kind', 'scheduled');
    expect(privateGlyph).toHaveAttribute('data-privacy', 'private');
    expect(privateGlyph.getAttribute('aria-label')).toBe('Scheduled run, private');
    expect(await glyphFor('Nightly news digest')).toHaveAttribute('data-privacy', 'public');
  });

  it('draws a run the session list does not carry as not yet known, never public', async () => {
    cachedSessionList = [];
    mocks.getScheduleSessions.mockResolvedValue([
      { id: 'run-unlisted', name: 'Run with no messages yet', messageCount: 0 },
    ]);
    renderDetails();

    const glyph = await glyphFor('Run with no messages yet');
    expect(glyph).not.toHaveAttribute('data-privacy', 'public');
    expect(glyph).toHaveAttribute('data-privacy', 'unknown');
  });
});

describe('names, not ids (principle 10)', () => {
  const monoAncestor = (element: HTMLElement | null): boolean => {
    for (let node = element; node; node = node.parentElement) {
      if (node.classList?.contains('font-mono')) return true;
    }
    return false;
  };

  /**
   * An unnamed run has no name to show, so its id is its name, printed ONCE and
   * in the data face. The working directory is a path: it is not a row line.
   */
  it('names an unnamed run by its id once, in mono, and leaves the path off the row', async () => {
    mocks.getScheduleSessions.mockResolvedValue([
      { id: 'sess-20260902-7f3', name: null, workingDir: '/tmp/work', messageCount: 2 },
    ]);
    renderDetails();

    const rendered = await screen.findAllByText('sess-20260902-7f3');
    expect(rendered).toHaveLength(1);
    expect(monoAncestor(rendered[0] as HTMLElement)).toBe(true);
    expect(screen.queryByText('/tmp/work')).toBeNull();
  });

  /** Run rows are sans tabular: date, message count and tokens. */
  it('says a run’s facts in the sans face with tabular figures', async () => {
    mocks.getScheduleSessions.mockResolvedValue([
      {
        id: 'run-1',
        name: 'Nightly cohort pull',
        createdAt: '2026-10-07T21:00:00Z',
        messageCount: 4,
      },
    ]);
    renderDetails();

    const meta = (await screen.findByText('4 messages')).parentElement as HTMLElement;
    expect(meta).toHaveClass('tabular-nums');
    expect(meta).toHaveTextContent(/ · 4 messages$/);
    expect(monoAncestor(meta)).toBe(false);
  });

  /**
   * The running chat used to be its session id in mono on a definition row.
   * Now it is a button that opens that chat (spec 3.10, "an Open chat button").
   */
  it('opens the running chat instead of printing its id', async () => {
    mocks.listSchedules.mockResolvedValue([
      { ...schedule, currently_running: true, current_session_id: 'sess-running-1' },
    ]);
    mocks.getSession.mockResolvedValue({ data: { id: 'sess-running-1' } });
    renderDetails();

    fireEvent.click(await screen.findByRole('button', { name: scheduleCopy.detail.openChat }));
    expect(await screen.findByTestId('opened-run')).toHaveTextContent('sess-running-1');
    expect(mocks.getSession).toHaveBeenCalledWith(
      expect.objectContaining({ path: { session_id: 'sess-running-1' }, headers: { proof: 'x' } })
    );
  });
});

/**
 * The flat rebuild (astryx §4.5). Each assertion below names a shape the view
 * used to have and no longer may.
 */
describe('the detail view is rows and hairlines, not boxes', () => {
  const SOURCE = readFileSync(join(__dirname, 'ScheduleDetailView.tsx'), 'utf8');
  /**
   * Comments stripped, for the reason `settingsVocabulary.test.ts` records for
   * its own banned-class rule: prose explaining why a construction is gone is
   * not a use of it. The docblock below names `<Card>` and `h-screen` on
   * purpose — that history is the point of the docblock — and a raw substring
   * search over the file would read those two words as the defect.
   */
  const CODE = SOURCE.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');

  /**
   * ⚠ Asserted at the SOURCE, and it has to be. jsdom has no layout engine and
   * never runs Tailwind, so a card and a bare div render identically here —
   * the box is only real in a browser against the built stylesheet.
   */
  it('mounts no Card, and sizes itself from its parent rather than the viewport', () => {
    expect(CODE).not.toMatch(/from '\.\.\/ui\/card'/);
    expect(CODE).not.toContain('<Card');
    // `h-screen` is the anti-pattern MainPanelLayout's own comment warns about:
    // it forces viewport height regardless of the parent, which breaks the view
    // inside an embedded pane.
    expect(CODE).not.toContain('h-screen');
    expect(CODE).toMatch(/<MainPanelLayout\b[^>]*>/);
  });

  it('renders the facts by name as definition rows, one hairline list', async () => {
    renderDetails();

    for (const label of ['Runs', 'Workflow', 'Last run']) {
      expect(await screen.findByText(label)).toBeInTheDocument();
    }
    // Names, never ids (principle 10): no raw cron row and no Id row; the
    // workflow is its file name, the full path one hover away.
    expect(screen.queryByText('Cron')).toBeNull();
    expect(screen.queryByText('Id')).toBeNull();
    const workflowRow = screen.getByText('Workflow').closest('.biorouter-settings-row');
    expect(workflowRow).not.toBeNull();
    expect(workflowRow).toHaveTextContent('daily-meditation.yaml');
    expect(workflowRow).not.toHaveTextContent('/tmp/');
  });

  it('puts back, the name and the status in the band, with Run now as its one accent', async () => {
    const onBack = vi.fn();
    renderDetails(onBack);

    const band = screen.getByTestId('page-header');
    expect(
      await screen.findByRole('heading', { level: 1, name: 'Daily Meditation' })
    ).toBeInTheDocument();
    expect(band).toHaveTextContent('Scheduled');
    fireEvent.click(screen.getByRole('button', { name: scheduleCopy.detail.back }));
    expect(onBack).toHaveBeenCalledTimes(1);
    // The old Actions section and its notes are gone.
    expect(screen.queryByText('Actions')).toBeNull();
  });

  it('says a paused schedule is paused as text, never as a filled pill', async () => {
    mocks.listSchedules.mockResolvedValue([{ ...schedule, paused: true }]);
    renderDetails();

    const paused = await screen.findByText('Paused');
    for (let node: HTMLElement | null = paused; node; node = node.parentElement) {
      expect(node.className).not.toMatch(/bg-background-\w+\/\d/);
      if (node.classList.contains('biorouter-page-header')) break;
    }
    // The hue is carried by a dot beside the word, the §3.4 idiom.
    expect(paused.querySelector('[data-slot="status-dot"]')).toHaveAttribute('data-tone', 'idle');
  });

  /**
   * Spec 3.10: the three notes are deleted. The status says Paused; Resume's
   * tooltip says what it does; nothing explains the state in prose.
   */
  it('explains the paused state through Resume, not a note', async () => {
    mocks.listSchedules.mockResolvedValue([{ ...schedule, paused: true }]);
    renderDetails();

    expect(await screen.findByRole('button', { name: scheduleCopy.resume })).toBeInTheDocument();
    expect(screen.queryByText(/will not run automatically/)).toBeNull();
  });

  it('deletes from the band menu after confirming, then goes back', async () => {
    mocks.deleteSchedule.mockResolvedValue(undefined);
    const onBack = vi.fn();
    renderDetails(onBack);

    await screen.findByRole('heading', { level: 1, name: 'Daily Meditation' });
    openBandMenu();
    fireEvent.click(await screen.findByRole('menuitem', { name: scheduleCopy.delete }));
    expect(screen.getByRole('dialog')).toHaveTextContent(/workflow it runs is left in place/);
    fireEvent.click(screen.getByRole('button', { name: scheduleCopy.deleteConfirm }));

    await waitFor(() => expect(mocks.deleteSchedule).toHaveBeenCalledWith('daily-meditation'));
    await waitFor(() => expect(onBack).toHaveBeenCalledTimes(1));
  });

  /**
   * ⚠ The one state a sandbox cannot be put into: `currently_running` is
   * reconciled against a live process, so a daemon started against a JSON that
   * claims a run is in flight clears the flag before the interface sees it
   * (measured). The branch is exercised here or nowhere.
   */
  it('replaces the idle actions with the run’s own while a run is in flight', async () => {
    mocks.listSchedules.mockResolvedValue([{ ...schedule, currently_running: true }]);
    renderDetails();

    expect(await screen.findByRole('button', { name: scheduleCopy.stopRun })).toBeInTheDocument();
    const runNow = screen.getByRole('button', { name: scheduleCopy.runNow });
    expect(runNow).toBeDisabled();
    // A disabled control still says why, without a hover.
    expect(runNow.parentElement).toHaveAccessibleDescription(scheduleCopy.detail.availableAfterRun);
    // Pause is replaced by Stop, not left greyed out.
    expect(screen.queryByRole('button', { name: scheduleCopy.pause })).not.toBeInTheDocument();

    openBandMenu();
    expect(await screen.findByRole('menuitem', { name: scheduleCopy.inspectRun })).toBeEnabled();
    expect(screen.getByRole('menuitem', { name: scheduleCopy.edit })).toHaveAttribute(
      'aria-disabled',
      'true'
    );

    // No prose explains the state: the status says Running.
    expect(screen.queryByText(/This schedule is running/)).not.toBeInTheDocument();
  });
});
