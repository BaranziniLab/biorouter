import { type ReactNode, useEffect, useState, useRef } from 'react';
import { IpcRendererEvent } from 'electron';
import {
  HashRouter,
  Routes,
  Route,
  useNavigate,
  useLocation,
  useSearchParams,
} from 'react-router-dom';
import { openSharedSessionFromDeepLink } from './sessionLinks';
import { type SharedSessionDetails } from './sharedSessions';
import { ErrorUI } from './components/ErrorBoundary';
import { TOAST_SURFACE_CLASS_NAME } from './components/alerts/NotificationSurface';
import { ExtensionInstallModal } from './components/ExtensionInstallModal';
import { ToastContainer } from 'react-toastify';
import AnnouncementModal from './components/AnnouncementModal';
import UpdateAvailableModal from './components/UpdateAvailableModal';
import ProviderGuard from './components/ProviderGuard';
import { createSession } from './sessions';

import { ChatType } from './types/chat';
import Hub from './components/Hub';
import { PairRouteState } from './components/Pair';
import { ChatGroupsProvider, useChatGroups } from './contexts/ChatGroupsContext';
import { requestNewTab } from './components/chatGroups/newTabRegistry';
import { useEmptyPairRedirect } from './components/chatGroups/useEmptyPairRedirect';
import { runCloseActiveTabCommand } from './utils/closeActiveTabCommand';
import { isTerminalFocused, requestNewTerminalPane } from './utils/terminalFocus';
import { TerminalDockProvider } from './contexts/TerminalDockContext';
import ChatGroupsShell from './components/chatGroups/ChatGroupsShell';
import SettingsView, { SettingsViewOptions } from './components/settings/SettingsView';
import SessionsView from './components/sessions/SessionsView';
import SharedSessionView from './components/sessions/SharedSessionView';
import SchedulesView from './components/schedule/SchedulesView';
import ProviderSettings from './components/settings/providers/ProviderSettingsPage';
import { AppLayout } from './components/Layout/AppLayout';
import { ChatProvider } from './contexts/ChatContext';
import LauncherView from './components/LauncherView';

import 'react-toastify/dist/ReactToastify.css';
import { useConfig } from './components/ConfigContext';
import { ModelAndProviderProvider } from './components/ModelAndProviderContext';
import { AppNonPrivateModelDisclosureGate } from './components/privacy/AppNonPrivateModelDisclosureGate';
import { FirstRunPrivacyNoticeGate } from './components/privacy/FirstRunPrivacyNoticeGate';
import { ThemeProvider } from './contexts/ThemeContext';
import PermissionSettingsView from './components/settings/permission/PermissionSetting';

import ExtensionsView, { ExtensionsViewOptions } from './components/extensions/ExtensionsView';
import WorkflowsView from './components/workflows/WorkflowsView';
import SkillsView from './components/skills/SkillsView';
import KnowledgeView from './components/knowledge/KnowledgeView';
import { KnowledgeProvider } from './components/knowledge/KnowledgeContext';
import ApplicationsView from './components/applications/ApplicationsView';
import { View, ViewOptions } from './utils/navigationUtils';

import { useNavigation } from './hooks/useNavigation';
import { errorMessage } from './utils/conversionUtils';
import { getInitialWorkingDir } from './utils/workingDir';
import { deliverLauncherMessage } from './utils/launcherMessage';
import { ChatStreamProvider } from './hooks/chatStreamStore';
import { AppTooltipLayer } from './components/ui/AppTooltipLayer';

// Route Components
const HubRouteWrapper = () => {
  const setView = useNavigation();
  return <Hub setView={setView} />;
};

/**
 * TerminalDockProvider wraps /pair ONLY.
 *
 * It holds the PER-CHAT-TAB terminals, keyed by tab id, above the tab switch so
 * a tab's shell survives switching away from it (BaseChat, keyed by tab id,
 * remounts on every switch). Its scope is deliberately this route and no wider:
 * every other surface (/extensions' mini-chat,
 * the Hub) has no provider, so useTerminalDock() returns null there and BaseChat
 * keeps its own local per-chat dock exactly as it does today. Hoisting this to
 * the app root would let those surfaces share terminal state across windows.
 */
