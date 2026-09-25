import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CrewMessage, Invitation } from '../crewApi';
import { buildPeopleDirectory } from '../identity';
import { MARK_READ_KEY } from './ChannelRow';
import { sidebarCopy } from './copy';
import { SidebarAnnouncer } from './SidebarAnnouncer';
import { unreadBadgeText } from './sidebarView';
import { TEAM_COPY_CLOSE_MS } from './TeamSection';
import { teamRoles, TeamSections } from './TeamSections';
import { COLLAPSED_TEAMS_STORAGE_KEY } from './useCollapsedTeams';
import {
  alice,
  bob,
  connection,
  makeController,
  makeSnapshot,
  renderWithCrew,
  TEAM_LAB,
  TEAM_SC,
  type ControllerOverrides,
} from './sidebarTestUtils';

function renderTeams(overrides: ControllerOverrides = {}, renameEnabled = false) {
  return renderWithCrew(
    <SidebarAnnouncer>
      <TeamSections renameEnabled={renameEnabled} />
    </SidebarAnnouncer>,
    makeController(overrides)
  );
}

const header = (name: RegExp | string) => screen.getByRole('button', { name });

/**
 * A spy on whichever clipboard is installed now. `userEvent.setup()` replaces
 * `navigator.clipboard` with its own stub, so call this AFTER it.
 */
function spyClipboard() {
  return vi.spyOn(navigator.clipboard, 'writeText').mockResolvedValue(undefined);
}
const channelRow = (name: RegExp | string) => screen.getByRole('button', { name });

