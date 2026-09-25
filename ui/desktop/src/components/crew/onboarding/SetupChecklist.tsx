import { useState, type ReactNode } from 'react';
import { Check, UserPlus } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import {
  connectionServer,
  displayNameIsUsername,
  institutionLabel,
  personDisplayName,
  usePeopleDirectory,
  workspaceName,
} from '../identity';
import { serverLabel, useKnownInstitutions } from '../sidebar/sidebarView';
import { useCrew } from '../state/CrewControllerContext';
import { checklistCopy } from './copy';
import { dismissNameOffer, offeredName } from './joinContext';
import { useNameSuggestion } from './NameSuggestionNote';
import { SetupCard } from './parts';

const HIDDEN_KEY = 'biorouter.crew.setupChecklistHidden.v1';

function readHidden(): string[] {
  try {
    const raw = globalThis.localStorage?.getItem(HIDDEN_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed) ? parsed.filter((item) => typeof item === 'string') : [];
  } catch {
    return [];
  }
}

function writeHidden(ids: string[]) {
  try {
    globalThis.localStorage?.setItem(HIDDEN_KEY, JSON.stringify(ids));
  } catch {
    // A per-viewer convenience: without storage the checklist simply stays hidden this session.
  }
}

/**
 * The name each host saved with **Use** on this computer this session, per connection and account
 * (Q4-48), recorded once the save succeeded. Module state, not component state: saving the name
 * refreshes the workspace, and the checklist can unmount and mount again on the way (the main area
 * passes through a view with no snapshot), which threw a component's memory away and took the row
 * with it.
 */
const usedNames = new Map<string, string>();

/** Tests only: forget every Use. */
export function resetSetupChecklistForTests(): void {
  usedNames.clear();
}

/**
 * "Set institution to UCSF…": the institution as every chip and the Mark dialog this opens write it
 * (`institutionLabel`, Q4-47, Carol R4-5), never the raw ID the confirmation writes. Its own
 * component so the provider list is read only while the button is on screen.
 */
function SetInstitutionLabel({ institution }: { institution: string }) {
  const known = useKnownInstitutions();
  return <>{checklistCopy.setInstitution(institutionLabel(institution, known) ?? institution)}</>;
}

interface ChecklistAction {
  /** Unique within its row. */
  key: string;
  label: ReactNode;
  run: () => void;
  disabled?: boolean;
  variant?: 'outline' | 'ghost';
}

interface ChecklistRow {
  key: 'name' | 'institution' | 'team' | 'invite';
  label: string;
  done: boolean;
  actions?: ChecklistAction[];
  hint?: string;
}

/**
 * "Get {workspace} ready": the host's first steps — confirm the institution, create a team, invite
 * people — each ticking as the verified workspace shows it done. Host only, and only while a step
 * is open; Hide is remembered on this computer. `force` keeps it on screen without Hide (it is the
 * whole of the host's empty workspace).
 *
 * While the host's display name is still their username and the server account has a name, a row
 * comes first: "Your name: Use “Alice Chen”?" with Use and Edit… (Q3-51). The host never joins,
 * so the join flow's offer never reached them, and their first invitation went out naming them
 * only `@alice`. It is the same offer as the composer's note (naming D2): never applied silently,
 * and answering it here answers it there. Once used, the row stays and ticks with the name ("Your
 * name: Alice Chen ✓"), so the card keeps its shape (Q4-48): it no longer vanishes between Use
 * saving and the workspace naming them, or when the card mounts again.
 *
 * When the daemon's name for the server differs from its address (the host typed the address and
 * their SSH config calls it `lab-server`), a line under the title says so (Q4-34): every surface
 * from here on names the alias.
 *
 * `compact` is the one line a channel keeps while the host is still alone in the workspace (T-22):
 * "No one else has joined {workspace} yet." with **Invite people to {workspace}…**. Opening a
 * channel used to take the checklist, and its invite row, away with it. It goes by itself once
 * someone else joins or asks to, and honors the same Hide.
 *
 * Every action opens the dialog that asks; the irreversible institution label goes through its own
 * confirmation. The broker decides each one.
 */
