import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { useEffect } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { ToastContainer, toast, type ToastTransitionProps } from 'react-toastify';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { toastWarning } from '../../../toasts';
import { CrewHttpError } from '../crewApi';
import {
  forgetChannelLabels,
  rememberChannelLabels,
  useChatCrewAccess,
  useChatCrewAccessState,
} from './chatCrewAccess';
import { ChatCrewAccessBar, crewHoldToastId, useCrewComposerHold } from './ChatCrewAccessBar';
import {
  CREW_CONNECT_ROUTE_STATE,
  chatAccessRouteState,
  crewConnectRequestOf,
} from './ChatConnectNote';
import { accessCopy } from './copy';
import { connection, grantRow } from './testing';
import { announceGrantsChanged, forgetUnconfirmedRevocations } from './useCrewGrants';

const mocks = vi.hoisted(() => ({ crewHttp: vi.fn(), navigate: vi.fn() }));

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: mocks.crewHttp };
});
vi.mock('react-router-dom', async () => {
  const actual = await vi.importActual<typeof import('react-router-dom')>('react-router-dom');
  return { ...actual, useNavigate: () => mocks.navigate };
});

interface Daemon {
  connections?: unknown[];
  grants: () => unknown[];
  revoke?: () => unknown;
  failConnections?: boolean;
}

function installDaemon(daemon: Daemon) {
  mocks.crewHttp.mockImplementation(async (path: string, method = 'GET') => {
    if (path === '/connections') {
      if (daemon.failConnections) throw new Error('daemon down');
      return { connections: daemon.connections ?? [connection] };
    }
    if (path === '/connections/conn-1/grants' && method === 'GET')
      return { grants: daemon.grants() };
    if (path.endsWith('/revoke') && method === 'POST')
      return daemon.revoke ? daemon.revoke() : { revoked: true, remote_revocation_confirmed: true };
    return {};
  });
}

/** What BaseChat mounts: the lookup, the bar above the composer, and the composer's hold. */
function Chat({ sessionId = 'chat-1' }: { sessionId?: string }) {
  const access = useChatCrewAccess(sessionId);
  const published = useChatCrewAccessState(sessionId);
  const hold = useCrewComposerHold(sessionId);
  return (
    <div>
      <ChatCrewAccessBar access={access} chatTitle="Plot review" />
      <p data-testid="blocked">{String(access.blocksComposer)}</p>
      <p data-testid="state">{access.state}</p>
      <p data-testid="published">{String(published)}</p>
      <p data-testid="hold">{hold ? `${hold.title} | ${hold.message}` : 'none'}</p>
      <Composer sessionId={sessionId} blocked={access.blocksComposer} />
    </div>
  );
}

const sent = vi.fn();

/**
 * Enter, as `ChatInput` handles it (`ChatInput.tsx`, the `Enter` branch of its key handler): send
 * when nothing holds the chat, else show the Crew hold's toast — `BaseChat` passes
 * `access.blocksComposer` as `submissionBlocked`. `ChatInput.crewCommand.test.tsx` pins the
 * composer's half with the real component; this is the other half, from the real lookup and bar.
 */
function Composer({ sessionId, blocked }: { sessionId: string; blocked: boolean }) {
  const hold = useCrewComposerHold(sessionId);
  return (
    <textarea
      aria-label="Message"
      defaultValue="Thanks! Can you also tell me what a good OD600 starting value is?"
      onKeyDown={(event) => {
        if (event.key !== 'Enter') return;
        event.preventDefault();
        if (!blocked) sent();
        else if (hold) toastWarning({ title: hold.title, msg: hold.message });
      }}
    />
  );
}

/** Toasts appear and leave at once, as in `DeclassifySessionDialog.toastLayer.test.tsx`. */
function NoAnimation({ children, isIn, done }: ToastTransitionProps) {
  useEffect(() => {
    if (!isIn) done();
  }, [isIn, done]);
  return <>{children}</>;
}

/** The route state a one-hop navigation carried: the Chat access pane intent, nothing else. */
function expectOneHop(sessionId: string) {
  const calls = mocks.navigate.mock.calls;
  const call = calls[calls.length - 1];
  expect(call?.[0]).toBe(`/crew?sessionId=${sessionId}`);
  const options = call?.[1] as { state?: Record<string, unknown> } | undefined;
  expect(Object.keys(options?.state ?? {})).toEqual(Object.keys(chatAccessRouteState()));
  expect(Object.values(options?.state ?? {})[0]).toEqual(expect.any(String));
}

function renderChat(sessionId?: string) {
  return render(
    <MemoryRouter>
      <Chat sessionId={sessionId} />
    </MemoryRouter>
  );
}

