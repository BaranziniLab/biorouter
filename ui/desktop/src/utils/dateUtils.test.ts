import { describe, expect, it } from 'vitest';
import type { Session } from '../api';
import { groupSessionsByDate } from './dateUtils';

const NOW = new Date(2026, 9, 9, 15, 0, 0).getTime(); // 9 Oct 2026, 15:00 local

function session(id: string, updated: Date): Session {
  return {
    id,
    name: id,
    working_dir: '/tmp',
    created_at: updated.toISOString(),
    updated_at: updated.toISOString(),
    message_count: 1,
    extension_data: {},
  } as Session;
}

describe('groupSessionsByDate', () => {
  it('uses the coarse buckets the sidebar uses, newest first', () => {
    const groups = groupSessionsByDate(
      [
        session('a', new Date(2026, 9, 9, 9)),
        session('b', new Date(2026, 9, 8, 23, 50)),
        session('c', new Date(2026, 9, 5)),
        session('d', new Date(2026, 8, 20)),
        session('e', new Date(2026, 7, 2)),
        session('f', new Date(2025, 7, 2)),
      ],
      NOW
    );
    expect(groups.map((group) => group.label)).toEqual([
      'Today',
      'Yesterday',
      'Previous 7 days',
      'Previous 30 days',
      'August',
      'August 2025',
    ]);
    expect(groups.map((group) => group.sessions.map((s) => s.id))).toEqual([
      ['a'],
      ['b'],
      ['c'],
      ['d'],
      ['e'],
      ['f'],
    ]);
  });

  it('keeps the arrival order inside a bucket and never makes one heading per day', () => {
    const groups = groupSessionsByDate(
      [
        session('x', new Date(2026, 9, 6)),
        session('y', new Date(2026, 9, 4)),
        session('z', new Date(2026, 9, 3)),
      ],
      NOW
    );
    expect(groups).toHaveLength(1);
    expect(groups[0].key).toBe('week');
    expect(groups[0].sessions.map((s) => s.id)).toEqual(['x', 'y', 'z']);
  });
});
