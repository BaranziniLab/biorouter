import { CSSProperties, useCallback, useEffect, useLayoutEffect, useRef } from 'react';
import { ChevronDown, X } from '../icons/app-icons';
import { cn } from '../../utils';
import { getSessionTitlePadding } from '../Layout/TitlebarControls';
import { useTabStripOverflow } from '../Layout/useTabStripOverflow';
import { ContextMenu, ContextMenuTrigger } from '../ui/context-menu';
import { ChatRowContextMenuContent } from '../chats/ChatRowContextMenu';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';
import { ChatTab, ChatTabId, ChatGroupId } from './chatGroupsTypes';
import { ChatKindIcon } from '../chats/ChatKindIcon';
import type { ChatKindSource } from '../chats/chatKind';
import type { SessionClassification } from '../../api';
import { useTabDragReorder } from './useTabDragReorder';
import { useTabBandWindowGesture } from './useTabBandWindowGesture';
import { useChatTabDrag } from './ChatTabDragContext';
import type { TabAnnotation } from './workspaceCommandPlanner';

/**
 * A tab is not a session row — it persists `{tabId, sessionId, title}` and
 * nothing about lineage — so the kind has to be assembled from what the strip
 * actually holds: the title (which still catches `app:` and legacy branch
 * names) plus the annotation the workspace planner recorded when a sub-agent
 * opened a tab.
 *
 * ⚠ Deliberately NOT widened by fetching the session: the strip renders on
 * every keystroke of a rename, and a per-tab fetch there is how a tab strip
 * becomes the slowest thing in the app.
 */
function tabKindSource(
  tab: { title: string },
  annotation?: { badge?: string; parentSessionId?: string }
): ChatKindSource {
  // ⚠ Only `badge === 'subagent'` marks a sub-agent here — deliberately NOT
  // `parentSessionId`. `TabAnnotation.badge` is a free-form string and the
  // planner may record a parent link for other reasons; the strip's own suite
  // guards that an annotation carrying a parent but no badge marks nothing.
  return {
    name: tab.title,
    session_type: annotation?.badge === 'subagent' ? 'sub_agent' : null,
  };
}

/**
 * The window-drag handle kept at the RIGHT end of the tab band, in px.
 *
 * The scroll box is `no-drag` across its whole width (see the note on it), so
 * without this the ~700px the tabs live in would stop moving the window
 * altogether. It is applied as PADDING on the wrap, and that is the only place
 * it can be STATIC: padding lives inside the wrap's `drag` border box, and the
 * wrap's box is a function of the header, never of how many tabs there are.
 *
 * One band height, so the handle reads as a square at the end of the row.
 */
export const TAB_BAND_DRAG_GUTTER = 44;

function prefersReducedMotion(): boolean {
  return (
    typeof window.matchMedia === 'function' &&
    window.matchMedia('(prefers-reduced-motion: reduce)').matches
  );
}

