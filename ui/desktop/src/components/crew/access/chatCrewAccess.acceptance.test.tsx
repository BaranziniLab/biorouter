import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError } from '../crewApi';
import { forgetChannelLabels, useChatCrewAccess, type ChatTurnErrorText } from './chatCrewAccess';
import { ChatCrewAccessBar } from './ChatCrewAccessBar';
import { accessCopy } from './copy';
import { crewTurnRefusalCopy } from './crewTurnRefusal';
import { connection, grantRow } from './testing';
import {
  announceGrantsChanged,
  forgetUnconfirmedRevocations,
  UNCONFIRMED_REVOKE_WATCH_MS,
} from './useCrewGrants';

/**
 * The ordinary chat's Crew bar against the final acceptance findings: D-1 (a policy change reads as
 * the settings sentence and holds the chat, with Grant access again), F3 (a revoke waiting for the
 * workspace follows the daemon's own confirmation, and never tells a connected person to
 * reconnect), and O-1 (nothing here ever moves the window to Crew: only a person's click does).
 */

const mocks = vi.hoisted(() => ({ crewHttp: vi.fn() }));
vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: mocks.crewHttp };
});

/** What the daemon answers now; tests change it as the story goes. */
let daemon: {
  connections: Record<string, unknown>[];
  grants: Record<string, unknown>[];
  revoke?: () => unknown;
};

function installDaemon() {
  mocks.crewHttp.mockImplementation(async (path: string, method = 'GET') => {
    if (path === '/connections') return { connections: daemon.connections };
    if (path === '/connections/conn-1/grants' && method === 'GET') return { grants: daemon.grants };
    if (path.endsWith('/revoke') && method === 'POST')
      return daemon.revoke ? daemon.revoke() : { revoked: true, remote_revocation_confirmed: true };
    return {};
  });
}

function Where() {
  const location = useLocation();
  return <p data-testid="location">{`${location.pathname}${location.search}`}</p>;
}

/** What BaseChat mounts, with the chat's turn error handed in as BaseChat hands it. */
function Chat({ turnError }: { turnError?: ChatTurnErrorText | null }) {
  const access = useChatCrewAccess('chat-1', { turnError });
  return (
    <div>
      <ChatCrewAccessBar access={access} chatTitle="Plot review" />
      <p data-testid="state">{access.state}</p>
      <p data-testid="because">{String(access.expiredBecause)}</p>
      <p data-testid="blocked">{String(access.blocksComposer)}</p>
    </div>
  );
}

const CHAT_ROUTE = '/pair?resumeSessionId=chat-1';

function renderChat(turnError?: ChatTurnErrorText | null) {
  const view = render(
    <MemoryRouter initialEntries={[CHAT_ROUTE]}>
      <Chat turnError={turnError} />
      <Where />
    </MemoryRouter>
  );
  return {
    ...view,
    rerenderWith(next?: ChatTurnErrorText | null) {
      view.rerender(
        <MemoryRouter initialEntries={[CHAT_ROUTE]}>
          <Chat turnError={next} />
          <Where />
        </MemoryRouter>
      );
    },
  };
}

const state = () => screen.getByTestId('state').textContent;
const location = () => screen.getByTestId('location').textContent;
const grantReads = () =>
  mocks.crewHttp.mock.calls.filter(([path]) => path === '/connections/conn-1/grants').length;

/** The workspace's own refusal, as a daemon from before D-1 passed it on to the chat. */
const ENVELOPE =
  'Crew broker refused request: {"code":"grant_expired","message":"grant_expired: run revoked, expired or policy changed"}';

beforeEach(() => {
  vi.clearAllMocks();
  forgetUnconfirmedRevocations();
  forgetChannelLabels();
  daemon = { connections: [connection], grants: [grantRow({ session_id: 'chat-1' })] };
  installDaemon();
});
afterEach(() => {
  vi.useRealTimers();
  Reflect.deleteProperty(document, 'visibilityState');
});

