import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
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
      expect(screen.queryByRole('button', { name: /^Model/ })).toBeNull();

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

  describe('Advanced', () => {
    it('keeps Also read unmounted while closed and summarizes what it adds', async () => {
      const user = userEvent.setup();
      renderCrew(Layout);
      const task = await openAgent(user);
      expect(screen.queryByRole('checkbox')).toBeNull();
      expect(screen.getByText(agentCopy.advancedSummary(0))).toBeInTheDocument();

      await user.click(screen.getByRole('button', { name: 'Advanced' }));
      const also = screen.getByRole('checkbox', { name: '#methods' });
      // Archived channels and the current channel are not offered.
      expect(screen.queryByRole('checkbox', { name: '#old-notes' })).toBeNull();
      expect(screen.queryByRole('checkbox', { name: '#general' })).toBeNull();
      await user.click(also);
      await user.click(screen.getByRole('button', { name: 'Advanced' }));
      expect(screen.getByText(agentCopy.advancedSummary(1))).toBeInTheDocument();

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
      await user.click(screen.getByRole('button', { name: 'Advanced' }));
      expect(screen.getByText(agentCopy.folderExec('/home/alice/crew-work'))).toBeInTheDocument();
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
