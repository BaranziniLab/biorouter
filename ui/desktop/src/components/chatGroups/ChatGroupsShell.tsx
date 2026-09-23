import { isTabKeyboardFocusPending } from './chatTabKeyboardFocus';
import { useCallback, useMemo, useState, useEffect, useRef, Fragment, ReactElement } from 'react';
import BaseChat from '../BaseChat';
import InAppTerminalDock from '../InAppTerminalDock';
import { ChatType } from '../../types/chat';
import { useChatGroups } from '../../contexts/ChatGroupsContext';
import { useTerminalDock } from '../../contexts/TerminalDockContext';
import { ChatTabStrip } from './ChatTabStrip';
import { Plus } from '../icons/app-icons';
import { ChatGroupSplitter } from './ChatGroupSplitter';
import { ChatDropOverlay, ChatTabGhost } from './ChatDropOverlay';
import { ChatTabDragProvider } from './ChatTabDragContext';
import { useTabDragReorder } from './useTabDragReorder';
import { CrossWindowPhase, ScreenPoint, payloadFromTab } from './tabTearOff';
import {
  countWindowTabs,
  measureStripBands,
  resolveMergeInsertion,
  type MergeInsertion,
} from './tabTearOffBridge';
import { DropTarget } from './dropZones';
import { groupCountOf } from './chatGroupsLayout';
import { GroupLayoutSnapshot, snapshotGroupLayout } from './chatGroupsReducer';
import { firstLeaf, leafGroupIds, GroupLayout, ChatGroupId } from './chatGroupsTypes';
import { isDefaultSessionName } from '../../utils/sessionNameSync';
import { splitSnapshotIsStale, splitYieldAction, splitYieldSample } from '../Layout/yieldLadder';
import { useIsMobile } from '../../hooks/use-mobile';
import { useSidebar } from '../ui/sidebar';
import { SIDEBAR_COMPACT_TITLE_WIDTH } from '../Layout/TitlebarControls';
import { setFocusedChatSession } from '../../utils/extensionErrorUtils';
import {
  getCachedSessionList,
  preloadSessionList,
  subscribeSessionList,
} from '../../utils/sessionListCache';
import { useLiveSessionTiers, useLiveSessionTypes } from '../../hooks/chatStreamStore';
import { mergeSessionTiers, raiseTier } from '../privacy/sessionTier';
import { useSessionListTiers } from '../privacy/useSessionListTiers';
import { getSession, type Session, type SessionClassification, type SessionType } from '../../api';
import { userActionHeaders } from '../../utils/userAction';
import {
  forgetSessionTypesExcept,
  recallSessionTypes,
  rememberSessionType,
} from './sessionTypeMemory';

/** Rule 6 of `useTabTitlesFromSessionList`: the first wait before asking again… */
const ROW_READ_RETRY_BASE_MS = 1_500;
/** …doubling up to this… */
const ROW_READ_RETRY_MAX_MS = 10_000;
/** …this many times per list. About 30 s in all. */
const ROW_READ_MAX_RETRIES = 5;

/**
 * Did the daemon ANSWER a row read it returned no row for? Only a client error
 * does: 403 (refused) and 404 (gone) are answers, and rule 6 never asks again
 * after one. Everything else is a question nobody answered — no response at
 * all, a 5xx, a 408 or 429 that says to try later, and a status of 0, which is
 * no HTTP status at all (measured: the Electron renderer reports a CDP-fulfilled
 * 503 as 0, so a probe that assumed only `>= 500` saw no retry).
 */
function isAnswer(status: number | undefined): boolean {
  return status !== undefined && status >= 400 && status < 500 && status !== 408 && status !== 429;
}

/** Every chat a tab in this window holds. */
function heldSessionIds(
  state: { groups: Record<string, { tabs: { sessionId: string }[] }> } | undefined
) {
  const held = new Set<string>();
  for (const group of Object.values(state?.groups ?? {})) {
    for (const tab of group.tabs) if (tab.sessionId) held.add(tab.sessionId);
  }
  return held;
}

/**
 * `base`, with a type from the first of `sources` that has one for each held
 * chat `base` has none for. The same object when nothing is filled, so the
 * strip is not re-rendered by a merge that changed nothing.
 */
function fillSessionTypes(
  base: Record<string, SessionType>,
  held: ReadonlySet<string>,
  sources: readonly Readonly<Record<string, SessionType>>[]
): Record<string, SessionType> {
  let next = base;
  for (const sessionId of held) {
    if (base[sessionId]) continue;
    for (const source of sources) {
      const sessionType = source[sessionId];
      if (!sessionType) continue;
      if (next === base) next = { ...base };
      next[sessionId] = sessionType;
      break;
    }
  }
  return next;
}

function readCachedSessionTypes(): Record<string, SessionType> {
  const types: Record<string, SessionType> = {};
  for (const session of getCachedSessionList() ?? []) {
    if (session.session_type) types[session.id] = session.session_type;
  }
  return types;
}

/**
 * Session type per session id, as the shared session-list cache reports it —
 * `useSessionListTiers` for the type, and seeded and followed exactly as that
 * hook is, so a listed row's kind is on the same render as its tier.
 *
 * `useTabTitlesFromSessionList`'s reconcile also takes each listed tab's type,
 * but from an EFFECT, which commits once before its update does. The list's
 * tier does not wait for it, so a subagent's tab opened from History's subagent
 * list committed `data-chat-kind="chat" data-privacy="private"` first
 * (`ChatGroupsShell.subagentKind.test.tsx`, measured by a `Profiler` on the
 * first commit).
 */
function useSessionListTypes(): Record<string, SessionType> {
  const [types, setTypes] = useState<Record<string, SessionType>>(readCachedSessionTypes);
  useEffect(() => {
    const read = () => {
      const next = readCachedSessionTypes();
      setTypes((prev) => {
        const ids = Object.keys(prev);
        const differ =
          ids.length !== Object.keys(next).length || ids.some((id) => prev[id] !== next[id]);
        return differ ? next : prev;
      });
    };
    read();
    // The shell's other readers of the list warm it; this only follows it.
    return subscribeSessionList(read);
  }, []);
  return types;
}

/** `map` without the chats no tab holds; the same object when there are none. */
function onlyHeld<T>(map: Record<string, T>, held: ReadonlySet<string>): Record<string, T> {
  let next = map;
  for (const sessionId of Object.keys(map)) {
    if (held.has(sessionId)) continue;
    if (next === map) next = { ...map };
    delete next[sessionId];
  }
  return next;
}

interface ChatGroupsShellProps {
  /** Mirrors the focused chat up to App's hubChat, so AppSidebar's recents
   *  highlight and document.title keep working with ZERO sidebar edits. */
  onChatChange: (chat: ChatType) => void;
}

interface RenderGroupArgs {
  groupId: ChatGroupId;
}

/**
 * Renders the layout TREE.
 *
 * `path` addresses the branch being rendered, from the root: [] is the root,
 * [1, 0] is the first child of the second child. The resize action is
 * path-addressed for the same reason firstLeaf is a walk and not an index — a
 * position in a flattened list means a different node the moment the tree
 * reshapes, and a splitter that resizes the wrong branch is the kind of bug that
 * only shows up at depth 2.
 */
function renderLayout(
  layout: GroupLayout,
  path: readonly number[],
  renderGroup: (args: RenderGroupArgs) => ReactElement,
  renderBranch: (
    layout: Extract<GroupLayout, { kind: 'branch' }>,
    path: readonly number[],
    children: ReactElement[]
  ) => ReactElement
): ReactElement {
  if (layout.kind === 'leaf') return renderGroup({ groupId: layout.groupId });
  const children = layout.children.map((child, index) =>
    renderLayout(child, [...path, index], renderGroup, renderBranch)
  );
  return renderBranch(layout, path, children);
}

