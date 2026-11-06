import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  DecisionRequestError,
  parseDecisionCatalog,
  parseDecisionModelEntries,
  parseDecisionSettings,
  type DecisionSettings,
  type MonitorCriterion,
} from '../../utils/sessionDecisions';
import {
  MonitorsDialog,
  monitorSlug,
  monitorSummary,
  newMonitor,
  validateMonitor,
} from './MonitorsDialog';

const mocks = vi.hoisted(() => ({ save: vi.fn(), catalog: vi.fn(), settings: vi.fn() }));
vi.mock('../../utils/sessionDecisions', async (original) => ({
  ...(await original<typeof import('../../utils/sessionDecisions')>()),
  saveDecisionSettings: mocks.save,
  fetchDecisionCatalog: mocks.catalog,
  fetchDecisionSettings: mocks.settings,
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

/** Every open reads what is stored now; changes wait for that answer. */
async function drawReady(overrides: Partial<Parameters<typeof MonitorsDialog>[0]> = {}) {
  const handles = draw(overrides);
  await waitFor(() => expect(screen.queryByTestId('monitors-loading')).toBeNull());
  return handles;
}

beforeEach(() => {
  mocks.save
    .mockReset()
    .mockImplementation(async (_id: string, settings: DecisionSettings) => settings);
  mocks.catalog.mockReset().mockResolvedValue({ models: MODELS, sessionUnavailableReason: null });
  mocks.settings.mockReset().mockResolvedValue({ data: STORED, missing: false });
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

  it('refuses every model, the chat’s own included, when the chat may use none', () => {
    const crew =
      'This chat works under a Crew grant, which fixes the one model its content may reach.';
    const monitor = {
      ...newMonitor(),
      name: 'Done',
      question: { instructions: 'Is it done?', kind: { type: 'binary' as const }, threshold: null },
    };
    expect(monitor.model).toBeNull();
    expect(
      validateMonitor(monitor, { chatTier: 'public', models: MODELS, chatRefusal: crew }).model
    ).toBe(crew);
    const local = {
      ...monitor,
      model: { provider: 'ollama_decision', model: 'tev1:0.8b', adapter: null },
    };
    expect(
      validateMonitor(local, { chatTier: 'public', models: MODELS, chatRefusal: crew }).model
    ).toBe(crew);
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

  it('names a Baseten model in the list by its alias, as its model menu does', () => {
    const monitor = STORED.monitors[0];
    draw({
      settings: {
        ...STORED,
        monitors: [
          {
            ...monitor,
            model: {
              provider: 'baseten_decision',
              model: 'decision-models/baseten-joey-2b-latest',
              adapter: null,
            },
          },
        ],
      },
      chatTier: 'public',
    });
    const item = screen.getByTestId('monitor-item');
    expect(item).toHaveTextContent('After each step · joey-1');
    expect(item).not.toHaveTextContent('baseten-joey-2b-latest');
  });

  it('shows what is missing inline and does not save it', async () => {
    await drawReady();
    fireEvent.click(screen.getByTestId('monitor-add'));
    fireEvent.click(screen.getByTestId('monitor-done'));
    const editor = screen.getByTestId('monitor-editor');
    expect(within(editor).getByText('Give the monitor a name.')).toBeInTheDocument();
    expect(within(editor).getByText('Write the question the model answers.')).toBeInTheDocument();
    fireEvent.click(screen.getByTestId('monitors-save'));
    expect(mocks.save).not.toHaveBeenCalled();
  });

  it('adds a monitor and saves the whole list with the contract shape', async () => {
    const { onSaved, onOpenChange } = await drawReady();
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
    await drawReady();
    fireEvent.click(screen.getByRole('button', { name: 'Remove Form reached' }));
    expect(screen.queryByTestId('monitor-item')).toBeNull();
    fireEvent.click(screen.getByTestId('monitors-save'));
    await waitFor(() => expect(mocks.save).toHaveBeenCalled());
    expect((mocks.save.mock.calls[0][1] as DecisionSettings).monitors).toEqual([]);
  });

  it('shows the daemon’s refusal above the footer and stays open', async () => {
    mocks.save.mockRejectedValue(new Error('A chat can have at most 8 monitors.'));
    const { onOpenChange } = await drawReady();
    fireEvent.click(screen.getByTestId('monitors-save'));
    expect(await screen.findByTestId('monitors-save-error')).toHaveTextContent(
      'A chat can have at most 8 monitors.'
    );
    expect(onOpenChange).not.toHaveBeenCalledWith(false);
  });

  it('offers a public model to a private chat only disabled, with the reason', async () => {
    await drawReady();
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

  /**
   * Opened from the composer's `+` while the summary is folded away, nothing has
   * read the chat's settings yet. An empty list there would be a lie, and saving
   * it would replace every stored monitor and the chat's default model.
   */
  it('opened before the settings are known, reads them and holds Save until they arrive', async () => {
    const stored = parseDecisionSettings({
      ...STORED,
      defaultModel: { provider: 'ollama_decision', model: 'tev1:0.8b' },
    })!;
    expect(stored.defaultModel).not.toBeNull();
    let answer: (value: { data: DecisionSettings; missing: boolean }) => void = () => {};
    mocks.settings.mockReturnValue(new Promise((resolve) => (answer = resolve)));
    draw({ settings: null });
    expect(mocks.settings).toHaveBeenCalledWith('s1', expect.any(AbortSignal));
    expect(screen.getByTestId('monitors-loading')).toBeInTheDocument();
    expect(screen.getByTestId('monitors-save')).toBeDisabled();
    expect(screen.getByTestId('monitor-add')).toBeDisabled();
    fireEvent.click(screen.getByTestId('monitors-save'));
    expect(mocks.save).not.toHaveBeenCalled();

    answer({ data: stored, missing: false });
    expect(await screen.findByTestId('monitor-item')).toHaveTextContent('Form reached');
    expect(screen.queryByTestId('monitors-loading')).toBeNull();
    fireEvent.click(screen.getByTestId('monitors-save'));
    await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1));
    const saved = mocks.save.mock.calls[0][1] as DecisionSettings;
    expect(saved.monitors.map((monitor) => monitor.id)).toEqual(['form-reached']);
    expect(saved.defaultModel).toEqual(stored.defaultModel);
  });

  it('keeps Save off when the settings cannot be read', async () => {
    mocks.settings.mockRejectedValue(new Error('Decision settings unavailable'));
    draw({ settings: null });
    expect(await screen.findByTestId('monitors-load-error')).toBeInTheDocument();
    expect(screen.getByTestId('monitors-save')).toBeDisabled();
    expect(screen.getByTestId('monitor-add')).toBeDisabled();
  });

  /**
   * The caller's copy is a snapshot that an idle chat never refreshes, while
   * `biorouter session monitors` or another window writes the same record. Save
   * replaces the whole record, so the dialog reads what is stored on every open.
   */
  it('starts from what is stored now, not the caller’s older copy, and saves it whole', async () => {
    const fromCli = parseDecisionSettings({
      defaultModel: { provider: 'ollama_decision', model: 'tev1:0.8b' },
      monitors: [
        {
          id: 'form-reached',
          name: 'Form reached',
          question: {
            instructions: 'Has the agent reached the consent form?',
            kind: { binary: {} },
          },
          threshold: 0.85,
          action: 'stop',
        },
        {
          id: 'signed',
          name: 'Signed',
          question: { instructions: 'Has the form been signed?', kind: { binary: {} } },
          threshold: 0.9,
          action: 'notify',
        },
      ],
    })!;
    mocks.settings.mockResolvedValue({ data: fromCli, missing: false });
    const { onSaved } = draw();
    expect(mocks.settings).toHaveBeenCalledWith('s1', expect.any(AbortSignal));
    expect(screen.getByTestId('monitors-save')).toBeDisabled();
    await waitFor(() => expect(screen.getAllByTestId('monitor-item')).toHaveLength(2));
    expect(screen.getAllByTestId('monitor-item')[1]).toHaveTextContent('Signed');

    fireEvent.click(screen.getByRole('switch', { name: /Form reached/ }));
    fireEvent.click(screen.getByTestId('monitors-save'));
    await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1));
    const saved = mocks.save.mock.calls[0][1] as DecisionSettings;
    expect(saved.monitors.map((monitor) => [monitor.id, monitor.enabled])).toEqual([
      ['form-reached', false],
      ['signed', true],
    ]);
    expect(saved.defaultModel).toEqual(fromCli.defaultModel);
    await waitFor(() => expect(onSaved).toHaveBeenCalledWith(saved));
  });

  it('uses the caller’s copy when what is stored cannot be read', async () => {
    mocks.settings.mockRejectedValue(new Error('offline'));
    await drawReady();
    expect(screen.getByTestId('monitor-item')).toHaveTextContent('Form reached');
    expect(screen.getByTestId('monitors-save')).not.toBeDisabled();
  });

  it('draws its editor and rows as filled boxes whose controls rest one step back', async () => {
    await drawReady();
    // A fill never sits on a fill: `.br-fill-surface` re-points the controls' ground.
    expect(screen.getByTestId('monitor-item')).toHaveClass('br-fill-surface');
    fireEvent.click(screen.getByTestId('monitor-add'));
    expect(screen.getByTestId('monitor-editor')).toHaveClass('br-fill-surface');
    const css = readFileSync(join(__dirname, 'decisionsPanel.css'), 'utf8');
    for (const selector of ['.br-monitors__item', '.br-monitors__editor']) {
      const body = css.match(new RegExp(`\\n${selector.replace('.', '\\.')} \\{([^}]*)\\}`))?.[1];
      expect(body, selector).toBeTruthy();
      expect(body).toMatch(/border: 1px solid transparent;/);
      expect(body).toMatch(/background-color: var\(--background-fill\);/);
    }
  });

  it('in a browser, saves the same way: a keyless daemon takes the write', async () => {
    document.documentElement.dataset.biorouterSurface = 'headless';
    await drawReady();
    expect(screen.getByTestId('monitors-save')).not.toBeDisabled();
    fireEvent.click(screen.getByTestId('monitors-save'));
    await waitFor(() => expect(mocks.save).toHaveBeenCalled());
  });

  /**
   * A Crew grant rules out every decision model, so the daemon gives each row
   * its sentence; a monitor left on the chat's decision model names no row,
   * and used to be saved to be refused at every check.
   */
  it('in a Crew-scoped chat, blocks Save on the chat’s decision model with the Crew sentence', async () => {
    const crew =
      'This chat works under a Crew grant, which fixes the one model its content may reach, so a decision model cannot read it.';
    mocks.catalog.mockResolvedValue(
      parseDecisionCatalog({
        models: MODELS.map((entry) => ({ ...entry, unavailableReason: crew })),
        sessionUnavailableReason: crew,
      })
    );
    await drawReady({ models: undefined, chatTier: 'public' });
    await waitFor(() => expect(mocks.catalog).toHaveBeenCalled());
    fireEvent.click(screen.getByTestId('monitor-add'));
    fireEvent.change(screen.getByTestId('monitor-name'), { target: { value: 'Task achieved' } });
    fireEvent.change(screen.getByTestId('monitor-question'), {
      target: { value: 'Has the agent finished what was asked?' },
    });
    fireEvent.click(screen.getByTestId('monitors-save'));
    const field = screen.getByTestId('monitor-model');
    expect(await within(field).findByRole('alert')).toHaveTextContent(crew);
    expect(mocks.save).not.toHaveBeenCalled();
  });

  it('asks for the catalog naming the chat, and takes the daemon’s own reason', async () => {
    mocks.catalog.mockResolvedValue(
      parseDecisionCatalog({
        models: [
          {
            ...MODELS[0],
            unavailableReason: 'Your institution does not cover this model.',
          },
        ],
      })
    );
    await drawReady({ models: undefined });
    await waitFor(() => expect(mocks.catalog).toHaveBeenCalledWith(expect.any(AbortSignal), 's1'));
    fireEvent.click(screen.getByTestId('monitor-add'));
    const input = within(screen.getByTestId('monitor-model')).getByRole('combobox');
    fireEvent.keyDown(input, { key: 'ArrowDown' });
    expect(
      await screen.findByText('Your institution does not cover this model.')
    ).toBeInTheDocument();
  });
});

/** A monitor as `PUT /sessions/{id}/decision-settings` takes it, parsed as the dialog reads it. */
function storedMonitor(wire: Record<string, unknown>): MonitorCriterion {
  return parseDecisionSettings({
    defaultModel: null,
    monitors: [
      {
        id: 'm1',
        name: 'Watcher',
        question: { instructions: 'q', kind: { binary: {} } },
        threshold: 0.85,
        action: 'stop',
        ...wire,
      },
    ],
  })!.monitors[0];
}

const SEVERITY = {
  id: 'severity',
  name: 'Severity',
  question: {
    instructions: 'How severe is the reported relapse?',
    kind: { score: { levels: ['none', 'mild', 'severe'] } },
  },
};

/**
 * The line under a monitor's name says the answer it watches, as the daemon
 * reads `option` (`MonitorCriterion::pass_threshold`) and as the decision card
 * names the same threshold, for monitors this dialog did not make.
 */
describe('the summary of a monitor', () => {
  it('names P(no) for a yes/no monitor watching no, in any case', () => {
    expect(monitorSummary(storedMonitor({ option: 'no' }))).toMatch(/^P\(no\) ≥ 0\.85 · /);
    expect(monitorSummary(storedMonitor({ option: 'NO' }))).toMatch(/^P\(no\) ≥ 0\.85 · /);
    expect(monitorSummary(storedMonitor({ option: 'yes' }))).toMatch(/^P\(yes\) ≥ 0\.85 · /);
    expect(monitorSummary(storedMonitor({ option: null }))).toMatch(/^P\(yes\) ≥ 0\.85 · /);
  });

  it('names a score monitor’s level by its label, whether stored as the label or its index', () => {
    expect(monitorSummary(storedMonitor({ ...SEVERITY, option: 'mild' }))).toBe(
      "P(level ≥ mild) ≥ 0.85 · Stop the turn · After each step · The chat's decision model"
    );
    expect(monitorSummary(storedMonitor({ ...SEVERITY, option: '2' }))).toMatch(
      /^P\(level ≥ severe\) ≥ 0\.85 · /
    );
  });

  it('names a choice monitor’s option', () => {
    const course = storedMonitor({
      question: {
        instructions: 'Which course?',
        kind: { choice: { options: { rrms: null, spms: null } } },
      },
      option: 'spms',
      threshold: 0.7,
    });
    expect(monitorSummary(course)).toMatch(/^P\(spms\) ≥ 0\.70 · /);
  });
});

describe('a monitor the dialog did not make', () => {
  it('shows a score monitor as a score with its levels, and saves it unchanged after a rename', async () => {
    const severity = parseDecisionSettings({
      defaultModel: null,
      monitors: [{ ...SEVERITY, threshold: 0.8, action: 'notify', option: '1' }],
    })!;
    mocks.settings.mockResolvedValue({ data: severity, missing: false });
    await drawReady({ settings: severity });
    expect(screen.getByTestId('monitor-item')).toHaveTextContent('P(level ≥ mild) ≥ 0.80');
    fireEvent.click(screen.getByRole('button', { name: 'Edit Severity' }));

    const editor = screen.getByTestId('monitor-editor');
    expect(within(editor).getByRole('radio', { name: 'Score' })).toHaveAttribute(
      'aria-checked',
      'true'
    );
    expect(within(editor).getByRole('radio', { name: 'Yes or no' })).toHaveAttribute(
      'aria-checked',
      'false'
    );
    expect(
      within(screen.getByTestId('monitor-levels'))
        .getAllByRole('listitem')
        .map((item) => item.textContent)
    ).toEqual(['none', 'mild', 'severe']);
    expect(screen.getByTestId('monitor-score-fires-on')).toHaveTextContent(
      'mild or a higher level'
    );

    fireEvent.change(screen.getByTestId('monitor-name'), { target: { value: 'Relapse severity' } });
    fireEvent.click(screen.getByTestId('monitor-done'));
    fireEvent.click(screen.getByTestId('monitors-save'));
    await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1));
    const saved = (mocks.save.mock.calls[0][1] as DecisionSettings).monitors[0];
    expect(saved.name).toBe('Relapse severity');
    expect(saved.option).toBe('1');
    expect(saved.question.kind).toEqual({ type: 'score', levels: ['none', 'mild', 'severe'] });
  });

  it('converts a score monitor only when the person picks another kind', async () => {
    const severity = parseDecisionSettings({
      defaultModel: null,
      monitors: [{ ...SEVERITY, threshold: 0.8, action: 'notify', option: 'mild' }],
    })!;
    mocks.settings.mockResolvedValue({ data: severity, missing: false });
    await drawReady({ settings: severity });
    fireEvent.click(screen.getByRole('button', { name: 'Edit Severity' }));
    fireEvent.click(screen.getByRole('radio', { name: 'Yes or no' }));
    expect(screen.queryByRole('radio', { name: 'Score' })).toBeNull();
    expect(screen.queryByTestId('monitor-levels')).toBeNull();
    fireEvent.click(screen.getByTestId('monitor-done'));
    fireEvent.click(screen.getByTestId('monitors-save'));
    await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1));
    const saved = (mocks.save.mock.calls[0][1] as DecisionSettings).monitors[0];
    expect(saved.question.kind).toEqual({ type: 'binary' });
    expect(saved.option).toBeNull();
  });

  it('shows a yes/no monitor that watches no, and Fires on sets no and back to yes', async () => {
    const watchingNo = parseDecisionSettings({
      defaultModel: null,
      monitors: [
        {
          id: 'form-reached',
          name: 'Form reached',
          question: {
            instructions: 'Has the agent reached the consent form?',
            kind: { binary: {} },
          },
          threshold: 0.85,
          action: 'stop',
          option: 'no',
        },
      ],
    })!;
    mocks.settings.mockResolvedValue({ data: watchingNo, missing: false });
    await drawReady({ settings: watchingNo });
    expect(screen.getByTestId('monitor-item')).toHaveTextContent('P(no) ≥ 0.85');
    fireEvent.click(screen.getByRole('button', { name: 'Edit Form reached' }));
    const firesOn = screen.getByTestId('monitor-fires-on');
    expect(within(firesOn).getByRole('radio', { name: 'No' })).toHaveAttribute(
      'aria-checked',
      'true'
    );

    // Back to yes: no option, the default the daemon reads.
    fireEvent.click(within(firesOn).getByRole('radio', { name: 'Yes' }));
    fireEvent.click(screen.getByTestId('monitor-done'));
    expect(screen.getByTestId('monitor-item')).toHaveTextContent('P(yes) ≥ 0.85');
    fireEvent.click(screen.getByTestId('monitors-save'));
    await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1));
    expect((mocks.save.mock.calls[0][1] as DecisionSettings).monitors[0].option).toBeNull();
  });

  it('sets a new yes/no monitor to fire on no', async () => {
    await drawReady();
    fireEvent.click(screen.getByTestId('monitor-add'));
    const firesOn = screen.getByTestId('monitor-fires-on');
    expect(within(firesOn).getByRole('radio', { name: 'Yes' })).toHaveAttribute(
      'aria-checked',
      'true'
    );
    fireEvent.change(screen.getByTestId('monitor-name'), { target: { value: 'Still open' } });
    fireEvent.change(screen.getByTestId('monitor-question'), {
      target: { value: 'Has the form been submitted?' },
    });
    fireEvent.click(within(firesOn).getByRole('radio', { name: 'No' }));
    fireEvent.click(screen.getByTestId('monitor-done'));
    fireEvent.click(screen.getByTestId('monitors-save'));
    await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1));
    const saved = (mocks.save.mock.calls[0][1] as DecisionSettings).monitors[1];
    expect(saved).toMatchObject({ name: 'Still open', option: 'no' });
    expect(saved.question.kind).toEqual({ type: 'binary' });
  });
});

