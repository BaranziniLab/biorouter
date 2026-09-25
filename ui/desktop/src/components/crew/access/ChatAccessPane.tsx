import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent,
  type ReactNode,
} from 'react';
import { useNavigate } from 'react-router-dom';
import {
  cacheGet,
  isDefaultSessionName,
  subscribeSessionNameChanges,
} from '../../../utils/sessionNameSync';
import { sessionGrantState, type CrewSessionGrant } from '../api/grants';
import { AlertCircle } from '../../icons/app-icons';
import { Badge } from '../../ui/badge';
import { Button } from '../../ui/button';
import { Checkbox } from '../../ui/Checkbox';
import { Disclosure } from '../../ui/disclosure';
import { Note } from '../../ui/note';
import {
  PersonName,
  channelName,
  connectionNames,
  identityCopy,
  teamName,
  usePeopleDirectory,
  type DaemonPersonLabels,
} from '../identity';
import { useCrew, useCrewErrorSlot, useCrewSurfaceReset } from '../state/CrewControllerContext';
import { accessStatusOf, accessStatusTone, channelLabels, chatTitleOf } from './accessRows';
import { rememberChannelLabels } from './chatCrewAccess';
import { accessCopy } from './copy';
import { InlineConfirm, RevokeResultNote } from './RevokeControls';
import {
  announceGrantsChanged,
  isUnconfirmedRevocation,
  useCrewGrants,
  type RevokeOutcome,
} from './useCrewGrants';

export interface ChatAccessPaneProps {
  /**
   * The chat to show. Defaults to the pane intent's `sessionId`, then to the chat that sent the
   * person here with `/crew` (`?sessionId=`).
   */
  sessionId?: string;
  /** Layout only. */
  className?: string;
}

const chatRoute = (sessionId: string) => `/pair?resumeSessionId=${encodeURIComponent(sessionId)}`;

/**
 * Whether focus is where the details pane put it on opening — its own heading, or nowhere — and not
 * somewhere the person has since moved it.
 */
function focusIsOnPaneHeading(from: HTMLElement): boolean {
  const active = document.activeElement;
  if (!active || active === document.body) return true;
  const pane = from.closest('aside');
  return Boolean(pane?.contains(active) && active.tagName === 'H2');
}

/** How long after the consent appears the pane's own heading focus is still redirected to Allow. */
const ALLOW_FOCUS_WINDOW_MS = 1000;

/**
 * Focus Allow when the consent appears (live QA round 4, Q4-13). The details pane focuses its
 * heading on opening, and after `/crew` + Enter the browser drew its default `outline: auto` box
 * round it, which read as a text field. The consent's one action takes focus instead, but only
 * from that heading (or from nowhere): focus the person moved elsewhere stays put.
 *
 * The pane focuses its heading in an effect that may run before or after the consent mounts (the
 * grant list may still be loading), so both orders are covered: a check once this commit is done,
 * and, for a short while, a redirect when the heading takes focus. A held Enter's repeats never
 * press Allow: see {@link onAllowKeyDown}.
 */
function useFocusAllowOnConsent() {
  const stop = useRef<(() => void) | null>(null);
  useEffect(() => () => stop.current?.(), []);
  return useCallback((button: HTMLButtonElement | null) => {
    stop.current?.();
    stop.current = null;
    if (!button) return;
    const move = () => {
      if (button.isConnected && !button.disabled && focusIsOnPaneHeading(button)) button.focus();
    };
    const pane = button.closest('aside');
    const onFocusIn = (event: FocusEvent) => {
      if (event.target instanceof HTMLElement && event.target.tagName === 'H2') move();
    };
    pane?.addEventListener('focusin', onFocusIn);
    const check = window.setTimeout(move, 0);
    const done = window.setTimeout(() => stop.current?.(), ALLOW_FOCUS_WINDOW_MS);
    stop.current = () => {
      pane?.removeEventListener('focusin', onFocusIn);
      window.clearTimeout(check);
      window.clearTimeout(done);
      stop.current = null;
    };
  }, []);
}

/**
 * Allow is the consent, so only a deliberate press grants: the auto-repeat of an Enter still held
 * from `/crew` (or from any earlier control) is not one.
 */
function onAllowKeyDown(event: KeyboardEvent<HTMLButtonElement>) {
  if ((event.key === 'Enter' || event.key === ' ') && event.repeat) event.preventDefault();
}

function knownTitle(name: string | null | undefined): string | null {
  return name && !isDefaultSessionName(name) ? chatTitleOf({ session_name: name }) : null;
}

