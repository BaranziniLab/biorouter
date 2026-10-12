import { act, render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  ChatGroupsProvider,
  resetLiveChatTabsForTests,
  useChatGroups,
} from '../../contexts/ChatGroupsContext';
import { announceSessionName } from '../../utils/sessionNameSync';
import { chatGroupsReducer, createInitialChatGroupsState } from './chatGroupsReducer';
import { getWindowId, loadChatGroups, saveChatGroups } from './chatGroupsStorage';
import { resetNewTabRegistry } from './newTabRegistry';
import { useSavedTabTitleSync } from './useSavedTabTitleSync';

/**
 * V2-R3-1: a rename made while `/pair` is not mounted reaches the chat's tab.
 *
 * The tab store mounts with `/pair` only, and its rename subscription with it.
 * The sidebar is on every route, so renaming a chat from its sidebar row on
 * History left the sidebar and History on the new name and the tab (and the
 * strip's overflow list) on the old one, across returns to the chat and across
 * a reload; it corrected only when the tab was made active. Reproduced twice in
 * the running app.
 *
 * Each case renders what the app does: the app root, which lives as long as the
 * renderer, and a `/pair` that mounts and unmounts as the person changes route.
 * The name channel is the real one.
 */

// The provider reads the live-turn registry; nothing here runs a turn.
vi.mock('../../hooks/chatStreamStore', () => ({
  useRunningChats: () => [],
  defaultChatStreamRegistry: { peekController: () => undefined },
}));

function AppRootSync() {
  useSavedTabTitleSync();
  return null;
}

/** Every tab's chat and title, in strip order. */
function TabTitles() {
  const groups = useChatGroups();
  const titles = Object.values(groups?.state.groups ?? {}).flatMap((group) =>
    group.tabs.map((tab) => `${tab.sessionId}: ${tab.title}`)
  );
  return <span data-testid="tab-titles">{titles.join(' | ')}</span>;
}

function App({ onPair, appRoot = true }: { onPair: boolean; appRoot?: boolean }) {
  return (
    <MemoryRouter initialEntries={['/pair']}>
      {appRoot && <AppRootSync />}
      {onPair && (
        <ChatGroupsProvider>
          <TabTitles />
        </ChatGroupsProvider>
      )}
    </MemoryRouter>
  );
}

const OLD = 'Knowledge base smoke test';
const NEW = 'Knowledge base smoke test Y';

/** Two open chats, as the provider saved them the last time `/pair` showed. */
function seedSavedTabs() {
  const state = [
    { sessionId: 's1', title: OLD, userSetName: true },
    { sessionId: 's2', title: 'Poem and shell command' },
  ].reduce(
    (current, payload) => chatGroupsReducer(current, { type: 'openTab', payload }),
    createInitialChatGroupsState()
  );
  saveChatGroups(state, getWindowId());
}

const tabTitles = () => screen.getByTestId('tab-titles').textContent;

function renameFromSidebar(name: string, origin: 'user' | 'sync' = 'user') {
  act(() => announceSessionName({ sessionId: 's1', name, userSetName: true, origin }));
}

describe('useSavedTabTitleSync: a rename on another route reaches the tab', () => {
  beforeEach(() => {
    localStorage.clear();
    sessionStorage.clear();
    resetLiveChatTabsForTests();
    resetNewTabRegistry();
    seedSavedTabs();
  });

  it('shows the new name in the tab when /pair mounts again', () => {
    const view = render(<App onPair />);
    expect(tabTitles()).toBe(`s1: ${OLD} | s2: Poem and shell command`);

    // To History, rename from the sidebar row, back to the chat.
    view.rerender(<App onPair={false} />);
    renameFromSidebar(NEW);
    view.rerender(<App onPair />);

    expect(tabTitles()).toBe(`s1: ${NEW} | s2: Poem and shell command`);
  });

  it('(control) without the app root sync, the tab comes back under its old name', () => {
    // The bug as measured: only the provider listened, and it was not mounted.
    const view = render(<App onPair appRoot={false} />);
    view.rerender(<App onPair={false} appRoot={false} />);
    renameFromSidebar(NEW);
    view.rerender(<App onPair appRoot={false} />);

    expect(tabTitles()).toBe(`s1: ${OLD} | s2: Poem and shell command`);
  });

  it('keeps the new name across a reload', () => {
    // Renamed on History before /pair ever mounted in this renderer.
    const view = render(<App onPair={false} />);
    renameFromSidebar(NEW);
    view.unmount();

    // A new renderer: nothing in memory, only the saved record.
    resetLiveChatTabsForTests();
    const tabs = Object.values(loadChatGroups(getWindowId())!.groups).flatMap((g) => g.tabs);
    expect(tabs.find((tab) => tab.sessionId === 's1')?.title).toBe(NEW);

    render(<App onPair />);
    expect(tabTitles()).toBe(`s1: ${NEW} | s2: Poem and shell command`);
  });

  it('puts the old name back when the rename is refused', () => {
    // `renameSessionOptimistically`: the new name at once, then the previous
    // one announced again when the daemon refuses.
    const view = render(<App onPair={false} />);
    renameFromSidebar(NEW);
    renameFromSidebar(OLD, 'sync');
    view.rerender(<App onPair />);

    expect(tabTitles()).toBe(`s1: ${OLD} | s2: Poem and shell command`);
  });

  it('still renames a mounted tab at once, and the saved record agrees', () => {
    render(<App onPair />);
    renameFromSidebar(NEW);

    expect(tabTitles()).toBe(`s1: ${NEW} | s2: Poem and shell command`);
    const tabs = Object.values(loadChatGroups(getWindowId())!.groups).flatMap((g) => g.tabs);
    expect(tabs.find((tab) => tab.sessionId === 's1')?.title).toBe(NEW);
  });
});
