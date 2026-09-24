import { fireEvent, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  connection,
  currentCrew,
  general,
  installDaemon,
  installObserver,
  renderCrew,
} from '../channel/crewTestHarness';
import { CrewHttpError } from '../crewApi';
import { useCrew } from '../state/CrewControllerContext';
import { agentCopy, unknownOutcomeCopy } from './copy';
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

/**
 * The unknown-outcome gate has its own file because its lock is module-scoped by design (C10):
 * it survives a remount, so it would also survive into the next test of a shared file. Each
 * test here that leaves it set runs last.
 */
describe('AgentTaskPane: the unknown-outcome gate', () => {
  it('holds one checkbox, keeps Start disabled, and restarts only after inspection with a new request id', async () => {
    const user = userEvent.setup();
    let starts = 0;
    const requestIds: string[] = [];
    mocks.crewHttp.mockImplementation(
      async (path: string, method = 'GET', body?: { request_id?: string }) => {
        if (path === '/connections') return { connections: [connection] };
        if (path === `/connections/${connection.id}/runs` && method === 'POST') {
          starts += 1;
          requestIds.push(body?.request_id ?? '');
          if (starts === 1)
            throw new CrewHttpError('outcome unknown', 502, 'crew_start_outcome_unknown');
          return {};
        }
        return {};
      }
    );
    const view = renderCrew(Layout);
    const task = await openAgent(user);
    fireEvent.change(task, { target: { value: 'uncertain' } });
    await chooseModel(user, 'fixture-model');
    await user.click(startButton());

    expect(await screen.findByText(unknownOutcomeCopy.title)).toBeInTheDocument();
    expect(
      screen.getByText(
        unknownOutcomeCopy.body(unknownOutcomeCopy.destination('#general', 'Analysis Lab'))
      )
    ).toBeInTheDocument();
    expect(startButton()).toBeDisabled();
    fireEvent.click(startButton());
    expect(starts).toBe(1);
    expect(screen.getAllByRole('checkbox')).toHaveLength(1);

    // The lock is module-scoped: it survives Crew remounting.
    view.unmount();
    renderCrew(Layout);
    const again = await openAgent(user);
    expect(await screen.findByText(unknownOutcomeCopy.title)).toBeInTheDocument();
    expect(startButton()).toBeDisabled();
    const restart = screen.getByRole('button', { name: unknownOutcomeCopy.restart });
    expect(restart).toBeDisabled();

    // Start a new task runs the form's own validation first.
    fireEvent.change(again, { target: { value: '' } });
    await chooseModel(user, 'fixture-model');
    await user.click(screen.getByRole('checkbox', { name: unknownOutcomeCopy.checked }));
    expect(restart).toBeEnabled();
    await user.click(restart);
    expect(again).toBeInvalid();
    expect(starts).toBe(1);

    fireEvent.change(again, { target: { value: 'edited after remount' } });
    await user.click(restart);
    await waitFor(() => expect(starts).toBe(2));
    expect(requestIds[1]).not.toBe(requestIds[0]);
    await waitFor(() => expect(screen.queryByText(unknownOutcomeCopy.title)).toBeNull());
  });

  it('sends the person to the task and to chat history, unchecking the confirmation', async () => {
    const user = userEvent.setup();
    mocks.crewHttp.mockImplementation(async (path: string, method = 'GET') => {
      if (path === '/connections') return { connections: [connection] };
      if (path === `/connections/${connection.id}/runs` && method === 'POST')
        throw new CrewHttpError('outcome unknown', 502, 'crew_start_outcome_unknown');
      return {};
    });
    installObserver({
      runs: [{ run_id: 'run-9', channel_id: general.id, session_id: 's-9', status: 'running' }],
    });
    renderCrew(Layout);
    const row = document.createElement('div');
    row.setAttribute('data-crew-run-id', 'run-9');
    document.body.appendChild(row);
    try {
      const task = await openAgent(user);
      fireEvent.change(task, { target: { value: 'uncertain' } });
      await chooseModel(user, 'fixture-model');
      await user.click(startButton());
      await screen.findByText(unknownOutcomeCopy.title);

      await user.click(screen.getByRole('checkbox'));
      await user.click(screen.getByRole('button', { name: unknownOutcomeCopy.openHistory }));
      expect(mocks.navigate).toHaveBeenCalledWith('/sessions');
      expect(screen.getByRole('checkbox')).not.toBeChecked();

      await user.click(screen.getByRole('button', { name: unknownOutcomeCopy.showTask }));
      expect(currentCrew().ui.pane).toBeNull();
      await waitFor(() => expect(row).toHaveClass('crew-highlight'));
    } finally {
      row.remove();
    }
  });
});
