import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, act, waitFor } from '@testing-library/react';
import {
  MemoryRouter,
  useNavigate,
  type InitialEntry,
  type NavigateFunction,
} from 'react-router-dom';
import type { ChatGroupsState } from '../components/chatGroups/chatGroupsTypes';
import { ChatGroupsProvider, resetLiveChatTabsForTests, useChatGroups } from './ChatGroupsContext';
import { requestNewTab, resetNewTabRegistry } from '../components/chatGroups/newTabRegistry';
import {
  beginComposerSend,
  existingChatComposerDraftKey,
  composerDraftKeyForTab,
  hasComposerDraft,
  readComposerDraft,
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
let navigateTo: NavigateFunction | null = null;
/** Every state the probe rendered, in order: `[0]` is the provider's FIRST state. */
let rendered: ChatGroupsState[] = [];
function Probe() {
  ctx = useChatGroups();
  navigateTo = useNavigate();
  if (ctx) rendered.push(ctx.state);
  const group = ctx?.activeGroup;
  return (
    <div>
      <span data-testid="tabs">{group?.tabs.map((t) => t.tabId).join(',') ?? ''}</span>
      <span data-testid="sessions">
        {group?.tabs.map((t) => `${t.tabId}=${t.sessionId}`).join(',') ?? ''}
      </span>
      <span data-testid="active">{group?.activeTabId ?? ''}</span>
    </div>
  );
}

const mount = (entry: InitialEntry = '/pair') =>
  render(
    <MemoryRouter initialEntries={[entry]}>
      <ChatGroupsProvider>
        <Probe />
      </ChatGroupsProvider>
    </MemoryRouter>
  );

const tabIds = () => screen.getByTestId('tabs').textContent!.split(',').filter(Boolean);

const deleteTempFile = vi.fn();

beforeEach(() => {
  rendered = [];
  resetLiveChatTabsForTests();
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

/**
 * The layout the pane-swap defect was measured in: the left pane SHOWING a draft,
 * the right pane showing a chat with a second draft BEHIND it, and the right
 * pane focused.
 */
const splitWithDraftBehindChat = async () => {
  const view = mount();
  act(() => void requestNewTab());
  await waitFor(() => expect(tabIds()).toHaveLength(1));
  const [leftDraft] = tabIds();
  act(() => ctx!.dispatch({ type: 'openTab', payload: { sessionId: 's-chat', title: 'chat' } }));
  await waitFor(() => expect(tabIds()).toHaveLength(2));
  const chat = tabIds()[1];
  act(() =>
    ctx!.dispatch({ type: 'moveTabToGroup', tabId: chat, targetGroupId: 'grp-1', zone: 'right' })
  );
  const right = Object.keys(ctx!.state.groups).find((id) => id !== 'grp-1')!;
  act(() => ctx!.dispatch({ type: 'openTab', payload: { sessionId: '', groupId: right } }));
  const rightDraft = ctx!.state.groups[right].tabs[1].tabId;
  act(() => ctx!.dispatch({ type: 'activateTab', tabId: chat }));
  for (const [tabId, text] of [
    [leftDraft, 'LEFT PANE DRAFT'],
    [rightDraft, 'RIGHT BACKGROUND DRAFT'],
  ]) {
    saveComposerDraft(composerDraftKeyForTab(tabId), { text, images: [], files: [] });
  }
  const shown = (state: ChatGroupsState) =>
    Object.fromEntries(Object.values(state.groups).map((g) => [g.groupId, g.activeTabId]));
  expect(shown(ctx!.state)).toEqual({ 'grp-1': leftDraft, [right]: chat });
  expect(ctx!.state.activeGroupId).toBe(right);
  return { view, leftDraft, chat, right, rightDraft, shown };
};

describe('a new tab holding an unsent message', () => {
  it('is still in the strip, and in view, after leaving /pair and coming back', async () => {
    const { view, unsent, blank } = await twoNewTabsFirstUnsent();

    act(() => view.unmount());
    mount();

    expect(tabIds()).toContain(unsent);
    // Settings preserves every live tab, including an empty one.
    expect(tabIds()).toContain(blank);
    expect(screen.getByTestId('active')).toHaveTextContent(unsent);
    expect(hasComposerDraft(composerDraftKeyForTab(unsent))).toBe(true);
  });

  it('is focused, not joined by a blank tab, when Cmd+T brings the person back', async () => {
    const { view, unsent, blank } = await twoNewTabsFirstUnsent();
    act(() => view.unmount());

    // Cmd+T with no provider mounted is remembered; the next provider cashes it.
    act(() => void requestNewTab());
    mount();

    await waitFor(() => expect(screen.getByTestId('active')).toHaveTextContent(unsent));
    expect(tabIds()).toEqual([unsent, blank]);
  });

  it('in a split, coming back focuses the draft on screen and no pane loses its chat', async () => {
    // Measured in the dev app on 7200e293: left pane showing "LEFT PANE DRAFT";
    // right pane showing the chat "Prompt injection test", a new tab holding
    // "RIGHT BACKGROUND DRAFT" behind it. Settings, then New chat: the right
    // pane's chat was replaced by its background tab.
    const { view, leftDraft, chat, right, rightDraft, shown } = await splitWithDraftBehindChat();

    act(() => view.unmount());
    act(() => void requestNewTab());
    mount();

    await waitFor(() => expect(ctx!.state.activeGroupId).toBe('grp-1'));
    expect(shown(ctx!.state)).toEqual({ 'grp-1': leftDraft, [right]: chat });
    expect(ctx!.state.groups[right].tabs.map((t) => t.tabId)).toEqual([chat, rightDraft]);
    expect(readComposerDraft(composerDraftKeyForTab(rightDraft))?.text).toBe(
      'RIGHT BACKGROUND DRAFT'
    );
  });

  const bySidebar = (): InitialEntry => ({ pathname: '/pair', state: { newChat: true } });
  const byRememberedCmdT = (): InitialEntry => {
    act(() => void requestNewTab());
    return '/pair';
  };
  it.each([
    ['the sidebar’s New chat', bySidebar],
    ['a remembered Cmd+T', byRememberedCmdT],
  ] as const)(
    'an arrival by %s is in the FIRST state, before any pane mounts',
    async (_, arrive) => {
      // Resolved after mount, the arrival raced every pane's composer taking the
      // focus as it mounted: measured in the dev app, the left pane's draft was
      // resumed and the right pane's chat still ended up focused, with the caret.
      const { view, leftDraft, chat, right, shown } = await splitWithDraftBehindChat();
      act(() => view.unmount());
      rendered = [];

      mount(arrive());

      expect(rendered[0].activeGroupId).toBe('grp-1');
      expect(shown(rendered[0])).toEqual({ 'grp-1': leftDraft, [right]: chat });
      await waitFor(() => expect(ctx!.state.activeGroupId).toBe('grp-1'));
      expect(shown(ctx!.state)).toEqual({ 'grp-1': leftDraft, [right]: chat });
    }
  );

  it('a mount that is not an arrival keeps the layout it loaded', async () => {
    const { view, chat, right } = await splitWithDraftBehindChat();
    act(() => view.unmount());
    rendered = [];

    mount('/pair?resumeSessionId=s-chat');

    expect(rendered[0].activeGroupId).toBe(right);
    expect(rendered[0].groups[right].activeTabId).toBe(chat);
  });

  it('is gone after a RELOAD, which restores nothing', async () => {
    const { view, unsent } = await twoNewTabsFirstUnsent();
    act(() => view.unmount());

    // A reload is a new renderer: the drafts are memory, the layout is storage.
    resetComposerDraftsForTests();
    resetLiveChatTabsForTests();
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

describe('a message whose start is still in flight (D4)', () => {
  it('keeps its tab through a trip to Settings, and is handed back into it there', async () => {
    // Measured: send from a new tab with the start held, click Settings, then
    // New chat — the tab had already been pruned (it held no draft yet), a blank
    // tab opened, and the failure's "Your message was kept." went under a key no
    // tab had, which the next layout change deleted along with its image.
    const { view, unsent, blank } = await twoNewTabsFirstUnsent();
    const key = composerDraftKeyForTab(blank);
    act(() => ctx!.dispatch({ type: 'activateTab', tabId: blank }));
    const send = beginComposerSend(key);

    act(() => view.unmount());
    act(() => void requestNewTab());
    mount();

    await waitFor(() => expect(screen.getByTestId('active')).toHaveTextContent(blank));
    expect(tabIds()).toEqual(expect.arrayContaining([unsent, blank]));
    expect(tabIds()).toHaveLength(2);

    act(() =>
      send.giveBack({
        text: 'failed while I was away',
        images: [{ id: 'img-9', filePath: '/tmp/away.png', dataUrl: 'data:x' }],
        files: [],
      })
    );
    // A layout change after the give-back does not release it: its tab exists.
    act(() => ctx!.dispatch({ type: 'activateTab', tabId: unsent }));
    expect(readComposerDraft(key)?.text).toBe('failed while I was away');
    expect(deleteTempFile).not.toHaveBeenCalledWith('/tmp/away.png');
  });
});

describe('a chat started elsewhere does not take a tab holding an unsent message (D1)', () => {
  const arriveWithStartedChat = (sessionId: string, state: Record<string, unknown>) =>
    act(() =>
      navigateTo!(`/pair?resumeSessionId=${sessionId}`, {
        state: { resumeSessionId: sessionId, initialMessage: 'hello', ...state },
      })
    );

  it('a message sent from Home uses the empty tab beside the draft', async () => {
    const { view, unsent, blank } = await twoNewTabsFirstUnsent();
    act(() => view.unmount());
    mount();
    expect(tabIds()).toEqual([unsent, blank]);

    arriveWithStartedChat('s-home', {});

    await waitFor(() =>
      expect(screen.getByTestId('sessions').textContent).toContain(`${blank}=s-home`)
    );
    expect(tabIds()).toHaveLength(2);
    expect(screen.getByTestId('sessions').textContent).toContain(`${unsent}=`);
    expect(screen.getByTestId('sessions').textContent).not.toContain(`${unsent}=s-home`);
    expect(hasComposerDraft(composerDraftKeyForTab(unsent))).toBe(true);
    expect(deleteTempFile).not.toHaveBeenCalled();
  });

  it('a new tab’s chat binds to that tab even when another tab holding a draft is in view', async () => {
    const { unsent, blank } = await twoNewTabsFirstUnsent();
    // `blank` sent its message; the person is now looking at `unsent`.
    beginComposerSend(composerDraftKeyForTab(blank));

    arriveWithStartedChat('s-new', { originTabId: blank });

    await waitFor(() =>
      expect(screen.getByTestId('sessions').textContent).toContain(`${blank}=s-new`)
    );
    expect(screen.getByTestId('sessions').textContent).toContain(`${unsent}=,`);
    expect(hasComposerDraft(composerDraftKeyForTab(unsent))).toBe(true);
    expect(deleteTempFile).not.toHaveBeenCalled();
  });
});

describe('existing chat composer ownership', () => {
  it('retains an inactive existing-chat draft across provider remount and releases it on close', async () => {
    const view = mount();
    act(() =>
      ctx!.dispatch({
        type: 'openTab',
        payload: { sessionId: 'existing-chat', title: 'Existing chat' },
      })
    );
    await waitFor(() => expect(tabIds()).toHaveLength(1));
    const tabId = tabIds()[0];
    const key = existingChatComposerDraftKey(tabId, 'existing-chat');
    saveComposerDraft(key, { text: 'quoted follow-up', images: [], files: [] });
    act(() => void requestNewTab());
    await waitFor(() => expect(tabIds()).toHaveLength(2));
    expect(readComposerDraft(key)?.text).toBe('quoted follow-up');
    view.unmount();
    mount();
    expect(readComposerDraft(key)?.text).toBe('quoted follow-up');
    act(() => ctx!.dispatch({ type: 'closeTab', tabId }));
    await waitFor(() => expect(readComposerDraft(key)).toBeUndefined());
  });
});
