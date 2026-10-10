import type { SessionSummary } from '../../api';
import { groupByChatDate } from '../../utils/chatDateBuckets';
import { isDefaultSessionName } from '../../utils/sessionNameSync';
import { isCrewTask } from '../chats/chatKind';

/**
 * How the sidebar arranges its chats (owner message 3): grouped by date (the
 * default), by folder, or not at all, and sorted by last activity (the
 * default), by creation or by name. Pure, so every combination is tested
 * without a DOM; `RecentChats` only draws what this returns.
 */
export type SidebarGroupBy = 'date' | 'folder' | 'none';
export type SidebarSortBy = 'activity' | 'created' | 'name';

export interface SidebarChatView {
  groupBy: SidebarGroupBy;
  sortBy: SidebarSortBy;
}

export const SIDEBAR_GROUP_BY_OPTIONS: readonly SidebarGroupBy[] = ['date', 'folder', 'none'];
export const SIDEBAR_SORT_BY_OPTIONS: readonly SidebarSortBy[] = ['activity', 'created', 'name'];

export const DEFAULT_SIDEBAR_CHAT_VIEW: SidebarChatView = { groupBy: 'date', sortBy: 'activity' };

export function isDefaultSidebarChatView(view: SidebarChatView): boolean {
  return (
    view.groupBy === DEFAULT_SIDEBAR_CHAT_VIEW.groupBy &&
    view.sortBy === DEFAULT_SIDEBAR_CHAT_VIEW.sortBy
  );
}

export interface SidebarChatGroup {
  /** Stable key: a date bucket key, `folder:<path>`, or `all`. */
  key: string;
  kind: 'date' | 'folder' | 'all';
  /** The header text. Empty for the one ungrouped list, which has no header. */
  label: string;
  /** Folder groups: the full path with `~` for home, for the header's tooltip. */
  path?: string;
  /** Folder groups: the raw `working_dir`, for "New chat in this folder" and "Copy folder path". */
  workingDir?: string;
  /** Folder groups: the Crew task folder, which offers no new chat. */
  isCrew?: boolean;
  sessions: SessionSummary[];
}

export interface ArrangeOptions {
  /** The person's home folder, so it reads as `~`. */
  homeDir?: string | null;
}

// ── Times and names ───────────────────────────────────────────────────────

function parseTime(value: string | null | undefined): number {
  const parsed = value ? Date.parse(value) : Number.NaN;
  return Number.isNaN(parsed) ? 0 : parsed;
}

/** Last activity: `updated_at`, falling back to `created_at`. What the server orders by. */
export function activityTime(session: SessionSummary): number {
  return parseTime(session.updated_at) || parseTime(session.created_at);
}

export function createdTime(session: SessionSummary): number {
  return parseTime(session.created_at) || parseTime(session.updated_at);
}

const NAME_COLLATOR = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });

/** A to Z, numbers in numeric order, case ignored; default names ("New chat") last. */
function compareByName(left: SessionSummary, right: SessionSummary): number {
  const leftDefault = isDefaultSessionName(left.name?.trim());
  const rightDefault = isDefaultSessionName(right.name?.trim());
  if (leftDefault !== rightDefault) return leftDefault ? 1 : -1;
  return (
    NAME_COLLATOR.compare(left.name ?? '', right.name ?? '') || left.id.localeCompare(right.id)
  );
}

function compareBy(sortBy: SidebarSortBy): (left: SessionSummary, right: SessionSummary) => number {
  if (sortBy === 'name') return compareByName;
  const time = sortBy === 'created' ? createdTime : activityTime;
  return (left, right) => time(right) - time(left) || left.id.localeCompare(right.id);
}

export function sortSidebarChats(
  sessions: readonly SessionSummary[],
  sortBy: SidebarSortBy
): SessionSummary[] {
  return [...sessions].sort(compareBy(sortBy));
}

// ── Folders ───────────────────────────────────────────────────────────────