/** The chat beside a toast layer that is not part of it, as the app mounts one per window. */
function renderChatWithToasts() {
  render(<ToastContainer transition={NoAnimation} />);
  return renderChat();
}

const pressEnter = () =>
  fireEvent.keyDown(screen.getByRole('textbox', { name: 'Message' }), { key: 'Enter' });

const lookupCalls = () => mocks.crewHttp.mock.calls.filter(([path]) => path === '/connections');

describe('the ordinary chat’s Crew access', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    forgetUnconfirmedRevocations();
    forgetChannelLabels();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('shows nothing and holds nothing when the chat has no grant', async () => {
    installDaemon({ grants: () => [grantRow({ session_id: 'someone-else' })] });
    renderChat();
    await waitFor(() => expect(screen.getByTestId('state')).toHaveTextContent('none'));
    expect(screen.queryByTestId('crew-chat-access-bar')).toBeNull();
    expect(screen.getByTestId('blocked')).toHaveTextContent('false');
  });

  it('shows nothing, holds nothing and asks no further when there is no saved connection', async () => {
    installDaemon({ connections: [], grants: () => [] });
    renderChat();
    await waitFor(() => expect(screen.getByTestId('state')).toHaveTextContent('none'));
    expect(mocks.crewHttp.mock.calls.map(([path]) => path)).toEqual(['/connections']);
    act(() => {
      window.dispatchEvent(new Event('focus'));
    });
    expect(lookupCalls()).toHaveLength(1);
  });

  it('never holds the composer when the lookup fails', async () => {
    installDaemon({ failConnections: true, grants: () => [] });
    renderChat();
    await waitFor(() => expect(lookupCalls()).toHaveLength(1));
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByTestId('state')).toHaveTextContent('unknown');
    expect(screen.getByTestId('blocked')).toHaveTextContent('false');
    expect(screen.queryByTestId('crew-chat-access-bar')).toBeNull();
  });

  it('says a connected chat is connected, and where its channel is, when Crew named it', async () => {
    rememberChannelLabels('conn-1', new Map([['channel-1', '#general']]), ['channel-1']);
    installDaemon({ grants: () => [grantRow({ session_id: 'chat-1' })] });
    renderChat();
    const chip = await screen.findByRole('button', { name: accessCopy.chatChipName('#general') });
    expect(chip).toHaveTextContent('Crew · #general');
    expect(screen.getByRole('button', { name: accessCopy.revokeButton })).toBeInTheDocument();
    expect(screen.getByTestId('blocked')).toHaveTextContent('false');
    expect(screen.getByTestId('published')).toHaveTextContent('active');
    expect(screen.getByTestId('hold')).toHaveTextContent('none');

    // "…, manage access": the chip opens the chat's access pane in Crew, in one hop.
    fireEvent.click(chip);
    expectOneHop('chat-1');
  });

  /**
   * T-55: "Revoke access" was a ghost button — plain text beside a chip. It is a filled secondary
   * control now. jsdom loads no stylesheet, so the variant's classes are what can be asserted.
   */
  it('draws Revoke access as a real button, not ghost text', async () => {
    installDaemon({ grants: () => [grantRow({ session_id: 'chat-1' })] });
    renderChat();
    const revoke = await screen.findByRole('button', { name: accessCopy.revokeButton });
    expect(revoke.className).toMatch(/\bbg-background-medium\b/);
    expect(revoke.className).not.toMatch(/\bbg-transparent\b/);
    expect(revoke.className).toMatch(/\bh-control-sm\b/);
  });

  it('names the workspace when this computer has not seen the channel’s name', async () => {
    installDaemon({ grants: () => [grantRow({ session_id: 'chat-1' })] });
    renderChat();
    expect(
      await screen.findByRole('button', {
        name: accessCopy.chatChipName(accessCopy.chatDestinationWorkspace('Fixture')),
      })
    ).toBeInTheDocument();
  });

  it('names the channel as the person saw it when granting, ahead of anything remembered', async () => {
    rememberChannelLabels('conn-1', new Map([['channel-1', '#remembered']]), ['channel-1']);
    installDaemon({
      grants: () => [
        grantRow({
          session_id: 'chat-1',
          labels: {
            workspace: 'lab',
            destination: { channel_id: 'channel-1', label: '#methods', team: 'Analysis Lab' },
          },
        }),
      ],
    });
    renderChat();
    expect(
      await screen.findByRole('button', { name: accessCopy.chatChipName('#methods') })
    ).toHaveTextContent('Crew · #methods');
  });

  it('falls back to the remembered name, then the recorded workspace, then the saved one', async () => {
    const destinationFor = async (labels: unknown, remember: boolean) => {
      forgetChannelLabels();
      if (remember)
        rememberChannelLabels('conn-1', new Map([['channel-1', '#remembered']]), ['channel-1']);
      installDaemon({ grants: () => [grantRow({ session_id: 'chat-1', labels })] });
      const view = renderChat();
      const chip = await screen.findByRole('button', { name: /^Crew · / });
      const text = chip.textContent;
      view.unmount();
      return text;
    };

    // A label for another channel is not this grant's: the remembered name is used instead.
    expect(
      await destinationFor(
        { workspace: 'lab', destination: { channel_id: 'channel-9', label: '#elsewhere' } },
        true
      )
    ).toBe(accessCopy.chatChip('#remembered'));
    expect(await destinationFor({ workspace: 'lab' }, false)).toBe(
      accessCopy.chatChip(accessCopy.chatDestinationWorkspace('lab'))
    );
    expect(await destinationFor({ workspace: 42 }, false)).toBe(
      accessCopy.chatChip(accessCopy.chatDestinationWorkspace('Fixture'))
    );
  });

  it('says a Crew channel when nothing names the channel or the workspace', async () => {
    installDaemon({
      connections: [{ id: 'conn-1', name: '' }],
      grants: () => [grantRow({ session_id: 'chat-1' })],
    });
    renderChat();
    expect(
      await screen.findByRole('button', {
        name: accessCopy.chatChipName(accessCopy.chatDestinationUnknown),
      })
    ).toBeInTheDocument();
  });

  it('revokes after asking, then says the chat can’t continue and holds the composer', async () => {
    rememberChannelLabels('conn-1', new Map([['channel-1', '#general']]), ['channel-1']);
    let revoked = false;
    installDaemon({
      grants: () => [grantRow({ session_id: 'chat-1', expired: revoked })],
      revoke: () => {
        revoked = true;
        return { revoked: true, remote_revocation_confirmed: true };
      },
    });
    renderChat();
    fireEvent.click(await screen.findByRole('button', { name: accessCopy.revokeButton }));
    const confirm = screen.getByRole('group', {
      name: 'Stop “Plot review” reading and posting in #general?',
    });
    fireEvent.click(within(confirm).getByRole('button', { name: accessCopy.confirmRevoke }));

    await waitFor(() =>
      expect(mocks.crewHttp).toHaveBeenCalledWith(
        '/connections/conn-1/sessions/chat-1/revoke',
        'POST'
      )
    );
    expect(await screen.findByText(accessCopy.chatRevoked('#general'))).toBeInTheDocument();
    await waitFor(() => expect(screen.getByTestId('blocked')).toHaveTextContent('true'));
    expect(screen.getByTestId('published')).toHaveTextContent('revoked');
  });

  it('offers a new chat or access again when the grant was revoked', async () => {
    installDaemon({ grants: () => [grantRow({ session_id: 'chat-1', expired: true })] });
    renderChat();
    const lapsed = await screen.findByTestId('crew-chat-access-lapsed');
    expect(lapsed).toHaveTextContent(
      accessCopy.chatRevoked(accessCopy.chatDestinationWorkspace('Fixture'))
    );
    expect(screen.getByTestId('blocked')).toHaveTextContent('true');

    fireEvent.click(within(lapsed).getByRole('button', { name: accessCopy.chatNewChat }));
    expect(mocks.navigate).toHaveBeenCalledWith('/pair', {
      replace: undefined,
      state: { newChat: true },
    });
    // One hop: straight to this chat's consent in Crew, not to a note whose button opens it.
    const grantAgain = within(lapsed).getByRole('button', { name: accessCopy.chatGrantAgain });
    expect(grantAgain.className).not.toMatch(/\bbg-transparent\b/);
    fireEvent.click(grantAgain);
    expectOneHop('chat-1');
  });

  /**
   * T-55: Enter in a held chat did nothing. The bar — the one place that knows the channel's name —
   * publishes the sentence the composer shows on Enter, and withdraws it when access is back or the
   * bar goes away.
   */
  it('publishes why a held chat cannot send, and withdraws it', async () => {
    rememberChannelLabels('conn-1', new Map([['channel-1', '#general']]), ['channel-1']);
    let expired = true;
    installDaemon({ grants: () => [grantRow({ session_id: 'chat-1', expired })] });
    const view = renderChat();
    await waitFor(() =>
      expect(screen.getByTestId('hold')).toHaveTextContent(
        `Can’t send | Crew access to #general was removed. Grant it again or start a new chat.`
      )
    );

    expired = false;
    act(() =>
      announceGrantsChanged({ connectionId: 'conn-1', sessionId: 'chat-1', change: 'granted' })
    );
    await waitFor(() => expect(screen.getByTestId('state')).toHaveTextContent('active'));
    expect(screen.getByTestId('hold')).toHaveTextContent('none');

    expired = true;
    act(() =>
      announceGrantsChanged({ connectionId: 'conn-1', sessionId: 'chat-1', change: 'revoked' })
    );
    await waitFor(() => expect(screen.getByTestId('hold')).not.toHaveTextContent('none'));
    view.unmount();

    // Nothing is left behind for a chat whose bar is gone.
    function Reader() {
      const hold = useCrewComposerHold('chat-1');
      return <p data-testid="orphan">{hold ? hold.message : 'none'}</p>;
    }
    render(<Reader />);
    expect(screen.getByTestId('orphan')).toHaveTextContent('none');
  });

  it('says an expired grant expired when Enter is pressed', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    rememberChannelLabels('conn-1', new Map([['channel-1', '#general']]), ['channel-1']);
    const expiresAt = Math.floor(Date.now() / 1000) + 2;
    installDaemon({ grants: () => [grantRow({ session_id: 'chat-1', expires_at: expiresAt })] });
    renderChat();
    await waitFor(() => expect(screen.getByTestId('state')).toHaveTextContent('active'));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3000);
    });
    await waitFor(() =>
      expect(screen.getByTestId('hold')).toHaveTextContent(
        'Crew access to #general expired. Grant it again or start a new chat.'
      )
    );
  });

  it('says an expired grant expired, and holds the composer at the moment it runs out', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const expiresAt = Math.floor(Date.now() / 1000) + 2;
    installDaemon({ grants: () => [grantRow({ session_id: 'chat-1', expires_at: expiresAt })] });
    renderChat();
    await waitFor(() => expect(screen.getByTestId('state')).toHaveTextContent('active'));
    const lookups = lookupCalls().length;

    await act(async () => {
      await vi.advanceTimersByTimeAsync(3000);
    });
    await waitFor(() => expect(screen.getByTestId('state')).toHaveTextContent('expired'));
    expect(screen.getByTestId('blocked')).toHaveTextContent('true');
    expect(
      screen.getByText(accessCopy.chatExpired(accessCopy.chatDestinationWorkspace('Fixture')))
    ).toBeInTheDocument();
    // The flip came from the clock, not from asking the daemon again.
    expect(lookupCalls()).toHaveLength(lookups);
  });

  it('says a 503 stopped only on this device, with Retry', async () => {
    let stopped = false;
    installDaemon({
      grants: () => [grantRow({ session_id: 'chat-1', expired: stopped })],
      revoke: () => {
        stopped = true;
        throw new CrewHttpError('Stopped here.', 503, 'crew_revocation_unconfirmed');
      },
    });
    renderChat();
    fireEvent.click(await screen.findByRole('button', { name: accessCopy.revokeButton }));
    fireEvent.click(screen.getByRole('button', { name: accessCopy.confirmRevoke }));
    expect(await screen.findByText(accessCopy.unconfirmed)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: accessCopy.retry })).toBeInTheDocument();
    await waitFor(() => expect(screen.getByTestId('blocked')).toHaveTextContent('true'));
  });

  it('keeps the chat usable when the revoke was refused', async () => {
    installDaemon({
      grants: () => [grantRow({ session_id: 'chat-1' })],
      revoke: () => {
        throw new CrewHttpError('A person must approve this.', 403, 'crew_user_action_required');
      },
    });
    renderChat();
    fireEvent.click(await screen.findByRole('button', { name: accessCopy.revokeButton }));
    fireEvent.click(screen.getByRole('button', { name: accessCopy.confirmRevoke }));
    expect(await screen.findByRole('alert')).toHaveTextContent(
      `${accessCopy.notRevoked} A person must approve this.`
    );
    expect(screen.getByTestId('blocked')).toHaveTextContent('false');
  });

  it('looks again when Crew announces a change to this chat, and not for another', async () => {
    installDaemon({ grants: () => [] });
    renderChat();
    await waitFor(() => expect(screen.getByTestId('state')).toHaveTextContent('none'));
    expect(lookupCalls()).toHaveLength(1);
    act(() =>
      announceGrantsChanged({ connectionId: 'conn-1', sessionId: 'other', change: 'granted' })
    );
    expect(lookupCalls()).toHaveLength(1);
    act(() =>
      announceGrantsChanged({ connectionId: 'conn-1', sessionId: 'chat-1', change: 'granted' })
    );
    await waitFor(() => expect(lookupCalls()).toHaveLength(2));
  });

  it('asks nothing for a chat that has no session yet', () => {
    installDaemon({ grants: () => [] });
    render(
      <MemoryRouter>
        <Chat sessionId="" />
      </MemoryRouter>
    );
    expect(mocks.crewHttp).not.toHaveBeenCalled();
    expect(screen.getByTestId('published')).toHaveTextContent('null');
  });
});

