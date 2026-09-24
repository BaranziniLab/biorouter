import { useState } from 'react';
import { Check } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import { usePeopleDirectory, workspaceName } from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import { checklistCopy } from './copy';
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

interface ChecklistRow {
  key: 'institution' | 'team' | 'invite';
  label: string;
  done: boolean;
  action?: { label: string; run: () => void; disabled?: boolean };
  hint?: string;
}

/**
 * "Get {workspace} ready": the host's three first steps — confirm the institution, create a team,
 * invite people — each ticking as the verified workspace shows it done. Host only, and only while
 * a step is open; Hide is remembered on this computer. `force` keeps it on screen without Hide (it
 * is the whole of the host's empty workspace).
 *
 * Every action opens the dialog that asks; the irreversible institution label goes through its own
 * confirmation. The broker decides each one.
 */
export function SetupChecklist({ force = false }: { force?: boolean }) {
  const crew = useCrew();
  const view = crew.snapshot ?? crew.lastVerified?.snapshot ?? null;
  const directory = usePeopleDirectory(view, crew.labels);
  const [hidden, setHidden] = useState<string[]>(readHidden);
  if (!view || !crew.isHost) return null;

  const workspaceId = view.workspace.id;
  const live = Boolean(crew.snapshot);
  const institution = crew.connection?.institution_id?.trim() || null;
  const isPublic = view.workspace.mode === 'public';
  const labelled = Boolean(view.workspace.institution_id);
  const activePeople = view.principals.filter((person) => person.active !== false).length;

  const rows: ChecklistRow[] = [
    {
      key: 'institution',
      label: checklistCopy.institution,
      done: labelled || isPublic,
      hint: isPublic && !labelled ? checklistCopy.institutionPublic : undefined,
      action: institution
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
    },
    {
      key: 'team',
      label: checklistCopy.team,
      done: view.teams.length > 0,
      action: {
        label: checklistCopy.createTeam,
        disabled: !live,
        run: () => crew.openDialog({ kind: 'create-team' }),
      },
    },
    {
      key: 'invite',
      label: checklistCopy.invite,
      done: activePeople > 1 || (view.pending_joins?.length ?? 0) > 0,
      action: {
        label: checklistCopy.invitePeople,
        disabled: !live,
        run: () => crew.openDialog({ kind: 'invite-people' }),
      },
    },
  ];
  const open = rows.some((row) => !row.done);
  if (!force && (!open || hidden.includes(workspaceId))) return null;

  return (
    <SetupCard
      title={checklistCopy.title(workspaceName(view.workspace, directory.host))}
      testId="crew-setup-checklist"
    >
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
            {!row.done && row.action ? (
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={row.action.disabled}
                onClick={row.action.run}
              >
                {row.action.label}
              </Button>
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
