import {
  act,
  createEvent,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from '@testing-library/react';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { useState } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Session } from '../../../api';
import { announceSessionBinding } from '../../../utils/sessionBindingSync';
import { announceSessionName, cacheSet } from '../../../utils/sessionNameSync';
import { agentCopy } from '../pane/copy';
import { CrewHttpError } from '../crewApi';
import CrewView from '../CrewView';
import { paneCopy } from '../pane/copy';
import { DetailsPane } from '../pane/DetailsPane';
import { useCrew } from '../state/CrewControllerContext';
import { ChatAccessPane } from './ChatAccessPane';
import { RevokeResultNote } from './RevokeControls';
import {
  ChatConnectNote,
  chatAccessRoute,
  chatAccessRouteState,
  forgetChatAccessIntents,
} from './ChatConnectNote';
import { accessCopy } from './copy';
import {
  callsTo,
  grantRow,
  installDaemon,
  renderWithController,
  type DaemonFixture,
} from './testing';
import {
  announceGrantsChanged,
  forgetUnconfirmedRevocations,
  UNCONFIRMED_REVOKE_WATCH_MS,
} from './useCrewGrants';

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
/**
 * The app's configured providers, as the pane reads them to name and check the chat's model. Stable
 * functions: a new one per render would refetch on every render. None configured by default, as a
 * pane with nothing to check a model against.
 */
const config = vi.hoisted(() => {
  const state = { providers: [] as unknown[] };
  return {
    state,
    value: {
      getProviders: async () => state.providers,
      read: async () => '',
      getProviderModels: async () => [],
    },
  };
});
vi.mock('../../ConfigContext', () => ({
  useConfig: () => config.value,
  usePrivacyTiersEnabled: () => true,
}));

const REVOKE_PATH = '/connections/conn-1/sessions/agent-1/revoke';
const GRANT_PATH = '/connections/conn-1/sessions/agent-1/grant';
const GRANTS_PATH = '/connections/conn-1/grants';

/**
 * The composer's note slot and the pane, as the layout places them: the note only once a channel is
 * selected, as `useComposerNote` does. "Toggle note" unmounts and remounts it, as a re-verification
 * does in the app.
 */
function Layout() {
  const { ui, channel, closePane } = useCrew();
  const [showNote, setShowNote] = useState(true);
  return (
    <div>
      {channel ? <p data-testid="channel-ready">{channel.name}</p> : null}
      <button type="button" onClick={() => setShowNote((shown) => !shown)}>
        Toggle note
      </button>
      {channel && showNote ? <ChatConnectNote /> : null}
      {ui.pane?.mode === 'chat-access' ? (
        <aside data-testid="pane">
          <button type="button" onClick={closePane}>
            Close pane
          </button>
          <ChatAccessPane />
        </aside>
      ) : null}
    </div>
  );
}

/** A chat this window already knows by name, as the chat store's recent-sessions cache holds it. */
function rememberChat(id: string, name: string) {
  cacheSet(id, { session: { id, name } as unknown as Session, messages: [] });
}

function setup(fixture: Partial<DaemonFixture> & { grants?: () => unknown[] }) {
  installDaemon(mocks, { grants: () => [], ...fixture });
  renderWithController(Layout);
}

const note = () => screen.getByTestId('crew-chat-connect-note');
const pane = () => screen.getByTestId('pane');

/**
 * Open this chat's pane from the note's button — unless `/crew` already opened it (Q3-28), in which
 * case the note is not drawn at all (Q4-14) and the open pane is the answer.
 */
async function openPaneFromNote(name: string | RegExp) {
  await screen.findByTestId('channel-ready');
  await waitFor(() => {
    if (screen.queryByTestId('pane')) return;
    within(note()).getByRole('button', { name });
  });
  if (!screen.queryByTestId('pane')) fireEvent.click(within(note()).getByRole('button', { name }));
  await waitFor(() => expect(pane()).not.toHaveTextContent(accessCopy.noteChecking));
  return pane();
}

