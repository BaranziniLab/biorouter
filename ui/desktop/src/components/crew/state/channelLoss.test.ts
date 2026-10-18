import { describe, expect, it } from 'vitest';
import type { Channel, Snapshot, Team } from '../crewApi';
import { channelLossReason, channelLossText, type ChannelLossView } from './channelLoss';
import { crewObservationCopy } from './copy';

const team: Team = {
  id: 'team-1',
  name: 'Lab',
  created_by: 'person-1',
  members: ['person-1'],
  general_channel_id: 'general',
};
const methods: Channel = {
  id: 'methods',
  team_id: 'team-1',
  name: 'methods',
  created_by: 'person-1',
  owner_id: 'person-2',
  members: ['person-1', 'person-2'],
  archived: false,
  classification: 'public_safe',
};
const general: Channel = { ...methods, id: 'general', name: 'general' };
const before: Pick<Snapshot, 'channels'> = { channels: [general, methods] };

function view(extra: Partial<ChannelLossView> = {}): ChannelLossView {
  return {
    workspace: { id: 'w', host_uid: 1000, mode: 'private', policy_epoch: 1 },
    teams: [team],
    channels: [general],
    ...extra,
  };
}
const open = (extra: Partial<ChannelLossView> = {}) =>
  view({
    workspace: { id: 'w', host_uid: 1000, mode: 'private', policy_epoch: 1, governance: 'open' },
    joinable_channels: [],
    joinable_teams: [],
    ...extra,
  });

describe('why the selected channel left the view (G7)', () => {
  it('reads a channel an open workspace lists nowhere, in a team still listed, as deleted', () => {
    expect(channelLossReason(before, open(), 'methods')).toBe('deleted');
  });

  it('reads a channel whose team left the view as deleted with its team, whoever manages', () => {
    // Nobody can leave a team or be removed from one, so a team leaves a member's view only
    // when it is deleted.
    const gone = { teams: [], channels: [] };
    expect(channelLossReason(before, view(gone), 'methods')).toBe('deleted');
    expect(channelLossReason(before, open(gone), 'methods')).toBe('deleted');
  });

  it('reads a channel an open workspace offers to join as one the person is no longer in', () => {
    const joinable = [{ id: 'methods', team_id: 'team-1', name: 'methods' }];
    expect(channelLossReason(before, open({ joinable_channels: joinable }), 'methods')).toBe(
      'left'
    );
    // Without an earlier view too: the offer alone proves the channel is there.
    expect(channelLossReason(null, open({ joinable_channels: joinable }), 'methods')).toBe('left');
  });

  it('cannot tell in a managed workspace, which lists no channel the person is not in', () => {
    expect(channelLossReason(before, view(), 'methods')).toBe('unknown');
    // An open workspace's broker that sent no joinable list says nothing either.
    expect(channelLossReason(before, open({ joinable_channels: undefined }), 'methods')).toBe(
      'unknown'
    );
  });

  it('proves nothing by what a list cut short leaves out', () => {
    expect(channelLossReason(before, open({ totals: { joinable_channels: 2 } }), 'methods')).toBe(
      'unknown'
    );
    expect(channelLossReason(before, open({ totals: { channels: 5 } }), 'methods')).toBe('unknown');
    expect(
      channelLossReason(before, view({ teams: [], channels: [], totals: { teams: 1 } }), 'methods')
    ).toBe('unknown');
    // A count the list meets is a whole list.
    expect(
      channelLossReason(
        before,
        open({ totals: { channels: 1, joinable_channels: 0, teams: 1 } }),
        'methods'
      )
    ).toBe('deleted');
  });

  it('proves nothing about an archived channel, which is never joinable', () => {
    const archived = { channels: [general, { ...methods, archived: true }] };
    expect(channelLossReason(archived, open(), 'methods')).toBe('unknown');
  });

  it('proves nothing without the view that still had the channel, or while it is still there', () => {
    expect(channelLossReason(null, open(), 'methods')).toBe('unknown');
    expect(channelLossReason({ channels: [general] }, open(), 'methods')).toBe('unknown');
    expect(channelLossReason(before, open({ channels: [general, methods] }), 'methods')).toBe(
      'unknown'
    );
    expect(channelLossReason(before, open(), '')).toBe('unknown');
  });

  it('does not infer a deleted team from one the person is offered to join', () => {
    const joinableTeam = [{ id: 'team-1', name: 'Lab' }];
    expect(
      channelLossReason(
        before,
        open({ teams: [], channels: [], joinable_teams: joinableTeam }),
        'methods'
      )
    ).toBe('unknown');
  });
});

describe('the sentence that closes it', () => {
  it('says deleted, no longer in, or what is true either way', () => {
    expect(channelLossText('deleted', '#methods')).toBe('#methods was deleted, so it was closed.');
    expect(channelLossText('left', '#methods')).toBe(
      'You’re no longer in #methods, so it was closed.'
    );
    expect(channelLossText('unknown', '#methods')).toBe(
      '#methods is no longer available to you, so it was closed.'
    );
    expect(channelLossText('deleted', null)).toBe(crewObservationCopy.channelDeleted);
    expect(channelLossText('left', null)).toBe(crewObservationCopy.channelLeft);
    expect(channelLossText('unknown', null)).toBe(crewObservationCopy.channelAccessLost);
  });

  it('never claims the person lost access when the channel may have been deleted', () => {
    for (const text of [
      channelLossText('unknown', '#methods'),
      channelLossText('unknown', null),
      crewObservationCopy.channelAccessChanged('#methods'),
    ])
      expect(text).not.toMatch(/no longer have access/);
  });
});
