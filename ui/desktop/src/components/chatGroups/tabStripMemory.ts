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
 * jumped instead of sliding (and each new strip started scrolled to its start,
 * V2-R7-1). The shell owns one record per group for as long as it is mounted
 * (`ChatGroupsShell`), so a reload, a new window, a tear-off and a return to the
 * chats route still start from nothing and appear at rest.
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
  /**
   * Where the strip was scrolled, written on every scroll and as a mount goes, or null before
   * the first. A new mount puts it back before it reveals the active tab, so a tab switch on a
   * strip that overflows leaves the strip where it was and the clicked tab under the pointer
   * (vision V2-R7-1: every mount started at scroll 0, the reveal then carried the clicked tab to
   * the strip's end, and a second click landed on another tab).
   */
  scrollLeft: number | null;
  /**
   * Whether the strip showed its ▾ when it last rendered. A new mount starts with it, so the
   * strip already has the width its scroll was taken at when the scroll is put back: without it
   * the first layout had no ▾, the strip was 28px wider, and a strip scrolled to its end came
   * back 28px short of it.
   */
  overflowMenu: boolean;
}

export function createTabStripMemory(): TabStripMemory {
  return {
    offsets: new Map(),
    orderKey: null,
    painted: false,
    scrollLeft: null,
    overflowMenu: false,
  };
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