describe('chat access: grant', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    forgetUnconfirmedRevocations();
    forgetChatAccessIntents();
  });

  it('keeps the Allow flow when the chat has no grant, and does not navigate after Allow', async () => {
    const grants: unknown[] = [];
    setup({ grants: () => grants });

    // Q3-28: `/crew` from a chat with no grant opens the consent by itself.
    await waitFor(() => expect(pane()).toHaveTextContent('This chat will be able to'));
    const paneNode = pane();
    // Q4-14: and the note does not ask the same question under the timeline meanwhile.
    expect(screen.queryByTestId('crew-chat-connect-note')).toBeNull();

    expect(paneNode).toHaveTextContent('This chat will be able to');
    expect(paneNode).toHaveTextContent('Read #general');
    expect(paneNode).toHaveTextContent("Post in #general as Alice Chen's agent (@alice)");
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
    // UXN-7: Allow took its own button away; focus goes to the granted view's first action.
    await waitFor(() =>
      expect(within(pane()).getByRole('button', { name: accessCopy.backToChat })).toHaveFocus()
    );
    expect(
      within(pane()).getByRole('button', { name: accessCopy.revokeButton })
    ).toBeInTheDocument();
    expect(within(pane()).queryByRole('button', { name: accessCopy.allow })).toBeNull();
    // L10: the pane says where Revoke lives instead of leaving on its own.
    expect(mocks.navigate).not.toHaveBeenCalled();
    // Once the new grant is listed, the badge reads as it will when the pane is reopened.
    await waitFor(() => expect(within(pane()).getByText(/^Active · ends \S/)).toBeInTheDocument());
    expect(screen.queryByTestId('crew-chat-connect-note')).toBeNull();

    fireEvent.click(within(pane()).getByRole('button', { name: accessCopy.backToChat }));
    expect(mocks.navigate).toHaveBeenCalledWith('/pair?resumeSessionId=agent-1');

    // The note follows the new grant without a poll, once the pane no longer says it.
    fireEvent.click(screen.getByRole('button', { name: 'Close pane' }));
    await waitFor(() =>
      expect(note()).toHaveTextContent(accessCopy.noteActive('Plot review', '#general'))
    );
  });

  /**
   * HPC-N2: the consent listed Read and Post while the chat, with a private model, wrote files and
   * ran commands in the connection's remote work folder.
   */
  it.each([
    [true, accessCopy.folderRun('/home/crew_erin/crew-work')],
    [false, accessCopy.folderFiles('/home/crew_erin/crew-work')],
  ])('lists the remote work folder the chat reaches (commands: %s)', async (run, line) => {
    setup({
      grants: () => [],
      connection: { remote_root: '/home/crew_erin/crew-work', remote_execution: run },
    });
    await waitFor(() => expect(pane()).toHaveTextContent('This chat will be able to'));
    expect(pane()).toHaveTextContent(line);
  });

  it('lists no folder for a connection that has none', async () => {
    setup({ grants: () => [] });
    await waitFor(() => expect(pane()).toHaveTextContent('This chat will be able to'));
    expect(pane().querySelector('[data-crew-access-folder]')).toBeNull();
  });

  it('sends the channels chosen under Advanced as extra context', async () => {
    setup({});
    const paneNode = await openPaneFromNote(accessCopy.noteReviewName);
    expect(within(paneNode).queryByRole('checkbox')).toBeNull();
    // Q3-30: closed, Advanced says what the chat reads, not what it doesn't.
    expect(paneNode).toHaveTextContent('Reads only #general');
    expect(paneNode).not.toHaveTextContent(/nothing else/);

    fireEvent.click(within(paneNode).getByRole('button', { name: 'Advanced' }));
    // AG-F17: a channel no other team shares a name with is named as Ask my agent names it, without
    // its team; "Lab / #methods" on every row said the team where it told nothing apart.
    const methods = await within(paneNode).findByRole('checkbox', { name: '#methods' });
    expect(within(paneNode).queryByRole('checkbox', { name: /\// })).toBeNull();
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
    forgetChatAccessIntents();
  });

  it('shows Manage access and Revoke, not Allow', async () => {
    setup({ grants: () => [grantRow()] });
    await waitFor(() =>
      expect(note()).toHaveTextContent(accessCopy.noteActive('Plot review', '#general'))
    );
    // Q3-30: inside Crew, "This chat" does not say which one; the note names it.
    expect(note()).toHaveTextContent('“Plot review” can read and post in #general.');
    expect(within(note()).queryByRole('button', { name: accessCopy.noteReviewName })).toBeNull();

    const paneNode = await openPaneFromNote(accessCopy.noteManage);
    expect(paneNode).toHaveTextContent('“Plot review” can');
    expect(paneNode).toHaveTextContent('Reads #general');
    expect(paneNode).toHaveTextContent("Posts in #general as Alice Chen's agent (@alice)");
    // One badge wording for an active grant (T-55): never "Expires …" here and "Active" there.
    expect(within(paneNode).getByText(/^Active · ends \S/)).toBeInTheDocument();
    expect(paneNode).not.toHaveTextContent(/Expires/);
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
    // Q4-15: the question says the whole chat stops, not only its posts here.
    expect(confirm).toHaveTextContent(accessCopy.confirmStops);
    expect(confirm).toHaveTextContent('This chat will stop until you grant access again.');
    expect(callsTo(mocks, REVOKE_PATH, 'POST')).toHaveLength(0);
    fireEvent.click(within(confirm).getByRole('button', { name: accessCopy.confirmRevoke }));

    await waitFor(() => expect(mocks.crewHttp).toHaveBeenCalledWith(REVOKE_PATH, 'POST'));
    expect(await within(pane()).findByText(accessCopy.revoked('Plot review'))).toBeInTheDocument();
    expect(within(pane()).getByRole('button', { name: accessCopy.openChat })).toBeInTheDocument();
    // UXN-7: Revoke took its own button away; focus went to the page, now to what replaced it.
    await waitFor(() =>
      expect(within(pane()).getByRole('button', { name: accessCopy.openChat })).toHaveFocus()
    );
    expect(within(pane()).getByRole('button', { name: accessCopy.done })).toBeInTheDocument();
    await waitFor(() =>
      expect(callsTo(mocks, GRANTS_PATH, 'GET').length).toBeGreaterThan(listsBefore)
    );

    fireEvent.click(within(pane()).getByRole('button', { name: accessCopy.done }));
    await waitFor(() => expect(screen.queryByTestId('pane')).toBeNull());
    await waitFor(() => expect(note()).toHaveTextContent(accessCopy.noteRevoked('Plot review')));
    expect(
      within(note()).getByRole('button', { name: accessCopy.noteGrantAgain })
    ).toBeInTheDocument();

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

    // The connection is up, so the daemon is asking the workspace again by itself: the note
    // says so, and never tells a connected person to reconnect (F3).
    expect(await within(pane()).findByText(accessCopy.confirming)).toBeInTheDocument();
    expect(within(pane()).getByRole('alert')).toHaveTextContent(accessCopy.confirming);
    expect(within(pane()).queryByText(/reconnect/i)).toBeNull();
    expect(screen.queryByText(/Access revoked/)).toBeNull();
    // The list now shows the local stop; the pane keeps saying it is unconfirmed.
    await waitFor(() =>
      expect(within(pane()).getByText(accessCopy.status.unconfirmed)).toBeInTheDocument()
    );

    fireEvent.click(within(pane()).getByRole('button', { name: accessCopy.retry }));
    await waitFor(() => expect(attempts).toBe(2));
    expect(await within(pane()).findByText(accessCopy.confirming)).toBeInTheDocument();
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
    expect(await within(pane()).findByText(accessCopy.confirming)).toBeInTheDocument();
    expect(screen.queryByText(/Access revoked/)).toBeNull();
  });
});

describe('chat access: other states of the note', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    forgetUnconfirmedRevocations();
    forgetChatAccessIntents();
  });

  it('says a grant that ran out expired, and offers Grant again', async () => {
    setup({ grants: () => [grantRow({ expires_at: Math.floor(Date.now() / 1000) - 60 })] });
    await waitFor(() => expect(note()).toHaveTextContent(accessCopy.noteExpired('Plot review')));
    const paneNode = await openPaneFromNote(accessCopy.noteGrantAgain);
    expect(paneNode).toHaveTextContent('Crew access for “Plot review” expired.');
    expect(within(paneNode).getByRole('button', { name: accessCopy.allow })).toBeInTheDocument();
  });

  /**
   * Final polish, observation (b): a grant the workspace ended because Crew's settings changed
   * (D-1) read "expired" here, while the chat's bar and the CLI say the settings changed.
   */
  it('says Crew settings changed for a grant the workspace ended, never “expired”', async () => {
    setup({
      grants: () => [grantRow({ expired: true, revocation: 'ended_by_workspace' })],
    });
    await waitFor(() =>
      expect(note()).toHaveTextContent(accessCopy.noteSettingsChanged('Plot review'))
    );
    expect(note()).not.toHaveTextContent(/expired/i);
    const paneNode = await openPaneFromNote(accessCopy.noteGrantAgain);
    expect(paneNode).toHaveTextContent(
      'Crew settings changed since “Plot review” was given access.'
    );
    expect(paneNode).not.toHaveTextContent(/expired/i);
  });

  it('asks consent for this channel when granting again after a grant elsewhere was revoked', async () => {
    setup({
      grants: () => [
        grantRow({ channel_id: 'channel-2', source_channels: ['channel-2'], expired: true }),
      ],
    });
    await waitFor(() => expect(note()).toHaveTextContent(accessCopy.noteRevoked('Plot review')));
    const paneNode = await openPaneFromNote(accessCopy.noteGrantAgain);
    // The possessive sits inside the sentence, never after the closing quote (T-55).
    expect(paneNode).toHaveTextContent('Crew access for “Plot review” was revoked.');
    expect(paneNode).not.toHaveTextContent('”’s');
    expect(paneNode).toHaveTextContent('Read #general');
    expect(paneNode).toHaveTextContent("Post in #general as Alice Chen's agent (@alice)");
    expect(paneNode).not.toHaveTextContent('#methods');
  });

  it('names the other channel when the chat already posts elsewhere', async () => {
    setup({
      grants: () => [grantRow({ channel_id: 'channel-2', source_channels: ['channel-2'] })],
    });
    await waitFor(() =>
      expect(note()).toHaveTextContent(accessCopy.noteActiveElsewhere('Plot review', '#methods'))
    );
    expect(note()).toHaveTextContent('“Plot review” already uses #methods.');
    const paneNode = await openPaneFromNote(accessCopy.noteManage);
    expect(paneNode).toHaveTextContent("Posts in #methods as Alice Chen's agent (@alice)");
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
    await waitFor(() => expect(note()).toHaveTextContent(accessCopy.noteNone(null, '#general')));
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

/**
 * T-55 (live QA round 1): the consent never named the chat until after Allow, because only a
 * listed grant carries `session_name`. It now names the chat this window already knows — from the
 * chat store's cache, never a fetch — on the heading and on Allow itself.
 */
describe('chat access: the consent names the chat before Allow', () => {
  const NAMED = 'chat-named';
  const NAMED_GRANT = `/connections/conn-1/sessions/${NAMED}/grant`;

  beforeEach(() => {
    vi.clearAllMocks();
    forgetUnconfirmedRevocations();
    forgetChatAccessIntents();
  });

  it('names a chat this window knows in the sentence above Allow, never on the button', async () => {
    rememberChat(NAMED, 'Greeting exchange');
    installDaemon(mocks, { grants: () => [] });
    renderWithController(Layout, `/crew?sessionId=${NAMED}`);

    const paneNode = await openPaneFromNote(accessCopy.noteReviewName);
    expect(paneNode).toHaveTextContent('“Greeting exchange” will be able to');
    expect(paneNode).not.toHaveTextContent('conversation');
    // UXN-10: one short word. Naming the chat there wrapped the button to two lines.
    const allow = within(paneNode).getByRole('button', { name: accessCopy.allow });
    expect(accessCopy.allow).toBe('Allow');
    expect(allow).toHaveTextContent(/^Allow$/);

    fireEvent.click(allow);
    await waitFor(() => expect(callsTo(mocks, NAMED_GRANT, 'POST')).toHaveLength(1));
    expect(await within(pane()).findByText(accessCopy.connected)).toBeInTheDocument();
    expect(pane()).toHaveTextContent('“Greeting exchange” can');
  });

  it('keeps “This chat” and the pinned Allow for a default or unknown title', async () => {
    rememberChat('chat-default', 'New chat');
    installDaemon(mocks, { grants: () => [] });
    renderWithController(Layout, '/crew?sessionId=chat-default');

    const paneNode = await openPaneFromNote(accessCopy.noteReviewName);
    expect(paneNode).toHaveTextContent('This chat will be able to');
    expect(paneNode).not.toHaveTextContent('“New chat”');
    expect(within(paneNode).getByRole('button', { name: accessCopy.allow })).toBeInTheDocument();
  });

  it('follows a rename of the chat while the consent is open', async () => {
    installDaemon(mocks, { grants: () => [] });
    renderWithController(Layout, '/crew?sessionId=chat-renamed');
    const paneNode = await openPaneFromNote(accessCopy.noteReviewName);
    expect(paneNode).toHaveTextContent('This chat will be able to');

    act(() =>
      announceSessionName({
        sessionId: 'chat-renamed',
        name: 'Plot review',
        userSetName: false,
        origin: 'llm',
      })
    );
    expect(await within(pane()).findByText('“Plot review” will be able to')).toBeInTheDocument();
    expect(within(pane()).getByRole('button', { name: accessCopy.allow })).toBeInTheDocument();
  });

  /**
   * UXN-10: with keyboard focus on it, the button's name changed from "Allow this conversation…"
   * to "Allow “Ready response request”…" when the chat's title arrived.
   */
  it('keeps the focused Allow button’s name when the chat’s title arrives', async () => {
    installDaemon(mocks, { grants: () => [] });
    renderWithController(Layout, '/crew?sessionId=chat-renamed');
    const paneNode = await openPaneFromNote(accessCopy.noteReviewName);
    const allow = within(paneNode).getByRole('button', { name: accessCopy.allow });
    act(() => allow.focus());
    act(() =>
      announceSessionName({
        sessionId: 'chat-renamed',
        name: 'Ready response request',
        userSetName: false,
        origin: 'llm',
      })
    );
    expect(
      await within(pane()).findByText('“Ready response request” will be able to')
    ).toBeInTheDocument();
    expect(within(pane()).getByRole('button', { name: accessCopy.allow })).toBe(allow);
    expect(allow).toHaveFocus();
  });
});

/**
 * T-55: re-granting took three screens — the chat's "Grant access again" landed on the Crew note,
 * whose own button opened the pane. The chat's controls now carry an intent the note honours once,
 * as soon as it knows the grant state, so the consent is one hop from the chat.
 */
describe('chat access: one hop from the ordinary chat', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    forgetUnconfirmedRevocations();
    forgetChatAccessIntents();
  });

  function arriveFromChat(sessionId = 'agent-1') {
    return render(
      <MemoryRouter
        initialEntries={[
          {
            pathname: '/crew',
            search: chatAccessRoute(sessionId).slice('/crew'.length),
            state: chatAccessRouteState(),
          },
        ]}
      >
        <CrewView layout={Layout} />
      </MemoryRouter>
    );
  }

  it('opens the consent for a revoked chat without a second click, and grants only on Allow', async () => {
    installDaemon(mocks, { grants: () => [grantRow({ expired: true })] });
    arriveFromChat();

    expect(
      await within(await screen.findByTestId('pane')).findByText(
        accessCopy.paneRevoked('Plot review')
      )
    ).toBeInTheDocument();
    const allow = within(pane()).getByRole('button', { name: accessCopy.allow });
    // Opening the pane granted nothing.
    expect(callsTo(mocks, GRANT_PATH, 'POST')).toHaveLength(0);

    fireEvent.click(allow);
    await waitFor(() => expect(callsTo(mocks, GRANT_PATH, 'POST')).toHaveLength(1));
  });

  it('opens Manage for an active chat, the chip’s one hop', async () => {
    installDaemon(mocks, { grants: () => [grantRow()] });
    arriveFromChat();
    const paneNode = await screen.findByTestId('pane');
    expect(await within(paneNode).findByText('“Plot review” can')).toBeInTheDocument();
    expect(
      within(paneNode).getByRole('button', { name: accessCopy.revokeButton })
    ).toBeInTheDocument();
  });

  it('opens once: closing the pane is final, even when the note remounts and reads again', async () => {
    installDaemon(mocks, { grants: () => [grantRow({ expired: true })] });
    arriveFromChat();
    await within(await screen.findByTestId('pane')).findByText(
      accessCopy.paneRevoked('Plot review')
    );

    fireEvent.click(screen.getByRole('button', { name: 'Close pane' }));
    await waitFor(() => expect(screen.queryByTestId('pane')).toBeNull());
    const listsBefore = callsTo(mocks, GRANTS_PATH, 'GET').length;

    // A fresh note on the same history entry reads the grants again and must not reopen the pane.
    fireEvent.click(screen.getByRole('button', { name: 'Toggle note' }));
    expect(screen.queryByTestId('crew-chat-connect-note')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Toggle note' }));
    await waitFor(() => expect(note()).toHaveTextContent(accessCopy.noteRevoked('Plot review')));
    expect(callsTo(mocks, GRANTS_PATH, 'GET').length).toBeGreaterThan(listsBefore);
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.queryByTestId('pane')).toBeNull();
  });

  it('waits for a failed list to be read, and opens once the note knows what to offer', async () => {
    let fail = true;
    installDaemon(mocks, {
      grants: () => [grantRow({ expired: true })],
      listFailure: () => (fail ? new CrewHttpError('boom', 500, 'crew_internal') : null),
    });
    arriveFromChat();
    await waitFor(() => expect(note()).toHaveTextContent(accessCopy.listFailed));
    expect(screen.queryByTestId('pane')).toBeNull();

    fail = false;
    fireEvent.click(within(note()).getByRole('button', { name: accessCopy.listRetryName }));
    expect(
      await within(await screen.findByTestId('pane')).findByText(
        accessCopy.paneRevoked('Plot review')
      )
    ).toBeInTheDocument();
  });

  it('does nothing without the intent for a chat that has a grant: /crew waits for the note’s button', async () => {
    installDaemon(mocks, { grants: () => [grantRow({ expired: true })] });
    renderWithController(Layout);
    await waitFor(() => expect(note()).toHaveTextContent(accessCopy.noteRevoked('Plot review')));
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.queryByTestId('pane')).toBeNull();
  });

  it('leaves an active chat’s /crew arrival on the note, and never opens it later', async () => {
    let revoked = false;
    installDaemon(mocks, { grants: () => [grantRow({ expired: revoked })] });
    // A revoke made on another surface (the Access tab, the chat's own bar): the list changes and
    // the change is announced.
    const revokeFromElsewhere = async () => {
      revoked = true;
      announceGrantsChanged({ connectionId: 'conn-1', sessionId: 'agent-1', change: 'revoked' });
    };
    renderWithController(Layout);
    await waitFor(() =>
      expect(note()).toHaveTextContent(accessCopy.noteActive('Plot review', '#general'))
    );
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.queryByTestId('pane')).toBeNull();
    // The arrival was answered with the note: a later revoke elsewhere does not turn it into an
    // open consent.
    await act(async () => {
      await revokeFromElsewhere();
    });
    await waitFor(() => expect(note()).toHaveTextContent(accessCopy.noteRevoked('Plot review')));
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.queryByTestId('pane')).toBeNull();
  });
});

