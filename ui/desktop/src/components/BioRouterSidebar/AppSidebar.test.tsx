import { fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SessionSummary } from '../../api';
import { ChatProvider } from '../../contexts/ChatContext';
import type { ChatType } from '../../types/chat';
import { SidebarProvider } from '../ui/sidebar';

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
    data: { sessions: [session], has_more: false, next_offset: null },
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
    expect(newSessionButton).toHaveClass('h-8', 'w-full', 'px-3', 'py-2', 'text-sm');
    expect(newSessionButton).not.toHaveClass(
      'h-9',
      'border',
      'bg-background-default/55',
      'font-semibold'
    );
    // The brand row is the wordmark SVG now (D-39): "Bio" + "Router" live as the
    // SVG's own text nodes, so the accessible name is still there without a
    // separate <span>.
    expect(wordmark).toHaveTextContent('BioRouter');
    expect(wordmark).toHaveClass('h-8');
    // The 40px of dead air is gone: the titlebar band now reserves that space
    // explicitly and closes it with the hairline the chat/preview headers share.
    expect(sidebarContent).not.toHaveClass('pt-10');
    const titlebarBand = screen.getByTestId('sidebar-titlebar-band');
    // `h-chrome`, not a literal: this band, BaseChat's header and the artifact
    // strip all read `--chrome-height` (44px) so they can never drift apart at
    // the seam they share. Pinning the TOKEN rather than the number is the point
    // — if the band ever goes back to a hardcoded height, this fails even if the
    // number it hardcodes happens to be right today.
    expect(titlebarBand).toHaveClass('h-chrome', 'border-b', 'border-sidebar-border');
    expect(titlebarBand.compareDocumentPosition(wordmark)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);

    // The brand is a real row on the SAME text edge as every nav label. jsdom has
    // no layout engine, so alignment is pinned here as the class contract that
    // produces it; it was measured in a real browser against compiled Tailwind:
    //   mark 20px / wordmark 44px  ==  nav icon 20px / nav label 44px
    // Both rows are `px-3` inside a `px-2` parent and put a 16px mark before a
    // `gap-2`, so 8 + 12 + 16 + 8 = 44px on each side.
    expect(wordmark.parentElement).toHaveClass('px-2');
    expect(wordmark).toHaveClass('flex', 'items-center', 'gap-2', 'px-3');
    // The mark is the wordmark SVG, carrying the accessible name and sized to sit
    // on the row (its own viewBox keeps the underline in proportion).
    const brandMark = screen.getByTestId('sidebar-biorouter-mark');
    expect(brandMark.tagName.toLowerCase()).toBe('svg');
    expect(brandMark).toHaveAttribute('aria-label', 'BioRouter');
    expect(brandMark).toHaveClass('h-[22px]', 'w-auto');
    expect(homeButton.closest('[data-sidebar="group"]')).toHaveClass('px-2');
    expect(homeButton).toHaveClass('gap-2', 'px-3');
    expect(homeButton.querySelector('svg')).toHaveClass('h-4', 'w-4');

    // Astryx §4.1.3 REVERSED THIS PAIR. Home is first because it is where the
    // rail returns you; New chat is beneath it because it is the one thing
    // the rail does. An action in the top slot claims the position the eye reads
    // as "the top of the map".
    expect(wordmark.compareDocumentPosition(homeButton)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
    expect(homeButton.compareDocumentPosition(newSessionButton)).toBe(
      Node.DOCUMENT_POSITION_FOLLOWING
    );
    expect(homeButton).toHaveClass('h-8', 'px-3', 'py-2', 'text-sm');
    // 2px between rail rows, not the flush `gap-0` this asserted before. At zero
    // the rows' rounded washes touch, so a hover bleeds into its neighbours and
    // the destinations read as one block of colour. 2px is the gap the app's own
    // menu recipe uses between items, so the rail and the menus now agree.
    expect(primaryMenu).toHaveClass('gap-0.5');
    expect(primaryMenu).toContainElement(newSessionButton);
    expect(homeButton).not.toHaveClass('text-text-muted');
    expect(footer).toContainElement(settingsButton);
    expect(settingsButton).toHaveClass('h-8', 'w-full', 'px-3', 'py-2', 'text-sm');
    expect(settingsButton).not.toHaveClass('text-text-muted');
    expect(screen.getByTestId('sidebar-biorouter-mark')).toBeInTheDocument();
    // §4.1.4 — the UPPER rule is gone. The Components row and the Recents header
    // do the zoning between destinations and history, so a rule between them was
    // a third answer to a question two elements already answered.
    expect(screen.queryByTestId('sidebar-nav-divider')).toBeNull();
    // The one remaining rule, HALVED: `my-1` + the hairline is a 10px block. At
    // `my-2` it was 18px of rail spent on a 1px mark.
    expect(screen.getByTestId('sidebar-footer-divider')).toHaveClass(
      'h-px',
      'bg-sidebar-border',
      'mx-3.5',
      'my-1'
    );
    expect(screen.getByTestId('sidebar-footer-divider')).not.toHaveClass('!w-8', 'my-2');
    // §4.1.2 — no "MENU" header. 32px labelling something self-evident.
    expect(screen.queryByText('Menu')).toBeNull();
    expect(screen.queryByTestId('sidebar-menu-label')).toBeNull();
    expect(screen.getByText('Recents')).toBeInTheDocument();
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

  it('opens on click, indents its children, and keeps them at the same 32px height', () => {
    renderSidebar();
    fireEvent.click(screen.getByTestId('sidebar-components-disclosure'));

    const workflows = screen.getByTestId('sidebar-workflows-button');
    expect(screen.getByTestId('sidebar-components-group')).toContainElement(workflows);
    // Hierarchy by INDENT, never by size (§4.1.3): the text edge moves 24px, the
    // row height and type do not move at all.
    expect(workflows).toHaveClass('h-8', 'pl-9', 'text-sm');
    expect(workflows).not.toHaveClass('h-7', 'text-xs');
    // The parent rows keep the unindented edge, so the indent reads as a step.
    expect(screen.getByTestId('sidebar-home-button')).toHaveClass('px-3');
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