const PairRouteWrapper = ({ setChat }: { setChat: (chat: ChatType) => void }) => (
  <ChatGroupsProvider>
    <TerminalDockProvider>
      <PairRouteContent setChat={setChat} />
    </TerminalDockProvider>
  </ChatGroupsProvider>
);

/**
 * The URL adapter, and nothing else.
 *
 * This used to own /pair's session identity (and mirror it into the URL from an
 * effect of its own). ChatGroupsProvider is now the ONLY writer of
 * ?resumeSessionId= — two writers is precisely the mutual recursion R2 warns
 * about — so the old sync effect here is deleted. What remains are the entry
 * points that must create a session BEFORE anything can be opened: the Hub
 * composer's initialMessage, a workflow deeplink, and the sidebar's new-chat.
 * Each of them lands back here as a normal ?resumeSessionId= navigation, which
 * the provider consumes exactly once.
 */
const PairRouteContent = ({ setChat }: { setChat: (chat: ChatType) => void }) => {
  const { extensionsList } = useConfig();
  const location = useLocation();
  const navigate = useNavigate();
  const groups = useChatGroups();
  const routeState = (location.state as PairRouteState) || {};
  const [searchParams] = useSearchParams();
  const [isCreatingSession, setIsCreatingSession] = useState(false);

  const resumeSessionId = searchParams.get('resumeSessionId') ?? undefined;
  const workflowId = searchParams.get('workflowId') ?? undefined;
  const workflowDeeplinkFromConfig = window.appConfig?.get('workflowDeeplink') as
    | string
    | undefined;
  const isNewChat = routeState.newChat === true;

  const sessionIdFromState = routeState.resumeSessionId;
  // Identity comes from the URL and the route state only. The old
  // `|| chat.sessionId` fallback reached into the App-level singleton, which is
  // no longer /pair's identity: the focused tab is.
  const sessionId = isNewChat ? undefined : sessionIdFromState || resumeSessionId || undefined;
  const initialMessage = isNewChat ? undefined : routeState.initialMessage;
  const initialAttachments = isNewChat ? undefined : routeState.initialAttachments;

  const dispatch = groups?.dispatch;

  // No tabs → Home (issue #38). When the whole layout is empty and no cargo is
  // en route to becoming a tab (deep link, Hub submit, sidebar new-chat,
  // workflow, mid-flight session creation, pending Cmd+T), /pair is a dead-end
  // "New chat" pane — redirect to the Hub instead. All gates live in the
  // hook; isCreatingSession is the only piece that is component state here.
  useEmptyPairRedirect(isCreatingSession);

  // The sidebar's new-chat button: open ONE empty tab per navigation. The tab
  // carries sessionId '' until BaseChat's pre-session submit creates a real
  // session; that navigation then ADOPTS this tab in place (see the reducer's
  // empty-tab branch) rather than orphaning it beside a second one.
  const newChatKeyRef = useRef<string | null>(null);
  useEffect(() => {
    if (!isNewChat || !dispatch) return;
    if (newChatKeyRef.current === location.key) return;
    newChatKeyRef.current = location.key;
    dispatch({ type: 'openTab', payload: { sessionId: '' } });
  }, [isNewChat, location.key, dispatch]);

  // Create a session when we have something to say but nothing to say it in.
  useEffect(() => {
    if (
      !isNewChat &&
      (initialMessage || workflowId || workflowDeeplinkFromConfig) &&
      !sessionId &&
      !isCreatingSession
    ) {
      setIsCreatingSession(true);

      (async () => {
        try {
          const newSession = await createSession(getInitialWorkingDir(), {
            workflowId,
            workflowDeeplink: workflowDeeplinkFromConfig,
            allExtensions: extensionsList,
          });
          navigate(`/pair?resumeSessionId=${newSession.id}`, {
            replace: true,
            state: { resumeSessionId: newSession.id, initialMessage, initialAttachments },
          });
        } catch (error) {
          console.error('Failed to create session:', error);
        } finally {
          setIsCreatingSession(false);
        }
      })();
    }
  }, [
    initialMessage,
    initialAttachments,
    isNewChat,
    workflowId,
    workflowDeeplinkFromConfig,
    sessionId,
    isCreatingSession,
    extensionsList,
    navigate,
  ]);

  return <ChatGroupsShell onChatChange={setChat} />;
};

