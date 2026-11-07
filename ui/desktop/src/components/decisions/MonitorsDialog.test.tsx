import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  DecisionRequestError,
  decisionSettingsToWire,
  parseDecisionCatalog,
  parseDecisionModelEntries,
  parseDecisionSettings,
  type DecisionInstallStatus,
  type DecisionSettings,
  type MonitorCriterion,
} from '../../utils/sessionDecisions';
import {
  MonitorsDialog,
  THRESHOLD_STEPS,
  monitorSlug,
  monitorSummary,
  newMonitor,
  rearmForThreshold,
  thresholdStepsFor,
  validateMonitor,
} from './MonitorsDialog';

const mocks = vi.hoisted(() => ({
  save: vi.fn(),
  catalog: vi.fn(),
  settings: vi.fn(),
  install: vi.fn(),
  startInstall: vi.fn(),
}));
vi.mock('../../utils/sessionDecisions', async (original) => ({
  ...(await original<typeof import('../../utils/sessionDecisions')>()),
  saveDecisionSettings: mocks.save,
  fetchDecisionCatalog: mocks.catalog,
  fetchDecisionSettings: mocks.settings,
  fetchDecisionInstall: mocks.install,
  startDecisionInstall: mocks.startInstall,
}));

/** What `GET /decisions/models/install` answers on a machine with Ollama and no decision model. */
const INSTALL_STATUS: DecisionInstallStatus = {
  offers: [
    {
      model: 'tev1:0.8b',
      displayName: 'tev1:0.8b',
      approxSize: 'about 1.5 GB',
      note: 'small and fast',
      isDefault: true,
      installed: false,
    },
  ],
  ollama: {
    baseUrl: 'http://127.0.0.1:11434/',
    state: 'running',
    version: '0.40.15',
    message: null,
  },
  job: null,
};

/** The daemon's Crew sentence for a hosted model (`CREW_SCOPED_REFUSAL`). */
const CREW_HOSTED =
  'This chat works under a Crew grant, which lets its content reach only the model it was granted with and decision models that run on this computer, so a decision model hosted elsewhere cannot read it. Choose one that runs on this computer, such as tev1:0.8b.';

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
  mocks.install.mockReset().mockResolvedValue({ data: INSTALL_STATUS, missing: false });
  mocks.startInstall.mockReset();
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

/** A new monitor with a name and a question, so Done takes it. */
function fillNewMonitor(name: string, question: string) {
  fireEvent.click(screen.getByTestId('monitor-add'));
  fireEvent.change(screen.getByTestId('monitor-name'), { target: { value: name } });
  fireEvent.change(screen.getByTestId('monitor-question'), { target: { value: question } });
}

function thresholdThumb() {
  return within(screen.getByTestId('monitor-threshold')).getByRole('slider');
}

async function savedMonitors(): Promise<MonitorCriterion[]> {
  await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1));
  return (mocks.save.mock.calls[0][1] as DecisionSettings).monitors;
}

/**
 * Item 4. A monitor may fire below its threshold, may pause the turn when its
 * check fails, and may use any threshold from 0.01: the CLI and HTTP could set
 * all three, and the dialog could show none of them.
 */
