import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { Snapshot } from '../crewApi';
import { BrowseChannelsDialog } from './BrowseChannelsDialog';
import { browseCopy } from './copy';
import { bob, makeSnapshot, renderWithCrew, requestsFor } from './dialogsTestHarness';

/** Bob's view of an open workspace with a team and a channel he could join. */
function openWorkspace(overrides: Partial<Snapshot> = {}): Snapshot {
  const snapshot = makeSnapshot({ actor: bob });
  return {
    ...snapshot,
    workspace: { ...snapshot.workspace, governance: 'open' },
    joinable_teams: [{ id: 'team-2', name: 'Imaging Core', member_count: 4 }],
    joinable_channels: [
      {
        id: 'channel-raw',
        team_id: 'team-1',
        name: 'raw-data',
        classification: 'restricted',
        member_count: 2,
      },
    ],
    ...overrides,
  };
}

function renderBrowse(
  options: Parameters<typeof renderWithCrew>[1] = {},
  props: { teamId?: string } = {}
) {
  const onClose = vi.fn();
  const view = renderWithCrew(<BrowseChannelsDialog onClose={onClose} {...props} />, options);
  return { ...view, onClose };
}

describe('BrowseChannelsDialog (OW-2)', () => {
  it('lists the teams and channels an open workspace offers, each with a named Join', async () => {
    renderBrowse({ snapshot: openWorkspace() });
    const dialog = await screen.findByRole('dialog', { name: browseCopy.title });
    const teams = within(dialog).getByRole('region', { name: browseCopy.teams });
    expect(within(teams).getByText('Imaging Core')).toBeInTheDocument();
    expect(teams).toHaveTextContent(browseCopy.members(4));
    expect(
      within(teams).getByRole('button', { name: browseCopy.joinTeamLabel('Imaging Core') })
    ).toBeInTheDocument();
    const channels = within(dialog).getByRole('region', { name: browseCopy.channels });
    expect(within(channels).getByText('#raw-data')).toBeInTheDocument();
    expect(channels).toHaveTextContent(browseCopy.channelIn('Analysis Lab'));
    expect(channels).toHaveTextContent(browseCopy.restricted);
    expect(
      within(channels).getByRole('button', { name: browseCopy.joinChannelLabel('#raw-data') })
    ).toBeInTheDocument();
  });

  it('joins a channel with channel.join, opens it, and closes', async () => {
    const { crew, onClose } = renderBrowse({ snapshot: openWorkspace() });
    await act(async () => {
      fireEvent.click(
        await screen.findByRole('button', { name: browseCopy.joinChannelLabel('#raw-data') })
      );
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'channel.join')).toEqual([{ channel_id: 'channel-raw' }])
    );
    expect(crew.selectChannel).toHaveBeenCalledWith('channel-raw');
    expect(onClose).toHaveBeenCalled();
  });

  it('joins a team with team.join and opens it', async () => {
    const { crew, onClose } = renderBrowse({ snapshot: openWorkspace() });
    await act(async () => {
      fireEvent.click(
        await screen.findByRole('button', { name: browseCopy.joinTeamLabel('Imaging Core') })
      );
    });
    await waitFor(() => expect(requestsFor(crew, 'team.join')).toEqual([{ team_id: 'team-2' }]));
    expect(crew.selectTeam).toHaveBeenCalledWith('team-2');
    expect(onClose).toHaveBeenCalled();
  });

  it('keeps every Join waiting while one is on its way (single flight)', async () => {
    let finish: () => void = () => {};
    const { crew } = renderBrowse({
      snapshot: openWorkspace(),
      request: () => new Promise<void>((resolve) => (finish = resolve)),
    });
    const team = await screen.findByRole('button', {
      name: browseCopy.joinTeamLabel('Imaging Core'),
    });
    const channel = screen.getByRole('button', { name: browseCopy.joinChannelLabel('#raw-data') });
    fireEvent.click(channel);
    await waitFor(() => expect(team).toBeDisabled());
    expect(channel).toBeDisabled();
    fireEvent.click(team);
    expect(requestsFor(crew, 'team.join')).toEqual([]);
    await act(async () => finish());
  });

  it('shows a refused join in the dialog, and stays', async () => {
    const { onClose } = renderBrowse({
      snapshot: openWorkspace(),
      request: () => {
        throw new Error('forbidden: workspace is not open');
      },
    });
    await act(async () => {
      fireEvent.click(
        await screen.findByRole('button', { name: browseCopy.joinChannelLabel('#raw-data') })
      );
    });
    expect(await screen.findByText('forbidden: workspace is not open')).toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();
  });

  it('lists one team’s channels when opened from that team', async () => {
    renderBrowse(
      {
        snapshot: openWorkspace({
          joinable_channels: [
            { id: 'channel-raw', team_id: 'team-1', name: 'raw-data' },
            { id: 'channel-atlas', team_id: 'team-9', name: 'atlas' },
          ],
        }),
      },
      { teamId: 'team-1' }
    );
    const dialog = await screen.findByRole('dialog', {
      name: browseCopy.titleTeam('Analysis Lab'),
    });
    expect(within(dialog).queryByRole('region', { name: browseCopy.teams })).toBeNull();
    expect(within(dialog).getByText('#raw-data')).toBeInTheDocument();
    expect(within(dialog).queryByText('#atlas')).toBeNull();
    // The team is in the title, so no row repeats it.
    expect(within(dialog).getByRole('region', { name: browseCopy.channels })).not.toHaveTextContent(
      browseCopy.channelIn('Analysis Lab')
    );
  });

  it('offers nothing to join in a managed workspace, whatever the snapshot lists', async () => {
    const managed = openWorkspace();
    renderBrowse({
      snapshot: { ...managed, workspace: { ...managed.workspace, governance: 'managed' } },
    });
    const dialog = await screen.findByRole('dialog', { name: browseCopy.title });
    expect(within(dialog).getByRole('status')).toHaveTextContent(browseCopy.nothing);
    expect(within(dialog).queryByRole('button', { name: /^Join/ })).toBeNull();
  });
});
