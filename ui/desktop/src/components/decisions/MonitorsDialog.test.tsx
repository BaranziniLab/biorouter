import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  parseDecisionModelEntries,
  parseDecisionSettings,
  type DecisionSettings,
} from '../../utils/sessionDecisions';
import { MonitorsDialog, monitorSlug, newMonitor, validateMonitor } from './MonitorsDialog';

const mocks = vi.hoisted(() => ({ save: vi.fn(), models: vi.fn() }));
vi.mock('../../utils/sessionDecisions', async (original) => ({
  ...(await original<typeof import('../../utils/sessionDecisions')>()),
  saveDecisionSettings: mocks.save,
  fetchDecisionModels: mocks.models,
}));

const MODELS = parseDecisionModelEntries([
  {
    provider: 'ollama_decision',
    model: 'tev1:0.8b',
    displayName: 'tev1:0.8b',
    contextTokens: 8192,
    kinds: ['binary', 'choice', 'score'],
    tier: 'private',
    adapter: 'system_one',
    local: true,
  },
  {
    provider: 'baseten_decision',
    model: 'decision-models/baseten-joey-2b-latest',
    displayName: 'joey-1',
    contextTokens: 16384,
    kinds: ['binary', 'choice', 'score'],
    tier: 'public',
    adapter: 'system_one',
    local: false,
  },
])!;

const STORED: DecisionSettings = parseDecisionSettings({
  defaultModel: null,
  monitors: [
    {
      id: 'form-reached',
      name: 'Form reached',
      question: { instructions: 'Has the agent reached the consent form?', kind: { binary: {} } },
      threshold: 0.85,
      action: 'stop',
    },
  ],
})!;

function draw(overrides: Partial<Parameters<typeof MonitorsDialog>[0]> = {}) {
  const onOpenChange = vi.fn();
  const onSaved = vi.fn();
  render(
    <MonitorsDialog
      open
      onOpenChange={onOpenChange}
      sessionId="s1"
      chatTier="private"
      settings={STORED}
      onSaved={onSaved}
      models={MODELS}
      {...overrides}
    />
  );
  return { onOpenChange, onSaved };
}

beforeEach(() => {
  mocks.save
    .mockReset()
    .mockImplementation(async (_id: string, settings: DecisionSettings) => settings);
  mocks.models.mockReset().mockResolvedValue(MODELS);
});

afterEach(() => {
  delete document.documentElement.dataset.biorouterSurface;
});

describe('validation, as the daemon would refuse it', () => {
  it('names the field: a missing name and question, too few or duplicate options', () => {
    const blank = newMonitor();
    expect(validateMonitor(blank, { chatTier: 'public', models: MODELS })).toMatchObject({
      name: expect.any(String),
      question: expect.any(String),
    });
    const choice = {
      ...blank,
      name: 'Course',
      question: {
        instructions: 'Which course?',
        kind: {
          type: 'choice' as const,
          options: [
            { name: 'rrms', description: null },
            { name: 'RRMS', description: null },
          ],
        },
        threshold: null,
      },
      option: 'rrms',
    };
    expect(validateMonitor(choice, { chatTier: 'public', models: MODELS }).options).toMatch(
      /its own name/
    );
  });

  it('refuses a public model for a private chat, and allows it for a public one', () => {
    const monitor = {
      ...newMonitor(),
      name: 'Done',
      question: { instructions: 'Is it done?', kind: { type: 'binary' as const }, threshold: null },
      model: {
        provider: 'baseten_decision',
        model: 'decision-models/baseten-joey-2b-latest',
        adapter: null,
      },
    };
    expect(validateMonitor(monitor, { chatTier: 'private', models: MODELS }).model).toMatch(
      /private/
    );
    expect(validateMonitor(monitor, { chatTier: 'public', models: MODELS }).model).toBeUndefined();
  });

  it('slugs an id that is unique in the chat', () => {
    expect(monitorSlug('Form reached', new Set())).toBe('form-reached');
    expect(monitorSlug('Form reached', new Set(['form-reached']))).toBe('form-reached-2');
    expect(monitorSlug('!!!', new Set())).toBe('monitor');
  });
});

