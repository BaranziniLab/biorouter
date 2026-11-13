import { act, render, screen } from '@testing-library/react';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { beforeAll, describe, expect, it, vi } from 'vitest';
import { ChatProvider } from '../../contexts/ChatContext';
import { SidebarProvider } from '../ui/sidebar';
import { crewSelectIntent } from '../crew/state/useCrewConnections';

/**
 * Chat parity WI-16: a clicked Crew notification opens Crew on its workspace and channel with the
 * request a chat's "Open in Crew" makes (`crewSelect`), whichever workspace Crew showed, and also
 * when Crew is already open. It used to open Crew on whatever was selected, and do nothing at all
 * while Crew was showing.
 */

const mocks = vi.hoisted(() => ({
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
    return 1;
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

let seen: { path: string; state: unknown } | null = null;
function Where() {
  const location = useLocation();
  seen = { path: location.pathname, state: location.state };
  return <span data-testid="where">{location.pathname}</span>;
}

function renderSidebar(path: string) {
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

describe('a clicked Crew notification opens its channel (WI-16)', () => {
  it('opens Crew with a request for that workspace and channel', () => {
    renderSidebar('/');
    act(() => mocks.options?.onOpenChannel('conn-1', 'c-general'));
    expect(screen.getByTestId('where')).toHaveTextContent('/crew');
    expect(crewSelectIntent(seen?.state)).toMatchObject({
      connectionId: 'conn-1',
      channelId: 'c-general',
    });
  });

  it('tells a Crew already open, with a new request each time', () => {
    renderSidebar('/crew');
    act(() => mocks.options?.onOpenChannel('conn-1', 'c-general'));
    const first = crewSelectIntent(seen?.state);
    expect(first).toMatchObject({ connectionId: 'conn-1', channelId: 'c-general' });
    act(() => mocks.options?.onOpenChannel('conn-2', 'c-methods'));
    const second = crewSelectIntent(seen?.state);
    expect(second).toMatchObject({ connectionId: 'conn-2', channelId: 'c-methods' });
    expect(second?.intentId).not.toBe(first?.intentId);
  });

  it('opens plain Crew for IDs of the wrong shape', () => {
    renderSidebar('/');
    act(() => mocks.options?.onOpenChannel('../etc', 'c-general'));
    expect(screen.getByTestId('where')).toHaveTextContent('/crew');
    expect(crewSelectIntent(seen?.state)).toBeNull();
  });
});
