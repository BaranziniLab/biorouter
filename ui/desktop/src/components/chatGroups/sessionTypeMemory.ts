import type { SessionType } from '../../api';

/**
 * The session type each tab's chat's row last reported, kept at MODULE scope so
 * it outlives the `/pair` route.
 *
 * `ChatGroupsShell` mounts inside `/pair`, and the type map it hands the tab
 * strip is React state, so it started empty on every return to a chat. A
 * delegated subagent's tab then drew the plain chat bubble until its row was
 * read again, and switched to the Bot glyph when it answered. Measured
 * 2026-09-14 on d7f02191 in the dev app: 641 ms after Settings → a sidebar
 * chat, 207 ms after History → back. A session's type does not change, so the
 * answer from the last visit is still the answer, and the shell seeds its state
 * from here when it mounts.
 *
 * Per renderer, so per window, and never persisted: a reload starts empty. Why
 * the reload case is left as it is lives with the shell's doc of the type map.
 *
 * # Two bounds
 *
 * - **Forgetting.** The shell calls {@link forgetSessionTypesExcept} with the
 *   chats its tabs hold on every reconcile, the same forgetting its own state
 *   map does. That keeps this to the tabs a window has open, and it is also
 *   what keeps an id that is reissued to a new chat from inheriting a closed
 *   subagent tab's kind. `create_session`'s high-water mark makes ids single
 *   use, so that happens only with a store that lacks the mark: an older build
 *   sharing the file, or a database restored from a backup.
 * - **A cap**, oldest entry first, for answers that land between reconciles,
 *   such as a read that answers after the shell has unmounted.
 */

/** Far above any window's tab count; a bound, not a working size. */
export const SESSION_TYPE_MEMORY_LIMIT = 512;

const remembered = new Map<string, SessionType>();

/** Record what a row said. The newest answer wins and counts as the newest entry. */
export function rememberSessionType(sessionId: string, sessionType: SessionType): void {
  remembered.delete(sessionId);
  remembered.set(sessionId, sessionType);
  while (remembered.size > SESSION_TYPE_MEMORY_LIMIT) {
    const oldest = remembered.keys().next();
    if (oldest.done) break;
    remembered.delete(oldest.value);
  }
}

/** What is remembered for these chats, as the shell's state map. */
export function recallSessionTypes(sessionIds: Iterable<string>): Record<string, SessionType> {
  const recalled: Record<string, SessionType> = {};
  for (const sessionId of sessionIds) {
    const sessionType = remembered.get(sessionId);
    if (sessionType) recalled[sessionId] = sessionType;
  }
  return recalled;
}

/** Drop every chat no tab holds. */
export function forgetSessionTypesExcept(held: ReadonlySet<string>): void {
  for (const sessionId of [...remembered.keys()]) {
    if (!held.has(sessionId)) remembered.delete(sessionId);
  }
}

/** How many chats are remembered. For tests. */
export function rememberedSessionTypeCount(): number {
  return remembered.size;
}

/** Forget everything. For tests, which share this module across a file. */
export function clearSessionTypeMemory(): void {
  remembered.clear();
}
