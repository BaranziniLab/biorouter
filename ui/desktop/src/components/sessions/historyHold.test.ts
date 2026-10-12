import { describe, expect, it } from 'vitest';
import type { Session } from '../../api';
import type { DateGroup } from '../../utils/dateUtils';
import {
  applyHeldHistory,
  holdHistory,
  liftHeldRow,
  OWN_MOVE_TTL_MS,
  pendingOwnMoves,
  type PendingOwnMove,
} from './historyHold';

const chat = (id: string, name = id): Session => ({ id, name }) as Session;
const group = (key: string, ...sessions: Session[]): DateGroup => ({
  key,
  label: key,
  sessions,
});
const ids = (groups: DateGroup[]) =>
  groups.map((g) => `${g.key}: ${g.sessions.map((s) => s.id).join(' ')}`);
const NO_CHILDREN = new Map<string, Session[]>();

describe('holding Chat history still while the pointer is over it (principle 9)', () => {
  const a = chat('a');
  const b = chat('b');
  const c = chat('c');
  const held = holdHistory([group('today', a, b), group('week', c)], NO_CHILDREN);

  it('shows the live list unchanged when nothing moved', () => {
    const live = [group('today', a, b), group('week', c)];
    const { groups } = applyHeldHistory(held, live, NO_CHILDREN);
    expect(ids(groups)).toEqual(['today: a b', 'week: c']);
  });

  it('keeps the order while a chat that arrived at the top waits', () => {
    const fresh = chat('new');
    const live = [group('today', fresh, a, b), group('week', c)];
    const { groups } = applyHeldHistory(held, live, NO_CHILDREN);
    expect(ids(groups)).toEqual(['today: a b', 'week: c']);
  });

  it('keeps the order while a chat that was resumed jumps ahead', () => {
    const live = [group('today', b, a), group('week', c)];
    const { groups } = applyHeldHistory(held, live, NO_CHILDREN);
    expect(ids(groups)).toEqual(['today: a b', 'week: c']);
  });

  it('lets a rename through, because it shifts nothing', () => {
    const renamed = { ...b, name: 'Renamed' } as Session;
    const live = [group('today', a, renamed), group('week', c)];
    const { groups } = applyHeldHistory(held, live, NO_CHILDREN);
    expect(groups[0].sessions[1].name).toBe('Renamed');
  });

  it('lets a removal through and drops a bucket that empties', () => {
    const live = [group('today', a, b)];
    const { groups } = applyHeldHistory(held, live, NO_CHILDREN);
    expect(ids(groups)).toEqual(['today: a b']);
    const none = applyHeldHistory(held, [group('today', a)], NO_CHILDREN).groups;
    expect(ids(none)).toEqual(['today: a']);
  });

  it('lets the next page arrive below every held row, into the bucket it belongs to', () => {
    const d = chat('d');
    const e = chat('e');
    const live = [group('today', a, b), group('week', c, d), group('month30', e)];
    const { groups } = applyHeldHistory(held, live, NO_CHILDREN);
    expect(ids(groups)).toEqual(['today: a b', 'week: c d', 'month30: e']);
  });

  it('does not repeat a bucket key when a held bucket reappears below', () => {
    const d = chat('d');
    const early = holdHistory([group('today', a)], NO_CHILDREN);
    const live = [group('today', a), group('week', d)];
    const { groups } = applyHeldHistory(early, live, NO_CHILDREN);
    expect(ids(groups)).toEqual(['today: a', 'week: d']);
    const sameBucket = applyHeldHistory(early, [group('today', a, d)], NO_CHILDREN).groups;
    expect(ids(sameBucket)).toEqual(['today: a d']);
  });
});

describe('subagent runs nested under a held parent', () => {
  const parent = chat('parent');
  const run1 = chat('run1');
  const run2 = chat('run2');
  const other = chat('other');
  const withRun = new Map([[parent.id, [run1]]]);
  const held = holdHistory([group('today', parent, other)], withRun);

  it('keeps the runs it had, and holds back one that started under the pointer', () => {
    const live = new Map([[parent.id, [run1, run2]]]);
    const { children } = applyHeldHistory(held, [group('today', parent, other)], live);
    expect(children.get(parent.id)?.map((s) => s.id)).toEqual(['run1']);
  });

  it('drops a run that was removed', () => {
    const { children } = applyHeldHistory(held, [group('today', parent, other)], NO_CHILDREN);
    expect(children.has(parent.id)).toBe(false);
  });

  it('holds back a first run under a parent that had none', () => {
    const live = new Map([[other.id, [run2]]]);
    const { children } = applyHeldHistory(held, [group('today', parent, other)], live);
    expect(children.has(other.id)).toBe(false);
  });

  it('brings a parent that arrived after the pointer, with its runs', () => {
    const late = chat('late');
    const live = new Map([[late.id, [run2]]]);
    const { groups, children } = applyHeldHistory(
      held,
      [group('today', parent, other, late)],
      live
    );
    expect(ids(groups)).toEqual(['today: parent other late']);
    expect(children.get(late.id)?.map((s) => s.id)).toEqual(['run2']);
  });
});

