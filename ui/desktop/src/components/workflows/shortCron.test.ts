import { describe, expect, it } from 'vitest';
import { clockTime, cronSentence, shortCron } from './shortCron';

describe('shortCron', () => {
  it('says what CronPicker writes in a few words', () => {
    expect(shortCron('0 0 14 * * *')).toBe('Daily 2 PM');
    expect(shortCron('0 30 9 * * *')).toBe('Daily 9:30 AM');
    expect(shortCron('0 0 9 * * 1')).toBe('Mon 9 AM');
    expect(shortCron('0 0 9 * * 1-5')).toBe('Weekdays 9 AM');
    expect(shortCron('0 0 0 1 * *')).toBe('Monthly 1st 12 AM');
    expect(shortCron('0 0 8 22 * *')).toBe('Monthly 22nd 8 AM');
    expect(shortCron('0 0 8 11 * *')).toBe('Monthly 11th 8 AM');
    expect(shortCron('0 15 6 4 7 *')).toBe('Yearly Jul 4 6:15 AM');
    expect(shortCron('0 0 * * * *')).toBe('Hourly');
    expect(shortCron('0 5 * * * *')).toBe('Hourly at :05');
    expect(shortCron('0 * * * * *')).toBe('Every minute');
    expect(shortCron('0 */15 * * * *')).toBe('Every 15 min');
    expect(shortCron('0 0 */2 * * *')).toBe('Every 2 h');
  });

  it('reads a five-field rule with the seconds taken as zero', () => {
    expect(shortCron('0 14 * * *')).toBe('Daily 2 PM');
  });

  it('falls back to a plain word for a rule it does not shorten, and to nothing for none', () => {
    expect(shortCron('0 0 9 1-7 * 1')).toBe('Scheduled');
    expect(shortCron('not a cron')).toBe('Scheduled');
    expect(shortCron(null)).toBeNull();
    expect(shortCron('')).toBeNull();
  });

  it('writes clock times the short way', () => {
    expect(clockTime(0, 0)).toBe('12 AM');
    expect(clockTime(12, 0)).toBe('12 PM');
    expect(clockTime(23, 5)).toBe('11:05 PM');
  });
});

describe('cronSentence', () => {
  it('says the whole rule, dropping the seconds field', () => {
    expect(cronSentence('0 0 14 * * *')).toBe('At 02:00 PM');
    expect(cronSentence('0 0 9 * * 1')).toMatch(/Monday/);
  });

  it('gives back the expression when it cannot read it', () => {
    expect(cronSentence('nonsense')).toBe('nonsense');
  });
});