beforeEach(() => {
  localStorage.removeItem(COLLAPSED_TEAMS_STORAGE_KEY);
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('team sections', () => {
  it('renders each team as typed — never upper-cased — with its channel rows in a list', () => {
    renderTeams();
    const lab = header('Analysis Lab, 4 channels');
    const sc = header('single-cell, 1 channel');
    expect(lab).toHaveTextContent('Analysis Lab');
    expect(sc).toHaveTextContent('single-cell');
    expect(lab).toHaveAttribute('aria-expanded', 'true');
    const list = document.getElementById(lab.getAttribute('aria-controls') ?? '');
    expect(list).toHaveAttribute('role', 'list');
    expect(within(list as HTMLElement).getAllByRole('listitem').length).toBeGreaterThan(0);
    // Team, channel and person IDs never reach the screen.
    expect(document.body.textContent).not.toMatch(/team-lab|chan-|person-/);
  });

  it('marks the selected channel with aria-current="page" and no other', () => {
    renderTeams();
    const methods = channelRow('methods');
    expect(methods).toHaveAttribute('aria-current', 'page');
    expect(document.querySelectorAll('[aria-current="page"]')).toHaveLength(1);
  });

  it('shows unread as weight and a neutral count, never as a hue', () => {
    renderTeams();
    const raw = channelRow('raw-data, 3 unread');
    expect(raw).toHaveAttribute('data-unread', 'true');
    const badge = raw.querySelector('[aria-hidden="true"].tabular-nums') as HTMLElement;
    expect(badge).toHaveTextContent('3');
    expect(badge.className).toContain('bg-background-medium');
    expect(badge.className).not.toMatch(/accent|danger|warning|success|info/);
    // A read channel is plain.
    expect(channelRow('general')).not.toHaveAttribute('data-unread');
  });

  it('caps the count at 99+', () => {
    expect(unreadBadgeText(99)).toBe('99');
    expect(unreadBadgeText(100)).toBe('99+');
    renderTeams({ snapshot: makeSnapshot({ unread: { 'chan-raw': 250 } }) });
    expect(channelRow('raw-data, 250 unread')).toHaveTextContent('99+');
  });

  it('collapses instantly — the rows go in the same render — and only the chevron turns', () => {
    renderTeams();
    const lab = header('Analysis Lab, 4 channels');
    const chevron = lab.querySelector('.crew-sidebar-chevron');
    expect(chevron).toHaveAttribute('data-turn', 'quarter');
    expect(channelRow('general')).toBeInTheDocument();

    const section = lab.closest('[data-crew-team]') as HTMLElement;
    fireEvent.click(lab);
    expect(lab).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByRole('button', { name: 'general' })).toBeNull();
    expect(within(section).queryByRole('button', { name: sidebarCopy.channel.add })).toBeNull();
    // The selected channel stays visible, so the person never loses their place.
    expect(channelRow('methods')).toHaveAttribute('aria-current', 'page');

    fireEvent.click(lab);
    expect(lab).toHaveAttribute('aria-expanded', 'true');
    expect(channelRow('general')).toBeInTheDocument();
  });

  it('remembers a collapse per viewer and per connection, and survives a remount', () => {
    const view = renderTeams();
    fireEvent.click(header('single-cell, 1 channel'));
    const stored = JSON.parse(localStorage.getItem(COLLAPSED_TEAMS_STORAGE_KEY) ?? '{}');
    expect(stored).toEqual({ [connection.id]: [TEAM_SC] });
    view.unmount();

    renderTeams();
    expect(header('single-cell, 1 channel')).toHaveAttribute('aria-expanded', 'false');
    expect(header('Analysis Lab, 4 channels')).toHaveAttribute('aria-expanded', 'true');
  });

  it('still renders when storage throws', () => {
    const getItem = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('denied');
    });
    const setItem = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('denied');
    });
    try {
      renderTeams();
      const sc = header('single-cell, 1 channel');
      fireEvent.click(sc);
      expect(sc).toHaveAttribute('aria-expanded', 'false');
    } finally {
      getItem.mockRestore();
      setItem.mockRestore();
    }
  });

  it('selects a channel in the current team without switching teams', () => {
    const view = renderTeams();
    fireEvent.click(channelRow('general'));
    expect(view.controller.selectTeam).not.toHaveBeenCalled();
    expect(view.controller.selectChannel).toHaveBeenCalledWith('chan-general');
  });

  it('switches team first when the channel is in another team', () => {
    const view = renderTeams();
    fireEvent.click(channelRow('intro'));
    expect(view.controller.selectTeam).toHaveBeenCalledWith(TEAM_SC);
    expect(view.controller.selectChannel).toHaveBeenCalledWith('chan-intro');
    const teamOrder = vi.mocked(view.controller.selectTeam).mock.invocationCallOrder[0];
    const channelOrder = vi.mocked(view.controller.selectChannel).mock.invocationCallOrder[0];
    expect(teamOrder).toBeLessThan(channelOrder);
  });

  it('keeps archived channels under a collapsed "Archived (n)" row', () => {
    renderTeams();
    const archived = screen.getByRole('button', { name: sidebarCopy.channel.archivedGroup(1) });
    expect(archived).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByRole('button', { name: 'old-notes' })).toBeNull();
    fireEvent.click(archived);
    expect(channelRow('old-notes')).toHaveAttribute('data-archived', 'true');
  });

  it('opens Create channel from the header + and the quiet row, and Create team from + Add team', () => {
    const view = renderTeams();
    fireEvent.click(screen.getByRole('button', { name: 'Create channel in Analysis Lab' }));
    expect(view.controller.openDialog).toHaveBeenLastCalledWith({
      kind: 'create-channel',
      teamId: TEAM_LAB,
    });
    const addChannel = screen.getAllByRole('button', { name: sidebarCopy.channel.add });
    fireEvent.click(addChannel[1]);
    expect(view.controller.openDialog).toHaveBeenLastCalledWith({
      kind: 'create-channel',
      teamId: TEAM_SC,
    });
    fireEvent.click(screen.getByRole('button', { name: sidebarCopy.team.add }));
    expect(view.controller.openDialog).toHaveBeenLastCalledWith({ kind: 'create-team' });
  });

  it('gives the team ⋯ menu its items, with Rename only once names are supported', async () => {
    const user = userEvent.setup();
    const view = renderTeams();
    const options = screen.getByRole('button', { name: 'Analysis Lab options' });
    expect(options).toHaveAttribute('aria-haspopup', 'menu');
    await user.click(options);
    let menu = await screen.findByRole('menu');
    expect(
      within(menu)
        .getAllByRole('menuitem')
        .map((item) => item.textContent)
    ).toEqual([
      sidebarCopy.teamMenu.createChannel,
      sidebarCopy.teamMenu.addPeople('Analysis Lab'),
      sidebarCopy.teamMenu.copyId,
    ]);
    await user.click(
      within(menu).getByRole('menuitem', { name: sidebarCopy.teamMenu.addPeople('Analysis Lab') })
    );
    expect(view.controller.openDialog).toHaveBeenCalledWith({
      kind: 'add-people',
      target: 'team',
      targetId: TEAM_LAB,
    });
    view.unmount();

    const renamed = renderTeams({}, true);
    await user.click(screen.getByRole('button', { name: 'Analysis Lab options' }));
    menu = await screen.findByRole('menu');
    await user.click(within(menu).getByRole('menuitem', { name: sidebarCopy.teamMenu.rename }));
    expect(renamed.controller.openDialog).toHaveBeenCalledWith({
      kind: 'rename',
      target: 'team',
      targetId: TEAM_LAB,
    });
  });

  it('offers Add people and Rename only to the team’s owner and the host (Q2-41)', async () => {
    const user = userEvent.setup();
    // Bob is neither: a member of a team Alice created, in a workspace Alice hosts.
    const member = renderTeams({ snapshot: makeSnapshot({ actor: bob }), isHost: false }, true);
    await user.click(screen.getByRole('button', { name: 'Analysis Lab options' }));
    let menu = await screen.findByRole('menu');
    expect(
      within(menu)
        .getAllByRole('menuitem')
        .map((item) => item.textContent)
    ).toEqual([sidebarCopy.teamMenu.createChannel, sidebarCopy.teamMenu.copyId]);
    member.unmount();

    // The host, who did not create the team, may manage it (the broker lets the host act on any
    // channel it can see; `dialogs/people.ts`).
    const snapshot = makeSnapshot({
      actor: bob,
      workspace: { id: 'workspace-1', host_uid: 1001, mode: 'private', policy_epoch: 1 },
    });
    renderTeams({ snapshot, isHost: true }, true);
    await user.click(screen.getByRole('button', { name: 'Analysis Lab options' }));
    menu = await screen.findByRole('menu');
    expect(
      within(menu)
        .getAllByRole('menuitem')
        .map((item) => item.textContent)
    ).toEqual([
      sidebarCopy.teamMenu.createChannel,
      sidebarCopy.teamMenu.addPeople('Analysis Lab'),
      sidebarCopy.teamMenu.rename,
      sidebarCopy.teamMenu.copyId,
    ]);
  });

  it('copies the team ID, says "Copied" on the item for a moment, then closes (Q2-34)', async () => {
    const user = userEvent.setup();
    const writeText = spyClipboard();
    const view = renderTeams();
    const options = screen.getByRole('button', { name: 'Analysis Lab options' });
    await user.click(options);
    const menu = await screen.findByRole('menu');
    // Last, after a separator.
    const items = within(menu).getAllByRole('menuitem');
    const copyItem = within(menu).getByRole('menuitem', { name: sidebarCopy.teamMenu.copyId });
    expect(items[items.length - 1]).toBe(copyItem);
    expect(copyItem.previousElementSibling).toHaveAttribute('role', 'separator');

    await user.click(copyItem);
    expect(writeText).toHaveBeenCalledWith(TEAM_LAB);
    // The menu stays open, and the item itself answers…
    expect(
      await within(menu).findByRole('menuitem', { name: sidebarCopy.teamMenu.copied })
    ).toBeVisible();
    expect(screen.getByRole('menu')).toBe(menu);
    // …spoken too, and never as a toast or in the connection bar.
    await waitFor(() =>
      expect(document.querySelector('[data-crew-sidebar-announcer]')).toHaveTextContent(
        sidebarCopy.clipboard.copied
      )
    );
    expect(view.controller.reportError).not.toHaveBeenCalled();
    // Then it closes by itself, and focus goes back to the ⋯ that opened it.
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull(), {
      timeout: TEAM_COPY_CLOSE_MS + 1000,
    });
    await waitFor(() => expect(options).toHaveFocus());
  });

  it('shows a refused copy on the item and keeps the menu open, never in the connection bar', async () => {
    const user = userEvent.setup();
    spyClipboard().mockRejectedValueOnce(new Error('denied'));
    const view = renderTeams();
    await user.click(screen.getByRole('button', { name: 'Analysis Lab options' }));
    const menu = await screen.findByRole('menu');
    await user.click(within(menu).getByRole('menuitem', { name: sidebarCopy.teamMenu.copyId }));
    expect(
      await within(menu).findByRole('menuitem', { name: sidebarCopy.teamMenu.copyFailed })
    ).toBeVisible();
    await waitFor(() =>
      expect(document.querySelector('[data-crew-sidebar-announcer]')).toHaveTextContent(
        sidebarCopy.clipboard.failed
      )
    );
    expect(view.controller.reportError).not.toHaveBeenCalled();
    await new Promise((resolve) => setTimeout(resolve, TEAM_COPY_CLOSE_MS + 100));
    expect(screen.getByRole('menu')).toBe(menu);
  });

  it('disables every action while it shows only the last verified copy', () => {
    const snapshot = makeSnapshot();
    renderTeams({
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
        teamId: TEAM_LAB,
        channelId: 'chan-methods',
        messages: [],
      },
    });
    // The places persist…
    expect(channelRow('methods')).toHaveAttribute('aria-current', 'page');
    // …and nothing is actionable.
    expect(screen.getByRole('button', { name: sidebarCopy.team.add })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Create channel in Analysis Lab' })).toBeDisabled();
  });

  it('renders nothing without a verified or last verified snapshot', () => {
    const { container } = renderTeams({ snapshot: null, observedPrivacy: null });
    expect(container.querySelector('[data-crew-sidebar-teams]')).toBeNull();
  });
});

