import { useEffect, useId, useMemo, useRef, type ReactNode } from 'react';
import { AlertTriangle } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../../ui/dropdown-menu';
import type { Snapshot } from '../crewApi';
import { directAddSupported } from '../dialogs/people';
import {
  identityCopy,
  joinerPerson,
  PersonName,
  personLabel,
  type PeopleDirectory,
} from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import { sidebarCopy } from './copy';
import { useSidebarAnnounce } from './SidebarAnnouncer';
import {
  invitationsToMe,
  joinedWithoutTeam,
  teamSections,
  useSidebarView,
  waitingToJoin,
  type InvitationRow,
  type JoinedRow,
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
// Who joined and is in none of the host's teams (Q3-52)
// ---------------------------------------------------------------------------------------------

/** The people newly in "Joined, not in your teams": in `after` and not in `before`. */
export function newlyWithoutTeam(
  before: ReadonlySet<string> | null,
  after: readonly JoinedRow[]
): JoinedRow[] {
  if (!before) return [];
  return after.filter((row) => !before.has(row.id));
}

/**
 * Speaks, politely, each person who newly appears in "Joined, not in your teams" (Q3-52). The
 * first verified view of a workspace is the baseline, as for Waiting to join, so opening Crew
 * never reads out the people already listed. The joined toast says THAT someone joined; this says
 * what is left to do, so the two never repeat each other.
 */
function useJoinedAnnouncements(
  rows: readonly JoinedRow[] | null,
  connectionId: string,
  workspaceId: string,
  dir: PeopleDirectory
) {
  const { announce } = useSidebarAnnounce();
  const seen = useRef<{ key: string; ids: Set<string> } | null>(null);

  useEffect(() => {
    if (!rows) return;
    const key = `${connectionId}\u0000${workspaceId}`;
    const before = seen.current?.key === key ? seen.current.ids : null;
    seen.current = { key, ids: new Set(rows.map((row) => row.id)) };
    const fresh = newlyWithoutTeam(before, rows);
    if (fresh.length === 0) return;
    announce(
      fresh.map((row) => copy.joined.announce(personLabel(row.person, 'inline', dir))).join(' ')
    );
  }, [rows, connectionId, workspaceId, dir, announce]);
}

/** A team the host can add someone to, as the picker names it. */
interface AddableTeam {
  id: string;
  name: string;
}

// ---------------------------------------------------------------------------------------------
// The sections
// ---------------------------------------------------------------------------------------------

/**
 * The sidebar's attention sections (ui-redesign-spec, "The Crew sidebar"), each shown only when
 * it has a row: **Invitations** addressed to me, with a small Join, and — for the host —
 * **Waiting to join**, with Let in… and the different-code warning, or, once an invitation has
 * run out, "Invitation expired" and Invite again…, and no Let in.
 *
 * A row still waiting for its code reads `@frank`, then `Frank Okafor (name on the server
 * account) · invited` on a line of its own, over "Let in… when they send their code" (Q2-42):
 * whose turn it is — the joiner sends a code, then the host lets them in — which the bare name and
 * button never said. The row's state — "invited", "Code entered" or "Invitation expired" — always
 * ends that second line, and only a button ever sits beside the username, which drops below it
 * when the two don't fit: the name and the state wrap rather than truncate, and the username is
 * never squeezed (Q3-53: one line cut "Gina Rossi · invited" to "Gina …"; then "Invitation
 * expired · Invite again…" beside it left `@crew_gina` a column one letter wide). A row whose
 * code the host entered reads "Code entered", not "Approved": the broker compares the code only
 * when the joiner's computer checks in (T-13). When a claim with a different code was refused,
 * the warning says what to do, and Let in… stays offered so a mistyped code can be entered again
 * (the dialog then offers Replace code).
 *
 * Last, for the host, **Joined, not in your teams** (Q3-52): each person who joined and is in none
 * of the host's teams, as "{name} · joined" with **Add to a team…** — straight to Add people for
 * the only team the host can add to, or a picker of them. The row leaves once the person is in
 * one; someone newly listed is announced politely. "Your" teams, because the snapshot holds only
 * the teams the host is in (`joinedWithoutTeam`).
 *
 * All of them name people through `PersonName` and never show an ID: an invitation the broker did
 * not name yet reads "Invitation" from its inviter, and a joiner reads `@bob` first (the joiner
 * context), because at a host's decision the account name is what matters.
 */
export function AttentionSections() {
  const crew = useCrew();
  const { snapshot, verified, dir, title } = useSidebarView(crew);
  const invitations = useMemo(() => invitationsToMe(snapshot, dir), [snapshot, dir]);
  const waiting = useMemo(() => waitingToJoin(snapshot), [snapshot]);
  useWaitingAnnouncements(snapshot, verified, crew.connectionId, title);
  const joined = useMemo(
    () => (crew.isHost ? joinedWithoutTeam(snapshot, dir) : []),
    [crew.isHost, snapshot, dir]
  );
  useJoinedAnnouncements(
    crew.isHost && verified ? joined : null,
    crew.connectionId,
    snapshot?.workspace?.id ?? '',
    dir
  );
  // The teams the host may add people to: `team.add_member` lets the host add to any of them where
  // the broker adds directly (`direct_add_v1`); otherwise inviting is the creator's alone (Q2-41).
  const directAdd = directAddSupported(crew.capabilities);
  const addable = useMemo<AddableTeam[]>(() => {
    const me = snapshot?.actor?.id;
    const creators = new Map((snapshot?.teams ?? []).map((team) => [team.id, team.created_by]));
    return teamSections(snapshot)
      .filter((section) => directAdd || (me !== undefined && creators.get(section.id) === me))
      .map((section) => ({ id: section.id, name: section.name }));
  }, [snapshot, directAdd]);
  if (invitations.length === 0 && waiting.length === 0 && joined.length === 0) return null;

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
      {joined.length > 0 && (
        <Section label={copy.section.joined} attention="joined">
          {joined.map((row) => (
            <JoinedItem key={row.id} row={row} teams={addable} dir={dir} actionable={verified} />
          ))}
        </Section>
      )}
    </>
  );
}

