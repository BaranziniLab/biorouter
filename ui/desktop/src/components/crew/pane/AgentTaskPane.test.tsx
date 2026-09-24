import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  connection,
  currentCrew,
  general,
  installDaemon,
  installObserver,
  makeSnapshot,
  message,
  methods,
  oldNotes,
  renderCrew,
  stateFrame,
  type FixtureSnapshot,
} from '../channel/crewTestHarness';
import { useCrew } from '../state/CrewControllerContext';
import { agentCopy, LONG_TASK_LINES } from './copy';
import { DetailsPane } from './DetailsPane';
import { mentionedFileNames, unsharedFileNames } from './presentation';

const mocks = vi.hoisted(() => ({
  crewHttp: vi.fn(),
  crewRequest: vi.fn(),
  observeCrew: vi.fn(),
  getProviders: vi.fn(),
  read: vi.fn(),
  getProviderModels: vi.fn(),
  navigate: vi.fn(),
}));

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return {
    ...actual,
    crewHttp: mocks.crewHttp,
    crewRequest: mocks.crewRequest,
    observeCrew: mocks.observeCrew,
  };
});
vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({
    getProviders: mocks.getProviders,
    read: mocks.read,
    getProviderModels: mocks.getProviderModels,
  }),
  usePrivacyTiersEnabled: () => true,
}));
vi.mock('react-router-dom', async () => {
  const actual = await vi.importActual<typeof import('react-router-dom')>('react-router-dom');
  return { ...actual, useNavigate: () => mocks.navigate };
});

const fixtureProvider = { name: 'fixture-provider', is_configured: true };
const versa = {
  name: 'versa_azure',
  is_configured: true,
  resolved_tier: 'private',
  affiliation: { kind: 'institutions', institutions: [{ id: 'ucsf', display_name: 'UCSF' }] },
  metadata: { display_name: 'Versa', known_models: [{ name: 'gpt-5.5' }] },
};
const openRouter = {
  name: 'openrouter',
  is_configured: true,
  resolved_tier: 'public',
  metadata: { display_name: 'OpenRouter', known_models: [{ name: 'free-model' }] },
};
const ollama = {
  name: 'ollama',
  is_configured: true,
  resolved_tier: 'private',
  affiliation: { kind: 'local', institutions: [] },
  metadata: { display_name: 'Ollama', known_models: [{ name: 'qwen3.6' }] },
};
const labGateway = {
  name: 'lab_gateway',
  is_configured: true,
  resolved_tier: 'private',
  affiliation: { kind: 'unstated', institutions: [] },
  metadata: { display_name: 'Lab gateway', known_models: [{ name: 'lab-model' }] },
};

/** The daemon's refusal, word for word (`crew/institution.rs` `check_provider`). */
const DAEMON_REFUSAL =
  "Daemon returned 400: Crew institution does not match the model's resolved affiliation; choose a local model or a model approved for this institution";

/**
 * A workspace whose institution is not the Versa model's: `foreign-synthetic`, as the security
 * critic's `foreign-lab` was. Both the workspace and the connection name it, as a verified
 * observation reports them.
 */
function installForeignWorkspace(
  overrides: Partial<FixtureSnapshot> = {},
  connectionInstitution: string | null = 'foreign-synthetic'
) {
  const base = makeSnapshot();
  const snapshot = makeSnapshot({
    workspace: { ...base.workspace, institution_id: 'foreign-synthetic', name: 'foreign-lab' },
    ...overrides,
  });
  mocks.observeCrew.mockImplementation(
    async (
      _connectionId: string,
      channelId: string | undefined,
      _after: string | null,
      signal: AbortSignal,
      receive: (frame: unknown) => void
    ) => {
      if (signal.aborted) return 'terminal';
      receive({ ...stateFrame({ snapshot }), connection_institution_id: connectionInstitution });
      if (channelId) {
        receive({
          type: 'messages',
          channel_id: channelId,
          messages: [],
          cursor: null,
          reset: true,
        });
      }
      return 'terminal';
    }
  );
}

function defaultModel(provider: string, model: string) {
  mocks.read.mockImplementation(async (key: string) =>
    key === 'BIOROUTER_PROVIDER' ? provider : key === 'BIOROUTER_MODEL' ? model : ''
  );
}