describe('pending team invitations and what a member cannot see', () => {
  const carol = { id: 'person-carol-0000', uid: 1002, username: 'carol', nickname: 'Carol Diaz' };
  const dana = { id: 'person-dana-0000', uid: 1003, username: 'dana', nickname: 'Dana Wu' };

  function teamInvitation(overrides: Partial<Invitation> = {}): Invitation {
    return {
      id: 'invitation-carol',
      kind: 'team',
      target_id: TEAM_LAB,
      principal_id: carol.id,
      inviter_id: alice.id,
      expires_at: 4_000_000_000,
      target_name: 'Analysis Lab',
      ...overrides,
    };
  }

  const ownerSnapshot = (invitations: Invitation[]) =>
    makeSnapshot({ principals: [alice, bob, carol, dana], invitations });

  it('shows the owner "· N invited", with the names in a tooltip and the header’s description (P0-2)', async () => {
    const user = userEvent.setup();
    renderTeams({
      snapshot: ownerSnapshot([
        teamInvitation(),
        teamInvitation({ id: 'invitation-dana', principal_id: dana.id }),
      ]),
    });
    const lab = header('Analysis Lab, 4 channels, 2 invited');
    const count = lab.querySelector('[data-crew-team-invited]') as HTMLElement;
    expect(count).toHaveTextContent('· 2 invited');
    expect(lab).toHaveAccessibleDescription(
      'Invited, not accepted yet: Carol Diaz (@carol), Dana Wu (@dana)'
    );
    await user.hover(count);
    expect(
      (
        await screen.findAllByText(
          'Invited, not accepted yet: Carol Diaz (@carol), Dana Wu (@dana)'
        )
      ).length
    ).toBeGreaterThan(0);
    // The other team has no pending invitation and says nothing.
    expect(header('single-cell, 1 channel')).not.toHaveTextContent(/invited/);
  });

  it('counts only standing invitations to the team, made by its owner', () => {
    renderTeams({
      snapshot: ownerSnapshot([
        teamInvitation({ id: 'expired', expired: true }),
        teamInvitation({ id: 'ran-out', expires_at: 1 }),
        teamInvitation({ id: 'channel', kind: 'channel', target_id: 'chan-general' }),
        teamInvitation({ id: 'other-team', target_id: TEAM_SC, principal_id: dana.id }),
      ]),
    });
    expect(header('Analysis Lab, 4 channels')).not.toHaveTextContent(/invited/);
    expect(header('single-cell, 1 channel, 1 invited')).toBeInTheDocument();
  });

  it('never shows a member the count, and tells them other channels appear once added (T-28)', () => {
    renderTeams({
      snapshot: makeSnapshot({
        actor: bob,
        principals: [alice, bob, carol],
        invitations: [teamInvitation({ principal_id: bob.id })],
      }),
      isHost: false,
    });
    expect(header('Analysis Lab, 4 channels')).not.toHaveTextContent(/invited/);
    const lab = header('Analysis Lab, 4 channels').closest('[data-crew-team]') as HTMLElement;
    const hint = lab.querySelector('[data-crew-member-hint]');
    expect(hint).toHaveTextContent('Other channels in Analysis Lab appear once someone adds you.');
    // A note, not a row: it is never a tab or arrow stop.
    expect(hint?.querySelector('button, [tabindex]')).toBeNull();
    expect(hint).not.toHaveAttribute('data-crew-row');

    fireEvent.click(header('Analysis Lab, 4 channels'));
    expect(lab.querySelector('[data-crew-member-hint]')).toBeNull();
  });

  it('shows the owner no member hint', () => {
    renderTeams();
    expect(document.querySelector('[data-crew-member-hint]')).toBeNull();
  });

  it('teamRoles names invitees through the directory, and an unknown one generically', () => {
    const snapshot = ownerSnapshot([
      teamInvitation({ principal_id: 'person-gone-0000' }),
      teamInvitation({ id: 'to-me', principal_id: alice.id }),
    ]);
    const roles = teamRoles(snapshot, buildPeopleDirectory(snapshot, null));
    expect(roles.get(TEAM_LAB)).toEqual({ kind: 'owner', invited: ['Unknown member'] });
    expect(roles.get(TEAM_SC)).toEqual({ kind: 'owner', invited: [] });
    expect(teamRoles(null, buildPeopleDirectory(null, null)).size).toBe(0);
  });

  it('counts a person with two live invitations to the team once', () => {
    const snapshot = ownerSnapshot([
      teamInvitation(),
      teamInvitation({ id: 'invitation-carol-again' }),
      teamInvitation({ id: 'invitation-dana', principal_id: dana.id }),
    ]);
    const roles = teamRoles(snapshot, buildPeopleDirectory(snapshot, null));
    expect(roles.get(TEAM_LAB)).toEqual({
      kind: 'owner',
      invited: ['Carol Diaz (@carol)', 'Dana Wu (@dana)'],
    });
    renderTeams({ snapshot });
    expect(header('Analysis Lab, 4 channels, 2 invited')).toHaveAccessibleDescription(
      'Invited, not accepted yet: Carol Diaz (@carol), Dana Wu (@dana)'
    );
  });
});

