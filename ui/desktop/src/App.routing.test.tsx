/* eslint-disable @typescript-eslint/no-explicit-any */

/**
 * @vitest-environment jsdom
 */

/**
 * The route table's edges: what an address nobody wrote a route for does, and
 * where the two routes PR #184 retired now land.
 *
 * ⚠ **A SEPARATE FILE from `App.test.tsx` because that one mocks
 * `react-router-dom` away entirely** — its `Routes` renders every child and its
 * `Route` renders its element unconditionally, so route MATCHING is not
 * observable there at all. Adding these cases to it would produce assertions
 * that pass no matter what the route table says. Here the real router runs
 * inside a `MemoryRouter`.
 *
 * ⚠ **The app shell is stubbed, and that is what makes the sidebar assertion
 * meaningful rather than decorative.** The real `AppLayout` mounts the sidebar,
 * the titlebar controls, the dependency modal and the extension reporter — none
 * of which this change touches, and all of which would need mocking anyway. The
 * property under test is not "the sidebar renders" (it always did, on every
 * other route); it is that the not-found page is a CHILD of the shell route, so
 * whatever the shell mounts is still on screen beside it. The stub renders an
 * `<Outlet />` inside a marked box, so "inside the shell" is directly
 * assertable — and the previous behaviour, a blank document with no shell at
 * all, fails it.
 */
import React from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter, Outlet } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AppInner } from './App';
import { echoPath } from './components/NotFoundView';

Object.defineProperty(window, 'history', {
  value: { replaceState: vi.fn(), state: null },
  writable: true,
});

