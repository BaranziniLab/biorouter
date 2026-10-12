import { getSession, type Session } from '../../api';
import { userActionHeaders } from '../../utils/userAction';
import type { AppManifest } from './appManagement';
import { appsCopy } from './copy';

const copy = appsCopy.buildChat;

/**
 * Where "Open the chat where this app was built" leads, decided by asking the
 * daemon rather than trusting `manifest.session_id`.
 *
 * # Why the id alone is not enough
 *
 * `create_app` records the id of the chat that called it and nothing keeps it
 * current: the chat can be deleted, and a store restored from a backup can issue
 * the same id again. An app installed from an export made before exports dropped
 * `session_id` (`agent_drafter/export.rs`) carries its author's id to a machine
 * where it names nothing, or names a different chat. The button used to navigate
 * on the bare id, so a deleted chat opened a tab that failed with "Failed to
 * resume session: Session not found", and the tab was saved, so it came back on
 * every reload.
 *
 * # The read
 *
 * `GET /sessions/{id}?metadata_only=true` is the gated singular read the chat-tab
 * shell already uses. It carries the user-action proof, so the reach gate answers
 * honestly: 404 for a chat that is gone, 403 for one this surface may not open.
 * Without the proof it would answer 403 for both (a missing chat is treated as
 * unreadable, so an unproven caller cannot tell private from deleted) — which is
 * why the proof is not optional here. `/agent/resume` sits behind the same gate,
 * so a 403 here means a click would be refused there too.
 *
 * ⚠ A 404 is strong evidence, not proof. The route has answered 404 for any
 * store error as well as for a missing row (`routes/session.rs`, `get_session`),
 * and a daemon of that vintage can be on the other end after the route is
 * fixed. So a rare store failure can read as "deleted". That is one reason a
 * deleted row's control stays clickable: the click asks again, and opens the
 * chat if the answer is now a 200.
 *
 * ⚠ The listing (`GET /sessions`) is the wrong tool and must not replace this: it
 * INNER JOINs messages and leaves out hidden and terminal sessions, so it would
 * report chats that exist as deleted.
 *
 * # What "unknown" means
 *
 * No response, a 5xx, a 408 or 429 (try later), or any status this file does not
 * name is a question nobody answered. The control stays enabled and the click
 * re-checks, because a daemon that is restarting must not make every row read
 * "deleted". Only the click refuses to navigate on it.
 */
export type BuildChatState =
  | { kind: 'none' }
  | { kind: 'checking' }
  | {
      kind: 'available';
      sessionId: string;
      title?: string;
      userSetName?: boolean;
      /**
       * `self`: the recorded chat. `parent`: the recorded chat was a subagent
       * run and this is the conversation that delegated it. `subagent`: the
       * recorded subagent run itself, because its parent could not be opened.
       */
      via: 'self' | 'parent' | 'subagent';
    }
  | { kind: 'deleted' }
  | { kind: 'replaced' }
  | { kind: 'unreachable' }
  | { kind: 'unknown' };

/**
 * How far a chat's times may sit on the wrong side of the app's `created_at`
 * and still be the chat that built it: born up to this much after, or last
 * written up to this much before. The app's time is whole Unix seconds and the
 * chat's come from a different writer (SQLite's one-second `datetime('now')`),
 * so this is rounding slack, not a window anyone relies on.
 */
export const BUILD_CHAT_CLOCK_SLACK_SECS = 300;

/** One read of a chat row, already sorted into what the button needs. */
export type BuildChatRowRead =
  | { kind: 'row'; row: Session }
  | { kind: 'deleted' }
  | { kind: 'unreachable' }
  | { kind: 'unknown' };

export type BuildChatRowReader = (sessionId: string) => Promise<BuildChatRowRead>;