describe('which side fires, low thresholds, and a failing check', () => {
  it('offers thresholds from 0.01 to 0.99 and keeps a stored threshold that is not a stop', async () => {
    expect(THRESHOLD_STEPS).toHaveLength(21);
    expect(THRESHOLD_STEPS[0].value).toBe('0.01');
    expect(THRESHOLD_STEPS.map((step) => step.value)).toContain('0.30');
    expect(THRESHOLD_STEPS[20].value).toBe('0.99');
    // A threshold set elsewhere is a stop of its own, in order.
    const steps = thresholdStepsFor(0.33).map((step) => step.value);
    expect(steps).toHaveLength(22);
    expect(steps.slice(6, 9)).toEqual(['0.30', '0.33', '0.35']);
    expect(thresholdStepsFor(0.3)).toBe(THRESHOLD_STEPS);

    const odd: DecisionSettings = {
      ...STORED,
      monitors: [{ ...STORED.monitors[0], threshold: 0.33 }],
    };
    mocks.settings.mockResolvedValue({ data: odd, missing: false });
    await drawReady({ settings: odd });
    fireEvent.click(screen.getByRole('button', { name: 'Edit Form reached' }));
    // Shown where it is, not snapped to the nearest stop.
    expect(thresholdThumb()).toHaveAttribute('aria-valuetext', '0.33');
    fireEvent.click(screen.getByTestId('monitor-done'));
    fireEvent.click(screen.getByTestId('monitors-save'));
    expect((await savedMonitors())[0].threshold).toBe(0.33);
  });

  it('keeps a stored threshold that is not a stop as a stop for the whole edit', async () => {
    const odd: DecisionSettings = {
      ...STORED,
      monitors: [{ ...STORED.monitors[0], threshold: 0.33 }],
    };
    mocks.settings.mockResolvedValue({ data: odd, missing: false });
    await drawReady({ settings: odd });
    fireEvent.click(screen.getByRole('button', { name: 'Edit Form reached' }));
    // 22 stops (0.33 among them), whatever the thumb is on.
    expect(thresholdThumb()).toHaveAttribute('aria-valuemax', '21');

    // Keys: off 0.33 and back again.
    fireEvent.keyDown(thresholdThumb(), { key: 'ArrowRight' });
    expect(thresholdThumb()).toHaveAttribute('aria-valuetext', '0.35');
    expect(thresholdThumb()).toHaveAttribute('aria-valuemax', '21');
    fireEvent.keyDown(thresholdThumb(), { key: 'ArrowLeft' });
    expect(thresholdThumb()).toHaveAttribute('aria-valuetext', '0.33');

    // A drag: a 210px rail puts stop i at x = 10i. Rebuilt from the draft, the
    // scale shrank to 21 stops at 0.35, and the next stop (0.40) was skipped.
    const root = screen.getByTestId('monitor-threshold');
    const rail = root.querySelector('[data-slot="slider-rail"]') as HTMLElement;
    vi.spyOn(rail, 'getBoundingClientRect').mockReturnValue({
      left: 0,
      right: 210,
      width: 210,
      top: 0,
      bottom: 24,
      height: 24,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    } as DOMRect);
    const pointer = { pointerId: 1, pointerType: 'mouse' };
    fireEvent.pointerDown(root, { ...pointer, clientX: 70, button: 0 });
    expect(thresholdThumb()).toHaveAttribute('aria-valuetext', '0.33');
    fireEvent.pointerMove(root, { ...pointer, clientX: 80 });
    expect(thresholdThumb()).toHaveAttribute('aria-valuetext', '0.35');
    fireEvent.pointerMove(root, { ...pointer, clientX: 90 });
    expect(thresholdThumb()).toHaveAttribute('aria-valuetext', '0.40');
    fireEvent.pointerMove(root, { ...pointer, clientX: 70 });
    expect(thresholdThumb()).toHaveAttribute('aria-valuetext', '0.33');
    fireEvent.pointerUp(root, { ...pointer, clientX: 70 });

    fireEvent.click(screen.getByTestId('monitor-done'));
    fireEvent.click(screen.getByTestId('monitors-save'));
    expect((await savedMonitors())[0].threshold).toBe(0.33);
  });

  it('reaches 0.01 at the low end of the slider', async () => {
    await drawReady();
    fillNewMonitor('Quiet', 'Has the agent asked a question?');
    fireEvent.keyDown(thresholdThumb(), { key: 'Home' });
    expect(thresholdThumb()).toHaveAttribute('aria-valuetext', '0.01');
    fireEvent.click(screen.getByTestId('monitor-done'));
    fireEvent.click(screen.getByTestId('monitors-save'));
    expect((await savedMonitors())[1].threshold).toBe(0.01);
  });

  it('saves a monitor that fires below its threshold and says P(yes) < 0.30', async () => {
    await drawReady();
    fillNewMonitor('Banana', 'Has the assistant written the word banana?');
    const direction = screen.getByTestId('monitor-direction');
    expect(within(direction).getByRole('radio', { name: 'At or above' })).toHaveAttribute(
      'aria-checked',
      'true'
    );
    expect(
      screen.getByText('Fires when the probability of the answer it watches is at or above this.')
    ).toBeInTheDocument();
    fireEvent.click(within(direction).getByRole('radio', { name: 'Below' }));
    expect(
      screen.getByText('Fires when the probability of the answer it watches falls below this.')
    ).toBeInTheDocument();
    fireEvent.keyDown(thresholdThumb(), { key: 'Home' });
    for (let i = 0; i < 6; i += 1) fireEvent.keyDown(thresholdThumb(), { key: 'ArrowRight' });
    expect(thresholdThumb()).toHaveAttribute('aria-valuetext', '0.30');
    fireEvent.click(screen.getByTestId('monitor-done'));
    expect(screen.getAllByTestId('monitor-item')[1]).toHaveTextContent('P(yes) < 0.30 · ');
    fireEvent.click(screen.getByTestId('monitors-save'));
    const saved = (await savedMonitors())[1];
    expect(saved).toMatchObject({ name: 'Banana', direction: 'below', threshold: 0.3 });
    expect(decisionSettingsToWire({ defaultModel: null, monitors: [saved] }).monitors).toEqual([
      expect.objectContaining({ direction: 'below', threshold: 0.3 }),
    ]);
  });

  it('says a choice monitor that fires below on an option as P(option) < p', () => {
    const course = parseDecisionSettings({
      defaultModel: null,
      monitors: [
        {
          id: 'course',
          name: 'Course',
          question: {
            instructions: 'Which course?',
            kind: { choice: { options: { rrms: null, spms: null } } },
          },
          option: 'spms',
          threshold: 0.2,
          direction: 'below',
        },
      ],
    })!.monitors[0];
    expect(monitorSummary(course)).toMatch(/^P\(spms\) < 0\.20 · /);
  });

  it('saves onError pause and says pauses if a check fails', async () => {
    await drawReady();
    fillNewMonitor('Signed', 'Has the form been signed?');
    const onError = screen.getByTestId('monitor-on-error');
    expect(within(onError).getByRole('radio', { name: 'Notify me' })).toHaveAttribute(
      'aria-checked',
      'true'
    );
    expect(
      screen.getByText(
        'Pause stops the turn whenever the decision model cannot answer, so a safety rule never goes unchecked.'
      )
    ).toBeInTheDocument();
    fireEvent.click(within(onError).getByRole('radio', { name: 'Pause the turn' }));
    fireEvent.click(screen.getByTestId('monitor-done'));
    expect(screen.getAllByTestId('monitor-item')[1]).toHaveTextContent('pauses if a check fails');
    fireEvent.click(screen.getByTestId('monitors-save'));
    expect((await savedMonitors())[1].onError).toBe('pause');
    // A new monitor still starts on notify, as the daemon and the CLI do.
    expect(newMonitor().onError).toBe('notify');
  });

  it('switching the direction clears a re-arm level', async () => {
    const latched: DecisionSettings = {
      ...STORED,
      monitors: [{ ...STORED.monitors[0], rearmBelow: 0.7 }],
    };
    mocks.settings.mockResolvedValue({ data: latched, missing: false });
    await drawReady({ settings: latched });
    fireEvent.click(screen.getByRole('button', { name: 'Edit Form reached' }));
    expect(screen.getByTestId('monitor-stops-once')).toHaveTextContent(
      'Stops once, then stays quiet until P falls below 0.70'
    );
    fireEvent.click(
      within(screen.getByTestId('monitor-direction')).getByRole('radio', { name: 'Below' })
    );
    expect(screen.queryByTestId('monitor-stops-once')).toBeNull();
    fireEvent.click(screen.getByTestId('monitor-done'));
    fireEvent.click(screen.getByTestId('monitors-save'));
    expect((await savedMonitors())[0]).toMatchObject({
      direction: 'below',
      rearmBelow: null,
      rearmAbove: null,
    });
  });

  it('keeps a re-arm level made over HTTP when the monitor is edited', async () => {
    const latched: DecisionSettings = {
      ...STORED,
      monitors: [{ ...STORED.monitors[0], rearmBelow: 0.7 }],
    };
    mocks.settings.mockResolvedValue({ data: latched, missing: false });
    await drawReady({ settings: latched });
    fireEvent.click(screen.getByRole('button', { name: 'Edit Form reached' }));
    fireEvent.change(screen.getByTestId('monitor-name'), { target: { value: 'Consent form' } });
    fireEvent.click(screen.getByTestId('monitor-done'));
    fireEvent.click(screen.getByTestId('monitors-save'));
    expect((await savedMonitors())[0]).toMatchObject({ name: 'Consent form', rearmBelow: 0.7 });
  });

  // The daemon refuses a re-arm level past the threshold (`settings.rs`'s
  // `validate`), and the dialog has no field for the level, so a threshold moved
  // past one used to make a Save the dialog could not fix.
  it('holds a kept re-arm level at a threshold moved below it, and gives it back', async () => {
    const latched: DecisionSettings = {
      ...STORED,
      monitors: [{ ...STORED.monitors[0], threshold: 0.8, rearmBelow: 0.6 }],
    };
    mocks.settings.mockResolvedValue({ data: latched, missing: false });
    await drawReady({ settings: latched });
    fireEvent.click(screen.getByRole('button', { name: 'Edit Form reached' }));
    const press = (key: string, times: number) => {
      for (let i = 0; i < times; i += 1) fireEvent.keyDown(thresholdThumb(), { key });
    };
    // 0.80 to 0.60: the level is not passed yet.
    press('ArrowLeft', 4);
    expect(screen.getByTestId('monitor-stops-once')).toHaveTextContent(
      'Stops once, then stays quiet until P falls below 0.60'
    );
    // On to 0.40, past it: the level stays at the threshold.
    press('ArrowLeft', 4);
    expect(thresholdThumb()).toHaveAttribute('aria-valuetext', '0.40');
    expect(screen.getByTestId('monitor-stops-once')).toHaveTextContent(
      'Stops once, then stays quiet until P falls below 0.40'
    );
    // Back to 0.80: the level the monitor opened with comes back.
    press('ArrowRight', 8);
    expect(screen.getByTestId('monitor-stops-once')).toHaveTextContent(
      'Stops once, then stays quiet until P falls below 0.60'
    );
    press('ArrowLeft', 8);
    fireEvent.click(screen.getByTestId('monitor-done'));
    fireEvent.click(screen.getByTestId('monitors-save'));
    const saved = (await savedMonitors())[0];
    expect(saved).toMatchObject({ threshold: 0.4, rearmBelow: 0.4, action: 'stop' });
    // What the daemon checks: a level no higher than the threshold.
    expect(saved.rearmBelow!).toBeLessThanOrEqual(saved.threshold);
  });

  it('holds a below monitor’s re-arm level at a threshold moved above it', async () => {
    const fromCli = parseDecisionSettings({
      defaultModel: null,
      monitors: [
        {
          id: 'banana',
          name: 'Banana',
          question: { instructions: 'Has the assistant written banana?', kind: { binary: {} } },
          threshold: 0.3,
          direction: 'below',
          rearmAbove: 0.5,
          action: 'stop',
        },
      ],
    })!;
    mocks.settings.mockResolvedValue({ data: fromCli, missing: false });
    await drawReady({ settings: fromCli });
    fireEvent.click(screen.getByRole('button', { name: 'Edit Banana' }));
    for (let i = 0; i < 8; i += 1) fireEvent.keyDown(thresholdThumb(), { key: 'ArrowRight' });
    expect(thresholdThumb()).toHaveAttribute('aria-valuetext', '0.70');
    expect(screen.getByTestId('monitor-stops-once')).toHaveTextContent(
      'Stops once, then stays quiet until P rises to 0.70'
    );
    fireEvent.click(screen.getByTestId('monitor-done'));
    fireEvent.click(screen.getByTestId('monitors-save'));
    const saved = (await savedMonitors())[0];
    expect(saved).toMatchObject({ direction: 'below', threshold: 0.7, rearmAbove: 0.7 });
    // What the daemon checks: a level no lower than the threshold.
    expect(saved.rearmAbove!).toBeGreaterThanOrEqual(saved.threshold);
  });

  it('gives a threshold move no level when the monitor has none, or the direction switch cleared it', () => {
    const opened: MonitorCriterion = { ...newMonitor(), threshold: 0.8, rearmBelow: 0.6 };
    expect(rearmForThreshold({ ...newMonitor(), threshold: 0.8 }, 0.4, null)).toEqual({
      rearmBelow: null,
      rearmAbove: null,
    });
    // Switched to below and back: the opened level does not come back with a drag.
    expect(rearmForThreshold({ ...opened, rearmBelow: null }, 0.4, opened)).toEqual({
      rearmBelow: null,
      rearmAbove: null,
    });
    // A level the threshold does not pass is left where it is.
    expect(rearmForThreshold(opened, 0.9, opened)).toEqual({ rearmBelow: 0.6, rearmAbove: null });
  });

  it('a below monitor read from the daemon is saved back as below', async () => {
    const fromCli = parseDecisionSettings({
      defaultModel: null,
      monitors: [
        {
          id: 'banana',
          name: 'Banana',
          question: {
            instructions: 'Has the assistant written the word banana?',
            kind: { binary: {} },
          },
          model: { provider: 'ollama_decision', model: 'tev1:0.8b' },
          threshold: 0.3,
          direction: 'below',
          rearmAbove: 0.5,
          action: 'stop',
          onError: 'pause',
        },
      ],
    })!;
    mocks.settings.mockResolvedValue({ data: fromCli, missing: false });
    await drawReady({ settings: fromCli });
    expect(screen.getByTestId('monitor-item')).toHaveTextContent(
      'P(yes) < 0.30 · Stop the turn · After each step · tev1:0.8b · pauses if a check fails'
    );
    fireEvent.click(screen.getByRole('button', { name: 'Edit Banana' }));
    expect(
      within(screen.getByTestId('monitor-direction')).getByRole('radio', { name: 'Below' })
    ).toHaveAttribute('aria-checked', 'true');
    expect(
      within(screen.getByTestId('monitor-on-error')).getByRole('radio', { name: 'Pause the turn' })
    ).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByTestId('monitor-stops-once')).toHaveTextContent(
      'Stops once, then stays quiet until P rises to 0.50'
    );
    fireEvent.click(screen.getByTestId('monitor-done'));
    fireEvent.click(screen.getByTestId('monitors-save'));
    const saved = (await savedMonitors())[0];
    // What is written is what was read: no key lost, none changed.
    expect(decisionSettingsToWire({ defaultModel: null, monitors: [saved] })).toEqual(
      decisionSettingsToWire(fromCli)
    );
    expect(saved).toMatchObject({ direction: 'below', rearmAbove: 0.5, onError: 'pause' });
  });
});