/**
 * Privacy tier per session id, for the tab strips (issue #56, R10).
 *
 * # Two sources, and the cache is the FALLBACK
 *
 * A tab whose chat has been opened in this window has a `ChatStreamController`
 * holding the row, and that store is the same one the header pill and the
 * composer read — `applyTurnBinding` patches the post-ratchet classification
 * onto it from the reply stream's FIRST frames. That is the live source, and it
 * is read here through `useLiveSessionTiers`.
 *
 * Only the ACTIVE tab of each pane mounts a `BaseChat`, so a tab never yet
 * visited in this window has no store at all. Those get their tier from the
 * shared session-list cache, exactly as every tab used to.
 *
 * # Why the cache alone was wrong (finding M8, measured 2026-09-10)
 *
 * A chat CREATED in this window is not in that cache and cannot be put there by
 * announcing at create time: `GET /sessions` INNER JOINs `messages`
 * (`SessionStorage::list_sessions_by_types_maybe_empty`), so a row that has
 * recorded no message is not listable. `refreshSessionBinding` patches only an
 * entry the cache already holds — deliberately, since list membership belongs
 * to the list channel. So: new chat, one turn on a private model, sqlite
 * `privacy_tier=private`, the sidebar row private (it reads a freshly-fetched
 * list), the model chip private — and the active tab's own dot still
 * `data-privacy="public"` 52.9 seconds later.
 *
 * The comment this replaces claimed the gap was closed "twice over". Both
 * mechanisms it named are real and neither reaches this map: the reply stream's
 * classification lands on the STORE, and `refreshSessionBinding`'s list patch is
 * a no-op for a session the list has never carried.
 *
 * # The merge is `max`, not "freshest wins"
 *
 * The tier is a ratchet server-side (`crates/biorouter/src/privacy/mod.rs`)
 * with one exit, the user's declassification — so a `public` is only a lower
 * bound, and a `private` holds until a declassification that BOTH sources are
 * told about (`sessionRowSync`, the change feed). {@link mergeSessionTiers}
 * folds the two with `max` and `undefined` stays absent. The invariant, which
 * `ChatGroupsShell.privacy.test.tsx` pins: this map may render private-from-
 * either-source or absent, and can never render public over a source that
 * still holds private. There is no failure mode in which it over-marks — no
 * source here invents a tier, they only report a row. `ChatTabStrip`'s
 * `privacyTiers` prop doc states the same thing, and the two must not drift
 * apart again.
 *
 * ⚠ `max` across sources is only right because each source follows its OWN
 * row down. The live map once refused to (it "mirrored the ratchet"), so a
 * declassified chat's tab stayed private for as long as it was open — defect D1
 * of 2026-09-13; see `ChatStreamRegistry.subscribeSessionTiers`.
 *
 * # The cache still has to be warmed here
 *
 * An earlier version of this comment claimed AppSidebar warmed it "at module
 * scope"; it does not. `preloadSessionList` lives inside AppSidebar's
 * `preloadHome()`, wired to `onFocus`/`onPointerEnter` on the Home nav entry,
 * so it fires only if the user points at Home. What actually warmed the cache
 * on a normal launch was the Hub index route mounting `SessionsInsights`, which
 * calls `refreshSessionList()` — an incidental side effect of an unrelated
 * screen, and absent in a window that opens straight onto a chat. So the strip
 * warms it here, through `useSessionListTiers`: `preloadSessionList()` returns
 * early when the cache is non-null and swallows its own errors, costing one
 * fetch on a cold start.
 *
 * In jsdom, where the module is mocked or the fetch fails, the cache stays null
 * and a tab with no store is simply absent from this map.
 *
 * # Absent is drawn as NOT YET KNOWN, never as Public
 *
 * This map was right about "absent" all along; the glyph was not. Until
 * 2026-09-14 `ChatKindIcon` rendered an absent tier as `data-privacy="public"`,
 * and this map is empty for a subagent's tab on every mount of this shell —
 * its only source is the per-mount read below. Measured on 1.90.4: a private
 * chat's subagent tabs read Public for ~0.5 s (3.2 s on the tester's machine)
 * after Settings → a sidebar chat, and every tab read Public at 440 ms after a
 * reload. They now draw dimmed and "privacy not yet known" until their row
 * answers (`ChatGroupsShell.tierPending.test.tsx`).
 *
 * # A third source: the rows the list leaves out
 *
 * The list is `include_subagents=false`, so a delegated subagent's tab — which
 * the daemon opens in the background, and which has no store until someone
 * clicks it — was in neither source. Measured 2026-09-13: a private parent's
 * child, `privacy_tier=private` in sqlite, drawn `data-privacy="public"` after
 * a reload. `useTabTitlesFromSessionList` reads such a tab's own row, and hands
 * its tier in here as `outsideListTiers`. It is folded with the same `max`, so
 * it can raise a tab and never lower one.
 *
 * ⚠ What re-emits through `subscribeSessionList` is narrower than it looks.
 * Any `emitChange` reaches this hook — a completed `refreshSessionList`, the
 * name-channel patch, `updateCachedSessionList` (SessionListView's rename and
 * delete), `clearSessionListCache`, `notifySessionListChanged`. That last one
 * now has three production callers rather than one: `useDiverge`, the import
 * handler in `SessionListView`, and `refreshSessionBinding` the first time a
 * chat finds itself missing from a list that has been fetched — which is how a
 * chat born in this window reaches Home recents and See-all. It is NOT how the
 * tab dot gets fixed; the live store above is, and it costs no request.
 */
function useSessionPrivacyTiers(
  outsideListTiers: Record<string, SessionClassification>
): Record<string, SessionClassification> {
  // The cache half, read and warmed by the one hook every tier-drawing surface
  // without a row of its own shares (a schedule's run list is the other).
  const cachedTiers = useSessionListTiers();
  const liveTiers = useLiveSessionTiers();

  // Memoised on the three inputs, each of which is identity-stable while
  // unchanged: the merged object is a prop on every strip in every pane, so a
  // fresh one per render would re-render all of them once per streamed token.
  return useMemo(
    () => mergeSessionTiers(cachedTiers, liveTiers, outsideListTiers),
    [cachedTiers, liveTiers, outsideListTiers]
  );
}