export interface ChatTabStripProps {
  tabs: ChatTab[];
  activeTabId: ChatTabId | null;
  /** The group this strip belongs to. Rides along with each drag gesture so the
   *  shared hook can tell a same-strip reorder from a cross-group move. */
  groupId?: ChatGroupId;
  /**
   * False when another group has focus. Only the FOCUSED group's active tab is
   * the painted pill; an inactive group's active tab keeps a neutral fill and no
   * pill shadow (spec: "Focus costs zero extra chrome"). Defaults to true so the
   * single-group case — and the strip's own tests — are unchanged.
   */
  groupActive?: boolean;
  /** Session ids with a live turn — from useRunningChats(). */
  runningSessionIds: readonly string[];
  /**
   * BR-71 Task 26's tab annotations, keyed by session id.
   *
   * A PROP, not a `useChatGroups()` call, for the same reason `runningSessionIds`
   * is: this component is pure-props and three test files render it bare, outside
   * any provider. OPTIONAL with a `{}` default so those files keep compiling and
   * passing with no edit — the strip's whole contract is that it can be rendered
   * with nothing but its own props. The shell's own suites additionally mock
   * `useChatGroups` with hand-written stubs that have no `tabAnnotations`, so the
   * default is what keeps `groups.tabAnnotations` being `undefined` a no-op there.
   */
  tabAnnotations?: Record<string, TabAnnotation>;
  /**
   * Privacy tier per SESSION id (issue #56, R10) — not per tab, and not a field
   * on `ChatTab`.
   *
   * `ChatTab` is persisted by chatGroupsStorage, and a tier persisted with a tab
   * is a tier that can be read back stale. The unsafe direction is the likely
   * one: the tier only ever RISES during a session, so a cached `public` would
   * leave a now-private chat unmarked. A session-keyed map recomputed from
   * sources with no state of their own has no such thing to go stale.
   *
   * ⚠ **Recomputed from TWO sources, folded with `max`** — the chat stores this
   * window holds (live, and where a ratchet during a turn first appears) over
   * the session-list cache (which covers tabs never opened here, and which
   * cannot carry a chat created in this window until it has recorded a
   * message). `ChatGroupsShell.useSessionPrivacyTiers` builds it, its doc gives
   * the measurement, and the two must not drift apart again: reading the list
   * cache ALONE is finding M8 — the active tab's dot stuck on public while the
   * sidebar, the chip and sqlite all read private.
   *
   * Optional with a `{}` default, like `tabAnnotations`, so the four suites that
   * mount this strip bare keep compiling untouched. An unknown session is
   * unmarked — silence, never an assertion of Public.
   */
  privacyTiers?: Record<string, SessionClassification>;
  onSelect: (tabId: ChatTabId) => void;
  onClose: (tabId: ChatTabId) => void;
  onReorder: (draggedTabId: ChatTabId, targetTabId: ChatTabId) => void;
  /**
   * True only for the FIRST leaf of the layout tree — computed by the shell with
   * firstLeaf(), a tree walk, never an array index. See chatGroupsTypes.firstLeaf.
   */
  reserveTitlebar: boolean;
  isCompactSidebarOverlayOpen: boolean;
  endSlot?: React.ReactNode;
  /**
   * The insertion caret for a CROSS-WINDOW merge — a tab being dragged into this
   * strip from another window.
   *
   * It needs its own prop because `dragOverTabId` cannot supply it. While the
   * mouse button is held, the OS delivers every pointer event to the window the
   * drag STARTED in; this window receives nothing at all (measured with real OS
   * input, design §8 Phase 0), so its own drag state stays empty for the whole
   * gesture. The caret is driven by IPC from the main process instead.
   *
   * The rendering is deliberately identical — the same `[data-dropbefore]`
   * hairline the local reorder draws. Merging into a strip IS inserting into a
   * strip; a second visual for it would be a second answer to a question the
   * strip already answers.
   */
  remoteDropBeforeTabId?: ChatTabId | null;
}

