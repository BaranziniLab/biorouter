import { cn } from '../../utils';
import { usePrivacyTiersEnabled } from '../ConfigContext';
import type { SessionClassification } from '../../api/types.gen';
import { chatIconFor, chatKindOf, chatPrivacyMark, type ChatKindSource } from './chatKind';

interface ChatKindIconProps {
  session: ChatKindSource;
  tier?: SessionClassification | null;
  className?: string;
  /**
   * Overrides the default `chat-kind-icon` test id. The sidebar keys its rows
   * by session (`recent-chat-glyph-<id>`) so a suite can assert about ONE row
   * in a list of many; a component that hard-coded one id for every instance
   * would take that away.
   */
  testId?: string;
  /**
   * This is the chat the user is currently in.
   *
   * ⚠ It exists to fix a THREE-way ink precedence that no single caller could
   * express: active beats private beats default. The sidebar wants its selected
   * row in the accent-bar ink; a private chat wants the accent ink; everything
   * else wants the subtle ink. Left to `cn`'s last-wins merge, whichever of the
   * two the caller happened to pass would silently win — and passing the
   * subtle ink for an inactive row would have erased the private tint on every
   * inactive private chat. Naming the case here keeps the whole order in one
   * place.
   */
  isActive?: boolean;
}

/**
 * The one glyph that says what a chat IS — a chat, a branch, a sub-agent, an
 * app, a scheduled run, a terminal — and whether it is private.
 *
 * ⚠ **This replaced the dense privacy dot on every chat list.** The dot was a
 * second mark sitting beside an icon that was identical for all six kinds, so a
 * list of chats carried one bit of differentiation (private or not) and none at
 * all about what any row was. Folding the tier into the glyph frees the row and
 * removes a mark the eye had to learn separately.
 *
 * ⚠ **The tier ink is applied here, not by the caller**, so the six surfaces
 * that draw chats cannot disagree about what private looks like — which is
 * exactly what happened when each one hung its own `PrivacyBadge`.
 *
 * ⚠ **It honours the master privacy switch** for the same reason `PrivacyBadge`
 * does (issue #56, DR-15): when nothing is enforcing tiers, a padlocked bubble
 * claiming protection is a false statement. The KIND still renders — that fact
 * is true either way — and only the privacy treatment stands down, to
 * `data-privacy="off"`.
 *
 * ⚠ **A tier no source has read is drawn as NOT YET KNOWN, never as Public** —
 * the kind's unmarked shape, dimmed, with "privacy not yet known" in its name.
 * It rendered `tier ?? 'public'` until 2026-09-14, and a private chat's
 * subagent tabs said Public for seconds on every Settings round-trip and every
 * reload. `ChatPrivacyMark` records the measurement and why "private until
 * confirmed" is not the fix either. A caller with no tier passes none; it must
 * never pass `'public'` to fill the gap.
 */
export function ChatKindIcon({
  session,
  tier,
  className,
  testId,
  isActive = false,
}: ChatKindIconProps) {
  const mark = chatPrivacyMark(tier, usePrivacyTiersEnabled());
  const kind = chatKindOf(session);
  const { Icon, label } = chatIconFor(kind, mark);

  return (
    <Icon
      data-testid={testId ?? 'chat-kind-icon'}
      data-chat-kind={kind}
      // `private` | `public` | `unknown` | `off`. The dimming that makes
      // `unknown` look unlike `public` is AUTHORED CSS keyed on this attribute
      // (`.br-chat-kind-icon[data-privacy='unknown']` in main.css), not a
      // Tailwind class here: under BIOROUTER_NO_HMR a newly written utility
      // can silently fail to generate, and this state must not depend on
      // class-scanning having worked.
      data-privacy={mark}
      // `role="img"` + `aria-label`: a bare <svg> with a label is not announced
      // by every screen reader — the same trap the dense dot fell into.
      role="img"
      aria-label={label}
      // Precedence, left to right, last wins: base → caller → tier. The tier
      // ink sits AFTER the caller's so an inactive private chat keeps its
      // marking, and stands down when the row is active because "you are here"
      // is the more urgent of the two and the padlock shape still says private.
      className={cn(
        'br-chat-kind-icon flex-none',
        className,
        mark === 'private' && !isActive ? 'text-text-accent' : undefined
      )}
    />
  );
}