/**
 * The chat's title as this window already knows it — the chat store's recent-sessions cache and its
 * rename broadcasts — so the consent can name the chat BEFORE Allow, when no grant row carries
 * `session_name` yet (live QA round 1, T-55). Never fetches: an unknown or default title
 * ("New chat") is `null`, and the pane says "This chat".
 */
export function useKnownChatTitle(sessionId: string | null): string | null {
  const [title, setTitle] = useState(() =>
    sessionId ? knownTitle(cacheGet(sessionId)?.session.name) : null
  );
  useEffect(() => {
    if (!sessionId) {
      setTitle(null);
      return;
    }
    setTitle(knownTitle(cacheGet(sessionId)?.session.name));
    return subscribeSessionNameChanges((change) => {
      if (change.sessionId === sessionId) setTitle(knownTitle(change.name));
    });
  }, [sessionId]);
  return title;
}

/**
 * The body of the details pane in `chat-access` mode (ui-redesign-spec, "Revoke", "The Chat access
 * pane"; the pane's header, title and close control belong to the details pane).
 *
 * - **No grant:** the consent summary — which chat, what it will be able to read and where it will
 *   post, as whom — with Advanced "Also read", and **Allow “Plot review” to read and post in
 *   #general** (the pinned **Allow this conversation to read and post here** while the chat's title
 *   is unknown).
 * - **After Allow:** "Connected.", a primary **Back to chat** and **Revoke access**. It does not
 *   navigate by itself (L10), so the person sees where Revoke lives.
 * - **Active:** the summary with its "Active · ends 4:40 PM" badge (plain "Active" only until the
 *   new grant is listed), **Open chat** and **Revoke access**, which asks inline first.
 * - **Results:** a confirmed revoke (200) says so, with Open chat and Done; a 503 is "Stopped on
 *   this device" with Retry; anything else is "Not revoked" with the daemon's words and Retry.
 *
 * React gates nothing: Revoke is offered whenever the daemon lists a grant (or just granted one),
 * and the daemon decides what happens. Errors from Allow render here, in this pane's error slot.
 */
