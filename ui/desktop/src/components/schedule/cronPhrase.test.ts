import { describe, expect, it } from 'vitest';
import { clockWithMinutes, cronPhrase, cronPhraseOrThrow } from './cronPhrase';

/**
 * V3-15: the Scheduler read "At 03:00 AM" (a padded hour, no frequency) where the spec's own
 * example is "Every day at 3:00 AM". One phrase function now feeds every surface.
 */
describe('cronPhrase', () => {
  it('says the frequency and leaves the hour unpadded', () => {
    expect(cronPhrase('0 0 3 * * *')).toBe('Every day at 3:00 AM');
    expect(cronPhrase('0 0 14 * * *')).toBe('Every day at 2:00 PM');
    expect(cronPhrase('0 30 0 * * *')).toBe('Every day at 12:30 AM');
    expect(cronPhrase('0 5 12 * * *')).toBe('Every day at 12:05 PM');
  });

  it('reads a five-field rule with the seconds taken as 0', () => {
    expect(cronPhrase('0 9 * * *')).toBe('Every day at 9:00 AM');
  });

  it('names the days of a weekly rule', () => {
    expect(cronPhrase('0 0 9 * * 1')).toBe('Every Monday at 9:00 AM');
    expect(cronPhrase('0 30 9 * * 1-5')).toBe('Every weekday at 9:30 AM');
    expect(cronPhrase('0 0 9 * * MON-FRI')).toBe('Every weekday at 9:00 AM');
    expect(cronPhrase('0 0 9 * * 1,3,5')).toBe('Every Monday, Wednesday and Friday at 9:00 AM');
    expect(cronPhrase('0 0 9 * * 0,6')).toBe('Every Sunday and Saturday at 9:00 AM');
    expect(cronPhrase('0 0 9 * * 7')).toBe('Every Sunday at 9:00 AM');
    expect(cronPhrase('0 0 9 * * 0-6')).toBe('Every day at 9:00 AM');
  });

  it('reads monthly and yearly rules', () => {
    expect(cronPhrase('0 0 9 1 * *')).toBe('On the 1st of every month at 9:00 AM');
    expect(cronPhrase('0 0 9 22 * *')).toBe('On the 22nd of every month at 9:00 AM');
    expect(cronPhrase('0 0 9 11 * *')).toBe('On the 11th of every month at 9:00 AM');
    expect(cronPhrase('0 0 9 4 7 *')).toBe('Every year on July 4 at 9:00 AM');
  });

  it('reads the short intervals', () => {
    expect(cronPhrase('0 * * * * *')).toBe('Every minute');
    expect(cronPhrase('30 * * * * *')).toBe('Every minute at second 30');
    expect(cronPhrase('0 */15 * * * *')).toBe('Every 15 minutes');
    expect(cronPhrase('0 0 * * * *')).toBe('Every hour');
    expect(cronPhrase('0 15 * * * *')).toBe('Every hour at :15');
    expect(cronPhrase('0 0 */6 * * *')).toBe('Every 6 hours');
  });

  it('falls back to a trimmed cronstrue sentence for a shape it does not phrase', () => {
    const phrase = cronPhrase('0 0 9,17 * * *');
    expect(phrase).toMatch(/9:00 AM/);
    expect(phrase).not.toMatch(/\b0\d:\d\d/);
  });

  it('returns the expression itself when nothing can read it', () => {
    expect(cronPhrase('nonsense')).toBe('nonsense');
  });

  it('throws only through the picker variant', () => {
    expect(cronPhraseOrThrow('0 0 3 * * *')).toBe('Every day at 3:00 AM');
    expect(() => cronPhraseOrThrow('nonsense')).toThrow();
  });
});

describe('clockWithMinutes', () => {
  it('always shows the minutes and never pads the hour', () => {
    expect(clockWithMinutes(0, 0)).toBe('12:00 AM');
    expect(clockWithMinutes(9, 5)).toBe('9:05 AM');
    expect(clockWithMinutes(12, 0)).toBe('12:00 PM');
    expect(clockWithMinutes(23, 59)).toBe('11:59 PM');
  });
});
