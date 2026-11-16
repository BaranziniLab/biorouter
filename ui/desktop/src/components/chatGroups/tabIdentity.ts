import type { ChatKindSource } from '../chats/chatKind';
import { historyRowTitle } from '../sessions/historyRow';
import type { SessionType } from '../../api';

/*
 * What a tab IS and what it SAYS, from what the strip holds. Moved here, unchanged,
 * from `ChatTabStrip.tsx`, because the shell now computes the same identity for
 * every pane (a compact pane's placeholder shows its chat's glyph and title), and
 * every shell test mocks `./ChatTabStrip` with a factory: helpers left there would
 * be undefined in all of them.
 */

/**
 * A tab is not a session row — it persists `{tabId, sessionId, title}` and
 * nothing about lineage — so the kind has to be assembled from what the strip
 * actually holds: the title (which still catches `app:` and legacy branch
 * names), the annotation the workspace planner recorded when a sub-agent
 * opened a tab, and the session type the tab's own row reported.
 *
 * ⚠ **Two sources for "sub-agent", ORed, because neither survives alone.** The
 * annotation is there from the moment the daemon opens the tab, but it is React
 * state in `ChatGroupsProvider`, which mounts inside the `/pair` route and is
 * never persisted: leaving `/pair` (Settings, History) or reloading drops it,
 * and every subagent tab then read as a plain chat (measured 2026-09-14 on
 * 1.90.4). The row survives any of that, but arrives only once the shell has it
 * — from the cached session list, from the live store that loaded the chat, or
 * from its own read of a chat the list leaves out — and always no later than
 * the tier the same row brings. So the annotation covers the first moments and
 * the row covers every remount after them.
 *
 * ⚠ Deliberately NOT widened by fetching the session HERE: the strip renders on
 * every keystroke of a rename, and a per-tab fetch there is how a tab strip
 * becomes the slowest thing in the app. The row is the one
 * `ChatGroupsShell.useTabTitlesFromSessionList` already holds, once per list.
 */
export function tabKindSource(
  tab: { title: string },
  annotation?: { badge?: string; parentSessionId?: string },
  rowSessionType?: SessionType,
  rowWorkingDir?: string
): ChatKindSource {
  // ⚠ Only `badge === 'subagent'` or a row that says `sub_agent` marks a
  // sub-agent here — deliberately NOT `parentSessionId`, from either source.
  // `TabAnnotation.badge` is a free-form string and the planner may record a
  // parent link for other reasons; the strip's own suite guards that an
  // annotation carrying a parent but no badge marks nothing.
  //
  // And only `sub_agent` is taken from the row: it is the one kind the
  // annotation carried and a route change lost. Taking the rest would redraw
  // every scheduled or terminal chat's tab from a row that may not have
  // answered yet — a change to tabs this fix has no measurement for.
  const subagent = annotation?.badge === 'subagent' || rowSessionType === 'sub_agent';
  return {
    name: tab.title,
    session_type: subagent ? 'sub_agent' : null,
    // Read only by the Crew test (`isCrewTask`): a Crew task's folder survives
    // a rename that its title prefix does not.
    working_dir: rowWorkingDir ?? null,
  };
}

/**
 * The title a tab SHOWS: its stored title minus what its glyph already says, from
 * the one resolver History and the sidebar use (`historyRowTitle`; principle 2, say
 * it once; principle 10, names not IDs; vision V1-R2-10 and V1-R3-6).
 *
 * Given what {@link tabKindSource} builds, two kinds change: a sub-agent's tab reads
 * "Reply with ALPHA" beside the robot glyph instead of "Subagent: Reply with ALPHA",
 * and an app's chat reads "Living news router" instead of `app:living-news-router`.
 * A scheduled run keeps "Scheduled job: <id>": the strip does not take `scheduled`
 * from the row, so the tab draws the plain chat glyph and the prefix is the only
 * thing that says what it is. If that ever changes, the title follows on its own,
 * because both read the same source.
 *
 * Display only. The stored `tab.title` is what the rename field starts from and what
 * the rename rolls back to, and the overflow filter matches it as well as this.
 */
export function tabShownTitle(source: ChatKindSource, role?: string | null): string {
  // F-15 / C-3: a subagent's tab is titled by its role ("Reviewer"), the name the
  // roster gives it, and falls back to its short task. Display only, like the
  // rest: a later list refresh that renames the stored title cannot undo it.
  if (role && source.session_type === 'sub_agent') return role;
  return historyRowTitle(source);
}

/** The tab's tooltip: "{role}: {task}" when the role is shown, else the shown title. */
export function tabTooltip(source: ChatKindSource, role?: string | null): string {
  const shown = tabShownTitle(source, role);
  const task = historyRowTitle(source);
  return shown !== task ? `${shown}: ${task}` : shown;
}
