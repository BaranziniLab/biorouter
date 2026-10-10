import type { ChatGroupId } from './chatGroupsTypes';

/**
 * What a chat tab strip remembers between two MOUNTS of itself (vision V1-H1).
 *
 * ⚠ **The strip remounts on every tab switch, so it cannot keep this in refs.**
 * It renders inside its chat's header (`renderSessionTitle`), and the shell keys
 * that whole chat on the active tab id (`ChatGroupPane`'s `BaseChat key`). So
 * clicking "+", choosing "Open in new tab" or closing the active tab builds a
 * NEW strip, and a strip that only remembered in refs read every one of those
 * tabs as part of a restore: no tab ever grew in, and the survivors of a close
 * jumped instead of sliding. The shell owns one record per group for as long as
 * it is mounted (`ChatGroupsShell`), so a reload, a new window, a tear-off and a
 * return to the chats route still start from nothing and appear at rest.
 *
 * A module of its own, not an export of `ChatTabStrip.tsx`, because the shell's
 * suites replace that module with a stub and the shell still needs this.
 */
export interface TabStripMemory {
  /** Each tab's `offsetLeft` the last time the strip measured it. */
  offsets: Map<string, number>;
  /** The tab order those offsets were measured in. */
  orderKey: string | null;
  /** True once the strip has painted with at least one tab. */
  painted: boolean;
}

export function createTabStripMemory(): TabStripMemory {
  return { offsets: new Map(), orderKey: null, painted: false };
}

/** The shell's records, one per group. */
export type TabStripMemories = Map<ChatGroupId, TabStripMemory>;

/** This group's record, created on first use. */
export function tabStripMemoryFor(store: TabStripMemories, groupId: ChatGroupId): TabStripMemory {
  let record = store.get(groupId);
  if (!record) {
    record = createTabStripMemory();
    store.set(groupId, record);
  }
  return record;
}

/** Drop the records of groups that are no longer in the layout. */
export function forgetTabStripMemories(
  store: TabStripMemories,
  liveGroupIds: readonly ChatGroupId[]
): void {
  const live = new Set(liveGroupIds);
  for (const groupId of [...store.keys()]) {
    if (!live.has(groupId)) store.delete(groupId);
  }
}