const SettingsRoute = () => {
  const location = useLocation();
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const setView = useNavigation();

  // Get viewOptions from location.state, history.state, or URL search params
  const viewOptions =
    (location.state as SettingsViewOptions) || (window.history.state as SettingsViewOptions) || {};

  // If section is provided via URL search params, add it to viewOptions
  const sectionFromUrl = searchParams.get('section');
  if (sectionFromUrl) {
    viewOptions.section = sectionFromUrl;
  }

  return <SettingsView onClose={() => navigate('/')} setView={setView} viewOptions={viewOptions} />;
};

const SessionsRoute = () => {
  return <SessionsView />;
};

const SchedulesRoute = () => {
  const navigate = useNavigate();
  return <SchedulesView onClose={() => navigate('/')} />;
};

const WorkflowsRoute = () => {
  return <WorkflowsView />;
};

const SkillsRoute = () => <SkillsView />;
const KnowledgeRoute = () => <KnowledgeView />;

const PermissionRoute = () => {
  const location = useLocation();
  const navigate = useNavigate();
  const parentView = location.state?.parentView as View;
  const parentViewOptions = location.state?.parentViewOptions as ViewOptions;

  return (
    <PermissionSettingsView
      onClose={() => {
        // Navigate back to parent view with options
        switch (parentView) {
          case 'chat':
            navigate('/');
            break;
          case 'pair':
            navigate('/pair');
            break;
          case 'settings':
            navigate('/settings', { state: parentViewOptions });
            break;
          case 'sessions':
            navigate('/sessions');
            break;
          case 'schedules':
            navigate('/schedules');
            break;
          case 'workflows':
            navigate('/workflows');
            break;
          default:
            navigate('/');
        }
      }}
    />
  );
};

const ConfigureProvidersRoute = () => {
  const navigate = useNavigate();

  return (
    <div className="w-screen h-screen bg-background-default">
      <ProviderSettings
        onClose={() => navigate('/settings', { state: { section: 'models' } })}
        isOnboarding={false}
      />
    </div>
  );
};

interface WelcomeRouteProps {
  onSelectProvider: () => void;
}

const WelcomeRoute = ({ onSelectProvider }: WelcomeRouteProps) => {
  const navigate = useNavigate();

  return (
    <div className="w-screen h-screen bg-background-default">
      <ProviderSettings
        onClose={() => {
          navigate('/', { replace: true });
        }}
        isOnboarding={true}
        onProviderLaunched={() => {
          onSelectProvider();
          navigate('/', { replace: true });
        }}
      />
    </div>
  );
};

// Wrapper component for SharedSessionRoute to access parent state
const SharedSessionRouteWrapper = ({
  isLoadingSharedSession,
  setIsLoadingSharedSession,
  sharedSessionError,
}: {
  isLoadingSharedSession: boolean;
  setIsLoadingSharedSession: (loading: boolean) => void;
  sharedSessionError: string | null;
}) => {
  const location = useLocation();
  const setView = useNavigation();

  const historyState = window.history.state;
  const sessionDetails = (location.state?.sessionDetails ||
    historyState?.sessionDetails) as SharedSessionDetails | null;
  const error = location.state?.error || historyState?.error || sharedSessionError;
  const shareToken = location.state?.shareToken || historyState?.shareToken;
  const baseUrl = location.state?.baseUrl || historyState?.baseUrl;

  return (
    <SharedSessionView
      session={sessionDetails}
      isLoading={isLoadingSharedSession}
      error={error}
      onRetry={async () => {
        if (shareToken && baseUrl) {
          setIsLoadingSharedSession(true);
          try {
            await openSharedSessionFromDeepLink(
              `biorouter://sessions/${shareToken}`,
              setView,
              baseUrl
            );
          } catch (error) {
            console.error('Failed to retry loading shared session:', error);
          } finally {
            setIsLoadingSharedSession(false);
          }
        }
      }}
    />
  );
};

