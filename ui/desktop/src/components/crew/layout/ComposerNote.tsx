import { useState, type ReactNode } from 'react';
import { Info, Landmark } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import { ChatConnectNote } from '../access';
import type { Snapshot } from '../crewApi';
import {
  channelName,
  displayNameIsUsername,
  personDisplayName,
  personLabel,
  usePeopleDirectory,
} from '../identity';
import { NameSuggestionNote } from '../onboarding';
import { useJoinContext } from '../onboarding/joinContext';
import { workspaceTitle } from '../sidebar';
import { useCrew } from '../state/CrewControllerContext';
import type { CrewController } from '../state/types';
import { layoutCopy } from './copy';

/** Hosts who chose "Not now" for the institution label, per workspace, on this computer. */
const LATER_KEY = 'biorouter.crew.institutionLabelLater.v1';

function readLater(): string[] {
  try {
    const raw = globalThis.localStorage?.getItem(LATER_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed) ? parsed.filter((item) => typeof item === 'string') : [];
  } catch {
    return [];
  }
}

function writeLater(ids: string[]): void {
  try {
    globalThis.localStorage?.setItem(LATER_KEY, JSON.stringify(ids));
  } catch {
    // A per-viewer convenience: without storage the note simply stays hidden this session.
  }
}

/** The verified snapshot of the selected connection, or null (never the last verified copy). */
function verifiedSnapshot(crew: CrewController): Snapshot | null {
  return crew.snapshot && crew.observedPrivacy?.connectionId === crew.connectionId
    ? crew.snapshot
    : null;
}

/** The viewer has been offered ownership of the selected channel. */
function ownershipOffered(crew: CrewController, snapshot: Snapshot): boolean {
  const pending = crew.channel?.pending_owner;
  return typeof pending === 'string' && pending !== '' && pending === snapshot.actor.id;
}

/**
 * The institution this host could label the workspace with: the verified connection's, while the
 * workspace is Private and still unlabelled. The label is permanent, so the note only offers the
 * confirmation; the broker decides.
 */
function institutionToLabel(crew: CrewController, snapshot: Snapshot): string | null {
  if (!crew.isHost) return null;
  if (snapshot.workspace.mode !== 'private' || snapshot.workspace.institution_id) return null;
  const institution = crew.connection?.institution_id?.trim();
  return institution ? institution : null;
}

/**
 * "You've been offered ownership of #methods." with **Accept ownership** — the old banner's
 * `transfer.accept`, moved above the composer. A failure goes to the connection bar: the composer's
 * own error slot speaks for sends.
 */
function OwnershipOfferNote({ snapshot }: { snapshot: Snapshot }) {
  const crew = useCrew();
  const dir = usePeopleDirectory(snapshot, crew.labels);
  const channel = crew.channel;
  if (!channel) return null;
  const here = channelName(channel);
  const owner =
    channel.owner_id && channel.owner_id !== snapshot.actor.id ? channel.owner_id : null;
  const key = 'mutate:transfer.accept';
  return (
    <Note
      tone="info"
      role="status"
      icon={Info}
      action={
        <Button
          type="button"
          variant="secondary"
          size="sm"
          disabled={crew.isPending(key)}
          onClick={() =>
            void crew.act('global', key, () =>
              crew.mutate('transfer.accept', { channel_id: channel.id })
            )
          }
        >
          {layoutCopy.ownership.accept}
        </Button>
      }
    >
      <p>
        {owner
          ? layoutCopy.ownership.offeredBy(personLabel(owner, 'authority', dir), here)
          : layoutCopy.ownership.offered(here)}
      </p>
    </Note>
  );
}

/**
 * "Label lab as ucsf? This can't be changed later." — the host setup note (ui-redesign-spec,
 * "Rail controls": Confirm workspace institution). **Set institution to ucsf…** opens the same
 * irreversible-label confirmation as the setup checklist and the Privacy tab; "Not now" hides the
 * note for this workspace on this computer, and the checklist and Privacy tab still offer it.
 */
function HostInstitutionNote({
  institution,
  workspace,
  onLater,
}: {
  institution: string;
  workspace: string;
  onLater(): void;
}) {
  const crew = useCrew();
  return (
    <Note
      tone="neutral"
      role="status"
      icon={Landmark}
      action={
        <Button
          type="button"
          variant="secondary"
          size="sm"
          disabled={crew.isPending('mutate:policy.set')}
          onClick={() =>
            crew.openDialog({
              kind: 'confirm',
              confirm: { action: 'set-institution', institutionId: institution },
            })
          }
        >
          {layoutCopy.institution.set(institution)}
        </Button>
      }
    >
      <p className="text-label">{layoutCopy.institution.title(workspace, institution)}</p>
      <p className="text-text-muted">
        {layoutCopy.institution.body}{' '}
        <Button
          type="button"
          variant="link"
          className="h-auto p-0 align-baseline"
          onClick={onLater}
        >
          {layoutCopy.institution.later}
        </Button>
      </p>
    </Note>
  );
}

/**
 * The one standing note directly above the composer card (ui-redesign-spec, "The composer"), in
 * priority order: the chat-connect note, an ownership offer, the host's institution note, the
 * first-join name suggestion. The composer puts its own answers — a send or upload failure, the
 * picker a drop opened — above whatever this returns.
 *
 * Each candidate is chosen only when it will render, so the composer never holds an empty slot. A
 * note describes the verified channel, so nothing stands here while Crew re-verifies.
 */
export function useComposerNote(): ReactNode {
  const crew = useCrew();
  const snapshot = verifiedSnapshot(crew);
  const workspaceId = snapshot?.workspace.id ?? '';
  const [later, setLater] = useState<string[]>(readLater);
  const joinContext = useJoinContext(crew.connectionId);

  if (!snapshot || !crew.channel) return null;

  // The chat-connect note speaks in every grant state while the route carries `?sessionId=`.
  if (crew.grantSessionId) return <ChatConnectNote />;

  if (ownershipOffered(crew, snapshot)) return <OwnershipOfferNote snapshot={snapshot} />;

  const institution = institutionToLabel(crew, snapshot);
  if (institution && !later.includes(workspaceId)) {
    return (
      <HostInstitutionNote
        institution={institution}
        workspace={workspaceTitle(snapshot, crew.connections, crew.connectionId)}
        onLater={() => {
          const next = [...new Set([...later, workspaceId])];
          setLater(next);
          writeLater(next);
        }}
      />
    );
  }

  // Mirrors `NameSuggestionNote`'s own eligibility, so the slot is not held open for a person who
  // already chose a name. The suggestion itself arrives a moment later from the broker.
  const actor = snapshot.actor;
  const unnamed = displayNameIsUsername(
    personDisplayName(actor.display_name ?? actor.nickname, actor.username),
    actor.username
  );
  if (joinContext.suggestName && unnamed) return <NameSuggestionNote />;

  return null;
}