/** Item A. Who set a monitor, and the subagents a person's monitor also watches. */
describe('owners and subagents', () => {
  const ownedSettings = () =>
    parseDecisionSettings({
      defaultModel: null,
      monitors: [
        {
          id: 'drift',
          name: 'Drift',
          question: { instructions: 'Has the subagent left its task?', kind: { binary: {} } },
          threshold: 0.8,
          action: 'stop',
          setBy: 'agent',
          setBySession: 'parent-1',
        },
        {
          id: 'secrets',
          name: 'Secrets',
          question: { instructions: 'Is a secret in the output?', kind: { binary: {} } },
          threshold: 0.9,
          action: 'stop',
          onError: 'pause',
          inheritedFrom: 'parent-1',
        },
      ],
    })!;

  it('round trips owner fields', async () => {
    const owned = ownedSettings();
    mocks.settings.mockResolvedValue({ data: owned, missing: false });
    await drawReady({ settings: owned, chatTier: 'public' });
    const [drift, secrets] = screen.getAllByTestId('monitor-item');
    expect(within(drift).getByTestId('monitor-set-by-agent')).toHaveTextContent('Set by the agent');
    expect(within(drift).queryByTestId('monitor-inherited')).toBeNull();
    expect(within(secrets).getByTestId('monitor-inherited')).toHaveTextContent(
      'From the chat that started this subagent'
    );
    fireEvent.click(screen.getByTestId('monitors-save'));
    const saved = await savedMonitors();
    expect(decisionSettingsToWire({ defaultModel: null, monitors: saved })).toEqual(
      decisionSettingsToWire(owned)
    );
    expect(saved[0]).toMatchObject({ setBy: 'agent', setBySession: 'parent-1' });
    expect(saved[1]).toMatchObject({ setBy: 'person', inheritedFrom: 'parent-1' });
  });

  it('makes an agent’s monitor the person’s once the person changes it, not when only opened', async () => {
    const owned = ownedSettings();
    mocks.settings.mockResolvedValue({ data: owned, missing: false });
    await drawReady({ settings: owned, chatTier: 'public' });
    // Opened and closed with nothing changed: still the agent's.
    fireEvent.click(screen.getByRole('button', { name: 'Edit Drift' }));
    fireEvent.click(screen.getByTestId('monitor-done'));
    expect(screen.getByTestId('monitor-set-by-agent')).toBeInTheDocument();
    // Changed: the person's.
    fireEvent.click(screen.getByRole('button', { name: 'Edit Drift' }));
    fireEvent.change(screen.getByTestId('monitor-name'), { target: { value: 'Off task' } });
    fireEvent.click(screen.getByTestId('monitor-done'));
    expect(screen.queryByTestId('monitor-set-by-agent')).toBeNull();
    fireEvent.click(screen.getByTestId('monitors-save'));
    const saved = await savedMonitors();
    expect(saved[0]).toMatchObject({ name: 'Off task', setBy: 'person', setBySession: null });
    expect(decisionSettingsToWire({ defaultModel: null, monitors: saved }).monitors).toEqual([
      expect.not.objectContaining({ setBy: expect.anything() }),
      expect.objectContaining({ inheritedFrom: 'parent-1' }),
    ]);
  });

  it('makes an agent’s monitor the person’s when the person switches it off', async () => {
    const owned = ownedSettings();
    mocks.settings.mockResolvedValue({ data: owned, missing: false });
    await drawReady({ settings: owned, chatTier: 'public' });
    fireEvent.click(screen.getByRole('switch', { name: 'Drift on' }));
    fireEvent.click(screen.getByTestId('monitors-save'));
    expect((await savedMonitors())[0]).toMatchObject({
      enabled: false,
      setBy: 'person',
      setBySession: null,
    });
  });

  it('hides the subagent checkbox on a subagent chat', async () => {
    // Shown on a chat that starts subagents...
    await drawReady();
    fireEvent.click(screen.getByTestId('monitor-add'));
    expect(screen.getByTestId('monitor-subagents')).toBeInTheDocument();
    cleanup();
    // ...and not on a subagent's own chat, whose monitors cannot reach further.
    await drawReady({ isSubagentChat: true });
    fireEvent.click(screen.getByTestId('monitor-add'));
    expect(screen.queryByTestId('monitor-subagents')).toBeNull();
  });

  it('reads a subagent chat from its monitors when the caller does not say', async () => {
    const owned = ownedSettings();
    mocks.settings.mockResolvedValue({ data: owned, missing: false });
    await drawReady({ settings: owned, chatTier: 'public' });
    fireEvent.click(screen.getByTestId('monitor-add'));
    expect(screen.queryByTestId('monitor-subagents')).toBeNull();
  });

  it('on a chat that starts subagents, sets a monitor to also watch them', async () => {
    await drawReady();
    fillNewMonitor('Secrets', 'Is a secret in the output?');
    const box = screen.getByTestId('monitor-subagents');
    expect(box).toHaveTextContent('Also watch the subagents this chat starts');
    fireEvent.click(within(box).getByRole('checkbox'));
    fireEvent.click(screen.getByTestId('monitor-done'));
    fireEvent.click(screen.getByTestId('monitors-save'));
    expect((await savedMonitors())[1].applyToSubagents).toBe(true);
  });
});

