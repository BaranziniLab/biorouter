import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError } from '../crewApi';
import { useCrew } from '../state/CrewControllerContext';
import { ChatAccessPane } from './ChatAccessPane';
import { ChatConnectNote } from './ChatConnectNote';
import { accessCopy } from './copy';
import {
  callsTo,
  grantRow,
  installDaemon,
  renderWithController,
  type DaemonFixture,
} from './testing';
import { forgetUnconfirmedRevocations } from './useCrewGrants';

const mocks = vi.hoisted(() => ({
  crewHttp: vi.fn(),
  crewRequest: vi.fn(),
  observeCrew: vi.fn(),
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
vi.mock('react-router-dom', async () => {
  const actual = await vi.importActual<typeof import('react-router-dom')>('react-router-dom');
  return { ...actual, useNavigate: () => mocks.navigate };
});

const REVOKE_PATH = '/connections/conn-1/sessions/agent-1/revoke';
const GRANT_PATH = '/connections/conn-1/sessions/agent-1/grant';
const GRANTS_PATH = '/connections/conn-1/grants';

/** The composer's note slot and the pane, as the layout places them. */
function Layout() {
  const { ui, channel } = useCrew();
  return (
    <div>
      {channel ? <p data-testid="channel-ready">{channel.name}</p> : null}
      <ChatConnectNote />
      {ui.pane?.mode === 'chat-access' ? (
        <aside data-testid="pane">
          <ChatAccessPane />
        </aside>
      ) : null}
    </div>
  );
}

function setup(fixture: Partial<DaemonFixture> & { grants?: () => unknown[] }) {
  installDaemon(mocks, { grants: () => [], ...fixture });
  renderWithController(Layout);
}

const note = () => screen.getByTestId('crew-chat-connect-note');
const pane = () => screen.getByTestId('pane');

async function openPaneFromNote(name: string | RegExp) {
  await screen.findByTestId('channel-ready');
  const noteNode = await screen.findByTestId('crew-chat-connect-note');
  fireEvent.click(await within(noteNode).findByRole('button', { name }));
  await waitFor(() => expect(pane()).not.toHaveTextContent(accessCopy.noteChecking));
  return pane();
}

describe('chat access: grant', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    forgetUnconfirmedRevocations();
  });

  it('keeps the Allow flow when the chat has no grant, and does not navigate after Allow', async () => {
    const grants: unknown[] = [];
    setup({ grants: () => grants });

    await waitFor(() => expect(note()).toHaveTextContent(accessCopy.noteNone('#general')));
    const paneNode = await openPaneFromNote(accessCopy.noteReviewName);

    expect(paneNode).toHaveTextContent('This chat will be able to');
    expect(paneNode).toHaveTextContent('Read #general');
    expect(paneNode).toHaveTextContent('Post in #general as Alice Chen (@alice)');
    expect(paneNode).toHaveTextContent(accessCopy.expiry);
    expect(within(paneNode).queryByRole('button', { name: accessCopy.revokeButton })).toBeNull();

    grants.push(grantRow());
    fireEvent.click(within(paneNode).getByRole('button', { name: accessCopy.allow }));

    await waitFor(() => expect(callsTo(mocks, GRANT_PATH, 'POST')).toHaveLength(1));
    expect(callsTo(mocks, GRANT_PATH, 'POST')[0][2]).toMatchObject({
      channel_id: 'channel-1',
      context_channels: ['channel-1'],
      expected_mode: 'private',
    });
    expect(await within(pane()).findByText(accessCopy.connected)).toBeInTheDocument();
    expect(within(pane()).getByRole('button', { name: accessCopy.backToChat })).toBeInTheDocument();
    expect(
      within(pane()).getByRole('button', { name: accessCopy.revokeButton })
    ).toBeInTheDocument();
    expect(within(pane()).queryByRole('button', { name: accessCopy.allow })).toBeNull();
    // L10: the pane says where Revoke lives instead of leaving on its own.
    expect(mocks.navigate).not.toHaveBeenCalled();
    // The note follows the new grant without a poll.
    await waitFor(() => expect(note()).toHaveTextContent(accessCopy.noteActive('#general')));

    fireEvent.click(within(pane()).getByRole('button', { name: accessCopy.backToChat }));
    expect(mocks.navigate).toHaveBeenCalledWith('/pair?resumeSessionId=agent-1');
  });

  it('sends the channels chosen under Advanced as extra context', async () => {
    setup({});
    const paneNode = await openPaneFromNote(accessCopy.noteReviewName);
    expect(within(paneNode).queryByRole('checkbox')).toBeNull();

    fireEvent.click(within(paneNode).getByRole('button', { name: 'Advanced' }));
    const methods = await within(paneNode).findByRole('checkbox', { name: 'Lab / #methods' });
    // Archived channels are not offered.
    expect(within(paneNode).queryByRole('checkbox', { name: /old-notes/ })).toBeNull();
    fireEvent.click(methods);
    fireEvent.click(within(paneNode).getByRole('button', { name: accessCopy.allow }));

    await waitFor(() => expect(callsTo(mocks, GRANT_PATH, 'POST')).toHaveLength(1));
    expect(callsTo(mocks, GRANT_PATH, 'POST')[0][2]).toMatchObject({
      context_channels: ['channel-1', 'channel-2'],
    });
  });

  it('shows a refused grant once, in the pane, and keeps the Allow button', async () => {
    setup({
      grant: () => {
        throw new CrewHttpError('The workspace refused that model.', 403, 'crew_policy_refused');
      },
    });
    const paneNode = await openPaneFromNote(accessCopy.noteReviewName);
    const allow = within(paneNode).getByRole('button', { name: accessCopy.allow });
    fireEvent.click(allow);

    expect(await within(paneNode).findByRole('alert')).toHaveTextContent(
      'The workspace refused that model.'
    );
    expect(screen.getAllByText('The workspace refused that model.')).toHaveLength(1);
    expect(within(paneNode).getByRole('button', { name: accessCopy.allow })).toBe(allow);
    expect(within(paneNode).queryByText(accessCopy.connected)).toBeNull();
  });
});

