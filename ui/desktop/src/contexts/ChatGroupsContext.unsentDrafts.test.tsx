import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, act, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { ChatGroupsProvider, useChatGroups } from './ChatGroupsContext';
import { requestNewTab, resetNewTabRegistry } from '../components/chatGroups/newTabRegistry';
import {
  composerDraftKeyForTab,
  hasComposerDraft,
  resetComposerDraftsForTests,
  saveComposerDraft,
} from '../utils/composerDrafts';

vi.mock('../hooks/chatStreamStore', () => ({
  useRunningChats: () => [],
  defaultChatStreamRegistry: { peekController: () => undefined },
}));
vi.mock('../utils/sessionNameSync', () => ({ subscribeSessionNameChanges: () => () => undefined }));

/**
 * A new tab's unsent message, from the side of the tab strip: the tab that holds
 * it is not discarded by leaving /pair, arriving back focuses it, and closing or
 * binding the tab is what releases it.
 *
 * Measured on 1.90.4 with a failed start's toast still reading "Your message was
 * kept.": click Settings, come back to /pair, and the new-chat tab was gone from
 * the strip. `ChatGroupsProvider` is mounted under /pair only, so leaving it is
 * an unmount and coming back re-reads the layout from storage, where a tab with
 * no chat was pruned. Here that trip is exactly that: unmount, mount.
 */

let ctx: ReturnType<typeof useChatGroups> = null;
function Probe() {
  ctx = useChatGroups();
  const group = ctx?.activeGroup;
  return (
    <div>
      <span data-testid="tabs">{group?.tabs.map((t) => t.tabId).join(',') ?? ''}</span>
      <span data-testid="active">{group?.activeTabId ?? ''}</span>
    </div>
  );
}

const mount = () =>
  render(
    <MemoryRouter initialEntries={['/pair']}>
      <ChatGroupsProvider>
        <Probe />
      </ChatGroupsProvider>
    </MemoryRouter>
  );

const tabIds = () => screen.getByTestId('tabs').textContent!.split(',').filter(Boolean);

const deleteTempFile = vi.fn();

beforeEach(() => {
  localStorage.clear();
  sessionStorage.clear();
  resetNewTabRegistry();
  resetComposerDraftsForTests();
  deleteTempFile.mockReset();
  Object.assign(window, { electron: { ...(window.electron ?? {}), deleteTempFile } });
});

/** Two new tabs; the FIRST holds an unsent message and is the one in view. */
const twoNewTabsFirstUnsent = async () => {
  const view = mount();
  act(() => void requestNewTab());
  act(() => void requestNewTab());
  await waitFor(() => expect(tabIds()).toHaveLength(2));
  const [unsent, blank] = tabIds();
  saveComposerDraft(composerDraftKeyForTab(unsent), {
    text: 'Your message was kept.',
    images: [{ id: 'img-1', filePath: '/tmp/kept.png', dataUrl: 'data:x' }],
    files: [],
  });
  act(() => ctx!.dispatch({ type: 'activateTab', tabId: unsent }));
  await waitFor(() => expect(screen.getByTestId('active')).toHaveTextContent(unsent));
  return { view, unsent, blank };
};

describe('a new tab holding an unsent message', () => {
  it('is still in the strip, and in view, after leaving /pair and coming back', async () => {
    const { view, unsent, blank } = await twoNewTabsFirstUnsent();

    act(() => view.unmount());
    mount();

    expect(tabIds()).toContain(unsent);
    // A blank tab is still pruned, as it always was.
    expect(tabIds()).not.toContain(blank);
    expect(screen.getByTestId('active')).toHaveTextContent(unsent);
    expect(hasComposerDraft(composerDraftKeyForTab(unsent))).toBe(true);
  });

  it('is focused, not joined by a blank tab, when Cmd+T brings the person back', async () => {
    const { view, unsent } = await twoNewTabsFirstUnsent();
    act(() => view.unmount());

    // Cmd+T with no provider mounted is remembered; the next provider cashes it.
    act(() => void requestNewTab());
    mount();

    await waitFor(() => expect(screen.getByTestId('active')).toHaveTextContent(unsent));
    expect(tabIds()).toEqual([unsent]);
  });

  it('is gone after a RELOAD, which restores nothing', async () => {
    const { view, unsent } = await twoNewTabsFirstUnsent();
    act(() => view.unmount());

    // A reload is a new renderer: the drafts are memory, the layout is storage.
    resetComposerDraftsForTests();
    mount();

    expect(tabIds()).not.toContain(unsent);
  });

  it('releases its draft, and the image it owned, when the tab is closed', async () => {
    const { unsent } = await twoNewTabsFirstUnsent();

    act(() => ctx!.dispatch({ type: 'closeTab', tabId: unsent }));

    await waitFor(() => expect(hasComposerDraft(composerDraftKeyForTab(unsent))).toBe(false));
    expect(deleteTempFile).toHaveBeenCalledWith('/tmp/kept.png');
  });

  it('releases its draft when the tab binds to a chat', async () => {
    const { unsent } = await twoNewTabsFirstUnsent();

    act(() => ctx!.dispatch({ type: 'bindSession', tabId: unsent, sessionId: 'sess-1' }));

    await waitFor(() => expect(hasComposerDraft(composerDraftKeyForTab(unsent))).toBe(false));
  });
});