/**
 * Reconcile every tab's title against the session list — INCLUDING the tabs
 * that are not active.
 *
 * # What was broken
 *
 * A tab title is the only name in the app that is PERSISTED by the renderer
 * (`chatGroupsStorage`), and until now only two things could correct one: the
 * name channel (`subscribeSessionNameChanges`, mirrored into every tab by
 * `ChatGroupsContext`) and `handleSessionLoaded` below — which fires from
 * BaseChat, and only the ACTIVE tab mounts a BaseChat. So a name this window
 * never heard announced stayed wrong on a background tab across reload after
 * reload, while the sidebar — which re-reads the server on every load — showed
 * the right one. Measured on 2026-09-12: the sidebar read "Instruction-following
 * tests" while the same chat's background tab read "Penguin prompt test" — on one
 * screen, at the same moment, unchanged across three reloads, and correcting
 * instantly the moment that tab was made active.
 *
 * The daemon auto-renames a chat after EVERY one of its first few turns
 * (`SessionManager::maybe_update_name`), after the reply stream has already
 * closed, and emits no signal for it — so "a name this window never heard
 * announced" is the normal case, not an exotic one. The poll in
 * `chatStreamStore.finishTurn` announces what it catches; this closes the
 * general hole, including a rename made by the CLI, a schedule, or another
 * window while this one was shut.
 *
 * # Why the session list, and why it cannot go stale again
 *
 * This is the SAME cache `useSessionPrivacyTiers` above already reads and warms
 * — no new request, no polling. It is re-read on mount and on every emit, so a
 * tab's title is checked against the server's row every time this window loads
 * and every time that list changes. A title can therefore be wrong only for as
 * long as this cache is, and never across a load.
 *
 * # The two things it must not do
 *
 *   1. **Overwrite a name the user typed.** A user rename sets `userSetName` on
 *      the tab (optimistically, in `BaseChat.handleRename`) and `user_set_name`
 *      on the row, and the daemon never auto-renames such a session again. A
 *      user-named tab is therefore skipped outright rather than compared: the
 *      name channel already carries every user rename to every tab
 *      synchronously, so there is nothing here for this hook to add, and
 *      skipping makes the snap-back structurally impossible rather than merely
 *      unlikely. (`sessionListCache` guards the other half of that race — a list
 *      response that was issued BEFORE the rename it would undo.)
 *   2. **Downgrade a named tab to the placeholder.** A row still reading "New
 *      chat" is a lower bound, never a correction, so it is never adopted over a
 *      title that already has a real name.
 *
 * # The tabs the list leaves out
 *
 * The list is not every chat. It is usually `GET /sessions?include_subagents=false`,
 * so it usually carries no delegated subagent's chat — deliberately, because those
 * are not sidebar chats and must not become sidebar chats to fix a tab. (Usually:
 * the cache is module-global, and History's "Show subagent runs" refetches it WITH
 * them, where it stays until something asks for the other flag. See the kind,
 * below, for what that changes.) It also
 * omits a chat that has recorded no message yet (`GET /sessions` INNER JOINs
 * `messages`). Measured 2026-09-13: the daemon opened a private chat's subagent
 * in a background tab (`open_tab`, `focus: false`, no title), sqlite named it
 * `Subagent: Reply with exactly the word ECHOSUB and nothing else.`, the list
 * held 5543 rows and not that one, and the tab read "New chat" across every
 * reload — nothing that could name it ever ran, because only an active tab
 * mounts the BaseChat whose load renames it.
 *
 * So a tab whose chat is not in the list is asked about on its own, with
 * `GET /sessions/{id}?metadata_only=true`. That is the singular read every chat
 * surface uses: it answers only through `session_reach`, and it carries the
 * user's proof — without which a private parent's subagent, which is private
 * too, answers 403 (measured) and the tab would keep the placeholder silently.
 * A refused read changes nothing, and neither does an answer for a chat no tab
 * holds any more.
 *
 * Four rules keep it from being a poll or a snap-back:
 *
 *   3. **Once per list, per chat.** A tab is read again only when the list
 *      itself has changed since its last read — the moment an ordinary tab is
 *      re-checked too — never because the shell re-rendered or the tab's own
 *      title moved. It waits for a list: with none, it cannot know what is
 *      left out.
 *   4. **An answer is for the title it was asked about.** If the tab's title
 *      changed while the read was out, the tab holds the later fact (the name
 *      channel, the tab's own load), and the answer is dropped. Rules 1 and 2
 *      apply to the answer as they do to a list row.
 *   5. **Only the newest read of a chat lands.** An earlier read that answers
 *      late is dropped rather than written over a later one.
 *   6. **A read nobody answered is asked again, a few times.** No response at
 *      all (the daemon is away or restarting), a 5xx, or any other status that
 *      is not a client error re-issues the read after 1.5 s, doubling to at most
 *      10 s, up to five times per list (`isAnswer`). Rule 3 alone
 *      never did: it marks a chat as read for a list when the read is ISSUED,
 *      so a read that failed waited for the next list. Measured 2026-09-14 on
 *      d7f02191: one failed read during a route change left both subagent tabs
 *      `data-chat-kind="chat"` and `data-privacy="unknown"` for more than 20 s
 *      after the daemon answered again. A refusal (403) or a missing chat (404)
 *      IS an answer and is never asked again, and only the newest read of a
 *      chat may ask again (rule 5). That needs the status, which is why the
 *      read does not pass `throwOnError`: under it the client throws the body
 *      and drops the response.
 *
 * The same row reports the chat's privacy tier, which the tab strip had no
 * source for either; it is returned for `useSessionPrivacyTiers` to fold in
 * with `max`. That is why a tab the user named is still READ — only its name is
 * off limits. Measured before this was so: a private subagent's tab the user
 * had renamed kept its name and drew `data-privacy="public"`.
 *
 * # And its session type, for the tab's kind
 *
 * A row also says `session_type: 'sub_agent'`, and that is returned too, for
 * the strip to OR with the workspace annotation. The annotation was the strip's
 * only source for "this is a sub-agent", and it lives in `ChatGroupsProvider`'s
 * React state, which mounts inside the `/pair` route and is never persisted.
 * Measured 2026-09-14 on 1.90.4: two delegated subagent tabs read
 * `data-chat-kind="subagent"`, and after Settings → a sidebar chat, History →
 * back, or a reload, both read `data-chat-kind="chat"` for good.
 *
 * ⚠ **From EITHER source — the list row as much as the row read on its own.**
 * The first version of this took the type only from the singular read, on the
 * premise that every subagent tab is out of the list. It is not: with History's
 * "Show subagent runs" ticked the cached list holds the `sub_agent` rows, the
 * reconcile finds each subagent tab IN the list, and no singular read is ever
 * made. Measured 2026-09-14 by an independent tester, on the desktop and on
 * `biorouter serve`: History with the box ticked → back, and both subagent tabs
 * read `data-chat-kind="chat"` for 35 s; "Open in new tab" on a subagent row
 * from that list opened a tab that never got the glyph.
 *
 * Like the tier, a type is kept from ANY answer, whatever rules 4 and 5 do to
 * its name: it is a fact about the session, not about the title the read was
 * asked about, and a tab's title can move while its read is out (the name
 * channel, the tab's own load). Unlike the tier it is not raised: the latest row
 * wins, from whichever source, because a session's type does not change. The
 * answer for an id could change only if the id were reissued to a new chat, and
 * `create_session`'s high-water mark (`SESSION_ID_HIGH_WATER_DDL` in
 * `session_manager.rs`) makes ids single use. A store without that mark can
 * still reissue one: an older build sharing the file, or a database restored
 * from a backup. So an entry is forgotten once no tab holds its chat, which
 * also keeps the map to the tabs this window has open. The tier map from the
 * same reads is forgotten the same way, for the same two reasons.
 *
 * ⚠ **The type also outlives the shell.** Each type is written to
 * `sessionTypeMemory` as well, a module-scope map with the same forgetting and a
 * hard cap, and the state is seeded from it on mount. Without that, the state
 * started empty on every return to `/pair`, and a subagent's tab drew the chat
 * bubble until its row was read again. Measured 2026-09-14 on d7f02191: 641 ms
 * after Settings → a sidebar chat, 207 ms after History → back, then the Bot.
 * The tier is NOT remembered this way. A tier can go down (a declassification),
 * and "not yet known" until the row answers is the tier's deliberate state
 * (`ChatGroupsShell.tierPending.test.tsx`).
 *
 * ⚠ **A kind arrives no later than the tier it is drawn with, from every
 * source.** This block used to say a reload leaves every subagent tab dimmed
 * ("privacy not yet known") until its row answers, and then changes once. That
 * held for a BACKGROUND tab and was false for the ACTIVE one: its BaseChat
 * loads the chat into the live store, and the store published the row's tier
 * (`useLiveSessionTiers`) long before the session list landed and the row read
 * could begin. Measured 2026-09-14 on b6fab4a1 after a reload, the active
 * subagent tab read `data-chat-kind="chat" data-privacy="private"` — an
 * undimmed plain chat, marked private — then the Bot: 649 → 1634 ms and
 * 1000 → 2295 ms on the desktop, 385 → 1371 ms on `biorouter serve`
 * (reproduced: 570 → 1603 ms and 895 → 1929 ms). So a tier has three sources,
 * and each now brings the type from the same row, on the same render:
 *
 *   - **the row read** (`outsideListTiers` and this state, set together from one
 *     answer);
 *   - **the live store** (`useLiveSessionTypes`, written by the registry before
 *     the tier, from the same snapshot);
 *   - **the cached list** (`useSessionListTypes`, seeded on the first render as
 *     `useSessionListTiers` is — the reconcile below takes a listed tab's type
 *     too, but from an effect, which commits one frame late).
 *
 * The last two are folded in DURING RENDER (`fillSessionTypes`), never from an
 * effect: an effect commits the render before it, and that commit is the
 * `chat|private` frame this exists to remove. A type already in this state wins
 * over both — it is the latest row this shell was handed for a chat a tab holds,
 * and the store keeps a chat's row for the life of the renderer, so for an id
 * reissued to a new chat (above) the store is the stale one. The store's types
 * are written to `sessionTypeMemory` too, with the same forgetting and cap.
 *
 * What is left is a tab no source has answered for, which has no tier either:
 * a background subagent tab after a reload. It is dimmed "not yet known" and
 * changes once, when its row lands with both. It is not given a pending KIND of
 * its own: that could begin only once the list has landed, since before that
 * nothing says which tabs the list leaves out, so a subagent's tab would change
 * twice (bubble, pending, Bot) and a new chat's tab — also out of the list until
 * it records a message — would gain a change it does not have today. Nor is the
 * type persisted to survive a reload: that would store a fact about a session
 * beside the tab, which nothing here does.
 */
