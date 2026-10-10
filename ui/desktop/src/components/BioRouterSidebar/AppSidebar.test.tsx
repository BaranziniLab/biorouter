import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SessionSummary } from '../../api';
import { ChatProvider } from '../../contexts/ChatContext';
import type { ChatType } from '../../types/chat';
import {
  SIDEBAR_OVERLAY_BODY_CLASS,
  Sidebar,
  SidebarInset,
  SidebarProvider,
  SidebarTrigger,
} from '../ui/sidebar';

const mocks = vi.hoisted(() => ({
  listSessions: vi.fn(),
  listSidebarSessions: vi.fn(),
}));

vi.mock('../../api', () => ({
  listSessions: mocks.listSessions,
  listSidebarSessions: mocks.listSidebarSessions,
}));

vi.mock('../../hooks/chatStreamStore', () => ({
  useRunningChats: () => [],
}));

vi.mock('./SidebarUpdateButton', () => ({
  default: () => null,
}));

import AppSidebar from './AppSidebar';
import { SAME_ROUTE_RESET_EVENT } from '../../hooks/useSameRouteReset';

const session: SessionSummary = {
  id: 'session-1',
  name: 'Inspect latest cohort',
  created_at: '2026-07-14T12:00:00.000Z',
  updated_at: '2026-07-15T12:00:00.000Z',
  working_dir: '/workspace/cohort-study',
  message_count: 4,
  user_set_name: true,
};

beforeAll(() => {
  Object.defineProperty(window, 'matchMedia', {
    writable: true,
    value: vi.fn().mockImplementation((query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })),
  });
});

beforeEach(() => {
  mocks.listSessions.mockResolvedValue({ data: { sessions: [session] } });
  mocks.listSidebarSessions.mockResolvedValue({
    data: { sessions: [session], has_more: false, next_cursor: null },
  });
});

function SidebarHarness() {
  const location = useLocation();
  const [chat, setChat] = useState<ChatType>({
    sessionId: 'previous-session',
    name: 'Existing chat',
    messages: [],
    workflow: null,
  });

  return (
    <ChatProvider chat={chat} setChat={setChat}>
      <SidebarProvider>
        <AppSidebar currentPath={location.pathname} onSelectSession={vi.fn()} />
      </SidebarProvider>
      <output data-testid="location-state">{`${location.pathname}${location.search}`}</output>
      <output data-testid="chat-session">{chat.sessionId || 'empty'}</output>
      <output data-testid="route-state">{JSON.stringify(location.state)}</output>
    </ChatProvider>
  );
}

