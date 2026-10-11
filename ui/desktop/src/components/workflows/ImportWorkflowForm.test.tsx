import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import ImportWorkflowForm from './ImportWorkflowForm';
import { workflowsCopy } from './copy';

vi.mock('../../api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../api')>()),
  parseWorkflow: vi.fn(),
}));
vi.mock('../../workflow/workflow_management', () => ({ saveWorkflow: vi.fn() }));

const copy = workflowsCopy.importDialog;
const CSS = readFileSync(join(__dirname, 'workflows.css'), 'utf8');

/**
 * Vision round 1, V3-08 (and R-27). The Import dialog's drop zone was a dashed, bare-icon, two-line
 * box beside Knowledge's and Add skill's solid, plated, one-line one. It is one recipe now, on the
 * fill ground with no resting outline.
 */
describe('ImportWorkflowForm', () => {
  const renderForm = () =>
    render(<ImportWorkflowForm isOpen onClose={vi.fn()} onSuccess={vi.fn()} />);

  it('draws one plate over one line in the drop zone', () => {
    renderForm();
    const zone = screen.getByRole('button', { name: copy.drop });

    expect(copy.drop).toBe('Drop a .yaml or .json file here');
    expect(zone).toHaveClass('br-workflow-dropzone');
    expect(zone.querySelector('.br-workflow-drop-plate svg')).not.toBeNull();
    // One line: the old "or click to browse" second line is gone.
    expect(within(zone).queryByText(/click to browse/i)).toBeNull();
    expect(zone.querySelectorAll('p')).toHaveLength(0);
  });

  it('rests on the fill, not on a dashed outline', () => {
    const zone = /\.br-workflow-dropzone\s*\{([^}]*)\}/.exec(CSS)?.[1] ?? '';
    expect(zone).toMatch(/background-color:\s*var\(--background-fill\)/);
    expect(zone).toMatch(/border:\s*1px solid transparent/);
    expect(zone).not.toMatch(/dashed/);
    expect(CSS).toMatch(/\.br-workflow-dropzone:hover\s*\{[^}]*var\(--background-fill-strong\)/);
  });

  it('draws Cancel as the filled secondary', () => {
    renderForm();
    expect(screen.getByRole('button', { name: copy.cancel })).toHaveClass('bg-background-medium');
  });
});
