import React from 'react';
import { Outlet, useNavigate, useLocation } from 'react-router-dom';
import AppSidebar from '../BioRouterSidebar/AppSidebar';
import { View, ViewOptions } from '../../utils/navigationUtils';
import { useNavigation } from '../../hooks/useNavigation';
import { Sidebar, SidebarInset, SidebarProvider, useSidebar } from '../ui/sidebar';
import { getInitialWorkingDir } from '../../utils/workingDir';
import DependencySetupModal from '../DependencySetupModal';
import ExtensionUpdateReporter from '../ExtensionUpdateReporter';
import {
  getTitlebarControlReserve,
  TitlebarControls,
  TITLEBAR_CONTROL_RESERVE_PROPERTY,
} from './TitlebarControls';
import { SIDEBAR_COMPACT_WIDTH } from './yieldLadder';

// Rung 1's threshold. Imported, not re-declared: this is the same number the
// titlebar reserve and the chat's compact title key off, and a local copy that
// drifted would desynchronise the ladder from the chrome with nothing failing.
const SIDEBAR_AUTO_COLLAPSE_WIDTH = SIDEBAR_COMPACT_WIDTH;

/**
 * What the width watcher should do to the sidebar, given a width sample.
 *
 * Pure and exported so the rule can be tested without geometry: the bug this
 * encodes was a STATE bug (the effect fighting the user's own click), not a
 * layout one, so it is provable in a unit test.
 *
 * `wasCompact` is the side of the threshold we were on last sample, or null if
 * we have never sampled. Anything other than a CROSSING returns 'none' — that
 * is the whole point: if the width did not change buckets, the sidebar is in
 * the state the user asked for and we must not touch it.
 */
export type SidebarAutoCollapseAction = 'collapse' | 'restore' | 'none';

export function sidebarAutoCollapseAction(opts: {
  wasCompact: boolean | null;
  isCompact: boolean;
  open: boolean;
  autoCollapsed: boolean;
}): SidebarAutoCollapseAction {
  const { wasCompact, isCompact, open, autoCollapsed } = opts;
  // Not a crossing → the user's state stands.
  if (wasCompact === isCompact) return 'none';
  // Crossed INTO compact: only take the sidebar away if it is actually showing.
  if (isCompact) return open ? 'collapse' : 'none';
  // Crossed OUT of compact: only give it back if WE were the ones who took it.
  return autoCollapsed ? 'restore' : 'none';
}

/**
 * The routes whose own top band holds interactive controls where the 32px titlebar drag strip
 * lies: the chat (`/`, `/pair`) and Crew (`/crew`, whose workspace switcher, channel header and
 * pane header sit in that band). On them `biorouter-chat-route-active` makes the strip stop
 * taking pointer events (`main.css`), so it cannot take the clicks meant for those controls
 * (issue #74). Crew's bands declare no drag region of their own; the real-app check with the
 * sidebar open and collapsed is the only proof, because jsdom sees no drag rects.
 */
export function isChatRoute(pathname: string): boolean {
  return (
    pathname === '/' ||
    pathname === '/pair' ||
    pathname === '/crew' ||
    pathname.startsWith('/crew/')
  );
}

/** Keeps `biorouter-chat-route-active` on `<body>` exactly while a chat route is showing. */
export function useChatRouteBodyClass(pathname: string): void {
  React.useEffect(() => {
    document.body.classList.toggle('biorouter-chat-route-active', isChatRoute(pathname));
    return () => document.body.classList.remove('biorouter-chat-route-active');
  }, [pathname]);
}

