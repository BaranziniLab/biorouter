import { act, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { ScheduleModal } from './ScheduleModal';
import { focusDialogOnOpen } from './dialogFocus';

vi.mock('../../workflow/workflow_management', () => ({
  getStorageDirectory: () => '/tmp',
}));

const job = {
  id: 'nightly-cohort',
  source: '/tmp/nightly.yaml',
  cron: '0 0 2 * * *',
  last_run: null,
  currently_running: false,
  paused: false,
};

/**
 * Vision round 1, V3-20. Opened with the mouse, the Schedule dialogs showed their first closed
 * Select ("Every [Day]") autofocused and painted in the grey focus surface. A dialog with no text
 * field now takes focus itself; one that opens on a text field keeps it there.
 */
describe('where a schedule dialog puts focus when it opens', () => {
  const open = async (schedule: typeof job | null) => {
    await act(async () => {
      render(
        <ScheduleModal
          isOpen
          onClose={vi.fn()}
          onSubmit={vi.fn()}
          schedule={schedule}
          isLoadingExternally={false}
          apiErrorExternally={null}
        />
      );
    });
  };

  it('focuses the dialog itself when editing, not the first closed Select', async () => {
    await open(job);

    const dialog = screen.getByRole('dialog');
    expect(document.activeElement).toBe(dialog);
    expect(dialog.contains(screen.getByRole('combobox', { name: 'Repeat every' }))).toBe(true);
    expect(document.activeElement).not.toBe(screen.getByRole('combobox', { name: 'Repeat every' }));
  });

  it('still opens New schedule on its Name field', async () => {
    await open(null);

    expect(document.activeElement).toBe(screen.getByLabelText('Name'));
  });
});

describe('focusDialogOnOpen', () => {
  it('cancels Radix’s own choice and focuses the dialog the event came from', () => {
    const dialog = document.createElement('div');
    dialog.tabIndex = -1;
    document.body.appendChild(dialog);
    const event = new CustomEvent('focusScope.autoFocusOnMount', { cancelable: true });
    dialog.addEventListener('focusScope.autoFocusOnMount', focusDialogOnOpen);

    dialog.dispatchEvent(event);

    expect(event.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(dialog);
    dialog.remove();
  });
});