function useTabTitlesFromSessionList(groups: ReturnType<typeof useChatGroups>): {
  outsideListTiers: Record<string, SessionClassification>;
  rowSessionTypes: Record<string, SessionType>;
} {
  const dispatch = groups?.dispatch;
  // Read through a ref so the effect depends on the SIGNATURE below and not on
  // state identity — the shell re-renders on every streamed token, and this
  // effect resubscribes each time it re-runs.
  const stateRef = useRef(groups?.state);
  stateRef.current = groups?.state;

  // The tier of each chat read on its own (see above). Raised, never lowered.
  const [outsideListTiers, setOutsideListTiers] = useState<Record<string, SessionClassification>>(
    {}
  );
  // The session type each tab's chat's row reported — the list's row or the one
  // read on its own (see above) — for the tab's kind. Seeded from the memory, so
  // a tab that was a sub-agent's before the shell last unmounted still is on the
  // first paint of this mount.
  const [rowSessionTypes, setRowSessionTypes] = useState<Record<string, SessionType>>(() =>
    recallSessionTypes(heldSessionIds(groups?.state))
  );
  // Which list the reads below were issued against: bumped when the list array
  // itself is replaced, so rule 3 compares numbers rather than pinning old arrays.
  const listRef = useRef<{ rows: readonly Session[] | null; generation: number }>({
    rows: null,
    generation: 0,
  });
  // Per chat: the list generation it was last read for, and the newest read's
  // sequence number (rule 5).
  const readForGenerationRef = useRef(new Map<string, number>());
  const newestReadRef = useRef(new Map<string, number>());
  const readSeqRef = useRef(0);
  // Rule 6, per chat: the list its unanswered reads were counted against, how
  // many there have been, and the timer that will ask again.
  const retryRef = useRef(
    new Map<string, { generation: number; count: number; timer?: ReturnType<typeof setTimeout> }>()
  );
  // The latest reconcile, for a retry timer to call: the effect below re-creates
  // it whenever the tabs change, and a timer outlives that.
  const reconcileRef = useRef<(() => void) | null>(null);
  // An answer that lands after the shell is gone has nowhere to go.
  const mountedRef = useRef(false);
  useEffect(() => {
    mountedRef.current = true;
    const retries = retryRef.current;
    return () => {
      mountedRef.current = false;
      for (const retry of retries.values()) clearTimeout(retry.timer);
      retries.clear();
    };
  }, []);

  // The (session, title) pairs this hook compares, and nothing else. A tab
  // opened, closed, bound or renamed changes it; a reorder, a split or a token
  // does not.
  const tabTitleSignature = useMemo(() => {
    const parts: string[] = [];
    for (const group of Object.values(groups?.state.groups ?? {})) {
      for (const tab of group.tabs) {
        if (!tab.sessionId) continue;
        parts.push(`${tab.sessionId}\u241F${tab.title}\u241F${tab.userSetName ? 1 : 0}`);
      }
    }
    return parts.sort().join('\u241E');
  }, [groups?.state]);

  useEffect(() => {
    if (!dispatch) return;

    /** Rules 1 and 2, for a row from either source. */
    const renameFromRow = (sessionId: string, row: Pick<Session, 'name' | 'user_set_name'>) => {
      const state = stateRef.current;
      if (!state || !row.name) return false;
      for (const group of Object.values(state.groups)) {
        for (const tab of group.tabs) {
          if (tab.sessionId !== sessionId || tab.userSetName || row.name === tab.title) continue;
          // Rule 2: the placeholder is a lower bound, never a correction.
          if (isDefaultSessionName(row.name) && !isDefaultSessionName(tab.title)) continue;
          // One dispatch per SESSION, not per tab: `renameTab` already mirrors
          // into every tab bound to that session.
          dispatch({
            type: 'renameTab',
            sessionId,
            title: row.name,
            userSetName: row.user_set_name ?? false,
          });
          return true;
        }
      }
      return false;
    };

    const titleOf = (sessionId: string): string | undefined => {
      for (const group of Object.values(stateRef.current?.groups ?? {})) {
        const tab = group.tabs.find((t) => t.sessionId === sessionId);
        if (tab) return tab.title;
      }
      return undefined;
    };

    /**
     * Fold the types some rows reported into `rowSessionTypes`, in ONE update:
     * the latest row wins, and — when `held` is given — an entry for a chat no
     * tab holds any more is dropped. Same object back when nothing changed, so
     * an identical answer (every list refresh, in the common case) costs no
     * strip render. The memory gets the same writes and the same forgetting,
     * outside the updater, which React may call twice.
     */
    const keepSessionTypes = (
      types: ReadonlyMap<string, SessionType>,
      held?: ReadonlySet<string>
    ) => {
      for (const [sessionId, sessionType] of types) rememberSessionType(sessionId, sessionType);
      if (held) forgetSessionTypesExcept(held);
      setRowSessionTypes((prev) => {
        let next = held ? onlyHeld(prev, held) : prev;
        for (const [sessionId, sessionType] of types) {
          if (next[sessionId] !== sessionType) {
            if (next === prev) next = { ...prev };
            next[sessionId] = sessionType;
          }
        }
        return next;
      });
    };

    /** Stop asking about a chat again: it was answered, or no tab holds it. */
    const dropRetry = (sessionId: string) => {
      clearTimeout(retryRef.current.get(sessionId)?.timer);
      retryRef.current.delete(sessionId);
    };

    /** Rule 6: ask again later, unless a newer read is out or the retries are spent. */
    const scheduleRetry = (sessionId: string, seq: number, generation: number) => {
      // A read of a chat no tab holds has no newest entry any more, so this
      // also stops asking about a closed tab.
      if (!mountedRef.current || newestReadRef.current.get(sessionId) !== seq) return;
      const previous = retryRef.current.get(sessionId);
      clearTimeout(previous?.timer);
      // A new list starts the count again: the daemon answered for it.
      const count = previous?.generation === generation ? previous.count + 1 : 1;
      if (count > ROW_READ_MAX_RETRIES) {
        retryRef.current.set(sessionId, { generation, count });
        return;
      }
      const delay = Math.min(ROW_READ_RETRY_BASE_MS * 2 ** (count - 1), ROW_READ_RETRY_MAX_MS);
      const timer = setTimeout(() => {
        const entry = retryRef.current.get(sessionId);
        if (entry) entry.timer = undefined;
        if (!mountedRef.current) return;
        // Un-mark rule 3 for this chat, unless something has read it since.
        if (
          newestReadRef.current.get(sessionId) === seq &&
          readForGenerationRef.current.get(sessionId) === generation
        ) {
          readForGenerationRef.current.delete(sessionId);
        }
        // The reconcile re-reads it if a tab still holds it, under every rule.
        reconcileRef.current?.();
      }, delay);
      retryRef.current.set(sessionId, { generation, count, timer });
    };

    const readOutsideList = (sessionId: string, askedAbout: string, generation: number) => {
      readForGenerationRef.current.set(sessionId, generation);
      const seq = ++readSeqRef.current;
      newestReadRef.current.set(sessionId, seq);
      void (async () => {
        let row: Session | undefined;
        let status: number | undefined;
        try {
          const result = await getSession({
            path: { session_id: sessionId },
            // A name, a flag and a tier: nothing here needs the conversation.
            query: { metadata_only: true },
            // ⚠ Not optional. A subagent of a private chat is private, and
            // without the proof the reach gate refuses it (403, measured).
            headers: await userActionHeaders(),
            // No `throwOnError`: rule 6 turns on the status, which it drops.
          });
          row = result.data;
          // Absent when the request never completed, whatever the type says.
          status = (result.response as Response | undefined)?.status;
        } catch {
          // Nothing answered.
        }
        // Whether to ask again is the newest read's call alone (rule 5).
        const newest = newestReadRef.current.get(sessionId) === seq;
        if (!row) {
          if (!isAnswer(status)) {
            // Rule 6: nobody answered, so there is nothing to keep yet.
            scheduleRetry(sessionId, seq, generation);
          } else if (newest) {
            // Refused (a chat this caller may not reach) or deleted: an answer,
            // and the tab keeps what it has.
            dropRetry(sessionId);
          }
          return;
        }
        if (newest) dropRetry(sessionId);
        // A chat no tab holds any more has no tab to draw it, and keeping its
        // row would outlive the forgetting in `reconcile`.
        if (row.id !== sessionId || titleOf(sessionId) === undefined) return;
        // Remembered even when the shell has gone: the next mount seeds from it.
        if (row.session_type) rememberSessionType(sessionId, row.session_type);
        if (!mountedRef.current) return;
        // The tier is a fact whenever it was read — the ratchet only rises —
        // so it is kept even when the name below is not.
        const tier = row.privacy_tier ?? undefined;
        if (tier) {
          setOutsideListTiers((prev) => {
            const raised = raiseTier(prev[sessionId], tier);
            return raised && raised !== prev[sessionId] ? { ...prev, [sessionId]: raised } : prev;
          });
        }
        // So is the type, and for the same reason it is kept before rules 5
        // and 4 can drop the name.
        if (row.session_type) keepSessionTypes(new Map([[sessionId, row.session_type]]));
        // Rule 5, then rule 4.
        if (newestReadRef.current.get(sessionId) !== seq) return;
        if (titleOf(sessionId) !== askedAbout) return;
        renameFromRow(sessionId, row);
      })();
    };

    const reconcile = () => {
      const rows = getCachedSessionList();
      const state = stateRef.current;
      if (!rows || !state) return;
      if (rows !== listRef.current.rows) {
        listRef.current = { rows, generation: listRef.current.generation + 1 };
      }
      const { generation } = listRef.current;
      const rowById = new Map(rows.map((row) => [row.id, row]));
      const seen = new Set<string>();
      const listedTypes = new Map<string, SessionType>();
      for (const group of Object.values(state.groups)) {
        for (const tab of group.tabs) {
          if (!tab.sessionId || seen.has(tab.sessionId)) continue;
          seen.add(tab.sessionId);
          const row = rowById.get(tab.sessionId);
          if (row) {
            // A listed row's type counts as much as a row read on its own: with
            // History's "Show subagent runs" ticked, a subagent tab IS listed,
            // and is never read on its own (see "its session type", above).
            if (row.session_type) listedTypes.set(tab.sessionId, row.session_type);
            // Rule 1 lives inside: a user-named tab is never renamed.
            renameFromRow(tab.sessionId, row);
          } else if (readForGenerationRef.current.get(tab.sessionId) !== generation) {
            // Read even for a tab the user named: its NAME is left alone
            // (rule 1, inside `renameFromRow`), but its tier is still a fact
            // the strip has no other source for.
            readOutsideList(tab.sessionId, tab.title, generation);
          }
        }
      }
      // Forget chats no tab holds any more, so the maps cannot grow.
      for (const sessionId of readForGenerationRef.current.keys()) {
        if (!seen.has(sessionId)) {
          readForGenerationRef.current.delete(sessionId);
          newestReadRef.current.delete(sessionId);
        }
      }
      for (const sessionId of [...retryRef.current.keys()]) {
        if (!seen.has(sessionId)) dropRetry(sessionId);
      }
      // The listed types, and the same forgetting for the type map and the
      // tier map — which also keeps an id reissued by a store without the
      // high-water mark from inheriting a closed tab's kind or tier.
      keepSessionTypes(listedTypes, seen);
      setOutsideListTiers((prev) => onlyHeld(prev, seen));
    };
    reconcileRef.current = reconcile;
    reconcile();
    // Subscribe BEFORE asking for the fetch, for the reason `useSessionPrivacyTiers`
    // gives: a cache that resolved in between would emit to nobody.
    const unsubscribe = subscribeSessionList(reconcile);
    preloadSessionList();
    return unsubscribe;
  }, [dispatch, tabTitleSignature]);

  // The two sources that publish a tier during render publish a type the same
  // way, and are folded in during render (see "from every source", above).
  const listSessionTypes = useSessionListTypes();
  const liveSessionTypes = useLiveSessionTypes();
  const sessionTypes = useMemo(
    () =>
      fillSessionTypes(rowSessionTypes, heldSessionIds(groups?.state), [
        listSessionTypes,
        liveSessionTypes,
      ]),
    [rowSessionTypes, groups?.state, listSessionTypes, liveSessionTypes]
  );
  // Remembered like a row's answer: the reconcile prunes the memory to the held
  // chats, and its cap bounds it.
  useEffect(() => {
    for (const sessionId of heldSessionIds(stateRef.current)) {
      const sessionType = liveSessionTypes[sessionId];
      if (sessionType) rememberSessionType(sessionId, sessionType);
    }
  }, [liveSessionTypes, tabTitleSignature]);

  return { outsideListTiers, rowSessionTypes: sessionTypes };
}

