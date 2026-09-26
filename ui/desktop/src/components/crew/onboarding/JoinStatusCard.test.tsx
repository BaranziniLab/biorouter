import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CREW_NOT_CONNECTED } from '../api/join';
import { CrewHttpError, type CrewConnection } from '../crewApi';
import { usePendingHost } from '../sidebar/sidebarView';
import { deriveCrewScreen } from '../state/crewStatus';
import { noteConnectionVerified, resetConnectionMemoryForTests } from '../state/useCrewConnections';
import { joinStateCopy, legacyJoinCopy } from './copy';
import { resetJoinClaimForTests } from './joinClaimState';
import { readJoinContext, resetJoinContextForTests, updateJoinContext } from './joinContext';
import { JOIN_POLL_INTERVAL_MS, JoinStatusCard, LEGACY_JOIN_STATUS } from './JoinStatusCard';
import { OnboardingScreen } from './OnboardingScreen';
import { DEVICE_KEY, fakeConnection, makeCrew, renderWithCrew, type CrewRender } from './testCrew';

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

/** The screen the real derivation picks for a saved, not-yet-joined connection. */
function joinScreen(inFlight: boolean) {
  return deriveCrewScreen({
    connectionsState: 'loaded',
    connectionCount: 1,
    connection: fakeConnection(),
    lastConnectFailure: null,
    inFlight,
    signInOpen: false,
    view: null,
    channelId: '',
    observationError: false,
    notJoined: true,
  });
}

/**
 * The card inside the real screen switch, with a `connect` that marks itself pending for 100 ms
 * the way the controller's `act('connect')` does. While it runs, the real derivation says
 * `connecting`, so `ConnectingCard` replaces the card; when it settles the card comes back as a new
 * mount. The card's own reconnect therefore remounts it, which is what the real app does.
 *
 * Each `update` renders inside React's `act` (the render helper wraps it). The 100 ms wait is a
 * fake timer that `advance` fires, so no `act` scope is left open across the steps.
 * `settleAfterRemountMs` makes `connect`'s promise settle that long after the card came back, so
 * the new mount meets a connect that an earlier mount started and that is still running.
 */
function renderRemountingScreen({ settleAfterRemountMs = 0 } = {}) {
  let rendered: CrewRender | null = null;
  let inFlight = false;
  const connect = vi.fn(async (_opts?: { userInitiated?: boolean }) => {
    inFlight = true;
    rendered?.update({ screen: joinScreen(inFlight) });
    await new Promise((resolve) => setTimeout(resolve, 100));
    inFlight = false;
    rendered?.update({ screen: joinScreen(inFlight) });
    if (settleAfterRemountMs > 0) {
      await new Promise((resolve) => setTimeout(resolve, settleAfterRemountMs));
    }
  });
  const crew = makeCrew({
    connectionId: 'conn-1',
    connection: fakeConnection(),
    connections: [fakeConnection()],
    screen: joinScreen(false),
    connect,
  });
  rendered = renderWithCrew(<OnboardingScreen />, crew);
  return { connect, rendered };
}

const countCalls = (method: string) =>
  mocks.crewHttp.mock.calls.filter(([, called]) => called === method).length;

/**
 * Fake time in steps shorter than a connect. React renders an `act` scope's updates when the scope
 * ends, so one long step would batch a connect's `connecting` and `join` screens into one render
 * and the card would never unmount.
 */
async function advance(ms: number) {
  const step = 50;
  for (let passed = 0; passed < ms; passed += step) {
    await act(async () => {
      await vi.advanceTimersByTimeAsync(Math.min(step, ms - passed));
    });
  }
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
}