describe('keyboard', () => {
  it('holds one tab stop, on the selected channel', () => {
    renderTeams();
    const stops = Array.from(document.querySelectorAll<HTMLElement>('[data-crew-row]')).filter(
      (row) => row.tabIndex === 0
    );
    expect(stops).toEqual([channelRow('methods')]);
  });

  it('moves between rows with ↑/↓, Home and End, across teams', () => {
    renderTeams();
    const methods = channelRow('methods');
    act(() => methods.focus());
    fireEvent.keyDown(methods, { key: 'ArrowDown' });
    expect(channelRow('raw-data, 3 unread')).toHaveFocus();
    fireEvent.keyDown(document.activeElement as HTMLElement, { key: 'ArrowUp' });
    fireEvent.keyDown(document.activeElement as HTMLElement, { key: 'ArrowUp' });
    expect(channelRow('general')).toHaveFocus();
    fireEvent.keyDown(document.activeElement as HTMLElement, { key: 'ArrowUp' });
    expect(header('Analysis Lab, 4 channels')).toHaveFocus();
    fireEvent.keyDown(document.activeElement as HTMLElement, { key: 'End' });
    expect(screen.getByRole('button', { name: sidebarCopy.team.add })).toHaveFocus();
    fireEvent.keyDown(document.activeElement as HTMLElement, { key: 'ArrowUp' });
    expect(screen.getAllByRole('button', { name: sidebarCopy.channel.add })[1]).toHaveFocus();
    fireEvent.keyDown(document.activeElement as HTMLElement, { key: 'ArrowUp' });
    expect(channelRow('intro')).toHaveFocus();
    fireEvent.keyDown(document.activeElement as HTMLElement, { key: 'Home' });
    expect(header('Analysis Lab, 4 channels')).toHaveFocus();
    // The row that last had focus now holds the tab stop.
    expect(header('Analysis Lab, 4 channels').tabIndex).toBe(0);
    expect(channelRow('methods').tabIndex).toBe(-1);
  });

  it('collapses with ← and expands with → on a team header', () => {
    renderTeams();
    const lab = header('Analysis Lab, 4 channels');
    act(() => lab.focus());
    fireEvent.keyDown(lab, { key: 'ArrowLeft' });
    expect(lab).toHaveAttribute('aria-expanded', 'false');
    fireEvent.keyDown(lab, { key: 'ArrowRight' });
    expect(lab).toHaveAttribute('aria-expanded', 'true');
  });

  it('returns from a channel to its team header with ←', () => {
    renderTeams();
    const intro = channelRow('intro');
    act(() => intro.focus());
    fireEvent.keyDown(intro, { key: 'ArrowLeft' });
    expect(header('single-cell, 1 channel')).toHaveFocus();
  });

  it('hands the tab stop back to the current channel once focus leaves the rail (Q2-46)', () => {
    renderTeams();
    const addChannel = screen.getAllByRole('button', { name: sidebarCopy.channel.add })[0];
    act(() => addChannel.focus());
    // While focus is inside, the stop follows it…
    expect(addChannel.tabIndex).toBe(0);
    expect(channelRow('methods').tabIndex).toBe(-1);
    // …and once it leaves, Tab or Shift+Tab back in lands on the channel the person is in, never
    // on "+ Add channel", one Enter away from a duplicate channel.
    const outside = document.createElement('button');
    document.body.appendChild(outside);
    act(() => outside.focus());
    expect(channelRow('methods').tabIndex).toBe(0);
    expect(addChannel.tabIndex).toBe(-1);
    outside.remove();
  });

  it('keeps the stop where it is while focus moves between rows', () => {
    renderTeams();
    const methods = channelRow('methods');
    act(() => methods.focus());
    fireEvent.keyDown(methods, { key: 'ArrowUp' });
    expect(channelRow('general')).toHaveFocus();
    expect(channelRow('general').tabIndex).toBe(0);
  });

  it('moves from a header’s + to the next row too', () => {
    renderTeams();
    const plus = screen.getByRole('button', { name: 'Create channel in Analysis Lab' });
    act(() => plus.focus());
    fireEvent.keyDown(plus, { key: 'ArrowDown' });
    expect(channelRow('general')).toHaveFocus();
  });
});

