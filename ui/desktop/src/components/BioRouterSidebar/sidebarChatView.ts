import type { SessionSummary } from '../../api';
import { chatDateBucket, type ChatDateBucket } from '../../utils/chatDateBuckets';
import { isDefaultSessionName } from '../../utils/sessionNameSync';
import { isCrewTask } from '../chats/chatKind';
import { historyRowTitle } from '../sessions/historyRow';

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

/**
 * The title a chat row shows, before the "Untitled chat" fallback: the one
 * resolver History uses (`historyRowTitle`), so a chat says its kind once, in
 * its glyph (vision V1-R3-6). A scheduled run, stored as `Scheduled job: <job
 * id>`, reads as the job id beside the clock; an app's chat, stored as
 * `app:<app id>`, reads as the app's name. Keyed on the kind, not the prefix, so
 * a chat a person titled "Scheduled job: x" keeps its title.
 *
 * Display only: the stored name is what a rename starts from and what an
 * export is titled, and the Meditation job skips scheduled runs by the prefix.
 */
export function sidebarRowTitle(session: SessionSummary): string {
  return historyRowTitle(session).trim();
}

const NAME_COLLATOR = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });

/**
 * Sorted, with each row's sort key read ONCE (R-28). The comparator used to
 * parse both rows' timestamps on every comparison, about 44,000 `Date.parse`
 * calls to sort 2,000 chats, which every view switch paid again.
 */
export function sortSidebarChats(
  sessions: readonly SessionSummary[],
  sortBy: SidebarSortBy
): SessionSummary[] {
  if (sortBy === 'name') {
    const keyed = sessions.map((session) => ({
      session,
      isDefault: isDefaultSessionName(session.name?.trim()),
      // What the row SHOWS: a scheduled run files under its job id, not under
      // "Scheduled job:" (`sidebarRowTitle`).
      name: historyRowTitle(session),
    }));
    // A to Z, numbers in numeric order, case ignored; default names ("New chat") last.
    keyed.sort((left, right) => {
      if (left.isDefault !== right.isDefault) return left.isDefault ? 1 : -1;
      return (
        NAME_COLLATOR.compare(left.name, right.name) ||
        left.session.id.localeCompare(right.session.id)
      );
    });
    return keyed.map((entry) => entry.session);
  }
  const time = sortBy === 'created' ? createdTime : activityTime;
  const keyed = sessions.map((session) => ({ session, time: time(session) }));
  keyed.sort(
    (left, right) => right.time - left.time || left.session.id.localeCompare(right.session.id)
  );
  return keyed.map((entry) => entry.session);
}

/**
 * `groupByChatDate`'s buckets, asked once per calendar day instead of once
 * per chat (R-28). A month bucket's label comes from a new
 * `Intl.DateTimeFormat`, which made date grouping 2,000 chats cost 55 ms;
 * the bucket depends only on the local calendar day (and `now`), so a sorted
 * list asks for a handful. Same groups, labels and order as `groupByChatDate`,
 * which a test pins.
 */
export function groupSidebarChatsByDate(
  sorted: readonly SessionSummary[],
  timeOf: (session: SessionSummary) => number,
  now: number
): Array<{ key: string; label: string; items: SessionSummary[] }> {
  const bucketByDay = new Map<number, ChatDateBucket>();
  const groups = new Map<
    string,
    { key: string; label: string; rank: number; items: SessionSummary[] }
  >();
  for (const session of sorted) {
    const time = timeOf(session);
    const date = new Date(time);
    const day = date.getFullYear() * 10_000 + date.getMonth() * 100 + date.getDate();
    let bucket = bucketByDay.get(day);
    if (!bucket) {
      bucket = chatDateBucket(time, now);
      bucketByDay.set(day, bucket);
    }
    const existing = groups.get(bucket.key);
    if (existing) existing.items.push(session);
    else
      groups.set(bucket.key, {
        key: bucket.key,
        label: bucket.label,
        rank: bucket.rank,
        items: [session],
      });
  }
  return [...groups.values()]
    .sort((left, right) => left.rank - right.rank)
    .map(({ key, label, items }) => ({ key, label, items }));
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

  const timeOf = view.sortBy === 'created' ? createdTime : activityTime;
  if (view.groupBy === 'date') {
    return groupSidebarChatsByDate(sorted, timeOf, now).map((group) => ({
      key: group.key,
      kind: 'date' as const,
      label: group.label,
      sessions: group.items,
    }));
  }

  const byFolder = new Map<string, { sessions: SessionSummary[]; latest: number }>();
  for (const session of sorted) {
    const folder = normalizeFolder(session.working_dir);
    const time = timeOf(session);
    const existing = byFolder.get(folder);
    if (existing) {
      existing.sessions.push(session);
      if (time > existing.latest) existing.latest = time;
    } else {
      byFolder.set(folder, { sessions: [session], latest: time });
    }
  }
  const labels = folderLabels([...byFolder.keys()], options.homeDir);
  const groups = [...byFolder.entries()].map(([folder, { sessions: members, latest }]) => ({
    latest,
    group: {
      key: `folder:${folder}`,
      kind: 'folder' as const,
      label: labels.get(folder) ?? folder,
      path: folder ? tildePath(folder, options.homeDir) : undefined,
      workingDir: folder || undefined,
      isCrew: Boolean(folder) && isCrewTaskFolder(folder),
      sessions: members,
    } satisfies SidebarChatGroup,
  }));

  groups.sort((left, right) =>
    view.sortBy === 'name'
      ? NAME_COLLATOR.compare(left.group.label, right.group.label) ||
        left.group.key.localeCompare(right.group.key)
      : right.latest - left.latest || left.group.key.localeCompare(right.group.key)
  );
  return groups.map((entry) => entry.group);
}

