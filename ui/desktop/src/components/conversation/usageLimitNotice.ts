import type { Message } from '../../api';

/**
 * The notices the daemon writes when a usage limit is near or reached.
 *
 * ⚠ Mirrors the `*_PREFIX` constants in `crates/biorouter/src/agents/usage_limit.rs`,
 * and a Rust test reads these exact declarations. Change both or neither.
 *
 * A stop notice is stored (user-visible, model-hidden) as an ordinary inline
 * system notification, so a reader that does not know these prefixes still
 * shows it. The text after the prefix carries the amounts and where to change
 * the limit; recognise a notice by its prefix only.
 */
export const USAGE_LIMIT_CHAT_REACHED_PREFIX = 'Usage limit reached for this chat: ';
export const USAGE_LIMIT_MONTH_REACHED_PREFIX = 'Monthly usage limit reached: ';
export const USAGE_LIMIT_CHAT_WARNING_PREFIX = 'This chat is near its usage limit: ';
export const USAGE_LIMIT_MONTH_WARNING_PREFIX = 'Near the monthly usage limit: ';

export type UsageLimitNoticeKind = 'chatReached' | 'monthReached' | 'chatWarning' | 'monthWarning';

/** Which usage limit notice a notice text is, or `null` for any other text. */
export function usageLimitNoticeKindOfText(text: string): UsageLimitNoticeKind | null {
  if (text.startsWith(USAGE_LIMIT_CHAT_REACHED_PREFIX)) return 'chatReached';
  if (text.startsWith(USAGE_LIMIT_MONTH_REACHED_PREFIX)) return 'monthReached';
  if (text.startsWith(USAGE_LIMIT_CHAT_WARNING_PREFIX)) return 'chatWarning';
  if (text.startsWith(USAGE_LIMIT_MONTH_WARNING_PREFIX)) return 'monthWarning';
  return null;
}

/** Which usage limit notice this row is, or `null` when it is not one. */
export function usageLimitNoticeKind(message: Message | undefined): UsageLimitNoticeKind | null {
  if (!message || message.role !== 'assistant' || message.content.length !== 1) return null;
  const [content] = message.content;
  if (content.type !== 'systemNotification' || content.notificationType !== 'inlineMessage') {
    return null;
  }
  return usageLimitNoticeKindOfText(content.msg);
}

/** Is this row a stored "a usage limit stopped this chat" notice? */
export function isUsageLimitStopNotice(message: Message | undefined): boolean {
  const kind = usageLimitNoticeKind(message);
  return kind === 'chatReached' || kind === 'monthReached';
}
