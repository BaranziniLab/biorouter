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
   * This is the chat the user is currently in: the body takes the default
   * ink instead of the muted one. The lock badge keeps the accent either way
   * (it is authored CSS on `.br-icon-lock-badge`), so a private active row
   * still shows the badge in colour.
   */
  isActive?: boolean;
}

/**
 * The one glyph that says what a chat IS (a chat, a Crew task, a branch, a
 * sub-agent, an app, a scheduled run, a terminal) and whether it is private.
 *
 * ⚠ **This replaced the dense privacy dot on every chat list.** The dot was a
 * second mark sitting beside an icon that was identical for all six kinds, so a
 * list of chats carried one bit of differentiation (private or not) and none at
 * all about what any row was. Folding the tier into the glyph frees the row and
 * removes a mark the eye had to learn separately.
 *
 * ⚠ **Private is the lock badge, a shape, on every kind.** The glyph no
 * longer carries a tier ink: the coral body made an all-private list a wall of
 * coral glyphs. The ink is applied here, not by each caller, so the surfaces
 * that draw chats cannot disagree: the body is muted at rest and default ink
 * on the active row, and only the badge wears the family accent (authored in
 * main.css, `.br-icon-lock-badge`). A caller's className still wins over the
 * body ink, because it comes last.
 *
 * ⚠ **It always draws in a 16px slot** (`--icon-row`, authored on
 * `.br-chat-kind-icon` in main.css), whatever size class a caller passes, so a
 * chat glyph never sits a size below the nav icons beside it.
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
      // Precedence, left to right, last wins: base, body ink, caller.
      className={cn(
        'br-chat-kind-icon flex-none',
        isActive ? 'text-text-default' : 'text-text-muted',
        className
      )}
    />
  );
}
