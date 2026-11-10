import { describe, expect, it } from 'vitest';
import type { Message } from '../../api';
import { USAGE_LIMIT_CHAT_REACHED_PREFIX } from './usageLimitNotice';
import { latestUsageLimitNoticeKey } from './usageLimitTranscript';

const notice = (id: string | null, msg: string): Message => ({
  id,
  role: 'assistant',
  created: 0,
  metadata: { userVisible: true, agentVisible: false },
  content: [{ type: 'systemNotification', notificationType: 'inlineMessage', msg }],
});
const text = (id: string): Message => ({
  id,
  role: 'user',
  created: 0,
  metadata: { userVisible: true, agentVisible: true },
  content: [{ type: 'text', text: 'hello' }],
});

describe('the newest stored usage limit notice', () => {
  it('is keyed by its id, so a new notice moves the key', () => {
    const one = notice('n1', `${USAGE_LIMIT_CHAT_REACHED_PREFIX}10,000 tokens.`);
    expect(latestUsageLimitNoticeKey([text('a'), one])).toBe('n1');
    const two = notice('n2', `${USAGE_LIMIT_CHAT_REACHED_PREFIX}10,000 tokens.`);
    expect(latestUsageLimitNoticeKey([text('a'), one, text('b'), two])).toBe('n2');
  });

  it('is keyed by its place when it has no id', () => {
    const one = notice(null, `${USAGE_LIMIT_CHAT_REACHED_PREFIX}10,000 tokens.`);
    expect(latestUsageLimitNoticeKey([text('a'), one])).toBe('at:1');
  });

  it('is absent when no row is one, including other inline notices', () => {
    expect(latestUsageLimitNoticeKey([])).toBeUndefined();
    expect(latestUsageLimitNoticeKey([text('a'), notice('s', 'Stopped.')])).toBeUndefined();
  });
});
