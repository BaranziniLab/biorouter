import { retainTabGreetings } from '../components/common/Greeting';
import {
  createContext,
  useContext,
  useReducer,
  useMemo,
  useEffect,
  useRef,
  useState,
  useCallback,
} from 'react';
import { useLocation } from 'react-router-dom';
import {
  chatGroupsReducer,
  ChatGroupsAction,
  activeSessionIdOf,
  activeGroupOf,
  activeTabOf,
  findTabBySession,
} from '../components/chatGroups/chatGroupsReducer';
import {
  loadChatGroupsOrInitial,
  saveChatGroups,
  getWindowId,
} from '../components/chatGroups/chatGroupsStorage';
import {
  useChatGroupsUrlSync,
  UrlOpenRequest,
} from '../components/chatGroups/useChatGroupsUrlSync';
import { registerCloseActiveTab } from '../components/chatGroups/closeActiveTabRegistry';
import {
  registerNewTab,
  consumePendingNewTab,
  hasPendingNewTab,
  acknowledgeNewTabCommit,
} from '../components/chatGroups/newTabRegistry';
import {
  isTabCycleEvent,
  tabCycleOffset,
  nextTabIndex,
  isWithinArtifactPanel,
} from '../utils/tabCycle';
import {
  ChatGroupsState,
  ChatGroup,
  ChatTab,
  leafGroupIds,
} from '../components/chatGroups/chatGroupsTypes';
import { useRunningChats, defaultChatStreamRegistry } from '../hooks/chatStreamStore';
import { subscribeSessionNameChanges } from '../utils/sessionNameSync';
import { runPanelCommand } from '../components/chatGroups/panelCommands';
import {
  registerWorkspaceCommands,
  drainPendingWorkspaceCommands,
  type WorkspaceCommand,
  type WorkspaceCommandResult,
} from '../components/chatGroups/workspaceCommandRegistry';
import {
  planWorkspaceCommand,
  type TabAnnotation,
} from '../components/chatGroups/workspaceCommandPlanner';
import { useWorkspaceChannel, buildEchoFrame } from '../hooks/useWorkspaceChannel';
import {
  composerDraftKeyForTab,
  holdsUnsentMessage,
  retainTabComposerDrafts,
  retainExistingChatComposerDrafts,
  unsentComposerTabs,
} from '../utils/composerDrafts';
import { retainComposerQueues } from '../utils/composerQueues';
import { toastError, toastInfo, toastWarning } from '../toasts';

/**
 * BR-71 §5: a workspace `notify` carries a level, and it has to survive the trip
 * to the screen. The daemon stamps one on every frame (`workspace_extension.rs`
 * sends "info" for the cross-session actions autonomous mode must not perform
 * silently), so collapsing them onto one channel is not cosmetic: a failure
 * rendered as a green success check is a lie the user acts on, and an
 * informational notice about ANOTHER agent's action, dressed as a confirmation,
 * reads as "your thing worked".
 *
 * `toastService` has only success/error/loading, which is why this reaches for
 * the module-level helpers instead — the same ones MCPUIResourceRenderer uses.
 * Nothing in the app configures `toastService`'s silent/shouldThrow flags, so no
 * policy is being bypassed.
 */
function notifyToast(level: string | undefined): (props: { title: string; msg: string }) => void {
  switch (level?.toLowerCase()) {
    case 'error':
    case 'fatal':
    case 'critical':
      return toastError;
    case 'warn':
    case 'warning':
      return toastWarning;
    default:
      // Including no level at all: a notice, never a confirmation.
      return toastInfo;
  }
}

/**
 * BR-71: a daemon-opened tab is a session this renderer is not driving, so it
 * attaches the observer stream rather than owning a /reply stream.
 * `observeSession` lands on ChatStreamController in Task 27; the optional shape
 * is what lets this executor compile and run against either version.
 */
type ObservableStream = { observeSession?: () => void };

interface ChatGroupsContextValue {
  state: ChatGroupsState;
  dispatch: (action: ChatGroupsAction) => void;
  activeGroup: ChatGroup | undefined;
  activeTab: ChatTab | undefined;
  activeSessionId: string;
  runningSessionIds: readonly string[];
  /** sessionId → badge / parent link pushed by a workspace `annotate_tab`. */
  tabAnnotations: Record<string, TabAnnotation>;
}

