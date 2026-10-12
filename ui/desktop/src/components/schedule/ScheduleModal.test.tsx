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
   * An empty name never reaches the daemon, and the sentence that says so is the app's own
   * (vision V3-R4-1). The Input used to carry the native `required`, so Chromium answered the
   * submit with its own themeless bubble ("Please fill out this field.") and `handleLocalSubmit`
   * never ran: no danger edge, no line under the field. The form is `noValidate` now and the
   * Field says it: `aria-invalid` on the control and the message through `aria-describedby`.
   */
  it('answers an empty name with the Field error, not a native bubble', async () => {
    const user = userEvent.setup();
    const { onSubmit } = renderModal();

    await user.type(screen.getByLabelText('Workflow file'), '/tmp/wf.yaml');
    await user.click(screen.getByRole('button', { name: 'Create schedule' }));

    const name = screen.getByLabelText('Name');
    expect(onSubmit).not.toHaveBeenCalled();
    expect(await screen.findByRole('alert')).toHaveTextContent('Enter a name for this schedule.');
    expect(name).toHaveAttribute('aria-invalid', 'true');
    expect(name).toHaveAccessibleDescription('Enter a name for this schedule.');

    // Typing clears it again, the same as every other Field error.
    await user.type(name, 'a');
    expect(name).not.toHaveAttribute('aria-invalid');
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('answers an empty workflow file the same way', async () => {
    const user = userEvent.setup();
    const { onSubmit } = renderModal();

    await user.type(screen.getByLabelText('Name'), 'daily-summary');
    await user.click(screen.getByRole('button', { name: 'Create schedule' }));

    const file = screen.getByLabelText('Workflow file');
    expect(onSubmit).not.toHaveBeenCalled();
    expect(await screen.findByRole('alert')).toHaveTextContent('Choose a workflow file.');
    expect(file).toHaveAttribute('aria-invalid', 'true');
    expect(file).toHaveAccessibleDescription('Choose a workflow file.');
  });

  /**
   * The wiring the two cases above stand on, stated at the source. jsdom does run constraint
   * validation on a submit, so a native `required` (or a form without `noValidate`) fails them,
   * but real Chromium shows its bubble where jsdom shows nothing, so the attributes are pinned
   * as well: neither control carries `required`, both are `aria-required`, and the form opts out.
   */
  it('leaves validation to the Field: noValidate, no native required, aria-required instead', () => {
    renderModal();

    expect(document.getElementById('schedule-form')).toHaveAttribute('novalidate');
    for (const label of ['Name', 'Workflow file']) {
      const control = screen.getByLabelText(label);
      expect(control).not.toHaveAttribute('required');
      expect(control).toHaveAttribute('aria-required', 'true');
    }
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

/**
 * Coral means "this will do something" (vision V3-R9-3, coordinator ruling 22, shared-controls.md
 * "A Button that cannot act"). New schedule opened with Name and Workflow file empty and a coral,
 * enabled "Create schedule" whose only answer was to name the missing field. Like the provider
 * dialogs and Create and run, it is the neutral secondary until it can act, and it stays enabled so
 * the click still says what is missing. `bg-background-accent` is the `default` variant's fill and
 * `br-fill-rest` the `secondary` one's, so the class says which of the two is drawn.
 */
describe('ScheduleModal primary button colour', () => {
  const create = () => screen.getByRole('button', { name: 'Create schedule' });
  const isCoral = (el: HTMLElement) => /\bbg-background-accent\b/.test(el.className);
  const isNeutral = (el: HTMLElement) =>
    /\bbr-fill-rest\b/.test(el.className) && !/\bbg-background-accent\b/.test(el.className);

  it('is neutral and still enabled on an empty form', () => {
    renderModal();

    expect(isNeutral(create())).toBe(true);
    expect(create()).toBeEnabled();
  });

  it('stays neutral until BOTH the name and the workflow file are filled', async () => {
    const user = userEvent.setup();
    renderModal();

    await user.type(screen.getByLabelText('Name'), 'daily-summary');
    expect(isNeutral(create())).toBe(true);

    await user.type(screen.getByLabelText('Workflow file'), '/tmp/wf.yaml');
    expect(isCoral(create())).toBe(true);
    expect(create()).toBeEnabled();
  });

  it('turns neutral again when a filled field is cleared', async () => {
    const user = userEvent.setup();
    renderModal();

    await user.type(screen.getByLabelText('Name'), 'daily-summary');
    await user.type(screen.getByLabelText('Workflow file'), '/tmp/wf.yaml');
    expect(isCoral(create())).toBe(true);

    await user.clear(screen.getByLabelText('Workflow file'));
    expect(isNeutral(create())).toBe(true);
  });

  /**
   * A name the daemon would refuse is not "filled": the click would only name the rule, so the button
   * is judged by the same `scheduleNameProblem` the submit runs.
   */
  it('is neutral while the name breaks the rule, and coral once it is fixed', async () => {
    const user = userEvent.setup();
    renderModal();

    await user.type(screen.getByLabelText('Workflow file'), '/tmp/wf.yaml');
    await user.type(screen.getByLabelText('Name'), 'daily summary');
    expect(isNeutral(create())).toBe(true);

    await user.clear(screen.getByLabelText('Name'));
    await user.type(screen.getByLabelText('Name'), 'daily-summary');
    expect(isCoral(create())).toBe(true);
  });

  it('still names the missing field when pressed neutral', async () => {
    const user = userEvent.setup();
    const { onSubmit } = renderModal();

    await user.click(create());

    expect(onSubmit).not.toHaveBeenCalled();
    expect(await screen.findByRole('alert')).toHaveTextContent('Enter a name for this schedule.');
  });

  it('is coral when editing, where there is nothing to fill in', () => {
    renderModal({
      schedule: {
        id: 'daily-summary',
        source: '/tmp/wf.yaml',
        cron: '0 0 14 * * *',
        last_run: null,
        currently_running: false,
        paused: false,
      } as never,
    });

    const save = screen.getByRole('button', { name: 'Save changes' });
    expect(isCoral(save)).toBe(true);
    expect(save).toBeEnabled();
  });
});

/**
 * V4-R7-5 (`<Input machine />`): the workflow file is a path, a machine string, so Chromium must
 * not underline a correct `/Users/me/daily.yaml` or capitalise its first letter. The schedule's
 * Name above it is the person's own label and keeps its spellcheck. jsdom has no spellchecker;
 * what is pinned is the markup the engine reads (`ui/input.test.tsx` pins the primitive).
 */
describe('ScheduleModal machine strings', () => {
  it('turns spellcheck, auto-correct and auto-capitalise off on the workflow file, and not on the name', () => {
    renderModal();

    const workflowFile = screen.getByLabelText('Workflow file');
    expect(workflowFile).toHaveAttribute('spellcheck', 'false');
    expect(workflowFile).toHaveAttribute('autocorrect', 'off');
    expect(workflowFile).toHaveAttribute('autocapitalize', 'off');

    const name = screen.getByLabelText('Name');
    expect(name).not.toHaveAttribute('spellcheck');
    expect(name).not.toHaveAttribute('autocorrect');
    expect(name).not.toHaveAttribute('autocapitalize');
  });
});
