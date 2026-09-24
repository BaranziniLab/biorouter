import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { crewStatusCopy } from '../state/copy';
import { sidebarCopy } from './copy';
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
    expect(header).toHaveTextContent('Signed in as alice@hpc.ucsf.edu');
    expect(within(header).getByText(crewStatusCopy.verified)).toBeInTheDocument();
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
    expect(within(menu).getByText('ssh: connection refused')).toBeInTheDocument();
    expect(within(menu).getByText(crewStatusCopy.cantConnect)).toBeInTheDocument();
  });

  it('lists the workspace items, the connection tools, and Add a workspace', async () => {
    renderWithCrew(<WorkspaceSwitcher />);
    const { menu } = await openMenu();
    const names = within(menu)
      .getAllByRole('menuitem')
      .map((item) => item.textContent);
    expect(names).toEqual([
      copy.invite('Fixture'),
      copy.people,
      copy.privacy,
      copy.access,
      copy.createTeam,
      copy.reconnect,
      copy.signIn,
      copy.disconnect,
      copy.settings,
      copy.add,
    ]);
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

  it('Reconnect is a user-initiated connect; Sign in… and Disconnect call theirs', async () => {
    const controller = makeController();
    renderWithCrew(<WorkspaceSwitcher />, controller);
    await choose(copy.reconnect);
    expect(controller.connect).toHaveBeenCalledWith({ userInitiated: true });
    await choose(copy.signIn);
    expect(controller.openSignIn).toHaveBeenCalledTimes(1);
    await choose(copy.disconnect);
    expect(controller.disconnect).toHaveBeenCalledTimes(1);
  });

  it('disables Reconnect while a connect or sign-in runs', async () => {
    renderWithCrew(
      <WorkspaceSwitcher />,
      makeController({ isPending: (key) => key === 'connect' })
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
    // The connection tools stay available: they are how a person gets it verified again.
    expect(within(menu).getByRole('menuitem', { name: copy.reconnect })).not.toHaveAttribute(
      'aria-disabled'
    );
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
