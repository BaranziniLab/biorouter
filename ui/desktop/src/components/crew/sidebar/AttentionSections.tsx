import { useEffect, useId, useMemo, useRef, type ReactNode } from 'react';
import { AlertTriangle } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import type { Snapshot } from '../crewApi';
import { identityCopy, joinerPerson, PersonName, personLabel } from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import { sidebarCopy } from './copy';
import { useSidebarAnnounce } from './SidebarAnnouncer';
import {
  invitationsToMe,
  useSidebarView,
  waitingToJoin,
  type InvitationRow,
  type WaitingRow,
} from './sidebarView';
import './crew-sidebar.css';

const copy = sidebarCopy;

/** The pending-action key an invitation's Accept runs under. */
export const acceptKey = (invitationId: string) => `mutate:invitation.accept:${invitationId}`;

// ---------------------------------------------------------------------------------------------
// What changed in Waiting to join (T-17)
// ---------------------------------------------------------------------------------------------

/** One pending join, as far as an announcement cares. */
export interface WaitingState {
  approved: boolean;
  expired: boolean;
  /** The broker's `mismatched_attempts`: how many claims with a different code it refused. */
  mismatches: number;
}

export type WaitingChange =
  | { kind: 'waiting'; username: string }
  | { kind: 'code-entered'; username: string }
  | { kind: 'mismatch'; username: string };

/** The host's pending joins by username, from the verified snapshot (S3a, host only). */
export function waitingStates(
  snapshot: Pick<Snapshot, 'pending_joins'> | null
): Map<string, WaitingState> {
  const states = new Map<string, WaitingState>();
  const pending = snapshot?.pending_joins;
  if (!Array.isArray(pending)) return states;
  for (const join of pending) {
    if (!join || typeof join.username !== 'string') continue;
    const mismatches =
      typeof join.mismatched_attempts === 'number' && Number.isFinite(join.mismatched_attempts)
        ? Math.max(0, Math.floor(join.mismatched_attempts))
        : 0;
    states.set(join.username, {
      approved: join.approved === true,
      expired: join.expired === true,
      mismatches,
    });
  }
  return states;
}

/**
 * What a person should hear about the Waiting to join list moving from `before` to `after`:
 * someone new waiting, a code the host entered, and a new different-code attempt — each once.
 *
 * `before === null` is the first view of this workspace: it is the baseline, not news, so
 * nothing is said about the people already waiting when Crew opens. A row that leaves the list
 * is not announced here: when it left because the person joined, the layout's "joined" toast
 * already says so, and a second, differently worded sentence would only repeat it.
 */
export function waitingChanges(
  before: ReadonlyMap<string, WaitingState> | null,
  after: ReadonlyMap<string, WaitingState>
): WaitingChange[] {
  if (!before) return [];
  const changes: WaitingChange[] = [];
  for (const [username, now] of after) {
    const was = before.get(username);
    if (!now.expired) {
      if ((!was || was.expired) && !now.approved) changes.push({ kind: 'waiting', username });
      else if (was && !was.approved && now.approved) {
        changes.push({ kind: 'code-entered', username });
      }
    }
    if (now.mismatches > (was?.mismatches ?? 0)) changes.push({ kind: 'mismatch', username });
  }
  return changes;
}

/**
 * Speaks the Waiting to join changes (T-17): polite for someone waiting and a code entered, and
 * `role="alert"` for a new different-code attempt, once per new `mismatched_attempts` value.
 * Only verified views are compared, and the baseline follows the connection and the workspace,
 * so switching workspaces or re-verifying never replays what is already on screen.
 */
function useWaitingAnnouncements(
  snapshot: Snapshot | null,
  verified: boolean,
  connectionId: string,
  workspace: string
) {
  const { announce, alert } = useSidebarAnnounce();
  const seen = useRef<{ key: string; states: Map<string, WaitingState> } | null>(null);
  const current = verified ? snapshot : null;

  useEffect(() => {
    if (!current) return;
    const key = `${connectionId}\u0000${current.workspace?.id ?? ''}`;
    const states = waitingStates(current);
    const before = seen.current?.key === key ? seen.current.states : null;
    seen.current = { key, states };
    const changes = waitingChanges(before, states);
    if (changes.length === 0) return;

    const spoken = changes.flatMap((change) =>
      change.kind === 'waiting'
        ? [copy.waiting.announceWaiting(change.username, workspace)]
        : change.kind === 'code-entered'
          ? [copy.waiting.announceCodeEntered(change.username)]
          : []
    );
    if (spoken.length > 0) announce(spoken.join(' '));
    const warnings = changes
      .filter((change) => change.kind === 'mismatch')
      .map((change) => copy.waiting.alertOtherDevice(change.username));
    if (warnings.length > 0) alert(warnings.join(' '));
  }, [current, connectionId, workspace, announce, alert]);
}

// ---------------------------------------------------------------------------------------------
// The sections
// ---------------------------------------------------------------------------------------------

