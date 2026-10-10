import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { ChevronRight, Home, NewChat, Settings } from '../icons/app-icons';
import { ENTITY_ICONS } from '../icons/entity-icons';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { announceSameRouteReset } from '../../hooks/useSameRouteReset';
import {
  SidebarContent,
  SidebarFooter,
  SidebarMenu,
  SidebarMenuItem,
  SidebarMenuButton,
  SidebarGroup,
  SidebarGroupContent,
} from '../ui/sidebar';
import { BioRouterWordmark } from '../icons/BioRouterWordmark';
import { ViewOptions, View, navigateWithViewTransition } from '../../utils/navigationUtils';
import { useChatContext } from '../../contexts/ChatContext';
import { DEFAULT_CHAT_TITLE } from '../../contexts/ChatContext';
import EnvironmentBadge from './EnvironmentBadge';
import { useRunningChats } from '../../hooks/chatStreamStore';
import { preloadSessionList } from '../../utils/sessionListCache';
import { preloadHomeActivity } from '../../utils/homeInsightsCache';
import SidebarUpdateButton from './SidebarUpdateButton';
import DaemonRestartNotice from './DaemonRestartNotice';
import { Badge } from '../ui/badge';
import { attentionBadgeText } from '../crew/attention/crewAttention';
import { useCrewAttention } from '../crew/attention/useCrewAttention';
import RecentChats from './RecentChats';
import useSidebarSessions from './useSidebarSessions';
import { sidebarCopy } from './copy';
import { isDefaultSidebarChatView } from './sidebarChatView';
import { useSidebarChatView } from './useSidebarChatView';
import './sidebar.css';

function preloadHome(): void {
  preloadHomeActivity();
  preloadSessionList();
}

interface SidebarProps {
  onSelectSession: (sessionId: string) => void;
  refreshTrigger?: number;
  children?: React.ReactNode;
  setView?: (view: View, viewOptions?: ViewOptions) => void;
  currentPath?: string;
}

interface NavigationItem {
  type: 'item';
  path: string;
  label: string;
  icon: React.ComponentType<{ className?: string }>;
}

const settingsItem: NavigationItem = {
  type: 'item',
  path: '/settings',
  label: 'Settings',
  icon: Settings,
};

/**
 * THE RAIL CARRIES ONE DESTINATION AND ONE ACTION (Astryx §4.1.3, decision A-08).
 *
 * The audit measured ~462px of fixed chrome above the first recent chat, of
 * which 288px was nine nav rows. On a 720p window more than half the rail was
 * spent before any history showed.
 *
 * **Home** is first, because it is where the rail RETURNS you; **New chat** is
 * beneath it, because it is the one thing the rail DOES. Everything else is a
 * place you go occasionally, and those live behind one `Components` disclosure —
 * one click away, keeping their real icons, indented rather than shrunk.
 *
 * Order is load-bearing and was reversed from what shipped: New chat used to
 * sit above Home, which put an action in the position the eye reads as "the top
 * of the map".
 */
const primaryItems: NavigationItem[] = [
  {
    type: 'item',
    path: '/',
    label: 'Home',
    icon: Home,
  },
  {
    type: 'item',
    path: '/pair',
    label: 'New chat',
    icon: NewChat,
  },
  {
    type: 'item',
    path: '/crew',
    label: 'Crew',
    icon: ENTITY_ICONS.crew,
  },
];

/**
 * The six destinations behind the `Components` disclosure.
 *
 * ⚠ `/applications` is labelled "Built apps", by its SOURCE rather than by a
 * bare noun. It used to read "Applications" and sit one word away from a
 * second row, "Apps" (`/apps`), which listed the UI resources installed MCP
 * extensions advertised — a wholly separate feature, present since the first
 * commit and never requested, that was removed in September 2026. Only Agent
 * Drafter's own `GET /apps` remains, so "Built apps" now has nothing to be
 * confused with; the name is kept because it says what the list holds.
 */
const componentItems: NavigationItem[] = [
  {
    type: 'item',
    path: '/workflows',
    label: 'Workflows',
    icon: ENTITY_ICONS.workflow,
  },
  {
    type: 'item',
    path: '/schedules',
    label: 'Scheduler',
    icon: ENTITY_ICONS.schedule,
  },
  {
    type: 'item',
    path: '/extensions',
    label: 'Extensions',
    icon: ENTITY_ICONS.extension,
  },
  {
    type: 'item' as const,
    path: '/skills',
    label: 'Skills',
    icon: ENTITY_ICONS.skill,
  },
  {
    type: 'item' as const,
    path: '/knowledge',
    label: 'Knowledge',
    icon: ENTITY_ICONS.knowledge,
  },
  {
    type: 'item' as const,
    path: '/applications',
    label: 'Built apps',
    icon: ENTITY_ICONS.application,
  },
];

