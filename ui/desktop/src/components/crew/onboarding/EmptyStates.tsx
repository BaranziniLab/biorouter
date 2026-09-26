import { useEffect, useId, useLayoutEffect, useRef, useState, type RefObject } from 'react';
import { Hash, Inbox, KeyRound, Server, Users } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import { EmptyState } from '../../ui/empty-state';
import {
  connectionNames,
  personFromProjection,
  personLabel,
  teamName,
  usePeopleDirectory,
  workspaceName,
} from '../identity';
import type { Invitation, Snapshot } from '../crewApi';
import { serverLabel } from '../sidebar/sidebarView';
import type { ConnectFailureKind } from '../state/connectFailure';
import { useCrew } from '../state/CrewControllerContext';
import { canFocus, focusIsLost, restoreFocusSoon } from '../state/focusReturn';
import type { CrewController } from '../state/types';
import { emptyCopy } from './copy';
import { attemptTime } from './joinText';
import { SetupCard, SetupScreen, Spinner } from './parts';
import { SetupChecklist } from './SetupChecklist';

/**
 * The main-area states outside a channel that are not a join, trust or setup problem: connecting,
 * offline, sign-in needed, no team yet and no open channel. Each says where the person is and
 * offers the one action that moves them on.
 */

/** The snapshot to draw from: the verified one, else (while re-verifying) the last verified one. */
function viewOf(crew: CrewController): Snapshot | null {
  return crew.snapshot ?? crew.lastVerified?.snapshot ?? null;
}

/**
 * The server a connection reaches, for "Connecting to {server}…" and "Sign in to {server}": the
 * person's own SSH alias for it when the daemon found one (D-ALIAS), else its host — the one
 * on-screen name for a saved connection's server (`serverLabel`, Q4-34).
 */
function useServer(): string {
  const { connection } = useCrew();
  return serverLabel(connection) || connection?.name || '';
}

/** The connection's local label: its name, or `name — server` when two share a name. */
function useConnectionLabel(): string {
  const { connection, connections } = useCrew();
  if (!connection) return '';
  return connectionNames(connections).get(connection.id) ?? connection.name;
}

/**
 * When each connection's last connect attempt started, as the connecting card saw it arrive: every
 * connect Crew shows passes through that card (the main area is `connecting` while one runs and
 * nothing verified is on screen), whoever started it — the offline card's Connect, the workspace
 * menu, a chat's "Connect in Crew". Recorded when the card mounts, so the offline card that
 * replaces it once the attempt failed already finds it. Presentation only: the offline card's
 * "Tried again at …" reads it (Q4-07), and nothing decides anything from it.
 */
const connectTriedAt = new Map<string, number>();

/** Tests only: forget every recorded attempt. */
export function resetConnectAttemptsForTests(): void {
  connectTriedAt.clear();
}

/**
 * Marks an element that only holds keyboard focus while the control that had it is gone (the
 * connecting card's title, Q4-09). `focusOnceMounted` treats focus there as unclaimed, so it still
 * moves on to the channel once it opens.
 */
const FOCUS_HOLD = 'data-crew-focus-hold';

/** The props that make an element a focus hold; spread them onto it. */
export interface FocusHoldProps<T extends HTMLElement> {
  ref: RefObject<T | null>;
  tabIndex: -1;
  className: string;
  'data-crew-focus-hold': '';
}

/**
 * A place for keyboard focus to stand while a wait replaces the control that had it (Q4-09): the
 * connecting card's title, and any screen a connect passes through on its way to the channel (the
 * main area's `checking` skeleton). Spread the result onto an element that states the wait; when it
 * mounts with focus lost (on `<body>`, or on an element that just left), focus moves to it before
 * paint, so no frame has focus on the page. Never a Tab stop and never ringed (`tabindex="-1"`,
 * `.crew-onboard-focus-hold`), and `focusOnceMounted` treats it as unclaimed, so focus still moves
 * on to the channel once it opens. It takes nothing from a control that has focus.
 */
export function useFocusHold<T extends HTMLElement>(): FocusHoldProps<T> {
  const ref = useRef<T>(null);
  useLayoutEffect(() => {
    if (focusIsLost()) ref.current?.focus();
  }, []);
  return { ref, tabIndex: -1, className: 'crew-onboard-focus-hold', [FOCUS_HOLD]: '' };
}

export function ConnectingCard() {
  const { connectionId } = useCrew();
  const server = useServer();
  // Connect unmounts itself (the offline screen, the join card's Reconnect): hold focus on this
  // card's title rather than let it fall to `<body>` for as long as the connect takes (Q4-09).
  const hold = useFocusHold<HTMLSpanElement>();

  useEffect(() => {
    if (connectionId) connectTriedAt.set(connectionId, Date.now());
  }, [connectionId]);

  return (
    <SetupScreen>
      <SetupCard
        title={<span {...hold}>{emptyCopy.connecting(server)}</span>}
        testId="crew-connecting"
      >
        <Spinner />
      </SetupCard>
    </SetupScreen>
  );
}