describe('chat access: an active grant', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    forgetUnconfirmedRevocations();
  });

  it('shows Manage access and Revoke, not Allow', async () => {
    setup({ grants: () => [grantRow()] });
    await waitFor(() => expect(note()).toHaveTextContent(accessCopy.noteActive('#general')));
    expect(within(note()).queryByRole('button', { name: accessCopy.noteReviewName })).toBeNull();

    const paneNode = await openPaneFromNote(accessCopy.noteManage);
    expect(paneNode).toHaveTextContent('“Plot review” can');
    expect(paneNode).toHaveTextContent('Reads #general');
    expect(paneNode).toHaveTextContent('Posts in #general as Alice Chen (@alice)');
    expect(paneNode).toHaveTextContent(/Expires /);
    expect(
      within(paneNode).getByRole('button', { name: accessCopy.revokeButton })
    ).toBeInTheDocument();
    expect(within(paneNode).getByRole('button', { name: accessCopy.openChat })).toBeInTheDocument();
    expect(within(paneNode).queryByRole('button', { name: accessCopy.allow })).toBeNull();
  });

  it('confirms inline, revokes through the daemon and refetches to Revoked with Grant again', async () => {
    let revoked = false;
    setup({
      grants: () => [grantRow({ expired: revoked })],
      revoke: () => {
        revoked = true;
        return { revoked: true, remote_revocation_confirmed: true, session_id: 'agent-1' };
      },
    });
    const paneNode = await openPaneFromNote(accessCopy.noteManage);
    const listsBefore = callsTo(mocks, GRANTS_PATH, 'GET').length;

    fireEvent.click(within(paneNode).getByRole('button', { name: accessCopy.revokeButton }));
    const confirm = within(paneNode).getByRole('group', {
      name: 'Stop “Plot review” reading and posting in #general?',
    });
    expect(callsTo(mocks, REVOKE_PATH, 'POST')).toHaveLength(0);
    fireEvent.click(within(confirm).getByRole('button', { name: accessCopy.confirmRevoke }));

    await waitFor(() => expect(mocks.crewHttp).toHaveBeenCalledWith(REVOKE_PATH, 'POST'));
    expect(await within(pane()).findByText(accessCopy.revoked('Plot review'))).toBeInTheDocument();
    expect(within(pane()).getByRole('button', { name: accessCopy.openChat })).toBeInTheDocument();
    expect(within(pane()).getByRole('button', { name: accessCopy.done })).toBeInTheDocument();
    await waitFor(() =>
      expect(callsTo(mocks, GRANTS_PATH, 'GET').length).toBeGreaterThan(listsBefore)
    );
    await waitFor(() => expect(note()).toHaveTextContent(accessCopy.noteRevoked));
    expect(
      within(note()).getByRole('button', { name: accessCopy.noteGrantAgain })
    ).toBeInTheDocument();

    fireEvent.click(within(pane()).getByRole('button', { name: accessCopy.done }));
    await waitFor(() => expect(screen.queryByTestId('pane')).toBeNull());

    // Grant again reopens the consent for this chat.
    fireEvent.click(within(note()).getByRole('button', { name: accessCopy.noteGrantAgain }));
    expect(
      await within(pane()).findByText(accessCopy.paneRevoked('Plot review'))
    ).toBeInTheDocument();
    expect(within(pane()).getByRole('button', { name: accessCopy.allow })).toBeInTheDocument();
  });

  it('keeps access when the person chooses Keep access', async () => {
    setup({ grants: () => [grantRow()] });
    const paneNode = await openPaneFromNote(accessCopy.noteManage);
    fireEvent.click(within(paneNode).getByRole('button', { name: accessCopy.revokeButton }));
    const keep = within(paneNode).getByRole('button', { name: accessCopy.confirmKeep });
    await waitFor(() => expect(keep).toHaveFocus());
    fireEvent.click(keep);
    expect(within(paneNode).queryByRole('group')).toBeNull();
    expect(
      within(paneNode).getByRole('button', { name: accessCopy.revokeButton })
    ).toBeInTheDocument();
    expect(callsTo(mocks, REVOKE_PATH, 'POST')).toHaveLength(0);
  });

  it('reads a 503 crew_revocation_unconfirmed as stopped on this device, with Retry, never success', async () => {
    let stoppedLocally = false;
    let attempts = 0;
    setup({
      grants: () => [grantRow({ expired: stoppedLocally })],
      revoke: () => {
        attempts += 1;
        stoppedLocally = true;
        throw new CrewHttpError(
          'Stopped on this device. The workspace has not confirmed yet; reconnect and retry.',
          503,
          'crew_revocation_unconfirmed'
        );
      },
    });
    const paneNode = await openPaneFromNote(accessCopy.noteManage);
    fireEvent.click(within(paneNode).getByRole('button', { name: accessCopy.revokeButton }));
    fireEvent.click(within(paneNode).getByRole('button', { name: accessCopy.confirmRevoke }));

    expect(await within(pane()).findByText(accessCopy.unconfirmed)).toBeInTheDocument();
    expect(within(pane()).getByRole('alert')).toHaveTextContent(accessCopy.unconfirmed);
    expect(screen.queryByText(/Access revoked/)).toBeNull();
    // The list now shows the local stop; the pane keeps saying it is unconfirmed.
    await waitFor(() =>
      expect(within(pane()).getByText(accessCopy.status.unconfirmed)).toBeInTheDocument()
    );

    fireEvent.click(within(pane()).getByRole('button', { name: accessCopy.retry }));
    await waitFor(() => expect(attempts).toBe(2));
    expect(await within(pane()).findByText(accessCopy.unconfirmed)).toBeInTheDocument();
    expect(screen.queryByText(/Access revoked/)).toBeNull();
  });

  it.each([
    [
      400,
      'crew_profile_refused',
      'Crew connection is disconnected; authenticate and connect in Crew',
    ],
    [409, 'crew_grant_other_connection', 'This grant belongs to a different Crew connection.'],
    [403, 'crew_user_action_required', 'A person must approve this.'],
  ])(
    'reads a %i as not revoked, with the daemon’s words and Retry',
    async (status, code, message) => {
      setup({
        grants: () => [grantRow()],
        revoke: () => {
          throw new CrewHttpError(message, status, code);
        },
      });
      const paneNode = await openPaneFromNote(accessCopy.noteManage);
      fireEvent.click(within(paneNode).getByRole('button', { name: accessCopy.revokeButton }));
      fireEvent.click(within(paneNode).getByRole('button', { name: accessCopy.confirmRevoke }));

      const alert = await within(pane()).findByRole('alert');
      expect(alert).toHaveTextContent(`${accessCopy.notRevoked} ${message}`);
      expect(within(alert).getByRole('button', { name: accessCopy.retry })).toBeInTheDocument();
      expect(screen.queryByText(/Access revoked/)).toBeNull();
      expect(screen.queryByText(accessCopy.unconfirmed)).toBeNull();
      // Still active: the summary stays.
      expect(pane()).toHaveTextContent('Reads #general');
    }
  );

  it('never reads a 200 that says the workspace did not confirm as success', async () => {
    setup({
      grants: () => [grantRow()],
      revoke: () => ({ revoked: true, remote_revocation_confirmed: false }),
    });
    const paneNode = await openPaneFromNote(accessCopy.noteManage);
    fireEvent.click(within(paneNode).getByRole('button', { name: accessCopy.revokeButton }));
    fireEvent.click(within(paneNode).getByRole('button', { name: accessCopy.confirmRevoke }));
    expect(await within(pane()).findByText(accessCopy.unconfirmed)).toBeInTheDocument();
    expect(screen.queryByText(/Access revoked/)).toBeNull();
  });
});

