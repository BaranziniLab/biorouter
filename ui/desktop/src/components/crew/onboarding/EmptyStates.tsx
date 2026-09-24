import { Hash, Inbox, KeyRound, Server, Users } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import { EmptyState } from '../../ui/empty-state';
import {
  connectionNames,
  connectionServer,
  personFromProjection,
  personLabel,
  teamName,
  usePeopleDirectory,
  workspaceName,
} from '../identity';
import type { Invitation, Snapshot } from '../crewApi';
import { useCrew } from '../state/CrewControllerContext';
import type { CrewController } from '../state/types';
import { emptyCopy } from './copy';
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

/** The server a connection reaches, for "Connecting to {server}…" and "Sign in to {server}". */
function useServer(): string {
  const { connection } = useCrew();
  return connectionServer(connection) || connection?.name || '';
}

/** The connection's local label: its name, or `name — server` when two share a name. */
function useConnectionLabel(): string {
  const { connection, connections } = useCrew();
  if (!connection) return '';
  return connectionNames(connections).get(connection.id) ?? connection.name;
}

export function ConnectingCard() {
  const server = useServer();
  return (
    <SetupScreen>
      <SetupCard title={emptyCopy.connecting(server)} testId="crew-connecting">
        <Spinner />
      </SetupCard>
    </SetupScreen>
  );
}

export function OfflineState() {
  const { connect, isPending } = useCrew();
  const workspace = useConnectionLabel();
  return (
    <SetupScreen>
      <EmptyState
        icon={Server}
        title={emptyCopy.offlineTitle(workspace)}
        description={emptyCopy.offlineBody}
        actions={
          <Button
            type="button"
            disabled={isPending('connect')}
            onClick={() => void connect({ userInitiated: true })}
          >
            {emptyCopy.offlineAction(workspace)}
          </Button>
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
  /** Focus was not moved by the person: it is on nothing, or still on the button pressed. */
  function unclaimed(): boolean {
    const active = document.activeElement;
    return !active || active === document.body || !active.isConnected || active === origin;
  }
  function attempt() {
    if (stopped) return;
    if (!unclaimed()) {
      stop();
      return;
    }
    const composer = document.querySelector<HTMLElement>(COMPOSER_SELECTOR);
    if (!composer) return;
    composer.focus();
    stop();
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