describe('a Crew refusal in the chat (D-1)', () => {
  it('holds a chat whose grant was made under another policy epoch, as settings changed', async () => {
    daemon.connections = [{ ...connection, policy_epoch: 3 }];
    daemon.grants = [grantRow({ session_id: 'chat-1', policy_epoch: 2 })];
    renderChat();
    await waitFor(() => expect(state()).toBe('expired'));
    expect(screen.getByTestId('because')).toHaveTextContent('settings');
    expect(screen.getByTestId('blocked')).toHaveTextContent('true');
    expect(screen.getByTestId('crew-chat-access-lapsed')).toHaveTextContent(
      'Crew settings changed since this chat was given access to'
    );
    expect(screen.getByRole('button', { name: accessCopy.chatGrantAgain })).toBeInTheDocument();
    // Not the live chip, and no Revoke beside a grant the daemon refuses.
    expect(screen.queryByRole('button', { name: accessCopy.revokeButton })).toBeNull();
  });

  it('reads the daemon’s ended_by_workspace as settings changed, not as a revoke', async () => {
    daemon.grants = [
      grantRow({ session_id: 'chat-1', expired: true, revocation: 'ended_by_workspace' }),
    ];
    renderChat();
    await waitFor(() => expect(state()).toBe('expired'));
    expect(screen.getByTestId('because')).toHaveTextContent('settings');
    expect(screen.queryByText(/was removed/)).toBeNull();
  });

  it('moves to the held state the moment a refused turn appears, and reads the grant again', async () => {
    const chat = renderChat(null);
    await waitFor(() => expect(state()).toBe('active'));
    expect(screen.getByRole('button', { name: /^Crew · / })).toBeInTheDocument();
    const reads = grantReads();

    chat.rerenderWith({ message: ENVELOPE });
    await waitFor(() => expect(state()).toBe('expired'));
    expect(screen.getByTestId('because')).toHaveTextContent('settings');
    expect(screen.getByTestId('blocked')).toHaveTextContent('true');
    expect(screen.getByRole('button', { name: accessCopy.chatGrantAgain })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /^Crew · / })).toBeNull();
    await waitFor(() => expect(grantReads()).toBeGreaterThan(reads));
  });

  it('reads the daemon’s own sentence as the same refusal', async () => {
    const chat = renderChat(undefined);
    await waitFor(() => expect(state()).toBe('active'));
    chat.rerenderWith({ message: crewTurnRefusalCopy.settingsChanged });
    await waitFor(() => expect(state()).toBe('expired'));
    chat.rerenderWith(undefined);
    chat.rerenderWith({
      message:
        "This chat's Crew access was removed. Start a new chat, or grant access again from Crew.",
    });
    await waitFor(() => expect(screen.getByTestId('blocked')).toHaveTextContent('true'));
  });

  it('holds nothing for a refusal already on screen when the chat opens: it may predate a new grant', async () => {
    renderChat({ message: ENVELOPE });
    await waitFor(() => expect(state()).toBe('active'));
    await act(async () => {
      await Promise.resolve();
    });
    expect(state()).toBe('active');
    expect(screen.getByTestId('blocked')).toHaveTextContent('false');
  });

  it('forgets the refusal when the chat is granted again', async () => {
    const chat = renderChat(undefined);
    await waitFor(() => expect(state()).toBe('active'));
    chat.rerenderWith({ message: ENVELOPE });
    await waitFor(() => expect(state()).toBe('expired'));

    daemon.grants = [grantRow({ session_id: 'chat-1', run_id: 'run-fresh' })];
    act(() =>
      announceGrantsChanged({ connectionId: 'conn-1', sessionId: 'chat-1', change: 'granted' })
    );
    await waitFor(() => expect(state()).toBe('active'));
  });
});

