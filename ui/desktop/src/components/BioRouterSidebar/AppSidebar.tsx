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
import { ViewOptions, View, navigateToRoute } from '../../utils/navigationUtils';
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
import useSidebarSessions, { sessionsInTurn } from './useSidebarSessions';
import { sidebarCopy } from './copy';
import { isDefaultSidebarChatView } from './sidebarChatView';
import { useSidebarChatView } from './useSidebarChatView';
import { isBrowserSurface } from '../../utils/surface';
import { hasUsableModel, useDictationStatus } from '../dictation/dictationStore';
import { DICTIONARY_LABELS } from '../dictionary/copy';
import { useDictionaryWaitingCount } from '../dictionary/useDictionary';
import { StatusDot } from '../ui/status-dot';
import { homeFolder } from '../../utils/folderPath';
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

const DICTIONARY_PATH = '/dictionary';

/**
 * The seven destinations behind the `Components` disclosure. **Dictionary**
 * (DD-67) sits after Knowledge and before Built apps; it is hidden on a `serve`
 * browser until PR-21 and wherever managed policy turns the dictionary off.
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
    path: DICTIONARY_PATH,
    label: 'Dictionary',
    icon: ENTITY_ICONS.dictionary,
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
 * Set once, the first time this profile sees a verified speech model, when the
 * disclosure opens itself so **Dictionary** can be found (DD-67). A later
 * collapse sticks, because the reveal never runs again.
 */
const DICTIONARY_REVEALED_STORAGE_KEY = 'biorouter:dictionary-revealed';

function readDictionaryRevealed(): boolean {
  try {
    return window.localStorage.getItem(DICTIONARY_REVEALED_STORAGE_KEY) === 'true';
  } catch {
    // Blocked storage: never reveal, rather than reveal on every launch.
    return true;
  }
}