const arrangementCache = new WeakMap<readonly SessionSummary[], Map<string, SidebarChatGroup[]>>();

/**
 * {@link arrangeSidebarChats}, kept for as long as the list it arranged is held
 * (R-28, vision V1-R2-2): a switch back to a view already drawn is a lookup,
 * and so is the second call a development build makes. The key is the list
 * itself (a fresh read, or any change to a row, is a new array), the view, the
 * home folder and the local calendar day, the only part of `now` an
 * arrangement reads (date buckets are calendar days).
 *
 * ⚠ The list must not be changed in place; React state never is. The groups it
 * returns are shared between callers and must not be changed either.
 */
export function arrangeSidebarChatsCached(
  sessions: readonly SessionSummary[],
  view: SidebarChatView,
  now: number = Date.now(),
  options: ArrangeOptions = {}
): SidebarChatGroup[] {
  const today = new Date(now);
  const key = [
    view.groupBy,
    view.sortBy,
    options.homeDir ?? '',
    view.groupBy === 'date' ? today.toDateString() : '',
  ].join('\u0000');
  let byView = arrangementCache.get(sessions);
  if (!byView) {
    byView = new Map();
    arrangementCache.set(sessions, byView);
  }
  let arranged = byView.get(key);
  if (!arranged) {
    arranged = arrangeSidebarChats(sessions, view, now, options);
    byView.set(key, arranged);
  }
  return arranged;
}

// ── Draw what can be seen ─────────────────────────────────────────────────

/** Lines the list draws at first: more than a tall window shows. */
export const SIDEBAR_RENDER_WINDOW = 60;
/** Lines added each time the list is scrolled near its end. */
export const SIDEBAR_RENDER_WINDOW_STEP = 120;
/** One drawn line: a 28px row and the 2px between rows (`.br-nav-list`). */
export const SIDEBAR_LINE_HEIGHT_PX = 30;
/** Lines drawn past the list's bottom edge, so a scroll has rows to show. */
export const SIDEBAR_RENDER_OVERSCAN = 12;

/**
 * Lines a view draws at first in a list `listHeight` px tall (R-28, vision
 * V1-R2-2): what the list shows plus the overscan, and never more than
 * {@link SIDEBAR_RENDER_WINDOW}. A view switch mounts every line it draws, so
 * its cost grows with them: a 1440 x 900 window drew 60 lines to show 21 rows.
 * A list not measured yet (0, or no layout at all) draws the full window; the
 * list still grows on scroll, and a list its lines do not fill draws more.
 */
export function sidebarRenderWindow(listHeight: number): number {
  if (!Number.isFinite(listHeight) || listHeight <= 0) return SIDEBAR_RENDER_WINDOW;
  return Math.min(
    SIDEBAR_RENDER_WINDOW,
    Math.ceil(listHeight / SIDEBAR_LINE_HEIGHT_PX) + SIDEBAR_RENDER_OVERSCAN
  );
}

/**
 * The first `budget` lines of a drawn list (R-28). A line is a group's header
 * (`headerLines` says how many a group draws: a date or folder header is one,
 * the ungrouped list none), a row, or a folder's "Show more".
 *
 * Every row a view arranges used to be drawn at once, and a row is a context
 * menu, a tooltip and a rename editor: 2,000 of them took 2 to 4 s to mount in
 * the development build on every switch of Group by or Sort by. The whole list
 * is still held and arranged, so the order is right; only drawing waits until
 * the list is scrolled. `cut` says lines were left out, so the list grows its
 * window before it asks the daemon for another page.
 */