const AppLayoutContent: React.FC = () => {
  const navigate = useNavigate();
  const location = useLocation();
  const safeIsMacOS = (window?.electron?.platform || 'darwin') === 'darwin';
  const { isMobile, open, openMobile, setOpen } = useSidebar();
  const autoCollapsedSidebarRef = React.useRef(false);
  const resizeSettlingTimerRef = React.useRef<number | null>(null);
  // `open` is read by the width watcher but must NOT be one of its deps — see
  // the comment on that effect. A ref keeps the watcher's view of `open` fresh
  // without re-running it every time the user toggles the sidebar.
  const sidebarOpenRef = React.useRef(open);
  sidebarOpenRef.current = open;
  // Which side of SIDEBAR_AUTO_COLLAPSE_WIDTH we were on last time the watcher
  // ran. `null` = never measured, so the first run counts as a crossing.
  const wasCompactRef = React.useRef<boolean | null>(null);

  // Hide buttons when mobile sheet is showing
  const shouldHideButtons = isMobile && openMobile;

  useChatRouteBodyClass(location.pathname);

  /**
   * Auto-collapse the sidebar when the window gets too narrow, and restore it
   * when there is room again.
   *
   * This reacts ONLY to a WIDTH CROSSING — never to `open` itself. The previous
   * version listed `open` in its deps, which turned it into a reconciliation
   * that re-ran on every toggle, and that made the un-collapse button dead:
   *
   *   1. wide window, user collapses the sidebar by hand  → open=false, and the
   *      auto-collapse branch never ran, so autoCollapsedSidebarRef stayed FALSE
   *   2. user shrinks the window under the threshold → `shouldCollapse && open`
   *      is false (it is already closed), so the ref STILL stays false
   *   3. user clicks the toggle to bring it back → open=true re-runs this effect
   *      → `shouldCollapse && open && !ref` is now true → setOpen(false)
   *
   * The sidebar snapped shut inside the same tick as the click, so the button
   * looked broken and the rail was unrecoverable until you clicked a second
   * time. Gating on a crossing means a user toggle can never re-arm this.
   */
  React.useEffect(() => {
    const updateSidebarMode = () => {
      const shouldCollapse = window.innerWidth < SIDEBAR_AUTO_COLLAPSE_WIDTH;
      document.body.classList.toggle('biorouter-sidebar-compact', shouldCollapse && !isMobile);

      const action = sidebarAutoCollapseAction({
        wasCompact: wasCompactRef.current,
        isCompact: shouldCollapse,
        open: sidebarOpenRef.current,
        autoCollapsed: autoCollapsedSidebarRef.current,
      });
      wasCompactRef.current = shouldCollapse;

      if (action === 'collapse') {
        // Remember that WE took the sidebar away, so we know to give it back.
        autoCollapsedSidebarRef.current = true;
        setOpen(false);
      } else if (action === 'restore') {
        autoCollapsedSidebarRef.current = false;
        setOpen(true);
      }
    };

    updateSidebarMode();
    window.addEventListener('resize', updateSidebarMode);
    return () => {
      document.body.classList.remove('biorouter-sidebar-compact');
      window.removeEventListener('resize', updateSidebarMode);
    };
  }, [isMobile, setOpen]);

  React.useEffect(() => {
    const markWindowResizing = () => {
      document.body.classList.add('biorouter-window-resizing');
      if (resizeSettlingTimerRef.current !== null) {
        window.clearTimeout(resizeSettlingTimerRef.current);
      }
      resizeSettlingTimerRef.current = window.setTimeout(() => {
        resizeSettlingTimerRef.current = null;
        document.body.classList.remove('biorouter-window-resizing');
      }, 180);
    };

    window.addEventListener('resize', markWindowResizing);
    return () => {
      window.removeEventListener('resize', markWindowResizing);
      if (resizeSettlingTimerRef.current !== null) {
        window.clearTimeout(resizeSettlingTimerRef.current);
      }
      document.body.classList.remove('biorouter-window-resizing');
    };
  }, []);

  // Keep every top-level route on the shared navigation mapping.
  const navigateToView = useNavigation();
  const setView = (view: View, viewOptions?: ViewOptions) => navigateToView(view, viewOptions);

  const handleSelectSession = async (sessionId: string) => {
    // Navigate to chat with session data
    navigate('/', { state: { sessionId } });
  };

  const handleNewWindow = () => {
    window.electron.createChatWindow(undefined, getInitialWorkingDir());
  };

  return (
    <div
      className="relative flex w-full flex-1 animate-fade-in"
      style={
        {
          [TITLEBAR_CONTROL_RESERVE_PROPERTY]: `${getTitlebarControlReserve(safeIsMacOS)}px`,
        } as React.CSSProperties
      }
    >
      <TitlebarControls
        hidden={shouldHideButtons}
        isMacOS={safeIsMacOS}
        onNewWindow={handleNewWindow}
      />
      <Sidebar variant="inset" collapsible="offcanvas">
        <AppSidebar
          onSelectSession={handleSelectSession}
          setView={setView}
          currentPath={location.pathname}
        />
      </Sidebar>
      <SidebarInset>
        <main
          className="route-container biorouter-route-surface relative z-[60] flex h-full min-h-0 min-w-0 flex-1 flex-col"
          data-route-path={location.pathname}
        >
          <Outlet />
        </main>
      </SidebarInset>
    </div>
  );
};

export const AppLayout: React.FC = () => {
  return (
    <SidebarProvider>
      <AppLayoutContent />
      <DependencySetupModal />
      <ExtensionUpdateReporter />
    </SidebarProvider>
  );
};
