import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError } from '../crewApi';
import { joinStateCopy, legacyJoinCopy } from './copy';
import { resetJoinContextForTests, updateJoinContext } from './joinContext';
import { JOIN_POLL_INTERVAL_MS, JoinStatusCard, LEGACY_JOIN_STATUS } from './JoinStatusCard';
import { DEVICE_KEY, fakeConnection, makeCrew, renderWithCrew } from './testCrew';

const mocks = vi.hoisted(() => ({ crewHttp: vi.fn() }));

// The real `api/join` helpers run against a mocked transport, so what the card shows is what the
// helper let through from the daemon's answer.
vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: mocks.crewHttp };
});
vi.mock('../../InAppTerminalDock', () => ({ default: () => <div data-testid="dock" /> }));

/** This computer's code, as ITS daemon computed it. */
const LOCAL_CODE = '7QK2M9XA3JTPWZ4D';
const ALICE = { username: 'alice', display_name: 'Alice Chen' };

function answerJoin(body: Record<string, unknown>) {
  mocks.crewHttp.mockImplementation(async (path: string, method: string) => {
    if (path === '/connections/conn-1/join' && method === 'GET') return body;
    if (path === '/connections/conn-1/join' && method === 'POST') return { joined: true };
    throw new Error(`unexpected ${method} ${path}`);
  });
}

function renderCard(overrides = {}) {
  const crew = makeCrew({
    connectionId: 'conn-1',
    connection: fakeConnection(),
    connections: [fakeConnection()],
    screen: 'join',
    ...overrides,
  });
  renderWithCrew(<JoinStatusCard />, crew);
  return { crew };
}

beforeEach(() => {
  mocks.crewHttp.mockReset();
  resetJoinContextForTests();
});
afterEach(() => {
  vi.useRealTimers();
});

