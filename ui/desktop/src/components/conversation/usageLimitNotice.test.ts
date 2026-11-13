import { describe, expect, it } from 'vitest';
import type { Message } from '../../api';
import {
  isUsageLimitStopNotice,
  usageLimitNoticeKind,
  usageLimitNoticeKindOfText,
  USAGE_LIMIT_CHAT_REACHED_PREFIX,
  USAGE_LIMIT_MONTH_REACHED_PREFIX,
} from './usageLimitNotice';

function notice(msg: string): Message {
  return {
    id: 'n1',
    role: 'assistant',
    created: 0,
    content: [{ type: 'systemNotification', notificationType: 'inlineMessage', msg }],
    metadata: { userVisible: true, agentVisible: false },
  } as unknown as Message;
}

describe('usageLimitNotice', () => {
  it('recognises each notice by its prefix', () => {
    expect(usageLimitNoticeKindOfText(`${USAGE_LIMIT_CHAT_REACHED_PREFIX}1,000 tokens used`)).toBe(
      'chatReached'
    );
    expect(usageLimitNoticeKindOfText('Monthly usage limit reached: $5.00 used')).toBe(
      'monthReached'
    );
    expect(usageLimitNoticeKindOfText('This chat is near its usage limit: 80 tokens')).toBe(
      'chatWarning'
    );
    expect(usageLimitNoticeKindOfText('Near the monthly usage limit: 80 tokens')).toBe(
      'monthWarning'
    );
    expect(usageLimitNoticeKindOfText('Stopped.')).toBeNull();
  });

  it('only matches a stored inline notice', () => {
    const stop = notice(`${USAGE_LIMIT_MONTH_REACHED_PREFIX}10 tokens used`);
    expect(usageLimitNoticeKind(stop)).toBe('monthReached');
    expect(isUsageLimitStopNotice(stop)).toBe(true);

    const text = {
      ...stop,
      content: [{ type: 'text', text: `${USAGE_LIMIT_MONTH_REACHED_PREFIX}10` }],
    } as unknown as Message;
    expect(usageLimitNoticeKind(text)).toBeNull();
    expect(isUsageLimitStopNotice(undefined)).toBe(false);
    expect(isUsageLimitStopNotice(notice('Near the monthly usage limit: 1'))).toBe(false);
  });
});