/**
 * The failures the daemon keeps re-dialling by itself (`keepalive::worth_retrying`: a network
 * failure, and an SSH failure it could not classify). A sign-in, host-key or setup failure is
 * final until a person acts, so "Crew keeps trying" is never said for one (Q4-06).
 */
const RETRIED_FAILURES: readonly ConnectFailureKind[] = ['unreachable', 'ssh_failed'];

/** Where focus lands once Connect has left with the offline screen: the channel, when it opens. */
export const CONNECTED_FOCUS_TARGETS: readonly string[] = [
  '.crew-app h1 button',
  '.crew-app textarea[aria-label^="Message"]',
];

export function OfflineState() {
  const { connect, isPending, connectionId, lastConnectFailure } = useCrew();
  const workspace = useConnectionLabel();
  const server = useServer();
  const connectRef = useRef<HTMLButtonElement>(null);
  const pending = isPending('connect');
  const triedId = useId();
  // Re-read the attempt record when a connect ends with this screen still up.
  const [, noteAttempt] = useState(0);
  // After a failed connect of this connection: when it last tried, and why it failed, so a repeat
  // click that fails the same way still visibly did something (Q4-07). Read-only: the controller
  // decides nothing from it.
  const triedAt = connectionId ? (connectTriedAt.get(connectionId) ?? null) : null;
  const tried =
    lastConnectFailure && triedAt !== null
      ? emptyCopy.triedAgain(
          attemptTime(triedAt),
          emptyCopy.failureReason(lastConnectFailure.kind, server || workspace)
        )
      : null;
  const keepsTrying = Boolean(
    tried && lastConnectFailure && RETRIED_FAILURES.includes(lastConnectFailure.kind)
  );

  // The screen replaced what had focus (Retry's note, a closed dialog's opener): put it on the one
  // thing to do here rather than leave it on the page (Q2-20).
  useEffect(() => {
    if (focusIsLost()) connectRef.current?.focus();
  }, []);

  return (
    <SetupScreen>
      <EmptyState
        icon={Server}
        title={emptyCopy.offlineTitle(workspace)}
        description={emptyCopy.offlineBody}
        actions={
          <div className="crew-onboard-offline-actions">
            <Button
              ref={connectRef}
              type="button"
              // Not `disabled` while connecting: a disabled control drops focus to the page.
              aria-disabled={pending || undefined}
              // Focus lands back here after a failed attempt: a screen reader hears when it tried.
              aria-describedby={tried ? triedId : undefined}
              className="crew-onboard-waiting"
              onClick={(event) => {
                if (pending) return;
                const origin = event.currentTarget;
                if (connectionId) connectTriedAt.set(connectionId, Date.now());
                void connect({ userInitiated: true }).then(() => {
                  // Still here (the connect failed and this screen stayed): focus stays on it, and
                  // the line reports the attempt it just made.
                  if (origin.isConnected) {
                    noteAttempt((count) => count + 1);
                    return;
                  }
                  // Connect left with this screen: land on the channel once it opens (Q2-20).
                  restoreFocusSoon(null, CONNECTED_FOCUS_TARGETS);
                  focusOnceMounted(origin, CONNECTED_FOCUS_TARGETS);
                });
              }}
            >
              {emptyCopy.offlineAction(workspace)}
            </Button>
            {tried ? (
              <p
                id={triedId}
                className="text-supporting text-text-muted"
                data-testid="crew-offline-tried"
              >
                {tried}
                {keepsTrying ? ` ${emptyCopy.keepsTrying}` : null}
              </p>
            ) : null}
          </div>
        }
      />
    </SetupScreen>
  );
}

export function SignInNeededState() {
  const { openSignIn, signIn } = useCrew();
  const server = useServer();
  return (
    <SetupScreen>
      <EmptyState
        icon={KeyRound}
        title={emptyCopy.signInTitle(server)}
        description={emptyCopy.signInBody}
        actions={
          <Button type="button" disabled={signIn.open} onClick={openSignIn}>
            {emptyCopy.signInAction}
          </Button>
        }
      />
    </SetupScreen>
  );
}

/** The channel composer a person lands in once a team's channel opens. */
const COMPOSER_SELECTOR = 'textarea[aria-label^="Message #"]';

/** How long focus waits for the joined team's channel to open before it gives up. */
export const LANDING_FOCUS_TIMEOUT_MS = 10_000;

let stopLandingFocus: (() => void) | null = null;

/**
 * "Join {team}" unmounts itself: the invitation card is replaced by the team's channel, and focus
 * would fall to `<body>` (T-15). Once the composer mounts, put focus there, unless the person has
 * already put it somewhere themselves (then nothing moves). Returns a stop.
 */
export function focusComposerOnceMounted(origin: HTMLElement | null): () => void {
  return focusOnceMounted(origin, [COMPOSER_SELECTOR]);
}

/**
 * A control that unmounts itself as it acts (Join {team}, Connect): once the first of `selectors`
 * mounts, put focus on it, unless the person has already put focus somewhere themselves (then
 * nothing moves). Gives up after `LANDING_FOCUS_TIMEOUT_MS`. Returns a stop.
 */
