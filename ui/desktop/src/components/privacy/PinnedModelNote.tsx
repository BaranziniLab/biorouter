import { Note } from '../ui/note';
import { usePinnedModel } from './usePinnedModel';
import type { PinnedModelView } from '../../hooks/chatStreamStore';

/**
 * Issue #56 Gate B — the one line telling the user that this chat is running on
 * a model other than the one they selected, and why.
 *
 * ⚠ **Renders nothing unless there is something to say**, so a call site can
 * mount it unconditionally — the same shape (and the same reason)
 * {@link ../privacy/HostManagedModelNote.HostManagedModelNote} uses. The pin
 * frame arrives on every repaired turn, including the ordinary ones where it
 * names exactly what is already on screen; `usePinnedModel` is what tells those
 * apart.
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
  pinnedModel,
  className,
}: {
  pinnedModel?: PinnedModelView;
  /** Layout only — `mx-*`, `mb-*`. */
  className?: string;
}) {
  const { notice } = usePinnedModel(pinnedModel);
  if (!notice) return null;
  return (
    <Note tone="neutral" role="status" testId="pinned-model-note" className={className}>
      {notice}
    </Note>
  );
}
