import type { Session } from '../../api';
import type { DateGroup } from '../../utils/dateUtils';

/**
 * Principle 9, for Chat history: never move a row under the pointer.
 *
 * The list is live. A chat created or branched in another window, or a subagent
 * run that starts under a parent, arrives while someone is reading, and a
 * newest-first list puts it ABOVE what they were about to click. While the
 * pointer is over the list the rows keep the order they had when it came in;
 * the live order applies the moment it leaves. The sidebar does the same for its
 * own list (`BioRouterSidebar/sidebarChatView.ts`); this is the same rule for
 * History's date groups and the subagent runs nested under a parent.
 *
 * What still changes while held: a row's own content (a rename, a privacy
 * mark), a row that was removed, and rows that arrive BELOW every held row (the
 * next page of a scrolled list, which is how the list grows under a mouse
 * wheel), none of which shift anything above them. A row that would arrive above
 * or between held rows, or a new run under a held parent, waits.
 *
 * Pure, so it is tested without a DOM.
 */

/** The order the list showed when the pointer came in. */
export interface HeldHistory {
  groups: Array<{ key: string; label: string; ids: string[] }>;
  /** parent id → the ids of the runs nested under it, in order. */
  children: Record<string, string[]>;
}

export interface HistoryArrangement {
  groups: DateGroup[];
  children: Map<string, Session[]>;
}

export function holdHistory(
  groups: readonly DateGroup[],
  childrenByParentId: ReadonlyMap<string, readonly Session[]>
): HeldHistory {
  const children: Record<string, string[]> = {};
  for (const [parentId, runs] of childrenByParentId) {
    children[parentId] = runs.map((run) => run.id);
  }
  return {
    groups: groups.map(({ key, label, sessions }) => ({
      key,
      label,
      ids: sessions.map((session) => session.id),
    })),
    children,
  };
}

/**
 * `live` is the order History would show right now; the result is what to draw
 * while `held` stands. `liveChildren` is the live nesting, and a held parent's
 * runs are looked up in it by id so a run that was removed drops out.
 */
export function applyHeldHistory(
  held: HeldHistory,
  live: readonly DateGroup[],
  liveChildren: ReadonlyMap<string, readonly Session[]>
): HistoryArrangement {
  const liveFlat = live.flatMap((group) => group.sessions.map((session) => ({ group, session })));
  const byId = new Map(liveFlat.map((entry) => [entry.session.id, entry.session]));
  const heldIds = new Set(held.groups.flatMap((group) => group.ids));

  let lastHeld = -1;
  liveFlat.forEach((entry, index) => {
    if (heldIds.has(entry.session.id)) lastHeld = index;
  });

  const groups: DateGroup[] = held.groups
    .map(({ key, label, ids }) => ({
      key,
      label,
      sessions: ids.flatMap((id) => {
        const session = byId.get(id);
        return session ? [session] : [];
      }),
    }))
    .filter((group) => group.sessions.length > 0);

  // Rows that arrive after the last held row shift nothing above them.
  for (const { group, session } of liveFlat.slice(lastHeld + 1)) {
    if (heldIds.has(session.id)) continue;
    const last = groups[groups.length - 1];
    if (last && last.key.split(':')[0] === group.key) {
      last.sessions.push(session);
      continue;
    }
    const taken = groups.some((existing) => existing.key === group.key);
    groups.push({
      key: taken ? `${group.key}:after` : group.key,
      label: group.label,
      sessions: [session],
    });
  }

  const children = new Map<string, Session[]>();
  for (const { sessions } of groups) {
    for (const parent of sessions) {
      const heldRuns = held.children[parent.id];
      const liveRuns = liveChildren.get(parent.id);
      if (!heldRuns) {
        // A parent that was not held arrived after the pointer did: its runs
        // arrive with it.
        if (!heldIds.has(parent.id) && liveRuns && liveRuns.length > 0) {
          children.set(parent.id, [...liveRuns]);
        }
        continue;
      }
      const alive = new Map((liveRuns ?? []).map((run) => [run.id, run]));
      const runs = heldRuns.flatMap((id) => {
        const run = alive.get(id);
        return run ? [run] : [];
      });
      if (runs.length > 0) children.set(parent.id, runs);
    }
  }
  return { groups, children };
}