/** Read one chat's row with the user-action proof, and say what the answer was. */
export async function readBuildChatRow(sessionId: string): Promise<BuildChatRowRead> {
  let row: Session | undefined;
  let status: number | undefined;
  try {
    const result = await getSession({
      path: { session_id: sessionId },
      // A name, a type, a parent and a timestamp: nothing here needs the transcript.
      query: { metadata_only: true },
      // ⚠ Not optional; see the file comment. Without it a deleted chat reads 403.
      headers: await userActionHeaders(),
      // No `throwOnError`: the status is the answer, and a throw drops it.
    });
    row = result.data;
    // Absent when the request never completed, whatever the type says.
    status = (result.response as Response | undefined)?.status;
  } catch {
    return { kind: 'unknown' };
  }
  if (status === 200) {
    // ⚠ A 200 that is not this chat's row is not proof of anything — a proxy or
    // an older daemon answering a different shape. Unknown, never "available".
    if (row && typeof row === 'object' && row.id === sessionId) return { kind: 'row', row };
    return { kind: 'unknown' };
  }
  // 400 is the route rejecting the id's syntax before it reads the store: an id
  // that cannot name a chat on any machine (a hand-edited or legacy manifest).
  // From where the user sits that chat is as gone as a deleted one.
  if (status === 404 || status === 400) return { kind: 'deleted' };
  if (status === 403) return { kind: 'unreachable' };
  return { kind: 'unknown' };
}

/** Unix seconds of an ISO timestamp, or undefined when it does not parse. */
function isoSecs(value: string | null | undefined): number | undefined {
  if (!value) return undefined;
  const ms = Date.parse(value);
  return Number.isFinite(ms) ? ms / 1000 : undefined;
}

function createdSecs(row: Session): number | undefined {
  return isoSecs(row.created_at);
}

/**
 * A usable bound, or undefined. ⚠ The daemon serves a manifest with no
 * `created_at` as `0` (`#[serde(default)]` on a `u64`), never as absent, and 0
 * means "unknown" there exactly as it does in the row's date. Read as a real
 * time it made every chat newer than the app, so a hand-edited or legacy
 * manifest grayed out a chat that exists.
 */
function knownSecs(secs: number | null | undefined): number | undefined {
  return typeof secs === 'number' && Number.isFinite(secs) && secs > 0 ? secs : undefined;
}

/**
 * Was `row` created no later than `notAfterSecs` (plus the slack)? A row or a
 * bound with no usable time passes: this is an identity heuristic, and without a
 * time there is nothing to compare, so it must not turn a real chat into a
 * missing one.
 */
function predates(row: Session, notAfterSecs: number | null | undefined): boolean {
  const created = createdSecs(row);
  const bound = knownSecs(notAfterSecs);
  if (created === undefined || bound === undefined) return true;
  return created <= bound + BUILD_CHAT_CLOCK_SLACK_SECS;
}

/**
 * Could `row` be the chat that called `create_app` at `appCreatedSecs`? It must
 * have been born by then, and still written to then: the tool's result is a
 * message, and storing a message stamps the row's `updated_at` (so do the
 * turn's token totals), which nothing later moves back. A chat born after the
 * app is an id issued again; one that went quiet before the app names somebody
 * else's conversation. The same slack applies at both ends, and a missing time
 * passes.
 */
function spansAppCreation(row: Session, appCreatedSecs: number | null | undefined): boolean {
  const bound = knownSecs(appCreatedSecs);
  if (bound === undefined) return true;
  if (!predates(row, bound)) return false;
  const updated = isoSecs(row.updated_at);
  return updated === undefined || updated >= bound - BUILD_CHAT_CLOCK_SLACK_SECS;
}

function availableFrom(row: Session, via: 'self' | 'parent' | 'subagent'): BuildChatState {
  return {
    kind: 'available',
    sessionId: row.id,
    title: row.name || undefined,
    userSetName: row.user_set_name ?? undefined,
    via,
  };
}

/**
 * Resolve one app's build chat.
 *
 * - No recorded id: `none`.
 * - The row exists and spans the app's creation (born by then, still written
 *   to then): `available`. Otherwise it cannot be the chat that built the app,
 *   so that is `replaced`, shown like a deleted chat.
 *   ⚠ This is a heuristic over the manifest's own `created_at`, and it is only
 *   as good as that. It always catches a local id issued again after a backup
 *   restore, because that chat is born after the app. It catches an older
 *   export's id only when the local chat with that id was born after the app or
 *   had gone quiet before it; a local chat that was active across the author's
 *   build time passes, and a manifest written by hand passes whatever it says.
 *   What keeps ids off other machines is the export dropping `session_id`, not
 *   this check. (A hand-made export also ships its own launcher scripts, which
 *   the recipient runs, so its manifest is the least of what it can do.)
 * - The row is a subagent run with a parent: the button leads to the
 *   conversation the user was actually having, provided the parent exists and
 *   predates the run. Otherwise it leads to the run itself, labelled as such.
 *   ⚠ Deleting a parent does not delete its runs, so never mark the row deleted
 *   while either one can be opened.
 */