/**
 * Q3-28 (live QA round 3): typing `/crew` in a chat with no grant stopped at "Connect this chat to
 * #general? [Review access]" — Bob: "I already typed /crew; why am I being asked if I want to
 * connect?" The arrival opens the consent at once. Allow is still the consent: nothing is granted
 * before it.
 */
describe('chat access: /crew from a chat with no grant', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    forgetUnconfirmedRevocations();
    forgetChatAccessIntents();
  });

  it('opens the consent on the channel Crew shows, and grants only on Allow', async () => {
    rememberChat('chat-new', 'Greeting exchange');
    installDaemon(mocks, { grants: () => [] });
    renderWithController(Layout, '/crew?sessionId=chat-new');

    const paneNode = await screen.findByTestId('pane');
    expect(await within(paneNode).findByText('“Greeting exchange” will be able to')).toBeVisible();
    expect(paneNode).toHaveTextContent('Read #general');
    expect(screen.getByTestId('channel-ready')).toHaveTextContent('general');
    // Q4-14: one prompt for one decision. The note's "Connect “Greeting exchange” to #general?
    // [Review access]" is not drawn under the timeline while the pane asks the same question.
    expect(screen.queryByTestId('crew-chat-connect-note')).toBeNull();
    expect(screen.queryByText('Connect “Greeting exchange” to #general?')).toBeNull();
    expect(callsTo(mocks, '/connections/conn-1/sessions/chat-new/grant', 'POST')).toHaveLength(0);

    fireEvent.click(within(paneNode).getByRole('button', { name: accessCopy.allow }));
    await waitFor(() =>
      expect(callsTo(mocks, '/connections/conn-1/sessions/chat-new/grant', 'POST')).toHaveLength(1)
    );
  });

  it('opens once: closing the consent is final for that arrival', async () => {
    installDaemon(mocks, { grants: () => [] });
    renderWithController(Layout, '/crew?sessionId=chat-fresh');
    await screen.findByTestId('pane');
    fireEvent.click(screen.getByRole('button', { name: 'Close pane' }));
    await waitFor(() => expect(screen.queryByTestId('pane')).toBeNull());

    fireEvent.click(screen.getByRole('button', { name: 'Toggle note' }));
    fireEvent.click(screen.getByRole('button', { name: 'Toggle note' }));
    await waitFor(() => expect(note()).toHaveTextContent(accessCopy.noteNone(null, '#general')));
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.queryByTestId('pane')).toBeNull();
  });
});

