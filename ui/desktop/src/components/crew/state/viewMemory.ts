import type { CrewMessage, CrewMessagePeople } from '../crewApi';
import { resetBetweenTests } from './draftStash';
import type { ObservedPrivacy, PaneIntent, VerifiedView } from './types';

/**
 * What Crew remembers of the last verified view of each connection for the app session, so coming
 * back to Crew draws that view — dimmed and inert, as a re-verification does — instead of a blank
 * column and a "Loading messages…" skeleton (live QA round 4, Q4-04). Also the details pane the
 * person left open, per connection.
 *
 * SECURITY-SENSITIVE (human review). It is a presentation-only copy, exactly like the controller's
 * `lastVerified`, and never more:
 * - Memory only (module-level maps), never storage: nothing survives the app.
 * - Nothing is sent from it, enabled by it or decided by it. It is drawn dimmed and inert until a
 *   fresh verified view replaces it, and only for a connection the daemon calls connected.
 * - An entry is forgotten on every path that clears protected state: an observation end (every
 *   terminal frame carries `clear: true`), an observation failure or refusal, a Disconnect, a
 *   membership the workspace ended, and connection removal (`forgetConnectionMemory`).
 * - A view whose privacy moved — the workspace's mode or institution, or this connection's mode,
 *   institution or policy epoch — keeps none of the lists kept under the old one; a channel whose
 *   classification moved, or which the view no longer offers, loses its own
 *   (`rememberVerifiedView`). The first fresh view after coming back is checked the same way
 *   (`rememberedViewMoved`) before the copy may stay on screen.
 * - Only a channel's live tail that had loaded is kept (never an older page), and only
 *   `MAX_REMEMBERED_CHANNELS` channels per connection, `MAX_REMEMBERED_CONNECTIONS` connections.
 */

/** The most channels whose last page is kept per connection; the least recently shown goes first. */
export const MAX_REMEMBERED_CHANNELS = 20;
/** The most connections whose view is kept; the least recently written goes first. */
export const MAX_REMEMBERED_CONNECTIONS = 8;

/** A channel's live tail as it was last shown loaded, with the classification it was shown under. */
interface RememberedPage {
  messages: CrewMessage[];
  people: CrewMessagePeople | null;
  classification: string | null;
}

interface RememberedConnection {
  /** The last verified view, without its messages (they live per channel in `pages`). */
  view: Omit<VerifiedView, 'messages' | 'people'>;
  pages: Map<string, RememberedPage>;
}

const remembered = new Map<string, RememberedConnection>();
const paneIntents = new Map<string, PaneIntent>();

/** The privacy a view was verified under. The workspace policy epoch is deliberately absent. */
interface ViewPrivacy {
  workspaceMode: string;
  workspaceInstitution: string | null;
  connectionMode: string;
  connectionEpoch: number;
  connectionInstitution: string | null;
}

/** The parts of a verified view (or `state` frame) whose change drops what was kept. */
export interface ViewScopeSource {
  snapshot: {
    workspace: { mode: string; institution_id?: string | null };
    channels: readonly { id: string; classification?: string | null }[];
  };
  observedPrivacy: Pick<ObservedPrivacy, 'mode' | 'institutionId' | 'policyEpoch'>;
}

function privacyOf(source: ViewScopeSource): ViewPrivacy {
  return {
    workspaceMode: source.snapshot.workspace.mode,
    workspaceInstitution: source.snapshot.workspace.institution_id ?? null,
    connectionMode: source.observedPrivacy.mode,
    connectionEpoch: source.observedPrivacy.policyEpoch,
    connectionInstitution: source.observedPrivacy.institutionId ?? null,
  };
}

function samePrivacy(a: ViewPrivacy, b: ViewPrivacy): boolean {
  return (
    a.workspaceMode === b.workspaceMode &&
    a.workspaceInstitution === b.workspaceInstitution &&
    a.connectionMode === b.connectionMode &&
    a.connectionEpoch === b.connectionEpoch &&
    a.connectionInstitution === b.connectionInstitution
  );
}

/** The channel's classification in `source`, or undefined when the view does not offer it. */
function classificationIn(source: ViewScopeSource, channelId: string): string | null | undefined {
  const channel = source.snapshot.channels.find((item) => item.id === channelId);
  return channel ? (channel.classification ?? null) : undefined;
}