export function windowSidebarRows<Entry extends { rows: readonly unknown[]; more: unknown }>(
  entries: readonly Entry[],
  budget: number,
  headerLines: (entry: Entry) => number
): { entries: Entry[]; cut: boolean } {
  const drawn: Entry[] = [];
  let left = budget;
  for (const entry of entries) {
    const header = headerLines(entry);
    if (header + entry.rows.length === 0 && !entry.more) continue;
    // No room left, or room for a header and none of its rows: stop before it,
    // so the list never ends on a header that heads nothing.
    if (left <= 0 || (entry.rows.length > 0 && left <= header)) {
      return { entries: drawn, cut: true };
    }
    left -= header;
    const rows = Math.max(0, Math.min(entry.rows.length, left));
    left -= rows;
    if (rows < entry.rows.length) {
      drawn.push({ ...entry, rows: entry.rows.slice(0, rows), more: null });
      return { entries: drawn, cut: true };
    }
    if (entry.more) {
      if (left <= 0) {
        drawn.push({ ...entry, more: null });
        return { entries: drawn, cut: true };
      }
      left -= 1;
    }
    drawn.push(entry);
  }
  return { entries: drawn, cut: false };
}

/**
 * How many lines `windowSidebarRows` must be given to draw `sessionId`'s row,
 * or null when no row of that chat is in `entries`. A rename editor mounts in
 * its row; a row that is not drawn has nowhere to put it (R-28).
 */
export function linesToDrawRow<Entry extends { rows: readonly { id: string }[]; more: unknown }>(
  entries: readonly Entry[],
  sessionId: string,
  headerLines: (entry: Entry) => number
): number | null {
  let lines = 0;
  for (const entry of entries) {
    lines += headerLines(entry);
    const index = entry.rows.findIndex((row) => row.id === sessionId);
    if (index >= 0) return lines + index + 1;
    lines += entry.rows.length + (entry.more ? 1 : 0);
  }
  return null;
}

// ── Draw only the lines near the viewport ────────────────────────────────

/**
 * The lines the list draws for real, `[from, to)`, counted the way
 * {@link windowSidebarRows} counts them (a header, a row, a "Show more").
 */
export interface SidebarDrawRange {
  from: number;
  to: number;
}

/** One line of a group as the list draws it: a row, its "Show more", or a run of neither. */
export type SidebarDrawnLine<Row> =
  | { kind: 'row'; row: Row }
  | { kind: 'more' }
  /**
   * `lines` rows (or a "Show more") held but not drawn: ONE placeholder exactly
   * their height, so the scroll height and every drawn row's place are what
   * the real rows would give (`.br-chat-row-gap`).
   */
  | { kind: 'gap'; lines: number; key: string };

export type SidebarSlicedEntry<Entry extends { rows: readonly unknown[]; more: unknown }> = Omit<
  Entry,
  'rows' | 'more'
> & {
  /** The rows drawn for real, in order (the roving keys). */
  rows: Array<Entry['rows'][number]>;
  /** The group's "Show more", or null when it has none or it is not drawn. */
  more: Entry['more'] | null;
  /** False: the header's line is outside the range; draw a placeholder of its height. */
  headerDrawn: boolean;
  /** The group's rows and "Show more" in order, runs not drawn folded into gaps. */
  lines: Array<SidebarDrawnLine<Entry['rows'][number]>>;
  /** The line index of the group's first row (after its header). */
  rowsLine: number;
};

/**
 * Which lines of a windowed list are drawn for real (R-28, vision V1-R2-2
 * round 4). The list grew as it was scrolled and never shrank, so after a
 * scroll to its end every chat was a mounted row: with 5,542 drawn, a
 * right-click spent about 2 s restyling them and a view switch 0.5 s
 * unmounting them. Rows and "Show more" lines outside `range` are folded into
 * placeholders the height of the lines they stand for, and a header outside it
 * says so (`headerDrawn` false): the list draws an empty box of its height for
 * a folder header (a tooltip, a + and a menu button each) and draws a date
 * header anyway (one element).
 *
 * Always drawn whatever the range: every row `keep` names (the focused row, the
 * current chat, a row being renamed or whose menu is open), and every folder
 * header or "Show more" `keepLine` names (focused, or its menu open), so focus,
 * an open menu and an editor are never unmounted under the person, and the
 * single Tab stop stays on the current chat. Each sits on its own line between
 * the gaps, so the rows drawn read in the list's order; the roving keys bring
 * the list back to a focused line before they move from it (`RecentChats`). A
 * null range (the list not measured yet, or no layout at all, as in jsdom)
 * draws every line.
 */
