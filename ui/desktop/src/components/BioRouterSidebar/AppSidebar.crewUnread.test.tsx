import { act, render, screen } from '@testing-library/react';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { beforeAll, describe, expect, it, vi } from 'vitest';
import { ChatProvider } from '../../contexts/ChatContext';
import { SidebarProvider } from '../ui/sidebar';

const mocks = vi.hoisted(() => ({
  unread: 0,
  options: null as null | { onCrewRoute: boolean; onOpenChannel(a: string, b: string): void },
}));

vi.mock('../../api', () => ({
  listSessions: vi.fn(async () => ({ data: { sessions: [] } })),
  listSidebarSessions: vi.fn(async () => ({
    data: { sessions: [], has_more: false, next_cursor: null },
  })),
}));
vi.mock('../../hooks/chatStreamStore', () => ({ useRunningChats: () => [] }));
vi.mock('./SidebarUpdateButton', () => ({ default: () => null }));
vi.mock('../crew/attention/useCrewAttention', () => ({
  useCrewAttention: (options: NonNullable<typeof mocks.options>) => {
    mocks.options = options;
    return mocks.unread;
  },
}));

import AppSidebar from './AppSidebar';

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

function Where() {
  return <span data-testid="where">{useLocation().pathname}</span>;
}

function renderSidebar(path = '/') {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <ChatProvider
        chat={{ sessionId: 's', name: 'x', messages: [], workflow: null }}
        setChat={vi.fn()}
        contextKey="test"
      >
        <SidebarProvider>
          <AppSidebar onSelectSession={vi.fn()} currentPath={path} />
          <Where />
        </SidebarProvider>
      </ChatProvider>
    </MemoryRouter>
  );
}

describe('the Crew item counts unread Crew messages (M2)', () => {
  it('shows nothing when there is nothing unread', () => {
    mocks.unread = 0;
    renderSidebar();
    expect(screen.getByTestId('sidebar-crew-button')).toHaveAccessibleName('Crew');
  });

  it('shows the count, capped, and says it to assistive technology', () => {
    mocks.unread = 120;
    renderSidebar();
    const crew = screen.getByTestId('sidebar-crew-button');
    expect(crew).toHaveAccessibleName('Crew, 120 unread');
    expect(crew).toHaveTextContent('99+');
  });

  it('opens Crew when a notification is clicked, and tells the watcher which route shows', async () => {
    mocks.unread = 1;
    renderSidebar('/');
    expect(mocks.options?.onCrewRoute).toBe(false);
    act(() => mocks.options?.onOpenChannel('conn-1', 'c-general'));
    expect(screen.getByTestId('where')).toHaveTextContent('/crew');
  });
});
