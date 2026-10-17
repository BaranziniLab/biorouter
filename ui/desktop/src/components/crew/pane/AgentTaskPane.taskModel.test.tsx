import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  connection,
  currentCrew,
  general,
  installDaemon,
  installObserver,
  makeSnapshot,
  methods,
  renderCrew,
  stateFrame,
} from '../channel/crewTestHarness';
import { useCrew } from '../state/CrewControllerContext';
import { agentCopy } from './copy';
import { DetailsPane } from './DetailsPane';

const mocks = vi.hoisted(() => ({
  crewHttp: vi.fn(),
  crewRequest: vi.fn(),
  observeCrew: vi.fn(),
  getProviders: vi.fn(),
  read: vi.fn(),
  getProviderModels: vi.fn(),
  upsert: vi.fn(),
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
    upsert: mocks.upsert,
  }),
  usePrivacyTiersEnabled: () => true,
}));
vi.mock('react-router-dom', async () => {
  const actual = await vi.importActual<typeof import('react-router-dom')>('react-router-dom');
  return { ...actual, useNavigate: () => mocks.navigate };
});

/**
 * Ask my agent and the model for Crew tasks (design OW-11), the composer's `@agent` seed (OW-10)
 * and an open workspace's wide reads (`wide_runs_v1`).
 */

const versa = {
  name: 'versa_azure',
  is_configured: true,
  resolved_tier: 'private',
  affiliation: { kind: 'institutions', institutions: [{ id: 'ucsf', display_name: 'UCSF' }] },
  metadata: { display_name: 'Versa', known_models: [{ name: 'gpt-5.5' }] },
};
const ollama = {
  name: 'ollama',
  is_configured: true,
  resolved_tier: 'private',
  affiliation: { kind: 'local', institutions: [] },
  metadata: { display_name: 'Ollama', known_models: [{ name: 'qwen3.8:27b' }] },
};

let values: Record<string, unknown> = {};

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
const runPosts = () =>
  mocks.crewHttp.mock.calls.filter(
    ([path, method]) => path === `/connections/${connection.id}/runs` && method === 'POST'
  );

async function connected() {
  await waitFor(() => expect(currentCrew().status).toBe('connected'));
  await waitFor(() => expect(currentCrew().channel?.id).toBe(general.id));
}

beforeEach(() => {
  vi.clearAllMocks();
  installDaemon();
  installObserver();
  values = {
    BIOROUTER_PROVIDER: 'ollama',
    BIOROUTER_MODEL: 'qwen3.8:27b',
    CREW_TASK_MODEL: { provider: 'versa_azure', model: 'gpt-5.5' },
  };
  mocks.getProviders.mockResolvedValue([versa, ollama]);
  mocks.read.mockImplementation(async (key: string) => values[key] ?? null);
  mocks.getProviderModels.mockResolvedValue([]);
  mocks.upsert.mockImplementation(async (key: string, value: unknown) => {
    values[key] = value;
  });
});

