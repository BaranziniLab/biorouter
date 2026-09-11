import { getActive } from '../../api';
import { userActionHeaders } from '../../utils/userAction';
import { briefSelectionFailure } from './selectionWarning';

/** The shape both selection endpoints answer with — GET /active and POST /active. */
type SelectionPayload =
  | { primary_kb?: string | null; active_kb?: string | null; hidden_kbs?: string[] | null }
  | undefined;

/** `active_kb` is the deprecated mirror, read so a fresh renderer keeps working
 * against a daemon that predates `primary_kb`. */
export function readPrimary(data: SelectionPayload): string | null {
  return data?.primary_kb ?? data?.active_kb ?? null;
}

/** `null` means "this answer did not state a set" (a daemon that predates the
 * field) — distinct from an empty set, and the caller must leave what it has
 * rather than erase the session's whole working set. */
export function readHidden(data: SelectionPayload): string[] | null {
  return Array.isArray(data?.hidden_kbs)
    ? data.hidden_kbs.filter((id): id is string => typeof id === 'string')
    : null;
}

/** A chat's knowledge-base selection, as the daemon answered it. */
export interface KnowledgeSelection {
  primaryKbId: string | null;
  hiddenKbIds: ReadonlySet<string>;
}

/**
 * Read the selection of the chat `sessionId` names, or the machine-wide one
 * when it names none, for a surface that shows the selection or saves it.
 *
 * Resolves to `null` when the read failed, and never rejects.
 *
 * ⚠ **`null` is not "nothing hidden, nothing primary".** Both callers used to
 * read a failed request as exactly that, because `data?.hidden_kbs ?? []`
 * cannot tell the two apart: the `/` palette offered every base as one "in this
 * chat", and the create-workflow modal saved every base into a workflow that
 * outlives the chat. A failure is a genuine error since the read carries the
 * proof — a surface that cannot prove the person, a dropped connection, an
 * older daemon — and none of those is a statement about the chat. The same
 * rule `KnowledgeContext` states for `listBases`: a failed request is not an
 * empty answer. So the caller gets nothing to mistake for one.
 *
 * Never rejecting is the other half. The palette reads this beside its
 * commands, skills and extensions in one `Promise.all`, and a read that threw
 * took every one of them with it.
 */
export async function readKnowledgeSelection(
  sessionId: string | null | undefined
): Promise<KnowledgeSelection | null> {
  try {
    const res = await getActive({
      query: sessionId ? { session_id: sessionId } : undefined,
      // Issue #56 Task 58: a GET naming a PRIVATE chat is on the reach gate's
      // list, and the desktop gets through it by proving the person — as the
      // hydrate in `KnowledgeContext` does, and as `setActive` always has.
      headers: await userActionHeaders(),
      throwOnError: false,
    });
    if (!res.data) {
      console.warn('Knowledge selection not read:', briefSelectionFailure(res.error));
      return null;
    }
    return {
      primaryKbId: readPrimary(res.data),
      hiddenKbIds: new Set(readHidden(res.data) ?? []),
    };
  } catch (err) {
    console.warn('Knowledge selection not read:', briefSelectionFailure(err));
    return null;
  }
}
