import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
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
  MONITORS_COPY,
  MonitorsDialog,
  THRESHOLD_STEPS,
  monitorPeek,
  monitorRowLine,
  monitorSlug,
  newMonitor,
  rearmForThreshold,
  stopOnlyOnce,
  thresholdStepsFor,
  usesHiddenSettings,
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
    contextTokens: 2048,
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

const CLEF = parseDecisionModelEntries([
  {
    provider: 'ollama_decision',
    model: 'clef-flash:latest',
    displayName: 'clef-flash:latest',
    contextTokens: 16384,
    kinds: ['binary', 'choice', 'score'],
    tier: 'private',
    adapter: 'system_one',
    local: true,
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

/** A monitor as the daemon sends it, parsed as the dialog reads it. */
function wireMonitor(wire: Record<string, unknown>): MonitorCriterion {
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

/** The monitors as the wire writes them, to build a variant from. */
const wireMonitors = (settings: DecisionSettings) =>
  decisionSettingsToWire(settings).monitors as Record<string, unknown>[];

const settingsWith = (...monitors: MonitorCriterion[]): DecisionSettings => ({
  defaultModel: null,
  monitors,
});

const AGENT_C = wireMonitor({
  id: 'agent-c',
  name: 'Agent watch',
  question: { instructions: 'Is the agent stuck?', kind: { binary: {} } },
  setBy: 'agent',
  setBySession: 'parent-1',
});

function draw(overrides: Partial<Parameters<typeof MonitorsDialog>[0]> = {}) {
  const onOpenChange = vi.fn();
  const onSaved = vi.fn();
  const props = {
    open: true,
    onOpenChange,
    sessionId: 's1',
    chatTier: 'private' as const,
    settings: STORED,
    onSaved,
    models: MODELS,
    ...overrides,
  };
  const view = render(<MonitorsDialog {...props} />);
  return {
    onOpenChange,
    onSaved,
    rerender: (more: Partial<Parameters<typeof MonitorsDialog>[0]>) =>
      view.rerender(<MonitorsDialog {...props} {...more} />),
  };
}

/** Every open reads what is stored now; changes wait for that answer. */
async function drawReady(overrides: Partial<Parameters<typeof MonitorsDialog>[0]> = {}) {
  const handles = draw(overrides);
  await waitFor(() => expect(screen.queryByTestId('monitors-loading')).toBeNull());
  return handles;
}

/** Opens a row's `⋯` and picks an item. */
async function rowAction(name: string, item: 'Edit' | 'Remove') {
  const user = userEvent.setup();
  await user.click(screen.getByRole('button', { name: `More actions for ${name}` }));
  await user.click(await screen.findByRole('menuitem', { name: item }));
}

const editor = () => screen.getByTestId('monitor-editor');

function openMoreOptions() {
  const trigger = within(editor()).getByRole('button', { name: MONITORS_COPY.moreOptions });
  if (trigger.getAttribute('aria-expanded') !== 'true') fireEvent.click(trigger);
}

function moreOptionsOpen(): boolean {
  return (
    within(editor())
      .getByRole('button', { name: MONITORS_COPY.moreOptions })
      .getAttribute('aria-expanded') === 'true'
  );
}

/** A new monitor with a name and a question, so Save takes it. */
function fillNewMonitor(name: string, question: string) {
  fireEvent.click(screen.getByTestId('monitor-add'));
  fireEvent.change(screen.getByTestId('monitor-name'), { target: { value: name } });
  fireEvent.change(screen.getByTestId('monitor-question'), { target: { value: question } });
}

const saveEditor = () => fireEvent.click(screen.getByTestId('monitor-save'));

function thresholdThumb() {
  return within(screen.getByTestId('monitor-threshold')).getByRole('slider');
}

/** The monitors the Nth PUT wrote (1-based). */
async function savedMonitors(nth = 1): Promise<MonitorCriterion[]> {
  await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(nth));
  return (mocks.save.mock.calls[nth - 1][1] as DecisionSettings).monitors;
}

async function openModelMenu() {
  const input = within(screen.getByTestId('monitor-model')).getByRole('combobox');
  fireEvent.keyDown(input, { key: 'ArrowDown' });
  await screen.findAllByTestId('decision-model-option');
}

/** The menu's rows by name, each as `{ text, visible, disabled }`. */
function modelRows() {
  return screen.getAllByTestId('decision-model-option').map((node) => {
    const visible = node.cloneNode(true) as HTMLElement;
    visible.querySelectorAll('.sr-only').forEach((hidden) => hidden.remove());
    const row = node.closest('[aria-disabled]') ?? node.closest('[role="option"]');
    // The name, then the muted secondary text (as the eye reads them, hidden words dropped).
    const [name, secondary] = Array.from(visible.children).map((part) => part.textContent ?? '');
    return {
      node,
      text: node.textContent ?? '',
      visible: [name, secondary].filter(Boolean).join(' | '),
      disabled: row?.getAttribute('aria-disabled') === 'true',
    };
  });
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

  it('refuses a public model for a private chat in the desktop’s own line, and allows it for a public one', () => {
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
    expect(validateMonitor(monitor, { chatTier: 'private', models: MODELS }).model).toBe(
      "Public model: it can't read a private chat."
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

describe('the dialog', () => {
  it('keeps the title Monitors and says first that it holds the chat’s decision model (W2-7)', () => {
    draw();
    const dialog = screen.getByRole('dialog', { name: 'Monitors' });
    expect(dialog).toHaveAccessibleDescription(
      'Choose the decision model this chat uses, and add monitors that ask it about the chat as it runs.'
    );
    expect(within(dialog).getByText('Decision model for this chat')).toBeInTheDocument();
  });
});

describe('a monitor row', () => {
  it('shows the name and three facts, in words: what it does, its rule and when it checks', () => {
    draw();
    const item = screen.getByTestId('monitor-item');
    expect(item).toHaveTextContent('Form reached');
    const line = within(item).getByTestId('monitor-line');
    expect(line.textContent).toBe(
      'Stop the turn · when P(yes) is at or above 0.85 · after each step'
    );
    expect(item.textContent).not.toMatch(/[≥<]/);
  });

  it('says when a monitor checks on its row, not only in the peek (W3-8)', () => {
    expect(monitorRowLine(wireMonitor({ timing: 'post_iteration' }))).toBe(
      'Stop the turn · when P(yes) is at or above 0.85 · after each step'
    );
    expect(monitorRowLine(wireMonitor({ timing: 'turn_end', action: 'notify' }))).toBe(
      'Notify me · when P(yes) is at or above 0.85 · when the turn ends'
    );
  });

  it('is a list row in a flat shell, not a filled box; the editor is still a filled box', async () => {
    await drawReady();
    const row = screen.getByTestId('monitor-item');
    expect(row).toHaveClass('biorouter-list-row');
    expect(row).not.toHaveClass('br-fill-surface');
    expect(row.closest('ul')).toHaveClass('biorouter-list-shell', 'br-rows-flat');
    fireEvent.click(screen.getByTestId('monitor-add'));
    // A fill never sits on a fill: `.br-fill-surface` re-points the controls' ground.
    expect(editor()).toHaveClass('br-fill-surface');
    const css = readFileSync(join(__dirname, 'decisionsPanel.css'), 'utf8');
    const body = css.match(/\n\.br-monitors__editor \{([^}]*)\}/)?.[1];
    expect(body).toMatch(/border: 1px solid transparent;/);
    expect(body).toMatch(/background-color: var\(--background-fill\);/);
  });

  it('keeps the model, timing and extras for the peek, which also describes the switch', async () => {
    const named = parseDecisionSettings({
      defaultModel: null,
      monitors: [
        {
          ...wireMonitors(STORED)[0],
          model: { provider: 'ollama_decision', model: 'tev1:0.8b' },
          rearmBelow: 0.85,
          onError: 'pause',
        },
      ],
    })!;
    mocks.settings.mockResolvedValue({ data: named, missing: false });
    await drawReady({ settings: named });
    const item = screen.getByTestId('monitor-item');
    const peek =
      "Uses tev1:0.8b on this computer, after each step. Stops only once and pauses if the model can't answer.";
    expect(within(item).getByTestId('monitor-line')).toHaveAccessibleDescription(peek);
    // The model is not on the row itself.
    expect(within(item).getByTestId('monitor-line')).not.toHaveTextContent('tev1');
    const toggle = within(item).getByRole('switch', { name: 'Form reached' });
    expect(toggle).toHaveAccessibleDescription(
      `Stop the turn · when P(yes) is at or above 0.85 · after each step ${peek}`
    );
    // R1-3: the peek opens below its line, so it never covers the row's own name.
    fireEvent.click(within(item).getByTestId('monitor-line'));
    await waitFor(() => {
      const content = document.querySelector('[data-radix-popper-content-wrapper] [data-side]');
      expect(content).not.toBeNull();
      expect(content).toHaveAttribute('data-side', 'bottom');
    });
  });

  it('says a peek in plain sentences', () => {
    const base = wireMonitor({});
    expect(monitorPeek(base, MODELS, false)).toBe(
      "Uses the chat's default model, after each step."
    );
    expect(
      monitorPeek(
        wireMonitor({
          model: {
            provider: 'baseten_decision',
            model: 'decision-models/baseten-joey-2b-latest',
          },
          timing: 'turn_end',
          applyToSubagents: true,
        }),
        MODELS,
        false
      )
    ).toBe('Uses joey-1 on Baseten, when the turn ends. Also watches subagents.');
    expect(
      monitorPeek(
        wireMonitor({ model: { provider: 'ollama_decision', model: 'tev1:0.8b' } }),
        MODELS,
        true
      )
    ).toBe('Uses tev1:0.8b on the computer Biorouter runs on, after each step.');
  });

  it('names a Baseten model in the peek by its alias, never its slug', async () => {
    const hosted = settingsWith({
      ...STORED.monitors[0],
      model: {
        provider: 'baseten_decision',
        model: 'decision-models/baseten-joey-2b-latest',
        adapter: null,
      },
    });
    mocks.settings.mockResolvedValue({ data: hosted, missing: false });
    await drawReady({ settings: hosted, chatTier: 'public' });
    const line = screen.getByTestId('monitor-line');
    expect(line).toHaveAccessibleDescription('Uses joey-1 on Baseten, after each step.');
    expect(screen.getByTestId('monitor-item')).not.toHaveTextContent('baseten-joey-2b-latest');
  });

  it('names the answer a monitor watches, as the daemon reads `option`', () => {
    expect(monitorRowLine(wireMonitor({ option: 'no' }))).toBe(
      'Stop the turn · when P(no) is at or above 0.85 · after each step'
    );
    expect(monitorRowLine(wireMonitor({ option: 'NO' }))).toMatch(/P\(no\)/);
    expect(monitorRowLine(wireMonitor({ option: null }))).toMatch(/P\(yes\)/);
    const severity = {
      id: 'severity',
      name: 'Severity',
      question: {
        instructions: 'How severe?',
        kind: { score: { levels: ['none', 'mild', 'severe'] } },
      },
    };
    expect(monitorRowLine(wireMonitor({ ...severity, option: 'mild' }))).toBe(
      'Stop the turn · when P(mild or a higher level) is at or above 0.85 · after each step'
    );
    expect(monitorRowLine(wireMonitor({ ...severity, option: '2' }))).toMatch(
      /P\(severe or a higher level\)/
    );
    const course = wireMonitor({
      question: {
        instructions: 'Which course?',
        kind: { choice: { options: { rrms: null, spms: null } } },
      },
      option: 'spms',
      threshold: 0.2,
      direction: 'below',
      action: 'notify',
    });
    expect(monitorRowLine(course)).toBe('Notify me · when P(spms) is below 0.20 · after each step');
    expect(monitorRowLine({ ...course, option: null })).toBe(
      'Notify me · choose the option it fires on · after each step'
    );
  });

  it('marks who set it with a badge whose words are in its InfoTip', async () => {
    const owned = parseDecisionSettings({
      defaultModel: null,
      monitors: [
        { ...wireMonitors(settingsWith(AGENT_C))[0] },
        {
          id: 'secrets',
          name: 'Secrets',
          question: { instructions: 'Is a secret in the output?', kind: { binary: {} } },
          threshold: 0.9,
          inheritedFrom: 'parent-1',
        },
      ],
    })!;
    mocks.settings.mockResolvedValue({ data: owned, missing: false });
    await drawReady({ settings: owned, chatTier: 'public' });
    const [agent, inherited] = screen.getAllByTestId('monitor-item');
    expect(within(agent).getByTestId('monitor-set-by-agent')).toHaveAccessibleDescription(
      MONITORS_COPY.setByAgentInfo
    );
    expect(within(inherited).getByTestId('monitor-inherited')).toHaveTextContent('Inherited');
    expect(within(inherited).getByTestId('monitor-inherited')).toHaveAccessibleDescription(
      'Copied from the chat that started this subagent.'
    );
  });

  it('draws no ⋯ and a disabled switch on a read-only surface, and no Add', async () => {
    await drawReady({ readOnly: true });
    expect(screen.queryByRole('button', { name: /More actions/ })).toBeNull();
    expect(screen.getByRole('switch', { name: 'Form reached' })).toBeDisabled();
    expect(screen.queryByTestId('monitor-add')).toBeNull();
  });
});

describe('each change applies when it is made', () => {
  it('has no footer, one Cancel, and an editor Save that re-reads, PUTs once and stays open', async () => {
    const { onSaved, onOpenChange } = await drawReady();
    expect(screen.queryByTestId('monitors-save')).toBeNull();
    fillNewMonitor('Task achieved', 'Has the agent finished what was asked?');
    expect(screen.getAllByRole('button', { name: 'Cancel' })).toHaveLength(1);
    fireEvent.click(screen.getByRole('radio', { name: 'Tell the agent' }));
    openMoreOptions();
    fireEvent.click(screen.getByRole('radio', { name: 'When the turn ends' }));
    saveEditor();
    const saved = await savedMonitors();
    // The open read, then the write's own read.
    expect(mocks.settings).toHaveBeenCalledTimes(2);
    expect(saved.map((monitor) => monitor.id)).toEqual(['form-reached', 'task-achieved']);
    expect(saved[1]).toMatchObject({
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
    await waitFor(() => expect(screen.queryByTestId('monitor-editor')).toBeNull());
    expect(screen.getAllByTestId('monitor-item')).toHaveLength(2);
    expect(screen.getByTestId('monitors-status')).toHaveTextContent('Saved');
    expect(onSaved).toHaveBeenCalledWith({ defaultModel: null, monitors: saved });
    expect(onOpenChange).not.toHaveBeenCalledWith(false);
    // Focus returns to the saved row's ⋯.
    await waitFor(() =>
      expect(document.activeElement).toHaveAccessibleName('More actions for Task achieved')
    );
    // R1-2: a new editor is not "Saved"; the earlier write's status goes with it.
    fireEvent.click(screen.getByTestId('monitor-add'));
    expect(screen.getByTestId('monitor-editor')).toBeInTheDocument();
    expect(screen.getByTestId('monitors-status')).toHaveTextContent(/^$/);
    expect(screen.getByTestId('monitors-status')).toHaveAttribute('data-state', 'idle');
  });

  it('shows what is missing inline and writes nothing', async () => {
    await drawReady();
    fireEvent.click(screen.getByTestId('monitor-add'));
    saveEditor();
    expect(within(editor()).getByText('Give the monitor a name.')).toBeInTheDocument();
    expect(within(editor()).getByText('Write the question the model answers.')).toBeInTheDocument();
    expect(mocks.save).not.toHaveBeenCalled();
  });

  it('a row Switch PUTs at once, and makes an agent’s monitor the person’s', async () => {
    const owned = settingsWith(STORED.monitors[0], AGENT_C);
    mocks.settings.mockResolvedValue({ data: owned, missing: false });
    await drawReady({ settings: owned, chatTier: 'public' });
    fireEvent.click(screen.getByRole('switch', { name: 'Agent watch' }));
    const saved = await savedMonitors();
    expect(saved[1]).toMatchObject({ enabled: false, setBy: 'person', setBySession: null });
    expect(saved[0]).toMatchObject({ id: 'form-reached', enabled: true });
    expect(await screen.findByText('Saved')).toBeInTheDocument();
  });

  it('in draft mode says the change is ready for the first message, never "Saved"', async () => {
    const onChange = vi.fn();
    await drawReady({
      sessionId: '',
      draft: { chat: { boundProvider: 'versa_azure', boundModel: 'gpt-5.5' }, onChange },
    });
    fireEvent.click(screen.getByRole('switch', { name: 'Form reached' }));
    await waitFor(() => expect(onChange).toHaveBeenCalledTimes(1));
    const status = screen.getByTestId('monitors-status');
    expect(status).toHaveAttribute('data-state', 'saved');
    expect(status).toHaveTextContent(/^Ready for the first message$/);
    expect(status).not.toHaveTextContent('Saved');
    // Nothing is stored until the chat's first message is sent.
    expect(mocks.save).not.toHaveBeenCalled();
  });

  it('a refused toggle reverts the switch and says why under that row', async () => {
    mocks.save.mockRejectedValue(new Error('Nothing was saved: `Form reached` could not run.'));
    await drawReady();
    const toggle = screen.getByRole('switch', { name: 'Form reached' });
    fireEvent.click(toggle);
    expect(await screen.findByTestId('monitor-row-error')).toHaveTextContent(
      'Nothing was saved: Form reached could not run.'
    );
    expect(toggle).toHaveAttribute('aria-checked', 'true');
  });

  describe('every write re-reads what is stored and applies only its own change', () => {
    beforeEach(() => {
      // The open read, then each write's: an agent's monitor appeared in between.
      mocks.settings
        .mockResolvedValueOnce({ data: STORED, missing: false })
        .mockResolvedValue({ data: settingsWith(STORED.monitors[0], AGENT_C), missing: false });
    });

    it('keeps it through a Switch flip', async () => {
      await drawReady();
      fireEvent.click(screen.getByRole('switch', { name: 'Form reached' }));
      const saved = await savedMonitors();
      expect(saved.map((monitor) => [monitor.id, monitor.enabled])).toEqual([
        ['form-reached', false],
        ['agent-c', true],
      ]);
      expect(saved[1].setBy).toBe('agent');
    });

    it('keeps it through an editor Save', async () => {
      await drawReady();
      await rowAction('Form reached', 'Edit');
      fireEvent.change(screen.getByTestId('monitor-name'), { target: { value: 'Consent form' } });
      saveEditor();
      const saved = await savedMonitors();
      expect(saved.map((monitor) => monitor.name)).toEqual(['Consent form', 'Agent watch']);
    });

    it('keeps it through a Remove', async () => {
      await drawReady();
      await rowAction('Form reached', 'Remove');
      expect((await savedMonitors()).map((monitor) => monitor.id)).toEqual(['agent-c']);
    });
  });

  it('a write whose read fails starts from the latest PUT response', async () => {
    await drawReady();
    fireEvent.click(screen.getByRole('switch', { name: 'Form reached' }));
    expect((await savedMonitors(1))[0].enabled).toBe(false);
    mocks.settings.mockRejectedValue(new Error('offline'));
    await waitFor(() =>
      expect(screen.getByRole('switch', { name: 'Form reached' })).not.toBeDisabled()
    );
    fireEvent.click(screen.getByRole('switch', { name: 'Form reached' }));
    expect((await savedMonitors(2))[0].enabled).toBe(true);
  });

  it('removes with Undo: focus on Undo, the status holds words only, Undo puts it back', async () => {
    await drawReady();
    await rowAction('Form reached', 'Remove');
    expect(await savedMonitors()).toEqual([]);
    const undo = await screen.findByRole('button', { name: 'Undo removing Form reached' });
    expect(undo).toHaveTextContent('Undo');
    await waitFor(() => expect(document.activeElement).toBe(undo));
    const status = screen.getByRole('status');
    expect(status).toHaveTextContent('Removed Form reached.');
    expect(within(status).queryByRole('button')).toBeNull();
    // Removing the last monitor shows the empty state, and Undo stays reachable.
    expect(screen.getByText('No monitors yet')).toBeInTheDocument();

    mocks.settings.mockResolvedValue({ data: settingsWith(), missing: false });
    fireEvent.click(undo);
    expect((await savedMonitors(2)).map((monitor) => monitor.id)).toEqual(['form-reached']);
    await waitFor(() =>
      expect(document.activeElement).toHaveAccessibleName('More actions for Form reached')
    );
    expect(screen.queryByRole('button', { name: /Undo removing/ })).toBeNull();
  });

  it('Undo re-inserts only the removed monitor, at its old index, never a snapshot', async () => {
    const b = wireMonitor({ id: 'b', name: 'B' });
    const two = settingsWith(STORED.monitors[0], b);
    mocks.settings.mockResolvedValue({ data: two, missing: false });
    await drawReady({ settings: two });
    await rowAction('Form reached', 'Remove');
    expect((await savedMonitors(1)).map((monitor) => monitor.id)).toEqual(['b']);
    // Meanwhile an agent added C.
    mocks.settings.mockResolvedValue({ data: settingsWith(b, AGENT_C), missing: false });
    fireEvent.click(await screen.findByRole('button', { name: 'Undo removing Form reached' }));
    expect((await savedMonitors(2)).map((monitor) => monitor.id)).toEqual([
      'form-reached',
      'b',
      'agent-c',
    ]);
  });

  it('an Undo after the monitor was put back elsewhere writes nothing new for it', async () => {
    await drawReady();
    await rowAction('Form reached', 'Remove');
    await savedMonitors(1);
    mocks.settings.mockResolvedValue({ data: STORED, missing: false });
    fireEvent.click(await screen.findByRole('button', { name: 'Undo removing Form reached' }));
    await waitFor(() => expect(screen.getAllByTestId('monitor-item')).toHaveLength(1));
    expect(mocks.save).toHaveBeenCalledTimes(1);
  });

  it('closes with × or Escape while a write is in flight; the write still lands, the next open reads afresh', async () => {
    let finish: (value: DecisionSettings) => void = () => {};
    mocks.save.mockReturnValue(new Promise((resolve) => (finish = resolve)));
    const { onOpenChange, onSaved, rerender } = await drawReady();
    fireEvent.click(screen.getByRole('switch', { name: 'Form reached' }));
    await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1));
    expect(screen.getByTestId('monitors-status')).toHaveTextContent('Saving…');
    fireEvent.keyDown(screen.getByTestId('monitors-dialog'), { key: 'Escape' });
    expect(onOpenChange).toHaveBeenCalledWith(false);
    rerender({ open: false });
    const stored = settingsWith({ ...STORED.monitors[0], enabled: false });
    await act(async () => finish(stored));
    expect(onSaved).toHaveBeenCalledWith(stored);
    const reads = mocks.settings.mock.calls.length;
    rerender({ open: true });
    await waitFor(() => expect(mocks.settings.mock.calls.length).toBe(reads + 1));
  });

  it('locks every row control while a write is in flight', async () => {
    mocks.save.mockReturnValue(new Promise(() => {}));
    await drawReady();
    fireEvent.click(screen.getByRole('switch', { name: 'Form reached' }));
    await waitFor(() => expect(mocks.save).toHaveBeenCalled());
    expect(screen.getByRole('switch', { name: 'Form reached' })).toBeDisabled();
    expect(screen.getByTestId('monitor-add')).toBeDisabled();
  });

  it('in a browser, writes the same way: a keyless daemon takes the write', async () => {
    document.documentElement.dataset.biorouterSurface = 'headless';
    await drawReady();
    fireEvent.click(screen.getByRole('switch', { name: 'Form reached' }));
    await waitFor(() => expect(mocks.save).toHaveBeenCalled());
  });
});

describe('the editor, in place', () => {
  const two = settingsWith(STORED.monitors[0], wireMonitor({ id: 'b', name: 'B' }));

  it('replaces the row it edits, in its place, and locks the rest', async () => {
    mocks.settings.mockResolvedValue({ data: two, missing: false });
    await drawReady({ settings: two });
    await rowAction('Form reached', 'Edit');
    const rows = screen.getAllByTestId('monitor-item');
    expect(rows).toHaveLength(1);
    expect(
      editor().compareDocumentPosition(rows[0]) & Node.DOCUMENT_POSITION_FOLLOWING
    ).toBeTruthy();
    expect(screen.getByRole('switch', { name: 'B' })).toBeDisabled();
    expect(screen.getByTestId('monitor-add')).toBeDisabled();
    // No On switch in the editor: the row's switch is where a monitor is turned off.
    expect(within(editor()).queryByRole('switch')).toBeNull();
    await waitFor(() => expect(document.activeElement).toBe(screen.getByTestId('monitor-name')));
    fireEvent.click(screen.getByTestId('monitor-cancel'));
    await waitFor(() =>
      expect(document.activeElement).toHaveAccessibleName('More actions for Form reached')
    );
  });

  it('puts a new monitor’s editor at the end of the list, and Cancel returns to Add', async () => {
    mocks.settings.mockResolvedValue({ data: two, missing: false });
    await drawReady({ settings: two });
    fireEvent.click(screen.getByTestId('monitor-add'));
    const rows = screen.getAllByTestId('monitor-item');
    expect(
      rows[1].compareDocumentPosition(editor()) & Node.DOCUMENT_POSITION_FOLLOWING
    ).toBeTruthy();
    fireEvent.click(screen.getByTestId('monitor-cancel'));
    await waitFor(() => expect(document.activeElement).toBe(screen.getByTestId('monitor-add')));
  });

  it('keeps the editor open with the daemon’s refusal above its buttons', async () => {
    const sentence =
      'Nothing was saved: monitor "Task achieved" could not run. No decision model this private chat may use is available.';
    mocks.save.mockRejectedValue(new DecisionRequestError(sentence, 422, sentence));
    const { onOpenChange } = await drawReady();
    fillNewMonitor('Task achieved', 'Has the agent finished what was asked?');
    saveEditor();
    expect(await screen.findByTestId('monitor-save-error')).toHaveTextContent(sentence);
    expect(editor()).toBeInTheDocument();
    expect(onOpenChange).not.toHaveBeenCalledWith(false);
  });

  it('puts the question’s help in an InfoTip and a real example in the placeholder', async () => {
    await drawReady();
    fireEvent.click(screen.getByTestId('monitor-add'));
    const question = screen.getByTestId('monitor-question');
    expect(question).toHaveAttribute(
      'placeholder',
      'For example: Does the newest message ask to delete files?'
    );
    expect(question).toHaveAccessibleDescription(MONITORS_COPY.questionInfo);
    expect(screen.getByRole('button', { name: 'About Question' })).toBeInTheDocument();
  });

  describe('More options', () => {
    it('starts closed for a new monitor, with the defaults it hides as its summary', async () => {
      await drawReady();
      fireEvent.click(screen.getByTestId('monitor-add'));
      expect(moreOptionsOpen()).toBe(false);
      expect(
        within(editor()).getByRole('button', { name: MONITORS_COPY.moreOptions })
      ).toHaveAccessibleDescription('Yes or no · when P(yes) is at or above 0.85 · Default model');
    });

    it('starts closed for an edit of a pristine monitor', async () => {
      const pristine = settingsWith({ ...STORED.monitors[0], action: 'notify' });
      mocks.settings.mockResolvedValue({ data: pristine, missing: false });
      await drawReady({ settings: pristine });
      await rowAction('Form reached', 'Edit');
      expect(moreOptionsOpen()).toBe(false);
    });

    it.each([
      ['a choice', { kind: { type: 'choice', options: [{ name: 'a', description: null }] } }],
      ['a named model', { model: { provider: 'ollama_decision', model: 'tev1:0.8b' } }],
      ['when the turn ends', { timing: 'turn_end' }],
      ['pause the turn', { onError: 'pause' }],
      ['stop only once', { rearmBelow: 0.85 }],
      ['watching subagents', { applyToSubagents: true }],
      ['a threshold other than 0.85', { threshold: 0.6 }],
      ['Below', { direction: 'below' }],
      ['watching no', { option: 'no' }],
    ])('starts open for an edit of a monitor with %s', (_what, change) => {
      const base = { ...STORED.monitors[0] };
      const monitor =
        'kind' in change
          ? {
              ...base,
              question: {
                ...base.question,
                kind: change.kind as MonitorCriterion['question']['kind'],
              },
            }
          : ({ ...base, ...change } as MonitorCriterion);
      expect(usesHiddenSettings(monitor)).toBe(true);
      expect(usesHiddenSettings(base)).toBe(false);
    });

    it('opens on edit when the stored monitor uses one of them', async () => {
      const named = settingsWith({ ...STORED.monitors[0], timing: 'turn_end' });
      mocks.settings.mockResolvedValue({ data: named, missing: false });
      await drawReady({ settings: named });
      await rowAction('Form reached', 'Edit');
      expect(moreOptionsOpen()).toBe(true);
    });

    it('opens and focuses a hidden field that Save finds an error in', async () => {
      await drawReady();
      fillNewMonitor('Course', 'Which course is described?');
      openMoreOptions();
      fireEvent.click(screen.getByRole('radio', { name: 'One of several options' }));
      // Closed again, with the error inside it.
      fireEvent.click(within(editor()).getByRole('button', { name: MONITORS_COPY.moreOptions }));
      expect(moreOptionsOpen()).toBe(false);
      saveEditor();
      expect(moreOptionsOpen()).toBe(true);
      await waitFor(() =>
        expect(document.activeElement).toBe(screen.getByRole('textbox', { name: 'Option 1 name' }))
      );
      expect(screen.getByText('A choice needs at least two options.')).toBeInTheDocument();
      expect(mocks.save).not.toHaveBeenCalled();
    });

    it('names its fields in plain words, and nothing in the dialog says fail', async () => {
      await drawReady();
      fillNewMonitor('Stop', 'Is it unsafe?');
      fireEvent.click(screen.getByRole('radio', { name: 'Stop the turn' }));
      openMoreOptions();
      for (const label of ['Answer type', 'Check', "If the model can't answer", 'Fires when']) {
        expect(within(editor()).getByText(label)).toBeInTheDocument();
      }
      expect(screen.getByTestId('monitor-stops-once')).toBeInTheDocument();
      expect(screen.getByTestId('monitor-subagents')).toBeInTheDocument();
      expect(screen.queryByText(/fail/i)).toBeNull();
      expect(screen.getByTestId('monitors-dialog').textContent).not.toMatch(/fail/i);
    });
  });

  describe('Fires when', () => {
    it('is one group named by its label and described by its sentence', async () => {
      await drawReady();
      fireEvent.click(screen.getByTestId('monitor-add'));
      openMoreOptions();
      const group = screen.getByRole('group', { name: 'Fires when' });
      expect(group).toHaveAccessibleDescription('Fires when P(yes) is at or above 0.85.');
      expect(within(group).getByRole('radiogroup', { name: 'Answer it watches' })).toBeVisible();
      expect(within(group).getByRole('radiogroup', { name: 'Side' })).toBeVisible();
      expect(thresholdThumb()).toHaveAccessibleName('Threshold');
      expect(thresholdThumb()).toHaveAccessibleDescription(
        'Fires when P(yes) is at or above 0.85.'
      );
      fireEvent.click(within(group).getByRole('radio', { name: 'Below' }));
      fireEvent.keyDown(thresholdThumb(), { key: 'Home' });
      for (let i = 0; i < 6; i += 1) fireEvent.keyDown(thresholdThumb(), { key: 'ArrowRight' });
      expect(screen.getByTestId('monitor-fires-sentence')).toHaveTextContent(
        'Fires when P(yes) is below 0.30.'
      );
    });

    it('says a choice’s option, and asks for one first', async () => {
      await drawReady();
      fillNewMonitor('Course', 'Which course?');
      openMoreOptions();
      fireEvent.click(screen.getByRole('radio', { name: 'One of several options' }));
      expect(screen.getByTestId('monitor-fires-sentence')).toHaveTextContent(
        'Choose the option it fires on.'
      );
      fireEvent.change(screen.getByRole('textbox', { name: 'Option 1 name' }), {
        target: { value: 'rrms' },
      });
      fireEvent.change(screen.getByRole('textbox', { name: 'Option 2 name' }), {
        target: { value: 'spms' },
      });
      const watched = within(screen.getByTestId('monitor-fires-on')).getByRole('combobox');
      fireEvent.keyDown(watched, { key: 'ArrowDown' });
      fireEvent.click(await screen.findByRole('option', { name: 'rrms' }));
      expect(screen.getByTestId('monitor-fires-sentence')).toHaveTextContent(
        'Fires when P(rrms) is at or above 0.85.'
      );
    });

    it('says a score monitor’s level', async () => {
      const severity = parseDecisionSettings({
        defaultModel: null,
        monitors: [
          {
            id: 'severity',
            name: 'Severity',
            question: {
              instructions: 'How severe?',
              kind: { score: { levels: ['none', 'mild', 'severe'] } },
            },
            option: 'mild',
            threshold: 0.85,
          },
        ],
      })!;
      mocks.settings.mockResolvedValue({ data: severity, missing: false });
      await drawReady({ settings: severity });
      await rowAction('Severity', 'Edit');
      expect(screen.getByTestId('monitor-fires-sentence')).toHaveTextContent(
        'Fires when P(mild or a higher level) is at or above 0.85.'
      );
    });
  });

  it('writes a choice option’s description, and every option field has a placeholder', async () => {
    await drawReady({ chatTier: 'public' });
    fillNewMonitor('Course', 'Which course?');
    openMoreOptions();
    fireEvent.click(screen.getByRole('radio', { name: 'One of several options' }));
    const name1 = screen.getByRole('textbox', { name: 'Option 1 name' });
    expect(name1).toHaveAttribute('placeholder', 'Option 1');
    expect(screen.getByRole('textbox', { name: 'Option 2 name' })).toHaveAttribute(
      'placeholder',
      'Option 2'
    );
    const desc1 = screen.getByRole('textbox', { name: 'Option 1 description' });
    expect(desc1).toHaveAttribute('placeholder', 'What it means (optional)');
    fireEvent.change(name1, { target: { value: 'rrms' } });
    fireEvent.change(desc1, { target: { value: 'Relapsing-remitting MS' } });
    fireEvent.change(screen.getByRole('textbox', { name: 'Option 2 name' }), {
      target: { value: 'spms' },
    });
    fireEvent.keyDown(within(screen.getByTestId('monitor-fires-on')).getByRole('combobox'), {
      key: 'ArrowDown',
    });
    fireEvent.click(await screen.findByRole('option', { name: 'spms' }));
    saveEditor();
    const saved = (await savedMonitors())[1];
    expect(saved.question.kind).toEqual({
      type: 'choice',
      options: [
        { name: 'rrms', description: 'Relapsing-remitting MS' },
        { name: 'spms', description: null },
      ],
    });
    expect(saved.option).toBe('spms');
  });
});

describe('the model picker', () => {
  async function openPicker(overrides: Partial<Parameters<typeof MonitorsDialog>[0]> = {}) {
    await drawReady(overrides);
    fireEvent.click(screen.getByTestId('monitor-add'));
    openMoreOptions();
    await openModelMenu();
  }

  it('names where each model runs, the window only when small, and privacy by a lock', async () => {
    await openPicker({ chatTier: 'public', models: [...MODELS, ...CLEF] });
    const rows = modelRows();
    expect(rows.map((row) => row.visible)).toEqual([
      "Default model | the chat's decision model",
      'tev1:0.8b | this computer · 2K tokens',
      'joey-1 | Baseten',
      'clef-flash:latest | this computer',
    ]);
    expect(rows[1].text).toContain('Private model');
    expect(rows[3].text).toContain('Private model');
    expect(rows[2].text).not.toContain('Private model');
    for (const row of rows) expect(row.visible).not.toMatch(/\b(Private|Public)\b/);
  });

  it('says the desktop’s line for a public model in a private chat, whatever the daemon wrote', async () => {
    mocks.catalog.mockResolvedValue(
      parseDecisionCatalog({
        models: [
          MODELS[0],
          {
            ...MODELS[1],
            unavailableReason:
              'This chat is private, so `joey-1` (a public model) may not read it (decision_models lists them).',
          },
        ],
      })
    );
    await openPicker({ models: undefined });
    const joey = modelRows().find((row) => row.text.startsWith('joey-1'))!;
    expect(joey.disabled).toBe(true);
    expect(joey.visible).toContain("Public model: it can't read a private chat.");
    expect(joey.text).not.toMatch(/`|decision_models/);
    // The note that used to sit under the picker is gone.
    expect(screen.queryByText('Only a private model may read a private chat.')).toBeNull();
  });

  it('names the stored default model on the Default row, and shows a pick by its name alone', async () => {
    const withDefault = parseDecisionSettings({
      ...decisionSettingsToWire(STORED),
      defaultModel: { provider: 'ollama_decision', model: 'tev1:0.8b' },
    })!;
    mocks.settings.mockResolvedValue({ data: withDefault, missing: false });
    await openPicker({ settings: withDefault });
    expect(modelRows()[0].visible).toBe('Default model | tev1:0.8b');
    fireEvent.click(modelRows()[1].node);
    const value = within(screen.getByTestId('monitor-model')).getByText('tev1:0.8b');
    expect(value.closest('[data-testid="decision-model-option"]')).toBeNull();
  });

  it('asks for the catalog naming the chat, and shows the daemon’s own reason without backticks', async () => {
    mocks.catalog.mockResolvedValue(
      parseDecisionCatalog({
        models: [{ ...MODELS[0], unavailableReason: 'Your institution does not cover `tev1`.' }],
      })
    );
    await openPicker({ models: undefined });
    expect(mocks.catalog).toHaveBeenCalledWith(expect.any(AbortSignal), 's1');
    expect(screen.getByText('Your institution does not cover tev1.')).toBeInTheDocument();
  });

  it('in a Crew-scoped chat, holds Save at the Model field with the Crew sentence', async () => {
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
    fillNewMonitor('Task achieved', 'Has the agent finished what was asked?');
    saveEditor();
    // Hidden behind More options, so Save opens it and the field says why.
    expect(moreOptionsOpen()).toBe(true);
    const field = screen.getByTestId('monitor-model');
    expect(await within(field).findByRole('alert')).toHaveTextContent(crew);
    expect(mocks.save).not.toHaveBeenCalled();
  });
});

/** A monitor read from the daemon that this dialog does not make. */
describe('a monitor the dialog did not make', () => {
  const SEVERITY = {
    id: 'severity',
    name: 'Severity',
    question: {
      instructions: 'How severe is the reported relapse?',
      kind: { score: { levels: ['none', 'mild', 'severe'] } },
    },
  };

  it('shows a score monitor as a score with its levels, and saves it unchanged after a rename', async () => {
    const severity = parseDecisionSettings({
      defaultModel: null,
      monitors: [{ ...SEVERITY, threshold: 0.8, action: 'notify', option: '1' }],
    })!;
    mocks.settings.mockResolvedValue({ data: severity, missing: false });
    await drawReady({ settings: severity });
    expect(screen.getByTestId('monitor-line')).toHaveTextContent(
      'when P(mild or a higher level) is at or above 0.80'
    );
    await rowAction('Severity', 'Edit');
    // A score is a hidden setting, so More options starts open.
    expect(within(editor()).getByRole('radio', { name: 'Score' })).toHaveAttribute(
      'aria-checked',
      'true'
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
    saveEditor();
    const saved = (await savedMonitors())[0];
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
    await rowAction('Severity', 'Edit');
    fireEvent.click(screen.getByRole('radio', { name: 'Yes or no' }));
    expect(screen.queryByRole('radio', { name: 'Score' })).toBeNull();
    expect(screen.queryByTestId('monitor-levels')).toBeNull();
    saveEditor();
    const saved = (await savedMonitors())[0];
    expect(saved.question.kind).toEqual({ type: 'binary' });
    expect(saved.option).toBeNull();
  });

  it('shows a yes/no monitor that watches no, and sets it back to yes', async () => {
    const watchingNo = settingsWith({ ...STORED.monitors[0], option: 'no' });
    mocks.settings.mockResolvedValue({ data: watchingNo, missing: false });
    await drawReady({ settings: watchingNo });
    expect(screen.getByTestId('monitor-line')).toHaveTextContent('when P(no) is at or above 0.85');
    await rowAction('Form reached', 'Edit');
    const firesOn = screen.getByTestId('monitor-fires-on');
    expect(within(firesOn).getByRole('radio', { name: 'No' })).toHaveAttribute(
      'aria-checked',
      'true'
    );
    // Back to yes: no option, the default the daemon reads.
    fireEvent.click(within(firesOn).getByRole('radio', { name: 'Yes' }));
    saveEditor();
    expect((await savedMonitors())[0].option).toBeNull();
  });

  it('sets a new yes/no monitor to fire on no', async () => {
    await drawReady();
    fillNewMonitor('Still open', 'Has the form been submitted?');
    openMoreOptions();
    const firesOn = screen.getByTestId('monitor-fires-on');
    expect(within(firesOn).getByRole('radio', { name: 'Yes' })).toHaveAttribute(
      'aria-checked',
      'true'
    );
    fireEvent.click(within(firesOn).getByRole('radio', { name: 'No' }));
    saveEditor();
    const saved = (await savedMonitors())[1];
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
    openMoreOptions();
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

  it('says a failure with its message, and the Retry link reads again keeping the draft', async () => {
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
    const retry = within(field).getByRole('button', { name: 'Retry' });
    expect(retry).toHaveClass('br-monitors__inline-link');
    fireEvent.click(retry);
    await waitFor(() => expect(mocks.catalog).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(within(field).queryByTestId('monitor-models-error')).toBeNull());
    expect(screen.getByTestId('monitor-name')).toHaveValue('Task achieved');
    await openModelMenu();
    expect(modelRows().some((row) => row.text.startsWith('tev1:0.8b'))).toBe(true);
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

/** What the open read decides. */
describe('reading what is stored on open', () => {
  /**
   * Opened from the composer's `+` while the summary is folded away, nothing has
   * read the chat's settings yet. An empty list there would be a lie, and a write
   * from it would replace every stored monitor and the chat's default model.
   */
  it('opened before the settings are known, reads them and holds every change until they arrive', async () => {
    const stored = parseDecisionSettings({
      ...decisionSettingsToWire(STORED),
      defaultModel: { provider: 'ollama_decision', model: 'tev1:0.8b' },
    })!;
    let answer: (value: { data: DecisionSettings; missing: boolean }) => void = () => {};
    mocks.settings.mockReturnValueOnce(new Promise((resolve) => (answer = resolve)));
    draw({ settings: null });
    expect(mocks.settings).toHaveBeenCalledWith('s1', expect.any(AbortSignal));
    expect(screen.getByTestId('monitors-loading')).toBeInTheDocument();
    expect(screen.queryByTestId('monitor-add')).toBeNull();

    mocks.settings.mockResolvedValue({ data: stored, missing: false });
    answer({ data: stored, missing: false });
    expect(await screen.findByTestId('monitor-item')).toHaveTextContent('Form reached');
    fireEvent.click(screen.getByRole('switch', { name: 'Form reached' }));
    await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1));
    const saved = mocks.save.mock.calls[0][1] as DecisionSettings;
    expect(saved.monitors.map((monitor) => monitor.id)).toEqual(['form-reached']);
    expect(saved.defaultModel).toEqual(stored.defaultModel);
  });

  it('says so and offers no Add when the settings cannot be read', async () => {
    mocks.settings.mockRejectedValue(new Error('Decision settings unavailable'));
    draw({ settings: null });
    expect(await screen.findByTestId('monitors-load-error')).toHaveTextContent(
      "Could not read this chat's monitors. Close and open again to retry."
    );
    expect(screen.queryByTestId('monitor-add')).toBeNull();
  });

  it('starts from what is stored now, not the caller’s older copy, and writes it whole', async () => {
    const fromCli = parseDecisionSettings({
      defaultModel: { provider: 'ollama_decision', model: 'tev1:0.8b' },
      monitors: [
        wireMonitors(STORED)[0],
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
    await waitFor(() => expect(screen.getAllByTestId('monitor-item')).toHaveLength(2));
    fireEvent.click(screen.getByRole('switch', { name: 'Form reached' }));
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
    fireEvent.click(screen.getByRole('switch', { name: 'Form reached' }));
    expect((await savedMonitors())[0]).toMatchObject({ id: 'form-reached', enabled: false });
  });

  it('takes a newer copy from the caller between writes', async () => {
    const { rerender } = await drawReady();
    rerender({ settings: settingsWith(STORED.monitors[0], AGENT_C) });
    expect(await screen.findByText('Agent watch')).toBeInTheDocument();
  });
});

describe('empty, and at the limit', () => {
  it('shows the empty state with its one action', async () => {
    mocks.settings.mockResolvedValue({ data: settingsWith(), missing: false });
    await drawReady({ settings: settingsWith() });
    expect(screen.getByText('No monitors yet')).toBeInTheDocument();
    expect(screen.getByText(MONITORS_COPY.emptyDescription)).toBeInTheDocument();
    const add = screen.getByRole('button', { name: 'Add a monitor' });
    expect(screen.getAllByRole('button', { name: 'Add a monitor' })).toHaveLength(1);
    fireEvent.click(add);
    expect(editor()).toBeInTheDocument();
  });

  it('turns Add off and says the limit at eight monitors', async () => {
    const eight = settingsWith(
      ...Array.from({ length: 8 }, (_, i) => wireMonitor({ id: `m${i}`, name: `M${i}` }))
    );
    mocks.settings.mockResolvedValue({ data: eight, missing: false });
    await drawReady({ settings: eight });
    expect(screen.getByTestId('monitor-add')).toBeDisabled();
    expect(screen.getByTestId('monitors-status')).toHaveTextContent(
      'A chat can have at most 8 monitors.'
    );
  });
});

/**
 * Which side fires, low thresholds, and the model not answering: the CLI and HTTP
 * could set all three, and the dialog keeps every one of them.
 */
describe('which side fires, low thresholds, and when the model cannot answer', () => {
  it('offers thresholds from 0.01 to 0.99 and keeps a stored threshold that is not a stop', async () => {
    expect(THRESHOLD_STEPS).toHaveLength(21);
    expect(THRESHOLD_STEPS[0].value).toBe('0.01');
    expect(THRESHOLD_STEPS.map((step) => step.value)).toContain('0.30');
    expect(THRESHOLD_STEPS[20].value).toBe('0.99');
    const steps = thresholdStepsFor(0.33).map((step) => step.value);
    expect(steps).toHaveLength(22);
    expect(steps.slice(6, 9)).toEqual(['0.30', '0.33', '0.35']);
    expect(thresholdStepsFor(0.3)).toBe(THRESHOLD_STEPS);

    const odd = settingsWith({ ...STORED.monitors[0], threshold: 0.33 });
    mocks.settings.mockResolvedValue({ data: odd, missing: false });
    await drawReady({ settings: odd });
    await rowAction('Form reached', 'Edit');
    // A threshold other than 0.85 opens More options; shown where it is, not snapped.
    expect(thresholdThumb()).toHaveAttribute('aria-valuetext', '0.33');
    saveEditor();
    expect((await savedMonitors())[0].threshold).toBe(0.33);
  });

  it('keeps a stored threshold that is not a stop as a stop for the whole edit', async () => {
    const odd = settingsWith({ ...STORED.monitors[0], threshold: 0.33 });
    mocks.settings.mockResolvedValue({ data: odd, missing: false });
    await drawReady({ settings: odd });
    await rowAction('Form reached', 'Edit');
    expect(thresholdThumb()).toHaveAttribute('aria-valuemax', '21');
    fireEvent.keyDown(thresholdThumb(), { key: 'ArrowRight' });
    expect(thresholdThumb()).toHaveAttribute('aria-valuetext', '0.35');
    expect(thresholdThumb()).toHaveAttribute('aria-valuemax', '21');
    fireEvent.keyDown(thresholdThumb(), { key: 'ArrowLeft' });
    expect(thresholdThumb()).toHaveAttribute('aria-valuetext', '0.33');

    // A drag: a 210px rail puts stop i at x = 10i.
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
    saveEditor();
    expect((await savedMonitors())[0].threshold).toBe(0.33);
  });

  it('reaches 0.01 at the low end of the slider', async () => {
    await drawReady();
    fillNewMonitor('Quiet', 'Has the agent asked a question?');
    openMoreOptions();
    fireEvent.keyDown(thresholdThumb(), { key: 'Home' });
    expect(thresholdThumb()).toHaveAttribute('aria-valuetext', '0.01');
    saveEditor();
    expect((await savedMonitors())[1].threshold).toBe(0.01);
  });

  it('saves a monitor that fires below its threshold, said in words on its row', async () => {
    await drawReady();
    fillNewMonitor('Banana', 'Has the assistant written the word banana?');
    openMoreOptions();
    const direction = screen.getByTestId('monitor-direction');
    expect(within(direction).getByRole('radio', { name: 'At or above' })).toHaveAttribute(
      'aria-checked',
      'true'
    );
    fireEvent.click(within(direction).getByRole('radio', { name: 'Below' }));
    fireEvent.keyDown(thresholdThumb(), { key: 'Home' });
    for (let i = 0; i < 6; i += 1) fireEvent.keyDown(thresholdThumb(), { key: 'ArrowRight' });
    expect(thresholdThumb()).toHaveAttribute('aria-valuetext', '0.30');
    saveEditor();
    const saved = (await savedMonitors())[1];
    expect(saved).toMatchObject({ name: 'Banana', direction: 'below', threshold: 0.3 });
    expect(decisionSettingsToWire({ defaultModel: null, monitors: [saved] }).monitors).toEqual([
      expect.objectContaining({ direction: 'below', threshold: 0.3 }),
    ]);
    await waitFor(() => expect(screen.getAllByTestId('monitor-item')).toHaveLength(2));
    expect(screen.getAllByTestId('monitor-line')[1]).toHaveTextContent(
      'Notify me · when P(yes) is below 0.30'
    );
  });

  it('saves onError pause, said in the peek', async () => {
    await drawReady();
    fillNewMonitor('Signed', 'Has the form been signed?');
    openMoreOptions();
    const onError = screen.getByTestId('monitor-on-error');
    expect(within(onError).getByRole('radio', { name: 'Notify me' })).toHaveAttribute(
      'aria-checked',
      'true'
    );
    expect(screen.getByRole('button', { name: "About If the model can't answer" })).toBeVisible();
    fireEvent.click(within(onError).getByRole('radio', { name: 'Pause the turn' }));
    saveEditor();
    expect((await savedMonitors())[1].onError).toBe('pause');
    await waitFor(() => expect(screen.getAllByTestId('monitor-item')).toHaveLength(2));
    expect(screen.getAllByTestId('monitor-line')[1]).toHaveAccessibleDescription(
      "Uses the chat's default model, after each step. Pauses if the model can't answer."
    );
    // A new monitor still starts on notify, as the daemon and the CLI do.
    expect(newMonitor().onError).toBe('notify');
  });
});

/** A-4: "Stop only once" sets the re-arm level the CLI and HTTP could always set. */
describe('Stop only once', () => {
  const latched = (wire: Record<string, unknown>) =>
    settingsWith(wireMonitor({ id: 'form-reached', name: 'Form reached', ...wire }));

  it('is offered only for Stop the turn, unchecked for a new monitor', async () => {
    await drawReady();
    fillNewMonitor('Stop', 'Is it unsafe?');
    openMoreOptions();
    expect(screen.queryByTestId('monitor-stops-once')).toBeNull();
    fireEvent.click(screen.getByRole('radio', { name: 'Stop the turn' }));
    const box = within(screen.getByTestId('monitor-stops-once')).getByRole('checkbox', {
      name: 'Stop only once',
    });
    expect(box).not.toBeChecked();
    expect(box).toHaveAccessibleDescription(
      'After it stops the turn, the monitor stays quiet until P(yes) drops below 0.85 again.'
    );
  });

  it('sets the level at the threshold when checked, on either side, and round-trips', async () => {
    await drawReady();
    fillNewMonitor('Stop', 'Is it unsafe?');
    fireEvent.click(screen.getByRole('radio', { name: 'Stop the turn' }));
    openMoreOptions();
    fireEvent.click(screen.getByRole('checkbox', { name: 'Stop only once' }));
    saveEditor();
    expect((await savedMonitors())[1]).toMatchObject({ rearmBelow: 0.85, rearmAbove: null });

    const below = { ...newMonitor(), direction: 'below' as const, threshold: 0.3 };
    expect(stopOnlyOnce(below, true, null)).toEqual({ rearmBelow: null, rearmAbove: 0.3 });
    expect(stopOnlyOnce(below, false, null)).toEqual({ rearmBelow: null, rearmAbove: null });
  });

  it('says the kept level and side in its InfoTip, keeps a stored level, and clears it unchecked', async () => {
    const stored = latched({ threshold: 0.3, direction: 'below', rearmAbove: 0.5 });
    mocks.settings.mockResolvedValue({ data: stored, missing: false });
    await drawReady({ settings: stored });
    await rowAction('Form reached', 'Edit');
    const box = screen.getByRole('checkbox', { name: 'Stop only once' });
    expect(box).toBeChecked();
    expect(box).toHaveAccessibleDescription(
      'After it stops the turn, the monitor stays quiet until P(yes) rises to 0.50 or more again.'
    );
    fireEvent.click(box);
    expect(box).not.toBeChecked();
    fireEvent.click(box);
    // Checked again: the level the monitor opened with comes back.
    expect(box).toHaveAccessibleDescription(/rises to 0\.50 or more/);
    fireEvent.click(box);
    saveEditor();
    expect((await savedMonitors())[0]).toMatchObject({ rearmAbove: null, rearmBelow: null });
  });

  it('switching the direction clears a re-arm level', async () => {
    const stored = latched({ rearmBelow: 0.7 });
    mocks.settings.mockResolvedValue({ data: stored, missing: false });
    await drawReady({ settings: stored });
    await rowAction('Form reached', 'Edit');
    expect(screen.getByRole('checkbox', { name: 'Stop only once' })).toBeChecked();
    fireEvent.click(
      within(screen.getByTestId('monitor-direction')).getByRole('radio', { name: 'Below' })
    );
    expect(screen.getByRole('checkbox', { name: 'Stop only once' })).not.toBeChecked();
    saveEditor();
    expect((await savedMonitors())[0]).toMatchObject({
      direction: 'below',
      rearmBelow: null,
      rearmAbove: null,
    });
  });

  it('keeps a re-arm level made over HTTP when the monitor is renamed', async () => {
    const stored = latched({ rearmBelow: 0.7 });
    mocks.settings.mockResolvedValue({ data: stored, missing: false });
    await drawReady({ settings: stored });
    await rowAction('Form reached', 'Edit');
    fireEvent.change(screen.getByTestId('monitor-name'), { target: { value: 'Consent form' } });
    saveEditor();
    expect((await savedMonitors())[0]).toMatchObject({ name: 'Consent form', rearmBelow: 0.7 });
  });

  // The daemon refuses a re-arm level past the threshold (`settings.rs`'s `validate`).
  it('holds a kept level at a threshold moved below it, and gives it back', async () => {
    const stored = latched({ threshold: 0.8, rearmBelow: 0.6 });
    mocks.settings.mockResolvedValue({ data: stored, missing: false });
    await drawReady({ settings: stored });
    await rowAction('Form reached', 'Edit');
    const box = () => screen.getByRole('checkbox', { name: 'Stop only once' });
    const press = (key: string, times: number) => {
      for (let i = 0; i < times; i += 1) fireEvent.keyDown(thresholdThumb(), { key });
    };
    press('ArrowLeft', 4);
    expect(box()).toHaveAccessibleDescription(/drops below 0\.60 again/);
    press('ArrowLeft', 4);
    expect(thresholdThumb()).toHaveAttribute('aria-valuetext', '0.40');
    expect(box()).toHaveAccessibleDescription(/drops below 0\.40 again/);
    press('ArrowRight', 8);
    expect(box()).toHaveAccessibleDescription(/drops below 0\.60 again/);
    press('ArrowLeft', 8);
    saveEditor();
    const saved = (await savedMonitors())[0];
    expect(saved).toMatchObject({ threshold: 0.4, rearmBelow: 0.4, action: 'stop' });
    expect(saved.rearmBelow!).toBeLessThanOrEqual(saved.threshold);
  });

  it('holds a below monitor’s level at a threshold moved above it', async () => {
    const stored = latched({ threshold: 0.3, direction: 'below', rearmAbove: 0.5 });
    mocks.settings.mockResolvedValue({ data: stored, missing: false });
    await drawReady({ settings: stored });
    await rowAction('Form reached', 'Edit');
    for (let i = 0; i < 8; i += 1) fireEvent.keyDown(thresholdThumb(), { key: 'ArrowRight' });
    expect(thresholdThumb()).toHaveAttribute('aria-valuetext', '0.70');
    expect(screen.getByRole('checkbox', { name: 'Stop only once' })).toHaveAccessibleDescription(
      /rises to 0\.70 or more again/
    );
    saveEditor();
    const saved = (await savedMonitors())[0];
    expect(saved).toMatchObject({ direction: 'below', threshold: 0.7, rearmAbove: 0.7 });
    expect(saved.rearmAbove!).toBeGreaterThanOrEqual(saved.threshold);
  });

  it('gives a threshold move no level when the monitor has none, or the direction switch cleared it', () => {
    const opened: MonitorCriterion = { ...newMonitor(), threshold: 0.8, rearmBelow: 0.6 };
    expect(rearmForThreshold({ ...newMonitor(), threshold: 0.8 }, 0.4, null)).toEqual({
      rearmBelow: null,
      rearmAbove: null,
    });
    expect(rearmForThreshold({ ...opened, rearmBelow: null }, 0.4, opened)).toEqual({
      rearmBelow: null,
      rearmAbove: null,
    });
    expect(rearmForThreshold(opened, 0.9, opened)).toEqual({ rearmBelow: 0.6, rearmAbove: null });
  });

  it('saves a below monitor read from the daemon back exactly as it was read', async () => {
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
    expect(screen.getByTestId('monitor-line')).toHaveTextContent(
      'Stop the turn · when P(yes) is below 0.30'
    );
    await rowAction('Banana', 'Edit');
    expect(
      within(screen.getByTestId('monitor-direction')).getByRole('radio', { name: 'Below' })
    ).toHaveAttribute('aria-checked', 'true');
    expect(
      within(screen.getByTestId('monitor-on-error')).getByRole('radio', { name: 'Pause the turn' })
    ).toHaveAttribute('aria-checked', 'true');
    saveEditor();
    const saved = (await savedMonitors())[0];
    expect(decisionSettingsToWire({ defaultModel: null, monitors: [saved] })).toEqual(
      decisionSettingsToWire(fromCli)
    );
  });
});

/** Who set a monitor, and the subagents a person's monitor also watches. */
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

  it('round trips owner fields through a write to another monitor', async () => {
    const owned = ownedSettings();
    mocks.settings.mockResolvedValue({ data: owned, missing: false });
    await drawReady({ settings: owned, chatTier: 'public' });
    fireEvent.click(screen.getByRole('switch', { name: 'Secrets' }));
    const saved = await savedMonitors();
    expect(saved[0]).toMatchObject({ setBy: 'agent', setBySession: 'parent-1' });
    expect(saved[1]).toMatchObject({ setBy: 'person', inheritedFrom: 'parent-1', enabled: false });
  });

  it('makes an agent’s monitor the person’s once the person changes it, not when saved unchanged', async () => {
    const owned = ownedSettings();
    mocks.settings.mockResolvedValue({ data: owned, missing: false });
    await drawReady({ settings: owned, chatTier: 'public' });
    await rowAction('Drift', 'Edit');
    saveEditor();
    expect((await savedMonitors(1))[0]).toMatchObject({ setBy: 'agent' });
    await waitFor(() => expect(screen.queryByTestId('monitor-editor')).toBeNull());
    expect(screen.getByTestId('monitor-set-by-agent')).toBeInTheDocument();

    await rowAction('Drift', 'Edit');
    fireEvent.change(screen.getByTestId('monitor-name'), { target: { value: 'Off task' } });
    saveEditor();
    const saved = await savedMonitors(2);
    expect(saved[0]).toMatchObject({ name: 'Off task', setBy: 'person', setBySession: null });
    expect(decisionSettingsToWire({ defaultModel: null, monitors: saved }).monitors).toEqual([
      expect.not.objectContaining({ setBy: expect.anything() }),
      expect.objectContaining({ inheritedFrom: 'parent-1' }),
    ]);
  });

  it('hides the subagent checkbox on a subagent chat', async () => {
    await drawReady();
    fireEvent.click(screen.getByTestId('monitor-add'));
    openMoreOptions();
    expect(screen.getByTestId('monitor-subagents')).toBeInTheDocument();
    cleanup();
    await drawReady({ isSubagentChat: true });
    fireEvent.click(screen.getByTestId('monitor-add'));
    openMoreOptions();
    expect(screen.queryByTestId('monitor-subagents')).toBeNull();
  });

  it('reads a subagent chat from its monitors when the caller does not say', async () => {
    const owned = ownedSettings();
    mocks.settings.mockResolvedValue({ data: owned, missing: false });
    await drawReady({ settings: owned, chatTier: 'public' });
    fireEvent.click(screen.getByTestId('monitor-add'));
    openMoreOptions();
    expect(screen.queryByTestId('monitor-subagents')).toBeNull();
  });

  it('on a chat that starts subagents, sets a monitor to also watch them', async () => {
    await drawReady();
    fillNewMonitor('Secrets', 'Is a secret in the output?');
    openMoreOptions();
    const box = screen.getByTestId('monitor-subagents');
    expect(box).toHaveTextContent('Also watch the subagents this chat starts');
    fireEvent.click(within(box).getByRole('checkbox'));
    saveEditor();
    expect((await savedMonitors())[1].applyToSubagents).toBe(true);
  });
});

/** The client judges only the monitor being written; the daemon's own check still decides. */
describe('what a write checks', () => {
  const offHosted = () =>
    parseDecisionSettings({
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

  it('never holds a write back for a monitor that is off, whatever its model', async () => {
    const off = offHosted();
    mocks.settings.mockResolvedValue({ data: off, missing: false });
    await drawReady({ settings: off, chatTier: 'private' });
    expect(screen.queryByTestId('monitor-row-error')).toBeNull();
    fillNewMonitor('Signed', 'Has the form been signed?');
    saveEditor();
    expect((await savedMonitors()).map((monitor) => [monitor.id, monitor.enabled])).toEqual([
      ['hosted', false],
      ['signed', true],
    ]);
  });

  it('holds back switching it on, with the reason under its row', async () => {
    const off = offHosted();
    mocks.settings.mockResolvedValue({ data: off, missing: false });
    await drawReady({ settings: off, chatTier: 'private' });
    fireEvent.click(screen.getByRole('switch', { name: 'Hosted' }));
    expect(screen.getByTestId('monitor-row-error')).toHaveTextContent(
      "Public model: it can't read a private chat."
    );
    expect(mocks.save).not.toHaveBeenCalled();
  });

  it('shows a stored monitor that is on and cannot run with its reason, and still writes its siblings', async () => {
    const on = settingsWith(
      { ...offHosted().monitors[0], enabled: true },
      wireMonitor({ id: 'local', name: 'Local' })
    );
    mocks.settings.mockResolvedValue({ data: on, missing: false });
    await drawReady({ settings: on, chatTier: 'private' });
    const [hosted] = screen.getAllByTestId('monitor-item');
    expect(within(hosted).getByTestId('monitor-row-error')).toHaveTextContent(
      "Public model: it can't read a private chat."
    );
    fireEvent.click(screen.getByRole('switch', { name: 'Local' }));
    expect((await savedMonitors())[1]).toMatchObject({ id: 'local', enabled: false });
  });
});

/** A chat with no decision model it may use is offered one to install. */
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
    const install = await within(panel).findByRole('button', { name: 'Install tev1:0.8b' });
    expect(within(panel).getByRole('button', { name: 'About tev1:0.8b' })).toBeInTheDocument();
    expect(mocks.startInstall).not.toHaveBeenCalled();
    fireEvent.click(install);
    expect(mocks.startInstall).toHaveBeenCalledWith('tev1:0.8b');
    await waitFor(() => expect(mocks.catalog).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.queryByTestId('decision-model-install')).toBeNull());
    fireEvent.click(screen.getByTestId('monitor-add'));
    openMoreOptions();
    await openModelMenu();
    const row = modelRows().find((entry) => entry.text.startsWith('tev1:0.8b'))!;
    expect(row.disabled).toBe(false);
    // The old note under the picker is gone: the install panel at the top says it once.
    expect(screen.queryByTestId('monitor-no-usable-model')).toBeNull();
  });

  it('a Crew chat enables local models and disables hosted ones with the sentence', async () => {
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

    mocks.catalog.mockResolvedValue(
      parseDecisionCatalog({
        models: [MODELS[0], { ...MODELS[1], unavailableReason: CREW_HOSTED }],
        sessionUnavailableReason: null,
      })
    );
    await drawReady({ models: undefined, chatTier: 'public' });
    await waitFor(() => expect(mocks.catalog).toHaveBeenCalled());
    fireEvent.click(screen.getByTestId('monitor-add'));
    openMoreOptions();
    await openModelMenu();
    const rows = modelRows();
    const hosted = rows.find((row) => row.text.startsWith('joey-1'))!;
    expect(hosted.disabled).toBe(true);
    expect(hosted.visible).toContain(CREW_HOSTED);
    expect(rows.find((row) => row.text.startsWith('tev1:0.8b'))!.disabled).toBe(false);
    expect(rows[0].disabled).toBe(false);
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
