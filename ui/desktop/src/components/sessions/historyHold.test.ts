import { describe, expect, it } from 'vitest';
import type { Session } from '../../api';
import type { DateGroup } from '../../utils/dateUtils';
import { applyHeldHistory, holdHistory } from './historyHold';

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
