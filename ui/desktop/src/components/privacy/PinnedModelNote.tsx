import { Note } from '../ui/note';
import { usePinnedModel } from './usePinnedModel';
import type { PinnedModelView } from '../../hooks/chatStreamStore';
import type { Session } from '../../api/types.gen';

/**
 * Issue #56 / F2 — the one line telling the user that this chat is running on a
 * model other than the one they selected, and why.
 *
 * ⚠ **Renders nothing unless there is something to say**, so a call site can
 * mount it unconditionally — the same shape (and the same reason)
 * {@link ../privacy/HostManagedModelNote.HostManagedModelNote} uses. Most chats
 * run on exactly what is selected, and of the ones that do not, only a chat the
 * privacy barrier is holding has earned this sentence. `usePinnedModel` is what
 * tells those apart; see `pinnedModel.ts` for why the two questions are
 * separate.
 *
 * ⚠ **`neutral`, not `warning`.** Nothing has gone wrong. The chat is doing
 * precisely what being private means, the answer the user got is a real answer,
 * and their model choice is still in force in every other chat. A warning tone
 * here would tell them to go and fix something that is not broken.
 *
 * It has no dismiss control and never goes quiet, for the reason the standing
 * disclosure note already records: the condition is standing, so the statement
 * of it is too. It disappears on its own the moment the user selects a model
 * this chat can use.
 */
export function PinnedModelNote({
  session,
  reportedByTurn,
  className,
}: {
  /** The chat's own row: its classification and the binding it runs on. */
  session?: Session;
  /** A binding a turn reported for itself, which outranks the row. */
  reportedByTurn?: PinnedModelView;
  /** Layout only — `mx-*`, `mb-*`. */
  className?: string;
}) {
  const { notice } = usePinnedModel(session, reportedByTurn);
  if (!notice) return null;
  return (
    <Note tone="neutral" role="status" testId="pinned-model-note" className={className}>
      {notice}
    </Note>
  );
}
