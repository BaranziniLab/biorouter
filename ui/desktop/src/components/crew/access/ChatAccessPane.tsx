import {
  useCallback,
  useEffect,
  useId,
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
import { grantDestinationLabel, sessionGrantState, type CrewSessionGrant } from '../api/grants';
import { AlertCircle, AlertTriangle } from '../../icons/app-icons';
import { Badge } from '../../ui/badge';
import { Button } from '../../ui/button';
import { Checkbox } from '../../ui/Checkbox';
import { Disclosure } from '../../ui/disclosure';
import { Note } from '../../ui/note';
import {
  PersonName,
  channelName,
  channelNamesAcrossTeams,
  connectionNames,
  identityCopy,
  isMachineIdShaped,
  sanitizeDisplayText,
  usePeopleDirectory,
  type DaemonPersonLabels,
} from '../identity';
import { agentCopy } from '../pane/copy';
import { ModelTierMarks } from '../pane/CrewModelPicker';
import {
  knownInstitutions,
  modelDisplay,
  modelMismatch,
  modelRefusalText,
  protectedRunContext,
  publicModelRefusal,
  runInstitution,
  workspaceInstitutionLabel,
} from '../pane/presentation';
import { useConfiguredModels } from '../pane/useConfiguredModels';
import { useCrew, useCrewErrorSlot, useCrewSurfaceReset } from '../state/CrewControllerContext';
import { accessStatusOf, accessStatusTone, channelLabels, chatTitleOf } from './accessRows';
import { rememberChannelLabels } from './chatCrewAccess';
import { accessCopy } from './copy';
import {
  InlineConfirm,
  RevocationConfirmedNote,
  RevokeResultNote,
  useConfirmedAfterWait,
} from './RevokeControls';
import { useChatModel } from './useChatModel';
import {
  announceGrantsChanged,
  revocationUnconfirmed,
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
 *   post, as whom — with Advanced "Also read", and the pinned **Allow**, whose name never changes
 *   while it has focus (UXN-10).
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
    lastVerified,
  } = controller;
  // While the workspace is offline there is no verified view, but Revoke is still offered here
  // (F2): the last one verified this app session names the channel and the person, as the rest of
  // Crew does while it waits.
  const view = snapshot ?? lastVerified?.snapshot ?? null;
  const viewLabels = snapshot ? labels : (lastVerified?.labels ?? null);
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
  const dir = usePeopleDirectory(view, viewLabels as DaemonPersonLabels | null);
  const destinations = useMemo(() => channelLabels(view), [view]);
  const workspaces = useMemo(() => connectionNames(connections), [connections]);

  const [granted, setGranted] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [revoking, setRevoking] = useState(false);
  const [outcome, setOutcome] = useState<RevokeOutcome | null>(null);
  const revokeButton = useRef<HTMLButtonElement>(null);
  const allowButton = useFocusAllowOnConsent();
  // Allow and Revoke take their own button away with them: focus goes to the first action of the
  // view that replaced it (`data-crew-access-next`), rather than falling to the page (UXN-7).
  const paneRoot = useRef<HTMLDivElement>(null);
  const [focusNext, setFocusNext] = useState(0);
  useEffect(() => {
    if (focusNext === 0) return;
    paneRoot.current?.querySelector<HTMLElement>('[data-crew-access-next]')?.focus();
  }, [focusNext]);
  const cachedTitle = useKnownChatTitle(sessionId);
  // The chat's own model, which the grant binds (AG-F1, SF-F5): named in the consent, and checked
  // before Allow as Ask my agent checks its model before Start. Outside the app's configuration (a
  // test harness) nothing is known about it, and the pane says nothing about it.
  const models = useConfiguredModels({ optional: true });
  const chatModel = useChatModel(sessionId);
  const blockId = useId();

  // A new pane intent (Review, Manage, Grant again, a row) or another chat starts fresh.
  useEffect(() => {
    setGranted(false);
    setConfirming(false);
    setOutcome(null);
  }, [intent, sessionId]);

  const listedGrant: CrewSessionGrant | null = sessionId
    ? (grants.grants.find((item) => item.session_id === sessionId) ?? null)
    : null;
  // The daemon confirmed with the workspace by itself a revoke this pane saw answered 503, or the
  // workspace itself ended the run meanwhile: either way nothing there honours it now (F3).
  const listedConfirmed =
    listedGrant?.revocation === 'confirmed' || listedGrant?.revocation === 'ended_by_workspace';
  useEffect(() => {
    if (listedConfirmed)
      setOutcome((current) => (current?.kind === 'unconfirmed' ? null : current));
  }, [listedConfirmed]);
  const listedWaiting =
    outcome?.kind === 'unconfirmed' ||
    (listedGrant !== null &&
      sessionGrantState(listedGrant) !== 'active' &&
      revocationUnconfirmed(listedGrant));
  // Remembered for the pane's intent, not this mount (NEW-4): the body mounts again when Crew
  // reconnects and the offline screen gives way to the channel, and "Confirmed." stays until the
  // person dismisses it or leaves the pane.
  const confirmedAfterWait = useConfirmedAfterWait(
    listedGrant?.run_id ?? null,
    listedWaiting,
    listedConfirmed,
    intent
  );

  if (!sessionId) {
    return (
      <div ref={paneRoot} className={className} data-testid="crew-chat-access-pane">
        <p className="text-supporting text-text-muted">{accessCopy.paneNoChat}</p>
      </div>
    );
  }

  const grant = listedGrant;
  const state = grant ? sessionGrantState(grant) : null;
  // A revoke that stopped only on this device: the grant is no longer usable here, whatever the
  // list said a moment ago, and it stays that way until the workspace confirms or it is granted
  // again. The daemon asks the workspace again by itself once the connection is back (F3).
  const stoppedHere = listedWaiting;
  const confirmation =
    connections.find((item) => item.id === (grant?.connection_id ?? connectionId))?.status ===
    'connected'
      ? 'confirming'
      : 'offline';
  const active = !stoppedHere && (granted || state === 'active');
  // The daemon's name for the chat when a grant lists one; else the title this window knows.
  const chat = (grant ? chatTitleOf(grant) : null) ?? cachedTitle;
  const canGrant = sessionId === grantSessionId;
  // The grant's own connection when it is listed; this one when it was just granted here.
  const target = grant?.connection_id ?? (granted ? connectionId : null);
  const sameConnection = !grant || grant.connection_id === connectionId;

  // The name the person saw when granting, when no view names the channel now.
  const grantedAs = grant ? sanitizeDisplayText(grantDestinationLabel(grant)) : '';
  const destinationOf = (id: string) =>
    sameConnection
      ? (destinations.get(id) ??
        (grant && id === grant.channel_id && grantedAs ? grantedAs : accessCopy.unknownChannel))
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

  // The workspace and the chat's model, as the consent names them and the checks read them. The
  // workspace as Ask my agent names it: its own name, else the saved connection's.
  const signedName = sanitizeDisplayText(view?.workspace.name);
  const workspace =
    (signedName && !isMachineIdShaped(signedName) ? signedName : '') ||
    (connectionId && workspaces.get(connectionId)) ||
    identityCopy.unnamedWorkspace;
  const chatProvider = chatModel
    ? models.providers?.find((item) => item.name === chatModel.provider)
    : undefined;
  const shownModel = chatModel ? modelDisplay(chatModel, chatProvider) : null;
  const known = knownInstitutions(models.providers);
  const checkContext = {
    connection: controller.connection,
    snapshot: view,
    channel,
    contextChannels: consentExtras,
  };
  const institution = runInstitution({ ...checkContext, known });
  const mismatch = modelMismatch(chatProvider, institution);
  // What the daemon would refuse this chat's model for, said before Allow, which it disables
  // (SF-F5): the words Ask my agent says before Start.
  const mismatchText =
    mismatch && institution && shownModel
      ? mismatch.affiliation
        ? agentCopy.institutionMismatch(
            shownModel.model,
            mismatch.affiliation,
            workspace,
            institution.label
          )
        : agentCopy.institutionUnstated(shownModel.model, workspace, institution.label)
      : null;
  const publicText = publicModelRefusal({
    ...checkContext,
    provider: chatProvider,
    workspace,
    channelLabel: (item) => destinations.get(item.id) ?? channelName(item),
  });
  const blockText = mismatchText ?? publicText;

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
    setFocusNext((count) => count + 1);
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
    setFocusNext((count) => count + 1);
    rememberChannelLabels(connectionId, destinations, [channelId]);
    announceGrantsChanged({ connectionId, sessionId, change: 'granted' });
  };

  // A model refusal in the words Ask my agent uses (SF-F4, SF-F5); any other as the daemon words it.
  const errorNote =
    showError && controller.error ? (
      <Note tone="danger" icon={AlertCircle} role="alert" testId="crew-chat-access-error">
        {modelRefusalText({
          error: controller.error,
          mismatch: mismatchText,
          publicText,
          model: shownModel?.model ?? agentCopy.model,
          institution:
            institution?.label ?? workspaceInstitutionLabel(controller.connection, view, known),
        }) ?? controller.error.message}
      </Note>
    ) : null;

  // ── Loading and failure, before anything is known ────────────────────────────────────────
  if (!granted && !grant && (connectionsState !== 'loaded' || grants.status === 'loading')) {
    return (
      <div ref={paneRoot} className={className} data-testid="crew-chat-access-pane">
        <p role="status" className="text-supporting text-text-muted">
          {accessCopy.noteChecking}
        </p>
      </div>
    );
  }
  if (!granted && !grant && grants.anyFailed) {
    return (
      <div ref={paneRoot} className={className} data-testid="crew-chat-access-pane">
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
      <div ref={paneRoot} className={className} data-testid="crew-chat-access-pane">
        <RevokeResultNote
          outcome={outcome}
          chat={chat}
          onRetry={() => void revoke()}
          successActions={
            <>
              <Button
                type="button"
                variant="secondary"
                size="sm"
                onClick={openChat}
                data-crew-access-next=""
              >
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

  // Whose agent the chat posts as, at the authority point (AG-F1): its posts appear as "Dave
  // Patel's agent", never as Dave. Only for this connection's grants.
  const me = dir.me ? <PersonName person={dir.me} context="authority" dir={dir} agent you /> : null;
  // The connection's remote work folder, which the daemon opens to a chat whose model is not
  // public, commands included when the connection allows them (HPC-N2). Said unless the model is
  // known to be public: a consent that says too much is safer than one that says too little.
  const folder =
    sameConnection && controller.connection?.remote_root && chatProvider?.resolved_tier !== 'public'
      ? {
          path: controller.connection.remote_root,
          run: controller.connection.remote_execution === true,
        }
      : null;
  const folderLine = (future: boolean) =>
    folder
      ? future
        ? folder.run
          ? accessCopy.folderRun(folder.path)
          : accessCopy.folderFiles(folder.path)
        : folder.run
          ? accessCopy.foldersRun(folder.path)
          : accessCopy.foldersFiles(folder.path)
      : null;
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
        {who ? (
          <>
            {future ? accessCopy.postAs(where) : accessCopy.postsAs(where)} {who}
          </>
        ) : future ? (
          accessCopy.post(where)
        ) : (
          accessCopy.posts(where)
        )}
      </li>
      {folder ? (
        <li data-crew-access-folder="">
          <bdi translate="no">{folderLine(future)}</bdi>
        </li>
      ) : null}
    </ul>
  );

  // ── Stopped on this device, waiting for the workspace to confirm ─────────────────────────
  if (stoppedHere) {
    return (
      <div ref={paneRoot} className={className} data-testid="crew-chat-access-pane">
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
            confirmation={confirmation}
          />
          <div>
            <Button
              type="button"
              variant="secondary"
              size="sm"
              onClick={openChat}
              data-crew-access-next=""
            >
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
      <div ref={paneRoot} className={className} data-testid="crew-chat-access-pane">
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
                <Button type="button" size="sm" onClick={openChat} data-crew-access-next="">
                  {accessCopy.backToChat}
                </Button>
              ) : (
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  onClick={openChat}
                  data-crew-access-next=""
                >
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
      <div ref={paneRoot} className={className} data-testid="crew-chat-access-pane">
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
      ? grant.revocation === 'ended_by_workspace'
        ? accessCopy.paneSettingsChanged(chat)
        : accessCopy.paneExpired(chat)
      : grant
        ? accessCopy.paneRevoked(chat)
        : null;

  const confirmedNote = confirmedAfterWait.shown ? (
    <RevocationConfirmedNote onDismiss={confirmedAfterWait.dismiss} />
  ) : null;

  if (!canGrant) {
    return (
      <div ref={paneRoot} className={className} data-testid="crew-chat-access-pane">
        <div className="flex flex-col gap-3">
          {confirmedNote}
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
    <div ref={paneRoot} className={className} data-testid="crew-chat-access-pane">
      <form className="flex flex-col gap-3" onSubmit={(event) => void allow(event)}>
        {confirmedNote}
        {lapsed ? <p className="text-label text-text-default">{lapsed}</p> : null}
        <p className="text-label text-text-default">{accessCopy.willBeAble(chat)}</p>
        {summaryLines(true, here, consentExtras, me)}
        {/* Where and with what (AG-F1): the workspace, and the chat's model with its tier, as
            Ask my agent names a model. */}
        <dl className="crew-consent-facts text-secondary">
          <dt className="text-text-muted">{accessCopy.consentWorkspace}</dt>
          <dd className="min-w-0 break-words text-text-default">{workspace}</dd>
          {shownModel ? (
            <>
              <dt className="text-text-muted">{accessCopy.consentModel}</dt>
              <dd className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-text-default">
                <span className="min-w-0 break-words">
                  {agentCopy.modelChoice(shownModel.model, shownModel.provider)}
                </span>
                <ModelTierMarks
                  provider={chatProvider}
                  privateOnly={protectedRunContext(checkContext)}
                />
              </dd>
            </>
          ) : null}
        </dl>
        <p className="text-supporting text-text-muted">{accessCopy.fixedOnFirstAccess}</p>
        <p className="text-supporting text-text-muted">{accessCopy.expiry}</p>
        <AlsoRead
          contextChannels={contextChannels}
          setContextChannels={setContextChannels}
          currentChannelId={channelId}
          currentChannelName={here}
        />
        {/* Before Allow, which it disables: what the daemon would refuse this chat's model for. */}
        {blockText ? (
          <Note tone="warning" icon={AlertTriangle} testId="crew-chat-access-model-refused">
            <span id={blockId}>{blockText}</span>
          </Note>
        ) : null}
        {errorNote}
        <div className="flex justify-end">
          {/* One word that stays put (UXN-10): naming the chat here wrapped it to two lines and
              changed its name under keyboard focus when the chat's title arrived. The sentence
              above, `willBeAble`, names the chat. */}
          <Button
            key="crew-chat-access-allow"
            ref={allowButton}
            onKeyDown={onAllowKeyDown}
            type="submit"
            disabled={controller.isPending('grant') || blockText !== null}
            aria-describedby={blockText ? blockId : undefined}
          >
            {accessCopy.allow}
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
  // The team only where two teams have a channel by that name, as Ask my agent names the same list
  // (AG-F17): "Patel Group / #general" on every row said the team where it told nothing apart.
  const labels = useMemo(
    () => channelNamesAcrossTeams(snapshot?.channels ?? [], snapshot?.teams ?? []),
    [snapshot]
  );
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
            <span className="min-w-0 truncate">{labels.get(item.id) ?? channelName(item)}</span>
          </label>
        ))}
      </fieldset>
    </Disclosure>
  );
}