/**
 * Whether `next` may no longer show what was kept of `channelId` under `previous`: the privacy
 * moved, or the channel's classification moved, or `next` no longer offers the channel.
 */
export function rememberedViewMoved(
  previous: ViewScopeSource,
  next: ViewScopeSource,
  channelId: string
): boolean {
  if (!samePrivacy(privacyOf(previous), privacyOf(next))) return true;
  if (!channelId) return false;
  const now = classificationIn(next, channelId);
  return now === undefined || now !== (classificationIn(previous, channelId) ?? null);
}

/**
 * Remember `view` as the last verified view of its connection. `page` is the selected channel's
 * live tail when it had loaded (never an older page); without one, what is kept of that channel
 * stays as it was. Lists kept under another privacy, or of a channel that moved or went, are
 * dropped first.
 */
export function rememberVerifiedView(
  view: VerifiedView,
  page: { messages: readonly CrewMessage[]; people: CrewMessagePeople | null } | null
): void {
  const { connectionId, channelId } = view;
  if (!connectionId) return;
  const next: ViewScopeSource = view;
  const previous = remembered.get(connectionId);
  const pages = new Map<string, RememberedPage>();
  if (previous && samePrivacy(privacyOf(previous.view), privacyOf(next))) {
    for (const [id, kept] of previous.pages) {
      const now = classificationIn(next, id);
      if (now !== undefined && now === kept.classification) pages.set(id, kept);
    }
  }
  if (channelId && page) {
    const classification = classificationIn(next, channelId);
    if (classification !== undefined) {
      pages.delete(channelId);
      pages.set(channelId, {
        messages: [...page.messages],
        people: page.people,
        classification,
      });
    }
  }
  while (pages.size > MAX_REMEMBERED_CHANNELS) {
    const oldest = pages.keys().next().value;
    if (oldest === undefined) break;
    pages.delete(oldest);
  }
  const { messages: _messages, people: _people, ...rest } = view;
  remembered.delete(connectionId);
  remembered.set(connectionId, { view: rest, pages });
  while (remembered.size > MAX_REMEMBERED_CONNECTIONS) {
    const oldest = remembered.keys().next().value;
    if (oldest === undefined) break;
    remembered.delete(oldest);
  }
}

/**
 * The last verified view of `connectionId`, with the kept live tail of its channel — or null when
 * nothing is kept, or its channel's tail is not (a view whose channel has no loaded list would
 * claim the channel is empty). Presentation only.
 */
export function rememberedView(connectionId: string): VerifiedView | null {
  const entry = connectionId ? remembered.get(connectionId) : undefined;
  if (!entry) return null;
  const { channelId } = entry.view;
  const channel = entry.view.snapshot.channels.find((item) => item.id === channelId);
  const page = channelId ? entry.pages.get(channelId) : undefined;
  if (!channel || !page) return null;
  return { ...entry.view, messages: [...page.messages], people: page.people };
}

/** Forget the remembered view of `connectionId`: every path that clears protected state calls it. */
export function forgetRememberedView(connectionId: string): void {
  remembered.delete(connectionId);
}

/**
 * Remember the details pane the person left open on `connectionId` (its tab), or that they
 * closed it. Only the details mode is kept: Ask my agent and Chat access are about the channel and
 * the moment they were opened for.
 */
export function rememberPaneIntent(connectionId: string, intent: PaneIntent | null): void {
  if (!connectionId) return;
  if (intent?.mode === 'details') paneIntents.set(connectionId, { ...intent });
  else paneIntents.delete(connectionId);
}

/** The details pane to open again on `connectionId`, if the person left one open. */
export function rememberedPaneIntent(connectionId: string): PaneIntent | null {
  const intent = paneIntents.get(connectionId);
  return intent ? { ...intent } : null;
}

/** Forget everything kept for `connectionId`: its view, its channels' lists and its pane. */
export function forgetViewMemory(connectionId: string): void {
  remembered.delete(connectionId);
  paneIntents.delete(connectionId);
}

/** How many connections have a remembered view. For tests; never shown. */
export function rememberedViewCount(): number {
  return remembered.size;
}

/** Forget everything this module keeps. Tests share one module instance per file. */
export function resetViewMemoryForTests(): void {
  remembered.clear();
  paneIntents.clear();
}
resetBetweenTests(resetViewMemoryForTests);