export function ChatAccessPane({ sessionId: sessionProp, className }: ChatAccessPaneProps) {
  const controller = useCrew();
  const {
    connections,
    connectionId,
    connectionsState,
    channel,
    channelId,
    snapshot,
    labels,
    grantSessionId,
    contextChannels,
    setContextChannels,
    ui,
    subscribeSurfaceReset,
  } = controller;
  const navigate = useNavigate();
  const intent = ui.pane?.mode === 'chat-access' ? ui.pane : null;
  const sessionId = sessionProp ?? intent?.sessionId ?? grantSessionId ?? null;
  const ids = useMemo(() => connections.map((item) => item.id), [connections]);
  const grants = useCrewGrants(ids, {
    enabled: Boolean(sessionId),
    cacheScope: subscribeSurfaceReset,
  });
  const { refetch } = grants;
  useCrewSurfaceReset((reason) => {
    if (sessionId && reason === 'refresh') refetch();
  });
  const showError = useCrewErrorSlot('pane:chat-access');
  const dir = usePeopleDirectory(snapshot, labels as DaemonPersonLabels | null);
  const destinations = useMemo(() => channelLabels(snapshot), [snapshot]);
  const workspaces = useMemo(() => connectionNames(connections), [connections]);

  const [granted, setGranted] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [revoking, setRevoking] = useState(false);
  const [outcome, setOutcome] = useState<RevokeOutcome | null>(null);
  const revokeButton = useRef<HTMLButtonElement>(null);
  const allowButton = useFocusAllowOnConsent();
  const cachedTitle = useKnownChatTitle(sessionId);

  // A new pane intent (Review, Manage, Grant again, a row) or another chat starts fresh.
  useEffect(() => {
    setGranted(false);
    setConfirming(false);
    setOutcome(null);
  }, [intent, sessionId]);

  if (!sessionId) {
    return (
      <div className={className} data-testid="crew-chat-access-pane">
        <p className="text-supporting text-text-muted">{accessCopy.paneNoChat}</p>
      </div>
    );
  }

  const grant: CrewSessionGrant | null =
    grants.grants.find((item) => item.session_id === sessionId) ?? null;
  const state = grant ? sessionGrantState(grant) : null;
  // A revoke that stopped only on this device: the grant is no longer usable here, whatever the
  // list said a moment ago, and it stays that way until the workspace confirms or it is granted
  // again.
  const stoppedHere =
    outcome?.kind === 'unconfirmed' ||
    (grant !== null &&
      state !== 'active' &&
      isUnconfirmedRevocation(grant.connection_id, grant.session_id));
  const active = !stoppedHere && (granted || state === 'active');
  // The daemon's name for the chat when a grant lists one; else the title this window knows.
  const chat = (grant ? chatTitleOf(grant) : null) ?? cachedTitle;
  const canGrant = sessionId === grantSessionId;
  // The grant's own connection when it is listed; this one when it was just granted here.
  const target = grant?.connection_id ?? (granted ? connectionId : null);
  const sameConnection = !grant || grant.connection_id === connectionId;

  const destinationOf = (id: string) =>
    sameConnection
      ? (destinations.get(id) ?? accessCopy.unknownChannel)
      : accessCopy.unknownChannel;
  // What the listed grant can do (its own destination), and what Allow would grant: always the
  // selected channel plus the channels chosen under Advanced, whatever an old grant named.
  const here = channel ? channelName(channel) : accessCopy.unknownChannel;
  const consentExtras = contextChannels.filter(
    (id, index, all) => id && id !== channelId && all.indexOf(id) === index
  );
  const listed = grant && !granted;
  const destination = listed
    ? sameConnection
      ? destinationOf(grant.channel_id)
      : accessCopy.chatDestinationWorkspace(
          workspaces.get(grant.connection_id) ?? identityCopy.unnamedWorkspace
        )
    : here;
  const extraSources = listed
    ? grant.source_channels.filter(
        (id, index, all) => id && id !== grant.channel_id && all.indexOf(id) === index
      )
    : consentExtras;

  const openChat = () => navigate(chatRoute(sessionId));

  const revoke = async () => {
    if (!target) return;
    setConfirming(false);
    setRevoking(true);
    // For the past-access record (Q4-12): the listed grant, or, just after Allow here, what Allow
    // granted — the daemon's answer names the run.
    const result = await grants.revoke(
      target,
      sessionId,
      listed || !granted
        ? grant
        : {
            channel_id: channelId,
            session_name: chat,
            kind: 'chat',
            source_channels: [channelId, ...consentExtras],
          }
    );
    setRevoking(false);
    setOutcome(result);
    if (result.kind === 'revoked') setGranted(false);
  };

  const allow = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const ok = await controller.act('pane:chat-access', 'grant', async () => {
      await controller.grantSession({ contextChannels, sessionId });
      return true;
    });
    if (!ok) return;
    setOutcome(null);
    setGranted(true);
    rememberChannelLabels(connectionId, destinations, [channelId]);
    announceGrantsChanged({ connectionId, sessionId, change: 'granted' });
  };

  const errorNote =
    showError && controller.error ? (
      <Note tone="danger" icon={AlertCircle} role="alert" testId="crew-chat-access-error">
        {controller.error.message}
      </Note>
    ) : null;

  // ── Loading and failure, before anything is known ────────────────────────────────────────
  if (!granted && !grant && (connectionsState !== 'loaded' || grants.status === 'loading')) {
    return (
      <div className={className} data-testid="crew-chat-access-pane">
        <p role="status" className="text-supporting text-text-muted">
          {accessCopy.noteChecking}
        </p>
      </div>
    );
  }
  if (!granted && !grant && grants.anyFailed) {
    return (
      <div className={className} data-testid="crew-chat-access-pane">
        <Note
          tone="warning"
          icon={AlertCircle}
          role="alert"
          action={
            <Button
              type="button"
              variant="ghost"
              size="sm"
              aria-label={accessCopy.listRetryName}
              onClick={refetch}
            >
              {accessCopy.retry}
            </Button>
          }
        >
          {grants.error ?? accessCopy.listFailed}
        </Note>
      </div>
    );
  }

  // ── A confirmed revoke ───────────────────────────────────────────────────────────────────
  if (outcome?.kind === 'revoked') {
    return (
      <div className={className} data-testid="crew-chat-access-pane">
        <RevokeResultNote
          outcome={outcome}
          chat={chat}
          onRetry={() => void revoke()}
          successActions={
            <>
              <Button type="button" variant="secondary" size="sm" onClick={openChat}>
                {accessCopy.openChat}
              </Button>
              <Button type="button" variant="ghost" size="sm" onClick={controller.closePane}>
                {accessCopy.done}
              </Button>
            </>
          }
        />
      </div>
    );
  }

  // You, at the authority point: whom the chat posts as. Only for this connection's grants.
  const me = dir.me ? <PersonName person={dir.me} context="authority" dir={dir} /> : null;
  const summaryLines = (future: boolean, where: string, extras: string[], who: ReactNode) => (
    <ul className="flex flex-col gap-1 text-secondary text-text-default">
      <li>
        {future ? accessCopy.read(where) : accessCopy.reads(where)}
        {extras.length > 0 && (
          <span className="text-text-muted">
            {', '}
            {extras.map((id) => destinationOf(id)).join(', ')}
          </span>
        )}
      </li>
      <li>
        {future ? accessCopy.postAs(where) : accessCopy.postsAs(where)}
        {who ? <> {who}</> : null}
      </li>
    </ul>
  );

  // ── Stopped on this device, waiting for the workspace to confirm ─────────────────────────
  if (stoppedHere) {
    return (
      <div className={className} data-testid="crew-chat-access-pane">
        <div className="flex flex-col gap-3">
          <div className="flex items-start justify-between gap-2">
            <p className="min-w-0 text-label text-text-default">{accessCopy.chatName(chat)}</p>
            <Badge tone="warning">{accessCopy.status.unconfirmed}</Badge>
          </div>
          <RevokeResultNote
            outcome={
              outcome?.kind === 'unconfirmed'
                ? outcome
                : { kind: 'unconfirmed', message: accessCopy.unconfirmed }
            }
            chat={chat}
            onRetry={() => void revoke()}
            retrying={revoking}
          />
          <div>
            <Button type="button" variant="secondary" size="sm" onClick={openChat}>
              {accessCopy.openChat}
            </Button>
          </div>
        </div>
      </div>
    );
  }

  // ── Active: just granted here, or listed active ──────────────────────────────────────────
  if (active) {
    // The listed grant's own badge whenever it is the active one, so the same state reads "Active ·
    // ends 4:40 PM" both right after Allow (once the list has the grant) and when reopened.
    const badge =
      grant && state === 'active'
        ? accessStatusOf(grant, Date.now(), false)
        : { status: 'active' as const, label: accessCopy.status.active };
    return (
      <div className={className} data-testid="crew-chat-access-pane">
        <div className="flex flex-col gap-3">
          <div className="flex items-start justify-between gap-2">
            <p className="min-w-0 text-label text-text-default">{accessCopy.canNow(chat)}</p>
            <Badge tone={accessStatusTone(badge.status)}>{badge.label}</Badge>
          </div>
          {granted ? (
            <p role="status" className="text-secondary text-text-success">
              {accessCopy.connected}
            </p>
          ) : null}
          {summaryLines(false, destination, extraSources, sameConnection ? me : null)}
          <p className="text-supporting text-text-muted">{accessCopy.expiry}</p>
          {outcome ? (
            <RevokeResultNote
              outcome={outcome}
              chat={chat}
              onRetry={() => void revoke()}
              retrying={revoking}
            />
          ) : null}
          {confirming ? (
            <InlineConfirm
              question={accessCopy.confirm(chat, destination)}
              detail={accessCopy.confirmStops}
              confirmLabel={accessCopy.confirmRevoke}
              cancelLabel={accessCopy.confirmKeep}
              pending={revoking}
              onConfirm={() => void revoke()}
              onCancel={() => {
                setConfirming(false);
                window.setTimeout(() => revokeButton.current?.focus(), 0);
              }}
            />
          ) : (
            <div className="flex flex-wrap items-center justify-between gap-2">
              {granted ? (
                <Button type="button" size="sm" onClick={openChat}>
                  {accessCopy.backToChat}
                </Button>
              ) : (
                <Button type="button" variant="secondary" size="sm" onClick={openChat}>
                  {accessCopy.openChat}
                </Button>
              )}
              {outcome ? null : (
                <Button
                  ref={revokeButton}
                  type="button"
                  variant="destructive"
                  size="sm"
                  disabled={revoking || !target}
                  onClick={() => setConfirming(true)}
                >
                  {accessCopy.revokeButton}
                </Button>
              )}
            </div>
          )}
        </div>
      </div>
    );
  }

  // ── A task's access after the task: it ended with it, and a task is not granted again ────────
  if (grant && grant.kind === 'task') {
    return (
      <div className={className} data-testid="crew-chat-access-pane">
        <div className="flex flex-col gap-3">
          <p className="text-label text-text-default">{accessCopy.paneTaskFinished}</p>
          <div>
            <Button type="button" variant="secondary" size="sm" onClick={openChat}>
              {accessCopy.openChat}
            </Button>
          </div>
        </div>
      </div>
    );
  }

  // ── Revoked or expired, or never granted: the consent (only for the chat that sent us here) ─
  const lapsed =
    grant && state === 'expired'
      ? accessCopy.paneExpired(chat)
      : grant
        ? accessCopy.paneRevoked(chat)
        : null;

  if (!canGrant) {
    return (
      <div className={className} data-testid="crew-chat-access-pane">
        <div className="flex flex-col gap-3">
          {lapsed ? <p className="text-label text-text-default">{lapsed}</p> : null}
          <p className="text-supporting text-text-muted">{accessCopy.reconnectHow}</p>
          <div>
            <Button type="button" variant="secondary" size="sm" onClick={openChat}>
              {accessCopy.openChat}
            </Button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className={className} data-testid="crew-chat-access-pane">
      <form className="flex flex-col gap-3" onSubmit={(event) => void allow(event)}>
        {lapsed ? <p className="text-label text-text-default">{lapsed}</p> : null}
        <p className="text-label text-text-default">{accessCopy.willBeAble(chat)}</p>
        {summaryLines(true, here, consentExtras, me)}
        <p className="text-supporting text-text-muted">{accessCopy.expiry}</p>
        <AlsoRead
          contextChannels={contextChannels}
          setContextChannels={setContextChannels}
          currentChannelId={channelId}
          currentChannelName={here}
        />
        {errorNote}
        <div className="flex justify-end">
          {/* It names the chat, and a chat's title can be long: the label wraps rather than
              overflowing the pane or hiding which chat is being let in. The button's base class is
              `shrink-0`, so wrapping alone never narrowed it: its one-line width overflowed a
              328px pane to the left and clipped "Allow" off the front (live QA round 2, Q2-06).
              It takes the row's width instead, and its lines are centred. */}
          <Button
            key="crew-chat-access-allow"
            ref={allowButton}
            onKeyDown={onAllowKeyDown}
            type="submit"
            className="h-auto min-h-control-md w-full min-w-0 max-w-full whitespace-normal break-words py-1.5 text-center"
            disabled={controller.isPending('grant')}
          >
            {chat && channel ? accessCopy.allowChat(chat, here) : accessCopy.allow}
          </Button>
        </div>
      </form>
    </div>
  );
}

