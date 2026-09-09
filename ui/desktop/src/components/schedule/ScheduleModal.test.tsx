import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { ScheduleModal } from './ScheduleModal';

vi.mock('./CronPicker', () => ({
  CronPicker: () => <div data-testid="cron-picker" />,
}));

vi.mock('../../workflow/workflow_management', () => ({
  getStorageDirectory: () => '/tmp',
}));

function renderModal(overrides: Partial<Parameters<typeof ScheduleModal>[0]> = {}) {
  const onSubmit = vi.fn().mockResolvedValue(undefined);
  render(
    <ScheduleModal
      isOpen
      onClose={vi.fn()}
      onSubmit={onSubmit}
      schedule={null}
      isLoadingExternally={false}
      apiErrorExternally={null}
      {...overrides}
    />
  );
  return { onSubmit };
}

describe('ScheduleModal name validation', () => {
  /**
   * The reported bug (F4). `<img src=x onerror=alert(1)>` was sent, refused by
   * the daemon's `validate_schedule_id`, and reported back as "Failed to create
   * schedule: Unexpected response format" — a transport-shaped message for a
   * problem the user could fix in one keystroke.
   *
   * The daemon still refuses it; this stops the round trip being the first
   * place the user hears about the rule.
   */
  it('explains the rule instead of submitting a name the daemon will refuse', async () => {
    const user = userEvent.setup();
    const { onSubmit } = renderModal();

    await user.type(screen.getByLabelText('Name'), '<img src=x onerror=alert(1)>');
    await user.type(screen.getByLabelText('Workflow file'), '/tmp/wf.yaml');
    await user.click(screen.getByRole('button', { name: 'Create schedule' }));

    expect(await screen.findByRole('alert')).toHaveTextContent(
      "The name may only contain letters, digits, '-' and '_'."
    );
    expect(onSubmit).not.toHaveBeenCalled();
  });

  /**
   * An empty name never reaches the daemon either — but by the field's own
   * `required`, which the browser answers before the form submits, so the
   * "Enter a name" arm of `scheduleNameProblem` is a floor rather than the
   * sentence the user reads. Asserted because "nothing was submitted" is the
   * property that matters, and because a future edit that drops `required`
   * should have to look at this.
   */
  it('does not submit an empty name at all', async () => {
    const user = userEvent.setup();
    const { onSubmit } = renderModal();

    await user.type(screen.getByLabelText('Workflow file'), '/tmp/wf.yaml');
    await user.click(screen.getByRole('button', { name: 'Create schedule' }));

    expect(onSubmit).not.toHaveBeenCalled();
    expect(screen.getByLabelText('Name')).toBeRequired();
  });

  /**
   * The guard must not become a wall: a name that satisfies the daemon's rule
   * goes through untouched. Fails an over-eager regex (one that rejects `_`,
   * say) that would make the dialog refuse names the daemon accepts.
   */
  it('submits a name the daemon accepts', async () => {
    const user = userEvent.setup();
    const { onSubmit } = renderModal();

    await user.type(screen.getByLabelText('Name'), 'daily-summary_job2');
    await user.type(screen.getByLabelText('Workflow file'), '/tmp/wf.yaml');
    await user.click(screen.getByRole('button', { name: 'Create schedule' }));

    expect(onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'daily-summary_job2', workflow_source: '/tmp/wf.yaml' })
    );
    expect(screen.queryByRole('alert')).toBeNull();
  });

  /**
   * The daemon's refusal is the authority and still has to be readable when it
   * arrives — the half of F4 that lives on the server. `apiErrorExternally` is
   * what `createSchedule` now carries the daemon's own sentence into.
   */
  it('shows the daemon refusal it is handed', () => {
    renderModal({
      apiErrorExternally:
        "Failed to create schedule: Invalid job ID: schedule id 'x y' may only contain letters, digits, '-' and '_'",
    });

    expect(screen.getByRole('alert')).toHaveTextContent('may only contain letters, digits');
  });
});
