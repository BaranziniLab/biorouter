import { render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import CreateEditWorkflowModal from './CreateEditWorkflowModal';
import { workflowFormCopy as copy } from './copy';

vi.mock('../../api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../api')>()),
  getExtensions: vi.fn().mockResolvedValue({ data: { extensions: [] } }),
}));

const workflow = {
  title: 'Cohort review',
  description: 'Review the cohort',
  instructions: 'Review {{cohort}}',
};

/**
 * One editor shape (spec 3.10): `ModalShell size="lg" scrollBody` (640px) with a title only,
 * the shared field skin, and Cancel · Save · Save and run. Sharing lives on the Workflows row
 * menu ("Copy link"), not in the editor.
 */
describe('CreateEditWorkflowModal', () => {
  it('is the 640px editor with a title and nothing under it', () => {
    render(<CreateEditWorkflowModal isOpen onClose={vi.fn()} workflow={workflow} />);

    const dialog = screen.getByRole('dialog', { name: copy.editTitle });
    expect(dialog).toHaveAttribute('data-size', 'lg');
    expect(dialog).not.toHaveAttribute('aria-describedby');
    expect(within(dialog).queryByText(/learn more/i)).toBeNull();
    expect(within(dialog).queryByText(/share link/i)).toBeNull();
    expect(within(dialog).getByRole('textbox', { name: copy.title })).toHaveClass('br-field');
  });

  it('offers Cancel, Save and Save and run, with one primary', () => {
    render(<CreateEditWorkflowModal isOpen onClose={vi.fn()} workflow={workflow} />);

    const dialog = screen.getByRole('dialog');
    // Cancel is the filled secondary here as in every other dialog (V3-06), never ghost.
    expect(within(dialog).getByRole('button', { name: copy.cancel })).toHaveClass('br-fill-rest');
    expect(within(dialog).getByRole('button', { name: copy.save })).not.toHaveClass(
      'bg-background-accent'
    );
    expect(within(dialog).getByRole('button', { name: copy.saveAndRun })).toHaveClass(
      'bg-background-accent'
    );
    expect(dialog.querySelectorAll('button.bg-background-accent')).toHaveLength(1);
  });

  it('is titled for a new workflow in create mode, and creates rather than saves', () => {
    render(<CreateEditWorkflowModal isOpen onClose={vi.fn()} isCreateMode />);

    expect(screen.getByRole('dialog', { name: copy.newTitle })).toBeInTheDocument();
    // A workflow that does not exist yet is created, not saved (V3-17).
    expect(screen.getByRole('button', { name: copy.create })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: copy.save })).toBeNull();
    expect(screen.queryByRole('button', { name: copy.saveAndRun })).toBeNull();
    // Nothing is filled in yet, so nothing can be saved.
    expect(screen.getByRole('button', { name: copy.createAndRun })).toBeDisabled();
  });
});
