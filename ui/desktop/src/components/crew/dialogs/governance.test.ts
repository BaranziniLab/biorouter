import { describe, expect, it } from 'vitest';
import type { Snapshot } from '../crewApi';
import {
  DELETE_CAPABILITY,
  hasJoinable,
  isOpenWorkspace,
  managesChannel,
  managesTeam,
  mayDeleteChannel,
  mayDeleteTeam,
  mayEditContext,
  NARROW_RUN_CHANNELS,
  runChannelLimit,
  WIDE_RUN_CHANNELS,
  WIDE_RUNS_CAPABILITY,
  WORKSPACE_CONTEXT_CAPABILITY,
} from './governance';
import { alice, bob, carol, makeSnapshot } from './dialogsTestHarness';

/** The fixture workspace (Alice hosts and created Analysis Lab), seen by `actor`, governed so. */
function seenBy(
  actor: typeof alice | typeof bob | typeof carol,
  governance?: 'open' | 'managed',
  extra: Partial<Snapshot> = {}
): Snapshot {
  const base = makeSnapshot();
  return {
    ...base,
    actor,
    workspace: { ...base.workspace, ...(governance ? { governance } : {}) },
    channels: [
      ...base.channels,
      {
        id: 'channel-methods',
        team_id: 'team-1',
        name: 'methods',
        created_by: carol.id,
        owner_id: carol.id,
        members: [alice.id, bob.id, carol.id],
        archived: false,
        classification: 'restricted',
      },
    ],
    ...extra,
  };
}

const methods = (snapshot: Snapshot) =>
  snapshot.channels.find((channel) => channel.id === 'channel-methods') ?? null;
const general = (snapshot: Snapshot) =>
  snapshot.channels.find((channel) => channel.id === 'channel-general') ?? null;
const lab = (snapshot: Snapshot) => snapshot.teams[0] ?? null;
const deleting = { capabilities: [DELETE_CAPABILITY], viewerIsHost: false };

