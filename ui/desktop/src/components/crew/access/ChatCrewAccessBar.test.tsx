import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError } from '../crewApi';
import {
  forgetChannelLabels,
  rememberChannelLabels,
  useChatCrewAccess,
  useChatCrewAccessState,
} from './chatCrewAccess';
import { ChatCrewAccessBar } from './ChatCrewAccessBar';
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
  return (
    <div>
      <ChatCrewAccessBar access={access} chatTitle="Plot review" />
      <p data-testid="blocked">{String(access.blocksComposer)}</p>
      <p data-testid="state">{access.state}</p>
      <p data-testid="published">{String(published)}</p>
    </div>
  );
}

function renderChat(sessionId?: string) {
  return render(
    <MemoryRouter>
      <Chat sessionId={sessionId} />
    </MemoryRouter>
  );
}

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

    fireEvent.click(chip);
    expect(mocks.navigate).toHaveBeenCalledWith('/crew?sessionId=chat-1');
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
    fireEvent.click(within(lapsed).getByRole('button', { name: accessCopy.chatGrantAgain }));
    expect(mocks.navigate).toHaveBeenCalledWith('/crew?sessionId=chat-1');
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
