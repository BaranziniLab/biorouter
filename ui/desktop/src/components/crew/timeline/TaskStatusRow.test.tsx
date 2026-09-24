import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { identityCopy } from '../identity';
import { CANCELLABLE_RUN_STATUSES, runStatusPresentation } from '../state/crewStatus';
import { timelineCopy } from './copy';
import { Timeline } from './Timeline';
import {
  ID,
  MACHINE_STRING,
  installResizeObserverStub,
  makeController,
  message,
  renderWithController,
  run,
} from './timelineTestUtils';

/**
 * The viewer's task as a line in the log (ui-redesign-spec, "Task status row"
 * and "Run status words"): a word for every status — including one this
 * renderer has never seen — Stop only while the task can be stopped, Open and
 * Review into the task's OWN conversation, and no run ID on screen.
 */

installResizeObserverStub();

const STATUSES = [
  'starting',
  'running',
  'waiting_for_approval',
  'cancellation_pending',
  'cancellation_unconfirmed',
  'interrupted',
  'outcome_not_durable',
  'completed',
  'failed',
  'cancelled',
  'paused_by_host',
];

const WORDS: Record<string, string> = {
  starting: 'Starting…',
  running: 'Working…',
  waiting_for_approval: 'Waiting for your approval',
  cancellation_pending: 'Stopping…',
  cancellation_unconfirmed: 'Stop not confirmed',
  interrupted: 'Interrupted',
  outcome_not_durable: 'Outcome unknown',
  completed: 'Done',
  failed: 'Couldn’t finish',
  cancelled: 'Stopped',
  paused_by_host: 'Paused by host',
};

function renderTask(status: string, extra: Parameters<typeof run>[0] = {}) {
  const controller = makeController({
    messages: [
      message({
        actor_id: ID.alice,
        run_id: ID.run,
        body: 'Task: Plot counts by sample\nand post the figure.',
      }),
    ],
    runs: [run({ status, ...extra })],
  });
  const view = renderWithController(<Timeline />, controller);
  // The task row, not the agent's message row beside it (also a group, named "Your agent 10:02 AM…").
  const row = screen.getByRole('group', { name: new RegExp(`^${identityCopy.yourAgent} · `) });
  return { controller, row, ...view };
}