describe('MonitorsDialog', () => {
  it('lists the stored monitors with what each does', () => {
    draw();
    const item = screen.getByTestId('monitor-item');
    expect(item).toHaveTextContent('Form reached');
    expect(item).toHaveTextContent(
      "P(yes) ≥ 0.85 · Stop the turn · After each step · The chat's decision model"
    );
  });

  it('shows what is missing inline and does not save it', async () => {
    draw();
    fireEvent.click(screen.getByTestId('monitor-add'));
    fireEvent.click(screen.getByTestId('monitor-done'));
    const editor = screen.getByTestId('monitor-editor');
    expect(within(editor).getByText('Give the monitor a name.')).toBeInTheDocument();
    expect(within(editor).getByText('Write the question the model answers.')).toBeInTheDocument();
    fireEvent.click(screen.getByTestId('monitors-save'));
    expect(mocks.save).not.toHaveBeenCalled();
  });

  it('adds a monitor and saves the whole list with the contract shape', async () => {
    const { onSaved, onOpenChange } = draw();
    fireEvent.click(screen.getByTestId('monitor-add'));
    fireEvent.change(screen.getByTestId('monitor-name'), { target: { value: 'Task achieved' } });
    fireEvent.change(screen.getByTestId('monitor-question'), {
      target: { value: 'Has the agent finished what was asked?' },
    });
    fireEvent.click(screen.getByRole('radio', { name: 'Tell the agent' }));
    fireEvent.click(screen.getByRole('radio', { name: 'When the turn ends' }));
    fireEvent.click(screen.getByTestId('monitor-done'));
    expect(screen.getAllByTestId('monitor-item')).toHaveLength(2);
    fireEvent.click(screen.getByTestId('monitors-save'));
    await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1));
    const [sessionId, settings] = mocks.save.mock.calls[0] as [string, DecisionSettings];
    expect(sessionId).toBe('s1');
    expect(settings.monitors.map((monitor) => monitor.id)).toEqual([
      'form-reached',
      'task-achieved',
    ]);
    expect(settings.monitors[1]).toMatchObject({
      name: 'Task achieved',
      action: 'steer',
      timing: 'turn_end',
      enabled: true,
      threshold: 0.85,
      question: {
        instructions: 'Has the agent finished what was asked?',
        kind: { type: 'binary' },
      },
    });
    await waitFor(() => expect(onSaved).toHaveBeenCalled());
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it('removes a monitor', async () => {
    draw();
    fireEvent.click(screen.getByRole('button', { name: 'Remove Form reached' }));
    expect(screen.queryByTestId('monitor-item')).toBeNull();
    fireEvent.click(screen.getByTestId('monitors-save'));
    await waitFor(() => expect(mocks.save).toHaveBeenCalled());
    expect((mocks.save.mock.calls[0][1] as DecisionSettings).monitors).toEqual([]);
  });

  it('shows the daemon’s refusal above the footer and stays open', async () => {
    mocks.save.mockRejectedValue(new Error('A chat can have at most 8 monitors.'));
    const { onOpenChange } = draw();
    fireEvent.click(screen.getByTestId('monitors-save'));
    expect(await screen.findByTestId('monitors-save-error')).toHaveTextContent(
      'A chat can have at most 8 monitors.'
    );
    expect(onOpenChange).not.toHaveBeenCalledWith(false);
  });

  it('offers a public model to a private chat only disabled, with the reason', async () => {
    draw();
    fireEvent.click(screen.getByTestId('monitor-add'));
    const input = within(screen.getByTestId('monitor-model')).getByRole('combobox');
    fireEvent.keyDown(input, { key: 'ArrowDown' });
    const option = await screen.findByText('joey-1 · Public');
    const row = option.closest('[role="option"]') ?? option.closest('[aria-disabled]');
    expect(row).toHaveAttribute('aria-disabled', 'true');
    expect(
      screen.getByText('This chat is private, so a public model may not read it.')
    ).toBeInTheDocument();
    expect(screen.getByText('tev1:0.8b · Private · this computer')).toBeInTheDocument();
  });

  it('in a browser, saves the same way: a keyless daemon takes the write', async () => {
    document.documentElement.dataset.biorouterSurface = 'headless';
    draw();
    expect(screen.getByTestId('monitors-save')).not.toBeDisabled();
    fireEvent.click(screen.getByTestId('monitors-save'));
    await waitFor(() => expect(mocks.save).toHaveBeenCalled());
  });

  it('asks for the catalog naming the chat, and takes the daemon’s own reason', async () => {
    mocks.models.mockResolvedValue(
      parseDecisionModelEntries({
        models: [
          {
            ...MODELS[0],
            unavailableReason: 'Your institution does not cover this model.',
          },
        ],
      })
    );
    draw({ models: undefined });
    await waitFor(() => expect(mocks.models).toHaveBeenCalledWith(expect.any(AbortSignal), 's1'));
    fireEvent.click(screen.getByTestId('monitor-add'));
    const input = within(screen.getByTestId('monitor-model')).getByRole('combobox');
    fireEvent.keyDown(input, { key: 'ArrowDown' });
    expect(
      await screen.findByText('Your institution does not cover this model.')
    ).toBeInTheDocument();
  });
});