/** Item 6. The daemon checks a save; the dialog no longer refuses what the daemon would take. */
describe('what Save checks, and the daemon’s refusal', () => {
  it('does not hold Save back for a monitor that is off, whatever its model', async () => {
    const off = parseDecisionSettings({
      defaultModel: null,
      monitors: [
        {
          id: 'hosted',
          name: 'Hosted',
          question: { instructions: 'Is it done?', kind: { binary: {} } },
          threshold: 0.8,
          model: { provider: 'baseten_decision', model: 'decision-models/baseten-joey-2b-latest' },
          enabled: false,
        },
      ],
    })!;
    mocks.settings.mockResolvedValue({ data: off, missing: false });
    // A private chat, so the model is one it may not use.
    await drawReady({ settings: off, chatTier: 'private' });
    fireEvent.click(screen.getByTestId('monitors-save'));
    expect((await savedMonitors())[0]).toMatchObject({ id: 'hosted', enabled: false });
    expect(screen.queryByTestId('monitor-editor')).toBeNull();
  });

  it('still holds Save back, at the field, when that monitor is switched on', async () => {
    const off = parseDecisionSettings({
      defaultModel: null,
      monitors: [
        {
          id: 'hosted',
          name: 'Hosted',
          question: { instructions: 'Is it done?', kind: { binary: {} } },
          threshold: 0.8,
          model: { provider: 'baseten_decision', model: 'decision-models/baseten-joey-2b-latest' },
          enabled: false,
        },
      ],
    })!;
    mocks.settings.mockResolvedValue({ data: off, missing: false });
    await drawReady({ settings: off, chatTier: 'private' });
    fireEvent.click(screen.getByRole('switch', { name: 'Hosted on' }));
    fireEvent.click(screen.getByTestId('monitors-save'));
    expect(await within(screen.getByTestId('monitor-model')).findByRole('alert')).toHaveTextContent(
      'This chat is private, so a public model may not read it.'
    );
    expect(mocks.save).not.toHaveBeenCalled();
  });

  it('shows the daemon sentence when a save is refused (422)', async () => {
    const sentence =
      'Nothing was saved: monitor "Task achieved" could not run. No decision model this private chat may use is available.';
    // Only a hosted model is listed: nothing this private chat may use.
    mocks.catalog.mockResolvedValue(
      parseDecisionCatalog({ models: [MODELS[1]], sessionUnavailableReason: null })
    );
    mocks.save.mockRejectedValue(new DecisionRequestError(sentence, 422, sentence));
    const { onOpenChange } = await drawReady({ models: undefined });
    await waitFor(() => expect(mocks.catalog).toHaveBeenCalled());
    fillNewMonitor('Task achieved', 'Has the agent finished what was asked?');
    // Said under Model, as a note: never a refusal that holds Save back.
    const note = await within(screen.getByTestId('monitor-model')).findByTestId(
      'monitor-no-usable-model'
    );
    expect(note).toHaveTextContent(
      'No decision model this chat may use is installed, so this monitor cannot run until one is.'
    );
    expect(within(screen.getByTestId('monitor-model')).queryByRole('alert')).toBeNull();
    fireEvent.click(screen.getByTestId('monitors-save'));
    expect(await screen.findByTestId('monitors-save-error')).toHaveTextContent(sentence);
    expect(mocks.save).toHaveBeenCalledTimes(1);
    expect(onOpenChange).not.toHaveBeenCalledWith(false);
  });
});