/** Trailing separators off, except on a root (`/`, `C:\`). */
export function normalizeFolder(path: string | null | undefined): string {
  const trimmed = (path ?? '').trim();
  if (!trimmed) return '';
  const stripped = trimmed.replace(/[\\/]+$/, '');
  return stripped || trimmed.slice(0, 1);
}

function segmentsOf(path: string): string[] {
  return path.split(/[\\/]+/).filter(Boolean);
}

/** The path with the home folder written as `~`. */
export function tildePath(path: string, homeDir?: string | null): string {
  const home = normalizeFolder(homeDir);
  if (!home || !path) return path;
  if (path === home) return '~';
  if (path.startsWith(`${home}/`) || path.startsWith(`${home}\\`)) {
    return `~${path.slice(home.length)}`;
  }
  return path;
}

/**
 * The Crew task folder. Every Crew task runs in `<data dir>/crew/tasks`, whose
 * basename "tasks" says nothing, so the group is labelled "Crew". The test is
 * the crew chat kind's own folder test (`chats/chatKind.ts`), asked with the
 * folder alone so a title never decides it.
 */
export function isCrewTaskFolder(folder: string): boolean {
  return Boolean(folder) && isCrewTask({ working_dir: folder });
}

export const NO_FOLDER_LABEL = 'No folder';
export const CREW_FOLDER_LABEL = 'Crew';

/**
 * Short labels for a set of folders: the basename, `~` for home, `Crew` for
 * the Crew task folder, and `parent/basename` where two basenames collide (the
 * full `~` path where even that collides).
 */
export function folderLabels(
  folders: readonly string[],
  homeDir?: string | null
): Map<string, string> {
  const home = normalizeFolder(homeDir);
  const labels = new Map<string, string>();
  const plain: string[] = [];
  for (const folder of folders) {
    if (!folder) labels.set(folder, NO_FOLDER_LABEL);
    else if (home && folder === home) labels.set(folder, '~');
    else if (isCrewTaskFolder(folder)) labels.set(folder, CREW_FOLDER_LABEL);
    else plain.push(folder);
  }

  const byBasename = new Map<string, string[]>();
  for (const folder of plain) {
    const segments = segmentsOf(folder);
    const basename = segments[segments.length - 1] ?? folder;
    byBasename.set(basename, [...(byBasename.get(basename) ?? []), folder]);
  }
  for (const [basename, members] of byBasename) {
    if (members.length === 1) {
      labels.set(members[0], basename);
      continue;
    }
    const withParent = members.map((folder) => {
      const segments = segmentsOf(folder);
      return segments.length > 1 ? `${segments[segments.length - 2]}/${basename}` : basename;
    });
    members.forEach((folder, index) => {
      const label = withParent[index];
      const unique = withParent.filter((other) => other === label).length === 1;
      labels.set(folder, unique ? label : tildePath(folder, homeDir));
    });
  }
  return labels;
}

// ── Arrange ───────────────────────────────────────────────────────────────

/**
 * Arrange the sidebar's chats for a view (audit shell-sidebar §2.3):
 *
 * | group × sort | groups | group order | rows |
 * |---|---|---|---|
 * | date × activity | bucket of `updated_at` | newest first | `updated_at` desc |
 * | date × created | bucket of `created_at` | newest first | `created_at` desc |
 * | date × name | bucket of `updated_at` | newest first | name A to Z, default names last |
 * | folder × activity | `working_dir` | latest `updated_at` desc | `updated_at` desc |
 * | folder × created | `working_dir` | latest `created_at` desc | `created_at` desc |
 * | folder × name | `working_dir` | label A to Z | name A to Z |
 * | none × any | one list, no header | | by the sort |
 *
 * Ties break on the id, so a re-render never reorders equal rows.
 */