/**
 * The model list used to fail silently: the picker then offered only the chat's
 * decision model, with nothing said and nothing to retry.
 */
describe('when the model list cannot be read', () => {
  async function openEditor() {
    await drawReady({ models: undefined });
    await waitFor(() => expect(mocks.catalog).toHaveBeenCalled());
    fireEvent.click(screen.getByTestId('monitor-add'));
    return screen.getByTestId('monitor-model');
  }

  it('says an older service cannot list them, and offers no Retry that cannot help', async () => {
    mocks.catalog.mockResolvedValue({ models: [], sessionUnavailableReason: null, missing: true });
    const field = await openEditor();
    expect(await within(field).findByTestId('monitor-models-error')).toHaveTextContent(
      'This Biorouter service is older than the app, so it cannot list decision models. Restart Biorouter to finish updating.'
    );
    expect(within(field).queryByTestId('monitor-models-retry')).toBeNull();
  });

  it('says a refusal, with the daemon’s reason when it gave one', async () => {
    mocks.catalog.mockRejectedValue(
      new DecisionRequestError(
        'Open this chat in the app to see it.',
        403,
        'Open this chat in the app to see it.'
      )
    );
    const field = await openEditor();
    expect(await within(field).findByTestId('monitor-models-error')).toHaveTextContent(
      'Biorouter refused to list the decision models. Open this chat in the app to see it.'
    );
  });

  it('says a refusal without a reason in one sentence', async () => {
    mocks.catalog.mockRejectedValue(
      new DecisionRequestError('Decision models unavailable', 401, null)
    );
    const field = await openEditor();
    const error = await within(field).findByTestId('monitor-models-error');
    expect(error.firstChild?.textContent).toBe('Biorouter refused to list the decision models.');
  });

  it('says a failure with its message, and Retry reads again keeping the monitor being edited', async () => {
    mocks.catalog
      .mockRejectedValueOnce(new DecisionRequestError('Decision models unavailable', 500, null))
      .mockResolvedValue({ models: MODELS, sessionUnavailableReason: null, missing: false });
    const field = await openEditor();
    fireEvent.change(screen.getByTestId('monitor-name'), { target: { value: 'Task achieved' } });
    fireEvent.change(screen.getByTestId('monitor-question'), {
      target: { value: 'Has the agent finished what was asked?' },
    });
    expect(await within(field).findByTestId('monitor-models-error')).toHaveTextContent(
      'Could not load the decision models. Biorouter answered with status 500.'
    );

    fireEvent.click(within(field).getByRole('button', { name: 'Retry' }));
    await waitFor(() => expect(mocks.catalog).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(within(field).queryByTestId('monitor-models-error')).toBeNull());
    expect(screen.getByTestId('monitor-name')).toHaveValue('Task achieved');
    expect(screen.getByTestId('monitor-question')).toHaveValue(
      'Has the agent finished what was asked?'
    );
    // The models the retry read are offered now.
    fireEvent.keyDown(within(field).getByRole('combobox'), { key: 'ArrowDown' });
    expect(await screen.findByText('tev1:0.8b · Private · this computer')).toBeInTheDocument();
  });

  it('shows no old failure while a retry is still reading', async () => {
    mocks.catalog
      .mockRejectedValueOnce(new DecisionRequestError('Decision models unavailable', 500, null))
      .mockReturnValueOnce(new Promise(() => {}));
    const field = await openEditor();
    expect(await within(field).findByTestId('monitor-models-error')).toBeInTheDocument();

    fireEvent.click(within(field).getByRole('button', { name: 'Retry' }));
    await waitFor(() => expect(mocks.catalog).toHaveBeenCalledTimes(2));
    expect(within(field).queryByTestId('monitor-models-error')).toBeNull();
  });

  it('says a request that never reached the service with its message', async () => {
    mocks.catalog.mockRejectedValue(new TypeError('Failed to fetch'));
    const field = await openEditor();
    expect(await within(field).findByTestId('monitor-models-error')).toHaveTextContent(
      'Could not load the decision models. Failed to fetch'
    );
    expect(within(field).getByRole('button', { name: 'Retry' })).toBeInTheDocument();
  });
});