export async function resolveBuildChat(
  app: Pick<AppManifest, 'session_id' | 'created_at'>,
  read: BuildChatRowReader = readBuildChatRow
): Promise<BuildChatState> {
  const sessionId = app.session_id;
  if (!sessionId) return { kind: 'none' };
  const own = await read(sessionId);
  if (own.kind !== 'row') return { kind: own.kind };
  const chat = own.row;
  if (!spansAppCreation(chat, app.created_at)) return { kind: 'replaced' };
  if (chat.session_type === 'sub_agent' && chat.parent_session_id) {
    const parent = await read(chat.parent_session_id);
    if (parent.kind === 'row' && predates(parent.row, createdSecs(chat))) {
      return availableFrom(parent.row, 'parent');
    }
    return availableFrom(chat, 'subagent');
  }
  return availableFrom(chat, 'self');
}

/**
 * Resolve every app's build chat, reading each distinct chat once — several apps
 * are often built in one chat, and a subagent's parent may be shared too. Keyed
 * by app id.
 */
export async function resolveBuildChats(
  apps: readonly Pick<AppManifest, 'id' | 'session_id' | 'created_at'>[],
  read: BuildChatRowReader = readBuildChatRow
): Promise<Record<string, BuildChatState>> {
  const reads = new Map<string, Promise<BuildChatRowRead>>();
  const readOnce: BuildChatRowReader = (sessionId) => {
    let pending = reads.get(sessionId);
    if (!pending) {
      pending = read(sessionId);
      reads.set(sessionId, pending);
    }
    return pending;
  };
  const resolved = await Promise.all(
    apps.map(async (app) => [app.id, await resolveBuildChat(app, readOnce)] as const)
  );
  return Object.fromEntries(resolved);
}

/** The chat a resolved state leads to, if any. */
export function buildChatTarget(state: BuildChatState | undefined): string | undefined {
  return state?.kind === 'available' ? state.sessionId : undefined;
}

/**
 * Can the control navigate (or at least try)? `checking` and `unknown` stay
 * enabled on purpose: the click re-checks, and a slow or restarting daemon must
 * not gray out every row.
 */
export function buildChatEnabled(state: BuildChatState): boolean {
  return state.kind === 'available' || state.kind === 'checking' || state.kind === 'unknown';
}

/** The control's tooltip and accessible name, for one app. */
export function buildChatLabel(state: BuildChatState, appTitle: string): string {
  switch (state.kind) {
    case 'available':
      return state.via === 'subagent' ? copy.openSubagentLabel(appTitle) : copy.openLabel(appTitle);
    case 'checking':
    case 'unknown':
      return copy.openLabel(appTitle);
    case 'deleted':
      return copy.deletedLabel(appTitle);
    case 'replaced':
      return copy.replacedLabel(appTitle);
    case 'unreachable':
      return copy.unreachableLabel(appTitle);
    case 'none':
      return copy.noneLabel(appTitle);
  }
}

/** The always-visible note in the row's metadata line, or undefined when the control says enough. */
export function buildChatNote(state: BuildChatState): string | undefined {
  switch (state.kind) {
    case 'deleted':
      return copy.deletedNote;
    case 'replaced':
      return copy.replacedNote;
    case 'unreachable':
      return copy.unreachableNote;
    case 'none':
      return copy.noneNote;
    default:
      return undefined;
  }
}

/** Why a click did not open the chat, as the toast says it. */
export function buildChatRefusal(state: BuildChatState): string | undefined {
  switch (state.kind) {
    case 'deleted':
      return copy.deletedRefusal;
    case 'replaced':
      return copy.replacedRefusal;
    case 'unreachable':
      return copy.unreachableRefusal;
    case 'none':
      return copy.noneRefusal;
    case 'checking':
    case 'unknown':
      return copy.unansweredRefusal;
    case 'available':
      return undefined;
  }
}
