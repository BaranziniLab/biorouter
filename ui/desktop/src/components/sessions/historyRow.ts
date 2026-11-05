import { chatKindOf, type ChatKindSource } from '../chats/chatKind';
import { shortModelLabel } from '../../utils/sessionSubagents';

/**
 * Small pure helpers for a chat row's second line in Chat history and the
 * transcript headers. Kept free of React so they are tested on their own.
 */

/**
 * The folder a chat ran in, by name: the last segment of its working directory.
 *
 * A row shows names, not paths (principle 10); the full path stays reachable in
 * the row's tooltip and in the search. Trailing separators are ignored, both
 * separators are understood (a Windows path reaches a macOS window through an
 * import), and the root itself reads as `/`.
 */
export function folderName(path: string | null | undefined): string {
  if (!path) return '';
  const trimmed = path.replace(/[\\/]+$/, '');
  if (!trimmed) return path.startsWith('\\') ? '\\' : '/';
  const segments = trimmed.split(/[\\/]/);
  return segments[segments.length - 1] || trimmed;
}

/** The daemon names a delegated run `Subagent: <label>` (`subagent_tool.rs`). */
const SUBAGENT_NAME_PREFIX = /^Subagent:\s+/i;

/**
 * The scheduler names every run's chat `Scheduled job: <job id>` (`scheduler.rs`, when it
 * creates the `Scheduled` session). The job id is the name the person gave the job in the
 * Scheduler; the rest is the kind, which the clock glyph already says.
 */
const SCHEDULED_NAME_PREFIX = /^Scheduled job:\s+/i;

/** `living-news-router` becomes `Living news router`: words, sentence case. */
function humanizeSlug(slug: string): string {
  const words = slug.split(/[-_]+/).filter(Boolean).join(' ');
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/**
 * The title a History row shows: the chat's name, minus what the kind glyph already says
 * (principle 2, say it once; principle 10, names not IDs; vision V1-R2-10).
 *
 * - An app's chat is stored as `app:<app id>` (`app:<app id>:<worker profile>` for a worker
 *   profile's own chat). The App glyph says "app", and a slug with a colon is a machine
 *   string, so the row reads "Living news router", with the profile after it in parentheses.
 * - A sub-agent run is stored as `Subagent: <label>`. The robot glyph and the rail that ties
 *   it to its parent say that; the row shows the label. A name that is only the prefix is kept
 *   as it is, so a row never renders empty.
 * - A scheduled run is stored as `Scheduled job: <job id>`. The clock glyph says "scheduled",
 *   and a search for a job lists every run of it, so each row read "Scheduled job: <id>" ten
 *   times over (vision V1-R3-6). The row shows the job id, as the Scheduler names the job.
 *   Keyed on the KIND, not on the prefix alone: a chat a person happens to title "Scheduled
 *   job: x" keeps its title, and a run the person renamed has no prefix to drop.
 *
 * Display only. The stored name is untouched (the Meditation job skips scheduled runs by that
 * prefix, the rename field starts from it, and the row's filter still matches it, see
 * `matchesFilter`), and every other surface keeps its own spelling.
 */
export function historyRowTitle(session: ChatKindSource): string {
  const name = session.name ?? '';
  const kind = chatKindOf(session);
  if (kind === 'app') {
    const [, id = '', ...profile] = name.trim().split(':');
    const app = humanizeSlug(id.trim());
    if (!app) return name;
    const worker = humanizeSlug(profile.join(':').trim()).toLowerCase();
    return worker ? `${app} (${worker})` : app;
  }
  if (kind === 'subagent') {
    const label = name.replace(SUBAGENT_NAME_PREFIX, '').trim();
    return label || name;
  }
  if (kind === 'scheduled') {
    const job = name.replace(SCHEDULED_NAME_PREFIX, '').trim();
    return job || name;
  }
  return name;
}

/**
 * The `extension_data` key of a finished subagent run's outcome record, `{ status, ended_at,
 * result_line? }` (`SUBAGENT_OUTCOME_STATE` . `SUBAGENT_OUTCOME_VERSION` in
 * `crates/biorouter/src/agents/subagent_roster.rs`). Written once, when the run ends.
 */
export const SUBAGENT_OUTCOME_KEY = 'subagent_outcome.v1';

/** The ends a run's outcome record can name, and the only ones a History row draws. */
export type SubagentOutcomeStatus = 'completed' | 'blocked' | 'failed' | 'incomplete';

const OUTCOME_STATUSES: ReadonlySet<string> = new Set<SubagentOutcomeStatus>([
  'completed',
  'blocked',
  'failed',
  'incomplete',
]);

export interface SubagentOutcome {
  status: SubagentOutcomeStatus;
  /** First line of the run's summary, its error, or the question it stopped to ask. */
  resultLine: string | null;
}

/**
 * How a subagent run ended, read from its outcome record, or `null` when there is nothing to
 * say: no record (a run that is still going, one whose process went away, or one that predates
 * the record), a malformed one, or a status that is not an end (`running`, `interrupted`, ...).
 * History says nothing rather than guess; the chat's side panel is where a live run shows.
 */
export function subagentOutcomeOf(
  extensionData: Record<string, unknown> | null | undefined
): SubagentOutcome | null {
  const record = extensionData?.[SUBAGENT_OUTCOME_KEY];
  if (!record || typeof record !== 'object') return null;
  const { status, result_line: resultLine } = record as { status?: unknown; result_line?: unknown };
  if (typeof status !== 'string' || !OUTCOME_STATUSES.has(status)) return null;
  const line = typeof resultLine === 'string' ? resultLine.trim() : '';
  return { status: status as SubagentOutcomeStatus, resultLine: line || null };
}

/**
 * The model a run used, for the small chip on its History row: the short label the chat
 * summary and the terminal use too (`gpt-5.5`, not `gpt-5.5-2026-04-24`), which is what tells
 * two runs of one parent apart. The full id stays in the chip's tooltip
 * ([`runModelFullName`]). `null` when the row records none.
 */
export function runModelName(session: {
  model_config?: { model_name?: string | null } | null;
}): string | null {
  const name = runModelFullName(session);
  return name ? shortModelLabel(name) : null;
}

/** The model id a run recorded, unshortened, for the chip's tooltip. */
export function runModelFullName(session: {
  model_config?: { model_name?: string | null } | null;
}): string | null {
  const name = session.model_config?.model_name?.trim();
  return name ? name : null;
}

/** `extension_data` key of a run's spawn record, which holds the role its parent gave it. */
export const SUBAGENT_SPAWN_KEY = 'subagent_spawn.v1';

/**
 * The short role a run's parent gave it at spawn (`celsius`, `sq-3`), read from the spawn
 * record, or `null`. A History run leads with it, as the chat summary and the terminal do,
 * because the session name is the first words of the task and reads the same for every run of
 * a fan-out.
 */
export function subagentRoleOf(
  extensionData: Record<string, unknown> | null | undefined
): string | null {
  const record = extensionData?.[SUBAGENT_SPAWN_KEY];
  if (!record || typeof record !== 'object') return null;
  const role = (record as { role?: unknown }).role;
  if (typeof role !== 'string') return null;
  const trimmed = role.trim();
  return trimmed ? trimmed : null;
}
