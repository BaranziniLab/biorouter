import { renderHook } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { Snapshot } from '../crewApi';
import { personLabel } from './personLabel';
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
});
