import { describe, expect, it } from 'vitest';
import type { Session } from '../api';
import { groupByChatDate } from './chatDateBuckets';
import { formatChatDay, formatChatTimestamp, groupSessionsByDate } from './dateUtils';

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

  /**
   * A store of 5,500 chats, most of them older than 30 days (vision V1-R3-2). Seeded, so a
   * failure reproduces; instants straddle midnight, the DST changes, the year line, the
   * future, and values that are not dates.
   */
  function manyChats(count: number): Session[] {
    let seed = 20261011;
    const random = () => {
      seed = (seed * 1_664_525 + 1_013_904_223) % 4_294_967_296;
      return seed / 4_294_967_296;
    };
    return Array.from({ length: count }, (_, index) => {
      const roll = random();
      if (roll < 0.01) return { ...session(`bad-${index}`, new Date(NOW)), updated_at: 'never' };
      const hoursAgo = roll < 0.15 ? random() * 24 * 30 : 24 * 30 + random() * 24 * 450;
      const instant = new Date(NOW - hoursAgo * 3_600_000 + (roll > 0.995 ? 6 * 3_600_000 : 0));
      return session(`chat-${index}`, instant);
    });
  }

  it('draws the same groups as the sidebar helper, whatever the instants', () => {
    const chats = manyChats(3000);
    // The date the grouping reads, then the dates a day on either side of "now".
    for (const now of [NOW, NOW + 20 * 3_600_000, new Date(2026, 2, 8, 2, 30).getTime()]) {
      const expected = groupByChatDate(chats, (chat) => chat.updated_at, now).map(
        ({ key, label, items }) => ({ key, label, sessions: items })
      );
      expect(groupSessionsByDate(chats, now)).toEqual(expected);
    }
  });

  it('asks for a date label once per calendar day, not once per chat', () => {
    // `chatDateBucket` builds an Intl.DateTimeFormat for every day older than 30; per chat
    // that was 115 ms of the time History took to draw 5,500 rows.
    const chats = manyChats(5500);
    const days = new Set(
      chats.map((chat) => {
        const date = new Date(chat.updated_at);
        return `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
      })
    );
    // Counted by a subclass: a spy on the constructor would not build the formatter.
    const Real = Intl.DateTimeFormat;
    let constructed = 0;
    class Counting extends Real {
      constructor(...args: ConstructorParameters<typeof Real>) {
        super(...args);
        constructed += 1;
      }
    }
    Intl.DateTimeFormat = Counting as typeof Intl.DateTimeFormat;
    try {
      groupSessionsByDate(chats, NOW);
    } finally {
      Intl.DateTimeFormat = Real;
    }
    expect(constructed).toBeLessThanOrEqual(days.size);
    expect(constructed).toBeLessThan(chats.length / 5);
  });

  it('puts every unreadable date in one Older group, last', () => {
    const groups = groupSessionsByDate(
      [
        { ...session('a', new Date(NOW)), updated_at: '' },
        session('b', new Date(2026, 9, 9, 9)),
        { ...session('c', new Date(NOW)), updated_at: 'sometime' },
      ],
      NOW
    );
    expect(groups.map((group) => [group.label, group.sessions.map((s) => s.id)])).toEqual([
      ['Today', ['b']],
      ['Older', ['a', 'c']],
    ]);
  });
});

/** ICU may put a narrow no-break space before AM/PM; the words are what is pinned. */
const plain = (text: string) => text.replace(/\s/g, ' ');

describe('formatChatDay', () => {
  it('reads the time for today, the day this year, and the day with its year before', () => {
    expect(plain(formatChatDay(new Date(2026, 9, 9, 14, 39).toISOString(), NOW))).toBe('2:39 PM');
    expect(formatChatDay(new Date(2026, 9, 8, 23, 50).toISOString(), NOW)).toBe('Oct 8');
    expect(formatChatDay(new Date(2026, 8, 5, 10, 49).toISOString(), NOW)).toBe('Sep 5');
    expect(formatChatDay(new Date(2025, 8, 5, 10, 49).toISOString(), NOW)).toBe('Sep 5, 2025');
  });

  it('never prints a numeric US date, and says nothing for a value that is not a date', () => {
    expect(formatChatDay(new Date(2026, 8, 5, 10, 49).toISOString(), NOW)).not.toMatch(/\//);
    expect(formatChatDay('not a date', NOW)).toBe('');
  });
});

describe('formatChatTimestamp', () => {
  it('gives the full stamp in the same words, from an ISO string or epoch milliseconds', () => {
    const when = new Date(2026, 8, 5, 10, 49);
    expect(plain(formatChatTimestamp(when.toISOString()))).toBe('Sep 5, 2026, 10:49 AM');
    expect(plain(formatChatTimestamp(when.getTime()))).toBe('Sep 5, 2026, 10:49 AM');
    expect(formatChatTimestamp('not a date')).toBe('');
  });
});