export function focusOnceMounted(
  origin: HTMLElement | null,
  selectors: readonly string[]
): () => void {
  stopLandingFocus?.();
  if (typeof document === 'undefined' || typeof MutationObserver === 'undefined') return () => {};
  let stopped = false;
  const observer = new MutationObserver(() => {
    attempt();
  });
  const timer = setTimeout(() => stop(), LANDING_FOCUS_TIMEOUT_MS);
  function stop() {
    if (stopped) return;
    stopped = true;
    observer.disconnect();
    clearTimeout(timer);
    if (stopLandingFocus === stop) stopLandingFocus = null;
  }
  /**
   * Focus was not moved by the person: it is on nothing, still on the button pressed, or on a card
   * title that only held it while that button was gone (Q4-09).
   */
  function unclaimed(): boolean {
    const active = document.activeElement;
    return (
      !active ||
      active === document.body ||
      !active.isConnected ||
      active === origin ||
      active.hasAttribute(FOCUS_HOLD)
    );
  }
  function attempt() {
    if (stopped) return;
    if (!unclaimed()) {
      stop();
      return;
    }
    for (const selector of selectors) {
      const target = document.querySelector<HTMLElement>(selector);
      if (!canFocus(target)) continue;
      target.focus();
      stop();
      return;
    }
  }
  stopLandingFocus = stop;
  attempt();
  if (!stopped) observer.observe(document.body, { childList: true, subtree: true });
  return stop;
}

/** A team invitation addressed to the viewer that has not expired. */
function pendingTeamInvitation(view: Snapshot): Invitation | null {
  return (
    view.invitations.find(
      (invitation) =>
        invitation.kind === 'team' &&
        invitation.principal_id === view.actor.id &&
        invitation.expired !== true
    ) ?? null
  );
}

/**
 * A verified workspace with no team. The host gets the setup checklist; someone with a team
 * invitation gets "Join {team}"; any other member is told whom to ask, with Create a team as the
 * quiet alternative. Nothing is enabled from the last verified view while it re-verifies.
 */
export function NoTeamState() {
  const crew = useCrew();
  const view = viewOf(crew);
  const directory = usePeopleDirectory(view, crew.labels);
  const live = Boolean(crew.snapshot);
  if (!view) return null;
  if (crew.isHost) {
    return (
      <SetupScreen>
        <SetupChecklist force />
      </SetupScreen>
    );
  }
  const workspace = workspaceName(view.workspace, directory.host);
  const invitation = pendingTeamInvitation(view);
  if (invitation) {
    const team = invitation.target_name
      ? teamName({ id: invitation.target_id, name: invitation.target_name })
      : emptyCopy.aTeam;
    const inviter =
      personFromProjection(invitation.inviter) ?? directory.byId(invitation.inviter_id);
    const accepting = crew.isPending('mutate:invitation.accept');
    return (
      <SetupScreen>
        <EmptyState
          icon={Inbox}
          title={emptyCopy.invitedTitle(team)}
          description={
            inviter
              ? emptyCopy.invitedBody(personLabel(inviter, 'inline', directory))
              : emptyCopy.invitedBodyUnknown
          }
          actions={
            <Button
              type="button"
              disabled={!live || accepting}
              onClick={(event) => {
                const origin = event.currentTarget;
                void crew
                  .act('global', 'mutate:invitation.accept', async () => {
                    await crew.mutate('invitation.accept', { invitation_id: invitation.id });
                    return true;
                  })
                  .then((joined) => {
                    if (joined) focusComposerOnceMounted(origin);
                  });
              }}
            >
              {emptyCopy.invitedAction(team)}
            </Button>
          }
        />
      </SetupScreen>
    );
  }
  const host = directory.host
    ? personLabel(directory.host, 'inline', directory)
    : emptyCopy.yourHost;
  return (
    <SetupScreen>
      <EmptyState
        icon={Users}
        title={emptyCopy.memberTitle(workspace)}
        description={emptyCopy.memberBody(host)}
        actions={
          <Button
            type="button"
            variant="outline"
            disabled={!live}
            onClick={() => crew.openDialog({ kind: 'create-team' })}
          >
            {emptyCopy.memberAction}
          </Button>
        }
      />
    </SetupScreen>
  );
}

/** A team with no open channel (fixes L15: it offers Create channel). */
export function NoChannelState() {
  const crew = useCrew();
  const view = viewOf(crew);
  if (!view) return null;
  const team = view.teams.find((item) => item.id === crew.teamId) ?? view.teams[0] ?? null;
  const live = Boolean(crew.snapshot);
  return (
    <SetupScreen>
      <EmptyState
        icon={Hash}
        title={emptyCopy.noChannelTitle(teamName(team))}
        description={emptyCopy.noChannelBody}
        actions={
          <Button
            type="button"
            disabled={!live || !team}
            onClick={() => team && crew.openDialog({ kind: 'create-channel', teamId: team.id })}
          >
            {emptyCopy.noChannelAction}
          </Button>
        }
      />
    </SetupScreen>
  );
}
