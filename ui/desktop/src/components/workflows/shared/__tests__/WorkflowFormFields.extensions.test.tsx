import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useForm } from '@tanstack/react-form';

import { WorkflowFormFields } from '../WorkflowFormFields';
import { type WorkflowFormData } from '../workflowFormSchema';
import { workflowFormCopy as copy } from '../../copy';

const mocks = vi.hoisted(() => ({ getExtensions: vi.fn() }));

vi.mock('../../../../api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../../../api')>()),
  getExtensions: mocks.getExtensions,
}));

/**
 * Vision round 1, V3-05 (principle 10, show names not ids). The workflow editor's Extensions
 * picker titled its rows with the config keys (`chatrecall`, `agent_drafter`, `computercontroller`),
 * so nobody could tell which one Biorouter Copilot was. It names them as the Extensions page does.
 */
describe('the workflow editor’s Extensions picker', () => {
  beforeAll(() => {
    vi.stubGlobal(
      'ResizeObserver',
      class {
        observe() {}
        unobserve() {}
        disconnect() {}
      }
    );
  });
  afterAll(() => vi.unstubAllGlobals());

  beforeEach(() => {
    mocks.getExtensions.mockResolvedValue({
      data: {
        extensions: [
          { type: 'platform', name: 'chatrecall', description: 'Search past chats', enabled: true },
          { type: 'platform', name: 'agent_drafter', description: 'Build apps', enabled: true },
          { type: 'platform', name: 'code_execution', description: 'Run code', enabled: true },
          {
            type: 'builtin',
            name: 'autovisualiser',
            display_name: 'Auto Visualiser',
            description: 'Charts',
            enabled: true,
          },
          {
            type: 'builtin',
            name: 'computercontroller',
            display_name: 'Biorouter Copilot',
            description: 'Control the desktop',
            enabled: true,
          },
        ],
      },
    });
  });

  const Form = () => {
    const defaultValues: WorkflowFormData = {
      title: '',
      description: '',
      instructions: '',
      prompt: '',
      activities: [],
      parameters: [],
      jsonSchema: '',
    };
    const form = useForm({ defaultValues });
    return <WorkflowFormFields form={form} extensions={[]} onExtensionsChange={vi.fn()} />;
  };

  async function openPicker(user: ReturnType<typeof userEvent.setup>) {
    render(<Form />);
    await user.click(screen.getByRole('button', { name: copy.advanced }));
    await user.click(await screen.findByText('No extensions selected'));
  }

  it('titles each row with its display name, never its config key', async () => {
    const user = userEvent.setup();
    await openPicker(user);

    for (const name of [
      'Agent Drafter',
      'Auto Visualiser',
      'Biorouter Copilot',
      'Chat Recall',
      'Code Execution',
    ]) {
      expect(await screen.findByRole('checkbox', { name })).toBeInTheDocument();
    }
    for (const key of ['chatrecall', 'agent_drafter', 'code_execution', 'computercontroller']) {
      expect(screen.queryByText(key)).toBeNull();
    }
  });

  it('still finds a row by its key, which stays in the search index', async () => {
    const user = userEvent.setup();
    await openPicker(user);

    await user.type(screen.getByRole('searchbox', { name: copy.searchExtensions }), 'computercon');
    expect(await screen.findByRole('checkbox', { name: 'Biorouter Copilot' })).toBeInTheDocument();
    expect(screen.queryByRole('checkbox', { name: 'Chat Recall' })).toBeNull();
  });
});
