import { describe, expect, it } from 'vitest';

import { chatDateBucket, CHAT_DATE_BUCKET_LABELS, groupByChatDate } from './chatDateBuckets';

// Local time, so the calendar-day arithmetic is the one a person sees.
const NOW = new Date(2026, 9, 9, 12, 0, 0).getTime(); // Fri 9 Oct 2026, noon
const at = (year: number, month: number, day: number, hour = 12) =>
  new Date(year, month, day, hour, 0, 0).toISOString();

describe('chatDateBucket', () => {
  it('names today and yesterday by local calendar day', () => {
    expect(chatDateBucket(at(2026, 9, 9, 0), NOW).label).toBe('Today');
    expect(chatDateBucket(at(2026, 9, 8, 23), NOW).label).toBe('Yesterday');
  });

  it('puts days 2 to 7 in Previous 7 days and 8 to 30 in Previous 30 days', () => {
    expect(chatDateBucket(at(2026, 9, 7), NOW).label).toBe(CHAT_DATE_BUCKET_LABELS.week);
    expect(chatDateBucket(at(2026, 9, 2), NOW).label).toBe(CHAT_DATE_BUCKET_LABELS.week);
    expect(chatDateBucket(at(2026, 9, 1), NOW).label).toBe(CHAT_DATE_BUCKET_LABELS.month30);
    expect(chatDateBucket(at(2026, 8, 9), NOW).label).toBe(CHAT_DATE_BUCKET_LABELS.month30);
  });

  it('names older months, adding the year outside the current one', () => {
    expect(chatDateBucket(at(2026, 8, 8), NOW).label).toBe('September');
    expect(chatDateBucket(at(2026, 7, 1), NOW).label).toBe('August');
    expect(chatDateBucket(at(2025, 7, 1), NOW).label).toBe('August 2025');
  });

  it('never makes a header per day', () => {
    const labels = new Set(
      [2, 3, 4, 5, 6, 7].map((back) => chatDateBucket(at(2026, 9, 9 - back), NOW).label)
    );
    expect([...labels]).toEqual(['Previous 7 days']);
  });

  it('treats a future timestamp as today and an unreadable one as older', () => {
    expect(chatDateBucket(at(2026, 9, 12), NOW).label).toBe('Today');
    expect(chatDateBucket('not a date', NOW).label).toBe('Older');
  });
});

describe('groupByChatDate', () => {
  it('orders buckets newest first and keeps the caller order inside each', () => {
    const rows = [
      { id: 'b', when: at(2026, 9, 9) },
      { id: 'old', when: at(2025, 0, 3) },
      { id: 'a', when: at(2026, 9, 9) },
      { id: 'y', when: at(2026, 9, 8) },
      { id: 'sep', when: at(2026, 8, 1) },
    ];
    const groups = groupByChatDate(rows, (row) => row.when, NOW);
    expect(groups.map((group) => [group.label, group.items.map((row) => row.id)])).toEqual([
      ['Today', ['b', 'a']],
      ['Yesterday', ['y']],
      ['September', ['sep']],
      ['January 2025', ['old']],
    ]);
  });
});