/**
 * Q2-06 (live QA round 2): the Allow button overflowed the 328px pane to the left, and its label
 * read "w “Lab channel greeting” to read and post in #general". UXN-10 took the chat and channel
 * off the button altogether: its label is the one pinned word, so it neither wraps nor overflows,
 * nor changes its name under focus. Pinned at the source as well as on the rendered button.
 */
describe('chat access: the Allow button stays inside the pane', () => {
  it('labels Allow with the one pinned word in the source', () => {
    const source = readFileSync(join(__dirname, 'ChatAccessPane.tsx'), 'utf8');
    const allow = /<Button\s+key="crew-chat-access-allow"[\s\S]*?>\s*([^<]*?)\s*<\/Button>/.exec(
      source
    );
    expect(allow, 'the Allow button').not.toBeNull();
    expect(allow?.[1]).toBe('{accessCopy.allow}');
  });
});

/**
 * Q2-10 (live QA round 2): the chip on a chat connected to #methods, and "Grant access again" on
 * one whose access to #methods was revoked, opened Crew on #general — and the pane did not open.
 * The one hop now lands on the grant's own channel with the pane open.
 */
describe('chat access: one hop lands on the grant’s channel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    forgetUnconfirmedRevocations();
    forgetChatAccessIntents();
  });

  const onMethods = (extra: Record<string, unknown> = {}) =>
    grantRow({ channel_id: 'channel-2', source_channels: ['channel-2'], ...extra });

  function arrive(entry = chatAccessRoute('agent-1')) {
    return render(
      <MemoryRouter
        initialEntries={[
          {
            pathname: '/crew',
            search: entry.slice('/crew'.length),
            state: chatAccessRouteState(),
          },
        ]}
      >
        <CrewView layout={Layout} />
      </MemoryRouter>
    );
  }

  it('opens #methods with Manage for the chip of a chat connected there', async () => {
    installDaemon(mocks, { grants: () => [onMethods()] });
    arrive();
    const paneNode = await screen.findByTestId('pane');
    expect(await within(paneNode).findByText('“Plot review” can')).toBeInTheDocument();
    await waitFor(() => expect(screen.getByTestId('channel-ready')).toHaveTextContent('methods'));
    expect(paneNode).toHaveTextContent("Posts in #methods as Alice Chen's agent (@alice)");
    // The pane says it (Q4-14): the note is not drawn beside it.
    expect(screen.queryByTestId('crew-chat-connect-note')).toBeNull();
    // Settled: the pane stays open.
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByTestId('pane')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Close pane' }));
    await waitFor(() =>
      expect(note()).toHaveTextContent(accessCopy.noteActive('Plot review', '#methods'))
    );
  });

  it('reaches Allow for #methods in one hop from Grant access again', async () => {
    installDaemon(mocks, { grants: () => [onMethods({ expired: true })] });
    arrive();
    const allow = await within(await screen.findByTestId('pane')).findByRole('button', {
      name: accessCopy.allow,
    });
    expect(screen.getByTestId('channel-ready')).toHaveTextContent('methods');
    expect(callsTo(mocks, GRANT_PATH, 'POST')).toHaveLength(0);

    fireEvent.click(allow);
    await waitFor(() => expect(callsTo(mocks, GRANT_PATH, 'POST')).toHaveLength(1));
    expect(callsTo(mocks, GRANT_PATH, 'POST')[0][2]).toMatchObject({ channel_id: 'channel-2' });
  });

  it('makes closing the pane after the one hop final, and stays on the grant’s channel', async () => {
    installDaemon(mocks, { grants: () => [onMethods()] });
    arrive();
    await within(await screen.findByTestId('pane')).findByText('“Plot review” can');
    fireEvent.click(screen.getByRole('button', { name: 'Close pane' }));
    await waitFor(() => expect(screen.queryByTestId('pane')).toBeNull());
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByTestId('channel-ready')).toHaveTextContent('methods');
    expect(screen.queryByTestId('pane')).toBeNull();
  });

  it('stays on the channel shown when the grant’s channel is archived', async () => {
    installDaemon(mocks, {
      grants: () => [grantRow({ channel_id: 'channel-3', source_channels: ['channel-3'] })],
    });
    arrive();
    await within(await screen.findByTestId('pane')).findByText('“Plot review” can');
    expect(screen.getByTestId('channel-ready')).toHaveTextContent('general');
  });
});