/**
 * Q2-08 (live QA round 2): a chat whose grant stands while its Crew connection is down showed
 * nothing, and its next turn failed as "Model request failed". It says Crew is offline now, with
 * the way to connect, and holds nothing.
 */
describe('a chat whose Crew connection is offline', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    forgetUnconfirmedRevocations();
    forgetChannelLabels();
  });

  it('says Crew is offline, offers Connect in Crew, and neither holds the chat nor unlocks Crew', async () => {
    rememberChannelLabels('conn-1', new Map([['channel-1', '#general']]), ['channel-1']);
    installDaemon({
      connections: [{ ...connection, status: 'disconnected' }],
      grants: () => [grantRow({ session_id: 'chat-1' })],
    });
    renderChat();

    const note = await screen.findByTestId('crew-chat-access-offline');
    expect(note).toHaveTextContent(accessCopy.chatOffline('#general'));
    expect(note).toHaveTextContent(
      'Crew is offline. This chat can’t read or post in #general until you connect.'
    );
    expect(screen.getByTestId('state')).toHaveTextContent('offline');
    expect(screen.getByTestId('blocked')).toHaveTextContent('false');
    expect(screen.getByTestId('hold')).toHaveTextContent('none');
    // The grant still stands, so the extension menu keeps Crew switched on.
    expect(screen.getByTestId('published')).toHaveTextContent('active');
    // Not the connected chip, and no Revoke beside a connection that cannot carry it.
    expect(screen.queryByRole('button', { name: /^Crew · / })).toBeNull();

    // Q3-08: one click, as its label says. Crew connects the grant's connection on arrival and
    // opens this chat's access on its channel: the route carries both.
    fireEvent.click(within(note).getByRole('button', { name: accessCopy.chatConnectInCrew }));
    const [to, options] = mocks.navigate.mock.calls[mocks.navigate.mock.calls.length - 1] as [
      string,
      { state?: Record<string, unknown> },
    ];
    expect(to).toBe('/crew?sessionId=chat-1');
    expect(options.state).toEqual({
      ...Object.fromEntries(
        Object.keys(chatAccessRouteState()).map((key) => [key, expect.any(String)])
      ),
      [CREW_CONNECT_ROUTE_STATE]: 'conn-1',
    });
    expect(Object.keys(options.state ?? {}).sort()).toEqual(
      [...Object.keys(chatAccessRouteState()), CREW_CONNECT_ROUTE_STATE].sort()
    );
    // What Crew reads back: the grant's connection, once per intent.
    expect(crewConnectRequestOf(options.state)).toEqual({
      intentId: expect.any(String),
      connectionId: 'conn-1',
    });
    // A plain one-hop intent asks for no connect.
    expect(crewConnectRequestOf(chatAccessRouteState())).toBeNull();
  });

  it('reads as connected once the connection is back', async () => {
    let status = 'disconnected';
    installDaemon({
      grants: () => [grantRow({ session_id: 'chat-1' })],
      get connections() {
        return [{ ...connection, status }];
      },
    });
    renderChat();
    await waitFor(() => expect(screen.getByTestId('state')).toHaveTextContent('offline'));

    status = 'connected';
    act(() => {
      window.dispatchEvent(new Event('focus'));
    });
    await waitFor(() => expect(screen.getByTestId('state')).toHaveTextContent('active'));
    expect(screen.queryByTestId('crew-chat-access-offline')).toBeNull();
    expect(screen.getByRole('button', { name: accessCopy.revokeButton })).toBeInTheDocument();
  });
});

