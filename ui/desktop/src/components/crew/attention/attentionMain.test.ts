import { describe, expect, it } from 'vitest';
import {
  AttentionBadges,
  AttentionThrottle,
  NOTIFY_MAX_PER_INTERVAL,
  NOTIFICATION_TEXT_MAX_CHARS,
  attentionNotificationAllowed,
  parseAttentionRequest,
} from './attentionMain';

describe('parseAttentionRequest', () => {
  const valid = {
    key: 'anything the window says',
    title: 'Alice Chen mentioned you in #general',
    body: 'chen-lab',
    connectionId: 'conn-1',
    channelId: 'chan-1',
  };

  it('keys the limit by the ids, never by what the window called it', () => {
    expect(parseAttentionRequest(valid)).toEqual({ ...valid, key: 'conn-1:chan-1' });
  });

  it('shows plain one-line text only', () => {
    const parsed = parseAttentionRequest({
      ...valid,
      title: 'Mal\u202Elory <b>mentioned</b>\nyou',
      body: 'x'.repeat(NOTIFICATION_TEXT_MAX_CHARS + 20),
    });
    expect(parsed?.title).toBe('Mallory mentioned you');
    expect(Array.from(parsed?.body ?? '')).toHaveLength(NOTIFICATION_TEXT_MAX_CHARS);
  });

  it('keeps separators as word breaks and leaves out zero-width and private-use characters', () => {
    const parsed = parseAttentionRequest({
      ...valid,
      title: 'Alice\u2028Chen\u2029mentioned\u200B you\u{E000} in \u{F0000}#general\uFEFF',
      body: 'chen\u0000-lab',
    });
    expect(parsed?.title).toBe('Alice Chen mentioned you in #general');
    expect(parsed?.body).toBe('chen-lab');
  });

  it.each([
    ['nothing', null],
    ['no title', { ...valid, title: '  ' }],
    ['a path for an id', { ...valid, channelId: '../x' }],
    ['a number for an id', { ...valid, connectionId: 7 }],
  ])('ignores %s', (_label, raw) => {
    expect(parseAttentionRequest(raw)).toBeNull();
  });
});

describe('attentionNotificationAllowed', () => {
  it('lets the window in front, or any window when none is, and no other', () => {
    expect(attentionNotificationAllowed(1, null)).toBe(true);
    expect(attentionNotificationAllowed(1, 1)).toBe(true);
    expect(attentionNotificationAllowed(1, 2)).toBe(false);
  });
});

describe('AttentionBadges', () => {
  it('shows the largest count any window reports, and forgets a closed window', () => {
    const badges = new AttentionBadges();
    expect(badges.set(1, 3)).toBe(3);
    expect(badges.set(2, 5)).toBe(5);
    expect(badges.set(2, 0)).toBe(3);
    expect(badges.set(1, -4)).toBe(0);
    expect(badges.set(3, 'many')).toBe(0);
    badges.set(4, 9);
    expect(badges.forget(4)).toBe(0);
  });
});

describe('AttentionThrottle across channels', () => {
  it('lets no more than a handful through a minute, whatever channels they name', () => {
    const throttle = new AttentionThrottle(60_000);
    for (let index = 0; index < NOTIFY_MAX_PER_INTERVAL; index += 1)
      expect(throttle.allow(`conn:${index}`, index)).toBe(true);
    expect(throttle.allow('conn:another', 100)).toBe(false);
    expect(throttle.allow('conn:another', 60_000)).toBe(true);
  });
});