export function sliceSidebarLines<Entry extends { rows: readonly unknown[]; more: unknown }>(
  entries: readonly Entry[],
  range: SidebarDrawRange | null,
  headerLines: (entry: Entry) => number,
  keep: (row: Entry['rows'][number]) => boolean = () => false,
  keepLine: (entry: Entry, line: 'header' | 'more') => boolean = () => false
): Array<SidebarSlicedEntry<Entry>> {
  type Row = Entry['rows'][number];
  const inRange = (line: number) => range === null || (line >= range.from && line < range.to);
  const sliced: Array<SidebarSlicedEntry<Entry>> = [];
  let line = 0;
  for (const entry of entries) {
    const header = headerLines(entry);
    const headerDrawn = header === 0 || inRange(line) || keepLine(entry, 'header');
    line += header;
    const rowsLine = line;
    const lines: Array<SidebarDrawnLine<Row>> = [];
    const rows: Row[] = [];
    let gap = 0;
    let gapStart = 0;
    const closeGap = () => {
      if (gap > 0) lines.push({ kind: 'gap', lines: gap, key: `gap:${gapStart}` });
      gap = 0;
    };
    entry.rows.forEach((row, index) => {
      if (inRange(line) || keep(row)) {
        closeGap();
        lines.push({ kind: 'row', row });
        rows.push(row);
      } else {
        if (gap === 0) gapStart = index;
        gap += 1;
      }
      line += 1;
    });
    let more: Entry['more'] | null = null;
    if (entry.more) {
      if (inRange(line) || keepLine(entry, 'more')) {
        closeGap();
        lines.push({ kind: 'more' });
        more = entry.more;
      } else {
        if (gap === 0) gapStart = entry.rows.length;
        gap += 1;
      }
      line += 1;
    }
    closeGap();
    sliced.push({ ...entry, rows, more, headerDrawn, lines, rowsLine });
  }
  return sliced;
}

/**
 * Where a drawn group sits in the list, in the scroll container's content
 * coordinates (px): read from the DOM by the list, so the line math below
 * never assumes a header's or a gap's height.
 */
export interface SidebarGroupBox {
  /** The line index of the group's first row. */
  rowsLine: number;
  /** Lines the group's header takes above its rows. */
  headerLines: number;
  /** The top of the group's box, and of its first row (its box's bottom when it has none). */
  top: number;
  rowsTop: number;
}

/**
 * The first and last line inside `[viewTop, viewBottom]`, from the drawn
 * groups' boxes: rows and gaps sit on one pitch below their group's first row,
 * so a line is exact without measuring a row. `boxAt` is asked only for the
 * groups a binary search visits, so a list of 1,200 folder groups reads about
 * twenty boxes, not every one. Null when nothing can be measured.
 */
export function sidebarLinesInView(
  count: number,
  boxAt: (index: number) => SidebarGroupBox,
  viewTop: number,
  viewBottom: number,
  pitch: number = SIDEBAR_LINE_HEIGHT_PX
): { first: number; last: number } | null {
  if (count <= 0 || !(viewBottom > viewTop) || !(pitch > 0)) return null;
  const lineAt = (y: number) => {
    let low = 0;
    let high = count - 1;
    let found = -1;
    while (low <= high) {
      const middle = (low + high) >> 1;
      if (boxAt(middle).top <= y) {
        found = middle;
        low = middle + 1;
      } else {
        high = middle - 1;
      }
    }
    const box = boxAt(Math.max(0, found));
    if (found < 0 || y < box.rowsTop) return Math.max(0, box.rowsLine - box.headerLines);
    return box.rowsLine + Math.floor((y - box.rowsTop) / pitch);
  };
  return { first: lineAt(viewTop), last: lineAt(viewBottom) };
}

/**
 * The range to draw for the lines in view: those lines plus the overscan on
 * each side, kept as it is (the same object, so the list does not re-render)
 * while the lines in view stay at least half an overscan inside it. A null
 * `inView` (the list could not be measured) keeps whatever was drawn.
 */
export function nextSidebarDrawRange(
  current: SidebarDrawRange | null,
  inView: { first: number; last: number } | null,
  overscan: number = SIDEBAR_RENDER_OVERSCAN
): SidebarDrawRange | null {
  if (!inView) return current;
  const wanted = { from: Math.max(0, inView.first - overscan), to: inView.last + 1 + overscan };
  if (current) {
    const margin = Math.floor(overscan / 2);
    const coversTop = current.from <= Math.max(0, inView.first - margin);
    const coversBottom = current.to >= inView.last + 1 + margin;
    const bounded = current.to - current.from <= wanted.to - wanted.from + overscan;
    if (coversTop && coversBottom && bounded) return current;
  }
  return wanted;
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
