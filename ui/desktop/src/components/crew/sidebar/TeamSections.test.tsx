import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CrewMessage } from '../crewApi';
import { MARK_READ_KEY } from './ChannelRow';
import { sidebarCopy } from './copy';
import { SidebarAnnouncer } from './SidebarAnnouncer';
import { unreadBadgeText } from './sidebarView';
import { TeamSections } from './TeamSections';
import { COLLAPSED_TEAMS_STORAGE_KEY } from './useCollapsedTeams';
import {
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

  it('copies the team ID from its menu and says so without a toast', async () => {
    const user = userEvent.setup();
    const writeText = spyClipboard();
    renderTeams();
    await user.click(screen.getByRole('button', { name: 'Analysis Lab options' }));
    const menu = await screen.findByRole('menu');
    await user.click(within(menu).getByRole('menuitem', { name: sidebarCopy.teamMenu.copyId }));
    expect(writeText).toHaveBeenCalledWith(TEAM_LAB);
    await waitFor(() =>
      expect(document.querySelector('[data-crew-sidebar-announcer]')).toHaveTextContent(
        sidebarCopy.clipboard.copied
      )
    );
  });

  it('reports a clipboard refusal where errors render, not as success', async () => {
    const user = userEvent.setup();
    spyClipboard().mockRejectedValueOnce(new Error('denied'));
    const view = renderTeams();
    await user.click(screen.getByRole('button', { name: 'Analysis Lab options' }));
    const menu = await screen.findByRole('menu');
    await user.click(within(menu).getByRole('menuitem', { name: sidebarCopy.teamMenu.copyId }));
    await waitFor(() =>
      expect(view.controller.reportError).toHaveBeenCalledWith(
        sidebarCopy.clipboard.failed,
        'global'
      )
    );
    expect(document.querySelector('[data-crew-sidebar-announcer]')).toHaveTextContent('');
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

  it('opens on right-click (and so on Shift+F10 and the Menu key) with the copy items', async () => {
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