describe("a person's own rename lifted in the held order (R-30, ruling 16)", () => {
  const a = chat('a');
  const b = chat('b');
  const c = chat('c');
  const today = { key: 'today', label: 'Today' };
  const heldIds = (held: ReturnType<typeof holdHistory>) =>
    held.groups.map((g) => `${g.key}: ${g.ids.join(' ')}`);

  it('puts the row first in its bucket and still holds everything else', () => {
    const held = holdHistory([group('today', a, b), group('week', c)], NO_CHILDREN);
    expect(heldIds(liftHeldRow(held, { id: 'b', ...today }))).toEqual(['today: b a', 'week: c']);
  });

  it('opens a Today bucket for a row that was in an older one, and drops the bucket it empties', () => {
    const held = holdHistory([group('week', c)], NO_CHILDREN);
    expect(heldIds(liftHeldRow(held, { id: 'c', ...today }))).toEqual(['today: c']);
    const more = holdHistory([group('week', c, a)], NO_CHILDREN);
    expect(heldIds(liftHeldRow(more, { id: 'a', ...today }))).toEqual(['today: a', 'week: c']);
  });

  it('shows the lifted row at once even before the live list has moved it', () => {
    const held = holdHistory([group('today', a, b), group('week', c)], NO_CHILDREN);
    const lifted = liftHeldRow(held, { id: 'c', ...today });
    // The live list still has the old order and buckets.
    const { groups } = applyHeldHistory(
      lifted,
      [group('today', a, b), group('week', c)],
      NO_CHILDREN
    );
    expect(ids(groups)).toEqual(['today: c a b']);
  });

  it('leaves a chat that arrived while the pointer was here waiting', () => {
    const held = holdHistory([group('today', a, b)], NO_CHILDREN);
    const lifted = liftHeldRow(held, { id: 'b', ...today });
    const fresh = chat('fresh');
    const { groups } = applyHeldHistory(lifted, [group('today', fresh, b, a)], NO_CHILDREN);
    expect(ids(groups)).toEqual(['today: b a']);
  });

  it('returns the same hold for a row already first, or one it does not hold', () => {
    const held = holdHistory([group('today', a, b)], NO_CHILDREN);
    expect(liftHeldRow(held, { id: 'a', ...today })).toBe(held);
    expect(liftHeldRow(held, { id: 'nobody', ...today })).toBe(held);
  });

  it("puts a renamed run first among its parent's runs and moves nothing else", () => {
    const parent = chat('parent');
    const withRuns = new Map([[parent.id, [chat('run1'), chat('run2'), chat('run3')]]]);
    const held = holdHistory([group('today', parent, a)], withRuns);
    const lifted = liftHeldRow(held, { id: 'run3', ...today });
    expect(lifted.children.parent).toEqual(['run3', 'run1', 'run2']);
    expect(heldIds(lifted)).toEqual(['today: parent a']);
    expect(liftHeldRow(lifted, { id: 'run3', ...today })).toBe(lifted);
  });
});

describe('own moves waiting for the live list', () => {
  const entry = (id: string, at: number): PendingOwnMove => ({
    id,
    key: 'today',
    label: 'Today',
    activityAt: new Date(at).toISOString(),
    at,
  });

  it('are reported until they are older than the bound, and then forgotten', () => {
    const moves = new Map([
      ['fresh', entry('fresh', 10_000)],
      ['stale', entry('stale', 1_000)],
    ]);
    const now = 1_000 + OWN_MOVE_TTL_MS + 1;
    expect(pendingOwnMoves(moves, now).map((move) => move.id)).toEqual(['fresh']);
    expect([...moves.keys()]).toEqual(['fresh']);
  });
});