describe('the model a task starts on (OW-11)', () => {
  it('summarizes the model for Crew tasks rather than the app’s, and starts on it', async () => {
    const user = userEvent.setup();
    renderCrew(Layout);
    await connected();
    await user.click(screen.getByRole('button', { name: 'Ask my agent' }));
    const task = await screen.findByLabelText(agentCopy.task);
    expect(await screen.findByText('gpt-5.5 · Versa')).toBeInTheDocument();
    // It is already the model for Crew tasks: nothing to make it so.
    expect(screen.queryByRole('button', { name: agentCopy.useForFutureTasks })).toBeNull();
    fireEvent.change(task, { target: { value: 'tally the plates' } });
    await user.click(startButton());
    await waitFor(() => expect(runPosts()).toHaveLength(1));
    expect(runPosts()[0][2]).toMatchObject({ provider: 'versa_azure', model: 'gpt-5.5' });
  });

  it('falls back to the app’s default while Crew tasks have none', async () => {
    delete values.CREW_TASK_MODEL;
    const user = userEvent.setup();
    renderCrew(Layout);
    await connected();
    await user.click(screen.getByRole('button', { name: 'Ask my agent' }));
    expect(await screen.findByText('qwen3.8:27b · Ollama')).toBeInTheDocument();
  });

  it('makes another model the one for future tasks with a link, not a checkbox (C9)', async () => {
    const user = userEvent.setup();
    renderCrew(Layout);
    await connected();
    await user.click(screen.getByRole('button', { name: 'Ask my agent' }));
    await user.click(await screen.findByRole('button', { name: agentCopy.modelChangeName }));
    await user.click(await screen.findByRole('option', { name: /^qwen3\.8:27b/ }));
    const link = screen.getByRole('button', { name: agentCopy.useForFutureTasks });
    expect(screen.queryByRole('checkbox')).toBeNull();

    await user.click(link);
    expect(mocks.upsert).toHaveBeenCalledWith(
      'CREW_TASK_MODEL',
      { provider: 'ollama', model: 'qwen3.8:27b' },
      false
    );
    // The write is announced, the models are read again, and the link has done its job.
    await waitFor(() =>
      expect(screen.queryByRole('button', { name: agentCopy.useForFutureTasks })).toBeNull()
    );
    expect(screen.getByRole('status')).toHaveTextContent(
      agentCopy.usedForFutureTasks('qwen3.8:27b · Ollama')
    );
  });

  it('says a refused save beside the link, and keeps the choice on show', async () => {
    mocks.upsert.mockRejectedValue(new Error('409 Conflict'));
    const user = userEvent.setup();
    renderCrew(Layout);
    await connected();
    await user.click(screen.getByRole('button', { name: 'Ask my agent' }));
    await user.click(await screen.findByRole('button', { name: agentCopy.modelChangeName }));
    await user.click(await screen.findByRole('option', { name: /^qwen3\.8:27b/ }));
    await user.click(screen.getByRole('button', { name: agentCopy.useForFutureTasks }));
    expect(await screen.findByRole('alert')).toHaveTextContent(
      agentCopy.useForFutureTasksFailed('409 Conflict')
    );
    expect(screen.getByRole('button', { name: /^Model/ })).toHaveAccessibleName(
      'Model qwen3.8:27b · Ollama'
    );
  });
});

describe('opened by the composer’s @agent (OW-10)', () => {
  it('takes the words without the trigger, and a start clears the whole @agent draft', async () => {
    const user = userEvent.setup();
    renderCrew(Layout);
    await connected();
    fireEvent.change(screen.getByLabelText('Message #general'), {
      target: { value: '@agent plot it' },
    });
    act(() =>
      currentCrew().openPane({
        mode: 'agent',
        seed: { task: 'plot it', draft: '@agent plot it' },
      })
    );
    const task = await screen.findByLabelText(agentCopy.task);
    expect(task).toHaveValue('plot it');
    await screen.findByText('gpt-5.5 · Versa');
    await user.click(startButton());
    await waitFor(() => expect(runPosts()).toHaveLength(1));
    expect(runPosts()[0][2]).toMatchObject({ prompt: 'plot it' });
    await waitFor(() => expect(currentCrew().ui.pane).toBeNull());
    expect(screen.getByLabelText('Message #general')).toHaveValue('');
  });

  it('opens on the model picker when the composer’s model could not start the task', async () => {
    renderCrew(Layout);
    await connected();
    act(() =>
      currentCrew().openPane({
        mode: 'agent',
        seed: { task: 'plot it', draft: '@agent plot it', pickModel: true },
      })
    );
    expect(await screen.findByRole('listbox', { name: agentCopy.modelsLabel })).toBeInTheDocument();
    expect(screen.getByLabelText(agentCopy.task)).toHaveValue('plot it');
  });

  it('takes a new seed while it is already open', async () => {
    const user = userEvent.setup();
    renderCrew(Layout);
    await connected();
    await user.click(screen.getByRole('button', { name: 'Ask my agent' }));
    const task = await screen.findByLabelText(agentCopy.task);
    fireEvent.change(task, { target: { value: 'something else' } });
    act(() =>
      currentCrew().openPane({
        mode: 'agent',
        seed: { task: 'summarize counts.csv', draft: '/task summarize counts.csv' },
      })
    );
    await waitFor(() => expect(task).toHaveValue('summarize counts.csv'));
  });
});