vi.mock('./utils/providerUtils', () => ({
  initializeSystem: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('./utils/costDatabase', () => ({
  initializeCostDatabase: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('./api', () => ({
  initConfig: vi.fn().mockResolvedValue(undefined),
  readAllConfig: vi.fn().mockResolvedValue(undefined),
  backupConfig: vi.fn().mockResolvedValue(undefined),
  recoverConfig: vi.fn().mockResolvedValue(undefined),
  validateConfig: vi.fn().mockResolvedValue(undefined),
  startAgent: vi.fn().mockResolvedValue({ data: { session_id: 'test', messages: [] } }),
  resumeAgent: vi.fn().mockResolvedValue({ data: { session_id: 'test', messages: [] } }),
  // `KnowledgeProvider` wraps every route and hydrates the active knowledge base
  // on mount. Without this the mock throws, the provider logs a warning per
  // render, and the console noise buries a real failure.
  getActive: vi.fn().mockResolvedValue({ data: {} }),
  listBases: vi.fn().mockResolvedValue({ data: [] }),
}));

vi.mock('./sessions', () => ({
  fetchSessionDetails: vi.fn().mockResolvedValue({ sessionId: 'test', messages: [] }),
  generateSessionId: vi.fn(),
  createSession: vi.fn(),
}));

vi.mock('./utils/openRouterSetup', () => ({
  startOpenRouterSetup: vi.fn().mockResolvedValue({ success: false, message: 'Test' }),
}));

vi.mock('./utils/ollamaDetection', () => ({
  checkOllamaStatus: vi.fn().mockResolvedValue({ isRunning: false }),
}));

vi.mock('./components/ConfigContext', () => ({
  useConfig: () => ({
    read: vi.fn().mockResolvedValue(null),
    update: vi.fn(),
    getExtensions: vi.fn().mockReturnValue([]),
    addExtension: vi.fn(),
    updateExtension: vi.fn(),
    createProviderDefaults: vi.fn(),
  }),
  ConfigProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

vi.mock('./components/ErrorBoundary', () => ({
  ErrorUI: ({ error }: { error: Error }) => <div>Error: {error.message}</div>,
}));

// Passthrough: onboarding is a different question from routing, and a guard
// that swallowed the outlet would make every assertion below vacuous.
vi.mock('./components/ProviderGuard', () => ({
  default: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

vi.mock('./components/ModelAndProviderContext', () => ({
  ModelAndProviderProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  useModelAndProvider: () => ({
    provider: null,
    model: null,
    getCurrentModelAndProvider: vi.fn(),
    setCurrentModelAndProvider: vi.fn(),
  }),
}));

vi.mock('./contexts/ChatContext', () => ({
  ChatProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  useChatContext: () => ({
    chat: { id: 'test-id', name: 'Test Chat', messages: [], workflow: null },
    setChat: vi.fn(),
    setPairChat: vi.fn(),
    resetChat: vi.fn(),
    hasActiveSession: false,
    setWorkflow: vi.fn(),
    clearWorkflow: vi.fn(),
    contextKey: 'hub',
  }),
  DEFAULT_CHAT_TITLE: 'New Chat',
}));

vi.mock('./components/ui/ConfirmationModal', () => ({ ConfirmationModal: () => null }));
vi.mock('react-toastify', () => ({ ToastContainer: () => null }));
vi.mock('./components/AnnouncementModal', () => ({ default: () => null }));
vi.mock('./components/UpdateAvailableModal', () => ({ default: () => null }));
vi.mock('./components/ExtensionInstallModal', () => ({ ExtensionInstallModal: () => null }));

// The shell, reduced to the one thing this file is about: a marked box that
// renders whatever the matched child route is.
vi.mock('./components/Layout/AppLayout', () => ({
  AppLayout: () => (
    <div data-testid="app-shell">
      <nav data-testid="app-sidebar" aria-label="Sidebar" />
      <Outlet />
    </div>
  ),
}));

vi.mock('./components/Hub', () => ({ default: () => <div data-testid="home-view">Home</div> }));

/**
 * The other top-level views, stubbed. Their INTERNALS are irrelevant here — the
 * question this file asks about `/settings`, `/sessions` and the rest is only
 * "does the catch-all shadow them", which is answered by whether the route
 * resolves to this stub or to the not-found page. Rendering them for real
 * instead makes the test fail for reasons that have nothing to do with routing
 * (they fetch, they subscribe to `window.electron` channels, they mount the
 * search bar), which is a test that reports on the wrong thing.
 */
const { routeStub } = vi.hoisted(() => ({
  routeStub: (name: string) => ({
    default: () => <div data-testid={`route-${name}`}>{name}</div>,
  }),
}));
vi.mock('./components/settings/SettingsView', () => routeStub('settings'));
vi.mock('./components/sessions/SessionsView', () => routeStub('sessions'));
vi.mock('./components/schedule/SchedulesView', () => routeStub('schedules'));
vi.mock('./components/workflows/WorkflowsView', () => routeStub('workflows'));
vi.mock('./components/skills/SkillsView', () => routeStub('skills'));
vi.mock('./components/applications/ApplicationsView', () => routeStub('applications'));
vi.mock('./components/knowledge/KnowledgeView', () => routeStub('knowledge'));
vi.mock('./components/extensions/ExtensionsView', () => routeStub('extensions'));

const mockElectron = {
  getConfig: vi.fn().mockReturnValue({
    BIOROUTER_DEFAULT_PROVIDER: 'openai',
    BIOROUTER_DEFAULT_MODEL: 'gpt-4',
    BIOROUTER_ALLOWLIST_WARNING: false,
    BIOROUTER_WORKING_DIR: '/test/dir',
  }),
  logInfo: vi.fn(),
  on: vi.fn(),
  off: vi.fn(),
  reactReady: vi.fn(),
  getAllowedExtensions: vi.fn().mockResolvedValue([]),
  platform: 'darwin',
  createChatWindow: vi.fn(),
  cliStatus: vi.fn().mockResolvedValue(null),
};

(window as any).electron = mockElectron;
(window as any).appConfig = {
  get: (key: string) => (key === 'BIOROUTER_WORKING_DIR' ? '/test/dir' : null),
};

Object.defineProperty(window, 'matchMedia', {
  writable: true,
  value: vi.fn().mockImplementation((query) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: vi.fn(),
    removeListener: vi.fn(),
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    dispatchEvent: vi.fn(),
  })),
});

function renderAt(path: string) {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <AppInner />
    </MemoryRouter>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  window.sessionStorage.clear();
  window.localStorage.clear();
});

afterEach(() => {
  vi.clearAllMocks();
});

describe('an address with no route', () => {
  /**
   * The reported defect, in the shape it was reported: `#/zzz`, `#/scheduler`
   * (the real route is `#/schedules`) and any other typo rendered an EMPTY
   * document — `document.body.innerText.length === 0`, no sidebar, no header,
   * no error boundary — with `No routes matched location` in the console as the
   * only trace, and no way back short of reloading the app.
   */
  it.each(['/zzz-nonexistent', '/scheduler', '/settings/typo'])(
    'renders the not-found page at %s',
    async (path) => {
      renderAt(path);

      expect(await screen.findByText('There is nothing at this address')).toBeInTheDocument();
      expect(document.body.innerText ?? document.body.textContent).not.toBe('');
    }
  );

  /**
   * The half of the fix that is easy to get wrong: a catch-all declared as a
   * SIBLING of the shell route renders the message on the same bare canvas the
   * blank page had. The way out has to still be on screen.
   */
  it('renders it inside the app shell, so the sidebar is still there', async () => {
    renderAt('/zzz-nonexistent');

    const shell = await screen.findByTestId('app-shell');
    expect(screen.getByTestId('app-sidebar')).toBeInTheDocument();
    expect(shell).toContainElement(screen.getByText('There is nothing at this address'));
  });

  it('echoes the address that missed', async () => {
    renderAt('/zzz-nonexistent');

    expect(await screen.findByText(/\/zzz-nonexistent/)).toBeInTheDocument();
  });

  it('offers a way back to Home', async () => {
    renderAt('/zzz-nonexistent');

    expect(await screen.findByRole('button', { name: /go to home/i })).toBeInTheDocument();
  });

  /**
   * The catch-all must not have eaten the routes that exist. `*` outranks
   * nothing, but a catch-all placed at the wrong nesting level would shadow the
   * shell's own children, and every one of them would fail the same way.
   */
  it.each(['/', '/settings', '/sessions', '/schedules', '/workflows', '/skills', '/applications'])(
    'leaves the real route %s alone',
    async (path) => {
      renderAt(path);

      await waitFor(() => expect(screen.getByTestId('app-shell')).toBeInTheDocument());
      expect(screen.queryByText('There is nothing at this address')).not.toBeInTheDocument();
    }
  );
});

describe('the routes PR #184 retired', () => {
  /**
   * `/apps` was the MCP-apps browser and `/standalone-app` its single-app
   * surface. Both shipped, so both are in bookmarks and in `biorouter://` share
   * links; after #184 they matched nothing and rendered the blank window.
   *
   * They go to Home rather than to the not-found page — a link to a feature
   * that existed is not a typo — and deliberately NOT to `/applications`, which
   * is Built apps, a different feature that merely reads alike.
   */
  it.each(['/apps', '/standalone-app'])('sends %s to Home', async (path) => {
    renderAt(path);

    expect(await screen.findByTestId('home-view')).toBeInTheDocument();
    expect(screen.queryByText('There is nothing at this address')).not.toBeInTheDocument();
  });
});

/**
 * The address is echoed back so a stale bookmark tells the user WHICH link
 * died. It is also attacker-influenced text of unbounded length — a
 * `biorouter://` deep link carries a whole payload in its path — and it lands
 * in a `max-w-sm` paragraph, so it has to be trimmed before it is shown.
 * (React escapes it; the hazard here is layout, not injection.)
 */
describe('the address the not-found page echoes', () => {
  it('shows a short path exactly as it arrived', () => {
    expect(echoPath('/zzz-nonexistent')).toBe('/zzz-nonexistent');
  });

  it('always reads as a path, even if the router hands it one without a slash', () => {
    expect(echoPath('scheduler')).toBe('/scheduler');
  });

  it('trims a long one to something a sentence can hold', () => {
    const echoed = echoPath(`/${'a'.repeat(500)}`);
    expect(echoed.length).toBeLessThanOrEqual(72);
    expect(echoed.endsWith('…')).toBe(true);
  });
});
