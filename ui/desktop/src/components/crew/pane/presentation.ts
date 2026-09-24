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
  const restricted = (id: string) =>
    snapshot.channels.find((item) => item.id === id)?.classification === 'restricted';
  const protectedContext =
    connection?.mode === 'private' ||
    snapshot.workspace.mode === 'private' ||
    channel.classification === 'restricted' ||
    contextChannels.some(restricted) ||
    Boolean(connection?.remote_root);
  if (!protectedContext) return null;
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
