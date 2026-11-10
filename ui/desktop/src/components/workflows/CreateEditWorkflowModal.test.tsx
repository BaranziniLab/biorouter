import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { Workflow } from '../../workflow';
import CreateEditWorkflowModal, { SUPERVISION_KEPT } from './CreateEditWorkflowModal';
import { workflowFormCopy as copy } from './copy';

vi.mock('../../api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../api')>()),
  getExtensions: vi.fn().mockResolvedValue({ data: { extensions: [] } }),
}));

const saving = vi.hoisted(() => ({ save: vi.fn(), models: vi.fn() }));
vi.mock('../../workflow/workflow_management', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../workflow/workflow_management')>()),
  saveWorkflow: saving.save,
}));
vi.mock('../../utils/sessionDecisions', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../utils/sessionDecisions')>()),
  fetchDecisionModels: saving.models,
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

  it('saving keeps decision_settings and orchestration_plan', async () => {
    saving.save.mockResolvedValue('wf-1');
    saving.models.mockResolvedValue([]);
    const supervised: Workflow = {
      ...workflow,
      decision_settings: {
        defaultModel: { provider: 'ollama_decision', model: 'tev1:0.8b' },
        monitors: [
          {
            id: 'reached-ten',
            name: 'reached-ten',
            question: {
              instructions: 'Has the agent already echoed 10 or a larger number?',
              kind: { binary: {} },
            },
            threshold: 0.7,
            action: 'stop',
            onError: 'pause',
          },
        ],
      },
      orchestration_plan: {
        title: 'Echo plan',
        phases: [
          {
            id: 'count',
            title: 'Count',
            gate: { questions: { done: 'Did the agent reach 30?' }, threshold: 0.9 },
          },
        ],
      },
    };
    const onClose = vi.fn();
    render(<CreateEditWorkflowModal isOpen onClose={onClose} workflow={supervised} />);

    // A read-only summary: the editor has no fields for either block.
    const summary = screen.getByTestId('workflow-supervision');
    expect(
      within(summary).getByTestId('workflow-supervision-monitor-reached-ten')
    ).toHaveTextContent('reached-ten · chat default · stops the turn');
    expect(within(summary).getByTestId('workflow-supervision-gate-count')).toBeInTheDocument();
    expect(within(summary).getByTestId('workflow-supervision-kept')).toHaveTextContent(
      SUPERVISION_KEPT
    );
    expect(SUPERVISION_KEPT).toBe(
      'Monitors and phase gates are edited in the workflow file; saving here keeps them.'
    );

    fireEvent.change(screen.getByRole('textbox', { name: copy.title }), {
      target: { value: 'Cohort review, renamed' },
    });
    fireEvent.click(screen.getByRole('button', { name: copy.save }));
    await waitFor(() => expect(saving.save).toHaveBeenCalledTimes(1));
    const saved = saving.save.mock.calls[0][0] as Workflow;
    expect(saved.title).toBe('Cohort review, renamed');
    expect(saved.decision_settings).toEqual(supervised.decision_settings);
    expect(saved.orchestration_plan).toEqual(supervised.orchestration_plan);
  });

  it('shows no Supervision summary for a workflow that declares none', () => {
    render(<CreateEditWorkflowModal isOpen onClose={vi.fn()} workflow={workflow} />);
    expect(screen.queryByTestId('workflow-supervision')).toBeNull();
    expect(screen.queryByText(SUPERVISION_KEPT)).toBeNull();
  });
});
