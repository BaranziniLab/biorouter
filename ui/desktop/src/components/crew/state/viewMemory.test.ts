import { describe, expect, it } from 'vitest';
import type { Channel, CrewMessage, Snapshot } from '../crewApi';
import { forgetConnectionMemory } from './useCrewConnections';
import {
  forgetRememberedView,
  MAX_REMEMBERED_CHANNELS,
  MAX_REMEMBERED_CONNECTIONS,
  rememberedPaneIntent,
  rememberedView,
  rememberedViewCount,
  rememberedViewMoved,
  rememberPaneIntent,
  rememberVerifiedView,
} from './viewMemory';
import type { VerifiedView } from './types';

/**
 * The view kept across leaving Crew (live QA round 4, Q4-04; SECURITY-SENSITIVE): presentation
 * only, memory only, and never kept under a privacy or classification it was not verified under.
 */

const general: Channel = {
  id: 'channel-general',
  team_id: 'team-1',
  name: 'general',
  created_by: 'person-1',
  owner_id: 'person-1',
  members: ['person-1'],
  archived: false,
  classification: 'restricted',
};
const methods: Channel = {
  ...general,
  id: 'channel-methods',
  name: 'methods',
  classification: 'public_safe',
};

function snapshot(overrides: Partial<Snapshot> = {}): Snapshot {
  return {
    workspace: {
      id: 'workspace-1',
      host_uid: 1000,
      mode: 'private',
      institution_id: 'ucsf',
      policy_epoch: 1,
    },
    actor: { id: 'person-1', uid: 1000, username: 'alice', nickname: 'Alice' },
    principals: [],
    teams: [
      {
        id: 'team-1',
        name: 'Lab',
        created_by: 'person-1',
        members: ['person-1'],
        general_channel_id: general.id,
      },
    ],
    channels: [general, methods],
    invitations: [],
    runs: [],
    ...overrides,
  } as Snapshot;
}

function message(id: string, channelId = general.id): CrewMessage {
  return {
    id,
    sequence: `sequence-${id}`,
    channel_id: channelId,
    actor_id: 'person-1',
    body: `message ${id}`,
    created_at: 1_700_000_000,
    restricted: false,
    source_channels: [channelId],
    attachments: [],
  };
}

function view(overrides: Partial<VerifiedView> = {}): VerifiedView {
  return {
    connectionId: 'conn-1',
    snapshot: snapshot(),
    observedPrivacy: {
      connectionId: 'conn-1',
      mode: 'private',
      institutionId: 'ucsf',
      policyEpoch: 1,
    },
    runs: [],
    labels: null,
    teamId: 'team-1',
    channelId: general.id,
    messages: [],
    people: null,
    ...overrides,
  };
}

const page = (...ids: string[]) => ({ messages: ids.map((id) => message(id)), people: null });