describe('an open workspace whose broker reads wide (wide_runs_v1)', () => {
  function installWide(capabilities: string[]) {
    const base = makeSnapshot();
    const snapshot = makeSnapshot({
      workspace: { ...base.workspace, governance: 'open' } as never,
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
        receive({ ...stateFrame({ snapshot }), capabilities });
        if (channelId)
          receive({
            type: 'messages',
            channel_id: channelId,
            messages: [],
            cursor: null,
            reset: true,
          });
        return 'terminal';
      }
    );
  }

  it('reads every channel the person is in by default, says so, and lets them untick one', async () => {
    installWide(['presence_v1', 'wide_runs_v1']);
    const user = userEvent.setup();
    renderCrew(Layout);
    await connected();
    await user.click(screen.getByRole('button', { name: 'Ask my agent' }));
    const task = await screen.findByLabelText(agentCopy.task);
    const advanced = screen.getByRole('button', { name: 'Advanced' });
    expect(advanced).toHaveAccessibleDescription(agentCopy.readsChannels(2));
    expect(screen.getByText(agentCopy.scopeWide('#general', 2))).toBeInTheDocument();

    fireEvent.change(task, { target: { value: 'summarize the week' } });
    await screen.findByText('gpt-5.5 · Versa');
    await user.click(startButton());
    await waitFor(() => expect(runPosts()).toHaveLength(1));
    expect(runPosts()[0][2]).toMatchObject({ context_channels: [general.id, methods.id] });
  });

  it('starts on only what is left ticked', async () => {
    installWide(['wide_runs_v1']);
    const user = userEvent.setup();
    renderCrew(Layout);
    await connected();
    await user.click(screen.getByRole('button', { name: 'Ask my agent' }));
    const task = await screen.findByLabelText(agentCopy.task);
    await user.click(screen.getByRole('button', { name: 'Advanced' }));
    const also = screen.getByRole('checkbox', { name: '#methods' });
    expect(also).toBeChecked();
    // Archived channels are not read.
    expect(screen.queryByRole('checkbox', { name: '#old-notes' })).toBeNull();
    await user.click(also);
    expect(also).not.toBeChecked();
    await user.click(screen.getByRole('button', { name: 'Advanced' }));
    expect(screen.getByText('Reads only #general')).toBeInTheDocument();

    fireEvent.change(task, { target: { value: 'just here' } });
    await screen.findByText('gpt-5.5 · Versa');
    await user.click(startButton());
    await waitFor(() => expect(runPosts()).toHaveLength(1));
    expect(runPosts()[0][2]).toMatchObject({ context_channels: [general.id] });
  });

  it('reads only what is ticked when the broker does not read wide', async () => {
    installWide(['presence_v1']);
    const user = userEvent.setup();
    renderCrew(Layout);
    await connected();
    await user.click(screen.getByRole('button', { name: 'Ask my agent' }));
    await screen.findByLabelText(agentCopy.task);
    expect(screen.getByRole('button', { name: 'Advanced' })).toHaveAccessibleDescription(
      'Reads only #general'
    );
    expect(screen.getByText(agentCopy.scope('#general'))).toBeInTheDocument();
  });
});

describe('what the agent may do on the server (OW-6)', () => {
  it('says full access as the person, starting in the work folder', async () => {
    installDaemon([
      {
        ...connection,
        remote_root: '~/crew-work/lab',
        remote_execution: true,
        remote_access: 'full',
        server_label: 'leo',
      },
    ]);
    const user = userEvent.setup();
    renderCrew(Layout);
    await connected();
    await user.click(screen.getByRole('button', { name: 'Ask my agent' }));
    await screen.findByLabelText(agentCopy.task);
    const line = agentCopy.fullAccess('leo', '~/crew-work/lab');
    const advanced = screen.getByRole('button', { name: 'Advanced' });
    // Closed, the summary says it; open, its own line does.
    expect(advanced).toHaveAccessibleDescription(line);
    await user.click(advanced);
    expect(screen.getByText(line, { selector: 'p' })).toBeInTheDocument();
  });
});