// Route changes unmount this provider; only a new renderer ends these lifetimes.
const liveTabsByWindow = new Map<string, Set<string>>();

export function resetLiveChatTabsForTests() {
  liveTabsByWindow.clear();
}

const ChatGroupsContext = createContext<ChatGroupsContextValue | null>(null);

/** Returns null outside a provider (the ChatContext pattern) — never throws. */
export function useChatGroups(): ChatGroupsContextValue | null {
  return useContext(ChatGroupsContext);
}

export function ChatGroupsProvider({ children }: { children: React.ReactNode }) {
  const windowIdRef = useRef<string>('');
  if (!windowIdRef.current) windowIdRef.current = getWindowId();

  // The navigation this provider mounts on. Read for its first state only.
  const mountLocation = useLocation();
  const [state, dispatch] = useReducer(chatGroupsReducer, windowIdRef.current, (windowId) => {
    const loaded = loadChatGroupsOrInitial(windowId, {
      keepSessionlessTab: (tabId) =>
        liveTabsByWindow.get(windowId)?.has(tabId) === true ||
        holdsUnsentMessage(composerDraftKeyForTab(tabId)),
    });
    // An ARRIVAL (sidebar New chat, or a Cmd+T remembered while another route
    // was showing) lands in the FIRST state, so every pane mounts already
    // knowing which one is focused and only that pane's composer takes the
    // caret. Resolved after mount, the arrival raced the panes' own mount
    // focus, which moves the focus to whichever pane focused last: measured in
    // the dev app, the draft in the left pane was resumed and the right pane's
    // chat was left with the caret and the focus.
    //
    // The two dispatches that cash these requests in still run
    // (`useNewChatTabRequests`, `consumePendingNewTab` below). Resuming is
    // idempotent — once resumed, the tab with no chat is the one on screen in
    // the focused pane, and resuming again returns it — so they change nothing,
    // and they still consume the Cmd+T. Pure reads only: StrictMode calls this
    // initializer twice.
    const arriving =
      (mountLocation.state as { newChat?: unknown } | null)?.newChat === true || hasPendingNewTab();
    return arriving
      ? chatGroupsReducer(loaded, {
          type: 'openTab',
          payload: { sessionId: '', resumeUnsent: true },
        })
      : loaded;
  });

  const running = useRunningChats();
  // `completedAt` must be filtered out, exactly as AppSidebar:139 does: the
  // registry keeps finished entries in the snapshot for ~1.6s so the sidebar can
  // play its completion flourish. Without this filter a chat that just finished
  // would keep pulsing in the strip, and the strip would disagree with the
  // sidebar about which chats are live.
  const runningSessionIds = useMemo(
    () => running.filter((entry) => !entry.completedAt).map((entry) => entry.sessionId),
    [running]
  );

  // Persist. The transient route cargo is stripped inside saveChatGroups, not
  // here, so no caller can forget.
  useEffect(() => {
    liveTabsByWindow.set(
      windowIdRef.current,
      new Set(Object.values(state.groups).flatMap((group) => group.tabs.map((tab) => tab.tabId)))
    );
    saveChatGroups(state, windowIdRef.current);
  }, [state]);

  // Run after unmount cleanups: closing composers save before owner cleanup
  // removes their drafts and staged images.
  useEffect(() => {
    const tabs = leafGroupIds(state.layout).flatMap((id) => state.groups[id].tabs);
    retainTabComposerDrafts(tabs.filter((tab) => !tab.sessionId).map((tab) => tab.tabId));
    retainExistingChatComposerDrafts(
      tabs.flatMap((tab) => (tab.sessionId ? [{ tabId: tab.tabId, sessionId: tab.sessionId }] : []))
    );
  }, [state]);

  // A chat's queued messages live as long as the chat is open in a tab here.
  // Closing the tab drops them (and their temp images), so opening the chat
  // again later can never send a message queued behind a turn long finished.
  // Same timing as the drafts above: after the commit's unmount cleanups, so a
  // composer closing with its tab has already parked what it held.
  // See `utils/composerQueues.ts`.
  useEffect(() => {
    retainComposerQueues(
      leafGroupIds(state.layout).flatMap((id) =>
        state.groups[id].tabs.map((tab) => tab.sessionId).filter(Boolean)
      )
    );
  }, [state]);

  // A session rename anywhere mirrors into the strip.
  useEffect(
    () =>
      subscribeSessionNameChanges((change) => {
        dispatch({
          type: 'renameTab',
          sessionId: change.sessionId,
          title: change.name,
          userSetName: change.userSetName,
        });
      }),
    []
  );

  const handleUrlOpen = useCallback((request: UrlOpenRequest) => {
    dispatch({
      type: 'openTab',
      payload: {
        sessionId: request.sessionId,
        // Born with the right name when the opener knew it (a Recents click),
        // rather than showing the placeholder until BaseChat finishes loading.
        // Undefined for a deep link / fresh chat — the reducer falls back to
        // DEFAULT_TAB_TITLE and the late rename still corrects it.
        title: request.title,
        userSetName: request.userSetName,
        pendingInitialMessage: request.initialMessage,
        pendingInitialAttachments: request.initialAttachments,
        // Where a started chat may go: its own tab, and never a tab holding a
        // message the person has not sent. Sampled here, at the dispatch,
        // because the reducer is pure.
        originTabId: request.originTabId,
        unsentTabs: unsentComposerTabs(),
      },
    });
  }, []);

  // Cmd+W / Ctrl+W. The keystroke arrives as an IPC message from the File menu's
  // "Close Tab" item, NOT as a renderer keydown — see main.ts. That is deliberate
  // and it is the only safe arrangement: an Electron menu accelerator is consumed
  // by the menu before the web contents ever sees the key, so a renderer keydown
  // listener could not have won the race against the `role: 'close'` item that
  // used to hold Cmd+W. The window would have closed regardless of what we did
  // here, taking every other tab with it.
  //
  // Returning `true` claims the keystroke. Returning false (no tabs left to
  // close) lets App.tsx fall through to closing the window, which is what a
  // macOS user expects from Cmd+W on a tabless window.
  const activeTabId = activeGroupOf(state)?.activeTabId ?? null;
  const activeTabIdRef = useRef(activeTabId);
  activeTabIdRef.current = activeTabId;

  useEffect(
    () =>
      registerCloseActiveTab(() => {
        const tabId = activeTabIdRef.current;
        if (!tabId) return false;
        dispatch({ type: 'closeTab', tabId });
        return true;
      }),
    []
  );

  // Cmd+T / Ctrl+T. Arrives as IPC from the Go menu's "New Chat" item, for the
  // same reason Cmd+W does — the menu owns the accelerator and the renderer
  // never sees the keydown.
  //
  // A new tab is a tab with no session: sessionId '' is what BaseChat renders
  // as its centered empty composer. The reducer's dedupe is gated on a truthy
  // sessionId, so an empty tab never collapses onto an existing one — Cmd+T
  // twice gives two blank tabs, exactly as a browser does. `title` is left
  // undefined so DEFAULT_TAB_TITLE applies; a Cmd+T tab has no name yet and
  // inventing a placeholder here would put the naming rule in two places.
  const openNewTab = useCallback(() => {
    dispatch({ type: 'openTab', payload: { sessionId: '' } });
  }, []);

  useEffect(() => registerNewTab(openNewTab), [openNewTab]);

  // A Cmd+T that arrived while we were on another route (Settings, the Hub) was
  // remembered rather than dropped; App.tsx navigated here for us and this is
  // where the request is cashed in. consumePendingNewTab is consume-once, so
  // StrictMode's double mount opens one tab, not two.
  //
  // It is an ARRIVAL, so a tab already holding an unsent new chat is focused
  // rather than joined by a blank one (`OpenTabPayload.resumeUnsent`).
  useEffect(() => {
    if (consumePendingNewTab()) {
      dispatch({ type: 'openTab', payload: { sessionId: '', resumeUnsent: true } });
    }
  }, []);

  // A Cmd+T dispatched to THIS mounted provider stays observable in the
  // registry (hasPendingNewTab) until the tab it asked for has actually
  // COMMITTED — otherwise the empty-pair redirect's stale zero-tab effect
  // (issue #38) could bounce the window Home in the gap between the dispatch
  // and the commit, discarding the tab with the unmounting provider. Keyed on
  // `state` so it runs on every commit; child effects (the redirect) run
  // first, then this acknowledgement retires the request once tabs exist.
  useEffect(() => {
    acknowledgeNewTabCommit(
      leafGroupIds(state.layout).reduce((count, id) => count + state.groups[id].tabs.length, 0)
    );
  }, [state]);

  // Ctrl+Tab / Ctrl+Shift+Tab — cycle the focused group's strip, left to right,
  // wrapping. Unlike Cmd+T and Cmd+W this really is a DOM keydown: a dump of
  // the built application menu shows nothing claiming Ctrl+Tab, so the key
  // reaches the renderer and a listener is the honest mechanism.
  //
  // The preview panel cycles its OWN stack on the same key, so focus arbitrates
  // — see isWithinArtifactPanel, which both sides consult so their answers
  // cannot disagree. We return early on the panel's keystrokes unconditionally,
  // rather than only when we have nothing to cycle: both listeners are on
  // window in the capture phase, and "whoever runs first wins" would make the
  // behaviour depend on mount order.
  const stateRef = useRef(state);
  stateRef.current = state;

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!isTabCycleEvent(event)) return;
      if (isWithinArtifactPanel(event.target)) return;

      const current = stateRef.current;
      const group = activeGroupOf(current);
      if (!group) return;

      const activeIndex = group.tabs.findIndex((t) => t.tabId === group.activeTabId);
      const next = nextTabIndex(group.tabs.length, activeIndex, tabCycleOffset(event));
      // Null for 0 or 1 tabs. Do NOT preventDefault in that case — with nothing
      // to cycle, swallowing the key would only rob Tab of its focus behaviour.
      if (next === null) return;

      event.preventDefault();
      event.stopPropagation();
      dispatch({ type: 'activateTab', tabId: group.tabs[next].tabId });
    };

    window.addEventListener('keydown', onKeyDown, true);
    return () => window.removeEventListener('keydown', onKeyDown, true);
  }, []);

  // BR-71 §4.3: the daemon's workspace commands, executed here.
  //
  // Every DECISION (split refusal at MAX_GROUPS, focus etiquette, which side
  // effect a frame becomes) lives in the pure planner, which is unit-tested
  // against real reducer state. This is only the executor: dispatch the plan's
  // actions and perform its declared effects.
  const [tabAnnotations, setTabAnnotations] = useState<Record<string, TabAnnotation>>({});

  // A closed tab's annotation is forgotten, and its observer stream detached.
  // The map is keyed by session id and written straight from daemon frames, so
  // with no prune it only ever grows, for the life of the window, on input this
  // renderer does not control.
  //
  // The prune is deliberately scoped to sessions that HAD a tab and no longer
  // do, rather than to "every session without a tab right now": nothing orders
  // the daemon's frames, so an annotation can legitimately arrive before the tab
  // it describes, and the broader rule would delete it on the very next commit.
  const tabbedSessionsRef = useRef<ReadonlySet<string>>(new Set<string>());
  useEffect(() => {
    const tabbed = new Set(
      leafGroupIds(state.layout).flatMap((id) => state.groups[id].tabs.map((tab) => tab.sessionId))
    );
    const closed = [...tabbedSessionsRef.current].filter((id) => !tabbed.has(id));
    tabbedSessionsRef.current = tabbed;
    if (closed.length === 0) return;
    // The other end of the `observeSession()` below (§4.3: "until the tab
    // detaches or the user takes the session over"). That loop reconnects until
    // something stops it, and `getController` retains its controller for the
    // life of the renderer, so with no detach every daemon-opened tab leaves an
    // SSE subscription reconnecting forever for a chat nobody can see.
    //
    // `peekController`, so teardown never CREATES a controller; and
    // `releaseOwnership` detaches observers and abandons an unconsumed
    // Stop-and-Send lease, but does not cancel a daemon turn already running.
    for (const id of closed) defaultChatStreamRegistry.peekController(id)?.releaseOwnership();
    setTabAnnotations((prev) => {
      // Same object back when there is nothing to drop, so this never costs a
      // render on the ordinary tab close.
      if (!closed.some((id) => id in prev)) return prev;
      const next = { ...prev };
      for (const id of closed) delete next[id];
      return next;
    });
  }, [state]);

  useEffect(() => {
    const runPlan = (
      cmd: WorkspaceCommand
    ): WorkspaceCommandResult | Promise<WorkspaceCommandResult> => {
      // Panel reads are answered by the panel itself, not by the tab planner —
      // they change no layout, and the accessor is registered per session by
      // the chat that owns the panel.
      if (cmd.cmd === 'read_panel' || cmd.cmd === 'capture_panel') {
        return runPanelCommand(cmd);
      }
      const plan = planWorkspaceCommand(cmd, stateRef.current);
      // The plan is the WHOLE answer — nothing here is deferred to a later tick.
      // A split used to be finished off in a `queueMicrotask` that re-read
      // `stateRef`, on the theory that a new session's tab id does not exist
      // until `openTab` commits. It does not work: a frame arrives from
      // `ws.onmessage` on a macrotask, React commits on the Scheduler's own
      // macrotask, and the microtask therefore drains BEFORE the commit — so the
      // ref was pre-open, the lookup missed, the move was dropped, and the daemon
      // was told the window had split when it had not. The planner runs the
      // reducer itself to learn the id, and the move rides in this same batch.
      for (const action of plan.actions) dispatch(action);
      if (plan.openWindowSessionId) {
        // create-chat-window IPC: the session id goes in the resume-session
        // position (4th parameter — see preload.ts createChatWindow).
        window.electron?.createChatWindow?.(
          undefined,
          undefined,
          undefined,
          plan.openWindowSessionId
        );
      }
      if (plan.notify) {
        notifyToast(plan.notify.level)({ title: 'Workspace', msg: plan.notify.message });
      }
      if (plan.annotate) {
        const { sessionId, annotation } = plan.annotate;
        setTabAnnotations((prev) => ({ ...prev, [sessionId]: annotation }));
      }
      // Daemon-opened tabs are, by definition, sessions this renderer is not
      // driving: attach the observer stream (§4.3; Task 27) so the tab renders
      // live without owning a /reply stream.
      //
      // Only when the frame really produced a tab, though. `getController` is
      // not a lookup — it CREATES a ChatStreamController and retains it in a map
      // keyed by session id for the life of the renderer. Calling it for a
      // session that has no tab (an annotate for a chat this window never
      // opened, or an open_tab the planner refused at MAX_GROUPS) both starts a
      // stream for a chat that is nowhere on screen and leaks the controller,
      // once per frame, on input the daemon fully controls.
      if (cmd.session_id && plan.result.ok) {
        const opened = cmd.cmd === 'open_tab';
        const annotated =
          cmd.cmd === 'annotate_tab' && !!findTabBySession(stateRef.current, cmd.session_id);
        // BR-71 §3c. A tab the USER opened has no observer — nothing in this
        // renderer attaches one, because the tab is normally driven by its own
        // `/reply` stream and an idle tab has nothing to listen to. That stops
        // being true the moment another conversation can write into this one:
        // `workspace_send_prompt` makes the session change while this window is
        // looking straight at it, and with no observer the transcript sits
        // stale until a reload.
        //
        // The daemon sends this frame after the row is durable, and the
        // observer's FIRST frame is a full `UpdateConversation` snapshot from
        // the store — so the injected message renders whether or not the bus
        // publish beat the attach. That is deliberate: an ordering guarantee
        // between a broadcast and a socket handshake is not one worth relying on.
        const observed =
          cmd.cmd === 'observe' && !!findTabBySession(stateRef.current, cmd.session_id);
        if (opened || annotated || observed) {
          const stream = defaultChatStreamRegistry.getController(
            cmd.session_id
          ) as unknown as ObservableStream;
          stream.observeSession?.();
        }
      }
      return plan.result;
    };
    const dispose = registerWorkspaceCommands(runPlan);
    // Drain frames that arrived before this provider mounted (Settings-page
    // case — same rationale as consumePendingNewTab).
    for (const queued of drainPendingWorkspaceCommands()) runPlan(queued);
    return dispose;
  }, [dispatch]);

  const activeSessionId = activeSessionIdOf(state);
  useChatGroupsUrlSync({ activeSessionId, onOpen: handleUrlOpen });

  // The other half of §4.3: this window's layout, reported to the daemon so the
  // workspace tools can see what the user is actually looking at.
  const [workspaceSecret, setWorkspaceSecret] = useState<string | null>(null);
  useEffect(() => {
    // The same capability probe renderer.tsx uses to detect the headless case.
    if (typeof window.electron === 'undefined') return;
    if (typeof window.electron.getSecretKey !== 'function') return;
    let cancelled = false;
    void Promise.resolve(window.electron.getSecretKey())
      .then((key) => {
        if (!cancelled) setWorkspaceSecret(key || null);
      })
      .catch(() => {
        // No secret, no channel. The window keeps working without it.
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // FALSE until the secret resolves — `getSecretKey` is async, so `secret` is
  // null on the first render and this flag is the only thing standing between
  // every provider-mounting test suite and a live WebSocket in jsdom.
  const workspaceChannelEnabled =
    !!workspaceSecret &&
    typeof window.electron !== 'undefined' &&
    typeof window.electron.getSecretKey === 'function';

  const { sendEcho } = useWorkspaceChannel({
    secret: workspaceSecret,
    windowId: windowIdRef.current,
    enabled: workspaceChannelEnabled,
  });

  /**
   * ⚠ **`focused_session` has to mean FOCUSED, or the daemon cannot tell two
   * windows apart** (issue #78).
   *
   * The daemon chooses which window receives an `open_tab` with
   * `bridge::focused_or_recent()`, whose first branch takes the first bridge
   * whose echo carries a non-null `focused_session`. This effect used to put
   * the window's ACTIVE TAB in that field, with no OS-focus input anywhere on
   * the path — so every window holding a chat reported "focused", permanently,
   * and the tie fell through to `HashMap::values()` order: arbitrary, but
   * stable for the life of the process. That is exactly the reported symptom,
   * a spawned subagent tab landing in one particular *other* window each time.
   *
   * Tracking real focus makes that branch mean what it says. It does not
   * replace threading the originating window id through the spawn path, which
   * is the robust fix and stays open on #78 — it removes the ambiguity that
   * made the guess wrong in the ordinary case.
   */
  const [windowFocused, setWindowFocused] = useState(() =>
    typeof document === 'undefined' ? true : document.hasFocus()
  );
  useEffect(() => {
    const onFocus = () => setWindowFocused(true);
    const onBlur = () => setWindowFocused(false);
    window.addEventListener('focus', onFocus);
    window.addEventListener('blur', onBlur);
    return () => {
      window.removeEventListener('focus', onFocus);
      window.removeEventListener('blur', onBlur);
    };
  }, []);

  // Keyed on `state`, so it runs on every commit — the same placement as
  // acknowledgeNewTabCommit. `activeSessionId` is '' for an empty active group.
  useEffect(() => {
    sendEcho(
      buildEchoFrame(windowIdRef.current, windowFocused ? activeSessionId || null : null, state)
    );
  }, [state, activeSessionId, sendEcho, windowFocused]);

  const value = useMemo<ChatGroupsContextValue>(
    () => ({
      state,
      dispatch,
      activeGroup: activeGroupOf(state),
      activeTab: activeTabOf(state),
      activeSessionId,
      runningSessionIds,
      tabAnnotations,
    }),
    [state, activeSessionId, runningSessionIds, tabAnnotations]
  );

  useEffect(() => {
    retainTabGreetings(
      Object.values(state.groups).flatMap((group) => group.tabs.map((tab) => tab.tabId))
    );
  }, [state.groups]);

  return <ChatGroupsContext.Provider value={value}>{children}</ChatGroupsContext.Provider>;
}
