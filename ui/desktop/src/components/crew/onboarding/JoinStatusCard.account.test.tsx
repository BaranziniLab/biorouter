import { fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { resetConnectionMemoryForTests } from '../state/useCrewConnections';
import { joinStateCopy } from './copy';
import { resetJoinClaimForTests } from './joinClaimState';
import { resetJoinContextForTests, updateJoinContext } from './joinContext';
import { JoinStatusCard } from './JoinStatusCard';
import { OnboardingScreen } from './OnboardingScreen';
import { fakeConnection, makeCrew, renderWithCrew } from './testCrew';

/**
 * The join card against a workspace that belongs to accounts (account ownership AO-9): `ready`
 * joins at once only when the person asked for the workspace here, and otherwise waits for Join;
 * `removed` says AO-2's sentence; and no code is ever shown.
 */

const mocks = vi.hoisted(() => ({ crewHttp: vi.fn() }));

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: mocks.crewHttp };
});
vi.mock('../../InAppTerminalDock', () => ({ default: () => <div data-testid="dock" /> }));

function answer(status: Record<string, unknown>) {
  mocks.crewHttp.mockImplementation(async (path: string, method: string) => {
    if (path === '/connections/conn-1/join' && method === 'GET') return status;
    if (path === '/connections/conn-1/join' && method === 'POST')
      return { joined: true, status: 'joined', add_device: false };
    return {};
  });
}

const claims = () =>
  mocks.crewHttp.mock.calls.filter(
    ([path, method]) => path === '/connections/conn-1/join' && method === 'POST'
  );

function renderCard(connection = fakeConnection({ account_identity: true })) {
  const crew = makeCrew({
    connectionId: 'conn-1',
    connection,
    connections: [connection],
    screen: 'join',
  });
  renderWithCrew(<JoinStatusCard />, crew);
  return crew;
}

beforeEach(() => {
  mocks.crewHttp.mockReset();
  resetJoinClaimForTests();
  resetJoinContextForTests();
  resetConnectionMemoryForTests();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('JoinStatusCard on a workspace that belongs to accounts', () => {
  it('waits for Join when the person did not start the flow, then joins by account', async () => {
    answer({ status: 'ready', add_device: false, workspace_name: 'lab' });
    const crew = renderCard();
    const join = await screen.findByRole('button', { name: joinStateCopy.readyAction('lab') });
    expect(screen.getByText(joinStateCopy.readyTitle('lab'))).toBeInTheDocument();
    // Nothing is claimed by itself, and no code is shown.
    expect(claims()).toHaveLength(0);
    expect(screen.queryByRole('button', { name: /Copy your code/ })).toBeNull();
    fireEvent.click(join);
    await waitFor(() => expect(claims()).toHaveLength(1));
    await waitFor(() => expect(crew.setJoinStatus).toHaveBeenCalledWith('joined'));
  });

  it('joins at once when the person opened or joined the workspace here', async () => {
    updateJoinContext('conn-1', { joining: true, workspaceName: 'lab' });
    answer({ status: 'ready', add_device: false, workspace_name: 'lab' });
    const crew = renderCard();
    await waitFor(() => expect(claims()).toHaveLength(1));
    await waitFor(() => expect(crew.setJoinStatus).toHaveBeenCalledWith('joined'));
    expect(screen.queryByRole('button', { name: joinStateCopy.readyAction('lab') })).toBeNull();
  });

  it('says AO-2’s sentence when the workspace removed the account, and claims nothing', async () => {
    answer({ status: 'removed', add_device: false, workspace_name: 'lab' });
    renderCard();
    expect(await screen.findByTestId('crew-join-removed')).toHaveTextContent(
      'You were removed from lab. Ask a member to invite you again.'
    );
    expect(claims()).toHaveLength(0);
  });

  it('never asks for Add another computer when the account is not a member at all', async () => {
    answer({ status: 'not_invited', add_device: false, workspace_name: 'lab' });
    renderCard();
    expect(await screen.findByText(joinStateCopy.notInvitedTitle('lab'))).toBeInTheDocument();
    expect(screen.queryByTestId('crew-join-not-invited-before')).toBeNull();
    expect(document.body.textContent).not.toMatch(/Add another (computer|device)/);
  });

  it('never joins another connection on a Join pressed for the first (finding 40)', async () => {
    // A's claim never settles; B is ready, but the person never started its flow.
    mocks.crewHttp.mockImplementation(async (path: string, method: string) => {
      if (method === 'GET' && path === '/connections/conn-1/join')
        return { status: 'ready', add_device: false, workspace_name: 'lab' };
      if (method === 'GET' && path === '/connections/conn-2/join')
        return { status: 'ready', add_device: false, workspace_name: 'other-lab' };
      if (method === 'POST' && path === '/connections/conn-1/join') return new Promise(() => {});
      if (method === 'POST' && path === '/connections/conn-2/join')
        return { joined: true, status: 'joined', add_device: false };
      return {};
    });
    const posts = (id: string) =>
      mocks.crewHttp.mock.calls.filter(
        ([path, method]) => path === `/connections/${id}/join` && method === 'POST'
      );
    const first = fakeConnection({ account_identity: true });
    const second = fakeConnection({ id: 'conn-2', name: 'other-lab', account_identity: true });
    const rendered = renderWithCrew(
      <OnboardingScreen />,
      makeCrew({
        connectionId: 'conn-1',
        connection: first,
        connections: [first, second],
        screen: 'join',
      })
    );
    fireEvent.click(await screen.findByRole('button', { name: joinStateCopy.readyAction('lab') }));
    await waitFor(() => expect(posts('conn-1')).toHaveLength(1));
    rendered.update({ connectionId: 'conn-2', connection: second });
    expect(
      await screen.findByRole('button', { name: joinStateCopy.readyAction('other-lab') })
    ).toBeInTheDocument();
    expect(posts('conn-2')).toHaveLength(0);
  });

  it('keeps Add another computer on an older server', async () => {
    answer({ status: 'not_invited', add_device: false, workspace_name: 'lab' });
    renderCard(fakeConnection({ account_identity: false }));
    expect(await screen.findByTestId('crew-join-not-invited-before')).toHaveTextContent(
      /Add another computer for/
    );
  });
});