const COMPONENTS_EXPANDED_STORAGE_KEY = 'biorouter:sidebar-components-expanded';

/**
 * Collapsed by DEFAULT — that is where the 192px comes from (nine 32px nav rows
 * become three). Remembered, so a user who wants the six open never reopens
 * them. Storage failures (private mode, a sandboxed frame) fall back to
 * collapsed rather than throwing, matching what Recents already does one file
 * over.
 */
function readStoredComponentsExpanded(): boolean {
  try {
    return window.localStorage.getItem(COMPONENTS_EXPANDED_STORAGE_KEY) === 'true';
  } catch {
    return false;
  }
}

const AppSidebar: React.FC<SidebarProps> = ({ currentPath }) => {
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const chatContext = useChatContext();
  const runningChats = useRunningChats();
  const [chatView, setChatView] = useSidebarChatView();
  // Any view but the default sorts or groups the whole list, so it reads every
  // page first (spec 3.4 "Paging"); the default keeps lazy 10-row paging.
  const { sessions, hasMore, isLoading, isLoadingAll, loadMore } = useSidebarSessions({
    loadAll: !isDefaultSidebarChatView(chatView),
  });
  const currentSessionId = currentPath === '/pair' ? searchParams.get('resumeSessionId') : null;
  const runningSessionIds = useMemo(
    () =>
      new Set(runningChats.filter((entry) => !entry.completedAt).map((entry) => entry.sessionId)),
    [runningChats]
  );
  const homeDir = (window.appConfig?.get('BIOROUTER_HOME_DIR') as string | undefined) ?? null;
  const isMac = window.electron?.platform === 'darwin';

  useEffect(() => {
    const currentItem = [...primaryItems, ...componentItems, settingsItem].find(
      (item) => item.path === currentPath
    );

    const titleBits = ['Biorouter'];

    if (
      currentPath === '/pair' &&
      chatContext?.chat?.name &&
      chatContext.chat.name !== DEFAULT_CHAT_TITLE
    ) {
      titleBits.push(chatContext.chat.name);
    } else if (currentPath === '/sessions') {
      titleBits.push(sidebarCopy.documentTitle.history);
    } else if (currentPath !== '/' && currentItem) {
      titleBits.push(currentItem.label);
    }

    // A middle dot, not a spaced hyphen used as punctuation (F-26).
    document.title = titleBits.join(sidebarCopy.documentTitle.separator);
  }, [currentPath, chatContext?.chat?.name]);

  const isActivePath = (path: string) => {
    return currentPath === path;
  };

  const handleNavigation = (path: string) => {
    if (path === '/pair') {
      chatContext?.resetChat();
      navigateWithViewTransition(navigate, '/pair', { newChat: true });
      return;
    }

    // Re-selecting the destination you are already on used to do NOTHING.
    // React Router reconciles a same-path navigation rather than remounting
    // the route element, so a page holding sub-state — a schedule's run detail,
    // and the session history nested inside that — stayed exactly where it was
    // and only the in-page Back escaped. The row is lit the whole time, which
    // is this rail telling the user "you are here"; pressing it should get them
    // here, not leave them three levels down with no visible way out.
    if (path === currentPath) {
      announceSameRouteReset(path);
      return;
    }

    navigateWithViewTransition(navigate, path);
  };

  // Crew's unread messages across every connected workspace, and a notification when one
  // arrives while this window is not showing Crew in front (M2). A clicked notification opens
  // Crew, as the Crew item does, with that channel remembered.
  const crewUnread = useCrewAttention({
    onCrewRoute: currentPath === '/crew',
    onOpenChannel: () => {
      if (currentPath !== '/crew') handleNavigation('/crew');
    },
  });

  // A click opens the chat as its own tab; if it is already open anywhere the
  // reducer dedupes and just activates it. This is a cross-route entry, so it
  // keeps today's PUSH navigation — only in-strip tab clicks use replace, which
  // is what keeps Back returning you to wherever you came from INTO /pair.
  // `title` rides along in route state so the tab opens already named. It is
  // only ever a HINT: the URL alone still opens the chat (a deep link, a
  // reload, an external nav carry no state), and BaseChat's load still renames
  // the tab authoritatively. The summary carries `userSetName`, so a literal
  // user-chosen legacy placeholder is never mistaken for an automatic title.
  const handleOpenChat = (sessionId: string, title?: string, userSetName?: boolean) => {
    // NB: the 3rd arg of navigateWithViewTransition IS the route state itself,
    // not an options bag — `{ state: … }` here would nest it one level deep and
    // silently do nothing.
    navigateWithViewTransition(
      navigate,
      `/pair?resumeSessionId=${sessionId}`,
      title ? { title, userSetName } : undefined
    );
  };

  /**
   * NEW SESSION IS AN ACTION, AND ACTIONS DO NOT STAY LIT (Astryx §4.1.3).
   *
   * A destination keeps the selected wash and the accent rail because you are
   * still THERE. New chat fires and the view moves on — leaving a lit row
   * behind claims a location that is no longer true, and it is the reason a rail
   * with one action and one destination read as a rail with two selected states.
   */
  const isDestinationActive = (entry: NavigationItem) =>
    entry.path === '/pair' ? false : isActivePath(entry.path);

  /**
   * Drop focus after a POINTER activation only.
   *
   * `event.detail > 0` is the mouse/touch test: a keyboard `Enter` or `Space`
   * reports 0. Blurring blindly would strand a Tab user mid-rail with no visible
   * focus and nowhere obvious to resume from, so the row that must not stay lit
   * for a mouse must still keep focus for a keyboard.
   */
  const blurAfterPointerActivation = useCallback((event: React.MouseEvent<HTMLElement>) => {
    if (event.detail > 0) event.currentTarget.blur();
  }, []);

  /**
   * One row recipe for every destination (`.br-nav-row`, `sidebar.css`): 28px,
   * 13px, a 16px icon at x=16 and the label at x=40, muted at rest, the accent
   * rail on the current one. The Components children are NOT indented: the
   * chevron above marks the group, as Crew's team headers do (F-11).
   */
  const renderMenuItem = (entry: NavigationItem) => {
    const IconComponent = entry.icon;
    const isActive = isDestinationActive(entry);
    const isAction = entry.path === '/pair';
    const unread = entry.path === '/crew' ? crewUnread : 0;

    return (
      <SidebarMenuItem key={entry.path}>
        <SidebarMenuButton
          data-testid={`sidebar-${entry.label.toLowerCase().replace(/\s+/g, '-')}-button`}
          onClick={(event) => {
            handleNavigation(entry.path);
            if (isAction) blurAfterPointerActivation(event);
          }}
          aria-label={unread > 0 ? `${entry.label}, ${unread} unread` : undefined}
          aria-current={isActive ? 'page' : undefined}
          onFocus={entry.path === '/' ? preloadHome : undefined}
          onPointerEnter={entry.path === '/' ? preloadHome : undefined}
          isActive={isActive}
          className="br-nav-row no-drag"
        >
          {/* Icons take --sidebar-icon rather than inheriting the label's ink.
              In Parchment that token passes through to --sidebar-foreground, so
              nothing changes there; Alma Mater points it at UCSF teal, which is
              where the brand actually lives in that theme. */}
          <IconComponent className="br-nav-row-icon" />
          <span className="br-nav-row-label">{entry.label}</span>
          {isAction ? (
            <span className="br-nav-row-hint" aria-hidden="true">
              {sidebarCopy.shortcut.newChat(isMac)}
            </span>
          ) : null}
          {unread > 0 ? (
            <Badge tone="neutral" className="tabular-nums" aria-hidden="true">
              {attentionBadgeText(unread)}
            </Badge>
          ) : null}
        </SidebarMenuButton>
      </SidebarMenuItem>
    );
  };

  const [isComponentsExpanded, setIsComponentsExpanded] = useState(readStoredComponentsExpanded);
  const toggleComponents = useCallback(() => {
    setIsComponentsExpanded((wasExpanded) => {
      const nextExpanded = !wasExpanded;
      try {
        window.localStorage.setItem(COMPONENTS_EXPANDED_STORAGE_KEY, String(nextExpanded));
      } catch {
        // Persisting is best-effort; the disclosure still opens.
      }
      return nextExpanded;
    });
  }, []);

  // A lit row inside a collapsed section is an invisible one. When the user IS
  // on one of these destinations the group opens regardless of the stored
  // preference — and does NOT overwrite it, so navigating away collapses back to
  // whatever they chose.
  const isOnComponentRoute = componentItems.some((entry) => entry.path === currentPath);
  const showComponentChildren = isComponentsExpanded || isOnComponentRoute;

  return (
    <>
      <SidebarContent className="gap-0 overflow-hidden">
        {/* The titlebar band: traffic lights and the floating TitlebarControls
            strip live over this space, so the sidebar only reserves it. Its
            bottom hairline continues the chat and preview header hairline at
            y=44 and runs the sidebar's full width (the container has no padding
            of its own), giving the window one continuous top edge. `h-chrome`:
            this band, BaseChat's header and the artifact strip read
            `--chrome-height` and move together or not at all. */}
        <div
          data-testid="sidebar-titlebar-band"
          aria-hidden="true"
          className="h-chrome shrink-0 border-b border-sidebar-border"
        />

        {/* Brand row: 8px above and 4px below a 32px row, the wordmark 20px
            tall at the icon column (x=16). The wordmark IS the lockup — "Bio"
            navy + "Router" coral over the split underline (D-39) — and recolours
            navy -> UCSF teal on a dark surface on its own. */}
        <div className="br-sidebar-brand">
          <div data-testid="sidebar-biorouter-wordmark" className="br-sidebar-brand-row">
            <BioRouterWordmark
              data-testid="sidebar-biorouter-mark"
              className="br-sidebar-wordmark"
            />
            <EnvironmentBadge />
          </div>
        </div>

        {/* NO "MENU" HEADER (§4.1.2): a vertical list of destinations at the top
            of a rail does not need to be told apart from anything. */}
        <SidebarGroup className="br-nav-group">
          <SidebarGroupContent>
            <SidebarMenu className="br-nav-list">
              {primaryItems.map((entry) => renderMenuItem(entry))}

              {/* The disclosure row: a group header in Crew's team-toggle recipe,
                  a 16px chevron that turns 90°. It is a control, not a
                  destination, so it never takes the selected wash — even when one
                  of its children is the current route. Collapse is instant. */}
              <SidebarMenuItem>
                <SidebarMenuButton
                  data-testid="sidebar-components-disclosure"
                  aria-expanded={showComponentChildren}
                  aria-controls="sidebar-components-group"
                  onClick={toggleComponents}
                  className="br-nav-row no-drag"
                >
                  <ChevronRight aria-hidden="true" className="br-nav-chevron" />
                  <span className="br-nav-row-label">{sidebarCopy.components}</span>
                </SidebarMenuButton>
              </SidebarMenuItem>

              {showComponentChildren && (
                <div
                  id="sidebar-components-group"
                  data-testid="sidebar-components-group"
                  className="br-nav-list"
                >
                  {componentItems.map((entry) => renderMenuItem(entry))}
                </div>
              )}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>

        <RecentChats
          sessions={sessions}
          activeSessionId={currentSessionId}
          runningSessionIds={runningSessionIds}
          hasMore={hasMore}
          isLoadingMore={isLoading}
          isLoadingAll={isLoadingAll}
          onLoadMore={loadMore}
          onOpen={handleOpenChat}
          onViewAll={() => navigateWithViewTransition(navigate, '/sessions')}
          view={chatView}
          onViewChange={setChatView}
          homeDir={homeDir}
        />
      </SidebarContent>

      {/* The rail's ONE rule: a full-bleed hairline over the fixed footer
          (Crew's `.crew-sidebar-you`), replacing the inset `mx-3.5 my-1` rule. */}
      <SidebarFooter data-testid="sidebar-footer" className="br-sidebar-footer">
        <DaemonRestartNotice />
        <SidebarMenu className="br-nav-list">
          <SidebarUpdateButton />
          <SidebarMenuItem>
            <SidebarMenuButton
              data-testid="sidebar-settings-button"
              onClick={() => handleNavigation(settingsItem.path)}
              isActive={isActivePath(settingsItem.path)}
              aria-current={isActivePath(settingsItem.path) ? 'page' : undefined}
              className="br-nav-row no-drag"
            >
              <Settings className="br-nav-row-icon" />
              <span className="br-nav-row-label">{settingsItem.label}</span>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarFooter>
    </>
  );
};

export default AppSidebar;
