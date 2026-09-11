import type { ReactNode } from 'react';
import { Note } from '../ui/note';
import { subagentTabReadOnlyReason } from './subagentReadOnly';

/**
 * The composer's place on a chat tab, which a delegated subagent's tab in a
 * browser fills with the reason it has no composer (SD-8).
 *
 * Everywhere else — the desktop, and every chat that is not a subagent's — it
 * renders `children`, the composer, exactly as before. In a browser, on a
 * subagent's chat, it renders the explanation INSTEAD, and `children` never
 * mount. That is the point of doing it here rather than by disabling things
 * inside `ChatInput`: Send and steer are not the only writes the composer holds.
 * Stop, Stop-and-Send, the continuation banner's Take over and Abandon, and the
 * extension picker (`/agent/add_extension`, `/agent/remove_extension`) are all
 * refused for a subagent's chat on a daemon with no user-action key, so a
 * composer with its Send greyed out would still offer a row of controls that
 * fail on click. Not mounting it is the one version with no control left to
 * forget.
 */
export function SubagentComposerSlot({
  isSubagentChat,
  children,
}: {
  /** The chat on this tab is a delegated subagent's (`session_type === 'sub_agent'`). */
  isSubagentChat: boolean;
  children: ReactNode;
}) {
  const reason = isSubagentChat ? subagentTabReadOnlyReason() : null;
  if (!reason) return <>{children}</>;
  // Full width, no `mx-3`: this takes the CARD's place, and the card spans the
  // composer shell edge to edge. The `mx-3` on `PinnedModelNote` is right for
  // it and wrong here — that note sits above the card, on the context row's
  // inset rails.
  return (
    <Note tone="neutral" role="status" testId="subagent-read-only-note">
      {reason}
    </Note>
  );
}