describe('task status rows', () => {
  it.each(STATUSES)('reads %s as words, with Stop only while it can be stopped', (status) => {
    const { row } = renderTask(status);
    expect(row).toHaveAccessibleName(`${identityCopy.yourAgent} · ${WORDS[status]}`);
    expect(within(row).getByText('Plot counts by sample')).toBeInTheDocument();

    const stoppable = CANCELLABLE_RUN_STATUSES.includes(status);
    const stop = within(row).queryByRole('button', { name: timelineCopy.taskStopLabel });
    const again = within(row).queryByRole('button', { name: timelineCopy.taskStopAgain });
    if (status === 'cancellation_unconfirmed') {
      // The retry IS the stop here; two controls for one action would be noise.
      expect(again).toBeInTheDocument();
      expect(stop).toBeNull();
    } else {
      expect(Boolean(stop)).toBe(stoppable);
      expect(again).toBeNull();
    }
    expect(runStatusPresentation(status).stoppable).toBe(stoppable);

    const action = runStatusPresentation(status).action;
    expect(Boolean(within(row).queryByRole('button', { name: timelineCopy.taskOpenLabel }))).toBe(
      action === 'open'
    );
    expect(Boolean(within(row).queryByRole('button', { name: timelineCopy.taskReviewLabel }))).toBe(
      action === 'review'
    );

    expect(row.outerHTML).not.toMatch(MACHINE_STRING);
  });

  it('marks a waiting approval with a warning badge and Review, never approving here', async () => {
    const { row, controller } = renderTask('waiting_for_approval');
    expect(within(row).getByText('Waiting for your approval')).toHaveClass('text-text-warning');
    await userEvent.click(within(row).getByRole('button', { name: timelineCopy.taskReviewLabel }));
    expect(screen.getByTestId('location')).toHaveTextContent(
      `/pair?resumeSessionId=${encodeURIComponent(ID.session)}`
    );
    expect(controller.cancelRun).not.toHaveBeenCalled();
  });

  it('opens the task’s own conversation, not another chat', async () => {
    const { row } = renderTask('completed', { session_id: 'task-session-42' });
    await userEvent.click(within(row).getByRole('button', { name: timelineCopy.taskOpenLabel }));
    expect(screen.getByTestId('location')).toHaveTextContent(
      '/pair?resumeSessionId=task-session-42'
    );
  });

  it('asks before stopping: Stop opens the stop-task confirmation', async () => {
    const { row, controller } = renderTask('running');
    await userEvent.click(within(row).getByRole('button', { name: timelineCopy.taskStopLabel }));
    expect(controller.openDialog).toHaveBeenCalledWith({
      kind: 'confirm',
      confirm: { action: 'stop-task', runId: ID.run },
    });
    expect(controller.cancelRun).not.toHaveBeenCalled();
  });

  it('retries an unconfirmed stop directly, and disables it while a stop is in flight', async () => {
    const { row, controller, rerenderWith } = renderTask('cancellation_unconfirmed');
    await userEvent.click(within(row).getByRole('button', { name: timelineCopy.taskStopAgain }));
    expect(controller.cancelRun).toHaveBeenCalledWith(ID.run);
    rerenderWith({ ...controller, isPending: vi.fn((key: string) => key === 'run.cancel') });
    expect(screen.getByRole('button', { name: timelineCopy.taskStopAgain })).toBeDisabled();
  });

  it('keeps chat history, the error and then the task ID behind ⋯, in that order (Q2-62)', async () => {
    const user = userEvent.setup();
    const writeText = vi.spyOn(navigator.clipboard, 'writeText');
    const { row } = renderTask('failed', { error: 'Model refused the request.' });
    expect(within(row).getByText('Couldn’t finish')).toHaveClass('text-text-danger');
    expect(row).not.toHaveTextContent('Model refused');

    await user.click(within(row).getByRole('button', { name: timelineCopy.taskMoreActions }));
    const menu = await screen.findByRole('menu');
    // The person's actions first, a separator, then the machine string.
    expect(
      Array.from(menu.querySelectorAll('[role="menuitem"], [role="separator"]')).map((node) =>
        node.getAttribute('role') === 'separator' ? '—' : node.textContent
      )
    ).toEqual(['Show in chat history', timelineCopy.taskCopyError, '—', timelineCopy.taskCopyId]);

    // A copy answers in the menu, which then closes by itself.
    await user.click(screen.getByRole('menuitem', { name: timelineCopy.taskCopyId }));
    expect(writeText).toHaveBeenLastCalledWith(ID.run);
    expect(await screen.findByRole('menuitem', { name: timelineCopy.copied })).toHaveAttribute(
      'data-crew-copy-state',
      'copied'
    );
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull(), { timeout: 2000 });

    await user.click(within(row).getByRole('button', { name: timelineCopy.taskMoreActions }));
    await user.click(await screen.findByRole('menuitem', { name: timelineCopy.taskCopyError }));
    expect(writeText).toHaveBeenLastCalledWith('Model refused the request.');
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull(), { timeout: 2000 });

    await user.click(within(row).getByRole('button', { name: timelineCopy.taskMoreActions }));
    await user.click(await screen.findByRole('menuitem', { name: timelineCopy.taskOpenHistory }));
    expect(screen.getByTestId('location')).toHaveTextContent('/sessions');
  });

  it('offers Copy error only when there is an error', async () => {
    const { row } = renderTask('completed');
    await userEvent.click(within(row).getByRole('button', { name: timelineCopy.taskMoreActions }));
    expect(
      await screen.findByRole('menuitem', { name: timelineCopy.taskCopyId })
    ).toBeInTheDocument();
    expect(screen.queryByRole('menuitem', { name: timelineCopy.taskCopyError })).toBeNull();
  });

  it('pulses a running word gently and holds the others still', () => {
    const running = renderTask('running');
    expect(within(running.row).getByText('Working…')).toHaveClass('crew-task-running');
    running.unmount();
    const done = renderTask('completed');
    expect(within(done.row).getByText('Done')).not.toHaveClass('crew-task-running');
  });

  it('sits under the task’s result, not between the task and its result (Q2-62)', () => {
    const controller = makeController({
      messages: [
        message({ id: 'task', actor_id: ID.alice, run_id: ID.run, body: 'Task: Sum the columns' }),
        message({ id: 'step', actor_id: ID.alice, run_id: ID.run, body: 'Using crew__request' }),
        message({ id: 'result', actor_id: ID.alice, run_id: ID.run, body: 'The totals are 1.80.' }),
      ],
      runs: [run({ status: 'completed' })],
    });
    renderWithController(<Timeline />, controller);
    const row = screen.getByRole('group', { name: /^Your agent · / });
    const result = screen.getByText('The totals are 1.80.');
    expect(result.compareDocumentPosition(row) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    // The task keeps its title from its own post.
    expect(within(row).getByText('Sum the columns')).toBeInTheDocument();
  });

  it('sits with the task’s post while there is no result yet', () => {
    const controller = makeController({
      messages: [
        message({ id: 'task', actor_id: ID.alice, run_id: ID.run, body: 'Task: Sum the columns' }),
        message({ id: 'later', actor_id: ID.bob, body: 'Thanks!' }),
      ],
      runs: [run({ status: 'running' })],
    });
    renderWithController(<Timeline />, controller);
    const row = screen.getByRole('group', { name: /^Your agent · / });
    const later = screen.getByText('Thanks!');
    expect(row.compareDocumentPosition(later) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it('never stands above the loading skeleton (Q2-62)', () => {
    const controller = makeController({
      messages: [],
      messagesLoaded: false,
      runs: [run({ status: 'running' })],
    });
    renderWithController(<Timeline />, controller);
    expect(screen.queryByRole('group', { name: /^Your agent · / })).toBeNull();
    expect(document.querySelector('.crew-timeline-skeleton')).not.toBeNull();
  });

  it('shows a task whose request is not loaded at the end of the log, without a title', () => {
    const controller = makeController({
      messages: [message({ body: 'unrelated' })],
      runs: [run({ status: 'running' })],
    });
    renderWithController(<Timeline />, controller);
    const row = screen.getByRole('group', { name: /^Your agent · / });
    expect(row).toHaveTextContent(`${identityCopy.yourAgent} · Working…`);
    expect(row.querySelector('.crew-task-title')).toBeNull();
    const log = screen.getByRole('log');
    const rows = log.querySelectorAll('[data-crew-row]');
    expect(rows[rows.length - 1]).toBe(row);
  });
});
