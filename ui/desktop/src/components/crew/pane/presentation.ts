import type { ProviderDetails } from '../../../api';
import {
  affiliationPresentation,
  readProviderAffiliation,
} from '../../privacy/providerAffiliation';
import {
  getModelDisplayName,
  getProviderDisplayName,
} from '../../settings/models/predefinedModelsUtils';
import type { Channel, CrewConnection, Snapshot, Team } from '../crewApi';
import {
  connectionNames,
  identityCopy,
  institutionLabel,
  isMachineIdShaped,
  sanitizeDisplayText,
  usePeopleDirectory,
  type KnownInstitution,
  type PeopleDirectory,
} from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import type { CrewController } from '../state/types';
import { providerLabel, type ModelChoice } from './useConfiguredModels';

/**
 * What the pane draws, read from the controller.
 *
 * The snapshot is the verified one, or while a refresh re-verifies the last verified one for this
 * connection, so the pane's content — and a Task being written in it — survives a manual refresh.
 * It is presentation only: `verified` says whether the view is current, and nothing here
 * authorizes anything; the daemon and broker decide every action.
 */
export interface PanePresentation {
  crew: CrewController;
  verified: boolean;
  snapshot: Snapshot | null;
  channel: Channel | null;
  team: Team | null;
  dir: PeopleDirectory;
  isOwner: boolean;
  /** The workspace as the switcher names it: its own name, else the saved connection's. */
  workspace: string;
}

export function usePanePresentation(): PanePresentation {
  const crew = useCrew();
  const verified = Boolean(
    crew.snapshot && crew.observedPrivacy?.connectionId === crew.connectionId
  );
  const snapshot = crew.snapshot ?? crew.lastVerified?.snapshot ?? null;
  const labels = crew.snapshot ? crew.labels : (crew.lastVerified?.labels ?? null);
  const people = crew.snapshot ? crew.people : crew.lastVerified?.people;
  const dir = usePeopleDirectory(snapshot, labels, people ?? null);
  const channel = snapshot?.channels.find((item) => item.id === crew.channelId) ?? null;
  const team = channel
    ? (snapshot?.teams.find((item) => item.id === channel.team_id) ?? null)
    : null;
  const named = sanitizeDisplayText(snapshot?.workspace.name);
  return {
    crew,
    verified,
    snapshot,
    channel,
    team,
    dir,
    isOwner: Boolean(snapshot && channel && channel.owner_id === snapshot.actor.id),
    workspace:
      named && !isMachineIdShaped(named)
        ? named
        : (connectionNames(crew.connections).get(crew.connectionId) ??
          identityCopy.unnamedWorkspace),
  };
}

/** Copy a value; Crew never toasts a copy. */
export async function copyText(value: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(value);
    return true;
  } catch {
    return false;
  }
}

/** How long a copy control reads "Copied" (or "Couldn't copy") before it reads as before. */
export const COPY_FEEDBACK_MS = 1500;
/** How long a menu stays open showing "Copied" before it closes itself (Q2-34). */
export const MENU_COPY_CLOSE_MS = 600;

/**
 * A file named in a task (Q2-15): a word ending in one of the extensions a lab shares as data.
 * `(?!-|\.\w)` keeps `counts.csv.gz` and `counts.csv-old` from reading as `counts.csv`. A word
 * cannot hold a space, so `Plate Reader.csv` reads as `Reader.csv` unless it is double-quoted on
 * its own (`quotedFileName`); see `fileNameKey` for why that can never make the warning false.
 */
const FILE_NAME = /\b[\w.-]+\.(?:csv|tsv|xlsx?|json|txt|h5ad|parquet)\b(?!-|\.\w)/gi;

/**
 * Text in straight or curly double quotes, on one line. Not backticks: they hold code, where a
 * space separates arguments (`python merge.py counts.csv plate.csv` names two files), and a name
 * with a space inside code is double-quoted in turn (`` `python run.py "Plate Reader.csv"` ``).
 */
const QUOTED = /"([^"\n]+)"|“([^”\n]+)”/g;

/**
 * What says a quoted span is more than one name: a list (`,` `;` `&`, the word `and` or `or`), a
 * command's punctuation (`|` `<` `>` `=` `$` `*` `?`), a label (`Input: …`) or a flag (`-n`).
 */
const NOT_ONE_NAME = /[,;:&|<>=$*?]|(?:^|\s)(?:and|or)(?=\s|$)|(?:^|\s)[-–—]/i;

/** A word that ends in an extension, as another file or a script does (`qc.R`, `merge.py`). */
const DOTTED_WORD = /\.[a-z]\w*$/i;

/** The last segment of a path, as a shared file is named. */
export function fileBaseName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] ?? '';
}

/**
 * What a name is compared by: the file name `FILE_NAME` reads at its very end, without case, or
 * null when it reads none there. So a name compares as a task that wrote it out would be read —
 * `Plate Reader.csv` as `reader.csv`, `OD600 run 2.xlsx` as `2.xlsx` — and a file whose name has
 * a space answers the shortened mention it produces. The comparison can only ever err towards
 * silence: a file named exactly `X` always compares equal to `X`, so "No file named X is shared"
 * is said only when that is true.
 */