/**
 * Q3-04 (live QA round 3, P1): an open, focused chat with Crew access kept its live "Crew ·
 * #general" chip and Revoke for six minutes into an outage, and changed only on window focus or
 * reopen. While the chat holds a grant that stands and the window is visible, it re-reads the saved
 * connections — never the grants — every 15 s, and at once when the network comes or goes or the
 * window becomes visible again. Nothing reads on a timer without a grant, or while hidden.
 */
describe('an open chat notices a Crew outage while it is watched', () => {
  let visibility: Document['visibilityState'] = 'visible';

  beforeEach(() => {
    vi.clearAllMocks();
    forgetUnconfirmedRevocations();
    forgetChannelLabels();
    visibility = 'visible';
    Object.defineProperty(document, 'visibilityState', {
      configurable: true,
      get: () => visibility,
    });
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });
  afterEach(() => {
    vi.useRealTimers();
    // Back to jsdom's own answer.
    delete (document as { visibilityState?: unknown }).visibilityState;
  });

  const grantLists = () =>
    mocks.crewHttp.mock.calls.filter(([path]) => path === '/connections/conn-1/grants');

  function daemonWith(grants: () => unknown[]) {
    const saved = { status: 'connected' as string };
    installDaemon({
      grants,
      get connections() {
        return [{ ...connection, status: saved.status }];
      },
    });
    return saved;
  }

  async function wait(ms: number) {
    await act(async () => {
      await vi.advanceTimersByTimeAsync(ms);
    });
  }

  it('shows the offline bar within 15 s of the connection dropping, with no focus event', async () => {
    rememberChannelLabels('conn-1', new Map([['channel-1', '#general']]), ['channel-1']);
    const saved = daemonWith(() => [grantRow({ session_id: 'chat-1' })]);
    renderChat();
    expect(
      await screen.findByRole('button', { name: accessCopy.chatChipName('#general') })
    ).toBeInTheDocument();
    const grantsRead = grantLists().length;

    saved.status = 'disconnected';
    await wait(15_000);

    const note = await screen.findByTestId('crew-chat-access-offline');
    expect(note).toHaveTextContent(accessCopy.chatOffline('#general'));
    expect(screen.queryByRole('button', { name: accessCopy.revokeButton })).toBeNull();
    expect(screen.getByTestId('state')).toHaveTextContent('offline');
    // Only the connections were read again: the grant list is not polled.
    expect(grantLists()).toHaveLength(grantsRead);

    // And back: the chip returns on the next read, still without a focus event.
    saved.status = 'connected';
    await wait(15_000);
    await waitFor(() => expect(screen.getByTestId('state')).toHaveTextContent('active'));
    expect(screen.getByRole('button', { name: accessCopy.revokeButton })).toBeInTheDocument();
    expect(grantLists()).toHaveLength(grantsRead);
  });

  it('reads at once when the network goes or comes back', async () => {
    const saved = daemonWith(() => [grantRow({ session_id: 'chat-1' })]);
    renderChat();
    await waitFor(() => expect(screen.getByTestId('state')).toHaveTextContent('active'));
    const reads = lookupCalls().length;

    saved.status = 'disconnected';
    act(() => {
      window.dispatchEvent(new Event('offline'));
    });
    await waitFor(() => expect(screen.getByTestId('state')).toHaveTextContent('offline'));
    expect(lookupCalls()).toHaveLength(reads + 1);

    saved.status = 'connected';
    act(() => {
      window.dispatchEvent(new Event('online'));
    });
    await waitFor(() => expect(screen.getByTestId('state')).toHaveTextContent('active'));
    expect(lookupCalls()).toHaveLength(reads + 2);
  });

  it('reads nothing on a timer while the window is hidden, and reads at once when it is shown', async () => {
    const saved = daemonWith(() => [grantRow({ session_id: 'chat-1' })]);
    renderChat();
    await waitFor(() => expect(screen.getByTestId('state')).toHaveTextContent('active'));
    const reads = lookupCalls().length;

    visibility = 'hidden';
    act(() => {
      document.dispatchEvent(new Event('visibilitychange'));
    });
    saved.status = 'disconnected';
    await wait(60_000);
    expect(lookupCalls()).toHaveLength(reads);
    expect(screen.getByTestId('state')).toHaveTextContent('active');

    visibility = 'visible';
    act(() => {
      document.dispatchEvent(new Event('visibilitychange'));
    });
    await waitFor(() => expect(screen.getByTestId('state')).toHaveTextContent('offline'));
    expect(lookupCalls()).toHaveLength(reads + 1);
    // Watching again: the next read comes on the timer.
    await wait(15_000);
    expect(lookupCalls()).toHaveLength(reads + 2);
  });

  it('reads nothing on a timer for a chat without a grant', async () => {
    daemonWith(() => [grantRow({ session_id: 'someone-else' })]);
    renderChat();
    await waitFor(() => expect(screen.getByTestId('state')).toHaveTextContent('none'));
    const reads = lookupCalls().length;
    await wait(60_000);
    act(() => {
      window.dispatchEvent(new Event('offline'));
      window.dispatchEvent(new Event('online'));
      document.dispatchEvent(new Event('visibilitychange'));
    });
    await wait(0);
    expect(lookupCalls()).toHaveLength(reads);
  });

  it('stops watching once the grant no longer stands', async () => {
    let expired = false;
    daemonWith(() => [grantRow({ session_id: 'chat-1', expired })]);
    renderChat();
    await waitFor(() => expect(screen.getByTestId('state')).toHaveTextContent('active'));
    expired = true;
    act(() =>
      announceGrantsChanged({ connectionId: 'conn-1', sessionId: 'chat-1', change: 'revoked' })
    );
    await waitFor(() => expect(screen.getByTestId('state')).toHaveTextContent('revoked'));
    const reads = lookupCalls().length;
    await wait(60_000);
    expect(lookupCalls()).toHaveLength(reads);
  });

  it('keeps what it shows when a read fails: a missed read is not an outage', async () => {
    const saved = daemonWith(() => [grantRow({ session_id: 'chat-1' })]);
    renderChat();
    await waitFor(() => expect(screen.getByTestId('state')).toHaveTextContent('active'));
    const answer = mocks.crewHttp.getMockImplementation();
    mocks.crewHttp.mockImplementation(async (path: string, method?: string) => {
      if (path === '/connections') throw new Error('daemon busy');
      return answer?.(path, method);
    });
    const reads = lookupCalls().length;
    saved.status = 'disconnected';
    await wait(15_000);
    // The read was made, and failed.
    expect(lookupCalls().length).toBeGreaterThan(reads);
    expect(screen.getByTestId('state')).toHaveTextContent('active');
    expect(screen.getByRole('button', { name: accessCopy.revokeButton })).toBeInTheDocument();
  });

  it('looks the grant up again when its connection leaves the saved list', async () => {
    let connections: unknown[] = [connection];
    installDaemon({
      grants: () => [grantRow({ session_id: 'chat-1' })],
      get connections() {
        return connections;
      },
    });
    renderChat();
    await waitFor(() => expect(screen.getByTestId('state')).toHaveTextContent('active'));
    connections = [];
    await wait(15_000);
    await waitFor(() => expect(screen.getByTestId('state')).toHaveTextContent('none'));
    expect(screen.queryByTestId('crew-chat-access-bar')).toBeNull();
  });
});