/**
 * Q2-09 (live QA round 2): a task's grant ends with the task. Crew's note and pane say the task
 * is finished, and offer no re-grant: a task is not connected again from its chat.
 */
describe('chat access: a finished task', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    forgetUnconfirmedRevocations();
    forgetChatAccessIntents();
  });

  it('says the task is finished in the note, with nothing to press', async () => {
    installDaemon(mocks, { grants: () => [grantRow({ kind: 'task', expired: true })] });
    renderWithController(Layout);
    await waitFor(() => expect(note()).toHaveTextContent(accessCopy.noteTaskFinished));
    expect(within(note()).queryByRole('button')).toBeNull();
    expect(note()).not.toHaveTextContent(/revoked/i);
  });

  it('says so in the pane too, and offers no Allow', async () => {
    installDaemon(mocks, { grants: () => [grantRow({ kind: 'task', expired: true })] });
    function PaneOnly() {
      const { channel, openPane, ui } = useCrew();
      return (
        <div>
          {channel ? (
            <button
              type="button"
              onClick={() => openPane({ mode: 'chat-access', sessionId: 'agent-1' })}
            >
              Open access
            </button>
          ) : null}
          {ui.pane?.mode === 'chat-access' ? (
            <aside data-testid="pane">
              <ChatAccessPane />
            </aside>
          ) : null}
        </div>
      );
    }
    renderWithController(PaneOnly);
    fireEvent.click(await screen.findByRole('button', { name: 'Open access' }));
    const paneNode = await screen.findByTestId('pane');
    expect(await within(paneNode).findByText(accessCopy.paneTaskFinished)).toBeInTheDocument();
    expect(within(paneNode).queryByRole('button', { name: /^Allow/ })).toBeNull();
    expect(within(paneNode).getByRole('button', { name: accessCopy.openChat })).toBeInTheDocument();
  });
});

