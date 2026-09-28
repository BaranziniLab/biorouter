import { describe, expect, it } from 'vitest';
import {
  AttentionBadges,
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
      title: 'Mal‮lory <b>mentioned</b>\nyou',
      body: 'x'.repeat(NOTIFICATION_TEXT_MAX_CHARS + 20),
    });
    expect(parsed?.title).toBe('Mallory mentioned you');
    expect(Array.from(parsed?.body ?? '')).toHaveLength(NOTIFICATION_TEXT_MAX_CHARS);
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