/**
 * Collapsed by DEFAULT — that is where the 192px comes from (nine 32px nav rows
 * become three). Remembered, so a user who wants the seven open never reopens
 * them. Storage failures (private mode, a sandboxed frame) fall back to
 * collapsed rather than throwing, matching what the Chats list already does one file
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
  const currentSessionId = currentPath === '/pair' ? searchParams.get('resumeSessionId') : null;
  // A chat whose turn starts moves to the top under "Last activity" then, not
  // when the turn finishes (see `turnSessionIds`).
  const turnSessionIds = useMemo(() => sessionsInTurn(runningChats), [runningChats]);
  const { sessions, hasMore, isLoading, isLoadingAll, loadMore } = useSidebarSessions({
    loadAll: !isDefaultSidebarChatView(chatView),
    openSessionId: currentSessionId,
    turnSessionIds,
  });
  const runningSessionIds = useMemo(
    () =>
      new Set(runningChats.filter((entry) => !entry.completedAt).map((entry) => entry.sessionId)),
    [runningChats]
  );
  // The one rule for writing a folder (utils/folderPath.ts): `~` only while the daemon is this
  // machine's, so a folder group, its path and a chat's hover card read as the composer footer,
  // History and the terminal dock write the same folder. It changes labels, never group keys.
  const homeDir = homeFolder();
  const isMac = window.electron?.platform === 'darwin';
  const showShortcutHint = !isBrowserSurface();

  // Dictionary (DD-67). The status store is only read here, never loaded: it loads on the
  // first sign of dictation (a composer's focus, a microphone), so the sidebar sends nothing
  // when it mounts. Until it has loaded the row shows; managed `dictionary: false` hides it.
  const { status: dictationStatus } = useDictationStatus();
  const dictionaryAllowed = dictationStatus?.policy.dictionary.allowed !== false;
  const showDictionary = dictionaryAllowed;
  // The count reads `GET /dictionary/entries?status=suggested` only once the store says the
  // dictionary is allowed, on focus and every 60 s while the window is visible.
  const dictionaryWaiting = useDictionaryWaitingCount(
    showDictionary && dictationStatus?.policy.dictionary.allowed === true
  );
  const modelReady = !!dictationStatus && hasUsableModel(dictationStatus);
  const visibleComponentItems = useMemo(
    () => componentItems.filter((entry) => entry.path !== DICTIONARY_PATH || showDictionary),
    [showDictionary]
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
      navigateToRoute(navigate, '/pair', { newChat: true });
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

    navigateToRoute(navigate, path);
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
    // NB: the 3rd arg of navigateToRoute IS the route state itself,
    // not an options bag — `{ state: … }` here would nest it one level deep and
    // silently do nothing.
    navigateToRoute(
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
    const unread =
      entry.path === '/crew' ? crewUnread : entry.path === DICTIONARY_PATH ? dictionaryWaiting : 0;
    const unreadLabel =
      entry.path === DICTIONARY_PATH
        ? `${entry.label}, ${DICTIONARY_LABELS.waiting(unread)}`
        : `${entry.label}, ${unread} unread`;

    return (
      <SidebarMenuItem key={entry.path}>
        <SidebarMenuButton
          data-testid={`sidebar-${entry.label.toLowerCase().replace(/\s+/g, '-')}-button`}
          onClick={(event) => {
            handleNavigation(entry.path);
            if (isAction) blurAfterPointerActivation(event);
          }}
          aria-label={unread > 0 ? unreadLabel : undefined}
          aria-current={isActive ? 'page' : undefined}
          onFocus={entry.path === '/' ? preloadHome : undefined}
          onPointerEnter={entry.path === '/' ? preloadHome : undefined}
          isActive={isActive}
          className="br-nav-row no-drag"
        >
          {/* At rest an icon takes --sidebar-icon, which every family points
              at its muted label ink (the accent is held for the active rail);
              hovered, focused or current, the row decides and the icon wears
              the label's ink (sidebar.css). The token stays per family so
              check-contrast.mjs can assert it on every row fill. */}
          <IconComponent className="br-nav-row-icon" />
          <span className="br-nav-row-label">{entry.label}</span>
          {/* The shortcut is the Electron menu's accelerator, which a page
              served by `biorouter serve` never receives (and in a Windows or
              Linux browser Ctrl+T is the browser's own new tab), so the
              browser surface draws no hint rather than one that does nothing
              (SD-8). */}
          {isAction && showShortcutHint ? (
            <span className="br-nav-row-hint" aria-hidden="true">
              {sidebarCopy.shortcut.newChat(isMac)}
            </span>
          ) : null}
          {/* The neutral badge is the strong fill. On a row that is lit
              (hovered, current, focused) the row IS that fill, so there the
              badge steps back to the page ground (`.br-nav-row-badge`,
              sidebar.css): a fill never sits on a fill (R-27). */}
          {unread > 0 ? (
            <Badge tone="neutral" className="br-nav-row-badge tabular-nums" aria-hidden="true">
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
  const isOnComponentRoute = visibleComponentItems.some((entry) => entry.path === currentPath);
  const showComponentChildren = isComponentsExpanded || isOnComponentRoute;

  // The reveal (DD-67): the first time this profile sees a verified speech model, the
  // disclosure opens once and remembers it, so the person finds Dictionary right after the
  // install that makes it useful. The marker is written in the same step, so a later collapse
  // sticks.
  useEffect(() => {
    if (!modelReady || !showDictionary || readDictionaryRevealed()) return;
    try {
      window.localStorage.setItem(DICTIONARY_REVEALED_STORAGE_KEY, 'true');
      window.localStorage.setItem(COMPONENTS_EXPANDED_STORAGE_KEY, 'true');
    } catch {
      // Best effort: the disclosure still opens for this window.
    }
    setIsComponentsExpanded(true);
  }, [modelReady, showDictionary]);
  // Suggestions waiting behind a collapsed group show as a dot on the disclosure.
  const componentsDot = !showComponentChildren && dictionaryWaiting > 0;

  return (
    <>
      <SidebarContent className="gap-0 overflow-hidden">
        {/* The titlebar band: traffic lights and the floating TitlebarControls
            strip live over this space, so the sidebar only reserves it. Its
            bottom hairline continues the chat and preview header hairline at
            y=44 and runs the sidebar's full width (the container has no padding
            of its own), giving the window one continuous top edge. `h-chrome`:
            this band, BaseChat's header and the artifact strip read
            `--chrome-height` and move together or not at all. The hairline is
            `--border-subtle`, the chat header's own token, so the edge does not
            step in colour where the two meet at x=288 (vision V1-L8). */}
        <div
          data-testid="sidebar-titlebar-band"
          aria-hidden="true"
          className="h-chrome shrink-0 border-b border-border-subtle"
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
                  aria-label={
                    componentsDot
                      ? `${sidebarCopy.components}, ${DICTIONARY_LABELS.waiting(dictionaryWaiting)}`
                      : undefined
                  }
                  onClick={toggleComponents}
                  className="br-nav-row no-drag"
                >
                  <ChevronRight aria-hidden="true" className="br-nav-chevron" />
                  <span className="br-nav-row-label">{sidebarCopy.components}</span>
                  {componentsDot ? <StatusDot tone="warning" className="mr-1 shrink-0" /> : null}
                </SidebarMenuButton>
              </SidebarMenuItem>

              {/* A nested list inside its own item, so the children are list items
                  of a list (a bare div inside the menu's <ul> is not valid and
                  loses the list for a screen reader). */}
              {showComponentChildren && (
                <SidebarMenuItem>
                  <ul
                    id="sidebar-components-group"
                    data-testid="sidebar-components-group"
                    className="br-nav-list"
                  >
                    {visibleComponentItems.map((entry) => renderMenuItem(entry))}
                  </ul>
                </SidebarMenuItem>
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
          onViewAll={() => navigateToRoute(navigate, '/sessions')}
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