/**
 * Q2-09 (live QA round 2): a task's grant ends when the task does (T-25), and its chat then read
 * "Crew access to #general was removed … can't continue" under a warning, after a task that
 * succeeded. It says the task is finished now, calmly, and offers only a new chat.
 */
describe('a finished task’s chat', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    forgetUnconfirmedRevocations();
    forgetChannelLabels();
  });

  it.each([
    ['ended with the task', { expired: true }],
    ['ran out', { expires_at: Math.floor(Date.now() / 1000) - 60 }],
  ])(
    'says the task is finished when its grant %s, with no warning and no re-grant',
    async (_, ended) => {
      rememberChannelLabels('conn-1', new Map([['channel-1', '#general']]), ['channel-1']);
      installDaemon({ grants: () => [grantRow({ session_id: 'chat-1', kind: 'task', ...ended })] });
      renderChat();

      const note = await screen.findByTestId('crew-chat-access-finished');
      expect(note).toHaveTextContent(
        'This task is finished. Its access to #general ended when it finished.'
      );
      expect(note).not.toHaveTextContent(/removed|can’t continue/);
      // Neutral, and no warning glyph.
      expect(note.querySelector('svg')).toBeNull();
      expect(screen.queryByTestId('crew-chat-access-lapsed')).toBeNull();
      expect(
        within(note).getByRole('button', { name: accessCopy.chatNewChat })
      ).toBeInTheDocument();
      expect(screen.queryByRole('button', { name: accessCopy.chatGrantAgain })).toBeNull();

      // The daemon refuses its turns all the same, so the chat is held, and Enter says why.
      expect(screen.getByTestId('state')).toHaveTextContent('finished');
      expect(screen.getByTestId('blocked')).toHaveTextContent('true');
      expect(screen.getByTestId('hold')).toHaveTextContent(
        `${accessCopy.chatBlockedSendTitle} | ${accessCopy.chatBlockedSendTaskFinished('#general')}`
      );
    }
  );

  it('still reads a chat’s revoked grant as removed, with Grant access again', async () => {
    installDaemon({ grants: () => [grantRow({ session_id: 'chat-1', expired: true })] });
    renderChat();
    const lapsed = await screen.findByTestId('crew-chat-access-lapsed');
    expect(lapsed).toHaveTextContent(
      'was removed, so this chat can’t continue. It holds messages from the channel. Grant access again to continue, or start a new chat.'
    );
    expect(
      within(lapsed).getByRole('button', { name: accessCopy.chatGrantAgain })
    ).toBeInTheDocument();
    expect(screen.queryByTestId('crew-chat-access-finished')).toBeNull();
  });
});