export function ChatTabStrip({
  tabs,
  activeTabId,
  groupId,
  groupActive = true,
  runningSessionIds,
  tabAnnotations = {},
  privacyTiers = {},
  onSelect,
  onClose,
  onReorder,
  reserveTitlebar,
  isCompactSidebarOverlayOpen,
  endSlot,
  remoteDropBeforeTabId = null,
}: ChatTabStripProps) {
  // The shell provides ONE gesture for every strip, because a cross-group drag
  // must tint a group this strip does not render. Bare (as in this component's
  // own tests) the strip falls back to a private, reorder-only instance — which
  // is exactly the Stage-2 behaviour, unchanged.
  const sharedDrag = useChatTabDrag();
  const ownDrag = useTabDragReorder({ onReorder });
  const { draggedTabId, dragOverTabId, beginDrag, guardClick } = sharedDrag ?? ownDrag;
  const activeTabRef = useRef<HTMLButtonElement | null>(null);
  const stripRef = useRef<HTMLDivElement | null>(null);

  // The empty part of the band is a titlebar: drag moves the window,
  // double-click zooms. Done with pointer events and IPC, NOT by giving the
  // leftover space a `drag` app-region — see the hook, and the note on the
  // scroll box below for the race that rules the app-region version out.
  const bandGesture = useTabBandWindowGesture();

  // #37 — FLIP: tabs SLIDE to their new slots with the spring easing instead
  // of teleporting. Reorder is a pure array move in the reducer, so React
  // re-renders the strip with the tabs already in their final DOM positions;
  // this layout effect measures where each tab WAS (last snapshot), applies
  // the inverted translate, and lets a transition play it back to identity.
  // It keys on the rendered ORDER, so one pass covers drag-reorder, the shift
  // after a close, and a cross-group move — with no animation library.
  //
  // offsetLeft, not getBoundingClientRect: it is layout-relative, so a scrolled
  // strip or a moved window cannot poison the deltas between snapshots. In
  // jsdom every offset is 0, so the pass is a structural no-op there — the
  // spring itself is browser-verified (the repo's Prism/Tailwind lesson).
  const tabOffsetsRef = useRef<Map<string, number>>(new Map());
  const orderKey = tabs.map((t) => t.tabId).join('␟');

  useLayoutEffect(() => {
    const strip = stripRef.current;
    if (!strip) return;
    const previous = tabOffsetsRef.current;
    const next = new Map<string, number>();
    const els = Array.from(strip.querySelectorAll<HTMLElement>('[data-tab-id]'));
    for (const el of els) {
      if (el.dataset.tabId) next.set(el.dataset.tabId, el.offsetLeft);
    }
    tabOffsetsRef.current = next;

    if (prefersReducedMotion()) return;

    // Everything this pass schedules is retained so the cleanup can undo it:
    // an interrupted animation (unmount, or another reorder mid-slide) must
    // not leave a stale rAF callback scheduled or an inline transition
    // override shadowing the stylesheet's hover colours forever.
    const rafIds: number[] = [];
    const restores: Array<() => void> = [];

    for (const el of els) {
      const tabId = el.dataset.tabId;
      if (!tabId) continue;
      const before = previous.get(tabId);
      // A tab with no previous slot is newly opened — it appears in place
      // rather than sliding in from nowhere.
      if (before === undefined) continue;
      const delta = before - (next.get(tabId) ?? before);
      if (delta === 0) continue;

      // Invert without transitioning, then release to identity on the next
      // frame with the spring. The offset rides the individual `translate`
      // property, NOT the `transform` list: the br-tab-select settle animates
      // the individual `scale`, and per CSS Transforms 2 the CTM composes
      // translate → rotate → scale → transform — so a translateX() in
      // `transform` sits INSIDE the scale and gets shrunk with it (the active
      // successor would start off-slot while settling from 0.97), while the
      // individual `translate` composes OUTSIDE it and the slide stays exact
      // (Codex B6 re-review finding 4). Never left/top, and never a reparent —
      // the divider (::before) and drop hairline (::after) contracts stay
      // untouched.
      el.style.transition = 'none';
      el.style.translate = `${delta}px`;

      // Named + idempotent: reached from transitionend, transitioncancel AND
      // the effect cleanup, whichever comes first.
      const restore = () => {
        // Give the stylesheet back its own transition (hover colours).
        el.style.removeProperty('transition');
        el.style.removeProperty('translate');
        el.removeEventListener('transitionend', onDone);
        el.removeEventListener('transitioncancel', onDone);
      };
      const onDone = (event: Event) => {
        // Only OUR translate transition on THIS element: transitionend
        // bubbles, so a child's opacity fade (the close control) must not cut
        // the slide short. propertyName is best-effort — jsdom's synthetic
        // events omit it.
        if (event.target !== el) return;
        const propertyName = (event as TransitionEvent).propertyName;
        if (propertyName && propertyName !== 'translate') return;
        restore();
      };
      restores.push(restore);

      rafIds.push(
        window.requestAnimationFrame(() => {
          if (!el.isConnected) return;
          el.style.transition = 'translate var(--motion-base) var(--ease-spring)';
          el.style.translate = '';
          el.addEventListener('transitionend', onDone);
          el.addEventListener('transitioncancel', onDone);
        })
      );
    }

    return () => {
      for (const id of rafIds) window.cancelAnimationFrame(id);
      for (const restore of restores) restore();
    };
    // Keyed on the rendered order string — the DOM is the dependency here.
  }, [orderKey]);

  // Keep the focused tab in view as the strip scrolls past its shrink floor.
  useEffect(() => {
    // Feature-detected: scrollIntoView is absent in jsdom, and keeping the
    // focused tab visible must never be able to take the strip down.
    activeTabRef.current?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
  }, [activeTabId, tabs.length]);

  /**
   * Rung 3 of the yield ladder (D-32): shrink to the TAB_MIN_WIDTH floor (the
   * `--tab-min-width` token), then scroll,
   * then collapse into a ▾ menu — never wrap.
   *
   * The first two steps are pure CSS (`.br-tabstrip`: flex-wrap: nowrap;
   * overflow-x: auto) and already shipped. This is the last one: once tabs are
   * scrolled out of sight, the ▾ is how you reach them without scrubbing. The
   * measurement is a SHARED hook because the preview panel runs the same tab
   * language and must not have to re-derive the rule (card Z).
   */
  const showOverflowMenu = useTabStripOverflow(stripRef, tabs.length);

  const handleOverflowSelect = useCallback(
    (tabId: ChatTabId) => {
      // Selecting scrolls it into view through the effect above — the menu is a
      // way to REACH a scrolled-out tab, so landing on it without showing it
      // would be half an answer.
      onSelect(tabId);
    },
    [onSelect]
  );

  // getSessionTitlePadding moved here from BaseChat: the strip is now its only
  // consumer. The reserve is what stops the tabs sliding under the macOS traffic
  // lights when the sidebar is collapsed — and when it fails, it fails SILENTLY.
  // Named for what it IS, not how it is applied: it is applied as a MARGIN, so
  // that it sits outside this element's draggable border box (#74).
  const leftReserve = getSessionTitlePadding(isCompactSidebarOverlayOpen, reserveTitlebar);

  const handleKeyDown = (event: React.KeyboardEvent, index: number) => {
    // Roving tabindex: both tab surfaces have promised role="tablist" and never
    // delivered arrow-key navigation. This one does.
    if (event.key !== 'ArrowRight' && event.key !== 'ArrowLeft') return;
    event.preventDefault();
    const delta = event.key === 'ArrowRight' ? 1 : -1;
    const next = tabs[(index + delta + tabs.length) % tabs.length];
    if (next) onSelect(next.tabId);
  };

  return (
    // The wrap exists so the ▾ can sit OUTSIDE the strip's scroll box. Inside it
    // — even pinned with position: sticky — the button would occupy the strip's
    // own scrollWidth and keep alive the very overflow that summoned it: a latch
    // that never lets go. Outside, showing it only narrows clientWidth (an
    // overflowing strip overflows more) and hiding it only widens clientWidth (a
    // fitting strip fits better). Both directions are monotone, so the two states
    // cannot chase each other and the rung needs no hysteresis. See
    // shouldShowTabOverflowMenu.
    <div
      className="br-tabstrip-wrap flex h-full min-w-0 flex-1 items-center"
      data-testid="chat-tab-strip-reserve"
      // THE RESERVE LIVES HERE, OUTSIDE THE SCROLL BOX — and that is the whole
      // point. It was padding-left on the strip itself, which is the scrolling
      // element: padding is part of the scrollable content, so the moment the
      // strip scrolled (selecting any tab scrolls it into view — see
      // handleSelect) the reserve travelled left with it and the tabs rode up
      // under the traffic lights and the floating controls. Measured at 800px
      // with 11 tabs: scrollLeft 428 put a tab at x=17, beneath lights that end
      // at x=72 and controls that span 100–164.
      //
      // The unit test passed throughout, because it asserted the property was
      // SET. It was set. It just could not hold: a reserve inside the thing that
      // moves is not a reserve. On the wrap it is a fixed gutter — the scroll
      // box now BEGINS at 172px and its overflow clips there, so no scroll
      // offset can ever carry a tab across it.
      //
      // MARGIN, NOT PADDING, AND THAT IS LOAD-BEARING (#74). This element is
      // `-webkit-app-region: drag`. Padding lives INSIDE the border box, so
      // with the reserve as padding the drag rect still began at x=0 and
      // covered the very titlebar controls the reserve exists to protect —
      // the OS ate every click on them, because Electron folds app-region
      // rects in TREE order (a later `drag` re-covers an earlier `no-drag`,
      // z-index irrelevant) and this wrap is mounted after them. A margin is
      // OUTSIDE the border box, so the drag rect now starts at the reserve
      // and the tabs land in exactly the same place as before.
      //
      // Both rects in this row had to move: measured with real CGEventPost
      // input, the header's rect and this one were EACH sufficient on their
      // own to kill the buttons, so fixing only one changed nothing. See the
      // matching note in BaseChat.tsx.
      //
      // Do not "simplify" this back to padding, and do not try to remove a
      // rect from the fold by writing `-webkit-app-region: none` — measured,
      // an explicitly specified `none` computes to `no-drag` in Blink (it
      // subtracts a rect); only an ABSENT declaration is truly absent.
      //
      // paddingRight IS the window-drag handle, and unlike the reserve it is
      // deliberately INSIDE this drag box: the scroll box below is `no-drag`
      // edge to edge, so this gutter is the only part of the band left that can
      // move the window. See TAB_BAND_DRAG_GUTTER.
      style={
        {
          marginLeft: leftReserve,
          paddingRight: TAB_BAND_DRAG_GUTTER,
          WebkitAppRegion: 'drag',
        } as CSSProperties
      }
    >
      <div
        role="tablist"
        aria-label="Open chats"
        data-testid="chat-tab-strip"
        ref={stripRef}
        // The drag hit-tests THIS before it hit-tests the group's zones: a strip
        // sits entirely inside its group's `top` edge band, so routing it through
        // zoneFromRect would read every in-strip drag as "split upward" and
        // reorder would be unreachable. See useTabDragReorder's move handler.
        data-tab-strip-group={groupId}
        data-group-active={groupActive ? 'true' : 'false'}
        className="br-tabstrip br-tabstrip--inline h-full min-w-0 flex-1"
        // ONE no-drag rect for the whole strip, and it is STATIC. This element
        // is the last app-region declaration over the tabs, so it is the one
        // that decides every point inside the scroll box, and its box is a
        // function of the header — never of the tab list.
        //
        // ⚠ IT USED TO BE `drag`, WITH THE no-drag ON EACH TAB, AND THAT IS A
        // RACE. Blink recollects app-region rects during a paint lifecycle and
        // ships them to the browser process over IPC; until that lands, macOS
        // routes with the PREVIOUS set. Per-tab rects mean the set changes
        // every time a tab is opened, closed, torn off or merged in — so for
        // the width of that gap a brand-new tab still sits inside the strip's
        // stale `drag` rect and a press on it never reaches the renderer at
        // all: the OS reads it as a titlebar grab and the WINDOW moves. It is
        // not theory. Measured against the running app with CGEventPost input
        // by sweeping the cursor across the second tab's x-range (Electron's
        // DragRegionView swallows every event over a `drag` rect, so an arrival
        // IS the region read-out): settled with one tab, 0/11 sample points in
        // that range arrived; settled with two, 11/11; and with the renderer
        // busy for 2.5s starting the task after the tab was created — which is
        // what mounting a chat does — 1/11, for the whole 2.5s, while the DOM
        // had the tab there the entire time.
        //
        // A timer would only pick a number and hope. The fix is that there is
        // nothing left to recompute: this rect is identical for zero tabs and
        // for twenty, so no tab-list change can ever make the OS's copy wrong.
        //
        // Two consequences, both wanted:
        //   * the scroll box no longer moves the window. The band keeps a
        //     static handle at its right end instead (TAB_BAND_DRAG_GUTTER).
        //   * a window with ONE tab now delivers the press instead of moving
        //     the window, which is what tabTearOffBridge's countWindowTabs
        //     backstop was written for. Tearing that tab out is still refused
        //     (D5, in main), and merging it into another window — which the OS
        //     used to make unreachable — now works (D6a).
        //
        // Nothing inside this box may declare `drag`: it would fold LATER and
        // re-cover the tabs, which is #74 in miniature.
        //
        // paddingLeft: 0 cancels `.br-tabstrip`'s `padding: 0 8px` on the left
        // only, so the reserve on the wrap is the whole left inset and the first
        // tab still lands exactly on it (172px, or 16px unreserved) rather than
        // 8px further right. The right half of the shorthand is untouched.
        style={{ paddingLeft: 0, WebkitAppRegion: 'no-drag' } as CSSProperties}
        // THE EMPTY BAND IS A TITLEBAR — and this is how, given that it cannot
        // be an app region. The two handlers that START something —
        // `onPointerDown` and `onDoubleClick` — are gated on
        // `target === currentTarget` inside the hook, so only the background of
        // the scroll box (and the 3px gaps between tabs) begins a gesture: a
        // press on a tab, its close control or the end slot has a deeper target
        // and is untouched. `beginDrag` on the tab still owns tab drags, and it
        // never sees these events.
        //
        // The three that END a press are deliberately NOT gated. A release has
        // to close an open drag wherever it lands — the strip may have
        // re-rendered under the gesture, and main cannot see the button come up
        // — and ending a drag that was never started is a no-op.
        //
        // Nothing here declares an app region, which is the whole point: the
        // `no-drag` box above stays identical for zero tabs and for twenty, so
        // this adds no rect for the browser process to learn about a lifecycle
        // late. See useTabBandWindowGesture.
        onPointerDown={bandGesture.onPointerDown}
        onPointerUp={bandGesture.onPointerUp}
        onPointerCancel={bandGesture.onPointerCancel}
        onLostPointerCapture={bandGesture.onLostPointerCapture}
        onDoubleClick={bandGesture.onDoubleClick}
      >
        {tabs.map((tab, index) => {
          const isActive = tab.tabId === activeTabId;
          const isRunning = runningSessionIds.includes(tab.sessionId);
          return (
            <div
              key={tab.tabId}
              data-tab-id={tab.tabId}
              data-active={isActive ? 'true' : undefined}
              data-dragging={draggedTabId === tab.tabId ? 'true' : undefined}
              // One attribute, two sources: this window's own drag, or a merge
              // arriving from another window. They are mutually exclusive in
              // time — a cross-window drag delivers no pointer events here, so
              // `dragOverTabId` is null for its whole duration.
              data-dropbefore={
                dragOverTabId === tab.tabId || remoteDropBeforeTabId === tab.tabId
                  ? 'true'
                  : undefined
              }
              // ⚠ NO app-region declaration here, deliberately. A per-tab
              // `no-drag` is a rect that appears and moves with the tab list,
              // and that is exactly the race the strip's own no-drag closes —
              // see the note on the scroll box. It is already covered.
              className={cn('br-tab group')}
            >
              {/* #114: parity with History and Recents. `beginDrag` already
                  ignores anything but button 0, so a right-click cannot start a
                  tab drag — that guard is what makes a context menu safe here.

                  ⚠ **The trigger goes on the LABEL BUTTON, not on `.br-tab`.**
                  `asChild` on the wrapper would merge a handler onto the div
                  that owns the drag gesture and the drop-target attributes, and
                  the note above is explicit that nothing new may be declared on
                  that element. `tab.cwd` is a starting hint only — the new
                  window resumes `tab.sessionId`, whose record carries the real
                  directory — so an empty one is passed through rather than
                  guessed at. */}
              <ContextMenu>
                <ContextMenuTrigger asChild>
                  <button
                    ref={isActive ? activeTabRef : undefined}
                    type="button"
                    role="tab"
                    aria-selected={isActive}
                    tabIndex={isActive ? 0 : -1}
                    title={tab.title}
                    // Astryx §4.3: ONE icon-label gap across the app, 8px. The 7px
                    // here was the last arbitrary one in the strip — a document tab
                    // and a nav row now measure the same distance from glyph to
                    // word. `gap-2` is the token, not a magic number.
                    className="flex min-w-0 flex-1 items-center gap-2 bg-transparent text-left"
                    onPointerDown={(event) => beginDrag(event, tab.tabId, tab.title, groupId)}
                    onKeyDown={(event) => handleKeyDown(event, index)}
                    onClick={() => {
                      // Swallow the synthetic click that ends a drag — otherwise
                      // dropping a tab also activates whatever you dropped it on.
                      if (guardClick()) return;
                      onSelect(tab.tabId);
                    }}
                  >
                    {/* ⚠ The leading glyph now carries BOTH what the tab is and
                    whether it is private, replacing an identical-for-every-tab
                    bubble plus a separate dense privacy dot.

                    It keeps the two properties the dot was given for: no
                    animation, and no `group-hover:hidden` rule. The running dot
                    hides on hover so the close control can take its slot; a
                    privacy marker that disappears exactly when you point at the
                    tab is not a privacy marker. Living at the OTHER END of the
                    tab from the running dot is what lets a running private chat
                    show both at once instead of the two fighting over one slot.

                    The `sub` text chip is gone with it — a sub-agent now reads
                    as a robot glyph in the same slot as every other kind,
                    instead of a word competing with the title for width. */}
                    <ChatKindIcon
                      session={tabKindSource(tab, tabAnnotations[tab.sessionId])}
                      tier={privacyTiers[tab.sessionId]}
                      className="h-4 w-4 br-tab__privacy-dot"
                    />
                    <span className="br-tab__label">{tab.title}</span>
                  </button>
                </ContextMenuTrigger>
                <ChatRowContextMenuContent
                  target={{
                    sessionId: tab.sessionId,
                    workingDir: tab.cwd,
                    openInNewTab: () => onSelect(tab.tabId),
                  }}
                />
              </ContextMenu>

              {isRunning ? (
                // Inactive tabs swap the pulse for Close on hover. The active
                // tab keeps its pulse and reserves Close beside it: selecting a
                // running chat leaves the pointer over that tab, and hiding the
                // pulse there would make its running state disappear exactly
                // when the user opens it.
                <>
                  <span
                    className={cn('br-tab__dot', !isActive && 'group-hover:hidden')}
                    data-testid={`chat-tab-running-${tab.tabId}`}
                    aria-label="Running"
                    role="img"
                  />
                  <button
                    type="button"
                    aria-label={`Close ${tab.title}`}
                    data-testid={`chat-tab-close-${tab.tabId}`}
                    className={cn(
                      'flex-none rounded-sm p-0.5 text-text-muted opacity-0 hover:bg-background-medium hover:text-text-default focus-visible:opacity-100 group-hover:opacity-100'
                    )}
                    onPointerDown={(event) => event.stopPropagation()}
                    onClick={(event) => {
                      event.stopPropagation();
                      onClose(tab.tabId);
                    }}
                  >
                    <X className="h-4 w-4" />
                  </button>
                </>
              ) : (
                <button
                  type="button"
                  aria-label={`Close ${tab.title}`}
                  data-testid={`chat-tab-close-${tab.tabId}`}
                  className="flex-none rounded-sm p-0.5 text-text-muted opacity-0 transition-opacity hover:bg-background-medium hover:text-text-default focus-visible:opacity-100 group-hover:opacity-100"
                  onPointerDown={(event) => event.stopPropagation()}
                  onClick={(event) => {
                    event.stopPropagation();
                    onClose(tab.tabId);
                  }}
                >
                  <X className="h-4 w-4" />
                </button>
              )}
            </div>
          );
        })}
        {/* No app-region here either: this sits inside the scroll box, whose
            no-drag already covers it, and its x moves with the tab list. */}
        {endSlot ? <div className="flex-none">{endSlot}</div> : null}
      </div>

      {showOverflowMenu && (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              aria-label="Show all chats"
              data-testid="chat-tab-overflow-trigger"
              className="br-tabstrip__overflow"
              // This one DOES need its own no-drag: it sits outside the scroll
              // box, inside the wrap's drag rect. Its position is static (last
              // flex item before the gutter, so it hangs off the wrap's content
              // edge) but its EXISTENCE follows the tab count, so the frame in
              // which it first appears is still routed by the previous region
              // set and one click on it can be eaten. That is a single button
              // arriving at an overflow threshold and it self-heals on the next
              // paint — not the same animal as a tab press moving the window.
              style={{ WebkitAppRegion: 'no-drag' } as CSSProperties}
            >
              <ChevronDown className="h-4 w-4" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="max-h-[60vh] w-56 overflow-y-auto">
            {tabs.map((tab) => (
              <DropdownMenuItem
                key={tab.tabId}
                data-testid={`chat-tab-overflow-item-${tab.tabId}`}
                onSelect={() => handleOverflowSelect(tab.tabId)}
                className={cn('gap-2', tab.tabId === activeTabId && 'font-medium')}
              >
                {/* The overflow menu draws the SAME glyph as the strip: a tab
                    that scrolls out of view must not change what it is. */}
                <ChatKindIcon
                  session={tabKindSource(tab, tabAnnotations[tab.sessionId])}
                  tier={privacyTiers[tab.sessionId]}
                  className="h-4 w-4"
                />
                <span className="min-w-0 flex-1 truncate">{tab.title}</span>
                {runningSessionIds.includes(tab.sessionId) ? (
                  // The same glyph the tab carries. One glyph, one meaning — a
                  // second vocabulary for "running" inside the menu would be a
                  // third answer to a question the strip already answers.
                  <span className="br-tab__dot flex-none" aria-label="Running" role="img" />
                ) : null}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      )}
    </div>
  );
}

export default ChatTabStrip;
