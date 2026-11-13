import { renderHook } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { Snapshot } from '../crewApi';
import { personLabel } from './personLabel';
import { sanitizeAvatarText } from './displayText';
import { buildPeopleDirectory, usePeopleDirectory } from './usePeopleDirectory';
import type { CrewPeopleMap, DaemonPersonLabels } from './types';

const ALICE = '11111111-1111-4111-8111-111111111111';
const BOB = '22222222-2222-4222-8222-222222222222';

/** A snapshot typed as the wire helpers type it, so the two shapes cannot drift apart unnoticed. */
const wireSnapshot: Snapshot = {
  workspace: { id: 'w', host_uid: 1000, mode: 'private', institution_id: 'ucsf', policy_epoch: 1 },
  actor: { id: ALICE, uid: 1000, username: 'alice', nickname: 'Alice Chen' },
  principals: [
    { id: ALICE, uid: 1000, username: 'alice', nickname: 'Alice Chen' },
    { id: BOB, uid: 1001, username: 'bob', nickname: 'Bob Lee' },
  ],
  teams: [],
  channels: [],
  invitations: [],
  runs: [],
};

describe('usePeopleDirectory', () => {
  it('reads the wire snapshot as it is typed today', () => {
    const { result } = renderHook(() => usePeopleDirectory(wireSnapshot));
    expect(personLabel(BOB, 'inline', result.current)).toBe('Bob Lee (@bob)');
    expect(result.current.viewerIsHost).toBe(true);
  });

  it('keeps one directory while its inputs are unchanged, and rebuilds when they change', () => {
    const { result, rerender } = renderHook(({ snapshot }) => usePeopleDirectory(snapshot), {
      initialProps: { snapshot: wireSnapshot },
    });
    const first = result.current;
    rerender({ snapshot: wireSnapshot });
    expect(result.current).toBe(first);
    rerender({ snapshot: { ...wireSnapshot, principals: [wireSnapshot.principals[0]] } });
    expect(result.current).not.toBe(first);
    expect(personLabel(BOB, 'inline', result.current)).toBe('Unknown member');
  });

  it('accepts the labels map as the controller forwards it, and reads only boolean verdicts', () => {
    const loose: Readonly<Record<string, unknown>> = {
      [ALICE]: { collides: 'true' },
      [BOB]: 'collides',
    };
    const directory = buildPeopleDirectory(wireSnapshot, loose);
    expect(directory.collides(ALICE)).toBe(false);
    expect(directory.collides(BOB)).toBe(false);

    const typed: DaemonPersonLabels = { [BOB]: { full: '', short: '', collides: true } };
    expect(buildPeopleDirectory(wireSnapshot, typed).collides(BOB)).toBe(true);
  });

  it('fills authors the snapshot does not name from a message result’s people map', () => {
    const people: CrewPeopleMap = {
      '33333333-3333-4333-8333-333333333333': {
        username: 'dan',
        display_name: 'Dan Wu',
        active: false,
      },
      [BOB]: { username: 'mallory', display_name: 'Not Bob' },
    };
    const directory = buildPeopleDirectory(wireSnapshot, null, people);
    expect(personLabel('33333333-3333-4333-8333-333333333333', 'inline', directory)).toBe(
      'Dan Wu (@dan) · former member'
    );
    // The verified snapshot wins over a people-map entry for the same principal.
    expect(personLabel(BOB, 'inline', directory)).toBe('Bob Lee (@bob)');
  });

  it('never calls an author a former member when that account is a member now (AO-1)', () => {
    // bob was let in again under a new principal; his earlier messages carry the old one.
    const OLD_BOB = '44444444-4444-4444-8444-444444444444';
    const people: CrewPeopleMap = {
      [OLD_BOB]: { username: 'bob', display_name: 'Bob Lee', active: false },
      '33333333-3333-4333-8333-333333333333': {
        username: 'dan',
        display_name: 'Dan Wu',
        active: false,
      },
    };
    const snapshot: Snapshot = {
      ...wireSnapshot,
      former_principals: [{ id: OLD_BOB, username: 'bob', display_name: 'Bob Lee' }],
    };
    const directory = buildPeopleDirectory(snapshot, null, people);
    expect(personLabel(OLD_BOB, 'inline', directory)).not.toMatch(/former member/);
    // Still a former principal for lists and actions: bob is listed once, as the active member.
    expect(directory.isFormer(OLD_BOB)).toBe(true);
    expect(directory.people.filter((person) => person.username === 'bob')).toHaveLength(1);
    // Someone who left and is not back is still said to be a former member.
    expect(personLabel('33333333-3333-4333-8333-333333333333', 'inline', directory)).toBe(
      'Dan Wu (@dan) · former member'
    );
  });
});

describe('avatars', () => {
  const cp = (...points: number[]) => String.fromCodePoint(...points);
  // A family emoji is three emoji joined by U+200D; a heart takes U+FE0F.
  const family = cp(0x1f468, 0x200d, 0x1f469, 0x200d, 0x1f467);
  const heart = cp(0x2764, 0xfe0f);

  it('keep emoji sequences intact', () => {
    expect(sanitizeAvatarText(family)).toBe(family);
    expect(sanitizeAvatarText(heart)).toBe(heart);
    expect(sanitizeAvatarText(' AC ')).toBe('AC');
  });

  it('lose direction controls and control characters', () => {
    expect(sanitizeAvatarText(`A${cp(0x202e)}C${cp(0x2066)}${cp(0x7)}`)).toBe('AC');
    expect(sanitizeAvatarText(null)).toBe('');
  });

  it('reach the directory sanitized', () => {
    const directory = buildPeopleDirectory({
      ...wireSnapshot,
      principals: [
        { id: BOB, uid: 1001, username: 'bob', nickname: 'Bob', avatar: `${cp(0x202e)}${heart}` },
      ],
    });
    expect(directory.byId(BOB)?.avatar).toBe(heart);
  });
});
