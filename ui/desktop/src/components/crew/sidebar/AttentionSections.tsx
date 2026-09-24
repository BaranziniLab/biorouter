import { useId, useMemo, type ReactNode } from 'react';
import { AlertTriangle } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import { joinerPerson, PersonName, personLabel } from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import { sidebarCopy } from './copy';
import { invitationsToMe, useSidebarView, waitingToJoin, type InvitationRow } from './sidebarView';
import './crew-sidebar.css';

const copy = sidebarCopy;

/** The pending-action key an invitation's Accept runs under. */
export const acceptKey = (invitationId: string) => `mutate:invitation.accept:${invitationId}`;

/**
 * The sidebar's attention sections (ui-redesign-spec, "The Crew sidebar"), each shown only when
 * it has a row: **Invitations** addressed to me, with a small Accept, and — for the host —
 * **Waiting to join**, with Let in… and the different-code warning.
 *
 * Both name people through `PersonName` and never show an ID: an invitation the broker did not
 * name yet reads "Invitation" from its inviter, and a joiner reads `@bob` first (the joiner
 * context), because at a host's decision the account name is what matters.
 */
export function AttentionSections() {
  const crew = useCrew();
  const { snapshot, verified, dir } = useSidebarView(crew);
  const invitations = useMemo(() => invitationsToMe(snapshot, dir), [snapshot, dir]);
  const waiting = useMemo(() => waitingToJoin(snapshot), [snapshot]);
  if (invitations.length === 0 && waiting.length === 0) return null;

  return (
    <>
      {invitations.length > 0 && (
        <Section label={copy.section.invitations} attention="invitations">
          {invitations.map((invitation) => (
            <InvitationItem key={invitation.id} invitation={invitation} actionable={verified} />
          ))}
        </Section>
      )}
      {waiting.length > 0 && (
        <Section label={copy.section.waiting} attention="waiting">
          {waiting.map((join) => (
            <li key={join.username} className="flex flex-col gap-1 px-2 py-1.5">
              <div className="flex min-w-0 items-center gap-2">
                <span className="crew-sidebar-truncate min-w-0 flex-1 text-secondary">
                  <PersonName
                    person={joinerPerson(join.username, join.serverName)}
                    context="joiner"
                  />
                </span>
                {join.approved ? (
                  <span className="shrink-0 text-supporting text-text-muted">
                    {copy.waiting.approved}
                  </span>
                ) : (
                  <Button
                    variant="secondary"
                    size="xs"
                    className="no-drag shrink-0"
                    aria-label={copy.waiting.letInLabel(join.username)}
                    disabled={!verified}
                    onClick={() => crew.openDialog({ kind: 'let-in', username: join.username })}
                  >
                    {copy.waiting.letIn}
                  </Button>
                )}
              </div>
              {join.otherDeviceTried && (
                <p className="flex items-start gap-1.5 text-supporting text-text-warning">
                  <AlertTriangle className="mt-0.5 size-3 shrink-0" aria-hidden="true" />
                  <span>{copy.waiting.otherDevice(join.username)}</span>
                </p>
              )}
            </li>
          ))}
        </Section>
      )}
    </>
  );
}

function Section({
  label,
  attention,
  children,
}: {
  label: string;
  attention: string;
  children: ReactNode;
}) {
  const id = useId();
  return (
    <div className="crew-sidebar-section" data-crew-attention={attention}>
      <h2 id={id} className="crew-sidebar-section-label text-caps">
        {label}
      </h2>
      <ul role="list" aria-labelledby={id} className="crew-sidebar-list">
        {children}
      </ul>
    </div>
  );
}

function InvitationItem({
  invitation,
  actionable,
}: {
  invitation: InvitationRow;
  actionable: boolean;
}) {
  const crew = useCrew();
  const key = acceptKey(invitation.id);
  const accept = () =>
    void crew.act('global', key, () =>
      crew.mutate('invitation.accept', { invitation_id: invitation.id })
    );
  const label = invitation.target
    ? copy.invitation.acceptLabel(invitation.target)
    : copy.invitation.acceptFromLabel(personLabel(invitation.inviter, 'inline'));

  return (
    <li className="flex min-w-0 items-center gap-2 px-2 py-1.5">
      <div className="flex min-w-0 flex-1 flex-col">
        <bdi className="crew-sidebar-truncate text-secondary text-text-default">
          {invitation.target ?? copy.invitation.untitled}
        </bdi>
        <span className="crew-sidebar-truncate text-supporting text-text-muted">
          {copy.invitation.from} <PersonName person={invitation.inviter} context="inline" />
        </span>
      </div>
      <Button
        variant="secondary"
        size="xs"
        className="no-drag shrink-0"
        aria-label={label}
        disabled={!actionable || crew.isPending(key)}
        onClick={accept}
      >
        {copy.invitation.accept}
      </Button>
    </li>
  );
}