beforeEach(() => {
  mocks.crewHttp.mockReset();
  resetJoinContextForTests();
  resetJoinClaimForTests();
  resetConnectionMemoryForTests();
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
    expect(screen.getByTestId('crew-join-waiting')).toHaveTextContent(
      joinStateCopy.waiting('Alice')
    );
    expect(crew.setJoinStatus).toHaveBeenCalledWith('invited');

    // Copy takes what is shown: the daemon's code, grouped with its dashes (T-36). One noun for one
    // code, the one a sighted person reads: "your code", never "device code" (Q3-47).
    expect(screen.queryByRole('button', { name: /device code/i })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Copy your code' }));
    await waitFor(() =>
      expect(navigator.clipboard.writeText).toHaveBeenCalledWith('7QK2-M9XA-3JTP-WZ4D')
    );
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
    // Not "Invitation token": that read as the invitation the person already pasted (T-35).
    expect(token).toHaveAttribute('placeholder', 'Token from an older invitation');
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

  it('folds the token path behind "Having trouble joining?", and says when it is needed', async () => {
    answerJoin({ status: 'invited', code: LOCAL_CODE, inviter: ALICE });
    const { crew } = renderCard();
    const trouble = await screen.findByRole('button', { name: 'Having trouble joining?' });
    expect(trouble).toHaveAttribute('aria-expanded', 'false');
    // Folded: no second join form, token field or device key competes with the code.
    expect(screen.queryByLabelText('Enrollment invitation')).toBeNull();
    expect(screen.queryByText(joinStateCopy.otherBody('Alice'))).toBeNull();
    expect(document.body.textContent).not.toMatch(/Device key/);

    fireEvent.click(trouble);
    const section = screen.getByTestId('crew-join-trouble');
    const lines = within(section).getAllByText(/./, { selector: 'p' });
    // Waiting is the normal state, said first (Q4-43), true to how the host lets them in.
    expect(lines[0]).toHaveTextContent(
      'Alice hasn’t let you in yet. That’s normal: Alice lets you in by entering your code in Crew.'
    );
    // Then the join request, introduced as what it is, never "send this instead:" before a link
    // (Q4-43), never "Send this join request to your host" (Q2-35), and naming the host as the
    // card's own sentences do (Q3-46).
    expect(lines[1]).toHaveTextContent('If Alice asks for a join request:');
    expect(screen.queryByText(legacyJoinCopy.sendRequest)).toBeNull();
    // The 64-character device key stays folded behind Show (Q3-48).
    expect(document.body.textContent).not.toMatch(/Device key/);
    const show = within(section).getByRole('button', { name: legacyJoinCopy.showJoinRequest });
    expect(show).toHaveAttribute('aria-expanded', 'false');
    fireEvent.click(show);
    expect(
      within(section).getByRole('button', { name: legacyJoinCopy.hideRequest })
    ).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByText(new RegExp(`Device key: ${DEVICE_KEY}`))).toBeInTheDocument();

    // The token path is a second mechanism: behind its own quiet link, with no masked field or
    // "Join with a token" until the person says the host sent one (Q4-43).
    expect(screen.queryByLabelText('Enrollment invitation')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Join with a token' })).toBeNull();
    fireEvent.click(
      within(section).getByRole('button', { name: joinStateCopy.tokenInstead('Alice') })
    );
    const token = screen.getByLabelText('Enrollment invitation');
    // The link left as the field arrived: focus is in the field, not on the page.
    expect(token).toHaveFocus();
    // Still a credential: masked.
    expect(token).toHaveAttribute('type', 'password');
    // Someone who already pressed Join is not offered "Join workspace" again: the button names
    // the other way in (Q3-48).
    expect(screen.queryByRole('button', { name: 'Join workspace' })).toBeNull();
    const submit = screen.getByRole('button', { name: 'Join with a token' });
    expect(submit).toBeDisabled();
    fireEvent.change(token, { target: { value: 'token-123' } });
    fireEvent.click(submit);
    await waitFor(() =>
      expect(crew.request).toHaveBeenCalledWith(
        'auth.enroll',
        { invitation: 'token-123', public_key: DEVICE_KEY },
        { mutation: true }
      )
    );
    await waitFor(() => expect(crew.setJoinStatus).toHaveBeenCalledWith('joined'));
    expect(readJoinContext('conn-1').joining).toBe(false);
  });

  it('keeps the reassurance for a person with a code out, not for one who is not invited', async () => {
    answerJoin({ status: 'not_invited' });
    renderCard();
    fireEvent.click(await screen.findByRole('button', { name: joinStateCopy.other }));
    // Nobody is letting them in: only the two other ways remain.
    expect(screen.queryByText(/hasn’t let you in yet/)).toBeNull();
    expect(screen.getByText(joinStateCopy.otherBody('your host'))).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: joinStateCopy.tokenInstead('your host') })
    ).toBeInTheDocument();
  });

  it('waits on a person without animating, and says the app can close and when it expires (Q4-46)', async () => {
    // Unix seconds, as the broker writes them: a day from now.
    const expiresAt = Math.floor(Date.now() / 1000) + 86_400;
    answerJoin({
      status: 'invited',
      code: LOCAL_CODE,
      inviter: ALICE,
      workspace_name: 'lab',
      expires_at: expiresAt,
    });
    renderCard();
    const waiting = await screen.findByTestId('crew-join-waiting');
    expect(waiting).toHaveTextContent(joinStateCopy.waiting('Alice'));
    // No indeterminate progress for a wait on a person: no progress bar, no spinner, no sweep.
    expect(screen.queryByRole('progressbar')).toBeNull();
    expect(document.querySelector('.crew-onboard-spinner')).toBeNull();
    expect(document.querySelector('[class*="progress"]')).toBeNull();
    const when = new Intl.DateTimeFormat(undefined, {
      weekday: 'short',
      hour: 'numeric',
      minute: '2-digit',
    }).format(new Date(expiresAt * 1000));
    expect(screen.getByTestId('crew-join-wait-note')).toHaveTextContent(
      `${joinStateCopy.expires(when)} ${joinStateCopy.closeNote('Alice', 'lab')}`
    );
    // The still clock is authored CSS that never animates.
    const css = readFileSync(join(__dirname, 'onboarding.css'), 'utf8');
    const rules = css.match(/\.crew-onboard-wait[^{]*\{[^}]*\}/g) ?? [];
    expect(rules.length).toBeGreaterThan(0);
    for (const rule of rules) expect(rule).not.toMatch(/animation|transition/);
  });

  it('says an invitation whose time passed has expired, before the next poll says so', async () => {
    answerJoin({
      status: 'invited',
      code: LOCAL_CODE,
      inviter: ALICE,
      workspace_name: 'lab',
      expires_at: Math.floor(Date.now() / 1000) - 60,
    });
    renderCard();
    // Only that: "Alice can still let you in with the same code" would contradict it.
    const note = await screen.findByTestId('crew-join-wait-note');
    expect(note.textContent).toBe(joinStateCopy.expiredNow);
    expect(document.body.textContent).not.toContain(joinStateCopy.closeNote('Alice', 'lab'));
  });

  it('leaves out the expiry when the status names none', async () => {
    answerJoin({ status: 'invited', code: LOCAL_CODE, inviter: ALICE, workspace_name: 'lab' });
    renderCard();
    expect(await screen.findByTestId('crew-join-wait-note')).toHaveTextContent(
      joinStateCopy.closeNote('Alice', 'lab')
    );
    expect(screen.getByTestId('crew-join-wait-note')).not.toHaveTextContent(/expire/);
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

  it('reconnects once by itself when the join route says it is not connected', async () => {
    let connected = false;
    mocks.crewHttp.mockImplementation(async (path: string, method: string) => {
      if (path === '/connections/conn-1/join' && method === 'GET') {
        if (!connected) throw new CrewHttpError('Connect first.', 409, 'crew_not_connected');
        return { status: 'invited', code: LOCAL_CODE, inviter: ALICE, workspace_name: 'lab' };
      }
      throw new Error(`unexpected ${method} ${path}`);
    });
    const connect = vi.fn(async () => {
      connected = true;
    });
    renderCard({ connect });

    expect(await screen.findByText('Alice Chen (@alice) invited you to lab.')).toBeInTheDocument();
    expect(connect).toHaveBeenCalledOnce();
    expect(screen.queryByTestId('crew-join-not-connected')).toBeNull();
    expect(screen.queryByTestId('crew-join-reconnecting')).toBeNull();
    expect(document.body.textContent).not.toContain(joinStateCopy.pollFailed);
  });

  it('offers Reconnect, instead of a poll error forever, when one reconnect did not help', async () => {
    let connected = false;
    mocks.crewHttp.mockImplementation(async (path: string, method: string) => {
      if (path === '/connections/conn-1/join' && method === 'GET') {
        if (!connected) throw new CrewHttpError('Connect first.', 409, 'crew_not_connected');
        return { status: 'invited', code: LOCAL_CODE, inviter: ALICE, workspace_name: 'lab' };
      }
      throw new Error(`unexpected ${method} ${path}`);
    });
    // The automatic attempt fails (the controller records why; connect never throws).
    const connect = vi.fn(async (opts?: { userInitiated?: boolean }) => {
      if (opts?.userInitiated) connected = true;
    });
    updateJoinContext('conn-1', { workspaceName: 'lab' });
    renderCard({ connect });

    const lost = await screen.findByTestId('crew-join-not-connected');
    expect(lost).toHaveTextContent(joinStateCopy.notConnected('lab'));
    expect(connect).toHaveBeenCalledOnce();
    expect(connect).toHaveBeenCalledWith();
    const gets = () => mocks.crewHttp.mock.calls.filter(([, method]) => method === 'GET').length;
    expect(gets()).toBe(2);
    expect(document.body.textContent).not.toContain(joinStateCopy.pollFailed);

    // Polling stopped: nothing is asked again until the person reconnects.
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 50));
    });
    expect(gets()).toBe(2);
    expect(connect).toHaveBeenCalledOnce();

    fireEvent.click(screen.getByRole('button', { name: joinStateCopy.reconnect }));
    expect(connect).toHaveBeenLastCalledWith({ userInitiated: true });
    expect(await screen.findByText('Alice Chen (@alice) invited you to lab.')).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByTestId('crew-join-not-connected')).toBeNull());
  });

  it('reconnects and claims again when finishing the join finds the connection dropped', async () => {
    let claims = 0;
    mocks.crewHttp.mockImplementation(async (path: string, method: string) => {
      if (path === '/connections/conn-1/join' && method === 'GET')
        return { status: 'approved', inviter: ALICE, workspace_name: 'lab' };
      if (path === '/connections/conn-1/join' && method === 'POST') {
        claims += 1;
        if (claims === 1) throw new CrewHttpError('Connect first.', 409, 'crew_not_connected');
        return {
          joined: true,
          status: 'joined',
          inviter: { username: 'carol', display_name: 'Carol Diaz' },
          workspace_name: 'lab',
          add_device: false,
        };
      }
      throw new Error(`unexpected ${method} ${path}`);
    });
    const { crew } = renderCard();

    await waitFor(() => expect(crew.setJoinStatus).toHaveBeenCalledWith('joined'));
    expect(crew.connect).toHaveBeenCalledOnce();
    expect(claims).toBe(2);
    expect(screen.queryByText(joinStateCopy.claimFailed)).toBeNull();
    // Who the workspace says invited this computer is what the next screens name.
    expect(readJoinContext('conn-1')).toMatchObject({
      hostUsername: 'carol',
      hostDisplayName: 'Carol Diaz',
      workspaceName: 'lab',
      joining: false,
    });
  });

  it('reconnects once per approval when finishing the join keeps finding no connection, then waits for the person', async () => {
    vi.useFakeTimers();
    // A link that drops on every claim while the status keeps answering `approved`: without a
    // limit this is an unattended loop of SSH connects and claims signed with the device key.
    mocks.crewHttp.mockImplementation(async (path: string, method: string) => {
      if (path === '/connections/conn-1/join' && method === 'GET')
        return { status: 'approved', inviter: ALICE, workspace_name: 'lab' };
      if (path === '/connections/conn-1/join' && method === 'POST')
        throw new CrewHttpError('Connect first.', 409, CREW_NOT_CONNECTED);
      throw new Error(`unexpected ${method} ${path}`);
    });
    const connect = vi.fn(async (_opts?: { userInitiated?: boolean }) => {});
    updateJoinContext('conn-1', { workspaceName: 'lab' });
    renderCard({ connect });
    const count = (method: string) =>
      mocks.crewHttp.mock.calls.filter(([, called]) => called === method).length;
    const advance = async (ms: number) => {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(ms);
      });
    };

    await advance(0);
    await advance(JOIN_POLL_INTERVAL_MS * 4);
    // The first claim, and one more after the one automatic reconnect.
    expect(count('POST')).toBe(2);
    expect(connect).toHaveBeenCalledOnce();
    expect(connect).toHaveBeenCalledWith();
    expect(screen.getByTestId('crew-join-not-connected')).toHaveTextContent(
      joinStateCopy.notConnected('lab')
    );
    expect(screen.getByRole('button', { name: joinStateCopy.reconnect })).toBeInTheDocument();

    // The status keeps answering; Reconnect stays offered and nothing is claimed by itself.
    const gets = count('GET');
    await advance(JOIN_POLL_INTERVAL_MS * 3);
    expect(count('GET')).toBeGreaterThan(gets);
    expect(count('POST')).toBe(2);
    expect(connect).toHaveBeenCalledOnce();
    expect(screen.getByRole('button', { name: joinStateCopy.reconnect })).toBeInTheDocument();

    // The person's press is the reconnect: exactly one more claim, then Reconnect again.
    fireEvent.click(screen.getByRole('button', { name: joinStateCopy.reconnect }));
    await advance(0);
    await advance(JOIN_POLL_INTERVAL_MS * 3);
    expect(connect).toHaveBeenCalledTimes(2);
    expect(connect).toHaveBeenLastCalledWith({ userInitiated: true });
    expect(count('POST')).toBe(3);
    expect(screen.getByRole('button', { name: joinStateCopy.reconnect })).toBeInTheDocument();
  });

  it('keeps the one automatic reconnect per approval across the remount its own connect causes', async () => {
    vi.useFakeTimers();
    // The loop measured in the real app (40 connects and 40 claims in 20 s): the card's reconnect
    // shows `connecting`, which unmounts it, and each new mount claimed, found no connection and
    // reconnected "once" again.
    mocks.crewHttp.mockImplementation(async (path: string, method: string) => {
      if (path === '/connections/conn-1/join' && method === 'GET')
        return { status: 'approved', inviter: ALICE, workspace_name: 'lab' };
      if (path === '/connections/conn-1/join' && method === 'POST')
        throw new CrewHttpError('Connect first.', 409, CREW_NOT_CONNECTED);
      throw new Error(`unexpected ${method} ${path}`);
    });
    updateJoinContext('conn-1', { workspaceName: 'lab' });
    const { connect } = renderRemountingScreen();

    await advance(20_000);
    // The first claim, one automatic reconnect (which remounted the card), one more claim.
    expect(connect).toHaveBeenCalledOnce();
    expect(connect).toHaveBeenCalledWith();
    expect(countCalls('POST')).toBe(2);
    expect(screen.getByTestId('crew-join-not-connected')).toHaveTextContent(
      joinStateCopy.notConnected('lab')
    );
    expect(screen.getByRole('button', { name: joinStateCopy.reconnect })).toBeInTheDocument();

    // Nothing more happens by itself while the status keeps answering.
    const gets = countCalls('GET');
    await advance(15_000);
    expect(countCalls('GET')).toBeGreaterThan(gets);
    expect(connect).toHaveBeenCalledOnce();
    expect(countCalls('POST')).toBe(2);

    // The person's press works across the remount it causes: exactly one fresh claim.
    fireEvent.click(screen.getByRole('button', { name: joinStateCopy.reconnect }));
    await advance(JOIN_POLL_INTERVAL_MS * 3);
    expect(connect).toHaveBeenCalledTimes(2);
    expect(connect).toHaveBeenLastCalledWith({ userInitiated: true });
    expect(countCalls('POST')).toBe(3);
    expect(screen.getByRole('button', { name: joinStateCopy.reconnect })).toBeInTheDocument();
  });

  it.each([
    ['settles before the card comes back', 0],
    // The new mount's poll meets the old mount's connect still running: it waits for that connect
    // instead of counting a failed attempt, and asks again once it settles.
    ['settles after the card came back', 300],
  ])(
    'keeps the poll’s one automatic reconnect across the remount its own connect causes (connect %s)',
    async (_when, settleAfterRemountMs) => {
      vi.useFakeTimers();
      mocks.crewHttp.mockImplementation(async (path: string, method: string) => {
        if (path === '/connections/conn-1/join' && method === 'GET')
          throw new CrewHttpError('Connect first.', 409, CREW_NOT_CONNECTED);
        throw new Error(`unexpected ${method} ${path}`);
      });
      updateJoinContext('conn-1', { workspaceName: 'lab' });
      const { connect } = renderRemountingScreen({ settleAfterRemountMs });

      await advance(20_000);
      expect(connect).toHaveBeenCalledOnce();
      expect(connect).toHaveBeenCalledWith();
      expect(screen.getByTestId('crew-join-not-connected')).toHaveTextContent(
        joinStateCopy.notConnected('lab')
      );
      expect(screen.queryByTestId('crew-join-reconnecting')).toBeNull();
      expect(countCalls('POST')).toBe(0);
    }
  );

  it('claims again on the next mount when the claim settled after the card unmounted', async () => {
    vi.useFakeTimers();
    // The claim is still out when a connect takes the card away; its outcome lands on no mount.
    let answerClaim: ((value: unknown) => void) | null = null;
    let polls = 0;
    mocks.crewHttp.mockImplementation(async (path: string, method: string) => {
      if (path === '/connections/conn-1/join' && method === 'GET') {
        polls += 1;
        return { status: 'approved', inviter: ALICE, workspace_name: 'lab' };
      }
      if (path === '/connections/conn-1/join' && method === 'POST') {
        if (countCalls('POST') > 1)
          return { joined: true, status: 'joined', workspace_name: 'lab' };
        return new Promise((resolve) => {
          answerClaim = resolve;
        });
      }
      throw new Error(`unexpected ${method} ${path}`);
    });
    const { rendered } = renderRemountingScreen();
    await advance(0);
    expect(countCalls('POST')).toBe(1);
    expect(polls).toBeGreaterThan(0);

    // Unmount the card, let the claim fail with no mount to hear it, then bring the card back.
    rendered.update({ screen: joinScreen(true) });
    expect(screen.getByTestId('crew-connecting')).toBeInTheDocument();
    await act(async () => {
      answerClaim?.(Promise.reject(new CrewHttpError('Crew request failed (500)', 500)));
      await vi.advanceTimersByTimeAsync(0);
    });
    rendered.update({ screen: joinScreen(false) });
    await advance(JOIN_POLL_INTERVAL_MS * 2);

    // The next mount claimed again instead of spinning on a claim nobody was making.
    expect(countCalls('POST')).toBe(2);
    expect(rendered.crew().setJoinStatus).toHaveBeenCalledWith('joined');
  });

  it('claims again on the next mount when this mount showed a claim error and was then replaced', async () => {
    vi.useFakeTimers();
    // A claim error sits beside Retry; then a poll finds the link down and the automatic reconnect
    // replaces the card. The error went with that mount, so the next one must not spin forever.
    let gets = 0;
    mocks.crewHttp.mockImplementation(async (path: string, method: string) => {
      if (path === '/connections/conn-1/join' && method === 'GET') {
        gets += 1;
        if (gets === 2) throw new CrewHttpError('Connect first.', 409, CREW_NOT_CONNECTED);
        return { status: 'approved', inviter: ALICE, workspace_name: 'lab' };
      }
      if (path === '/connections/conn-1/join' && method === 'POST') {
        if (countCalls('POST') === 1) throw new CrewHttpError('Crew request failed (500)', 500);
        return { joined: true, status: 'joined', workspace_name: 'lab' };
      }
      throw new Error(`unexpected ${method} ${path}`);
    });
    const { connect, rendered } = renderRemountingScreen();
    await advance(0);
    expect(countCalls('POST')).toBe(1);
    expect(screen.getByRole('button', { name: joinStateCopy.retry })).toBeInTheDocument();

    await advance(JOIN_POLL_INTERVAL_MS * 2);
    expect(connect).toHaveBeenCalledOnce();
    expect(countCalls('POST')).toBe(2);
    expect(rendered.crew().setJoinStatus).toHaveBeenCalledWith('joined');
  });

  it('says so when the invitation adds this computer to the person’s account', async () => {
    answerJoin({
      status: 'invited',
      code: LOCAL_CODE,
      inviter: ALICE,
      workspace_name: 'lab',
      add_device: true,
    });
    renderCard();
    expect(
      await screen.findByText(joinStateCopy.invitedDevice('Alice Chen (@alice)', 'lab'))
    ).toBeInTheDocument();
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
  it('names the inviter for the rail as soon as a poll names them, not when the join ends (Q3-46)', async () => {
    vi.useFakeTimers();
    // The invitation named only the handle; the workspace knows the name.
    updateJoinContext('conn-1', {
      hostUsername: 'alice',
      hostDisplayName: null,
      workspaceName: 'lab',
    });
    answerJoin({ status: 'invited', code: LOCAL_CODE, inviter: ALICE, workspace_name: 'lab' });
    function Rail() {
      const { host } = usePendingHost({ connectionId: 'conn-1' });
      return <p data-testid="rail">{`Waiting for ${host ?? 'your host'} to let you in`}</p>;
    }
    const crew = makeCrew({
      connectionId: 'conn-1',
      connection: fakeConnection(),
      connections: [fakeConnection()],
      screen: 'join',
    });
    renderWithCrew(
      <>
        <JoinStatusCard />
        <Rail />
      </>,
      crew
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Still waiting, and the rail already says what the card says.
    expect(screen.getByText('Alice Chen (@alice) invited you to lab.')).toBeInTheDocument();
    expect(screen.getByTestId('rail')).toHaveTextContent(
      'Waiting for Alice Chen (@alice) to let you in'
    );
    expect(readJoinContext('conn-1')).toMatchObject({
      hostUsername: 'alice',
      hostDisplayName: 'Alice Chen',
    });
    expect(crew.setJoinStatus).not.toHaveBeenCalledWith('joined');

    // A poll that says the same thing writes nothing: the same record is read back.
    const recorded = readJoinContext('conn-1');
    await act(async () => {
      await vi.advanceTimersByTimeAsync(JOIN_POLL_INTERVAL_MS);
    });
    expect(countCalls('GET')).toBe(2);
    expect(readJoinContext('conn-1')).toBe(recorded);
  });

  it('tells a member the workspace removed that they are no longer in it (Q3-50)', async () => {
    updateJoinContext('conn-1', {
      hostUsername: 'alice',
      hostDisplayName: 'Alice Chen',
      workspaceName: 'lab',
    });
    // This computer was a member earlier in this session.
    noteConnectionVerified('conn-1');
    answerJoin({ status: 'not_invited' });
    renderCard();

    expect(await screen.findByText(joinStateCopy.removedTitle('lab'))).toBeInTheDocument();
    expect(
      screen.getByText(
        'This computer or your account was removed from lab. If you didn’t expect that, ask Alice Chen (@alice).'
      )
    ).toBeInTheDocument();
    // Removed, not "not yet": no invitation request to send, and no second way to join.
    expect(screen.queryByText(joinStateCopy.notInvitedTitle('lab'))).toBeNull();
    expect(screen.queryByText(/please invite/)).toBeNull();
    expect(screen.queryByRole('button', { name: joinStateCopy.other })).toBeNull();
  });

  it('believes the daemon when it recorded that the membership ended (Q3-50)', async () => {
    updateJoinContext('conn-1', { workspaceName: 'lab' });
    const ended = {
      ...fakeConnection(),
      last_error_code: 'crew_membership_ended',
    } as CrewConnection;
    answerJoin({ status: 'not_invited' });
    renderCard({ connection: ended, connections: [ended] });

    expect(await screen.findByText(joinStateCopy.removedTitle('lab'))).toBeInTheDocument();
    expect(
      screen.getByText(
        'This computer or your account was removed from lab. If you didn’t expect that, ask your host.'
      )
    ).toBeInTheDocument();
  });

  it('keeps "not in … yet" for someone this computer never saw admitted (Q3-50)', async () => {
    updateJoinContext('conn-1', { workspaceName: 'lab' });
    const other = {
      ...fakeConnection(),
      last_error_code: 'crew_ssh_auth_required',
    } as CrewConnection;
    answerJoin({ status: 'not_invited' });
    renderCard({ connection: other, connections: [other] });
    expect(await screen.findByText(joinStateCopy.notInvitedTitle('lab'))).toBeInTheDocument();
    expect(screen.queryByText(joinStateCopy.removedTitle('lab'))).toBeNull();
  });

  it('keeps the card at the top of the column, so an opened section grows it downward (Q3-48)', async () => {
    answerJoin({ status: 'invited', code: LOCAL_CODE, inviter: ALICE });
    renderCard();
    await screen.findByRole('button', { name: joinStateCopy.other });
    const column = document.querySelector('.crew-onboard-screen');
    expect(column).toHaveAttribute('data-anchor', 'top');

    // jsdom lays nothing out, so the rule that does the anchoring is checked where it is written:
    // top-anchored screens are not vertically centred.
    const css = readFileSync(join(__dirname, 'onboarding.css'), 'utf8');
    const rule = css.match(/\.crew-onboard-screen\[data-anchor='top'\]\s*\{([^}]*)\}/);
    expect(rule?.[1]).toMatch(/justify-content:\s*flex-start/);
  });
});
