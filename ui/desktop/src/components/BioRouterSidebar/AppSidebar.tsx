import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { ChevronDown, Home, Plus, Settings } from '../icons/app-icons';
import { ENTITY_ICONS } from '../icons/entity-icons';
import { useNavigate, useSearchParams } from 'react-router-dom';
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
import { cn } from '../../utils';
import { ViewOptions, View, navigateWithViewTransition } from '../../utils/navigationUtils';
import { useChatContext } from '../../contexts/ChatContext';
import { DEFAULT_CHAT_TITLE } from '../../contexts/ChatContext';
import EnvironmentBadge from './EnvironmentBadge';
import { useRunningChats } from '../../hooks/chatStreamStore';
import { preloadSessionList } from '../../utils/sessionListCache';
import { preloadHomeActivity } from '../../utils/homeInsightsCache';
import SidebarUpdateButton from './SidebarUpdateButton';
import RecentChats from './RecentChats';
import useSidebarSessions from './useSidebarSessions';

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
  tooltip: string;
}

const settingsItem: NavigationItem = {
  type: 'item',
  path: '/settings',
  label: 'Settings',
  icon: Settings,
  tooltip: 'Configure Biorouter settings',
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
    tooltip: 'Go back to the main chat screen',
  },
  {
    type: 'item',
    path: '/pair',
    label: 'New chat',
    icon: Plus,
    tooltip: 'Start a new chat',
  },
];

/**
 * The six destinations behind the `Components` disclosure.
 *
 * ⚠ `/applications` is labelled "Built apps", by its SOURCE rather than by a
 * bare noun. It used to read "Applications" and sit one word away from a
 * second row, "Apps" (`/apps`), which listed the UI resources installed MCP
 * extensions advertised — a wholly separate feature inherited from the
 * upstream fork and removed in September 2026. Only Agent Drafter's own
 * `GET /apps` remains, so "Built apps" now has nothing to be confused with;
 * the name is kept because it says what the list holds.
 */