/**
 * Q4-13 (live QA round 4): `/crew` + Enter opened the pane with focus on its heading, and the
 * browser drew its default `outline: auto` box round it — Jack and Bob read it as a text field. The
 * consent's one action, Allow, takes focus instead: only from that heading (or from nowhere), so
 * focus the person moved elsewhere stays where they put it.
 */
describe('chat access: focus when the consent opens', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    forgetUnconfirmedRevocations();
    forgetChatAccessIntents();
  });

  /** The real details pane, which focuses its heading when it opens, with the note beside it. */
  function PaneLayout() {
    const { channel, grantSessionId, openPane } = useCrew();
    return (
      <div>
        {channel ? <ChatConnectNote /> : null}
        {channel ? (
          <button
            type="button"
            onClick={() => openPane({ mode: 'chat-access', sessionId: grantSessionId ?? '' })}
          >
            Open access
          </button>
        ) : null}
        <DetailsPane agent={<div />} chatAccess={<ChatAccessPane />} />
      </div>
    );
  }

  const heading = () => document.querySelector<HTMLElement>('aside.crew-pane h2') ?? document.body;

  /** A grant list that answers only when the test says so. */
  function heldGrants() {
    let release: () => void = () => {};
    const ready = new Promise<void>((resolve) => {
      release = resolve;
    });
    installDaemon(mocks, { grants: () => [] });
    const route = mocks.crewHttp.getMockImplementation();
    mocks.crewHttp.mockImplementation(async (path: string, method = 'GET', body?: unknown) => {
      if (path === GRANTS_PATH && method === 'GET') {
        await ready;
        return { grants: [] };
      }
      return route?.(path, method, body);
    });
    return () => act(async () => release());
  }

  it('puts focus on Allow, not on the heading, when /crew opens the consent', async () => {
    rememberChat('chat-new', 'Greeting exchange');
    installDaemon(mocks, { grants: () => [] });
    renderWithController(PaneLayout, '/crew?sessionId=chat-new');

    const allow = await screen.findByRole('button', { name: accessCopy.allow });
    await waitFor(() => expect(allow).toHaveFocus());
    expect(heading()).not.toHaveFocus();
    expect(heading()).toHaveTextContent(paneCopy.chatAccessTitle);
    // Q4-14 in the real layout: no strip asks the same question while the pane does.
    expect(screen.queryByTestId('crew-chat-connect-note')).toBeNull();
    // Focus is not a consent.
    expect(callsTo(mocks, '/connections/conn-1/sessions/chat-new/grant', 'POST')).toHaveLength(0);
  });

  it('moves focus from the heading once a slow grant list lets the consent appear', async () => {
    const release = heldGrants();
    renderWithController(PaneLayout, '/crew?sessionId=chat-slow');
    fireEvent.click(await screen.findByRole('button', { name: 'Open access' }));
    await waitFor(() => expect(heading()).toHaveFocus());
    expect(screen.getByTestId('crew-chat-access-pane')).toHaveTextContent(accessCopy.noteChecking);

    await release();
    const allow = await screen.findByRole('button', { name: accessCopy.allow });
    await waitFor(() => expect(allow).toHaveFocus());
  });

  it('leaves focus where the person moved it before the consent appeared', async () => {
    const release = heldGrants();
    renderWithController(PaneLayout, '/crew?sessionId=chat-slow');
    fireEvent.click(await screen.findByRole('button', { name: 'Open access' }));
    await waitFor(() => expect(heading()).toHaveFocus());
    const close = screen.getByRole('button', { name: paneCopy.closeChatAccess });
    act(() => close.focus());

    await release();
    const allow = await screen.findByRole('button', { name: accessCopy.allow });
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20));
    });
    expect(close).toHaveFocus();
    expect(allow).not.toHaveFocus();
  });

  it('never presses Allow with the repeats of a held Enter', async () => {
    installDaemon(mocks, { grants: () => [] });
    renderWithController(PaneLayout, '/crew?sessionId=chat-held');
    const allow = await screen.findByRole('button', { name: accessCopy.allow });

    const held = createEvent.keyDown(allow, { key: 'Enter', repeat: true });
    fireEvent(allow, held);
    expect(held.defaultPrevented).toBe(true);
    const deliberate = createEvent.keyDown(allow, { key: 'Enter' });
    fireEvent(allow, deliberate);
    expect(deliberate.defaultPrevented).toBe(false);
  });
});