/** `pane.css` without its comments, for the rules jsdom never applies. */
const paneCss = readFileSync(join(__dirname, 'pane.css'), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
function cssRule(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  return new RegExp(`(^|\\n)${escaped}\\s*\\{([^}]*)\\}`).exec(paneCss)?.[2] ?? '';
}

function Layout() {
  const crew = useCrew();
  return (
    <div className="crew-stage">
      <section className="crew-channel">
        <textarea
          aria-label="Message #general"
          value={crew.draft.body}
          onChange={(event) => crew.setBody(event.target.value)}
        />
        <button type="button" onClick={() => crew.openPane({ mode: 'agent' })}>
          Ask my agent
        </button>
      </section>
      <DetailsPane />
    </div>
  );
}

const startButton = () => screen.getByRole('button', { name: agentCopy.start });

function runPosts() {
  return mocks.crewHttp.mock.calls.filter(
    ([path, method]) => path === `/connections/${connection.id}/runs` && method === 'POST'
  );
}

async function openAgent(user: ReturnType<typeof userEvent.setup>) {
  await waitFor(() => expect(currentCrew().status).toBe('connected'));
  await waitFor(() => expect(currentCrew().channel?.id).toBe(general.id));
  await user.click(screen.getByRole('button', { name: 'Ask my agent' }));
  return screen.findByLabelText(agentCopy.task);
}

async function chooseModel(user: ReturnType<typeof userEvent.setup>, model: string) {
  await user.click(screen.getByRole('button', { name: /^Model/ }));
  await user.click(await screen.findByRole('option', { name: new RegExp(`^${model}`) }));
}

beforeEach(() => {
  vi.clearAllMocks();
  installDaemon();
  installObserver();
  mocks.getProviders.mockResolvedValue([fixtureProvider]);
  mocks.getProviderModels.mockResolvedValue(['fixture-model']);
  mocks.read.mockResolvedValue('');
  let next = 1;
  vi.spyOn(globalThis.crypto, 'randomUUID').mockImplementation(
    () => `00000000-0000-4000-8000-${String(next++).padStart(12, '0')}`
  );
});

afterEach(() => {
  delete (window as unknown as { appConfig?: unknown }).appConfig;
});

describe('AgentTaskPane', () => {
  it('names the destination and the scope, and seeds Task once from the composer draft', async () => {
    const user = userEvent.setup();
    renderCrew(Layout);
    await waitFor(() => expect(currentCrew().status).toBe('connected'));
    fireEvent.change(screen.getByLabelText('Message #general'), {
      target: { value: 'Plot counts by sample' },
    });
    const task = await openAgent(user);
    expect(task).toHaveValue('Plot counts by sample');
    expect(task).toHaveFocus();
    expect(screen.getByText('Posts to #general in Analysis Lab · lab')).toBeInTheDocument();
    expect(screen.getByText(agentCopy.scope('#general'))).toBeInTheDocument();

    // Task has its own state: editing it leaves the draft alone, and the draft does not follow.
    fireEvent.change(task, { target: { value: 'Plot counts and post the figure' } });
    expect(screen.getByLabelText('Message #general')).toHaveValue('Plot counts by sample');
    fireEvent.change(screen.getByLabelText('Message #general'), { target: { value: 'hello' } });
    expect(task).toHaveValue('Plot counts and post the figure');
  });

  it('starts with the chosen model and the verified epochs, then clears Task and the seeded draft', async () => {
    const user = userEvent.setup();
    renderCrew(Layout);
    await waitFor(() => expect(currentCrew().status).toBe('connected'));
    fireEvent.change(screen.getByLabelText('Message #general'), { target: { value: 'run it' } });
    await openAgent(user);
    await chooseModel(user, 'fixture-model');
    expect(screen.getByRole('button', { name: /^Model/ })).toHaveAccessibleName(
      'Model fixture-model · fixture-provider'
    );
    await user.click(startButton());

    await waitFor(() => expect(runPosts()).toHaveLength(1));
    expect(runPosts()[0][2]).toEqual({
      expected_mode: 'private',
      expected_policy_epoch: 1,
      expected_workspace_policy_epoch: 1,
      channel_id: general.id,
      prompt: 'run it',
      provider: 'fixture-provider',
      model: 'fixture-model',
      context_channels: [general.id],
      posting_grant: true,
      request_id: '00000000-0000-4000-8000-000000000001',
    });
    await waitFor(() => expect(currentCrew().ui.pane).toBeNull());
    expect(screen.getByLabelText('Message #general')).toHaveValue('');
  });

  it('keeps a draft written after the pane opened', async () => {
    const user = userEvent.setup();
    renderCrew(Layout);
    await waitFor(() => expect(currentCrew().status).toBe('connected'));
    fireEvent.change(screen.getByLabelText('Message #general'), { target: { value: 'seed' } });
    await openAgent(user);
    fireEvent.change(screen.getByLabelText('Message #general'), { target: { value: 'newer' } });
    await chooseModel(user, 'fixture-model');
    await user.click(startButton());
    await waitFor(() => expect(currentCrew().ui.pane).toBeNull());
    expect(screen.getByLabelText('Message #general')).toHaveValue('newer');
  });

  it('keeps the same Start node across a failure and shows the error once, in the pane', async () => {
    const user = userEvent.setup();
    let starts = 0;
    const requestIds: string[] = [];
    mocks.crewHttp.mockImplementation(
      async (path: string, method = 'GET', body?: { request_id?: string }) => {
        if (path === '/connections') return { connections: [connection] };
        if (path === `/connections/${connection.id}/runs` && method === 'POST') {
          starts += 1;
          requestIds.push(body?.request_id ?? '');
          if (starts === 1) throw new Error('start failed');
          return {};
        }
        return {};
      }
    );
    renderCrew(Layout);
    const task = await openAgent(user);
    fireEvent.change(task, { target: { value: 'retry me' } });
    await chooseModel(user, 'fixture-model');
    const start = startButton();
    await user.click(start);
    expect(await screen.findAllByText('start failed')).toHaveLength(1);
    const pane = screen.getByRole('complementary', { name: agentCopy.title });
    expect(within(pane).getByRole('alert')).toHaveTextContent('start failed');
    expect(startButton()).toBe(start);

    await user.click(start);
    await waitFor(() => expect(starts).toBe(2));
    expect(requestIds[1]).toBe(requestIds[0]);
  });

  it('refuses to start without a model, on the field', async () => {
    const user = userEvent.setup();
    renderCrew(Layout);
    const task = await openAgent(user);
    fireEvent.change(task, { target: { value: 'no model' } });
    await user.click(startButton());
    expect(await screen.findByText(agentCopy.modelRequired)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /^Model/ })).toHaveAttribute('aria-invalid', 'true');
    expect(screen.getByRole('button', { name: /^Model/ })).toHaveFocus();
    expect(runPosts()).toHaveLength(0);
  });

  describe('Task', () => {
    it('says the task itself is posted in the channel, on the field (T-24)', async () => {
      const user = userEvent.setup();
      renderCrew(Layout);
      const task = await openAgent(user);
      expect(task).toHaveAccessibleDescription(agentCopy.taskPosted('#general'));
      expect(agentCopy.taskPosted('#general')).toBe(
        'Your task is posted in #general so everyone there can see what your agent was asked.'
      );
    });

    it('adds that long pasted data will be visible, once the task reads as pasted data', async () => {
      const user = userEvent.setup();
      renderCrew(Layout);
      const task = await openAgent(user);
      const rows = (count: number) =>
        Array.from({ length: count }, (_, index) => `sample_${index},0.4${index}`).join('\n');

      fireEvent.change(task, { target: { value: rows(LONG_TASK_LINES) } });
      expect(task).toHaveAccessibleDescription(agentCopy.taskPosted('#general'));

      fireEvent.change(task, { target: { value: rows(LONG_TASK_LINES + 1) } });
      expect(task).toHaveAccessibleDescription(
        `${agentCopy.taskPosted('#general')} ${agentCopy.taskLong}`
      );
    });

    it('is at least six lines, grows with its text, and shows focus with the accent edge (T-16)', async () => {
      const user = userEvent.setup();
      renderCrew(Layout);
      const task = await openAgent(user);
      expect(task).toHaveAttribute('rows', '6');
      expect(task).toHaveClass('crew-agent-task');
      // Focus used to REMOVE the hover ring and leave nothing (`focus:inset-ring-0`).
      expect(task.className).not.toMatch(/focus:inset-ring-0/);
      expect(task.className).not.toMatch(/(^|\s)focus(-visible)?:/);

      // jsdom applies no stylesheet, so the authored rules are held at the source.
      expect(cssRule('.crew-agent-task:focus-visible')).toMatch(
        /border-color:\s*var\(--border-accent\);/
      );
      const sizing = cssRule('.crew-agent-task');
      expect(sizing).toMatch(/field-sizing:\s*content;/);
      expect(sizing).toMatch(/min-height:\s*calc\(6lh \+ 14px\);/);
    });

    it('sizes itself and offers no native resize grip (Q2-30)', async () => {
      const user = userEvent.setup();
      renderCrew(Layout);
      const task = await openAgent(user);
      // The grip was Tailwind's `resize-y`; the field grows by itself (`field-sizing: content`).
      expect(task.className).not.toMatch(/(^|\s)resize(-[xy])?(\s|$)/);
      expect(cssRule('.crew-agent-task')).toMatch(/resize:\s*none;/);
    });
  });

  it('leaves an empty Task to native validation', async () => {
    const user = userEvent.setup();
    renderCrew(Layout);
    const task = await openAgent(user);
    fireEvent.change(task, { target: { value: '' } });
    await chooseModel(user, 'fixture-model');
    await user.click(startButton());
    expect(task).toBeInvalid();
    expect(runPosts()).toHaveLength(0);
  });

  describe('Model', () => {
    it('summarizes the app’s default model, with Change opening the picker', async () => {
      const user = userEvent.setup();
      mocks.getProviders.mockResolvedValue([versa, openRouter]);
      mocks.read.mockImplementation(async (key: string) =>
        key === 'BIOROUTER_PROVIDER' ? 'versa_azure' : key === 'BIOROUTER_MODEL' ? 'gpt-5.5' : ''
      );
      renderCrew(Layout);
      const task = await openAgent(user);
      expect(await screen.findByText('gpt-5.5 · Versa')).toBeInTheDocument();
      expect(screen.getByRole('group', { name: agentCopy.model })).toHaveTextContent(
        'gpt-5.5 · Versa'
      );
      expect(screen.queryByRole('button', { name: /^Model/ })).toBeNull();
      // It wraps rather than ellipsizing: the provider was the part a truncation cut (T-47).
      expect(screen.getByText('gpt-5.5 · Versa')).not.toHaveClass('truncate');
      // The label sits above the value, as Task's does, not centred beside what wraps (Q2-67).
      const group = screen.getByRole('group', { name: agentCopy.model });
      const label = within(group).getByText(agentCopy.model);
      expect(label.parentElement).toBe(group);
      expect(label.nextElementSibling).toContainElement(screen.getByText('gpt-5.5 · Versa'));
      expect(group).toHaveClass('flex-col');
      // One mark with words (Q2-67). #general is Restricted in a Private workspace, so only a
      // private model can run this task, and the mark says so in the task's words.
      const mark = within(group).getByTitle(agentCopy.privateOnly);
      expect(mark).toHaveTextContent('Private · UCSF');
      expect(within(group).queryByTestId('affiliation-badge')).toBeNull();
      expect(group.innerHTML).not.toMatch(/this chat/);

      fireEvent.change(task, { target: { value: 'use the default' } });
      await user.click(startButton());
      await waitFor(() => expect(runPosts()).toHaveLength(1));
      expect(runPosts()[0][2]).toMatchObject({ provider: 'versa_azure', model: 'gpt-5.5' });
    });

    it('opens the picker from Change with the default already chosen', async () => {
      const user = userEvent.setup();
      mocks.getProviders.mockResolvedValue([versa, openRouter]);
      mocks.read.mockImplementation(async (key: string) =>
        key === 'BIOROUTER_PROVIDER' ? 'versa_azure' : key === 'BIOROUTER_MODEL' ? 'gpt-5.5' : ''
      );
      renderCrew(Layout);
      await openAgent(user);
      await user.click(await screen.findByRole('button', { name: agentCopy.modelChangeName }));
      expect(
        await screen.findByRole('listbox', { name: agentCopy.modelsLabel })
      ).toBeInTheDocument();
      expect(screen.getByRole('button', { name: /^Model/ })).toHaveAccessibleName(
        'Model gpt-5.5 · Versa'
      );
      await user.click(screen.getByRole('option', { name: /^free-model/ }));
      expect(screen.getByRole('button', { name: /^Model/ })).toHaveAccessibleName(
        'Model free-model · OpenRouter'
      );
    });

    it('falls back to the picker when the default names an unconfigured provider', async () => {
      const user = userEvent.setup();
      mocks.read.mockImplementation(async (key: string) =>
        key === 'BIOROUTER_PROVIDER' ? 'versa_azure' : key === 'BIOROUTER_MODEL' ? 'gpt-5.5' : ''
      );
      renderCrew(Layout);
      await openAgent(user);
      expect(await screen.findByRole('button', { name: /^Model/ })).toHaveAccessibleName(
        `Model ${agentCopy.modelEmpty}`
      );
    });

    it('says no models are set up, points to Settings and cannot start', async () => {
      const user = userEvent.setup();
      mocks.getProviders.mockResolvedValue([{ name: 'unused', is_configured: false }]);
      renderCrew(Layout);
      await openAgent(user);
      expect(await screen.findByText(agentCopy.noModels)).toBeInTheDocument();
      await user.click(screen.getByRole('button', { name: agentCopy.openSettings }));
      expect(mocks.navigate).toHaveBeenCalledWith('/settings', { state: { section: 'models' } });
      expect(startButton()).toBeDisabled();
    });

    it('reports a provider list that could not load in the pane, without claiming none exist', async () => {
      const user = userEvent.setup();
      mocks.getProviders.mockRejectedValue(new Error('providers unavailable'));
      renderCrew(Layout);
      await openAgent(user);
      expect(await screen.findByText('providers unavailable')).toBeInTheDocument();
      expect(screen.queryByText(agentCopy.noModels)).toBeNull();
    });

    it('names the model as the chat composer’s model chip does (T-47)', async () => {
      const user = userEvent.setup();
      // The composer's helper reads the predefined list the host provides.
      (window as unknown as { appConfig: unknown }).appConfig = {
        get: (key: string) =>
          key === 'BIOROUTER_PREDEFINED_MODELS'
            ? JSON.stringify([
                {
                  id: 1,
                  name: 'gpt-5.5-2026-04-24',
                  provider: 'versa_azure',
                  alias: 'GPT-5.5',
                  subtext: 'Versa',
                },
              ])
            : undefined,
      };
      mocks.getProviders.mockResolvedValue([
        { ...versa, metadata: { display_name: 'Versa API Azure', known_models: [] } },
      ]);
      defaultModel('versa_azure', 'gpt-5.5-2026-04-24');
      renderCrew(Layout);
      await openAgent(user);
      expect(await screen.findByText('GPT-5.5 · Versa')).toBeInTheDocument();
      expect(screen.queryByText(/gpt-5\.5-2026-04-24/)).toBeNull();
    });

    it('warns when a Public model is chosen for a Restricted channel', async () => {
      const user = userEvent.setup();
      mocks.getProviders.mockResolvedValue([versa, openRouter]);
      renderCrew(Layout);
      await openAgent(user);
      expect(screen.queryByText(agentCopy.publicHint)).toBeNull();
      await chooseModel(user, 'free-model');
      expect(screen.getByText(agentCopy.publicHint)).toBeInTheDocument();
      await chooseModel(user, 'gpt-5.5');
      expect(screen.queryByText(agentCopy.publicHint)).toBeNull();
    });
  });

  describe('Institution (T-47)', () => {
    const mismatch = agentCopy.institutionMismatch(
      'gpt-5.5',
      'UCSF',
      'foreign-lab',
      'foreign-synthetic'
    );

    it('says before Start that the model is not approved here, and disables Start', async () => {
      const user = userEvent.setup();
      installForeignWorkspace();
      mocks.getProviders.mockResolvedValue([versa, ollama]);
      defaultModel('versa_azure', 'gpt-5.5');
      renderCrew(Layout);
      const task = await openAgent(user);
      expect(await screen.findByText(mismatch)).toBeInTheDocument();
      expect(mismatch).toBe(
        'gpt-5.5 is approved for UCSF. foreign-lab uses foreign-synthetic. Choose a model approved for foreign-synthetic, or a local model.'
      );
      expect(startButton()).toBeDisabled();
      expect(startButton()).toHaveAccessibleDescription(mismatch);
      fireEvent.change(task, { target: { value: 'sum the columns' } });
      fireEvent.click(startButton());
      expect(runPosts()).toHaveLength(0);
      expect(screen.queryByText(/resolved affiliation/)).toBeNull();
    });

    it('marks such models in the picker, and a local model starts', async () => {
      const user = userEvent.setup();
      installForeignWorkspace();
      mocks.getProviders.mockResolvedValue([versa, ollama]);
      defaultModel('versa_azure', 'gpt-5.5');
      renderCrew(Layout);
      const task = await openAgent(user);
      await user.click(await screen.findByRole('button', { name: agentCopy.modelChangeName }));
      const list = await screen.findByRole('listbox', { name: agentCopy.modelsLabel });
      const notHere = agentCopy.notApproved('foreign-synthetic');
      expect(within(list).getByRole('group', { name: /Versa/ })).toHaveTextContent(notHere);
      expect(within(list).getByRole('option', { name: /^gpt-5\.5/ })).toHaveTextContent(notHere);
      expect(within(list).getByRole('group', { name: /Ollama/ })).not.toHaveTextContent(notHere);

      await user.click(within(list).getByRole('option', { name: /^qwen3\.6/ }));
      expect(screen.queryByText(mismatch)).toBeNull();
      fireEvent.change(task, { target: { value: 'sum the columns' } });
      expect(startButton()).toBeEnabled();
      await user.click(startButton());
      await waitFor(() => expect(runPosts()).toHaveLength(1));
      expect(runPosts()[0][2]).toMatchObject({ provider: 'ollama', model: 'qwen3.6' });
    });

    it('explains a private model that states no institution the same way', async () => {
      const user = userEvent.setup();
      installForeignWorkspace();
      mocks.getProviders.mockResolvedValue([labGateway]);
      defaultModel('lab_gateway', 'lab-model');
      renderCrew(Layout);
      await openAgent(user);
      expect(
        await screen.findByText(
          agentCopy.institutionUnstated('lab-model', 'foreign-lab', 'foreign-synthetic')
        )
      ).toBeInTheDocument();
      expect(startButton()).toBeDisabled();
    });

    it('leaves Start to the daemon where it would not ask about the institution', async () => {
      const user = userEvent.setup();
      // A public workspace, a public connection and a public-safe channel: nothing is protected, so
      // the daemon holds no model to the institution, and neither does the pane.
      const base = makeSnapshot();
      installForeignWorkspace({
        workspace: {
          ...base.workspace,
          mode: 'public',
          institution_id: 'foreign-synthetic',
          name: 'foreign-lab',
        },
        channels: [{ ...general, classification: 'public_safe' }, oldNotes],
      });
      installDaemon([{ ...connection, mode: 'public' }]);
      mocks.getProviders.mockResolvedValue([versa]);
      defaultModel('versa_azure', 'gpt-5.5');
      renderCrew(Layout);
      const task = await openAgent(user);
      await screen.findByText('gpt-5.5 · Versa');
      expect(screen.queryByText(mismatch)).toBeNull();
      fireEvent.change(task, { target: { value: 'public work' } });
      expect(startButton()).toBeEnabled();
      await user.click(startButton());
      await waitFor(() => expect(runPosts()).toHaveLength(1));
    });

    it('rewords the daemon’s "resolved affiliation" refusal, and drops it for a new choice', async () => {
      const user = userEvent.setup();
      // The pane thinks Versa is fine here (a UCSF workspace); the daemon decides otherwise.
      mocks.crewHttp.mockImplementation(async (path: string, method = 'GET') => {
        if (path === '/connections') return { connections: [connection] };
        if (path === `/connections/${connection.id}/runs` && method === 'POST') {
          throw new Error(DAEMON_REFUSAL);
        }
        return {};
      });
      mocks.getProviders.mockResolvedValue([versa, ollama]);
      defaultModel('versa_azure', 'gpt-5.5');
      renderCrew(Layout);
      const task = await openAgent(user);
      await screen.findByText('gpt-5.5 · Versa');
      fireEvent.change(task, { target: { value: 'try it' } });
      await user.click(startButton());
      await waitFor(() => expect(runPosts()).toHaveLength(1));
      const pane = screen.getByRole('complementary', { name: agentCopy.title });
      expect(await within(pane).findByRole('alert')).toHaveTextContent(
        agentCopy.institutionRefused('gpt-5.5', 'UCSF')
      );
      expect(screen.queryByText(/resolved affiliation/)).toBeNull();

      await user.click(screen.getByRole('button', { name: agentCopy.modelChangeName }));
      await user.click(await screen.findByRole('option', { name: /^qwen3\.6/ }));
      await waitFor(() => expect(within(pane).queryByRole('alert')).toBeNull());
      expect(currentCrew().error).toBeNull();
    });

    it('still asks the daemon for a model whose institution it cannot see', async () => {
      const user = userEvent.setup();
      installForeignWorkspace();
      mocks.crewHttp.mockImplementation(async (path: string, method = 'GET') => {
        if (path === '/connections') return { connections: [connection] };
        if (path === `/connections/${connection.id}/runs` && method === 'POST') {
          throw new Error(DAEMON_REFUSAL);
        }
        return {};
      });
      renderCrew(Layout);
      const task = await openAgent(user);
      fireEvent.change(task, { target: { value: 'try it' } });
      await chooseModel(user, 'fixture-model');
      expect(startButton()).toBeEnabled();
      await user.click(startButton());
      await waitFor(() => expect(runPosts()).toHaveLength(1));
      expect(await screen.findByRole('alert')).toHaveTextContent(
        agentCopy.institutionRefused('fixture-model', 'foreign-synthetic')
      );
    });
  });

  describe('Advanced', () => {
    it('is not offered when it has nothing to add', async () => {
      const user = userEvent.setup();
      installObserver({ snapshot: makeSnapshot({ channels: [general, oldNotes] }) });
      renderCrew(Layout);
      await openAgent(user);
      expect(screen.queryByRole('button', { name: 'Advanced' })).toBeNull();
      expect(screen.queryByText(agentCopy.advancedSummary('#general', []))).toBeNull();
    });

    it('keeps Also read unmounted while closed and summarizes what it adds', async () => {
      const user = userEvent.setup();
      renderCrew(Layout);
      const task = await openAgent(user);
      expect(screen.queryByRole('checkbox')).toBeNull();
      // It says what the agent reads, never "Also reads nothing else" (Q2-67).
      expect(screen.getByText('Reads only #general')).toBeInTheDocument();
      expect(agentCopy.advancedSummary('#general', [])).toBe('Reads only #general');
      expect(screen.getByRole('button', { name: 'Advanced' })).toHaveAccessibleDescription(
        'Reads only #general'
      );

      await user.click(screen.getByRole('button', { name: 'Advanced' }));
      const also = screen.getByRole('checkbox', { name: '#methods' });
      // Archived channels and the current channel are not offered.
      expect(screen.queryByRole('checkbox', { name: '#old-notes' })).toBeNull();
      expect(screen.queryByRole('checkbox', { name: '#general' })).toBeNull();
      await user.click(also);
      await user.click(screen.getByRole('button', { name: 'Advanced' }));
      expect(screen.getByText('Also reads #methods')).toBeInTheDocument();
      expect(screen.queryByText(/Reads only/)).toBeNull();

      fireEvent.change(task, { target: { value: 'read more' } });
      await chooseModel(user, 'fixture-model');
      await user.click(startButton());
      await waitFor(() => expect(runPosts()).toHaveLength(1));
      expect(runPosts()[0][2]).toMatchObject({ context_channels: [general.id, methods.id] });
    });

    it('states the remote folder only when the connection has one', async () => {
      const user = userEvent.setup();
      installDaemon([
        { ...connection, remote_root: '/home/alice/crew-work', remote_execution: true },
      ]);
      renderCrew(Layout);
      await openAgent(user);
      // Closed, the summary names the folder too, and so never claims "Reads only".
      expect(screen.getByRole('button', { name: 'Advanced' })).toHaveAccessibleDescription(
        agentCopy.folderExec('/home/alice/crew-work')
      );
      expect(screen.queryByText(/Reads only/)).toBeNull();
      await user.click(screen.getByRole('button', { name: 'Advanced' }));
      expect(screen.getByText(agentCopy.folderExec('/home/alice/crew-work'))).toBeInTheDocument();
    });

    it('names each channel it also reads, then counts the rest (Q2-67)', () => {
      expect(agentCopy.advancedSummary('#general', ['#methods', 'Lab / #qc'])).toBe(
        'Also reads #methods, Lab / #qc'
      );
      expect(agentCopy.advancedSummary('#general', ['#a', '#b', '#c', '#d', '#e'])).toBe(
        'Also reads #a, #b, #c and 2 more'
      );
      expect(agentCopy.advancedSummary('#general', ['#a'], 'Can read /srv/x')).toBe(
        'Also reads #a · Can read /srv/x'
      );
    });
  });

  describe('Start (Q2-67)', () => {
    it('reads "Starting…" beside a spinner from the click until the pane closes', async () => {
      const user = userEvent.setup();
      let accept: (value: unknown) => void = () => undefined;
      mocks.crewHttp.mockImplementation(async (path: string, method = 'GET') => {
        if (path === '/connections') return { connections: [connection] };
        if (path === `/connections/${connection.id}/runs` && method === 'POST') {
          return new Promise((resolve) => {
            accept = resolve;
          });
        }
        return {};
      });
      renderCrew(Layout);
      const task = await openAgent(user);
      fireEvent.change(task, { target: { value: 'sum the columns' } });
      await chooseModel(user, 'fixture-model');
      const start = startButton();
      await user.click(start);

      await waitFor(() => expect(start).toHaveTextContent(agentCopy.starting));
      expect(start).toBeDisabled();
      expect(start.querySelector('.crew-agent-start-spinner')).not.toBeNull();
      expect(within(start).queryByText(agentCopy.start)).toBeNull();
      // Disabling Start takes focus off it, so the words are also spoken.
      const spoken = document.querySelector('[data-crew-agent-start-status]');
      expect(spoken).toHaveAttribute('aria-live', 'polite');
      expect(spoken).toHaveTextContent(agentCopy.starting);
      expect(agentCopy.starting).toBe('Starting…');

      await act(async () => accept({ run_id: 'run-1', session_id: 'session-1' }));
      await waitFor(() => expect(currentCrew().ui.pane).toBeNull());
    });

    it('gives Start its words back when the start fails', async () => {
      const user = userEvent.setup();
      mocks.crewHttp.mockImplementation(async (path: string, method = 'GET') => {
        if (path === '/connections') return { connections: [connection] };
        if (path === `/connections/${connection.id}/runs` && method === 'POST') {
          throw new Error('start failed');
        }
        return {};
      });
      renderCrew(Layout);
      const task = await openAgent(user);
      fireEvent.change(task, { target: { value: 'sum the columns' } });
      await chooseModel(user, 'fixture-model');
      const start = startButton();
      await user.click(start);
      expect(await screen.findAllByText('start failed')).toHaveLength(1);
      expect(start).toHaveTextContent(agentCopy.start);
      expect(start).toBeEnabled();
      expect(start.querySelector('.crew-agent-start-spinner')).toBeNull();
    });

    it('authors the spinner’s motion in pane.css, with a still rest under reduced motion', () => {
      expect(cssRule('.crew-agent-start-spinner')).toMatch(/animation:\s*crew-pane-spin\b/);
      expect(paneCss).toMatch(
        /prefers-reduced-motion:\s*reduce[\s\S]*\.crew-agent-start-spinner\s*\{\s*animation:\s*none;/
      );
    });
  });

  describe('a file the task names (Q2-15)', () => {
    const warning = (names: string[]) => agentCopy.fileNotShared(names, '#general');
    const fileWarning = () => screen.queryByTestId('crew-agent-file-warning');

    /** `blob.status` and `reference.get` as the daemon answers them for these fixtures. */
    function installFiles(
      blobs: Record<string, string>,
      references: Record<string, { label: string; path: string }> = {}
    ) {
      mocks.crewRequest.mockImplementation(
        async (_id: string, method: string, params?: Record<string, unknown>) => {
          if (method === 'messages.history') return { messages: [], cursor: null };
          if (method === 'blob.status') {
            const id = String(params?.blob_id);
            if (!(id in blobs)) throw new Error('unknown blob');
            return { id, channel_id: general.id, name: blobs[id], complete: true };
          }
          if (method === 'reference.get') {
            const reference = references[String(params?.reference_id)];
            if (!reference) throw new Error('unknown reference');
            return { ...reference, verified: false };
          }
          return {};
        }
      );
    }

    it('warns before Start when no message in the channel shares it, and still lets it start', async () => {
      const user = userEvent.setup();
      renderCrew(Layout);
      const task = await openAgent(user);
      fireEvent.change(task, {
        target: {
          value: 'Compute the means in my plate reader file (dave-plate-reader.csv). Use the file.',
        },
      });
      const note = await screen.findByTestId('crew-agent-file-warning');
      expect(note).toHaveTextContent(warning(['dave-plate-reader.csv']));
      expect(warning(['dave-plate-reader.csv'])).toBe(
        'No file named dave-plate-reader.csv is shared in #general. Your agent will say what it used instead.'
      );
      // It is before Start in the footer, and describes it.
      const pane = screen.getByRole('complementary', { name: agentCopy.title });
      expect(note.compareDocumentPosition(startButton()) & Node.DOCUMENT_POSITION_FOLLOWING).toBe(
        Node.DOCUMENT_POSITION_FOLLOWING
      );
      expect(pane).toContainElement(note);
      expect(startButton()).toHaveAccessibleDescription(warning(['dave-plate-reader.csv']));

      // A warning, not a gate.
      await chooseModel(user, 'fixture-model');
      expect(startButton()).toBeEnabled();
      await user.click(startButton());
      await waitFor(() => expect(runPosts()).toHaveLength(1));
    });

    it('says nothing for a file the channel shares, whatever its case, and names only the others', async () => {
      const user = userEvent.setup();
      installFiles({ 'blob-1': 'Dave-Plate-Reader.CSV' });
      installObserver({ messages: [{ ...message('1'), attachments: ['blob-1'] }] });
      renderCrew(Layout);
      await waitFor(() => expect(currentCrew().messages).toHaveLength(1));
      const task = await openAgent(user);
      fireEvent.change(task, { target: { value: 'Average dave-plate-reader.csv' } });
      await waitFor(() =>
        expect(mocks.crewRequest).toHaveBeenCalledWith(
          connection.id,
          'blob.status',
          { blob_id: 'blob-1' },
          false,
          expect.any(AbortSignal)
        )
      );
      await act(async () => undefined);
      expect(fileWarning()).toBeNull();

      fireEvent.change(task, {
        target: { value: 'Average dave-plate-reader.csv against layout.xlsx and qc.json' },
      });
      expect(await screen.findByTestId('crew-agent-file-warning')).toHaveTextContent(
        warning(['layout.xlsx', 'qc.json'])
      );
      expect(warning(['layout.xlsx', 'qc.json'])).toBe(
        'No files named layout.xlsx or qc.json are shared in #general. Your agent will say what it used instead.'
      );
      // Each shared file is looked up once, not once per keystroke.
      expect(
        mocks.crewRequest.mock.calls.filter(([, method]) => method === 'blob.status')
      ).toHaveLength(1);
    });

    it('counts a shared server path by its file name', async () => {
      const user = userEvent.setup();
      installFiles({}, { 'ref-1': { label: 'Counts', path: '/data/run7/Counts.TSV' } });
      installObserver({ messages: [{ ...message('1'), references: ['ref-1'] }] });
      renderCrew(Layout);
      await waitFor(() => expect(currentCrew().messages).toHaveLength(1));
      const task = await openAgent(user);
      fireEvent.change(task, { target: { value: 'Plot counts.tsv' } });
      await waitFor(() =>
        expect(mocks.crewRequest.mock.calls.some(([, method]) => method === 'reference.get')).toBe(
          true
        )
      );
      await act(async () => undefined);
      expect(fileWarning()).toBeNull();
    });

    it('stays quiet while it cannot tell: a shared file whose name could not be read', async () => {
      const user = userEvent.setup();
      installFiles({});
      installObserver({ messages: [{ ...message('1'), attachments: ['blob-gone'] }] });
      renderCrew(Layout);
      await waitFor(() => expect(currentCrew().messages).toHaveLength(1));
      const task = await openAgent(user);
      fireEvent.change(task, { target: { value: 'Average counts.csv' } });
      await waitFor(() =>
        expect(mocks.crewRequest.mock.calls.some(([, method]) => method === 'blob.status')).toBe(
          true
        )
      );
      await act(async () => undefined);
      // "No file named counts.csv is shared" might be false, so it is not said.
      expect(fileWarning()).toBeNull();
    });

    it('says nothing for a shared file whose name has a space, though the task names it unquoted', async () => {
      const user = userEvent.setup();
      installFiles({ 'blob-1': 'Plate Reader.csv' });
      installObserver({ messages: [{ ...message('1'), attachments: ['blob-1'] }] });
      renderCrew(Layout);
      await waitFor(() => expect(currentCrew().messages).toHaveLength(1));
      const task = await openAgent(user);
      // Read as `Reader.csv`: a word holds no space. The shared file answers it all the same.
      fireEvent.change(task, { target: { value: 'Compute OD ratios from Plate Reader.csv' } });
      await waitFor(() =>
        expect(mocks.crewRequest.mock.calls.some(([, method]) => method === 'blob.status')).toBe(
          true
        )
      );
      await act(async () => undefined);
      expect(fileWarning()).toBeNull();

      // Quoted, a name is taken whole, and a warning names it whole.
      fireEvent.change(task, {
        target: { value: 'Compute OD ratios from "Plate Reader.csv" and "OD600 run 2.xlsx"' },
      });
      expect(await screen.findByTestId('crew-agent-file-warning')).toHaveTextContent(
        warning(['OD600 run 2.xlsx'])
      );
    });

    it('names only the unshared file of a command or a quoted list, never the whole of it', async () => {
      const user = userEvent.setup();
      installFiles({ 'blob-1': 'counts.csv', 'blob-2': 'rep1.csv', 'blob-3': 'rep2.csv' });
      installObserver({
        messages: [{ ...message('1'), attachments: ['blob-1', 'blob-2', 'blob-3'] }],
      });
      renderCrew(Layout);
      await waitFor(() => expect(currentCrew().messages).toHaveLength(1));
      const task = await openAgent(user);

      fireEvent.change(task, { target: { value: 'Run `python merge.py counts.csv plate.csv`' } });
      expect(await screen.findByTestId('crew-agent-file-warning')).toHaveTextContent(
        warning(['plate.csv'])
      );
      expect(fileWarning()).not.toHaveTextContent('merge.py');

      fireEvent.change(task, {
        target: { value: 'Compare the replicates in "rep1.csv, rep2.csv, rep3.csv"' },
      });
      await waitFor(() => expect(fileWarning()).toHaveTextContent(warning(['rep3.csv'])));
      expect(warning(['rep3.csv'])).toBe(
        'No file named rep3.csv is shared in #general. Your agent will say what it used instead.'
      );
    });

    it('stays quiet on an older page, whose later messages are not loaded', async () => {
      const user = userEvent.setup();
      // The live tail shares counts.csv; the page before it (answered by `messages.history`) is
      // empty, and short of a full page, so it reads as the channel's start.
      installFiles({ 'blob-1': 'counts.csv' });
      installObserver({ messages: [{ ...message('5'), attachments: ['blob-1'] }] });
      renderCrew(Layout);
      await waitFor(() => expect(currentCrew().messages).toHaveLength(1));
      const task = await openAgent(user);
      act(() => currentCrew().loadOlder());
      await waitFor(() => expect(currentCrew().historyBefore).not.toBeNull());
      await waitFor(() => expect(currentCrew().messagesLoaded).toBe(true));
      expect(currentCrew().messages).toHaveLength(0);
      fireEvent.change(task, { target: { value: 'Average counts.csv' } });
      await act(async () => undefined);
      // counts.csv is shared, in a message this page does not hold.
      expect(fileWarning()).toBeNull();
    });

    it('stays quiet while the loaded messages may not be the whole channel', async () => {
      const user = userEvent.setup();
      installFiles({});
      // A full page: the channel may go further back, where the file could have been shared.
      const page = Array.from({ length: 200 }, (_, index) => message(String(index + 1)));
      installObserver({ messages: page });
      renderCrew(Layout);
      await waitFor(() => expect(currentCrew().messages).toHaveLength(200));
      const task = await openAgent(user);
      fireEvent.change(task, { target: { value: 'Average counts.csv' } });
      await act(async () => undefined);
      expect(fileWarning()).toBeNull();
      expect(
        mocks.crewRequest.mock.calls.filter(([, method]) => method === 'blob.status')
      ).toHaveLength(0);
    });

    it('stays quiet while the opening backlog is still arriving', async () => {
      const user = userEvent.setup();
      installObserver();
      const observe = mocks.observeCrew.getMockImplementation()!;
      mocks.observeCrew.mockImplementation(
        async (
          connectionId: string,
          channelId: string | undefined,
          after: string | null,
          signal: AbortSignal,
          receive: (frame: unknown) => void
        ) =>
          observe(connectionId, channelId, after, signal, (frame: unknown) =>
            // The daemon says more of the opening page is on its way.
            receive(
              (frame as { type?: string }).type === 'messages'
                ? { ...(frame as object), remaining: 3 }
                : frame
            )
          )
      );
      renderCrew(Layout);
      const task = await openAgent(user);
      expect(currentCrew().backlogComplete).toBe(false);
      fireEvent.change(task, { target: { value: 'Average counts.csv' } });
      await act(async () => undefined);
      expect(fileWarning()).toBeNull();
    });

    it('finds the names a task mentions, and not the ones inside a URL', () => {
      expect(
        mentionedFileNames(
          'Use (dave-plate-reader.csv), then Layout.XLSX and layout.xlsx; see https://x.org/ref.json.'
        )
      ).toEqual(['dave-plate-reader.csv', 'Layout.XLSX']);
      expect(mentionedFileNames('counts.csv.gz and counts.csv-old are not counts.csv')).toEqual([
        'counts.csv',
      ]);
      expect(mentionedFileNames('a.tsv b.xls c.txt d.h5ad e.parquet f.pdf')).toEqual([
        'a.tsv',
        'b.xls',
        'c.txt',
        'd.h5ad',
        'e.parquet',
      ]);
      expect(unsharedFileNames(['plate.csv', 'Other.CSV'], ['/data/PLATE.csv'])).toEqual([
        'Other.CSV',
      ]);
    });

    it('compares a name with a space as the task reads it, so the warning is never false', () => {
      // Unquoted, a name ends at a space…
      expect(mentionedFileNames('Compute OD ratios from Plate Reader.csv')).toEqual(['Reader.csv']);
      expect(mentionedFileNames('Plot OD600 run 2.xlsx')).toEqual(['2.xlsx']);
      // …and a shared file with that name answers the shortened mention.
      expect(unsharedFileNames(['Reader.csv'], ['Plate Reader.csv'])).toEqual([]);
      expect(unsharedFileNames(['2.xlsx'], ['/Users/dave/OD600 run 2.xlsx'])).toEqual([]);
      expect(unsharedFileNames(['plate.csv'], ['(v2) plate.csv', 'x(v3)plate.csv'])).toEqual([]);
      expect(unsharedFileNames(['hidden.csv'], ['.hidden.csv'])).toEqual([]);
      // A longer word is a different name: `myplate.csv` and `old-plate.csv` do not answer it.
      expect(unsharedFileNames(['plate.csv'], ['myplate.csv', 'old-plate.csv'])).toEqual([
        'plate.csv',
      ]);
      expect(unsharedFileNames(['counts.csv'], ['counts.csv.gz', 'counts.csv (1)'])).toEqual([
        'counts.csv',
      ]);

      // In double quotes, a name is taken whole and by its file name; a quoted URL is not one.
      expect(
        mentionedFileNames(
          'Use "Plate Reader.csv", “Layout v2.xlsx”, "/data/OD600 run 2.xlsx", "(v2) plate.csv" and "https://x.org/a b.csv"; "not a file" plate.csv'
        )
      ).toEqual([
        'Plate Reader.csv',
        'Layout v2.xlsx',
        'OD600 run 2.xlsx',
        '(v2) plate.csv',
        'plate.csv',
      ]);
      expect(unsharedFileNames(['Plate Reader.csv'], ['plate reader.CSV'])).toEqual([]);
      expect(unsharedFileNames(['Plate Reader.csv'], ['Reader.csv'])).toEqual([]);
      expect(unsharedFileNames(['Plate Reader.csv'], ['Plate Reader.tsv'])).toEqual([
        'Plate Reader.csv',
      ]);
    });

    it('never names a command, a list or a phrase in quotes as if it were one file', () => {
      // Backticks hold code, where a space separates arguments: each file is its own name.
      expect(mentionedFileNames('Run `python merge.py counts.csv plate.csv`')).toEqual([
        'counts.csv',
        'plate.csv',
      ]);
      expect(unsharedFileNames(['counts.csv', 'plate.csv'], ['counts.csv'])).toEqual(['plate.csv']);
      expect(mentionedFileNames('Run `Rscript qc.R counts.csv`')).toEqual(['counts.csv']);
      expect(mentionedFileNames('Run `cat counts.csv` on `OD600 run 2.xlsx`')).toEqual([
        'counts.csv',
        '2.xlsx',
      ]);
      // A name a command quotes is still one name.
      expect(mentionedFileNames('Run `python run.py "Plate Reader.csv"`')).toEqual([
        'Plate Reader.csv',
      ]);

      // Double quotes around a list are not one file named after the whole list…
      expect(
        mentionedFileNames('Compare the replicates in "rep1.csv, rep2.csv, rep3.csv"')
      ).toEqual(['rep1.csv', 'rep2.csv', 'rep3.csv']);
      expect(
        unsharedFileNames(['rep1.csv', 'rep2.csv', 'rep3.csv'], ['rep1.csv', 'rep2.csv'])
      ).toEqual(['rep3.csv']);
      expect(mentionedFileNames('"counts.csv and plate.csv", "a.tsv or b.tsv"')).toEqual([
        'counts.csv',
        'plate.csv',
        'a.tsv',
        'b.tsv',
      ]);
      // …and neither is a command, a flag, a label or a script beside a file.
      expect(
        mentionedFileNames(
          '"python merge.py Plate Reader.csv", "head -n 5 run.csv", "Input: layout.xlsx", "sort < qc.json", "x=1; y.txt"'
        )
      ).toEqual(['Reader.csv', 'run.csv', 'layout.xlsx', 'qc.json', 'y.txt']);
    });
  });

  it('cannot start while the view is being re-verified or the channel is archived', async () => {
    const user = userEvent.setup();
    installObserver({
      snapshot: makeSnapshot({ channels: [{ ...general, archived: true }, methods] }),
    });
    renderCrew(Layout);
    await waitFor(() => expect(currentCrew().status).toBe('connected'));
    act(() => currentCrew().selectChannel(general.id));
    await waitFor(() => expect(currentCrew().channel?.id).toBe(general.id));
    await user.click(screen.getByRole('button', { name: 'Ask my agent' }));
    await screen.findByLabelText(agentCopy.task);
    expect(startButton()).toBeDisabled();
  });
});
