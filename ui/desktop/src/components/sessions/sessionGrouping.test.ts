import { describe, expect, it } from 'vitest';
import type { SessionSummary } from '../../api';
import { sortSidebarChats } from '../BioRouterSidebar/sidebarChatView';
import { groupSessionsByParent, sortByActivity, withoutSubagents } from './sessionGrouping';

describe('sortByActivity (V1-R2-8)', () => {
  const tied = '2026-09-05T17:49:17Z';
  const rows = [
    { id: 'b-2', updated_at: tied, created_at: '2026-08-01T00:00:00Z' },
    { id: 'a-9', updated_at: '2026-09-05T18:00:00Z', created_at: '2026-09-05T18:00:00Z' },
    { id: 'c-1', updated_at: tied, created_at: '2026-09-01T00:00:00Z' },
    { id: 'a-1', updated_at: tied, created_at: '2026-09-04T00:00:00Z' },
    { id: 'z-0', updated_at: '2026-08-30T09:00:00Z', created_at: '2026-08-30T09:00:00Z' },
  ];

  it('is newest activity first, then id ascending among chats that share the stamp', () => {
    expect(sortByActivity(rows).map((r) => r.id)).toEqual(['a-9', 'a-1', 'b-2', 'c-1', 'z-0']);
  });

  it('gives the same answer whatever order the daemon returned', () => {
    const forward = sortByActivity(rows).map((r) => r.id);
    expect(sortByActivity([...rows].reverse()).map((r) => r.id)).toEqual(forward);
    expect(sortByActivity([rows[2], rows[4], rows[0], rows[3], rows[1]]).map((r) => r.id)).toEqual(
      forward
    );
  });

  it('does not touch its input', () => {
    const input = [...rows];
    sortByActivity(input);
    expect(input).toEqual(rows);
  });

  it('reads created_at when updated_at is missing or unreadable', () => {
    const odd = [
      { id: 'x', updated_at: 'nope', created_at: '2026-09-05T11:00:00Z' },
      { id: 'y', updated_at: null, created_at: '2026-09-05T12:00:00Z' },
      { id: 'w', updated_at: '2026-09-05T10:00:00Z', created_at: '2026-09-01T00:00:00Z' },
    ];
    expect(sortByActivity(odd).map((r) => r.id)).toEqual(['y', 'x', 'w']);
  });

  /**
   * The point of V1-R2-8: History and the sidebar are the same data in the same order. The
   * sidebar's own sort is the reference, so a change to either tie-break fails here.
   */
  it('orders exactly as the sidebar orders its chats under Last activity', () => {
    const sidebar = sortSidebarChats(rows as unknown as SessionSummary[], 'activity').map(
      (r) => r.id
    );
    expect(sortByActivity(rows).map((r) => r.id)).toEqual(sidebar);
  });
});

describe('groupSessionsByParent', () => {
  it('nests sub_agent rows under their parent and keeps orphans top-level', () => {
    const rows = [
      { id: 'p1', session_type: 'user' },
      { id: 'c1', session_type: 'sub_agent', parent_session_id: 'p1' },
      { id: 'c2', session_type: 'sub_agent', parent_session_id: 'gone' },
    ] as const;
    const grouped = groupSessionsByParent([...rows]);
    const parent = grouped.find((g) => g.session.id === 'p1');
    expect(parent?.children.map((c) => c.id)).toEqual(['c1']);
    // A child whose parent is not in the page still shows (top-level, badged).
    expect(grouped.some((g) => g.session.id === 'c2')).toBe(true);
  });

  // Only one level of nesting is rendered, so a grandchild has to come back to
  // top level. Hanging it off a row that is itself a child means nothing ever
  // scans it for children and the row disappears from History entirely —
  // contradicting the promise that nothing becomes unreachable. The backend
  // refuses nesting inside a delegation tree today, so this is latent; the
  // helper should not silently depend on an invariant enforced elsewhere.
  it('renders every row exactly once when the page holds a nested chain', () => {
    const rows = [
      { id: 'p1', session_type: 'user' },
      { id: 'c1', session_type: 'sub_agent', parent_session_id: 'p1' },
      { id: 'g1', session_type: 'sub_agent', parent_session_id: 'c1' },
    ];

    const rendered = groupSessionsByParent(rows)
      .flatMap(({ session, children }) => [session.id, ...children.map((c) => c.id)])
      .sort();

    expect(rendered).toEqual(['c1', 'g1', 'p1']);
  });

  it('does not loop or drop rows when parent links form a cycle', () => {
    const rows = [
      { id: 'a', session_type: 'sub_agent', parent_session_id: 'b' },
      { id: 'b', session_type: 'sub_agent', parent_session_id: 'a' },
    ];

    const rendered = groupSessionsByParent(rows)
      .flatMap(({ session, children }) => [session.id, ...children.map((c) => c.id)])
      .sort();

    expect(rendered).toEqual(['a', 'b']);
  });
});

describe('withoutSubagents', () => {
  it('drops sub_agent rows and preserves the array identity when there are none', () => {
    const clean = [{ id: 'a', session_type: 'user' }];
    // Identity matters: React state setters and memo consumers bail out on an
    // unchanged reference, so a fresh array here costs a render pass per
    // refresh on every surface that filters.
    expect(withoutSubagents(clean)).toBe(clean);

    const mixed = [...clean, { id: 'b', session_type: 'sub_agent', parent_session_id: 'a' }];
    expect(withoutSubagents(mixed).map((r) => r.id)).toEqual(['a']);
  });
});