/**
 * Q2-73 and Q2-74 (live QA round 2): after "Revoke access" in the chat itself, Enter did nothing
 * at all; and the "Can't send" toast followed the person into Crew.
 */
describe('Enter after a revoke in the chat, and its toast', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    forgetUnconfirmedRevocations();
    forgetChannelLabels();
  });
  afterEach(async () => {
    await act(async () => {
      toast.dismiss();
    });
  });

  async function revokeInChat() {
    fireEvent.click(await screen.findByRole('button', { name: accessCopy.revokeButton }));
    fireEvent.click(screen.getByRole('button', { name: accessCopy.confirmRevoke }));
    await waitFor(() =>
      expect(mocks.crewHttp).toHaveBeenCalledWith(
        '/connections/conn-1/sessions/chat-1/revoke',
        'POST'
      )
    );
  }

  it('holds the chat the moment the revoke lands, even before the list says so, and Enter says why', async () => {
    rememberChannelLabels('conn-1', new Map([['channel-1', '#general']]), ['channel-1']);
    // The daemon's list lags behind its own revoke: it still says active.
    installDaemon({ grants: () => [grantRow({ session_id: 'chat-1' })] });
    renderChatWithToasts();
    await screen.findByRole('button', { name: accessCopy.revokeButton });
    const lookupsBefore = lookupCalls().length;

    await revokeInChat();

    await waitFor(() => expect(screen.getByTestId('blocked')).toHaveTextContent('true'));
    expect(screen.getByTestId('state')).toHaveTextContent('revoked');
    expect(screen.getByTestId('published')).toHaveTextContent('revoked');
    expect(await screen.findByText(accessCopy.chatRevoked('#general'))).toBeInTheDocument();
    // The chat read its grant again after the revoke.
    await waitFor(() => expect(lookupCalls().length).toBeGreaterThan(lookupsBefore));
    // …and a list that still says active does not undo the hold.
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByTestId('blocked')).toHaveTextContent('true');

    pressEnter();
    expect(sent).not.toHaveBeenCalled();
    const shown = await screen.findByText(accessCopy.chatBlockedSendRevoked('#general'));
    expect(shown).toBeInTheDocument();
    expect(screen.getByText(accessCopy.chatBlockedSendTitle)).toBeInTheDocument();
  });

  it('forgets the revoke once access is granted again', async () => {
    let expired = false;
    let run = 'run-1';
    installDaemon({
      grants: () => [grantRow({ session_id: 'chat-1', expired, run_id: run })],
      revoke: () => {
        expired = true;
        return { revoked: true, remote_revocation_confirmed: true };
      },
    });
    renderChat();
    await revokeInChat();
    await waitFor(() => expect(screen.getByTestId('state')).toHaveTextContent('revoked'));

    expired = false;
    run = 'run-2';
    act(() =>
      announceGrantsChanged({ connectionId: 'conn-1', sessionId: 'chat-1', change: 'granted' })
    );
    await waitFor(() => expect(screen.getByTestId('state')).toHaveTextContent('active'));
    expect(screen.getByTestId('blocked')).toHaveTextContent('false');

    pressEnter();
    expect(sent).toHaveBeenCalledTimes(1);
  });

  it('never holds the chat for a revoke the daemon refused', async () => {
    installDaemon({
      grants: () => [grantRow({ session_id: 'chat-1' })],
      revoke: () => {
        throw new CrewHttpError('A person must approve this.', 403, 'crew_user_action_required');
      },
    });
    renderChat();
    await revokeInChat();
    expect(await screen.findByRole('alert')).toHaveTextContent(accessCopy.notRevoked);
    expect(screen.getByTestId('state')).toHaveTextContent('active');
    expect(screen.getByTestId('blocked')).toHaveTextContent('false');
  });

  it('shows one toast under a fixed id however often Enter is pressed, and takes it down with the chat', async () => {
    rememberChannelLabels('conn-1', new Map([['channel-1', '#general']]), ['channel-1']);
    installDaemon({ grants: () => [grantRow({ session_id: 'chat-1', expired: true })] });
    const view = renderChatWithToasts();
    await waitFor(() => expect(screen.getByTestId('blocked')).toHaveTextContent('true'));

    pressEnter();
    pressEnter();
    const message = accessCopy.chatBlockedSendRevoked('#general');
    await screen.findByText(message);
    expect(screen.getAllByText(message)).toHaveLength(1);
    const id = crewHoldToastId(accessCopy.chatBlockedSendTitle, message);
    expect(toast.isActive(id)).toBe(true);

    // Leaving the chat — for Crew, or anywhere — takes its reason with it.
    view.unmount();
    await waitFor(() => expect(screen.queryByText(message)).toBeNull());
    expect(toast.isActive(id)).toBe(false);
  });

  it('takes the toast down when the chat leaves for Crew from Grant access again', async () => {
    rememberChannelLabels('conn-1', new Map([['channel-1', '#general']]), ['channel-1']);
    installDaemon({ grants: () => [grantRow({ session_id: 'chat-1', expired: true })] });
    renderChatWithToasts();
    await waitFor(() => expect(screen.getByTestId('blocked')).toHaveTextContent('true'));
    pressEnter();
    const message = accessCopy.chatBlockedSendRevoked('#general');
    await screen.findByText(message);

    fireEvent.click(screen.getByRole('button', { name: accessCopy.chatGrantAgain }));
    expectOneHop('chat-1');
    await waitFor(() => expect(screen.queryByText(message)).toBeNull());
  });
});