export function arrangeSidebarChats(
  sessions: readonly SessionSummary[],
  view: SidebarChatView,
  now: number = Date.now(),
  options: ArrangeOptions = {}
): SidebarChatGroup[] {
  const sorted = sortSidebarChats(sessions, view.sortBy);

  if (view.groupBy === 'none') {
    return sorted.length ? [{ key: 'all', kind: 'all', label: '', sessions: sorted }] : [];
  }

  if (view.groupBy === 'date') {
    const timeOf = view.sortBy === 'created' ? createdTime : activityTime;
    return groupByChatDate(sorted, timeOf, now).map((group) => ({
      key: group.key,
      kind: 'date' as const,
      label: group.label,
      sessions: group.items,
    }));
  }

  const byFolder = new Map<string, SessionSummary[]>();
  for (const session of sorted) {
    const folder = normalizeFolder(session.working_dir);
    byFolder.set(folder, [...(byFolder.get(folder) ?? []), session]);
  }
  const labels = folderLabels([...byFolder.keys()], options.homeDir);
  const groups: SidebarChatGroup[] = [...byFolder.entries()].map(([folder, members]) => ({
    key: `folder:${folder}`,
    kind: 'folder',
    label: labels.get(folder) ?? folder,
    path: folder ? tildePath(folder, options.homeDir) : undefined,
    workingDir: folder || undefined,
    isCrew: Boolean(folder) && isCrewTaskFolder(folder),
    sessions: members,
  }));

  const latest = (group: SidebarChatGroup) =>
    Math.max(...group.sessions.map(view.sortBy === 'created' ? createdTime : activityTime));
  return groups.sort((left, right) =>
    view.sortBy === 'name'
      ? NAME_COLLATOR.compare(left.label, right.label) || left.key.localeCompare(right.key)
      : latest(right) - latest(left) || left.key.localeCompare(right.key)
  );
}

// ── Never move a row under the pointer ──────────────────────────────────────

/** The order the list showed when the pointer came in: groups and the ids in each. */
export interface HeldArrangement {
  view: SidebarChatView;
  groups: Array<Omit<SidebarChatGroup, 'sessions'> & { ids: string[] }>;
}

export function holdArrangement(
  groups: readonly SidebarChatGroup[],
  view: SidebarChatView
): HeldArrangement {
  return {
    view,
    groups: groups.map(({ sessions, ...group }) => ({
      ...group,
      ids: sessions.map((session) => session.id),
    })),
  };
}

/**
 * Principle 9: a live re-sort or insert that would shift rows beneath a
 * hovering pointer waits until the pointer leaves the list. Under "Last
 * activity" a running chat moves to the top; while the pointer is over the
 * list the rows keep the order they had, so a click never lands on a row that
 * moved under it.
 *
 * What still changes while held: a row's own content (a rename, a privacy
 * mark), a row that was removed, and rows that arrive BELOW every held row
 * (the next page of a scrolled list), which shift nothing above them. A row
 * that would arrive above or between held rows waits.
 */
export function applyHeldArrangement(
  held: HeldArrangement,
  live: readonly SidebarChatGroup[]
): SidebarChatGroup[] {
  const liveFlat = live.flatMap((group) => group.sessions.map((session) => ({ group, session })));
  const byId = new Map(liveFlat.map((entry) => [entry.session.id, entry.session]));
  const heldIds = new Set(held.groups.flatMap((group) => group.ids));

  let lastHeld = -1;
  liveFlat.forEach((entry, index) => {
    if (heldIds.has(entry.session.id)) lastHeld = index;
  });

  const result: SidebarChatGroup[] = held.groups
    .map(({ ids, ...group }) => ({
      ...group,
      sessions: ids.flatMap((id) => {
        const session = byId.get(id);
        return session ? [session] : [];
      }),
    }))
    .filter((group) => group.sessions.length > 0);

  for (const { group, session } of liveFlat.slice(lastHeld + 1)) {
    if (heldIds.has(session.id)) continue;
    const last = result[result.length - 1];
    if (last && last.key === group.key) {
      last.sessions.push(session);
      continue;
    }
    const taken = result.some((existing) => existing.key === group.key);
    result.push({
      ...group,
      key: taken ? `${group.key}:after` : group.key,
      sessions: [session],
    });
  }
  return result;
}

// ── The stored view ───────────────────────────────────────────────────────