describe('governance gates (OW-1, OW-2, OW-4)', () => {
  it('reads a workspace without the field as managed, today’s rules', () => {
    expect(isOpenWorkspace(seenBy(bob))).toBe(false);
    expect(isOpenWorkspace(seenBy(bob, 'managed'))).toBe(false);
    expect(isOpenWorkspace(seenBy(bob, 'open'))).toBe(true);
    expect(isOpenWorkspace(null)).toBe(false);
  });

  it('lets only a team’s creator and a channel’s owner manage them in a managed workspace', () => {
    const asBob = seenBy(bob);
    expect(managesTeam(asBob, lab(asBob))).toBe(false);
    expect(managesChannel(asBob, methods(asBob))).toBe(false);
    const asCarol = seenBy(carol);
    expect(managesChannel(asCarol, methods(asCarol))).toBe(true);
    const asAlice = seenBy(alice);
    expect(managesTeam(asAlice, lab(asAlice))).toBe(true);
    // The host is no exception to a channel someone else owns.
    expect(managesChannel(asAlice, methods(asAlice))).toBe(false);
  });

  it('lets every member manage the teams and channels they are in once the workspace is open', () => {
    const asBob = seenBy(bob, 'open');
    expect(managesTeam(asBob, lab(asBob))).toBe(true);
    expect(managesChannel(asBob, methods(asBob))).toBe(true);
    // Not a channel Dan is not in, nor a team he is not in.
    const outsider = seenBy(
      { id: 'person-dan', uid: 1003, username: 'dan', nickname: 'Dan' } as typeof bob,
      'open'
    );
    expect(managesChannel(outsider, methods(outsider))).toBe(false);
    expect(managesTeam(outsider, lab(outsider))).toBe(false);
  });

  it('offers Delete channel only on a delete_v1 broker, and never for #general', () => {
    const asCarol = seenBy(carol);
    expect(mayDeleteChannel(asCarol, methods(asCarol), { viewerIsHost: false })).toBe(false);
    expect(mayDeleteChannel(asCarol, methods(asCarol), deleting)).toBe(true);
    const asAlice = seenBy(alice, 'open');
    expect(mayDeleteChannel(asAlice, general(asAlice), deleting)).toBe(false);
  });

  it('lets the owner delete a channel in a managed workspace, the host only an orphaned one', () => {
    const asBob = seenBy(bob);
    expect(mayDeleteChannel(asBob, methods(asBob), deleting)).toBe(false);
    const asHost = seenBy(alice);
    expect(mayDeleteChannel(asHost, methods(asHost), { ...deleting, viewerIsHost: true })).toBe(
      false
    );
    const orphaned = seenBy(alice, 'managed', {
      principals: [alice, bob, { ...carol, active: false }],
    });
    expect(mayDeleteChannel(orphaned, methods(orphaned), { ...deleting, viewerIsHost: true })).toBe(
      true
    );
  });

  it('lets any member of the channel delete it in an open workspace, archived or not', () => {
    const asBob = seenBy(bob, 'open');
    expect(mayDeleteChannel(asBob, methods(asBob), deleting)).toBe(true);
    const archived = { ...methods(asBob)!, archived: true };
    expect(mayDeleteChannel(asBob, archived, deleting)).toBe(true);
  });

  it('lets the creator delete a team, the host when the creator left, any member when open', () => {
    const asAlice = seenBy(alice);
    expect(mayDeleteTeam(asAlice, lab(asAlice), deleting)).toBe(true);
    const asBob = seenBy(bob);
    expect(mayDeleteTeam(asBob, lab(asBob), deleting)).toBe(false);
    expect(mayDeleteTeam(asBob, lab(asBob), { ...deleting, viewerIsHost: true })).toBe(false);
    const left = seenBy(bob, 'managed', { principals: [{ ...alice, active: false }, bob, carol] });
    expect(mayDeleteTeam(left, lab(left), { ...deleting, viewerIsHost: true })).toBe(true);
    const open = seenBy(bob, 'open');
    expect(mayDeleteTeam(open, lab(open), deleting)).toBe(true);
    expect(mayDeleteTeam(open, lab(open), { viewerIsHost: false })).toBe(false);
  });

  it('lets anyone edit the context in an open workspace, the host alone in a managed one', () => {
    const capabilities = [WORKSPACE_CONTEXT_CAPABILITY];
    expect(mayEditContext(seenBy(bob), { capabilities, viewerIsHost: false })).toBe(false);
    expect(mayEditContext(seenBy(alice), { capabilities, viewerIsHost: true })).toBe(true);
    expect(mayEditContext(seenBy(bob, 'open'), { capabilities, viewerIsHost: false })).toBe(true);
    expect(mayEditContext(seenBy(alice), { capabilities: [], viewerIsHost: true })).toBe(false);
  });

  it('finds something to browse only in an open workspace that lists it', () => {
    const joinable = {
      joinable_channels: [{ id: 'channel-raw', team_id: 'team-1', name: 'raw' }],
      joinable_teams: [{ id: 'team-2', name: 'Imaging' }],
    };
    expect(hasJoinable(seenBy(bob, 'managed', joinable))).toBe(false);
    expect(hasJoinable(seenBy(bob, 'open', joinable))).toBe(true);
    expect(hasJoinable(seenBy(bob, 'open', joinable), 'team-1')).toBe(true);
    expect(hasJoinable(seenBy(bob, 'open', joinable), 'team-2')).toBe(false);
    expect(hasJoinable(seenBy(bob, 'open'))).toBe(false);
  });

  it('lets a task read 100 channels on a wide_runs_v1 broker, 20 on an older one', () => {
    expect(runChannelLimit([WIDE_RUNS_CAPABILITY])).toBe(WIDE_RUN_CHANNELS);
    expect(WIDE_RUN_CHANNELS).toBe(100);
    expect(runChannelLimit(null)).toBe(NARROW_RUN_CHANNELS);
    expect(NARROW_RUN_CHANNELS).toBe(20);
  });
});