/**
 * One "Joined, not in your teams" row (Q3-52): "{name} · joined", and Add to a team… — the Add
 * people dialog for the only team the host can add to, or a picker when there are several. With
 * none, the row still says who is waiting to be placed.
 */
function JoinedItem({
  row,
  teams,
  dir,
  actionable,
}: {
  row: JoinedRow;
  teams: readonly AddableTeam[];
  dir: PeopleDirectory;
  /** False while the sidebar shows only the last verified copy: nothing is actionable then. */
  actionable: boolean;
}) {
  const crew = useCrew();
  const name = personLabel(row.person, 'inline', dir);
  const addTo = (teamId: string) =>
    crew.openDialog({ kind: 'add-people', target: 'team', targetId: teamId });
  const only = teams.length === 1 ? teams[0] : null;

  return (
    <li className="flex min-w-0 items-center gap-2 px-2 py-1.5" data-crew-joined="">
      <span className="crew-sidebar-wrap min-w-0 flex-1 text-secondary">
        <PersonName person={row.person} context="inline" dir={dir} />
        <span className="text-text-muted" data-crew-joined-state="">
          {` ${copy.waiting.separator} ${copy.joined.state}`}
        </span>
      </span>
      {only ? (
        <Button
          variant="secondary"
          size="xs"
          className="no-drag"
          aria-label={copy.joined.addToTeamNamedLabel(name, only.name)}
          disabled={!actionable}
          onClick={() => addTo(only.id)}
        >
          {copy.joined.addToTeam}
        </Button>
      ) : teams.length > 1 ? (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="secondary"
              size="xs"
              className="no-drag"
              aria-label={copy.joined.addToTeamLabel(name)}
              disabled={!actionable}
            >
              {copy.joined.addToTeam}
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" data-crew-menu="joined-team">
            {teams.map((team) => (
              <DropdownMenuItem key={team.id} onSelect={() => addTo(team.id)}>
                <bdi className="crew-sidebar-truncate">{team.name}</bdi>
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      ) : null}
    </li>
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
  // Where the row stands, always on the name's line (Q3-53): a sentence beside the username left
  // it a column one letter wide ("Invitation expired · Invite again…" is about as wide as the row).
  const state = join.expired
    ? { key: 'expired', text: copy.waiting.expired }
    : join.approved
      ? { key: 'code-entered', text: copy.waiting.approved }
      : { key: 'invited', text: copy.waiting.invited };
  // Only ever a button: Invite again…, Let in…, or — for a code entered — Let in… again only after
  // a different code was tried, so a typo can be fixed.
  const action = join.expired ? (
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
  ) : join.approved ? (
    join.otherDeviceTried ? (
      letIn(join.username)
    ) : null
  ) : (
    letIn(join.username, nextId)
  );
  return (
    <li className="flex flex-col gap-1 px-2 py-1.5" data-crew-waiting={join.username}>
      {/* The DOM reads in speaking order — `@gina`, then "Gina Rossi (name on the server
          account) · invited", then Let in… — while the CSS puts Let in… beside the username when
          both fit and below it when they don't, and the name and state on a line of their own
          (Q3-53). */}
      <div className="crew-sidebar-waiting-head">
        <span
          className="crew-sidebar-waiting-handle crew-sidebar-wrap text-secondary"
          data-crew-waiting-handle=""
        >
          <PersonName person={joinerPerson(join.username)} context="joiner" />
        </span>
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
          <span data-crew-waiting-state={state.key}>
            {serverName ? (
              ` ${copy.waiting.separator} ${state.text}`
            ) : (
              <>
                <span className="sr-only">{` ${copy.waiting.separator} `}</span>
                {state.text}
              </>
            )}
          </span>
        </p>
        {action && <div className="crew-sidebar-waiting-action">{action}</div>}
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