export function ChatGroupsShell({ onChatChange }: ChatGroupsShellProps) {
  const groups = useChatGroups();
  const terminalDock = useTerminalDock();
  // Each map keeps its identity until one of its own sources changes — the tier
  // map is state, the type map is memoised over state and two snapshots — and
  // the wrapper object is new per render and is never passed on.
  const { outsideListTiers, rowSessionTypes } = useTabTitlesFromSessionList(groups);
  const privacyTiers = useSessionPrivacyTiers(outsideListTiers);

  const isMobile = useIsMobile();
  const { state: sidebarState } = useSidebar();
  const [isSidebarCompact, setIsSidebarCompact] = useState(
    () => typeof window !== 'undefined' && window.innerWidth < SIDEBAR_COMPACT_TITLE_WIDTH
  );
  useEffect(() => {
    const update = () => setIsSidebarCompact(window.innerWidth < SIDEBAR_COMPACT_TITLE_WIDTH);
    update();
    window.addEventListener('resize', update);
    return () => window.removeEventListener('resize', update);
  }, []);

  const isMacOS = (window?.electron?.platform || 'darwin') === 'darwin';
  const isCompactSidebarOverlayOpen = isSidebarCompact && !isMobile && sidebarState !== 'collapsed';
  const reserveTitlebarControls =
    isMacOS && (isMobile || isSidebarCompact || sidebarState === 'collapsed');

  // firstLeaf is a TREE WALK, never an array index. In a root `col` split both
  // strips sit at x=0 but only the TOP one collides with the traffic lights, so
  // an index would reserve the gap for the wrong strip — silently.
  const reservedGroupId = useMemo(() => (groups ? firstLeaf(groups.state.layout) : null), [groups]);

  const dispatch = groups?.dispatch;
  const handleSelect = useCallback(
    (tabId: string) => dispatch?.({ type: 'activateTab', tabId }),
    [dispatch]
  );
  const handleClose = useCallback(
    (tabId: string) => dispatch?.({ type: 'closeTab', tabId }),
    [dispatch]
  );
  const handleReorder = useCallback(
    (draggedTabId: string, targetTabId: string) =>
      dispatch?.({ type: 'reorderTab', draggedTabId, targetTabId }),
    [dispatch]
  );
  // The "+" at the end of a strip opens a fresh blank chat IN THAT group — the
  // same empty-session tab Cmd+T opens (sessionId '' is not deduped, so every
  // click is a new tab). Per-group, so a "+" on a split pane adds to that pane.
  const handleNewTab = useCallback(
    (groupId: ChatGroupId) => dispatch?.({ type: 'openTab', payload: { sessionId: '', groupId } }),
    [dispatch]
  );
  const handleDropToGroup = useCallback(
    (tabId: string, target: DropTarget) =>
      dispatch?.({
        type: 'moveTabToGroup',
        tabId,
        targetGroupId: target.groupId,
        zone: target.zone,
      }),
    [dispatch]
  );

  // ═════════════════════════════════════════════════════════════════════════
  // TAB TEAR-OFF AND MERGE — the renderer's half of the cross-window gesture.
  //
  // docs/design/astryx-adoption/tab-tear-off-and-merge.md, Phase 3. This window
  // plays BOTH roles and they never overlap in time:
  //
  //   as SOURCE — it holds the pointer capture, reports the screen point on
  //   every move that leaves its own content rect, and commits on release.
  //
  //   as TARGET — it receives no pointer events at all (measured, Phase 0), so
  //   its caret and its insert are driven entirely by IPC from main.
  //
  // NOTHING HERE ASKS WHETHER A TAB IS RUNNING. Design D1 refused to move a tab
  // with a turn in flight; it is superseded (§3.1) — the turn stream now has as
  // many subscribers as it likes, and the destination window rejoins the turn
  // off `/agent/resume` on load, from the sequence it has already painted.
  // ═════════════════════════════════════════════════════════════════════════

  const desktop = typeof window !== 'undefined' ? window.electron : undefined;
  const stateRefForDrag = useRef(groups?.state);
  stateRefForDrag.current = groups?.state;

  // `commitCrossWindow` needs the dragged tab and the grab offset, both of which
  // live in the hook it is an ARGUMENT to. A ref breaks that cycle without
  // making the callback identity change per pointermove — which would re-run the
  // hook's effect and drop its window listeners in the middle of a gesture.
  const dragRef = useRef<{ draggedTabId: string | null; grabOffset: { x: number; y: number } }>({
    draggedTabId: null,
    grabOffset: { x: 0, y: 0 },
  });

  /** The caret a MERGE is currently previewing in this window, if any. */
  const [remoteDrop, setRemoteDrop] = useState<MergeInsertion | null>(null);

  const reportCrossWindow = useCallback(
    (phase: CrossWindowPhase, point: ScreenPoint) => {
      if (!desktop?.tabDragMove) return;
      // `local` is reported exactly once, on the way back in — the hook
      // guarantees that. It means "no cross-window preview anywhere", which is
      // the same message `tabDragEnd` carries, so it uses the same door.
      if (phase.kind === 'local') desktop.tabDragEnd?.();
      else desktop.tabDragMove({ screenX: point.screenX, screenY: point.screenY });
    },
    [desktop]
  );

  const commitCrossWindow = useCallback(
    (phase: CrossWindowPhase, point: ScreenPoint) => {
      const state = stateRefForDrag.current;
      const dispatchNow = dispatch;
      if (!state || !dispatchNow || !desktop?.tabDragCommit) return;
      const draggedTabId = phase.kind === 'local' ? null : dragRef.current?.draggedTabId;
      if (!draggedTabId) return;
      const tab = leafGroupIds(state.layout)
        .flatMap((gid) => state.groups[gid]?.tabs ?? [])
        .find((candidate) => candidate.tabId === draggedTabId);
      if (!tab) return;

      // Obligation 2 / D5. `isOnlyTab` is sent rather than acted on here because
      // a tear-off of the last tab is a no-op while a MERGE of it is allowed and
      // closes this window — and only main knows which of those this drop is.
      const isOnlyTab = countWindowTabs(state) <= 1;

      void desktop
        .tabDragCommit({
          point: { screenX: point.screenX, screenY: point.screenY },
          grabOffset: dragRef.current?.grabOffset ?? { x: 0, y: 0 },
          tab: payloadFromTab(tab),
          isOnlyTab,
        })
        .then((result) => {
          // `noop` is the answer to everything that went wrong, and it means
          // exactly one thing: the tab stays. Never remove it speculatively.
          if (!result || result.outcome === 'noop') return;
          if (result.outcome === 'merge' && isOnlyTab) {
            // D6a — the target has ACKNOWLEDGED the insert, so this window is
            // now empty and closing it loses nothing. Closing outright rather
            // than closing the tab first: `closeTab` on the last tab would bounce
            // this window Home (useEmptyPairRedirect) for the frame before it
            // disappears.
            desktop.closeWindow?.();
            return;
          }
          dispatchNow({ type: 'closeTab', tabId: draggedTabId });
        })
        .catch(() => {
          // An IPC failure is not a reason to lose a chat.
        });
    },
    [desktop, dispatch]
  );

  // ONE gesture for every strip: the tint has to appear over the TARGET group,
  // which is a sibling of the source strip, so the drag cannot live inside a
  // strip. Handed down through ChatTabDragProvider.
  const drag = useTabDragReorder({
    onReorder: handleReorder,
    onDropToGroup: handleDropToGroup,
    onCrossWindow: reportCrossWindow,
    onCrossWindowCommit: commitCrossWindow,
  });
  dragRef.current = {
    draggedTabId: drag.draggedTabId,
    grabOffset: drag.ghost
      ? { x: drag.ghost.grabOffsetX, y: drag.ghost.grabOffsetY }
      : { x: 0, y: 0 },
  };

  const layoutForBands = groups?.state.layout;

  /**
   * Tell main where this window's tab strips are.
   *
   * Viewport-relative — main translates with `getContentBounds()`, which the
   * renderer cannot do for itself (`window.screenX` is the WINDOW origin, and
   * the difference from the content origin is the title bar, which is precisely
   * the band being measured).
   *
   * Reported on mount, on resize, and whenever the layout tree changes, because
   * a split gives this window a second strip and each one is an independent
   * merge target. NOT on window MOVE: bands are viewport-relative, so a move
   * cannot change them, and main re-reads the content rect on every pointermove
   * anyway.
   */
  useEffect(() => {
    const report = () => desktop?.tabDragRegisterBands?.(measureStripBands(document));
    // One frame late so the strips have been laid out — a `getBoundingClientRect`
    // taken in the same commit that added a pane measures the pane before flex
    // has divided the row.
    const raf = window.requestAnimationFrame(report);
    window.addEventListener('resize', report);
    return () => {
      window.cancelAnimationFrame(raf);
      window.removeEventListener('resize', report);
    };
    // `layoutForBands` alone: the group COUNT is derived from the tree, so a
    // split that adds a strip already changes this identity.
  }, [desktop, layoutForBands]);

  /**
   * TARGET-SIDE. Main forwards the source's screen point; this window converts
   * it to its own client coordinates and paints the ordinary insertion caret.
   *
   * `screenX − window.screenX` is the conversion, and it was measured to the
   * pixel in Phase 0 rather than assumed. This window must NOT raise or focus
   * itself here: it does not hold the pointer, and coming forward would take the
   * capture away from the window that does (design D3).
   */
  useEffect(() => {
    if (!desktop?.on) return;
    return desktop.on('tab-drag:preview', (_event, ...args) => {
      const payload = args[0] as
        | { active?: boolean; screenX?: number; screenY?: number }
        | undefined;
      if (!payload?.active || typeof payload.screenX !== 'number') {
        setRemoteDrop(null);
        return;
      }
      setRemoteDrop(
        resolveMergeInsertion(
          document,
          payload.screenX - window.screenX,
          (payload.screenY ?? 0) - window.screenY
        )
      );
    });
  }, [desktop]);

  /**
   * TARGET-SIDE, the commit. Insert at the caret the preview was showing, then
   * acknowledge — and only then does the source drop its copy (D6a). A refusal
   * (`false`) leaves the tab where it was, which is the right answer for every
   * way this can fail: no strip under the point, a layout that changed between
   * the preview and the release, a window mid-reload.
   *
   * ═══════════════════════════════════════════════════════════════════════
   * THE DEADLINE IS THE OTHER HALF OF D6a, AND IT USED TO BE MISSING HERE.
   *
   * Main gives up on an unanswered merge after a couple of seconds and tells
   * the source to KEEP its tab. This window had no deadline at all — so a
   * window busy with a heavy streaming turn could process the request long
   * after that, insert the tab, and acknowledge into a request that no longer
   * existed. The ack was dropped, the source kept its tab, and the SAME
   * SESSION was then open in two windows.
   *
   * `expiresAt` is main's deadline already backed off by a grace period, so a
   * request accepted here still has time on main's clock for the ack to get
   * back. Past it, refusing is not a degraded outcome: the tab simply stays
   * where the user last saw it.
   * ═══════════════════════════════════════════════════════════════════════
   */
  useEffect(() => {
    if (!desktop?.on || !dispatch) return;
    return desktop.on('tab-drag:merge', (_event, ...args) => {
      const payload = args[0] as
        | {
            requestId: number;
            tab?: {
              sessionId: string;
              title: string;
              userSetName: boolean;
              cwd?: string;
              workflowId?: string;
            };
            screenX: number;
            screenY: number;
            expiresAt?: number;
          }
        | undefined;
      setRemoteDrop(null);
      if (!payload?.tab) return;
      if (typeof payload.expiresAt === 'number' && Date.now() >= payload.expiresAt) {
        // Refuse EXPLICITLY rather than staying silent: main is still holding
        // the source's commit open, and an answer now ends it immediately
        // instead of making the user watch out the rest of the timeout.
        desktop.tabDragAckMerge?.(payload.requestId, false);
        return;
      }
      const insertion = resolveMergeInsertion(
        document,
        payload.screenX - window.screenX,
        payload.screenY - window.screenY
      );
      if (!insertion) {
        desktop.tabDragAckMerge?.(payload.requestId, false);
        return;
      }
      dispatch({
        type: 'openTab',
        payload: {
          sessionId: payload.tab.sessionId,
          title: payload.tab.title,
          userSetName: payload.tab.userSetName,
          cwd: payload.tab.cwd,
          workflowId: payload.tab.workflowId,
          groupId: insertion.groupId,
          index: insertion.index,
        },
      });
      dispatch({ type: 'setActiveGroup', groupId: insertion.groupId });
      desktop.tabDragAckMerge?.(payload.requestId, true);
    });
  }, [desktop, dispatch]);

  // A gesture that ends any way at all must leave no caret in any window. The
  // hook reports `local` on Escape and on re-entry, but a pointerup outside goes
  // straight to the commit path — which clears main's preview itself — and an
  // unmount mid-drag reports nothing at all.
  useEffect(() => () => desktop?.tabDragEnd?.(), [desktop]);

  /**
   * Mirror the loaded session's real name onto its tab.
   *
   * The rename mirror in ChatGroupsContext only listens to
   * `announceSessionName`, and BaseChat only announces on an explicit RENAME —
   * nothing announces when a session merely LOADS. So a tab opened from the
   * sidebar or a deep link kept its creation-time placeholder and sat there
   * reading "New chat" while document.title showed the real name. Caught by
   * driving the app; jsdom could not have shown it, because it needs a session
   * to actually load.
   */
  const handleSessionLoaded = useCallback(
    (session: { id: string; name: string; userSetName: boolean; workingDir?: string } | null) => {
      if (!session?.id) return;
      // ⚠ Recorded BEFORE the name guard below, and separately from it.
      //
      // `ChatTab.cwd` had no writer, so `payloadFromTab` always omitted it and
      // a torn-off window fell back to `os.homedir()` — every new chat opened
      // there was created in `~` instead of the project. Folding this into the
      // rename dispatch would inherit its `!session.name` guard and lose the
      // directory for any session that has not been named yet, which is
      // exactly a freshly created one.
      if (session.workingDir) {
        dispatch?.({ type: 'setTabCwd', sessionId: session.id, cwd: session.workingDir });
      }
      if (!session.name) return;
      dispatch?.({
        type: 'renameTab',
        sessionId: session.id,
        title: session.name,
        userSetName: session.userSetName,
      });
    },
    [dispatch]
  );

  const activeGroupId = groups?.state.activeGroupId;
  const handleFocusGroup = useCallback(
    (groupId: ChatGroupId) => {
      if (groupId === activeGroupId) return;
      dispatch?.({ type: 'setActiveGroup', groupId });
    },
    [dispatch, activeGroupId]
  );

  const layout = groups?.state.layout;
  const groupCount = useMemo(() => (layout ? groupCountOf(layout) : 1), [layout]);

  /**
   * Rung 4 of the yield ladder (D-32): a split merges back to one group rather
   * than render two useless slivers.
   *
   * Modelled on AppLayout's sidebar watcher, down to the shape of the rule,
   * because it is the same effect and would fail the same way. Three things make
   * it safe to let it move a layout the user built by hand:
   *
   *   1. it fires ONLY on a width CROSSING. `state` is read through a ref and is
   *      NOT a dep — a layout change must never re-run this, or splitting a
   *      narrow window by hand would be undone inside the same tick as the drop.
   *      That is the exact bug that made the sidebar's un-collapse button dead.
   *   2. it is REVERSIBLE, and only reverses what WE did: the snapshot is what
   *      we owe the user, and it is taken at the moment we take the split away.
   *   3. the user can always overrule it. Split again while merged and the
   *      snapshot is forfeit (splitSnapshotIsStale) — their layout is theirs, and
   *      growing the window will not throw it away to restore ours.
   *
   * Observed on the shell's own box, not the window: the sidebar collapsing
   * changes the room available to the groups without the window moving at all.
   * No feedback loop is possible — merging changes the TREE INSIDE this box, and
   * the box is sized by SidebarInset above it, so the callback cannot retrigger
   * itself. Hence no hysteresis: none is needed, and adding it on spec would only
   * make the crossing harder to reason about.
   */
  const treeRef = useRef<HTMLDivElement | null>(null);
  const stateRef = useRef(groups?.state);
  stateRef.current = groups?.state;
  const mergeSnapshotRef = useRef<GroupLayoutSnapshot | null>(null);
  const lastShellWidthRef = useRef<number | null>(null);

  useEffect(() => {
    const tree = treeRef.current;
    if (!tree || !dispatch) return;

    const sample = () => {
      const state = stateRef.current;
      if (!state) return;
      const width = tree.clientWidth;
      // An unmeasured box is not a narrow one. Without this, a 0-width sample —
      // first paint, a hidden window, a display change — reads as "everything
      // fits" and hands the split back at zero pixels.
      if (!(width > 0)) return;
      const groupCountNow = groupCountOf(state.layout);

      // Did the user build their own layout while we had theirs merged away? Then
      // ours is forfeit — checked BEFORE the fit, so the rest of this sample
      // judges the layout the user actually has.
      if (mergeSnapshotRef.current && splitSnapshotIsStale({ groupCount: groupCountNow })) {
        mergeSnapshotRef.current = null;
      }

      const { wasFitting, isFitting } = splitYieldSample({
        layout: state.layout,
        snapshotLayout: mergeSnapshotRef.current?.layout ?? null,
        lastWidth: lastShellWidthRef.current,
        width,
      });
      const action = splitYieldAction({
        wasFitting,
        isFitting,
        groupCount: groupCountNow,
        autoMerged: mergeSnapshotRef.current !== null,
      });
      // Recorded before the dispatch, exactly as the sidebar records wasCompact:
      // the re-render that follows must not read a stale previous side.
      lastShellWidthRef.current = width;

      if (action === 'merge') {
        mergeSnapshotRef.current = snapshotGroupLayout(state);
        dispatch({ type: 'mergeAllGroups' });
      } else if (action === 'restore') {
        const snapshot = mergeSnapshotRef.current;
        mergeSnapshotRef.current = null;
        if (snapshot) dispatch({ type: 'restoreLayout', snapshot });
      }
    };

    sample();

    if (typeof ResizeObserver === 'undefined') {
      window.addEventListener('resize', sample);
      return () => window.removeEventListener('resize', sample);
    }
    const observer = new ResizeObserver(sample);
    observer.observe(tree);
    return () => observer.disconnect();
  }, [dispatch]);

  // Terminals are keyed by tab id and outlive the tab's BaseChat (which remounts
  // on every tab switch). When a tab is CLOSED for good, nothing else disposes
  // its hidden-but-live terminal, so the shell hands the dock the set of tab ids
  // that still exist and it drops the rest — unmounting them, which disposes
  // their pty. Idempotent: retain is a no-op when nothing needs dropping.
  const allTabIds = useMemo(() => {
    if (!groups) return [] as string[];
    return leafGroupIds(groups.state.layout).flatMap(
      (gid) => groups.state.groups[gid]?.tabs.map((t) => t.tabId) ?? []
    );
  }, [groups]);
  const retainTerminals = terminalDock?.retain;
  useEffect(() => {
    retainTerminals?.(allTabIds);
  }, [retainTerminals, allTabIds]);

  if (!groups || !layout) return null;

  const renderGroup = ({ groupId }: RenderGroupArgs) => {
    const group = groups.state.groups[groupId];
    if (!group) return <div key={groupId} />;
    const activeTab = group.tabs.find((t) => t.tabId === group.activeTabId);
    const isActiveGroup = groupId === groups.state.activeGroupId;
    // Every terminal key this group can own: one per tab, plus the empty-tab
    // key. The pane renders only its OWN terminals (a terminal belongs to one
    // pane), and shows the one whose tab is active in THIS pane — so a 4-way
    // split can have four terminals open at once, each scoped to its pane and
    // each the width of its pane, never a bar spanning the whole window.
    const groupTabKeys = [...group.tabs.map((t) => t.tabId), `${groupId}-empty`];

    const strip = (
      <ChatTabStrip
        tabs={group.tabs}
        activeTabId={group.activeTabId}
        groupId={groupId}
        groupActive={isActiveGroup}
        runningSessionIds={groups.runningSessionIds}
        tabAnnotations={groups.tabAnnotations}
        privacyTiers={privacyTiers}
        // What each tab's chat's row says it is — read on its own, listed, or
        // loaded by the live store. The strip ORs a `sub_agent` here with
        // `tabAnnotations`, which do not survive leaving `/pair` or a reload.
        sessionTypes={rowSessionTypes}
        // The MERGE caret. It cannot come from `dragOverTabId` like the local
        // one does: while a cross-window drag is in flight this window receives
        // no pointer events at all, so its own drag state is empty and the caret
        // is driven entirely by IPC from main (design D3, Phase 0).
        remoteDropBeforeTabId={remoteDrop?.groupId === groupId ? remoteDrop.beforeTabId : null}
        onSelect={handleSelect}
        onClose={handleClose}
        onReorder={handleReorder}
        reserveTitlebar={reserveTitlebarControls && groupId === reservedGroupId}
        isCompactSidebarOverlayOpen={isCompactSidebarOverlayOpen}
        endSlot={
          <button
            type="button"
            aria-label="New chat"
            title="New chat"
            data-testid="chat-tab-new"
            onClick={() => handleNewTab(groupId)}
            className="br-tab-new ml-0.5 flex h-6 w-6 items-center justify-center rounded-md text-text-muted transition-colors hover:bg-background-medium hover:text-text-default"
          >
            <Plus className="h-4 w-4" />
          </button>
        }
      />
    );

    return (
      <ChatGroupPane
        key={groupId}
        groupId={groupId}
        isActiveGroup={isActiveGroup}
        groupCount={groupCount}
        dropZone={drag.dropTarget?.groupId === groupId ? drag.dropTarget.zone : null}
        onFocusGroup={handleFocusGroup}
        terminalDock={terminalDock}
        groupTabKeys={groupTabKeys}
        // The tab is the chat's identity: keying on tabId (not sessionId) keeps
        // a tab's mount stable across a session bind. The SAME id keys the tab's
        // terminal, so it too survives the bind and switching tabs switches it.
        tabKey={activeTab?.tabId ?? `${groupId}-empty`}
        // No tab yet means this pane is the placeholder, and the placeholder is
        // always replaced rather than filled: the key above changes when the
        // real tab lands, which unmounts this whole subtree. Drawing a greeting
        // it will throw away made a new window show one heading, drop it, and
        // unroll a different one.
        suppressGreeting={activeTab === undefined}
        sessionId={activeTab?.sessionId ?? ''}
        initialMessage={activeTab?.pendingInitialMessage}
        initialAttachments={activeTab?.pendingInitialAttachments}
        // Spend the route cargo exactly once. The reducer has always had
        // `consumePending`; nothing ever dispatched it, so a tab kept the
        // message that created its session forever and re-sent it on every
        // remount (BR duplicate-submission bug, 2026-07-18).
        onInitialMessageConsumed={
          activeTab
            ? () => dispatch?.({ type: 'consumePending', tabId: activeTab.tabId })
            : undefined
        }
        renderSessionTitle={() => strip}
        onChatChange={onChatChange}
        onSessionLoaded={handleSessionLoaded}
      />
    );
  };

  const tree = renderLayout(layout, [], renderGroup, (branch, path, children) => (
    <ChatGroupBranch
      key={`branch-${path.join('-')}`}
      branch={branch}
      path={path}
      onResize={(sizes) => dispatch?.({ type: 'resizeBranch', path, sizes })}
    >
      {children}
    </ChatGroupBranch>
  ));

  return (
    <ChatTabDragProvider value={drag}>
      <div className="flex h-full min-h-0 w-full flex-col">
        {/* treeRef: rung 4 measures the room the GROUPS have, which the sidebar
            can change without the window moving. */}
        {/* Terminals render INSIDE their pane now (see ChatGroupPane), not here
            at the shell as a full-width bar below every group. A terminal belongs
            to one pane, is the width of that pane, and a split can show several
            at once — each scoped to its own session's working directory. */}
        <div ref={treeRef} className="flex min-h-0 flex-1">
          {tree}
        </div>
      </div>
      {/* The ghost renders at the SHELL, not in the source strip: it is fixed to
          the viewport and must not be clipped by the strip's overflow-x:auto. */}
      {/* `detached` is D7's "this will become a window" state: the tilt goes to
          0, it takes the dashed accent outline, and it CLAMPS to this window's
          frame — Electron cannot paint outside it, so an unclamped ghost simply
          flies past the edge and is clipped, which reads as the tab having been
          eaten rather than as one about to become a window. */}
      {drag.ghost && (
        <ChatTabGhost ghost={drag.ghost} detached={drag.crossWindow.kind !== 'local'} />
      )}
    </ChatTabDragProvider>
  );
}