describe('the view kept across leaving Crew', () => {
  it('hands back the last view with its channel’s loaded page, and nothing without one', () => {
    rememberVerifiedView(view(), null);
    // No loaded page of #general: a view without it would claim the channel is empty.
    expect(rememberedView('conn-1')).toBeNull();

    rememberVerifiedView(view(), page('a', 'b'));
    const kept = rememberedView('conn-1');
    expect(kept?.channelId).toBe(general.id);
    expect(kept?.messages.map((item) => item.id)).toEqual(['a', 'b']);

    // A later view without a page keeps the page it had.
    rememberVerifiedView(view({ runs: [] }), null);
    expect(rememberedView('conn-1')?.messages).toHaveLength(2);
    expect(rememberedView('conn-2')).toBeNull();
  });

  it('keeps each channel’s own page, and hands back the one of the channel last shown', () => {
    rememberVerifiedView(view(), page('a'));
    rememberVerifiedView(view({ channelId: methods.id }), {
      messages: [message('m', methods.id)],
      people: null,
    });
    expect(rememberedView('conn-1')?.messages.map((item) => item.id)).toEqual(['m']);
    rememberVerifiedView(view(), null);
    expect(rememberedView('conn-1')?.messages.map((item) => item.id)).toEqual(['a']);
  });

  it.each([
    [
      'the workspace mode',
      view({ snapshot: snapshot({ workspace: { ...snapshot().workspace, mode: 'public' } }) }),
    ],
    [
      'the workspace institution',
      view({
        snapshot: snapshot({ workspace: { ...snapshot().workspace, institution_id: 'ucla' } }),
      }),
    ],
    [
      'this connection’s mode',
      view({ observedPrivacy: { ...view().observedPrivacy, mode: 'public' } }),
    ],
    [
      'this connection’s policy epoch',
      view({ observedPrivacy: { ...view().observedPrivacy, policyEpoch: 2 } }),
    ],
    [
      'this connection’s institution',
      view({ observedPrivacy: { ...view().observedPrivacy, institutionId: 'ucla' } }),
    ],
  ])('drops every page kept under another privacy: %s', (_what, moved) => {
    rememberVerifiedView(view(), page('a'));
    rememberVerifiedView(view({ channelId: methods.id }), {
      messages: [message('m', methods.id)],
      people: null,
    });
    rememberVerifiedView({ ...moved, channelId: general.id }, null);
    expect(rememberedView('conn-1')).toBeNull();
  });

  it('drops the page of a channel whose classification moved, or which is gone', () => {
    rememberVerifiedView(view(), page('a'));
    rememberVerifiedView(view({ channelId: methods.id }), {
      messages: [message('m', methods.id)],
      people: null,
    });
    // #general is public_safe now; #methods is unchanged.
    rememberVerifiedView(
      view({
        channelId: methods.id,
        snapshot: snapshot({
          channels: [{ ...general, classification: 'public_safe' as const }, methods],
        }),
      }),
      null
    );
    expect(rememberedView('conn-1')?.messages.map((item) => item.id)).toEqual(['m']);
    rememberVerifiedView(view({ snapshot: snapshot({ channels: [general, methods] }) }), null);
    expect(rememberedView('conn-1')).toBeNull();

    // #methods is no longer offered.
    rememberVerifiedView(view({ channelId: methods.id }), {
      messages: [message('m', methods.id)],
      people: null,
    });
    rememberVerifiedView(view({ snapshot: snapshot({ channels: [general] }) }), null);
    rememberVerifiedView(view({ channelId: methods.id }), null);
    expect(rememberedView('conn-1')).toBeNull();
  });

  it('says a fresh view moved when its privacy or the channel’s classification did', () => {
    const kept = view();
    expect(rememberedViewMoved(kept, view(), general.id)).toBe(false);
    expect(
      rememberedViewMoved(
        kept,
        view({ observedPrivacy: { ...kept.observedPrivacy, mode: 'public' } }),
        general.id
      )
    ).toBe(true);
    expect(
      rememberedViewMoved(
        kept,
        view({
          snapshot: snapshot({
            channels: [{ ...general, classification: 'public_safe' as const }],
          }),
        }),
        general.id
      )
    ).toBe(true);
    expect(
      rememberedViewMoved(kept, view({ snapshot: snapshot({ channels: [methods] }) }), general.id)
    ).toBe(true);
    // Another channel moving says nothing about this one.
    expect(
      rememberedViewMoved(
        kept,
        view({
          snapshot: snapshot({
            channels: [general, { ...methods, classification: 'restricted' as const }],
          }),
        }),
        general.id
      )
    ).toBe(false);
  });

  it('is bounded: channels per connection, and connections', () => {
    for (let index = 0; index < MAX_REMEMBERED_CHANNELS + 5; index += 1) {
      const id = `channel-${index}`;
      const channels = [general, { ...general, id }];
      rememberVerifiedView(view({ channelId: id, snapshot: snapshot({ channels }) }), {
        messages: [message(`m${index}`, id)],
        people: null,
      });
    }
    // The oldest channels' pages went first; the last one's is kept.
    rememberVerifiedView(view(), page('a'));
    expect(rememberedView('conn-1')?.messages.map((item) => item.id)).toEqual(['a']);

    for (let index = 0; index < MAX_REMEMBERED_CONNECTIONS + 3; index += 1) {
      const connectionId = `conn-${index + 10}`;
      rememberVerifiedView(
        view({ connectionId, observedPrivacy: { ...view().observedPrivacy, connectionId } }),
        page('a')
      );
    }
    expect(rememberedViewCount()).toBe(MAX_REMEMBERED_CONNECTIONS);
    expect(rememberedView('conn-10')).toBeNull();
  });

  it('forgets a connection’s view on request, and everything of a removed connection', () => {
    rememberVerifiedView(view(), page('a'));
    forgetRememberedView('conn-1');
    expect(rememberedView('conn-1')).toBeNull();

    rememberVerifiedView(view(), page('a'));
    rememberPaneIntent('conn-1', { mode: 'details', tab: 'members' });
    forgetConnectionMemory('conn-1');
    expect(rememberedView('conn-1')).toBeNull();
    expect(rememberedPaneIntent('conn-1')).toBeNull();
  });

  it('hands back copies: changing one never changes what is kept', () => {
    rememberVerifiedView(view(), page('a'));
    rememberedView('conn-1')!.messages.push(message('injected'));
    expect(rememberedView('conn-1')?.messages.map((item) => item.id)).toEqual(['a']);
  });
});

describe('the details pane kept across leaving Crew', () => {
  it('keeps only a details pane, per connection, until the person closes it', () => {
    rememberPaneIntent('conn-1', { mode: 'details', tab: 'members' });
    expect(rememberedPaneIntent('conn-1')).toEqual({ mode: 'details', tab: 'members' });
    expect(rememberedPaneIntent('conn-2')).toBeNull();

    // Ask my agent and Chat access are about a moment, not a place: not kept.
    rememberPaneIntent('conn-1', { mode: 'agent' });
    expect(rememberedPaneIntent('conn-1')).toBeNull();
    rememberPaneIntent('conn-1', { mode: 'details', tab: 'files' });
    rememberPaneIntent('conn-1', null);
    expect(rememberedPaneIntent('conn-1')).toBeNull();
  });
});