function fileNameKey(name: string): string | null {
  let key: string | null = null;
  for (const match of name.matchAll(FILE_NAME)) {
    if ((match.index ?? 0) + match[0].length === name.length) key = match[0].toLowerCase();
  }
  return key;
}

/**
 * A double-quoted span read as one file name, spaces and all, by its last path segment — or null
 * when that segment holds anything but one name, which is then read word by word as unquoted text
 * is. It must end in the one file name `FILE_NAME` finds in it, and hold no other word ending in an
 * extension and nothing `NOT_ONE_NAME` matches. So `"rep1.csv, rep2.csv"` is two names and
 * `"python merge.py Plate Reader.csv"` is `Reader.csv`, while `"OD600 run 2.xlsx"` stays whole.
 * Words before the name (`"my counts.csv"`) cannot be told from a name's own (`"Plate Reader.csv"`),
 * but "No file named my counts.csv is shared" is still true: see `fileNameKey`.
 */
function quotedFileName(inner: string): string | null {
  const name = fileBaseName(inner).trim();
  if (fileNameKey(name) === null || NOT_ONE_NAME.test(name)) return null;
  if ([...name.matchAll(FILE_NAME)].length !== 1) return null;
  const words = name.split(/\s+/);
  return words.slice(0, -1).some((word) => DOTTED_WORD.test(word)) ? null : name;
}

/**
 * The file names a task mentions, first mention first, each once whatever its case.
 *
 * - A name alone in double quotes (`"Plate Reader.csv"`, `“OD600 run 2.xlsx”`) is taken whole,
 *   spaces and all (`quotedFileName`). Anywhere else — backticked code, a quoted list or command —
 *   a name ends at a space.
 * - A name inside a URL (`https://…/table.csv`) is where the agent is sent, not a file it expects
 *   to find shared, so it is not one of them.
 */
export function mentionedFileNames(text: string): string[] {
  const found: Array<{ at: number; name: string }> = [];
  const quoted: Array<[number, number]> = [];
  for (const match of text.matchAll(QUOTED)) {
    const inner = (match[1] ?? match[2] ?? '').trim();
    const name = quotedFileName(inner);
    if (name === null) continue;
    const at = match.index ?? 0;
    quoted.push([at, at + match[0].length]);
    if (!inner.includes('://')) found.push({ at, name });
  }
  for (const match of text.matchAll(FILE_NAME)) {
    const start = match.index ?? 0;
    if (quoted.some(([from, to]) => start >= from && start < to)) continue;
    let token = start;
    while (token > 0 && !/\s/.test(text[token - 1])) token -= 1;
    if (text.slice(token, start).includes('://')) continue;
    found.push({ at: start, name: match[0] });
  }
  found.sort((a, b) => a.at - b.at);
  const names: string[] = [];
  const seen = new Set<string>();
  for (const { name } of found) {
    const key = name.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    names.push(name);
  }
  return names;
}

/**
 * The mentioned names no shared file answers to, compared by `fileNameKey` of each shared file's
 * name: a shared `/data/Plate.CSV` answers `plate.csv`, and a shared `Plate Reader.csv` answers
 * `Reader.csv`, which is how an unquoted task mentions it.
 */
export function unsharedFileNames(
  mentioned: readonly string[],
  shared: Iterable<string>
): string[] {
  const keys = new Set<string>();
  for (const name of shared) {
    const key = fileNameKey(fileBaseName(name).trim());
    if (key !== null) keys.add(key);
  }
  return mentioned.filter((name) => {
    const key = fileNameKey(name);
    return key !== null && !keys.has(key);
  });
}

/**
 * A model as the chat composer's model chip names it (T-47): `getModelDisplayName` (the
 * predefined-model alias, else the id) and `getProviderDisplayName` (the predefined subtext), then
 * the provider's own display name. Imported, never re-implemented, so the two surfaces agree.
 */
export function modelDisplay(
  choice: ModelChoice,
  provider: ProviderDetails | undefined
): { model: string; provider: string } {
  // The helpers read the predefined list through `window.appConfig`, which only a host provides
  // (Electron's preload, the web shell's shim). Without one there is no list to consult, and asking
  // would log a parse failure on every render.
  const predefined = typeof window !== 'undefined' && typeof window.appConfig?.get === 'function';
  return {
    model: (predefined && getModelDisplayName(choice.model)) || choice.model,
    provider:
      (predefined && getProviderDisplayName(choice.model)) ||
      providerLabel(provider, choice.provider),
  };
}

/** The institutions configured providers publish names for, so `ucsf` can read as its name. */
export function knownInstitutions(
  providers: readonly ProviderDetails[] | null
): KnownInstitution[] {
  return (providers ?? []).flatMap(
    (provider) => readProviderAffiliation(provider)?.institutions ?? []
  );
}