describe('a revoke waiting for the workspace (F3)', () => {
  it('says the workspace will confirm on reconnect while the connection is down — no “Reconnect to”', async () => {
    daemon.connections = [{ ...connection, status: 'disconnected' }];
    daemon.grants = [grantRow({ session_id: 'chat-1', expired: true, revocation: 'unconfirmed' })];
    renderChat();
    expect(await screen.findByText(accessCopy.unconfirmed)).toBeInTheDocument();
    expect(screen.queryByText(accessCopy.confirming)).toBeNull();
    expect(screen.queryByText(/Reconnect to/)).toBeNull();
    expect(screen.getByRole('button', { name: accessCopy.retry })).toBeInTheDocument();
  });

  it('follows the daemon from “Confirming with the workspace…” to “Confirmed”, with no click', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    daemon.grants = [grantRow({ session_id: 'chat-1', expired: true, revocation: 'unconfirmed' })];
    renderChat();
    // Connected: the daemon is asking the workspace again by itself.
    expect(await screen.findByText(accessCopy.confirming)).toBeInTheDocument();
    expect(screen.queryByText(/reconnect/i)).toBeNull();

    // The workspace confirms; the chat reads it on its own within the watch interval.
    daemon.grants = [grantRow({ session_id: 'chat-1', expired: true, revocation: 'confirmed' })];
    const reads = grantReads();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(UNCONFIRMED_REVOKE_WATCH_MS);
    });
    await waitFor(() => expect(grantReads()).toBeGreaterThan(reads));
    expect(await screen.findByText(accessCopy.confirmed)).toBeInTheDocument();
    expect(screen.queryByText(accessCopy.confirming)).toBeNull();
    expect(screen.queryByTestId('crew-access-unconfirmed')).toBeNull();
    // Still held: a confirmed revoke is still a revoke.
    expect(screen.getByTestId('blocked')).toHaveTextContent('true');

    // And the reads stop once nothing waits.
    const after = grantReads();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(UNCONFIRMED_REVOKE_WATCH_MS * 4);
    });
    expect(grantReads()).toBe(after);
  });

  it('turns a 503 answered in this bar into “Confirmed” once the daemon confirms it', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    let stopped = false;
    daemon.revoke = () => {
      stopped = true;
      daemon.grants = [
        grantRow({ session_id: 'chat-1', expired: true, revocation: 'unconfirmed' }),
      ];
      throw new CrewHttpError('Stopped here.', 503, 'crew_revocation_unconfirmed');
    };
    renderChat();
    fireEvent.click(await screen.findByRole('button', { name: accessCopy.revokeButton }));
    fireEvent.click(screen.getByRole('button', { name: accessCopy.confirmRevoke }));
    expect(await screen.findByText(accessCopy.confirming)).toBeInTheDocument();
    expect(stopped).toBe(true);

    daemon.grants = [grantRow({ session_id: 'chat-1', expired: true, revocation: 'confirmed' })];
    await act(async () => {
      await vi.advanceTimersByTimeAsync(UNCONFIRMED_REVOKE_WATCH_MS);
    });
    expect(await screen.findByText(accessCopy.confirmed)).toBeInTheDocument();
    expect(screen.queryByTestId('crew-access-unconfirmed')).toBeNull();
  });
});

describe('the chat never moves the window to Crew by itself (O-1)', () => {
  it('stays put through every grant, turn, connection and window event, and moves on a click', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    let visibility: 'visible' | 'hidden' = 'visible';
    Object.defineProperty(document, 'visibilityState', {
      configurable: true,
      get: () => visibility,
    });
    const chat = renderChat(undefined);
    await waitFor(() => expect(state()).toBe('active'));
    expect(location()).toBe(CHAT_ROUTE);

    const stays = async () => {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(20_000);
      });
      expect(location()).toBe(CHAT_ROUTE);
    };

    // Grant announcements from any surface: revoked, unconfirmed, stopped, then granted again
    // (which is what makes the chat usable once more).
    for (const change of ['revoked', 'unconfirmed', 'stopped', 'granted'] as const) {
      act(() => announceGrantsChanged({ connectionId: 'conn-1', sessionId: 'chat-1', change }));
      await stays();
    }
    await waitFor(() => expect(state()).toBe('active'));
    // A turn ending, the window coming back, the network coming and going.
    act(() => {
      window.dispatchEvent(new CustomEvent('message-stream-finished'));
      window.dispatchEvent(new Event('focus'));
      visibility = 'hidden';
      document.dispatchEvent(new Event('visibilitychange'));
      visibility = 'visible';
      document.dispatchEvent(new Event('visibilitychange'));
      window.dispatchEvent(new Event('offline'));
      window.dispatchEvent(new Event('online'));
    });
    await stays();

    // The connection drops (the offline bar, with its Connect in Crew) and comes back.
    daemon.connections = [{ ...connection, status: 'disconnected' }];
    await stays();
    await waitFor(() => expect(screen.getByTestId('crew-chat-access-offline')).toBeInTheDocument());
    await stays();
    daemon.connections = [connection];
    await stays();
    await waitFor(() => expect(state()).toBe('active'));

    // A turn refused for its Crew access, and a revoke that stops only on this device.
    chat.rerenderWith({ message: ENVELOPE });
    await waitFor(() => expect(state()).toBe('expired'));
    await stays();
    daemon.grants = [grantRow({ session_id: 'chat-1', expired: true, revocation: 'unconfirmed' })];
    act(() =>
      announceGrantsChanged({ connectionId: 'conn-1', sessionId: 'chat-1', change: 'unconfirmed' })
    );
    await stays();
    daemon.grants = [grantRow({ session_id: 'chat-1', expired: true, revocation: 'confirmed' })];
    await stays();
    expect(await screen.findByText(accessCopy.confirmed)).toBeInTheDocument();
    await stays();

    // Only the person's own click leaves for Crew.
    fireEvent.click(screen.getByRole('button', { name: accessCopy.chatGrantAgain }));
    expect(location()).toBe('/crew?sessionId=chat-1');
  });
});