/**
 * Advanced "Also read": the other channels the chat may read. Unmounted while closed, so its
 * checkboxes are not in the document until the person asks for them.
 */
function AlsoRead({
  contextChannels,
  setContextChannels,
  currentChannelId,
  currentChannelName,
}: {
  contextChannels: string[];
  setContextChannels(ids: string[]): void;
  currentChannelId: string;
  /** The selected channel as the consent names it, for the closed summary. */
  currentChannelName: string;
}) {
  const { snapshot } = useCrew();
  const candidates = useMemo(
    () =>
      (snapshot?.channels ?? []).filter((item) => item.id !== currentChannelId && !item.archived),
    [snapshot, currentChannelId]
  );
  const labels = useMemo(() => {
    // "Team / #channel" on every row: the list spans teams.
    const teams = new Map((snapshot?.teams ?? []).map((team) => [team.id, team]));
    return new Map(
      candidates.map((item) => [
        item.id,
        `${teamName(teams.get(item.team_id))} / ${channelName(item)}`,
      ])
    );
  }, [snapshot, candidates]);
  if (candidates.length === 0) return null;
  const chosen = contextChannels.filter((id) => candidates.some((item) => item.id === id));
  return (
    <Disclosure
      summary={accessCopy.alsoReadSummary(chosen.length, currentChannelName)}
      defaultOpen={chosen.length > 0}
    >
      <fieldset className="flex flex-col gap-1">
        <legend className="mb-1 text-label text-text-default">{accessCopy.alsoRead}</legend>
        {candidates.map((item) => (
          <label key={item.id} className="flex min-h-control-md items-center gap-2 text-secondary">
            <Checkbox
              checked={contextChannels.includes(item.id)}
              onChange={(event) =>
                setContextChannels(
                  event.target.checked
                    ? [...contextChannels, item.id]
                    : contextChannels.filter((id) => id !== item.id)
                )
              }
            />
            <span className="min-w-0 truncate">{labels.get(item.id)}</span>
          </label>
        ))}
      </fieldset>
    </Disclosure>
  );
}
