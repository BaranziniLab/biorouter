import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { groupedFingerprint, workspaceKeyFingerprint } from '../dialogs/fingerprint';
import { forgetJoinContext, updateJoinContext } from '../onboarding/joinContext';
import { connectionBarCopy } from '../channel/copy';
import { crewStatusCopy } from '../state/copy';
import { sidebarCopy } from './copy';
import { offersReconnect, unavailableReason } from './WorkspaceMenu';
import { WorkspaceSwitcher } from './WorkspaceSwitcher';
import {
  bob,
  connection,
  makeController,
  makeSnapshot,
  renderWithCrew,
  secondConnection,
} from './sidebarTestUtils';

const copy = sidebarCopy.workspaceMenu;

afterEach(() => {
  vi.restoreAllMocks();
});

async function openMenu(name: RegExp = /^Fixture/) {
  const user = userEvent.setup();
  await user.click(screen.getByRole('button', { name }));
  const menu = await screen.findByRole('menu');
  return { user, menu };
}

async function choose(item: string | RegExp, name?: RegExp) {
  const { user, menu } = await openMenu(name);
  await user.click(within(menu).getByRole('menuitem', { name: item }));
}

describe('WorkspaceSwitcher', () => {
  it('is a real menu trigger: the workspace name, aria-haspopup="menu" and a chevron', () => {
    renderWithCrew(<WorkspaceSwitcher />);
    const trigger = screen.getByRole('button', { name: /^Fixture/ });
    expect(trigger).toHaveAttribute('aria-haspopup', 'menu');
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
    const chevron = trigger.querySelector('.crew-sidebar-chevron');
    expect(chevron).toHaveAttribute('data-turn', 'half');
    expect(chevron).toHaveAttribute('aria-hidden', 'true');
    // Never a drag region: the band control opts out.
    expect(trigger).toHaveClass('no-drag');
    expect(trigger.getAttribute('style') ?? '').not.toMatch(/app-region|padding/);
  });

  it('turns the chevron by the trigger’s open state', async () => {
    renderWithCrew(<WorkspaceSwitcher />);
    const trigger = screen.getByRole('button', { name: /^Fixture/ });
    expect(trigger).toHaveAttribute('data-state', 'closed');
    await openMenu();
    expect(trigger).toHaveAttribute('data-state', 'open');
    expect(trigger).toHaveAttribute('aria-expanded', 'true');
  });

  it('shows a truncated name in full in a tooltip to its RIGHT, clear of the status row (T-21, Q2-17)', async () => {
    const user = userEvent.setup();
    // jsdom lays nothing out: make the name need more room than it has.
    vi.spyOn(HTMLElement.prototype, 'scrollWidth', 'get').mockReturnValue(200);
    vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(80);
    renderWithCrew(<WorkspaceSwitcher />);
    const trigger = screen.getByRole('button', { name: /^Fixture/ });
    // No native title: the app's tooltip layer would open it down over the status text.
    expect(trigger.querySelector('.crew-sidebar-switcher-name')).not.toHaveAttribute('title');
    await user.hover(trigger);
    expect(await screen.findByRole('tooltip')).toHaveTextContent('Fixture');
    const content = document.querySelector('[data-crew-switcher-tooltip]') as HTMLElement;
    expect(content).toHaveAttribute('data-side', 'right');
  });

  it('offers no tooltip for a name that fits: it would only repeat itself', async () => {
    const user = userEvent.setup();
    renderWithCrew(<WorkspaceSwitcher />);
    await user.hover(screen.getByRole('button', { name: /^Fixture/ }));
    await new Promise((resolve) => setTimeout(resolve, 700));
    expect(screen.queryByRole('tooltip')).toBeNull();
  });

  it('names the workspace by its own name when the broker sends one', () => {
    const snapshot = makeSnapshot({
      workspace: {
        id: 'workspace-1',
        host_uid: 1000,
        mode: 'private',
        institution_id: 'ucsf',
        policy_epoch: 1,
        name: 'lab',
      },
    });
    renderWithCrew(<WorkspaceSwitcher />, makeController({ snapshot }));
    expect(screen.getByRole('button', { name: /^lab/ })).toBeInTheDocument();
  });

  it('shows the header facts and the verified status line only while open', async () => {
    renderWithCrew(<WorkspaceSwitcher />);
    expect(screen.queryByText(crewStatusCopy.verified)).toBeNull();
    const { menu } = await openMenu();
    const header = menu.querySelector('[data-crew-menu-header]') as HTMLElement;
    expect(header).toHaveTextContent('Fixture');
    expect(header).toHaveTextContent('Hosted by Alice Chen (@alice)');
    // The PERSON, then the server — never the SSH login or an alias alone (T-40).
    expect(header).toHaveTextContent('Signed in as @alice on hpc.ucsf.edu');
    expect(header).not.toHaveTextContent('alice@hpc.ucsf.edu');
    expect(within(header).getByText(crewStatusCopy.verified)).toBeInTheDocument();
  });

  it('names the account by its username even when the login is an SSH alias', async () => {
    const aliased = { ...connection, ssh_target: 'lab-server' };
    renderWithCrew(
      <WorkspaceSwitcher />,
      makeController({ connection: aliased, connections: [aliased] })
    );
    const { menu } = await openMenu();
    const line = menu.querySelector('[data-crew-signed-in]') as HTMLElement;
    expect(line).toHaveTextContent('Signed in as @alice on lab-server');
  });

  it('names the server by the person’s own alias for it when the daemon sends one (D-ALIAS)', async () => {
    const labelled = {
      ...connection,
      ssh_target: 'crew_alice@52.33.141.141',
      server_label: 'lab-server',
    };
    renderWithCrew(
      <WorkspaceSwitcher />,
      makeController({ connection: labelled, connections: [labelled] })
    );
    const { menu } = await openMenu();
    const line = menu.querySelector('[data-crew-signed-in]') as HTMLElement;
    expect(line).toHaveTextContent('Signed in as @alice on lab-server');
    // The raw address belongs to Connection settings' details, not this header.
    expect(menu).not.toHaveTextContent('52.33.141.141');
  });

  it('shows the workspace key’s fingerprint after "identity verified" (Q2-04)', async () => {
    const key = '9dacd3e46f083a8a5b76c22ba7c39d939c81538ed20c6ee33e46c9d92931cad3';
    const expected = groupedFingerprint((await workspaceKeyFingerprint(key)) ?? '');
    expect(expected).toMatch(/^[0-9A-F]{4} [0-9A-F]{4} [0-9A-F]{4} [0-9A-F]{4}$/);
    const keyed = { ...connection, workspace_public_key: key };
    renderWithCrew(
      <WorkspaceSwitcher />,
      makeController({ connection: keyed, connections: [keyed] })
    );
    const { menu } = await openMenu();
    const header = menu.querySelector('[data-crew-menu-header]') as HTMLElement;
    const line = await within(header).findByText(expected);
    expect(line.closest('[data-crew-menu-fingerprint]')).toHaveTextContent(
      `${copy.fingerprint} ${expected}`
    );
    // After the verified status line, in the header the menu is described by.
    const verified = within(header).getByText(crewStatusCopy.verified);
    expect(verified.compareDocumentPosition(line) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(menu).toHaveAccessibleDescription(expect.stringContaining(expected));
  });

  it('shows no fingerprint for a key it cannot read', async () => {
    renderWithCrew(<WorkspaceSwitcher />);
    const { menu } = await openMenu();
    expect(menu.querySelector('[data-crew-menu-fingerprint]')).toBeNull();
  });

  // A joiner took the fingerprint for the code to send (the Join dialog folds it away), and beside
  // "Your join code stays the same." it read as that code. It explains "identity verified" only.
  it.each([
    'not-joined',
    'checking',
    'connecting',
    'sign-in-needed',
    'cant-verify',
    'cant-connect',
    'offline',
  ] as const)('shows no fingerprint while the status is %s (Q2-04)', async (status) => {
    const key = '9dacd3e46f083a8a5b76c22ba7c39d939c81538ed20c6ee33e46c9d92931cad3';
    const expected = groupedFingerprint((await workspaceKeyFingerprint(key)) ?? '');
    const keyed = { ...connection, workspace_public_key: key };
    const overrides = { connection: keyed, connections: [keyed] };
    const view = renderWithCrew(
      <WorkspaceSwitcher />,
      makeController({
        ...overrides,
        status,
        // A reconnect over a verified view reads "Connecting…"; every other status has no
        // verified snapshot yet.
        ...(status === 'connecting' ? {} : { snapshot: null, observedPrivacy: null }),
      })
    );
    await openMenu();
    const header = () => screen.getByRole('menu').querySelector('[data-crew-menu-header]');
    // The status line has rendered, and the key's digest has had its turn.
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(header()).not.toHaveTextContent(copy.fingerprint);
    expect(header()).not.toHaveTextContent(expected);
    // The same key, once verified: the line appears, so its absence above was the status alone.
    view.update(makeController({ ...overrides, status: 'connected' }));
    expect(await within(header() as HTMLElement).findByText(expected)).toBeInTheDocument();
  });

  it('tells a joiner what Reconnect and Disconnect each do, and that the code survives both (Q2-43, Q3-47)', async () => {
    renderWithCrew(
      <WorkspaceSwitcher />,
      makeController({ snapshot: null, observedPrivacy: null, status: 'not-joined' })
    );
    const { menu } = await openMenu();
    const reconnect = within(menu).getByRole('menuitem', { name: copy.reconnect });
    const disconnect = within(menu).getByRole('menuitem', { name: copy.disconnect });
    // Two different helpers, where one sentence under both said nothing about the difference.
    expect(reconnect).toHaveAccessibleDescription(
      'Try the connection again. Your code doesn’t change.'
    );
    expect(disconnect).toHaveAccessibleDescription(
      'Stop waiting for now. Your host can still let you in with the same code.'
    );
    for (const item of [reconnect, disconnect]) expect(item).not.toHaveAttribute('aria-disabled');
  });

  it('names the host the invitation named in the Disconnect helper', async () => {
    updateJoinContext(connection.id, {
      joining: true,
      hostUsername: 'crew_henry',
      hostDisplayName: 'Henry Ito',
    });
    try {
      renderWithCrew(
        <WorkspaceSwitcher />,
        makeController({ snapshot: null, observedPrivacy: null, status: 'not-joined' })
      );
      const { menu } = await openMenu();
      expect(
        within(menu).getByRole('menuitem', { name: copy.disconnect })
      ).toHaveAccessibleDescription(
        'Stop waiting for now. Henry Ito (@crew_henry) can still let you in with the same code.'
      );
    } finally {
      forgetJoinContext(connection.id);
    }
  });

  it('says nothing about a join code to a member', async () => {
    renderWithCrew(<WorkspaceSwitcher />, makeController({ status: 'offline' }));
    const { menu } = await openMenu();
    expect(menu.querySelector('[data-crew-join-code-kept]')).toBeNull();
    for (const name of [copy.reconnect, copy.disconnect]) {
      expect(within(menu).getByRole('menuitem', { name })).not.toHaveAttribute('aria-describedby');
    }
  });

  it('offers Reconnect only while the connection is not connected and verified (Q3-57)', async () => {
    // Connected · identity verified: Reconnect could only restart a healthy connection.
    const view = renderWithCrew(<WorkspaceSwitcher />);
    let { menu } = await openMenu();
    expect(within(menu).queryByRole('menuitem', { name: copy.reconnect })).toBeNull();
    expect(within(menu).getByRole('menuitem', { name: copy.disconnect })).toBeInTheDocument();
    view.unmount();

    for (const status of ['checking', 'offline', 'cant-connect', 'not-joined'] as const) {
      const other = renderWithCrew(
        <WorkspaceSwitcher />,
        makeController({ status, snapshot: null, observedPrivacy: null })
      );
      ({ menu } = await openMenu());
      expect(within(menu).getByRole('menuitem', { name: copy.reconnect })).toBeInTheDocument();
      other.unmount();
    }
  });

  it('states only the server before this connection’s identity is verified', async () => {
    renderWithCrew(
      <WorkspaceSwitcher />,
      makeController({ snapshot: null, observedPrivacy: null, status: 'sign-in-needed' })
    );
    const { menu } = await openMenu();
    const line = menu.querySelector('[data-crew-signed-in]') as HTMLElement;
    expect(line).toHaveTextContent('Server hpc.ucsf.edu');
    expect(line).not.toHaveTextContent(/Signed in/);
  });

  it('describes the menu by its header, which menu navigation would otherwise skip', async () => {
    renderWithCrew(<WorkspaceSwitcher />);
    const { menu } = await openMenu();
    const header = menu.querySelector('[data-crew-menu-header]') as HTMLElement;
    expect(header.id).not.toBe('');
    expect(menu.getAttribute('aria-describedby')?.split(' ')).toContain(header.id);
    expect(menu).toHaveAccessibleDescription(expect.stringContaining('Signed in as @alice'));
  });

  it('shows the last connection error in the header when the connection is not healthy', async () => {
    renderWithCrew(
      <WorkspaceSwitcher />,
      makeController({
        status: 'cant-connect',
        lastConnectFailure: { kind: 'unknown', message: 'ssh: connection refused' },
      })
    );
    const { menu } = await openMenu();
    // In words: a failure's `code: ` prefix never reaches the person (NEW-1, T-08).
    expect(within(menu).getByText(/^Connection refused$/)).toBeInTheDocument();
    expect(within(menu).getByText(crewStatusCopy.cantConnect)).toBeInTheDocument();
  });

  it('never shows the transport’s words for a failure, saved or just made (NEW-1)', async () => {
    const TRANSPORT =
      'Crew SSH failure [ssh_eof; child_before_cleanup=exit_255]: SSH connection closed; reconnect. Submitted operation outcome may be unknown; inspect history before retrying';
    const RAW = /Crew SSH failure|ssh_eof|child_before_cleanup|Submitted operation/;
    renderWithCrew(
      <WorkspaceSwitcher />,
      makeController({
        status: 'offline',
        connection: {
          ...connection,
          status: 'disconnected',
          last_error: TRANSPORT,
          last_error_code: 'crew_ssh_unreachable',
        },
      })
    );
    const { menu } = await openMenu();
    expect(
      within(menu).getByText(connectionBarCopy.unreachable('hpc.ucsf.edu'))
    ).toBeInTheDocument();
    expect(menu).not.toHaveTextContent(RAW);
  });

  it('says a failed Connect by its kind, not in the daemon’s words (NEW-1)', async () => {
    renderWithCrew(
      <WorkspaceSwitcher />,
      makeController({
        status: 'cant-connect',
        lastConnectFailure: {
          kind: 'ssh_failed',
          message:
            'Crew SSH failure [ssh_eof; child_before_cleanup=exit_255]: SSH connection closed; reconnect.',
        },
      })
    );
    const { menu } = await openMenu();
    expect(
      within(menu).getByText(connectionBarCopy.cantConnect('hpc.ucsf.edu'))
    ).toBeInTheDocument();
    expect(menu).not.toHaveTextContent(/ssh_eof|child_before_cleanup/);
  });

  it('lists the workspace items, the connection tools, and Add a workspace', async () => {
    renderWithCrew(<WorkspaceSwitcher />);
    const { menu } = await openMenu();
    const names = within(menu)
      .getAllByRole('menuitem')
      .map((item) => item.textContent);
    // Signed in: no "Sign in…" under "Signed in as @alice" (T-40).
    expect(names).toEqual([
      copy.invite('Fixture'),
      copy.people,
      copy.privacy,
      copy.access,
      copy.createTeam,
      copy.disconnect,
      copy.settings,
      copy.add,
    ]);
    expect(copy.access).toBe('Agent access…');
    // One saved connection: no Switch section.
    expect(within(menu).queryByRole('menuitemradio')).toBeNull();
    expect(within(menu).queryByText(copy.switchWorkspace)).toBeNull();
  });

  it('offers Invite people only to the host', async () => {
    renderWithCrew(<WorkspaceSwitcher />, makeController({ isHost: false }));
    const { menu } = await openMenu();
    expect(within(menu).queryByRole('menuitem', { name: copy.invite('Fixture') })).toBeNull();
  });

  it.each([
    [copy.invite('Fixture'), { kind: 'invite-people' }],
    [copy.people, { kind: 'workspace-settings', tab: 'people' }],
    [copy.privacy, { kind: 'workspace-settings', tab: 'privacy' }],
    [copy.access, { kind: 'workspace-settings', tab: 'agent-access' }],
    [copy.createTeam, { kind: 'create-team' }],
    [copy.settings, { kind: 'connection-settings', connectionId: connection.id }],
  ])('%s opens its dialog through the controller', async (item, intent) => {
    const controller = makeController();
    renderWithCrew(<WorkspaceSwitcher />, controller);
    await choose(item);
    expect(controller.openDialog).toHaveBeenCalledWith(intent);
  });

  it('Reconnect is a user-initiated connect; Disconnect calls its own', async () => {
    const controller = makeController({ status: 'offline' });
    renderWithCrew(<WorkspaceSwitcher />, controller);
    await choose(copy.reconnect);
    expect(controller.connect).toHaveBeenCalledWith({ userInitiated: true });
    await choose(copy.disconnect);
    expect(controller.disconnect).toHaveBeenCalledTimes(1);
  });

  it.each(['connected', 'checking', 'offline', 'cant-connect', 'not-joined'] as const)(
    'offers no Sign in… when the status is %s',
    async (status) => {
      renderWithCrew(<WorkspaceSwitcher />, makeController({ status }));
      const { menu } = await openMenu();
      expect(within(menu).queryByRole('menuitem', { name: copy.signIn })).toBeNull();
    }
  );

  it('offers Sign in… while sign-in is needed, and it opens Sign in', async () => {
    const controller = makeController({
      status: 'sign-in-needed',
      snapshot: null,
      observedPrivacy: null,
    });
    renderWithCrew(<WorkspaceSwitcher />, controller);
    await choose(copy.signIn);
    expect(controller.openSignIn).toHaveBeenCalledTimes(1);
  });

  it('disables Reconnect while a connect or sign-in runs', async () => {
    renderWithCrew(
      <WorkspaceSwitcher />,
      makeController({ status: 'connecting', isPending: (key) => key === 'connect' })
    );
    const { menu } = await openMenu();
    expect(within(menu).getByRole('menuitem', { name: copy.reconnect })).toHaveAttribute(
      'aria-disabled',
      'true'
    );
  });

  it('disables the workspace’s own items until its snapshot is verified', async () => {
    const snapshot = makeSnapshot();
    renderWithCrew(
      <WorkspaceSwitcher />,
      makeController({
        snapshot: null,
        observedPrivacy: null,
        effectivePrivacy: null,
        lastVerified: {
          connectionId: connection.id,
          snapshot,
          observedPrivacy: {
            connectionId: connection.id,
            mode: 'private',
            institutionId: 'ucsf',
            policyEpoch: 1,
          },
          runs: [],
          labels: null,
          teamId: '',
          channelId: '',
          messages: [],
        },
      })
    );
    const { menu } = await openMenu();
    for (const item of [copy.people, copy.privacy, copy.access, copy.createTeam]) {
      expect(within(menu).getByRole('menuitem', { name: item })).toHaveAttribute(
        'aria-disabled',
        'true'
      );
    }
    // …and says why, above them and in the menu's description (T-40). The connection is up and
    // only unverified, so the note does not tell this person to connect.
    const note = menu.querySelector('[data-crew-menu-note]') as HTMLElement;
    expect(note).toHaveTextContent(sidebarCopy.unavailable.notVerified);
    expect(menu.getAttribute('aria-describedby')?.split(' ')).toContain(note.id);
    // The connection tools stay available: they are how a person gets it verified again.
    expect(within(menu).getByRole('menuitem', { name: copy.reconnect })).not.toHaveAttribute(
      'aria-disabled'
    );
  });

  it('tells a joiner the workspace’s items open once they join', async () => {
    renderWithCrew(
      <WorkspaceSwitcher />,
      makeController({ snapshot: null, observedPrivacy: null, status: 'not-joined' })
    );
    const { menu } = await openMenu();
    expect(menu.querySelector('[data-crew-menu-note]')).toHaveTextContent(
      'Available after you join'
    );
  });

  it('shows no reason while nothing is disabled', async () => {
    renderWithCrew(<WorkspaceSwitcher />);
    const { menu } = await openMenu();
    expect(menu.querySelector('[data-crew-menu-note]')).toBeNull();
  });

  it('switches workspaces with a radio group when two or more are saved', async () => {
    const controller = makeController({ connections: [connection, secondConnection] });
    renderWithCrew(<WorkspaceSwitcher />, controller);
    const { user, menu } = await openMenu();
    expect(within(menu).getByText(copy.switchWorkspace)).toBeInTheDocument();
    const radios = within(menu).getAllByRole('menuitemradio');
    expect(radios).toHaveLength(2);
    expect(radios[0]).toHaveAttribute('aria-checked', 'true');
    expect(radios[0]).toHaveAccessibleName(`Fixture, ${crewStatusCopy.connected}`);
    expect(radios[1]).toHaveAccessibleName(`Imaging core, ${crewStatusCopy.offline}`);
    // Each dot is decorative beside its word.
    expect(radios[1].querySelector('[data-slot="status-dot"]')).toHaveAttribute(
      'data-tone',
      'idle'
    );
    await user.click(radios[1]);
    expect(controller.selectConnection).toHaveBeenCalledWith(secondConnection.id);
  });

  it('opens Join and Host from the Add a workspace submenu', async () => {
    const controller = makeController();
    renderWithCrew(<WorkspaceSwitcher />, controller);
    const { user, menu } = await openMenu();
    const sub = within(menu).getByRole('menuitem', { name: copy.add });
    expect(sub).toHaveAttribute('aria-haspopup', 'menu');
    // The keyboard path: → opens the submenu and moves into it, Enter chooses.
    sub.focus();
    await user.keyboard('{ArrowRight}');
    const join = await screen.findByRole('menuitem', { name: copy.addJoin });
    expect(screen.getByRole('menuitem', { name: copy.addHost })).toBeInTheDocument();
    await waitFor(() => expect(join).toHaveFocus());
    await user.keyboard('{Enter}');
    expect(controller.openDialog).toHaveBeenCalledWith({ kind: 'join' });
  });

  it('never shows an ID in the menu', async () => {
    const snapshot = makeSnapshot({ principals: [bob] });
    renderWithCrew(<WorkspaceSwitcher />, makeController({ snapshot }));
    const { menu } = await openMenu();
    await waitFor(() => expect(menu.textContent).not.toMatch(/person-|workspace-1|conn-1/));
  });
});

describe('offersReconnect', () => {
  it('is false only for a connection that is connected and verified', () => {
    expect(offersReconnect('connected')).toBe(false);
    for (const status of [
      'connecting',
      'reconnecting',
      'checking',
      'updating',
      'updates-unavailable',
      'not-joined',
      'offline',
      'sign-in-needed',
      'cant-connect',
      'cant-verify',
      'not-set-up',
    ] as const) {
      expect(offersReconnect(status)).toBe(true);
    }
    expect(offersReconnect(null)).toBe(true);
  });
});

describe('unavailableReason', () => {
  it('gives no reason once the workspace is ready', () => {
    expect(unavailableReason('connected', true)).toBeNull();
  });

  it('tells a joiner the items open once they join', () => {
    expect(unavailableReason('not-joined', false)).toBe(sidebarCopy.unavailable.notJoined);
  });

  it.each(['connected', 'checking', 'updating', 'updates-unavailable'])(
    'does not tell a person whose connection is up (%s) to connect',
    (status) => {
      expect(unavailableReason(status, false)).toBe(sidebarCopy.unavailable.notVerified);
    }
  );

  it.each([null, 'offline', 'connecting', 'sign-in-needed', 'cant-connect', 'cant-verify'])(
    'tells a person who is not connected (%s) that the items open once they are',
    (status) => {
      expect(unavailableReason(status, false)).toBe(sidebarCopy.unavailable.notConnected);
    }
  );
});
