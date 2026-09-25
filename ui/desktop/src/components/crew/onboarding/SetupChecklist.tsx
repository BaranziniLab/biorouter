import { useEffect, useState } from 'react';
import { Check, UserPlus } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import {
  displayNameIsUsername,
  personDisplayName,
  usePeopleDirectory,
  workspaceName,
} from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import { checklistCopy } from './copy';
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

interface ChecklistAction {
  label: string;
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
 * and answering it here answers it there. Once used, the row ticks with the name.
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
  // The row stays, ticked, once the offer it made was answered with a name: a step that vanished
  // on Use would read as undone.
  const [nameOffered, setNameOffered] = useState(false);
  const offering = suggestion !== null;
  useEffect(() => {
    if (offering) setNameOffered(true);
  }, [offering]);
  if (!view || !crew.isHost) return null;

  const actor = view.actor;
  const currentName = personDisplayName(actor.display_name ?? actor.nickname, actor.username);
  const named = !displayNameIsUsername(currentName, actor.username);
  const nameRow: ChecklistRow | null =
    suggestion && !named
      ? {
          key: 'name',
          label: checklistCopy.name(suggestion.name),
          done: false,
          actions: [
            {
              label: checklistCopy.useName,
              run: suggestion.use,
              disabled: !crew.snapshot || suggestion.pending,
            },
            { label: checklistCopy.editName, run: suggestion.edit, variant: 'ghost' },
          ],
        }
      : nameOffered && named
        ? { key: 'name', label: checklistCopy.nameSet(currentName), done: true }
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
              label: checklistCopy.setInstitution(institution),
              disabled: !live,
              run: () =>
                crew.openDialog({
                  kind: 'confirm',
                  confirm: { action: 'set-institution', institutionId: institution },
                }),
            }
          : {
              // No institution on this connection yet: the label needs one, so go where it is set.
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
          label: checklistCopy.invitePeople,
          disabled: !live,
          run: () => crew.openDialog({ kind: 'invite-people' }),
        },
      ],
    },
  ];
  const open = rows.some((row) => !row.done);
  const title = workspaceName(view.workspace, directory.host);

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
                    key={action.label}
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