/** Item 5. A chat with no decision model it may use is offered one to install. */
describe('the install panel', () => {
  it('offers nothing to install while a model this chat may use is listed', async () => {
    await drawReady({ models: undefined });
    await waitFor(() => expect(mocks.catalog).toHaveBeenCalledTimes(1));
    expect(screen.queryByTestId('decision-model-install')).toBeNull();
    expect(mocks.install).not.toHaveBeenCalled();
  });

  it('shows the install panel only when nothing is usable and re-reads the catalog after install', async () => {
    const local = parseDecisionModelEntries([MODELS[0]])!;
    mocks.catalog
      .mockResolvedValueOnce(parseDecisionCatalog({ models: [MODELS[1]] }))
      .mockResolvedValue({ models: [...local, MODELS[1]], sessionUnavailableReason: null });
    mocks.startInstall.mockResolvedValue({
      ...INSTALL_STATUS,
      job: {
        model: 'tev1:0.8b',
        state: 'installed',
        completedBytes: 1,
        totalBytes: 1,
        status: 'success',
        error: null,
      },
    });
    await drawReady({ models: undefined, chatTier: 'private' });
    const panel = await screen.findByTestId('decision-model-install');
    expect(panel).toHaveTextContent(
      'This chat is private, so only a decision model on this computer may read it, and none is installed.'
    );
    expect(panel).toHaveTextContent(
      'tev1:0.8b is a small decision model (about 1.5 GB) that runs in Ollama on this computer.'
    );
    // Nothing is downloaded until the person asks.
    expect(mocks.startInstall).not.toHaveBeenCalled();
    fireEvent.click(await within(panel).findByRole('button', { name: 'Install tev1:0.8b' }));
    expect(mocks.startInstall).toHaveBeenCalledWith('tev1:0.8b');
    await waitFor(() => expect(mocks.catalog).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.queryByTestId('decision-model-install')).toBeNull());
    // The model just installed is offered, and may be chosen.
    fireEvent.click(screen.getByTestId('monitor-add'));
    fireEvent.keyDown(within(screen.getByTestId('monitor-model')).getByRole('combobox'), {
      key: 'ArrowDown',
    });
    const row = await screen.findByText('tev1:0.8b · Private · this computer');
    expect(row.closest('[role="option"]')).not.toHaveAttribute('aria-disabled', 'true');
  });

  it('a Crew chat enables local models and disables hosted ones with the sentence', async () => {
    // Only hosted models: none may read it, and the install panel says the Crew rule.
    mocks.catalog.mockResolvedValue(
      parseDecisionCatalog({
        models: [{ ...MODELS[1], unavailableReason: CREW_HOSTED }],
        sessionUnavailableReason: null,
      })
    );
    await drawReady({ models: undefined, chatTier: 'public' });
    expect(await screen.findByTestId('decision-model-install')).toHaveTextContent(
      'This chat works under a Crew grant, so only a decision model on this computer may read it, and none is installed.'
    );
    cleanup();

    // A local model too: it is enabled, the hosted one is not, and nothing is offered.
    mocks.catalog.mockResolvedValue(
      parseDecisionCatalog({
        models: [MODELS[0], { ...MODELS[1], unavailableReason: CREW_HOSTED }],
        sessionUnavailableReason: null,
      })
    );
    await drawReady({ models: undefined, chatTier: 'public' });
    await waitFor(() => expect(mocks.catalog).toHaveBeenCalled());
    fireEvent.click(screen.getByTestId('monitor-add'));
    fireEvent.keyDown(within(screen.getByTestId('monitor-model')).getByRole('combobox'), {
      key: 'ArrowDown',
    });
    const hosted = (await screen.findByText('joey-1 · Public')).closest('[role="option"]');
    expect(hosted).toHaveAttribute('aria-disabled', 'true');
    expect(screen.getByText(CREW_HOSTED)).toBeInTheDocument();
    const local = screen
      .getByText('tev1:0.8b · Private · this computer')
      .closest('[role="option"]');
    expect(local).not.toHaveAttribute('aria-disabled', 'true');
    const chatDefault = screen
      .getAllByText("The chat's decision model")
      .map((node) => node.closest('[role="option"]'))
      .find(Boolean);
    expect(chatDefault).not.toHaveAttribute('aria-disabled', 'true');
    // A local model is there, so nothing is offered to install.
    expect(screen.queryByTestId('decision-model-install')).toBeNull();
  });

  it('offers no install when the chat may use no model at all, an installed one included', async () => {
    const ended = 'This chat’s Crew grant has ended, so no decision model may read it.';
    mocks.catalog.mockResolvedValue(
      parseDecisionCatalog({
        models: [{ ...MODELS[1], unavailableReason: ended }],
        sessionUnavailableReason: ended,
      })
    );
    await drawReady({ models: undefined, chatTier: 'public' });
    await waitFor(() => expect(mocks.catalog).toHaveBeenCalled());
    expect(screen.queryByTestId('decision-model-install')).toBeNull();
    expect(mocks.install).not.toHaveBeenCalled();
  });
});
