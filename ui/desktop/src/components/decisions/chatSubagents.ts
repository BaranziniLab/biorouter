/**
 * Whether a chat has started any subagent, for the Monitors dialog (owner decision 2):
 * once it has, "Also watch the subagents this chat starts" sits up front in a
 * monitor's editor instead of in "More options".
 *
 * The roster read carries the user-action proof (`userActionHeaders()`), as
 * `hooks/useSessionSubagents.ts` does: the daemon filters a private chat's children
 * through `lists_session`, and without the proof they would silently vanish.
 */
import { getSessionSubagents } from '../../api';
import { parseSessionSubagents } from '../../utils/sessionSubagents';
import { userActionHeaders } from '../../utils/userAction';
import { rememberedRoster } from '../summary/rosterMemory';

/**
 * What this window last saw: `true` when the Chat summary's last roster for the chat
 * named a subagent, else `null` (unknown, not "none": the memory forgets an empty one).
 */
export function knownHasSubagents(sessionId: string): boolean | null {
  const roster = rememberedRoster(sessionId);
  return roster && roster.subagents.length > 0 ? true : null;
}

/**
 * `GET /sessions/{session_id}/subagents`: whether the chat has started any subagent,
 * or `null` when that could not be read (an older daemon, a refusal, a network
 * failure). Never throws: a dialog that cannot tell keeps the option where it was.
 */
export async function fetchHasSubagents(
  sessionId: string,
  signal: AbortSignal
): Promise<boolean | null> {
  try {
    const headers = await userActionHeaders();
    if (signal.aborted) return null;
    const response = await getSessionSubagents({
      path: { session_id: sessionId },
      headers,
      signal,
    });
    if (response.error !== undefined || response.data === undefined) return null;
    const data = parseSessionSubagents(response.data as unknown);
    return data ? data.subagents.length > 0 : null;
  } catch {
    return null;
  }
}