describe('AppSidebar chat navigation', () => {
  it('labels the standard navigation action New chat, creates an empty route, and keeps recent history one click away', async () => {
    render(
      <MemoryRouter initialEntries={['/pair?resumeSessionId=previous-session']}>
        <SidebarHarness />
      </MemoryRouter>
    );

    expect(screen.queryByTestId('sidebar-history-button')).toBeNull();
    const newSessionButton = screen.getByTestId('sidebar-new-chat-button');
    const homeButton = screen.getByTestId('sidebar-home-button');
    const settingsButton = screen.getByTestId('sidebar-settings-button');
    const wordmark = screen.getByTestId('sidebar-biorouter-wordmark');
    const sidebarContent = document.querySelector('[data-sidebar="content"]');
    const footer = document.querySelector('[data-sidebar="footer"]');
    const primaryMenu = homeButton.closest('[data-sidebar="menu"]');

    expect(newSessionButton).toHaveTextContent('New chat');
    // One authored row recipe for every destination (`.br-nav-row` in
    // `sidebar.css`): 28px, 13px, muted at rest, the accent rail on the current
    // row. jsdom runs no CSS, so the class contract is pinned here and the
    // measured geometry in `sidebarGeometry.browser.test.ts`.
    expect(newSessionButton).toHaveClass('br-nav-row');
    expect(newSessionButton).not.toHaveClass('h-control-md', 'text-sm', 'px-3');
    // The brand row is the wordmark SVG (D-39): "Bio" + "Router" live as the
    // SVG's own text nodes, so the accessible name is there without a <span>.
    expect(wordmark).toHaveTextContent('BioRouter');
    expect(wordmark).toHaveClass('br-sidebar-brand-row');
    expect(sidebarContent).not.toHaveClass('pt-10');
    const titlebarBand = screen.getByTestId('sidebar-titlebar-band');
    // `h-chrome`, not a literal: this band, BaseChat's header and the artifact
    // strip all read `--chrome-height` (44px) so they can never drift apart at
    // the seam they share. The container has no padding now, so the band starts
    // at y=0 with no `-mt-2` to cancel one.
    expect(titlebarBand).toHaveClass('h-chrome', 'border-b', 'border-sidebar-border');
    expect(titlebarBand).not.toHaveClass('-mt-2');
    expect(titlebarBand.compareDocumentPosition(wordmark)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);

    // The brand sits in its own 44px block (8 above, 32px row, 4 below), its
    // mark 20px tall at the icon column.
    expect(wordmark.parentElement).toHaveClass('br-sidebar-brand');
    const brandMark = screen.getByTestId('sidebar-biorouter-mark');
    expect(brandMark.tagName.toLowerCase()).toBe('svg');
    expect(brandMark).toHaveAttribute('aria-label', 'BioRouter');
    expect(brandMark).toHaveClass('br-sidebar-wordmark');
    expect(homeButton.closest('[data-sidebar="group"]')).toHaveClass('br-nav-group');
    expect(homeButton.querySelector('svg')).toHaveClass('br-nav-row-icon');

    // Astryx §4.1.3 REVERSED THIS PAIR. Home is first because it is where the
    // rail returns you; New chat is beneath it because it is the one thing
    // the rail does.
    expect(wordmark.compareDocumentPosition(homeButton)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
    expect(homeButton.compareDocumentPosition(newSessionButton)).toBe(
      Node.DOCUMENT_POSITION_FOLLOWING
    );
    expect(homeButton).toHaveClass('br-nav-row');
    expect(primaryMenu).toHaveClass('br-nav-list');
    expect(primaryMenu).toContainElement(newSessionButton);
    // New chat's shortcut is a hint the row shows on hover or focus only.
    expect(newSessionButton.querySelector('.br-nav-row-hint')).toHaveAttribute(
      'aria-hidden',
      'true'
    );
    // The ten nav tooltips that could never show are gone (F-10).
    expect(newSessionButton.closest('[data-slot="tooltip-trigger"]')).toBeNull();
    expect(footer).toContainElement(settingsButton);
    expect(settingsButton).toHaveClass('br-nav-row');
    expect(screen.getByTestId('sidebar-biorouter-mark')).toBeInTheDocument();
    expect(screen.queryByTestId('sidebar-nav-divider')).toBeNull();
    // The footer's rule is a full-bleed hairline on the footer itself (Crew's
    // `.crew-sidebar-you`), not an inset `mx-3.5 my-1` divider element.
    expect(screen.queryByTestId('sidebar-footer-divider')).toBeNull();
    expect(footer).toHaveClass('br-sidebar-footer');
    // §4.1.2 — no "MENU" header. 32px labelling something self-evident.
    expect(screen.queryByText('Menu')).toBeNull();
    expect(screen.queryByTestId('sidebar-menu-label')).toBeNull();
    expect(screen.getByText('Chats')).toBeInTheDocument();
    expect(screen.queryByText('Recents')).toBeNull();
    expect(screen.getByTestId('view-all-chat-history')).toBeInTheDocument();
    expect(await screen.findByTestId('recent-chat-session-1')).toBeInTheDocument();

    fireEvent.click(newSessionButton);
    expect(screen.getByTestId('location-state')).toHaveTextContent('/pair');
    expect(screen.getByTestId('route-state')).toHaveTextContent('newChat');
    expect(screen.getByTestId('chat-session')).toHaveTextContent('empty');

    fireEvent.click(screen.getByTestId('recent-chat-session-1'));
    expect(screen.getByTestId('location-state')).toHaveTextContent(
      '/pair?resumeSessionId=session-1'
    );
    expect(screen.getByTestId('route-state')).toHaveTextContent('"userSetName":true');

    fireEvent.click(screen.getByTestId('view-all-chat-history'));
    expect(screen.getByTestId('location-state')).toHaveTextContent('/sessions');

    fireEvent.click(settingsButton);
    expect(screen.getByTestId('location-state')).toHaveTextContent('/settings');
  });
});

/**
 * ASTRYX §4.1.3 — the rail carries one destination and one action, and the other
 * six live behind one disclosure.
 *
 * This is where the 240px comes from: nine 32px nav rows become three. jsdom
 * cannot measure that, so what is pinned here is the STRUCTURE that produces it
 * — how many rows exist, which of them are children, and whether the group
 * remembers what the user chose.
 */
describe('AppSidebar — the Components disclosure', () => {
  const renderSidebar = (path = '/pair') =>
    render(
      <MemoryRouter initialEntries={[path]}>
        <SidebarHarness />
      </MemoryRouter>
    );

  beforeEach(() => {
    window.localStorage.clear();
  });

  it('collapses by default, so the rail opens with three rows and not nine', () => {
    renderSidebar();
    expect(screen.getByTestId('sidebar-home-button')).toBeInTheDocument();
    expect(screen.getByTestId('sidebar-new-chat-button')).toBeInTheDocument();
    expect(screen.getByTestId('sidebar-components-disclosure')).toBeInTheDocument();
    // The six destinations are reachable, not resident.
    expect(screen.queryByTestId('sidebar-components-group')).toBeNull();
    expect(screen.queryByTestId('sidebar-workflows-button')).toBeNull();
    expect(screen.queryByTestId('sidebar-knowledge-button')).toBeNull();
  });

  it('opens on click, and its children are rows like any other, at the same edge', () => {
    renderSidebar();
    const disclosure = screen.getByTestId('sidebar-components-disclosure');
    expect(disclosure).toHaveAttribute('aria-expanded', 'false');
    fireEvent.click(disclosure);
    expect(disclosure).toHaveAttribute('aria-expanded', 'true');
    // The chevron is the group's mark and the only thing that moves (Crew's
    // team header): it turns 90° in CSS keyed on `aria-expanded`.
    expect(disclosure.querySelector('svg')).toHaveClass('br-nav-chevron');

    const workflows = screen.getByTestId('sidebar-workflows-button');
    expect(screen.getByTestId('sidebar-components-group')).toContainElement(workflows);
    expect(screen.getByTestId('sidebar-components-group')).toHaveClass('br-nav-list');
    // No indent (F-11): the children sit on the same icon and label columns as
    // every other row, at the same height and type.
    expect(workflows).toHaveClass('br-nav-row');
    expect(workflows).not.toHaveClass('pl-9');
    expect(workflows.className).toBe(screen.getByTestId('sidebar-home-button').className);
  });

  it('remembers being opened', () => {
    const first = renderSidebar();
    fireEvent.click(screen.getByTestId('sidebar-components-disclosure'));
    expect(screen.getByTestId('sidebar-workflows-button')).toBeInTheDocument();
    first.unmount();

    renderSidebar();
    expect(screen.getByTestId('sidebar-workflows-button')).toBeInTheDocument();
  });

  it('opens itself when the current route is one of its children — without overwriting the preference', () => {
    // A lit row inside a collapsed section is an invisible one, so being ON a
    // component route forces the group open. It must NOT persist that: leaving
    // the route has to collapse back to whatever the user chose.
    const onRoute = renderSidebar('/knowledge');
    expect(screen.getByTestId('sidebar-knowledge-button')).toBeInTheDocument();
    expect(window.localStorage.getItem('biorouter:sidebar-components-expanded')).toBeNull();
    onRoute.unmount();

    renderSidebar('/pair');
    expect(screen.queryByTestId('sidebar-knowledge-button')).toBeNull();
  });
});

describe('AppSidebar — actions do not stay lit (§4.1.3)', () => {
  it('never gives New chat the selected wash, even standing on /pair', () => {
    render(
      <MemoryRouter initialEntries={['/pair']}>
        <SidebarHarness />
      </MemoryRouter>
    );
    const newSession = screen.getByTestId('sidebar-new-chat-button');
    // A destination keeps the wash because you are still there. New chat
    // fires and the view moves on, so a lit row would claim a location that is
    // no longer true — which is how a two-row rail came to show two selections.
    expect(newSession).toHaveAttribute('data-active', 'false');
    expect(screen.getByTestId('sidebar-home-button')).toHaveAttribute('data-active', 'false');
  });

  it('still lights Home when Home is where you are', () => {
    render(
      <MemoryRouter initialEntries={['/']}>
        <SidebarHarness />
      </MemoryRouter>
    );
    expect(screen.getByTestId('sidebar-home-button')).toHaveAttribute('data-active', 'true');
  });

  it('drops focus after a POINTER click and keeps it after a keyboard activation', () => {
    render(
      <MemoryRouter initialEntries={['/pair']}>
        <SidebarHarness />
      </MemoryRouter>
    );
    const newSession = screen.getByTestId('sidebar-new-chat-button');

    newSession.focus();
    // `detail > 0` is the mouse/touch signature.
    fireEvent.click(newSession, { detail: 1 });
    expect(document.activeElement).not.toBe(newSession);

    newSession.focus();
    // Enter and Space report detail 0. Blurring here would strand a Tab user
    // mid-rail with no visible focus and nowhere obvious to resume from.
    fireEvent.click(newSession, { detail: 0 });
    expect(document.activeElement).toBe(newSession);
  });
});

/**
 * Defect 3.3. Clicking a sidebar item you are already on did nothing at all:
 * react-router reconciles a same-path navigation rather than remounting, so a
 * page holding sub-state (a schedule's run detail) stayed exactly where it was
 * and only the in-page Back escaped. The row is lit — the user reads that as
 * "this is the destination" and expects the destination, not the sub-view they
 * drilled into.
 */
describe('clicking the sidebar item you are already on', () => {
  // Scheduler lives behind the collapsed `Components` disclosure.
  beforeEach(() => {
    window.localStorage.setItem('biorouter:sidebar-components-expanded', 'true');
  });

  it('announces a reset for that route instead of being a dead click', () => {
    const seen: string[] = [];
    const listener = (event: Event) =>
      seen.push((event as CustomEvent<{ path: string }>).detail.path);
    window.addEventListener(SAME_ROUTE_RESET_EVENT, listener);

    try {
      render(
        <MemoryRouter initialEntries={['/schedules']}>
          <SidebarHarness />
        </MemoryRouter>
      );

      fireEvent.click(screen.getByTestId('sidebar-scheduler-button'));

      expect(seen).toEqual(['/schedules']);
    } finally {
      window.removeEventListener(SAME_ROUTE_RESET_EVENT, listener);
    }
  });

  // Navigating somewhere else is a navigation, not a reset — otherwise every
  // arrival would clear state the destination is entitled to keep.
  it('says nothing when the click is a real navigation', () => {
    const seen: string[] = [];
    const listener = () => seen.push('reset');
    window.addEventListener(SAME_ROUTE_RESET_EVENT, listener);

    try {
      render(
        <MemoryRouter initialEntries={['/']}>
          <SidebarHarness />
        </MemoryRouter>
      );

      fireEvent.click(screen.getByTestId('sidebar-scheduler-button'));

      expect(seen).toEqual([]);
      expect(screen.getByTestId('location-state')).toHaveTextContent('/schedules');
    } finally {
      window.removeEventListener(SAME_ROUTE_RESET_EVENT, listener);
    }
  });
});

/**
 * The real rail in the overlay a narrow window gets, driven by the keyboard the way Erin drove
 * it in live QA round 4. The toggle sits before the panel, as the titlebar's does.
 */
function OverlayHarness() {
  const location = useLocation();
  const [chat, setChat] = useState<ChatType>({
    sessionId: 'previous-session',
    name: 'Existing chat',
    messages: [],
    workflow: null,
  });

  return (
    <ChatProvider chat={chat} setChat={setChat}>
      <SidebarProvider defaultOpen={false}>
        <SidebarTrigger />
        <Sidebar variant="inset" collapsible="offcanvas">
          <AppSidebar currentPath={location.pathname} onSelectSession={vi.fn()} />
        </Sidebar>
        <SidebarInset>
          <main>
            <p>{`Page ${location.pathname}`}</p>
          </main>
        </SidebarInset>
      </SidebarProvider>
    </ChatProvider>
  );
}

describe('AppSidebar in the overlay, by keyboard', () => {
  beforeEach(() => {
    document.body.classList.add(SIDEBAR_OVERLAY_BODY_CLASS);
  });

  afterEach(() => {
    document.body.classList.remove(SIDEBAR_OVERLAY_BODY_CLASS);
  });

  const toggle = () => screen.getByRole('button', { name: 'Toggle sidebar' });
  const sidebarState = () =>
    document.querySelector('[data-slot="sidebar"]')?.getAttribute('data-state');

  const openOverlayFromToggle = async (user: ReturnType<typeof userEvent.setup>) => {
    toggle().focus();
    await user.keyboard('{Enter}');
    expect(sidebarState()).toBe('expanded');
    expect(toggle()).toHaveFocus();
  };

  // Q4-53: each row's tooltip ("Go back to the main chat screen") is for the collapsed icon rail.
  // In the overlay it was hidden but still opened on focus, and took the first Escape.
  it('closes with one Escape after tabbing onto Home, and hands focus to the toggle', async () => {
    const user = userEvent.setup();
    render(
      <MemoryRouter initialEntries={['/crew']}>
        <OverlayHarness />
      </MemoryRouter>
    );
    await openOverlayFromToggle(user);

    await user.tab();
    expect(screen.getByTestId('sidebar-home-button')).toHaveFocus();
    expect(document.querySelectorAll('[data-slot="tooltip-content"]')).toHaveLength(0);

    await user.keyboard('{Escape}');
    expect(sidebarState()).toBe('collapsed');
    expect(toggle()).toHaveFocus();
  });

  // Q4-54: choosing Crew used to hand focus back to the toggle, so Erin arrived in Crew with
  // her place up in the titlebar. The page takes it now.
  it('hands focus to the page, not the toggle, when Crew is chosen', async () => {
    const user = userEvent.setup();
    render(
      <MemoryRouter initialEntries={['/']}>
        <OverlayHarness />
      </MemoryRouter>
    );
    await openOverlayFromToggle(user);

    await user.tab();
    await user.tab();
    await user.tab();
    expect(screen.getByTestId('sidebar-crew-button')).toHaveFocus();

    await user.keyboard('{Enter}');
    expect(screen.getByText('Page /crew')).toBeInTheDocument();
    expect(sidebarState()).toBe('collapsed');
    expect(screen.getByRole('main')).toHaveFocus();
    expect(toggle()).not.toHaveFocus();
  });
});