export const SIDEBAR_CHAT_VIEW_STORAGE_KEY = 'biorouter:sidebar-chat-view';
export const SIDEBAR_COLLAPSED_FOLDERS_STORAGE_KEY = 'biorouter:sidebar-collapsed-folders';
export const SIDEBAR_COLLAPSED_FOLDERS_MAX = 200;

type ViewStorage = Pick<Storage, 'getItem' | 'setItem'>;

function defaultStorage(): ViewStorage | null {
  try {
    return typeof window === 'undefined' ? null : window.localStorage;
  } catch {
    return null;
  }
}

/**
 * Any value into a valid view. Unknown fields fall back to the defaults one by
 * one, so a value written by a later build (a new sort) keeps the grouping it
 * can still read.
 */
export function clampSidebarChatView(raw: unknown): SidebarChatView {
  const record = raw && typeof raw === 'object' ? (raw as Record<string, unknown>) : {};
  const groupBy = SIDEBAR_GROUP_BY_OPTIONS.includes(record.groupBy as SidebarGroupBy)
    ? (record.groupBy as SidebarGroupBy)
    : DEFAULT_SIDEBAR_CHAT_VIEW.groupBy;
  const sortBy = SIDEBAR_SORT_BY_OPTIONS.includes(record.sortBy as SidebarSortBy)
    ? (record.sortBy as SidebarSortBy)
    : DEFAULT_SIDEBAR_CHAT_VIEW.sortBy;
  return { groupBy, sortBy };
}

/** Parse a stored value (the `storage` event hands over the raw string too). */
export function parseSidebarChatView(stored: string | null): SidebarChatView {
  if (stored === null) return DEFAULT_SIDEBAR_CHAT_VIEW;
  try {
    return clampSidebarChatView(JSON.parse(stored));
  } catch {
    return DEFAULT_SIDEBAR_CHAT_VIEW;
  }
}

export function readSidebarChatView(
  storage: ViewStorage | null = defaultStorage()
): SidebarChatView {
  if (!storage) return DEFAULT_SIDEBAR_CHAT_VIEW;
  try {
    return parseSidebarChatView(storage.getItem(SIDEBAR_CHAT_VIEW_STORAGE_KEY));
  } catch {
    return DEFAULT_SIDEBAR_CHAT_VIEW;
  }
}

export function writeSidebarChatView(
  view: SidebarChatView,
  storage: ViewStorage | null = defaultStorage()
): void {
  if (!storage) return;
  const { groupBy, sortBy } = clampSidebarChatView(view);
  try {
    storage.setItem(SIDEBAR_CHAT_VIEW_STORAGE_KEY, JSON.stringify({ v: 1, groupBy, sortBy }));
  } catch {
    // Storage is full or blocked; the view still changes for this window.
  }
}

/** The folders the person collapsed, newest last, at most {@link SIDEBAR_COLLAPSED_FOLDERS_MAX}. */
export function parseCollapsedFolders(stored: string | null): string[] {
  if (stored === null) return [];
  try {
    const parsed: unknown = JSON.parse(stored);
    if (!Array.isArray(parsed)) return [];
    return parsed
      .filter((entry): entry is string => typeof entry === 'string')
      .slice(-SIDEBAR_COLLAPSED_FOLDERS_MAX);
  } catch {
    return [];
  }
}

export function readCollapsedFolders(storage: ViewStorage | null = defaultStorage()): string[] {
  if (!storage) return [];
  try {
    return parseCollapsedFolders(storage.getItem(SIDEBAR_COLLAPSED_FOLDERS_STORAGE_KEY));
  } catch {
    return [];
  }
}

export function writeCollapsedFolders(
  folders: readonly string[],
  storage: ViewStorage | null = defaultStorage()
): void {
  if (!storage) return;
  try {
    storage.setItem(
      SIDEBAR_COLLAPSED_FOLDERS_STORAGE_KEY,
      JSON.stringify([...new Set(folders)].slice(-SIDEBAR_COLLAPSED_FOLDERS_MAX))
    );
  } catch {
    // Best effort, like the view itself.
  }
}
