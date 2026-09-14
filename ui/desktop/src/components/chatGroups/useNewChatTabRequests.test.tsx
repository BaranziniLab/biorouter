import { describe, it, expect, vi } from 'vitest';
import { act, render } from '@testing-library/react';
import { MemoryRouter, useLocation, useNavigate, type NavigateFunction } from 'react-router-dom';
import { useNewChatTabRequests } from './useNewChatTabRequests';
import type { ChatGroupsAction } from './chatGroupsReducer';

/**
 * Which New chat is an ARRIVAL. Only the navigation /pair mounted on: every
 * later press was made while /pair was on screen and must open a tab.
 */

let navigateTo: NavigateFunction | null = null;

function PairRoute({ dispatch }: { dispatch: (action: ChatGroupsAction) => void }) {
  const location = useLocation();
  navigateTo = useNavigate();
  const isNewChat = (location.state as { newChat?: boolean } | null)?.newChat === true;
  useNewChatTabRequests(isNewChat, dispatch);
  return null;
}

const resumeUnsentOf = (dispatch: ReturnType<typeof vi.fn>) =>
  dispatch.mock.calls.map(
    ([action]) => (action as Extract<ChatGroupsAction, { type: 'openTab' }>).payload.resumeUnsent
  );

describe('useNewChatTabRequests', () => {
  it('treats the navigation /pair mounted on as an arrival', () => {
    const dispatch = vi.fn();
    render(
      <MemoryRouter initialEntries={[{ pathname: '/pair', state: { newChat: true } }]}>
        <PairRoute dispatch={dispatch} />
      </MemoryRouter>
    );
    expect(resumeUnsentOf(dispatch)).toEqual([true]);
  });

  it('a New chat pressed on /pair after arriving from Recents opens a tab (D3)', () => {
    // Measured: Home → a Recents chat → click a tab holding a draft → sidebar New
    // chat. The strip did not change; the second press opened a tab.
    const dispatch = vi.fn();
    render(
      <MemoryRouter initialEntries={['/pair?resumeSessionId=s-recent']}>
        <PairRoute dispatch={dispatch} />
      </MemoryRouter>
    );
    expect(dispatch).not.toHaveBeenCalled();

    act(() => navigateTo!('/pair', { state: { newChat: true } }));

    expect(resumeUnsentOf(dispatch)).toEqual([false]);
  });

  it('every later press opens a tab, one per navigation', () => {
    const dispatch = vi.fn();
    render(
      <MemoryRouter initialEntries={[{ pathname: '/pair', state: { newChat: true } }]}>
        <PairRoute dispatch={dispatch} />
      </MemoryRouter>
    );
    act(() => navigateTo!('/pair', { state: { newChat: true } }));
    act(() => navigateTo!('/pair', { state: { newChat: true } }));

    expect(resumeUnsentOf(dispatch)).toEqual([true, false, false]);
  });
});