describe('chat access: other states of the note', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    forgetUnconfirmedRevocations();
  });

  it('says a grant that ran out expired, and offers Grant again', async () => {
    setup({ grants: () => [grantRow({ expires_at: Math.floor(Date.now() / 1000) - 60 })] });
    await waitFor(() => expect(note()).toHaveTextContent(accessCopy.noteExpired));
    const paneNode = await openPaneFromNote(accessCopy.noteGrantAgain);
    expect(paneNode).toHaveTextContent(accessCopy.paneExpired('Plot review'));
    expect(within(paneNode).getByRole('button', { name: accessCopy.allow })).toBeInTheDocument();
  });

  it('names the other channel when the chat already posts elsewhere', async () => {
    setup({
      grants: () => [grantRow({ channel_id: 'channel-2', source_channels: ['channel-2'] })],
    });
    await waitFor(() =>
      expect(note()).toHaveTextContent(accessCopy.noteActiveElsewhere('#methods'))
    );
    const paneNode = await openPaneFromNote(accessCopy.noteManage);
    expect(paneNode).toHaveTextContent('Posts in #methods as Alice Chen (@alice)');
    expect(within(paneNode).queryByRole('button', { name: accessCopy.allow })).toBeNull();
  });

  it('shows a failed list with Retry instead of guessing there is no grant', async () => {
    let fail = true;
    setup({
      grants: () => [],
      listFailure: () => (fail ? new CrewHttpError('boom', 500, 'crew_internal') : null),
    });
    await waitFor(() => expect(note()).toHaveTextContent(accessCopy.listFailed));
    expect(within(note()).queryByRole('button', { name: accessCopy.noteReviewName })).toBeNull();
    fail = false;
    fireEvent.click(within(note()).getByRole('button', { name: accessCopy.listRetryName }));
    await waitFor(() => expect(note()).toHaveTextContent(accessCopy.noteNone('#general')));
  });

  it('renders nothing without ?sessionId= and never asks for grants', async () => {
    installDaemon(mocks, { grants: () => [grantRow()] });
    renderWithController(Layout, '/crew');
    await screen.findByTestId('channel-ready');
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.queryByTestId('crew-chat-connect-note')).toBeNull();
    expect(callsTo(mocks, GRANTS_PATH, 'GET')).toHaveLength(0);
  });
});