/**
 * Q4-14 (live QA round 4): "Connect '{chat}' to #general? [Review access]" stayed under the
 * timeline while the consent for the same decision was open: two prompts for one decision.
 */
describe('chat access: the note while the pane is open', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    forgetUnconfirmedRevocations();
    forgetChatAccessIntents();
  });

  function OtherChatLayout() {
    const { channel, openPane, ui } = useCrew();
    return (
      <div>
        {channel ? <ChatConnectNote /> : null}
        {channel ? (
          <button
            type="button"
            onClick={() => openPane({ mode: 'chat-access', sessionId: 'another-chat' })}
          >
            Open another chat
          </button>
        ) : null}
        {ui.pane?.mode === 'chat-access' ? (
          <aside data-testid="pane">
            <ChatAccessPane />
          </aside>
        ) : null}
      </div>
    );
  }

  it('keeps this chat’s note while the pane shows another chat', async () => {
    installDaemon(mocks, { grants: () => [grantRow({ expired: true })] });
    renderWithController(OtherChatLayout);
    await waitFor(() => expect(note()).toHaveTextContent(accessCopy.noteRevoked('Plot review')));
    fireEvent.click(screen.getByRole('button', { name: 'Open another chat' }));
    await screen.findByTestId('pane');
    expect(note()).toHaveTextContent(accessCopy.noteRevoked('Plot review'));
  });

  it('hides the note while its own pane is open, and brings it back when the pane closes', async () => {
    installDaemon(mocks, { grants: () => [grantRow({ expired: true })] });
    renderWithController(Layout);
    await waitFor(() => expect(note()).toHaveTextContent(accessCopy.noteRevoked('Plot review')));
    fireEvent.click(within(note()).getByRole('button', { name: accessCopy.noteGrantAgain }));
    expect(await within(pane()).findByText(accessCopy.paneRevoked('Plot review'))).toBeVisible();
    expect(screen.queryByTestId('crew-chat-connect-note')).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: 'Close pane' }));
    await waitFor(() => expect(note()).toHaveTextContent(accessCopy.noteRevoked('Plot review')));
  });
});

/**
 * Final acceptance F3: a revoke that stopped only on this device is confirmed by the daemon itself
 * once the connection is back. The pane says so while it waits — "Confirming with the workspace…",
 * never "Reconnect" to a connected person — and says "Confirmed" when the daemon's list does.
 */
describe('chat access: a revoke waiting for the workspace (F3)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    forgetUnconfirmedRevocations();
    forgetChatAccessIntents();
  });

  it('follows the daemon from “Confirming with the workspace…” to “Confirmed”, with no click', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      let standing = 'unconfirmed';
      setup({ grants: () => [grantRow({ expired: true, revocation: standing })] });
      const paneNode = await openPaneFromNote(accessCopy.noteGrantAgain);
      expect(await within(paneNode).findByText(accessCopy.confirming)).toBeInTheDocument();
      expect(within(paneNode).queryByText(/reconnect/i)).toBeNull();
      expect(within(paneNode).getByText(accessCopy.status.unconfirmed)).toBeInTheDocument();

      standing = 'confirmed';
      await act(async () => {
        await vi.advanceTimersByTimeAsync(UNCONFIRMED_REVOKE_WATCH_MS);
      });
      expect(await within(pane()).findByText(accessCopy.confirmed)).toBeInTheDocument();
      expect(within(pane()).queryByText(accessCopy.confirming)).toBeNull();
      expect(within(pane()).queryByText(accessCopy.status.unconfirmed)).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });
});

/**
 * AG-F1, SF-F5: the consent named no model and no workspace, said nothing of the access binding
 * the chat for good, and offered Allow for a model the workspace refuses — the refusal came only
 * after the click, in the daemon's words, and stayed over #general once the pane closed.
 */