const componentItems: NavigationItem[] = [
  {
    type: 'item',
    path: '/workflows',
    label: 'Workflows',
    icon: ENTITY_ICONS.workflow,
    tooltip: 'Browse your saved workflows',
  },
  {
    type: 'item',
    path: '/schedules',
    label: 'Scheduler',
    icon: ENTITY_ICONS.schedule,
    tooltip: 'Manage scheduled runs',
  },
  {
    type: 'item',
    path: '/extensions',
    label: 'Extensions',
    icon: ENTITY_ICONS.extension,
    tooltip: 'Manage your extensions',
  },
  {
    type: 'item' as const,
    path: '/skills',
    label: 'Skills',
    icon: ENTITY_ICONS.skill,
    tooltip: 'Manage reusable instruction skills',
  },
  {
    type: 'item' as const,
    path: '/knowledge',
    label: 'Knowledge',
    icon: ENTITY_ICONS.knowledge,
    tooltip: 'Personal knowledge bases',
  },
  {
    type: 'item' as const,
    path: '/applications',
    label: 'Built apps',
    icon: ENTITY_ICONS.application,
    tooltip: 'Apps you built with Agent Drafter',
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
  const { sessions, hasMore, isLoading, loadMore } = useSidebarSessions();
  const currentSessionId = currentPath === '/pair' ? searchParams.get('resumeSessionId') : null;
  const runningSessionIds = useMemo(
    () =>
      new Set(runningChats.filter((entry) => !entry.completedAt).map((entry) => entry.sessionId)),
    [runningChats]
  );

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
      titleBits.push('Chat history');
    } else if (currentPath !== '/' && currentItem) {
      titleBits.push(currentItem.label);
    }

    document.title = titleBits.join(' - ');
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

    navigateWithViewTransition(navigate, path);
  };

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

  const renderMenuItem = (entry: NavigationItem, options?: { indented?: boolean }) => {
    const IconComponent = entry.icon;
    const isActive = isDestinationActive(entry);
    const isAction = entry.path === '/pair';

    return (
      <SidebarMenuItem key={entry.path}>
        <SidebarMenuButton
          data-testid={`sidebar-${entry.label.toLowerCase().replace(/\s+/g, '-')}-button`}
          onClick={(event) => {
            handleNavigation(entry.path);
            if (isAction) blurAfterPointerActivation(event);
          }}
          onFocus={entry.path === '/' ? preloadHome : undefined}
          onPointerEnter={entry.path === '/' ? preloadHome : undefined}
          isActive={isActive}
          tooltip={entry.tooltip}
          // HIERARCHY BY INDENT, NEVER BY SIZE (§4.1.3). A child of the
          // disclosure keeps the same 32px height and the same type; only its
          // text edge moves, by 24px. Shrinking it would say it is a lesser kind
          // of destination, which it is not.
          className={cn(
            'relative h-8 w-full justify-start rounded-lg py-2 text-sm transition-colors duration-150 before:absolute before:inset-y-2 before:left-0 before:w-0.5 before:bg-transparent hover:bg-sidebar-hover data-[active=true]:bg-sidebar-active data-[active=true]:font-medium data-[active=true]:before:bg-accent-bar',
            options?.indented ? 'pl-9 pr-3' : 'px-3'
          )}
        >
          {/* Icons take --sidebar-icon rather than inheriting the label's ink.
              In Parchment that token passes through to --sidebar-foreground, so
              nothing changes there; Alma Mater points it at UCSF teal, which is
              where the brand actually lives in that theme. */}
          <IconComponent className="h-4 w-4 text-sidebar-icon" />
          <span>{entry.label}</span>
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
            strip live over this space, so the sidebar only reserves it. Its bottom
            hairline continues the chat/preview header hairline, giving the window
            one continuous top edge.
            `-mt-2` cancels SidebarContent's 8px top padding so the band starts at
            the window's top edge (y=0), exactly like the chat/preview header —
            otherwise the band sat 8px low and its hairline fell ~9px below the tab
            strip's, breaking the "one continuous top edge".

            `h-chrome` (44px), not the `h-13` literal it carried: this is one of
            the three bands that had to drop 52 -> 44 TOGETHER, because they share
            a seam and a shrinking band beside a stationary one is a broken edge,
            not a compaction. The traffic lights sit in the 32px drag region above
            and clear it; the wordmark is NOT in this band (it is the row below),
            which is what made the drop safe to take. */}
        <div
          data-testid="sidebar-titlebar-band"
          aria-hidden="true"
          className="-mt-2 h-chrome shrink-0 border-b border-sidebar-border"
        />

        <div className="shrink-0">
          {/* Brand row. The wordmark IS the lockup now — "Bio" navy + "Router"
              coral over the split underline (D-39), replacing the old mono glyph
              + plain "Biorouter" text. Its left edge lands on the same 44px text
              edge as every nav label. The component recolours navy -> UCSF teal
              on a dark surface on its own. */}
          {/* `pt-4 pb-2`, not `pt-2` with nothing below it. When the titlebar band
              dropped 52 -> 44px the wordmark came up with it and landed 8px under
              the hairline with the first nav row directly beneath — three things
              stacked at one rhythm, which reads as crowded rather than dense. The
              brand lockup is not a list item; it earns air on both sides. 16px
              above separates it from the window chrome, 8px below separates it
              from the navigation it is not part of. */}
          <div className="px-2 pt-4 pb-2">
            <div
              data-testid="sidebar-biorouter-wordmark"
              className="flex h-8 items-center gap-2 px-3"
            >
              <BioRouterWordmark
                data-testid="sidebar-biorouter-mark"
                // h-[22px] so the wordmark's cap height reads at the same size as
                // the text-sm nav labels beside it — measured against "New chat"
                // in the calibration harness. (The SVG box includes the underline,
                // so the letters are smaller than the box; 17px looked undersized.)
                className="h-[22px] w-auto shrink-0"
              />
              <EnvironmentBadge />
            </div>
          </div>

          {/* NO "MENU" HEADER (§4.1.2). It was 32px labelling something
              self-evident: a vertical list of destinations at the top of a rail
              does not need to be told apart from anything. The Recents header
              stays, because it labels one list among two. */}
          <SidebarGroup className="px-2 pb-1">
            <SidebarGroupContent>
              {/* 2px between rail rows, not 0. At `gap-0` the rounded washes of
                  adjacent rows touch, so a hovered row bleeds into its neighbours
                  and the list reads as one block of colour rather than as
                  separable destinations. 2px is the same gap the app's own menu
                  recipe uses between items, so the rail and the menus agree. */}
              <SidebarMenu className="gap-0.5">
                {primaryItems.map((entry) => renderMenuItem(entry))}

                {/* The disclosure row: an item's twin, with a leading chevron in
                    place of an icon and no uppercase mini-label. It is a control,
                    not a destination, so it never takes the selected wash — even
                    when one of its children is the current route. The child's own
                    wash says where you are. */}
                <SidebarMenuItem>
                  <SidebarMenuButton
                    data-testid="sidebar-components-disclosure"
                    aria-expanded={showComponentChildren}
                    aria-controls="sidebar-components-group"
                    onClick={toggleComponents}
                    className="relative h-8 w-full justify-start rounded-lg px-3 py-2 text-sm transition-colors duration-150 hover:bg-sidebar-hover"
                  >
                    <ChevronDown
                      aria-hidden="true"
                      className={cn(
                        'h-4 w-4 text-sidebar-icon transition-transform duration-150',
                        showComponentChildren ? '' : '-rotate-90'
                      )}
                    />
                    <span>Components</span>
                  </SidebarMenuButton>
                </SidebarMenuItem>

                {showComponentChildren && (
                  <div id="sidebar-components-group" data-testid="sidebar-components-group">
                    {componentItems.map((entry) => renderMenuItem(entry, { indented: true }))}
                  </div>
                )}
              </SidebarMenu>
            </SidebarGroupContent>
          </SidebarGroup>
        </div>

        {/* THE UPPER DIVIDER IS GONE (§4.1.4). The Components row and the
            Recents header now do the zoning between the rail's destinations and
            its history, so a rule between them was a third answer to a question
            two elements already answered. */}
        <RecentChats
          sessions={sessions}
          activeSessionId={currentSessionId}
          runningSessionIds={runningSessionIds}
          hasMore={hasMore}
          isLoadingMore={isLoading}
          onLoadMore={loadMore}
          onOpen={handleOpenChat}
          onViewAll={() => navigateWithViewTransition(navigate, '/sessions')}
        />
      </SidebarContent>

      {/* The rail's ONE rule, and now it is unambiguous: it separates the
          scrolling history from the fixed footer. Halved to a 10px block
          (§4.1.4) — `my-1` + the hairline — because at `my-2` it was 18px of
          rail spent on a 1px mark. */}
      <div
        data-testid="sidebar-footer-divider"
        role="separator"
        className="mx-3.5 my-1 h-px shrink-0 bg-sidebar-border"
      />
      <SidebarFooter className="gap-1 p-2">
        <SidebarUpdateButton />
        <SidebarMenu className="gap-0">
          <SidebarMenuItem>
            <SidebarMenuButton
              data-testid="sidebar-settings-button"
              onClick={() => handleNavigation(settingsItem.path)}
              isActive={isActivePath(settingsItem.path)}
              tooltip={settingsItem.tooltip}
              className="relative h-8 w-full justify-start rounded-lg px-3 py-2 text-sm transition-colors duration-150 before:absolute before:inset-y-2 before:left-0 before:w-0.5 before:bg-transparent hover:bg-sidebar-hover data-[active=true]:bg-sidebar-active data-[active=true]:font-medium data-[active=true]:before:bg-accent-bar"
            >
              <Settings className="h-4 w-4" />
              <span>{settingsItem.label}</span>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarFooter>
    </>
  );
};

export default AppSidebar;