interface ChatGroupBranchProps {
  branch: Extract<GroupLayout, { kind: 'branch' }>;
  path: readonly number[];
  onResize: (sizes: number[]) => void;
  children: ReactElement[];
}

function ChatGroupBranch({ branch, onResize, children }: ChatGroupBranchProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  // Sizes are normalized on the branch by the reducer, but a persisted or
  // mid-migration tree can arrive with a sizes array that disagrees with the
  // children count. Falling back to an even split keeps a bad blob renderable
  // instead of collapsing panes to flexBasis: undefined.
  const sizes =
    branch.sizes.length === children.length
      ? branch.sizes
      : children.map(() => 1 / children.length);

  return (
    <div
      ref={containerRef}
      data-testid="chat-group-branch"
      data-dir={branch.dir}
      className={
        branch.dir === 'row'
          ? 'flex min-h-0 min-w-0 flex-1'
          : 'flex min-h-0 min-w-0 flex-1 flex-col'
      }
    >
      {children.map((child, index) => (
        <Fragment key={child.key ?? index}>
          {/* Splitter i lives between pane i and pane i+1. */}
          {index > 0 && (
            <ChatGroupSplitter
              dir={branch.dir}
              index={index - 1}
              sizes={sizes}
              containerRef={containerRef}
              onResize={onResize}
            />
          )}
          <div
            className="flex min-h-0 min-w-0"
            // `<size> 1 0` — grow PROPORTIONAL to the fraction, from a zero
            // basis. Not `0 0 <pct>%`: the splitters are flex siblings that
            // occupy real pixels, so percentages of the container would sum past
            // 100% and overflow. From a zero basis the panes divide exactly
            // whatever is left after the handles, in the right ratio.
            //
            // min-w-0 / min-h-0 is what makes the ratio hold: flex items default
            // to min-width:auto, so one long unbreakable transcript line would
            // otherwise push its pane past its share and silently rewrite the
            // split the user set.
            style={{ flex: `${sizes[index]} 1 0` }}
          >
            {child}
          </div>
        </Fragment>
      ))}
    </div>
  );
}