/**
 * The sidebar's attention sections (ui-redesign-spec, "The Crew sidebar"), each shown only when
 * it has a row: **Invitations** addressed to me, with a small Join, and — for the host —
 * **Waiting to join**, with Let in… and the different-code warning, or, once an invitation has
 * run out, "Invitation expired · Invite again…" and no Let in.
 *
 * A row still waiting for its code reads `@frank`, then `Frank Okafor (name on the server
 * account) · invited` on a line of its own, over "Let in… when they send their code" (Q2-42):
 * whose turn it is — the joiner sends a code, then the host lets them in — which the bare name and
 * button never said. The name and the state wrap rather than truncate (Q3-53: one line cut
 * "Gina Rossi · invited" to "Gina …" with nothing to reveal the rest), and Let in… never shrinks. A row whose code the host entered reads "Code entered",
 * not "Approved": the broker compares the code only when the joiner's computer checks in (T-13). When a claim with a different code was
 * refused, the warning says what to do, and Let in… stays offered so a mistyped code can be
 * entered again (the dialog then offers Replace code).
 *
 * Both name people through `PersonName` and never show an ID: an invitation the broker did not
 * name yet reads "Invitation" from its inviter, and a joiner reads `@bob` first (the joiner
 * context), because at a host's decision the account name is what matters.
 */
export function AttentionSections() {
  const crew = useCrew();
  const { snapshot, verified, dir, title } = useSidebarView(crew);
  const invitations = useMemo(() => invitationsToMe(snapshot, dir), [snapshot, dir]);
  const waiting = useMemo(() => waitingToJoin(snapshot), [snapshot]);
  useWaitingAnnouncements(snapshot, verified, crew.connectionId, title);
  if (invitations.length === 0 && waiting.length === 0) return null;

  const letIn = (username: string, describedBy?: string) => (
    <Button
      variant="secondary"
      size="xs"
      className="no-drag shrink-0"
      aria-label={copy.waiting.letInLabel(username)}
      aria-describedby={describedBy}
      disabled={!verified}
      onClick={() => crew.openDialog({ kind: 'let-in', username })}
    >
      {copy.waiting.letIn}
    </Button>
  );

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
            <WaitingItem key={join.username} join={join} verified={verified} letIn={letIn} />
          ))}
        </Section>
      )}
    </>
  );
}

/** One Waiting to join row. */
function WaitingItem({
  join,
  verified,
  letIn,
}: {
  join: WaitingRow;
  /** False while the sidebar shows only the last verified copy: nothing is actionable then. */
  verified: boolean;
  letIn(username: string, describedBy?: string): ReactNode;
}) {
  const crew = useCrew();
  const nextId = useId();
  // Still waiting for the joiner's code: say whose turn it is (Q2-42).
  const invited = !join.expired && !join.approved;
  // The joiner layout's own sanitising: the full name on the server account, or nothing.
  const serverName = joinerPerson(join.username, join.serverName).serverName;
  const second = Boolean(serverName) || invited;
  const action = join.expired ? (
    <span className="flex items-center gap-1 text-supporting text-text-muted">
      <span>{copy.waiting.expired}</span>
      <span aria-hidden="true">{` ${copy.waiting.separator} `}</span>
      <Button
        variant="ghost"
        size="xs"
        className="no-drag"
        aria-label={copy.waiting.inviteAgainLabel(join.username)}
        disabled={!verified}
        onClick={() => crew.openDialog({ kind: 'invite-people' })}
      >
        {copy.waiting.inviteAgain}
      </Button>
    </span>
  ) : join.approved ? (
    <span className="flex items-center gap-1.5">
      <span className="text-supporting text-text-muted" data-crew-waiting-state="code-entered">
        {copy.waiting.approved}
      </span>
      {join.otherDeviceTried && letIn(join.username)}
    </span>
  ) : (
    letIn(join.username, nextId)
  );
  return (
    <li className="flex flex-col gap-1 px-2 py-1.5" data-crew-waiting={join.username}>
      {/* A grid, so the DOM reads in speaking order — `@gina`, then "Gina Rossi (name on the
          server account) · invited", then Let in… — while Let in… sits on the first line beside
          the username and the name takes the whole second line (Q3-53). */}
      <div className="crew-sidebar-waiting-head">
        <span className="crew-sidebar-wrap text-secondary" data-crew-waiting-handle="">
          <PersonName person={joinerPerson(join.username)} context="joiner" />
        </span>
        {second && (
          <p
            className="crew-sidebar-waiting-name crew-sidebar-wrap text-supporting text-text-muted"
            data-crew-waiting-name=""
          >
            {serverName && (
              <>
                {/* Heard as one line with the username: "@gina · Gina Rossi (…) · invited". */}
                <span className="sr-only">{identityCopy.separator}</span>
                <span data-person-part="server-name">
                  <bdi>{serverName}</bdi> ({identityCopy.serverAccountName})
                </span>
              </>
            )}
            {invited && (
              <span data-crew-waiting-state="invited">
                {serverName ? (
                  ` ${copy.waiting.separator} ${copy.waiting.invited}`
                ) : (
                  <>
                    <span className="sr-only">{` ${copy.waiting.separator} `}</span>
                    {copy.waiting.invited}
                  </>
                )}
              </span>
            )}
          </p>
        )}
        <div className="crew-sidebar-waiting-action">{action}</div>
      </div>
      {invited && (
        <p id={nextId} className="text-supporting text-text-muted" data-crew-waiting-next="">
          {copy.waiting.nextStep}
        </p>
      )}
      {join.otherDeviceTried && (
        <p
          className="flex items-start gap-1.5 text-supporting text-text-warning"
          data-crew-waiting-warning=""
        >
          <AlertTriangle className="mt-0.5 size-3 shrink-0" aria-hidden="true" />
          <span>{copy.waiting.otherDevice(join.username)}</span>
        </p>
      )}
    </li>
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
        size="sm"
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