describe('JoinStatusCard', () => {
  it('shows the code this computer computed, grouped, and never one the server supplied', async () => {
    answerJoin({
      status: 'invited',
      code: LOCAL_CODE,
      inviter: ALICE,
      workspace_name: 'lab',
      // Fields a tampered or future answer might carry. None of them is ever a code to show.
      approved_code: 'ZZZZZZZZZZZZZZZZ',
      broker_code: 'YYYY-YYYY-YYYY-YYYY',
    });
    const { crew } = renderCard();

    expect(await screen.findByText('Alice Chen (@alice) invited you to lab.')).toBeInTheDocument();
    expect(screen.getByText(joinStateCopy.sendCode('Alice'))).toBeInTheDocument();
    expect(screen.getByText('7QK2-M9XA-3JTP-WZ4D')).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/ZZZZ|YYYY/);
    expect(
      screen.getByRole('progressbar', { name: joinStateCopy.waiting('Alice') })
    ).toBeInTheDocument();
    expect(crew.setJoinStatus).toHaveBeenCalledWith('invited');

    // Copy takes the ungrouped value the daemon computed.
    fireEvent.click(screen.getByRole('button', { name: 'Copy device code' }));
    await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenCalledWith(LOCAL_CODE));
  });

  it('refuses a code the daemon did not compute in its own canonical form', async () => {
    answerJoin({ status: 'invited', code: 'IIII-LLLL-OOOO-UUUU', inviter: ALICE });
    renderCard();
    // The answer is rejected as unreadable; nothing that looks like a code is shown.
    expect(await screen.findByText(new RegExp(joinStateCopy.pollFailed))).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/IIII|LLLL/);
  });

  it('asks for the code again after a mismatch, with the same local code', async () => {
    answerJoin({ status: 'code_mismatch', code: LOCAL_CODE, inviter: ALICE });
    renderCard();
    expect(await screen.findByText(joinStateCopy.mismatchCode('Alice'))).toBeInTheDocument();
    expect(screen.getByText('7QK2-M9XA-3JTP-WZ4D')).toBeInTheDocument();
  });

  it('tells a person who is not invited whom to ask, with a message to send', async () => {
    updateJoinContext('conn-1', {
      hostUsername: 'alice',
      hostDisplayName: 'Alice Chen',
      workspaceName: 'lab',
    });
    answerJoin({ status: 'not_invited' });
    renderCard();

    expect(await screen.findByText(joinStateCopy.notInvitedTitle('lab'))).toBeInTheDocument();
    expect(
      screen.getByText('Ask Alice Chen (@alice) to invite @bob. This page updates by itself.')
    ).toBeInTheDocument();
    expect(screen.getByText('Hi Alice, please invite @bob to lab in Crew.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: joinStateCopy.other })).toBeInTheDocument();
  });

  it('says an invitation expired', async () => {
    answerJoin({ status: 'expired', inviter: ALICE, workspace_name: 'lab' });
    renderCard();
    expect(
      await screen.findByText(
        'This invitation expired. Ask Alice Chen (@alice) to invite you again.'
      )
    ).toBeInTheDocument();
  });

  it('finishes the join once the host approved, and refreshes', async () => {
    answerJoin({ status: 'approved', inviter: ALICE, workspace_name: 'lab' });
    const { crew } = renderCard();

    expect(await screen.findByText(joinStateCopy.approved('lab'))).toBeInTheDocument();
    await waitFor(() =>
      expect(mocks.crewHttp).toHaveBeenCalledWith('/connections/conn-1/join', 'POST')
    );
    await waitFor(() => expect(crew.setJoinStatus).toHaveBeenCalledWith('joined'));
    expect(crew.refresh).toHaveBeenCalled();
    expect(mocks.crewHttp.mock.calls.filter(([, method]) => method === 'POST')).toHaveLength(1);
  });

  it('treats a join the daemon reports as done as joined', async () => {
    answerJoin({ status: 'joined' });
    const { crew } = renderCard();
    await waitFor(() => expect(crew.setJoinStatus).toHaveBeenCalledWith('joined'));
    expect(crew.refresh).toHaveBeenCalled();
  });

  it('offers the invitation-token path when the server cannot join by code', async () => {
    answerJoin({ status: 'unsupported' });
    const { crew } = renderCard();

    expect(await screen.findByText(legacyJoinCopy.title)).toBeInTheDocument();
    expect(crew.setJoinStatus).toHaveBeenCalledWith(LEGACY_JOIN_STATUS);
    expect(screen.getByText(new RegExp(`Device key: ${DEVICE_KEY}`))).toBeInTheDocument();

    const token = screen.getByLabelText('Enrollment invitation');
    expect(token).toHaveAttribute('placeholder', 'Invitation token');
    fireEvent.change(token, { target: { value: 'token-123' } });
    fireEvent.click(screen.getByRole('button', { name: 'Join workspace' }));

    await waitFor(() =>
      expect(crew.request).toHaveBeenCalledWith(
        'auth.enroll',
        { invitation: 'token-123', public_key: DEVICE_KEY },
        { mutation: true }
      )
    );
    await waitFor(() => expect(crew.refresh).toHaveBeenCalled());
  });

  it('offers the token path behind "Other ways to join" too', async () => {
    answerJoin({ status: 'invited', code: LOCAL_CODE, inviter: ALICE });
    renderCard();
    fireEvent.click(await screen.findByRole('button', { name: joinStateCopy.other }));
    expect(screen.getByLabelText('Enrollment invitation')).toBeInTheDocument();
  });

  it('opens straight on the token path when the probe already found it', () => {
    mocks.crewHttp.mockReturnValue(new Promise(() => {}));
    renderCard({ joinStatus: LEGACY_JOIN_STATUS });
    expect(screen.getByText(legacyJoinCopy.title)).toBeInTheDocument();
    expect(screen.queryByText(joinStateCopy.checking)).toBeNull();
  });

  it('treats a daemon without the join route as the token path', async () => {
    mocks.crewHttp.mockRejectedValue(new CrewHttpError('Crew request failed (404)', 404));
    const { crew } = renderCard();
    expect(await screen.findByText(legacyJoinCopy.title)).toBeInTheDocument();
    expect(crew.setJoinStatus).toHaveBeenCalledWith(LEGACY_JOIN_STATUS);
  });

  it('sends a host back to finishing the workspace it started', async () => {
    updateJoinContext('conn-1', { hostSetup: true, workspaceName: 'lab' });
    answerJoin({ status: 'not_invited' });
    const { crew } = renderCard();
    fireEvent.click(await screen.findByRole('button', { name: joinStateCopy.hostPendingAction }));
    expect(crew.openDialog).toHaveBeenCalledWith({ kind: 'host' });
    expect(screen.queryByText(joinStateCopy.notInvitedTitle('lab'))).toBeNull();
  });

  it('asks again every five seconds while visible', async () => {
    vi.useFakeTimers();
    answerJoin({ status: 'not_invited' });
    renderCard();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    const gets = () => mocks.crewHttp.mock.calls.filter(([, method]) => method === 'GET').length;
    expect(gets()).toBe(1);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(JOIN_POLL_INTERVAL_MS - 10);
    });
    expect(gets()).toBe(1);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(20);
    });
    expect(gets()).toBe(2);
  });

  it('waits while the page is hidden', async () => {
    vi.useFakeTimers();
    const visibility = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden');
    answerJoin({ status: 'not_invited' });
    renderCard();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(JOIN_POLL_INTERVAL_MS * 3);
    });
    expect(mocks.crewHttp).not.toHaveBeenCalled();

    visibility.mockReturnValue('visible');
    await act(async () => {
      document.dispatchEvent(new Event('visibilitychange'));
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(mocks.crewHttp).toHaveBeenCalledTimes(1);
    visibility.mockRestore();
  });
});