export function SetupChecklist({
  force = false,
  compact = false,
}: {
  force?: boolean;
  compact?: boolean;
}) {
  const crew = useCrew();
  const view = crew.snapshot ?? crew.lastVerified?.snapshot ?? null;
  const directory = usePeopleDirectory(view, crew.labels);
  const [hidden, setHidden] = useState<string[]>(readHidden);
  // Only the host's full checklist carries the name row; the compact line and a member ask nothing.
  const suggestion = useNameSuggestion(crew.isHost && !compact);
  if (!view || !crew.isHost) return null;

  const actor = view.actor;
  const currentName = personDisplayName(actor.display_name ?? actor.nickname, actor.username);
  const named = !displayNameIsUsername(currentName, actor.username);
  const nameKey = `${crew.connectionId}:${actor.username}`;
  // The row stays, ticked, once the offer was answered with a name. `offeredName` is the broker's
  // offer this session, which outlives this component and the offer's own answer; `used` is a Use
  // whose save succeeded, for the moment before the workspace names them.
  const offered = offeredName(crew.connectionId, actor.username);
  const used = usedNames.get(nameKey) ?? null;
  const connectionId = crew.connectionId;
  /**
   * The offer's own Use (`useNameSuggestion`: the same save, the same answer recorded), which also
   * remembers the name once the save succeeded, so the row can tick before the refreshed workspace
   * names them. A failed save leaves the offer open, beside the error, to try again.
   */
  const saveName = (name: string) => {
    const avatar = crew.snapshot?.actor.avatar ?? null;
    void crew
      .act('global', 'mutate:profile.update', async () => {
        await crew.mutate('profile.update', { nickname: name, avatar });
        return true;
      })
      .then((saved) => {
        if (!saved) return;
        usedNames.set(nameKey, name);
        dismissNameOffer(connectionId);
      });
  };
  const nameRow: ChecklistRow | null =
    suggestion && !named
      ? {
          key: 'name',
          label: checklistCopy.name(suggestion.name),
          done: false,
          actions: [
            {
              key: 'use-name',
              label: checklistCopy.useName,
              run: () => saveName(suggestion.name),
              disabled: !crew.snapshot || suggestion.pending,
            },
            {
              key: 'edit-name',
              label: checklistCopy.editName,
              run: suggestion.edit,
              variant: 'ghost',
            },
          ],
        }
      : named && (offered || used)
        ? { key: 'name', label: checklistCopy.nameSet(currentName), done: true }
        : used && !suggestion
          ? // Saved, and the workspace has not named them yet: the name they chose, ticked.
            { key: 'name', label: checklistCopy.nameSet(used), done: true }
          : null;

  const workspaceId = view.workspace.id;
  const live = Boolean(crew.snapshot);
  const institution = crew.connection?.institution_id?.trim() || null;
  const isPublic = view.workspace.mode === 'public';
  const labelled = Boolean(view.workspace.institution_id);
  const activePeople = view.principals.filter((person) => person.active !== false).length;

  const rows: ChecklistRow[] = [
    ...(nameRow ? [nameRow] : []),
    {
      key: 'institution',
      label: checklistCopy.institution,
      done: labelled || isPublic,
      hint: isPublic && !labelled ? checklistCopy.institutionPublic : undefined,
      actions: [
        institution
          ? {
              key: 'set-institution',
              label: <SetInstitutionLabel institution={institution} />,
              disabled: !live,
              run: () =>
                crew.openDialog({
                  kind: 'confirm',
                  confirm: { action: 'set-institution', institutionId: institution },
                }),
            }
          : {
              // No institution on this connection yet: the label needs one, so go where it is set.
              key: 'connection-settings',
              label: checklistCopy.connectionSettings,
              run: () =>
                crew.openDialog({ kind: 'connection-settings', connectionId: crew.connectionId }),
            },
      ],
    },
    {
      key: 'team',
      label: checklistCopy.team,
      done: view.teams.length > 0,
      actions: [
        {
          key: 'create-team',
          label: checklistCopy.createTeam,
          disabled: !live,
          run: () => crew.openDialog({ kind: 'create-team' }),
        },
      ],
    },
    {
      key: 'invite',
      label: checklistCopy.invite,
      done: activePeople > 1 || (view.pending_joins?.length ?? 0) > 0,
      actions: [
        {
          key: 'invite-people',
          label: checklistCopy.invitePeople,
          disabled: !live,
          run: () => crew.openDialog({ kind: 'invite-people' }),
        },
      ],
    },
  ];
  const open = rows.some((row) => !row.done);
  const title = workspaceName(view.workspace, directory.host);
  const address = connectionServer(crew.connection);
  const label = serverLabel(crew.connection);
  const alias =
    address && label && label !== address ? checklistCopy.serverAlias(title, address, label) : null;

  if (compact) {
    const invite = rows.find((row) => row.key === 'invite');
    if (!invite || invite.done || hidden.includes(workspaceId)) return null;
    return (
      <div
        className="crew-onboard-nudge"
        role="group"
        aria-label={checklistCopy.label}
        data-testid="crew-setup-invite-nudge"
      >
        <span className="crew-onboard-nudge-text text-supporting text-text-muted">
          {checklistCopy.aloneTitle(title)}
        </span>
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={!live}
          onClick={() => crew.openDialog({ kind: 'invite-people' })}
        >
          <UserPlus aria-hidden />
          {checklistCopy.invitePeopleTo(title)}
        </Button>
      </div>
    );
  }

  if (!force && (!open || hidden.includes(workspaceId))) return null;

  return (
    <SetupCard title={checklistCopy.title(title)} testId="crew-setup-checklist">
      {alias ? (
        <p className="text-supporting text-text-muted" data-testid="crew-setup-server-alias">
          {alias}
        </p>
      ) : null}
      <ul className="crew-onboard-checklist" aria-label={checklistCopy.label}>
        {rows.map((row) => (
          <li
            key={row.key}
            className="crew-onboard-check-row"
            data-done={row.done ? 'true' : 'false'}
          >
            <span className="crew-onboard-check-mark" aria-hidden="true">
              {row.done ? <Check className="h-3.5 w-3.5" /> : null}
            </span>
            <span className="crew-onboard-check-text text-label">
              {row.label}
              <span className="sr-only">{row.done ? `, ${checklistCopy.done}` : ''}</span>
              {!row.done && row.key === 'institution' && !institution ? (
                <span className="block text-supporting text-text-muted">
                  {checklistCopy.institutionMissing}
                </span>
              ) : null}
              {row.hint ? (
                <span className="block text-supporting text-text-muted">{row.hint}</span>
              ) : null}
            </span>
            {!row.done && row.actions?.length ? (
              <span className="crew-onboard-check-actions">
                {row.actions.map((action) => (
                  <Button
                    key={action.key}
                    type="button"
                    size="sm"
                    variant={action.variant ?? 'outline'}
                    disabled={action.disabled}
                    onClick={action.run}
                  >
                    {action.label}
                  </Button>
                ))}
              </span>
            ) : null}
          </li>
        ))}
      </ul>
      {!force ? (
        <div className="crew-onboard-actions">
          <Button
            type="button"
            size="sm"
            variant="ghost"
            onClick={() => {
              const next = [...new Set([...hidden, workspaceId])];
              setHidden(next);
              writeHidden(next);
            }}
          >
            {checklistCopy.hide}
          </Button>
        </div>
      ) : null}
    </SetupCard>
  );
}