const ExtensionsRoute = () => {
  const navigate = useNavigate();
  const location = useLocation();

  // Get viewOptions from location.state or history.state (for deep link extensions)
  const viewOptions =
    (location.state as ExtensionsViewOptions) ||
    (window.history.state as ExtensionsViewOptions) ||
    {};

  return (
    <ExtensionsView
      onClose={() => navigate(-1)}
      setView={(view, options) => {
        switch (view) {
          case 'chat':
            navigate('/');
            break;
          case 'pair':
            navigate('/pair', { state: options });
            break;
          case 'settings':
            navigate('/settings', { state: options });
            break;
          default:
            navigate('/');
        }
      }}
      viewOptions={viewOptions}
    />
  );
};

export function AppInner() {
  const [fatalError, setFatalError] = useState<string | null>(null);
  const [isLoadingSharedSession, setIsLoadingSharedSession] = useState(false);
  const [sharedSessionError, setSharedSessionError] = useState<string | null>(null);
  const [didSelectProvider, setDidSelectProvider] = useState<boolean>(false);

  const navigate = useNavigate();
  const setView = useNavigation();

  const [chat, setChat] = useState<ChatType>({
    sessionId: '',
    name: 'New chat',
    messages: [],
    workflow: null,
  });

  const { addExtension } = useConfig();

  useEffect(() => {
    console.log('Sending reactReady signal to Electron');
    try {
      window.electron.reactReady();
    } catch (error) {
      console.error('Error sending reactReady:', error);
      setFatalError(
        `React ready notification failed: ${error instanceof Error ? error.message : 'Unknown error'}`
      );
    }
  }, []);

  useEffect(() => {
    const handleOpenSharedSession = async (_event: IpcRendererEvent, ...args: unknown[]) => {
      const link = args[0] as string;
      window.electron.logInfo(`Opening shared session from deep link ${link}`);
      setIsLoadingSharedSession(true);
      setSharedSessionError(null);
      try {
        await openSharedSessionFromDeepLink(link, (_view: View, options?: ViewOptions) => {
          navigate('/shared-session', { state: options });
        });
      } catch (error) {
        console.error('Unexpected error opening shared session:', error);
        // Navigate to shared session view with error
        const shareToken = link.replace('biorouter://sessions/', '');
        const options = {
          sessionDetails: null,
          error: error instanceof Error ? error.message : 'Unknown error',
          shareToken,
        };
        navigate('/shared-session', { state: options });
      } finally {
        setIsLoadingSharedSession(false);
      }
    };
    return window.electron.on('open-shared-session', handleOpenSharedSession);
  }, [navigate]);

  useEffect(() => {
    console.log('Setting up keyboard shortcuts');
    const handleKeyDown = (event: KeyboardEvent) => {
      const isMac = window.electron.platform === 'darwin';
      if ((isMac ? event.metaKey : event.ctrlKey) && event.key === 'n') {
        event.preventDefault();
        try {
          window.electron.createChatWindow(undefined, getInitialWorkingDir());
        } catch (error) {
          console.error('Error creating new window:', error);
        }
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => {
      window.removeEventListener('keydown', handleKeyDown);
    };
  }, []);

  // Prevent default drag and drop behavior globally to avoid opening files in new windows
  // but allow our React components to handle drops in designated areas
  useEffect(() => {
    const preventDefaults = (e: globalThis.DragEvent) => {
      // Only prevent default if we're not over a designated drop zone
      const target = e.target as HTMLElement;
      const isOverDropZone = target.closest('[data-drop-zone="true"]') !== null;

      if (!isOverDropZone) {
        e.preventDefault();
        e.stopPropagation();
      }
    };

    const handleDragOver = (e: globalThis.DragEvent) => {
      // Always prevent default for dragover to allow dropping
      e.preventDefault();
      e.stopPropagation();
    };

    const handleDrop = (e: globalThis.DragEvent) => {
      // Only prevent default if we're not over a designated drop zone
      const target = e.target as HTMLElement;
      const isOverDropZone = target.closest('[data-drop-zone="true"]') !== null;

      if (!isOverDropZone) {
        e.preventDefault();
        e.stopPropagation();
      }
    };

    // Add event listeners to document to catch drag events
    document.addEventListener('dragenter', preventDefaults, false);
    document.addEventListener('dragleave', preventDefaults, false);
    document.addEventListener('dragover', handleDragOver, false);
    document.addEventListener('drop', handleDrop, false);

    return () => {
      document.removeEventListener('dragenter', preventDefaults, false);
      document.removeEventListener('dragleave', preventDefaults, false);
      document.removeEventListener('dragover', handleDragOver, false);
      document.removeEventListener('drop', handleDrop, false);
    };
  }, []);

  useEffect(() => {
    const handleFatalError = (_event: IpcRendererEvent, ...args: unknown[]) => {
      const errorMessage = args[0] as string;
      console.error('Encountered a fatal error:', errorMessage);
      setFatalError(errorMessage);
    };
    return window.electron.on('fatal-error', handleFatalError);
  }, []);

  useEffect(() => {
    const handleOpenBrxtFile = (_event: IpcRendererEvent, ...args: unknown[]) => {
      const filePath = args[0];
      if (typeof filePath !== 'string') return;
      navigate('/extensions', { state: { brxtFilePath: filePath } });
    };
    return window.electron.on('open-brxt-file', handleOpenBrxtFile);
  }, [navigate]);

  useEffect(() => {
    const handleSetView = (_event: IpcRendererEvent, ...args: unknown[]) => {
      const newView = args[0] as View;
      const section = args[1] as string | undefined;
      console.log(
        `Received view change request to: ${newView}${section ? `, section: ${section}` : ''}`
      );

      if (section && newView === 'settings') {
        navigate(`/settings?section=${section}`);
      } else {
        navigate(`/${newView}`);
      }
    };

    return window.electron.on('set-view', handleSetView);
  }, [navigate]);

  // Cmd+W (Ctrl+W off mac). Sent by the File menu's "Close Tab" item — see
  // main.ts, where the accelerator had to be taken off `role: 'close'` so it
  // stops closing the whole window.
  //
  // This listener lives at the ROOT, not in ChatGroupsProvider, because it must
  // answer everywhere: the provider is mounted only under /pair, and Cmd+W on
  // Settings must still close the window like any other macOS app. The ladder —
  // focused terminal pane, then chat tab, then window — lives in
  // runCloseActiveTabCommand so the registry-driven tests gate the exact code
  // this handler runs (the keystroke never reaches the DOM; the menu owns it).
  //
  // No text-input guard: Cmd+W has no native editing behaviour to steal, and the
  // key never reaches the DOM anyway — the menu consumes it.
  useEffect(
    () =>
      window.electron.on('close-active-tab', () => {
        runCloseActiveTabCommand(() => window.electron.closeWindow());
      }),
    []
  );

  // Cmd+T / Ctrl+T — a new tab. Sent by the Go menu's "New Chat" item; like
  // Cmd+W it can only be a menu item, because the menu already owned the key and
  // would have eaten any renderer listener.
  //
  // What "new tab" MEANS is focus-aware: when the cursor is in the in-app
  // terminal, Cmd+T adds a new terminal PANE (the reflex a terminal user has);
  // otherwise it opens a new CHAT tab. isTerminalFocused only sees the visible
  // dock (a hidden one is display:none and cannot hold focus), and
  // requestNewTerminalPane returns false when no terminal is open — either way
  // we fall through to the chat path, so a chat-focused Cmd+T is unchanged.
  //
  // The chat path keeps the same root-level reasoning as Cmd+W: the tab surface
  // is mounted only under /pair, so off that route requestNewTab() finds no
  // handler. It then REMEMBERS the request and we navigate — the mounting
  // provider consumes it and opens the tab. Cmd+T on Settings therefore lands
  // you on a fresh chat, as the key does in a browser from any page.
  useEffect(
    () =>
      window.electron.on('new-chat-tab', () => {
        if (isTerminalFocused() && requestNewTerminalPane()) return;
        if (requestNewTab()) return;
        navigate('/pair');
      }),
    [navigate]
  );

  useEffect(() => {
    const handleFocusInput = (_event: IpcRendererEvent, ..._args: unknown[]) => {
      const inputField = document.querySelector('input[type="text"], textarea') as HTMLInputElement;
      if (inputField) {
        inputField.focus();
      }
    };
    return window.electron.on('focus-input', handleFocusInput);
  }, []);

  // Handle initial message from launcher. The session id must ride the query
  // string (the ChatGroups URL-sync inbox reads nothing else) and the
  // navigation must REPLACE the ?initialMessagePending=true bootstrap entry —
  // both invariants live with deliverLauncherMessage, whose wiring test drives
  // the real chain end to end (launcherMessageWiring.test.tsx).
  useEffect(() => {
    const handleSetInitialMessage = (_event: IpcRendererEvent, ...args: unknown[]) => {
      const initialMessage = args[0] as string;
      if (initialMessage) {
        console.log('Received initial message from launcher:', initialMessage);
        void deliverLauncherMessage(navigate, initialMessage);
      }
    };
    return window.electron.on('set-initial-message', handleSetInitialMessage);
  }, [navigate]);

  // Headless/browser mode has no second window, so `createChatWindow` re-enters
  // here as a DOM event instead (see renderer.tsx). Same delivery path, same
  // auto-submit — the difference is only that it lands in this tab.
  useEffect(() => {
    const handleSeededChat = (event: Event) => {
      const message = (event as CustomEvent<string>).detail;
      if (message) void deliverLauncherMessage(navigate, message);
    };
    window.addEventListener('biorouter:open-seeded-chat', handleSeededChat);
    return () => window.removeEventListener('biorouter:open-seeded-chat', handleSeededChat);
  }, [navigate]);

  if (fatalError) {
    return <ErrorUI error={errorMessage(fatalError)} />;
  }

  return (
    <>
      <AppTooltipLayer />
      {/* The toast layer's geometry lives here, inline, and both entries below
          exist because a stylesheet could not win the argument.

          `--toastify-z-index` is the library's own knob, declared on `:root` in
          the vendored ReactToastify.css — which is imported AFTER main.css, so
          a `:root` rule of ours loses on source order. That is why `--z-toast`
          shipped as a DEAD token: the ladder declared 600 and the container
          rendered 9999, and the one element that did honour the token (the
          usage-heatmap tooltip) therefore sat below any real toast. Setting the
          library's variable rather than `z-index` also fixes the `translate3d`
          the vendor derives from it.

          `top` docks the layer in the CORNER, one stack gap below the titlebar
          drag band. It briefly did the opposite — 144px, chosen to clear the
          tallest page header so a toast could never cover a page title or Chat
          history's Import Session button — and that is what put the
          extension-load report halfway down the chat pane, which is the bug
          this value closes. The reasoning and the floor live with
          `--toast-inset-top` in main.css; do not re-derive it from a header
          measurement here. */}
      <ToastContainer
        aria-label="Toast notifications"
        toastClassName={() => TOAST_SURFACE_CLASS_NAME}
        style={
          {
            '--toastify-z-index': 'var(--z-toast)',
            top: 'var(--toast-inset-top)',
            width: 'fit-content',
            minWidth: '280px',
            maxWidth: 'min(420px, calc(100vw - 32px))',
          } as React.CSSProperties
        }
        position="top-right"
        /* OLDEST NEAREST THE CORNER — the stack grows DOWNWARD, so the second
           notification lands directly below the first rather than shoving it
           down.

           ⚠ This is `false` on purpose and it CONTRADICTS the written design
           (`docs/design/astryx-adoption/astryx-ui-adoption-design.md` §3.7 says
           "newest nearest the corner"). The user asked for the opposite in so
           many words, and a user instruction outranks the doc; the doc is the
           thing that needs reconciling. It is also spelled out rather than left
           to the library, because react-toastify's default happens to agree
           today: without this prop the behaviour is correct by accident, and
           the next reader following §3.7 would add `newestOnTop` and silently
           undo a decision nothing was defending. */
        newestOnTop={false}
        autoClose={3000}
        closeOnClick
        pauseOnHover
      />
      <ExtensionInstallModal addExtension={addExtension} setView={setView} />
      <div className="relative w-screen h-screen overflow-hidden bg-background-canvas flex flex-col">
        <div className="titlebar-drag-region" />
        <ChatStreamProvider>
          <KnowledgeProvider sessionId={chat.sessionId || null}>
            <Routes>
              <Route path="launcher" element={<LauncherView />} />
              <Route
                path="welcome"
                element={<WelcomeRoute onSelectProvider={() => setDidSelectProvider(true)} />}
              />
              <Route path="configure-providers" element={<ConfigureProvidersRoute />} />
              <Route
                path="/"
                element={
                  <ProviderGuard didSelectProvider={didSelectProvider}>
                    <ChatProvider chat={chat} setChat={setChat} contextKey="hub">
                      <AppLayout />
                    </ChatProvider>
                  </ProviderGuard>
                }
              >
                <Route index element={<HubRouteWrapper />} />
                <Route path="pair" element={<PairRouteWrapper setChat={setChat} />} />
                <Route path="settings" element={<SettingsRoute />} />
                <Route
                  path="extensions"
                  element={
                    <ChatProvider chat={chat} setChat={setChat} contextKey="extensions">
                      <ExtensionsRoute />
                    </ChatProvider>
                  }
                />
                <Route path="applications" element={<ApplicationsView />} />
                <Route path="sessions" element={<SessionsRoute />} />
                <Route path="schedules" element={<SchedulesRoute />} />
                <Route path="workflows" element={<WorkflowsRoute />} />
                <Route path="skills" element={<SkillsRoute />} />
                <Route path="knowledge" element={<KnowledgeRoute />} />
                <Route
                  path="shared-session"
                  element={
                    <SharedSessionRouteWrapper
                      isLoadingSharedSession={isLoadingSharedSession}
                      setIsLoadingSharedSession={setIsLoadingSharedSession}
                      sharedSessionError={sharedSessionError}
                    />
                  }
                />
                <Route path="permission" element={<PermissionRoute />} />
              </Route>
            </Routes>
          </KnowledgeProvider>
        </ChatStreamProvider>
      </div>
    </>
  );
}

export function ImmediateHashRouter({ children }: { children: ReactNode }) {
  return <HashRouter useTransitions={false}>{children}</HashRouter>;
}

export default function App() {
  return (
    <ThemeProvider>
      <ModelAndProviderProvider>
        <ImmediateHashRouter>
          <AppInner />
        </ImmediateHashRouter>
        <AnnouncementModal />
        <UpdateAvailableModal />
        {/*
          Issue #56, DR-17 requirement 3 — the one-time disclosure of what a
          non-private model can reach, shown BEFORE the first turn.

          ⚠ Mounted here, at the app level, and not only inside `BaseChat`. The
          chat-level mount is keyed on the chat's bound provider, which does not
          exist until a session row does — i.e. until the first turn has already
          been sent. On a fresh install that made the disclosure a receipt. This
          one is keyed on the configured provider, so it lands as soon as one is
          bound, on whatever route the user is standing on.

          ⚠ Inside `ModelAndProviderProvider` because it reads that context, and
          outside the router because it belongs to the install, not to a view.
          `useSoleDisclosurePresenter` keeps the two mounts to one dialog.
        */}
        <AppNonPrivateModelDisclosureGate />
        {/*
          Issue #56 §15.5(3) — the day-one notice, shown once per install after
          the migration has marked part of the user's history private.

          ⚠ Mounted here for the same reason as the gate above it: the subject is
          the install, not a view. This one carries an extra obligation, though —
          Task 38 also ships the backfill, which is irreversible without a
          per-chat declassification, so a notice that exists only in the source
          tree leaves "day one is discovered, not shown", which is the failure
          §15.5 is named after.

          It is due only when the MIGRATION marked something the user can see, so
          a fresh install never renders it however many private chats it goes on
          to accumulate.
        */}
        <FirstRunPrivacyNoticeGate />
      </ModelAndProviderProvider>
    </ThemeProvider>
  );
}
