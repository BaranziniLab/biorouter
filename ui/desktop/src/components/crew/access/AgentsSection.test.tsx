import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ObservedRun } from '../crewApi';
import { useCrew } from '../state/CrewControllerContext';
import { AgentsSection } from './AgentsSection';
import { accessCopy } from './copy';
import { grantRow, installDaemon, renderWithController, type RunFixture } from './testing';
import { useAgentAccessCount } from './useAgentAccessCount';
import { forgetUnconfirmedRevocations } from './useCrewGrants';

const mocks = vi.hoisted(() => ({
  crewHttp: vi.fn(),
  crewRequest: vi.fn(),
  observeCrew: vi.fn(),
  onShowTask: vi.fn(),
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

const RUNS: RunFixture[] = [
  { run_id: 'run-1', channel_id: 'channel-1', session_id: 'task-1', status: 'running' },
  {
    run_id: 'run-2',
    channel_id: 'channel-2',
    session_id: 'task-2',
    status: 'waiting_for_approval',
  },
  { run_id: 'run-3', channel_id: 'channel-1', session_id: 'task-3', status: 'completed' },
];

const GRANTS = () => [
  grantRow({ session_id: 'chat-1', session_name: 'Plot review' }),
  grantRow({ session_id: 'chat-2', session_name: 'Old idea', expired: true }),
  grantRow({ session_id: 'task-1', run_id: 'run-1', kind: 'task' }),
];

function Probe() {
  const { ui, channel } = useCrew();
  const count = useAgentAccessCount();
  return (
    <div>
      <p data-testid="pane-intent">{JSON.stringify(ui.pane)}</p>
      <p data-testid="channel">{channel?.name ?? ''}</p>
      <p data-testid="chip" aria-label={count.accessibleName}>
        {count.label}
      </p>
    </div>
  );
}

function Layout() {
  const { snapshot } = useCrew();
  return (
    <>
      {snapshot ? (
        <AgentsSection onShowTask={(run: ObservedRun) => mocks.onShowTask(run.run_id)} />
      ) : null}
      <Probe />
    </>
  );
}

const section = () => screen.getByTestId('crew-agents-section');

describe('the Agents section', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    forgetUnconfirmedRevocations();
  });

  it('shows running, waiting and connected chats at rest, with Needs you on approval', async () => {
    installDaemon(mocks, { grants: GRANTS, runs: RUNS });
    renderWithController(Layout, '/crew');
    await waitFor(() => expect(section()).toHaveTextContent('Plot review'));

    const tasks = within(section()).getAllByTestId('crew-agents-task');
    expect(tasks.map((task) => task.textContent)).toEqual([
      '#general · Working…',
      '#methods · Waiting for your approval' + accessCopy.needsYou,
    ]);
    const chats = within(section()).getAllByTestId('crew-agents-chat');
    expect(chats.map((chat) => chat.textContent)).toEqual(['Plot review · #general']);
    expect(section()).not.toHaveTextContent('Old idea');
    expect(section()).not.toHaveTextContent('Done');
    expect(within(section()).getByRole('heading', { name: accessCopy.agents })).toBeInTheDocument();
  });

  it('shows revoked chats and finished tasks when asked', async () => {
    installDaemon(mocks, { grants: GRANTS, runs: RUNS });
    renderWithController(Layout, '/crew');
    await waitFor(() => expect(section()).toHaveTextContent('Plot review'));
    fireEvent.pointerDown(
      within(section()).getByRole('button', { name: accessCopy.agentsOptions }),
      {
        button: 0,
        ctrlKey: false,
      }
    );
    fireEvent.click(
      await screen.findByRole('menuitemcheckbox', { name: accessCopy.agentsShowAll })
    );
    await waitFor(() => expect(section()).toHaveTextContent('Old idea'));
    expect(section()).toHaveTextContent('#general · Done');
    const old = within(section())
      .getAllByTestId('crew-agents-chat')
      .find((chat) => chat.textContent?.includes('Old idea'));
    expect(old).toHaveTextContent(accessCopy.status.revoked);
  });

  it('opens a chat row’s Chat access pane in place', async () => {
    installDaemon(mocks, { grants: GRANTS, runs: RUNS });
    renderWithController(Layout, '/crew');
    await waitFor(() => expect(section()).toHaveTextContent('Plot review'));
    fireEvent.click(within(section()).getByText('Plot review'));
    expect(JSON.parse(screen.getByTestId('pane-intent').textContent ?? 'null')).toEqual({
      mode: 'chat-access',
      sessionId: 'chat-1',
    });
  });

  it('takes a task row to its channel and asks the timeline to show it', async () => {
    installDaemon(mocks, { grants: GRANTS, runs: RUNS });
    renderWithController(Layout, '/crew');
    await waitFor(() => expect(section()).toHaveTextContent('Plot review'));
    await waitFor(() => expect(screen.getByTestId('channel')).toHaveTextContent('general'));

    fireEvent.click(within(section()).getByText(/Waiting for your approval/));
    await waitFor(() => expect(screen.getByTestId('channel')).toHaveTextContent('methods'));
    expect(mocks.onShowTask).toHaveBeenCalledWith('run-2');
  });

  it('renders nothing with no running task and no connected chat', async () => {
    installDaemon(mocks, {
      grants: () => [grantRow({ expired: true })],
      runs: [{ run_id: 'r', channel_id: 'channel-1', session_id: 's', status: 'completed' }],
    });
    renderWithController(Layout, '/crew');
    await waitFor(() => expect(screen.getByTestId('channel')).toHaveTextContent('general'));
    await waitFor(() =>
      expect(mocks.crewHttp.mock.calls.some(([path]) => String(path).endsWith('/grants'))).toBe(
        true
      )
    );
    expect(screen.queryByTestId('crew-agents-section')).toBeNull();
  });
});

describe('the header chip count', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('counts the chats and running tasks posting in the selected channel', async () => {
    installDaemon(mocks, { grants: GRANTS, runs: RUNS });
    renderWithController(Layout, '/crew');
    // #general: Plot review (chat) and the running task-1 (task grant and run, once).
    await waitFor(() => expect(screen.getByTestId('chip')).toHaveTextContent('2 agents'));
    expect(screen.getByTestId('chip')).toHaveAttribute(
      'aria-label',
      '2 chats or agents can post here'
    );
  });
});