interface ChatGroupPaneProps {
  groupId: ChatGroupId;
  isActiveGroup: boolean;
  groupCount: number;
  dropZone: import('./dropZones').DropZone | null;
  onFocusGroup: (groupId: ChatGroupId) => void;
  terminalDock: ReturnType<typeof useTerminalDock>;
  /** Every terminal key this group can own (one per tab + the empty-tab key). */
  groupTabKeys: string[];
  tabKey: string;
  /** See `BaseChat`'s prop: the placeholder pane skips the rotating greeting. */
  suppressGreeting: boolean;
  sessionId: string;
  initialMessage?: string;
  initialAttachments?: import('../../types/message').UserAttachment[];
  onInitialMessageConsumed?: () => void;
  renderSessionTitle: () => ReactElement;
  onChatChange: (chat: ChatType) => void;
  onSessionLoaded: (
    session: { id: string; name: string; userSetName: boolean; workingDir?: string } | null
  ) => void;
}

/**
 * One group: the drop hit-test target, the focus target, and the host for its
 * active tab's chat.
 */
function ChatGroupPane({
  groupId,
  isActiveGroup,
  groupCount,
  dropZone,
  onFocusGroup,
  terminalDock,
  groupTabKeys,
  tabKey,
  sessionId,
  suppressGreeting,
  initialMessage,
  initialAttachments,
  onInitialMessageConsumed,
  renderSessionTitle,
  onChatChange,
  onSessionLoaded,
}: ChatGroupPaneProps) {
  // Extensions load per-session, so in a 4-way split four chats each finish
  // their own load and each has something to report. Telling the toast layer
  // which chat you are actually in lets a background group's clean load stay
  // silent instead of stacking over the transcript you are reading. Failures
  // still speak up from any group — see showExtensionLoadResults.
  useEffect(() => {
    if (isActiveGroup && sessionId) {
      setFocusedChatSession(sessionId);
    }
  }, [isActiveGroup, sessionId]);

  return (
    <div
      // The drag hit-test looks for exactly this attribute
      // (dropZones.dropTargetAtPoint). It must sit on the element whose
      // getBoundingClientRect IS the group's box, or the zones would be measured
      // against the wrong rectangle and the tint would land off the group.
      data-chat-group-id={groupId}
      data-active-group={isActiveGroup ? 'true' : 'false'}
      // flex COLUMN: the chat fills the pane and this pane's terminal (if any)
      // stacks below it, inside the pane — so the terminal is the pane's width,
      // not the window's. The box is still exactly the group's rectangle, which
      // is what the drag hit-test measures against.
      className="relative flex min-h-0 min-w-0 flex-1 flex-col"
      // CAPTURE phase, both: clicking anywhere in a group focuses it, including
      // on controls that stopPropagation on the bubble (the composer's buttons,
      // the strip's close ×). Focus must not depend on WHERE in the pane you
      // clicked. focus-capture covers the keyboard path — tabbing into a pane
      // focuses its group without any pointer at all.
      onPointerDownCapture={() => onFocusGroup(groupId)}
      onFocusCapture={() => onFocusGroup(groupId)}
    >
      <div className="flex min-h-0 min-w-0 flex-1">
        <BaseChat
          key={tabKey}
          // Scopes this tab's terminal. Same id as the React key, so the terminal
          // is tied to the tab, not the (rebindable) session.
          terminalKey={tabKey}
          setChat={onChatChange}
          sessionId={sessionId}
          initialMessage={initialMessage}
          initialAttachments={initialAttachments}
          onInitialMessageConsumed={onInitialMessageConsumed}
          suppressEmptyState={false}
          suppressGreeting={suppressGreeting}
          renderSessionTitle={renderSessionTitle}
          onSessionUpdate={onSessionLoaded}
          // The preview panel follows the ACTIVE group. State is kept, only the
          // render is gated — see BaseChat's artifactPanelEnabled doc.
          artifactPanelEnabled={isActiveGroup}
          // A session-scoped chat must never resize the OS window once it is not
          // the only one: a background group opening an artifact would resize the
          // window out from under the group you are actually looking at.
          allowWindowResize={groupCount === 1}
          // Only the focused pane's composer takes the caret when it mounts.
          // Every pane mounts at once when /pair is rebuilt, and a focus inside
          // a pane makes it the focused one (`onFocusCapture` above), so the
          // LAST pane used to win the caret and the focus — whichever pane an
          // arrival had just focused, and whichever the person had left.
          autoFocusComposer={isActiveGroup && !isTabKeyboardFocusPending(tabKey)}
        />
      </div>
      {/* This pane's OWN terminals, stacked below its chat inside the pane's
          flex column — so a terminal is the width of its pane and belongs to
          exactly one pane, never a full-window bar. Every terminal in the group
          stays mounted so its shell keeps running; only the one whose tab is
          active in THIS pane is visible (open). `onClose` HIDES a terminal (its
          panes live on); `onEmptied`, fired when its last pane closes, DESTROYS
          it. Each reads its own frozen cwd — its tab's session folder, captured
          on open. */}
      {terminalDock?.terminals
        .filter((terminal) => groupTabKeys.includes(terminal.key))
        .map((terminal) => (
          <InAppTerminalDock
            key={terminal.key}
            // Also its scope for "Run this code block": a Run clicked in this
            // tab's transcript must land in THIS terminal, not in whichever
            // pane happens to hold focus.
            dockKey={terminal.key}
            open={terminal.showing && terminal.key === tabKey}
            workingDir={terminal.workingDir}
            onClose={() => terminalDock.setOpen(terminal.key, false)}
            onEmptied={() => terminalDock.remove(terminal.key)}
          />
        ))}
      {dropZone && <ChatDropOverlay zone={dropZone} />}
    </div>
  );
}

export default ChatGroupsShell;
