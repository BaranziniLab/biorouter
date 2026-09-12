import type { ReactNode } from 'react';
import { Note } from '../ui/note';
import {
  composerSlotMode,
  subagentTabReadOnlyReason,
  type SubagentComposerKind,
} from './subagentReadOnly';

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
 *
 * ⚠ **`kind` is three-valued, and the third value renders NOTHING.** The slot
 * used to take a boolean, which reported "not a subagent" and "we do not know
 * yet" with the same `false` — and in a browser every source of that fact is an
 * asynchronous read, so the unknown state lasted seconds and mounted the
 * composer throughout. `composerSlotMode` is where that is decided; see
 * `subagentComposerKind` for why the daemon's tab badge does not close it on
 * its own (it is in-memory React state and does not survive a page reload).
 */
export function SubagentComposerSlot({
  kind,
  children,
}: {
  /**
   * What this tab knows about its chat: a delegated subagent's, definitely not
   * one, or not yet resolved. Computed by `subagentComposerKind`.
   */
  kind: SubagentComposerKind;
  children: ReactNode;
}) {
  const mode = composerSlotMode(kind);
  if (mode === 'composer') return <>{children}</>;
  // Nothing at all while the answer is in flight: an explanation that may turn
  // out to be wrong is worse than a beat of empty space under an empty
  // transcript, and a placeholder that flashes and vanishes reads as a fault.
  if (mode === 'withheld') return null;
  const reason = subagentTabReadOnlyReason() ?? '';
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