function canonical(id: string | null | undefined): string | null {
  const value = typeof id === 'string' ? id.trim().toLowerCase() : '';
  return value ? value : null;
}

/** The institution a task started here answers to. */
export interface RunInstitution {
  /** The canonical IDs the model must be approved for: the connection's and the workspace's. */
  ids: readonly string[];
  /** How the institution reads in a sentence. */
  label: string;
}

/**
 * The workspace's institution, as a sentence names it, whether or not a task would be held to it.
 * `null` when neither the workspace nor the connection names one.
 */
export function workspaceInstitutionLabel(
  connection: CrewConnection | null,
  snapshot: Snapshot | null,
  known: readonly KnownInstitution[]
): string | null {
  return (
    institutionLabel(snapshot?.workspace.institution_id, known) ??
    institutionLabel(connection?.institution_id, known)
  );
}

/**
 * Whether a task started here reads protected context, as far as the pane can see: the connection
 * or the workspace is Private, a channel the task reads is Restricted, or the connection has a
 * remote folder. The daemon then refuses a public model (`crew/institution.rs` `admission`) and
 * holds a private one to the institution. The daemon may also protect context the pane does not
 * see, so `false` means "not known to be protected", never "unprotected".
 */
export function protectedRunContext({
  connection,
  snapshot,
  channel,
  contextChannels,
}: {
  connection: CrewConnection | null;
  snapshot: Snapshot | null;
  channel: Channel | null;
  contextChannels: readonly string[];
}): boolean {
  if (!snapshot || !channel) return false;
  const restricted = (id: string) =>
    snapshot.channels.find((item) => item.id === id)?.classification === 'restricted';
  return (
    connection?.mode === 'private' ||
    snapshot.workspace.mode === 'private' ||
    channel.classification === 'restricted' ||
    contextChannels.some(restricted) ||
    Boolean(connection?.remote_root)
  );
}

/**
 * The institution a PRIVATE model must be approved for to start a task here, or `null` when the
 * pane cannot be sure the daemon will ask.
 *
 * A conservative mirror of the daemon's admission (`crates/biorouter/src/crew/institution.rs`,
 * `admission` then `check_provider`): the context is protected when the connection or the
 * workspace is Private, a channel the task reads is Restricted, or the connection has a remote
 * folder (which protects a private model's context). Only then are the connection's and the
 * workspace's institutions the model's owners. The daemon may also protect context the pane does
 * not see (a restricted message in a public-safe channel), so this can miss a refusal, never
 * invent one — and it decides nothing: the request path still runs for every eligible model.
 */
export function runInstitution({
  connection,
  snapshot,
  channel,
  contextChannels,
  known,
}: {
  connection: CrewConnection | null;
  snapshot: Snapshot | null;
  channel: Channel | null;
  contextChannels: readonly string[];
  known: readonly KnownInstitution[];
}): RunInstitution | null {
  if (!snapshot || !channel) return null;
  if (!protectedRunContext({ connection, snapshot, channel, contextChannels })) return null;
  const ids = [
    ...new Set(
      [canonical(snapshot.workspace.institution_id), canonical(connection?.institution_id)].filter(
        (id): id is string => id !== null
      )
    ),
  ];
  if (ids.length === 0) return null;
  const label = workspaceInstitutionLabel(connection, snapshot, known) ?? ids[0];
  return { ids, label };
}

/** Why a model cannot start a task here: who approved it, or `null` when it states no one. */
export interface ModelMismatch {
  affiliation: string | null;
}

/**
 * Whether a configured model is certainly not approved for `institution`, mirroring the daemon's
 * union-of-owners rule (`privacy::affiliation::owners_compatible`): a local model is approved
 * everywhere; an institutional one only when every institution covering it is every owner; a
 * private model that states no institution for none. A public model, and one whose affiliation
 * is unresolved, answer `null`: other rules (and the daemon) speak for them.
 */
export function modelMismatch(
  provider: ProviderDetails | undefined,
  institution: RunInstitution | null
): ModelMismatch | null {
  if (!provider || !institution) return null;
  if (provider.resolved_tier === 'public') return null;
  const affiliation = readProviderAffiliation(provider);
  if (!affiliation || affiliation.kind === 'local') return null;
  if (affiliation.kind === 'unstated') return { affiliation: null };
  const covered = institution.ids.every((owner) =>
    affiliation.institutions.every((item) => canonical(item.id) === owner)
  );
  return covered ? null : { affiliation: affiliationPresentation(affiliation)?.label ?? null };
}

const AFFILIATION_REFUSAL = /resolved affiliation/i;

/**
 * The daemon's institution refusal ("Crew institution does not match the model's resolved
 * affiliation; …", `crew/institution.rs` `check_provider`), which the pane rewords.
 */
export function isAffiliationRefusal(message: string | null | undefined): boolean {
  return typeof message === 'string' && AFFILIATION_REFUSAL.test(message);
}