describe('the channel context menu', () => {
  const lastMessage: CrewMessage = {
    id: 'message-9',
    sequence: '42',
    channel_id: 'chan-methods',
    actor_id: 'person-bob-0000',
    body: 'hello',
    created_at: 1,
    restricted: false,
    source_channels: [],
    attachments: [],
  };

  it('opens on right-click with the copy items', async () => {
    const writeText = spyClipboard();
    renderTeams();
    fireEvent.contextMenu(channelRow('general'));
    const menu = await screen.findByRole('menu');
    expect(
      within(menu)
        .getAllByRole('menuitem')
        .map((item) => item.textContent)
    ).toEqual([sidebarCopy.channelMenu.copyName, sidebarCopy.channelMenu.copyId]);
    fireEvent.click(within(menu).getByRole('menuitem', { name: sidebarCopy.channelMenu.copyName }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith('#general'));
  });

  // Chromium sends no `contextmenu` for these keys on macOS, so a test that only fires
  // `contextmenu` says nothing about the keyboard: fire the keys themselves.
  it.each([
    ['Shift+F10', { key: 'F10', shiftKey: true }],
    ['the Menu key', { key: 'ContextMenu' }],
  ])('opens from the keyboard with %s, and Escape returns focus to the row', async (_, keys) => {
    const user = userEvent.setup();
    const writeText = spyClipboard();
    renderTeams();
    const row = channelRow('general');
    act(() => row.focus());
    // A prevented keydown is what keeps Chromium's own dispatch (Linux, Windows) from opening
    // the menu a second time.
    expect(fireEvent.keyDown(row, keys)).toBe(false);
    const menu = await screen.findByRole('menu');
    expect(
      within(menu)
        .getAllByRole('menuitem')
        .map((item) => item.textContent)
    ).toEqual([sidebarCopy.channelMenu.copyName, sidebarCopy.channelMenu.copyId]);
    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull());
    expect(row).toHaveFocus();
    expect(writeText).not.toHaveBeenCalled();
  });

  it('reaches Copy channel name with the keyboard alone', async () => {
    const user = userEvent.setup();
    const writeText = spyClipboard();
    renderTeams();
    const row = channelRow('general');
    act(() => row.focus());
    fireEvent.keyDown(row, { key: 'F10', shiftKey: true });
    const menu = await screen.findByRole('menu');
    // Opened from the keyboard, the menu lands on its first item, as a native menu does.
    const copyName = within(menu).getByRole('menuitem', {
      name: sidebarCopy.channelMenu.copyName,
    });
    await waitFor(() => expect(copyName).toHaveFocus());
    await user.keyboard('{ArrowDown}');
    expect(
      within(menu).getByRole('menuitem', { name: sidebarCopy.channelMenu.copyId })
    ).toHaveFocus();
    await user.keyboard('{ArrowUp}');
    expect(copyName).toHaveFocus();
    await user.keyboard('{Enter}');
    await waitFor(() => expect(writeText).toHaveBeenCalledWith('#general'));
  });

  it.each([
    ['unmodified F10', { key: 'F10' }],
    ['Ctrl+Shift+F10', { key: 'F10', shiftKey: true, ctrlKey: true }],
    ['Shift with the Menu key', { key: 'ContextMenu', shiftKey: true }],
    ['Cmd with the Menu key', { key: 'ContextMenu', metaKey: true }],
  ])('leaves %s alone', (_, keys) => {
    renderTeams();
    const row = channelRow('general');
    act(() => row.focus());
    expect(fireEvent.keyDown(row, keys)).toBe(true);
    expect(screen.queryByRole('menu')).toBeNull();
  });

  it('copies the channel ID only from the menu', async () => {
    const writeText = spyClipboard();
    renderTeams();
    expect(document.body.textContent).not.toContain('chan-general');
    fireEvent.contextMenu(channelRow('general'));
    const menu = await screen.findByRole('menu');
    fireEvent.click(within(menu).getByRole('menuitem', { name: sidebarCopy.channelMenu.copyId }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith('chan-general'));
  });

  it('marks the selected unread channel read at its newest message, without a refresh', async () => {
    const view = renderTeams({
      snapshot: makeSnapshot({ unread: { 'chan-methods': 2 } }),
      messages: [lastMessage],
    });
    fireEvent.contextMenu(channelRow('methods, 2 unread'));
    const menu = await screen.findByRole('menu');
    fireEvent.click(within(menu).getByRole('menuitem', { name: sidebarCopy.channelMenu.markRead }));
    await waitFor(() =>
      expect(view.controller.markRead).toHaveBeenCalledWith('chan-methods', '42')
    );
    expect(view.controller.act).toHaveBeenCalledWith('global', MARK_READ_KEY, expect.any(Function));
    expect(view.controller.refresh).not.toHaveBeenCalled();
  });

  it('offers Mark as read only for the selected channel on its live tail', async () => {
    renderTeams({
      snapshot: makeSnapshot({ unread: { 'chan-methods': 2 } }),
      messages: [lastMessage],
      historyBefore: '40',
    });
    fireEvent.contextMenu(channelRow('methods, 2 unread'));
    const menu = await screen.findByRole('menu');
    expect(
      within(menu).queryByRole('menuitem', { name: sidebarCopy.channelMenu.markRead })
    ).toBeNull();
  });
});