describe('chat access: the chat’s model and workspace, before Allow', () => {
  const versa = {
    name: 'versa_azure',
    is_configured: true,
    resolved_tier: 'private',
    affiliation: { kind: 'institutions', institutions: [{ id: 'ucsf', display_name: 'UCSF' }] },
    metadata: { display_name: 'Versa', known_models: [{ name: 'gpt-5.5' }] },
  };
  const stanfordGateway = {
    name: 'stanford_gateway',
    is_configured: true,
    resolved_tier: 'private',
    affiliation: {
      kind: 'institutions',
      institutions: [{ id: 'stanford', display_name: 'Stanford' }],
    },
    metadata: { display_name: 'Stanford AI', known_models: [{ name: 'gpt-5.5' }] },
  };
  const openRouter = {
    name: 'openrouter',
    is_configured: true,
    resolved_tier: 'public',
    metadata: { display_name: 'OpenRouter', known_models: [{ name: 'free-model' }] },
  };

  /**
   * The chat, as the chat store's cache holds it once the chat has loaded in this window: untitled,
   * so Allow keeps its pinned name, and on `provider`'s `model` (none when omitted).
   */
  function chatOn(provider?: string, model?: string) {
    cacheSet('agent-1', {
      session: {
        id: 'agent-1',
        name: 'New chat',
        ...(provider && model
          ? { provider_name: provider, model_config: { model_name: model, toolshim: false } }
          : {}),
      } as unknown as Session,
      messages: [],
    });
  }

  const allow = () => within(pane()).getByRole('button', { name: accessCopy.allow });

  beforeEach(() => {
    vi.clearAllMocks();
    forgetUnconfirmedRevocations();
    forgetChatAccessIntents();
    config.state.providers = [];
    chatOn();
  });

  it('names the workspace, the chat’s model and its tier, whose agent posts, and what the access fixes', async () => {
    config.state.providers = [versa];
    chatOn('versa_azure', 'gpt-5.5');
    setup({});
    const paneNode = await openPaneFromNote(accessCopy.noteReviewName);
    expect(paneNode).toHaveTextContent('This chat will be able to');
    expect(
      within(paneNode)
        .getAllByRole('term')
        .map((term) => term.textContent)
    ).toEqual([accessCopy.consentWorkspace, accessCopy.consentModel]);
    expect(paneNode).toHaveTextContent(/Workspace\s*lab/);
    await waitFor(() => expect(pane()).toHaveTextContent('gpt-5.5 · Versa'));
    expect(pane().querySelector('[data-crew-model-tier="private"]')).not.toBeNull();
    expect(pane()).toHaveTextContent(
      'The first access fixes this chat’s workspace, channel and model.'
    );
    // Posts appear as the agent's, never as the person's own, named as the timeline names it
    // (UXN-10): "Alice Chen's agent @alice" drawn, never the possessive on the username.
    expect(pane()).toHaveTextContent("Post in #general as Alice Chen's agent (@alice)");
    expect(allow()).toBeEnabled();
  });

  it('disables Allow, with the reason, for a model the workspace’s institution has not approved', async () => {
    config.state.providers = [stanfordGateway];
    chatOn('stanford_gateway', 'gpt-5.5');
    setup({});
    await openPaneFromNote(accessCopy.noteReviewName);
    const reason = agentCopy.institutionMismatch('gpt-5.5', 'Stanford', 'lab', 'ucsf');
    expect(await within(pane()).findByText(reason)).toBeInTheDocument();
    expect(allow()).toBeDisabled();
    expect(allow()).toHaveAccessibleDescription(reason);
  });

  it('disables Allow, with the reason, for a public model in a Private workspace', async () => {
    config.state.providers = [openRouter];
    chatOn('openrouter', 'free-model');
    setup({});
    await openPaneFromNote(accessCopy.noteReviewName);
    const reason = agentCopy.publicWorkspace('lab');
    expect(await within(pane()).findByText(reason)).toBeInTheDocument();
    expect(allow()).toBeDisabled();
  });

  it('follows a model switched in the chat', async () => {
    config.state.providers = [versa, openRouter];
    chatOn('versa_azure', 'gpt-5.5');
    setup({});
    await openPaneFromNote(accessCopy.noteReviewName);
    await waitFor(() => expect(pane()).toHaveTextContent('gpt-5.5 · Versa'));
    expect(allow()).toBeEnabled();
    act(() =>
      announceSessionBinding({ sessionId: 'agent-1', provider: 'openrouter', model: 'free-model' })
    );
    expect(await within(pane()).findByText(agentCopy.publicWorkspace('lab'))).toBeInTheDocument();
    expect(allow()).toBeDisabled();
  });

  it('words the daemon’s institution refusal as Ask my agent does, never in its internal words', async () => {
    // The pane cannot tell who approved this model, so it lets Allow through to the daemon.
    config.state.providers = [];
    setup({
      grant: () => {
        throw new CrewHttpError(
          "Crew institution does not match the model's resolved affiliation; choose a local model or a model approved for this institution",
          400,
          'crew_request_refused'
        );
      },
    });
    const paneNode = await openPaneFromNote(accessCopy.noteReviewName);
    fireEvent.click(within(paneNode).getByRole('button', { name: accessCopy.allow }));
    expect(await within(paneNode).findByRole('alert')).toHaveTextContent(
      agentCopy.institutionRefused(agentCopy.model, 'ucsf')
    );
    expect(paneNode).not.toHaveTextContent(/resolved affiliation/);
  });

  it('shows a coded refusal’s own sentence from a daemon that words it', async () => {
    const sentence = 'This connection is for stanford, but lab belongs to ucsf.';
    setup({
      grant: () => {
        throw new CrewHttpError(sentence, 400, 'crew_institution_mismatch');
      },
    });
    const paneNode = await openPaneFromNote(accessCopy.noteReviewName);
    fireEvent.click(within(paneNode).getByRole('button', { name: accessCopy.allow }));
    expect(await within(paneNode).findByRole('alert')).toHaveTextContent(sentence);
  });
});

/** AG-F12: a Retry answered "stopped on this device" again changed nothing on screen. */
describe('RevokeResultNote after a Retry that still cannot reach the workspace', () => {
  it('says it checked just now, is announced again, and offers Connect', () => {
    const outcome = { kind: 'unconfirmed' as const, message: accessCopy.unconfirmed };
    const connect = vi.fn();
    const props = {
      chat: 'Plot review',
      onRetry: vi.fn(),
      confirmation: 'offline' as const,
      connectAction: (
        <button type="button" onClick={connect}>
          {accessCopy.connect}
        </button>
      ),
    };
    const view = render(<RevokeResultNote outcome={outcome} {...props} />);
    expect(screen.getByRole('alert')).toHaveTextContent(accessCopy.unconfirmed);
    expect(screen.queryByRole('button', { name: accessCopy.connect })).toBeNull();
    const first = screen.getByRole('alert');

    view.rerender(<RevokeResultNote outcome={outcome} {...props} retrying />);
    view.rerender(<RevokeResultNote outcome={{ ...outcome }} {...props} />);
    const again = screen.getByRole('alert');
    expect(again).toHaveTextContent(
      'Still can’t reach the workspace · checked just now. Connect to confirm it now.'
    );
    // A new alert, so it is announced again, not the old one with new words.
    expect(again).not.toBe(first);
    fireEvent.click(screen.getByRole('button', { name: accessCopy.connect }));
    expect(connect).toHaveBeenCalled();

    // Once the connection is back, the daemon confirms by itself: no "still can't reach".
    view.rerender(<RevokeResultNote outcome={outcome} {...props} confirmation="confirming" />);
    expect(screen.getByRole('alert')).toHaveTextContent(accessCopy.confirming);
  });
});
