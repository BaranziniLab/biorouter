import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, act, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { ChatGroupsProvider, useChatGroups } from './ChatGroupsContext';
import { resetNewTabRegistry } from '../components/chatGroups/newTabRegistry';
import {
  composerQueueKey,
  parkComposerQueue,
  readParkedComposerQueue,
  resetComposerQueuesForTests,
  type QueuedMessage,
} from '../utils/composerQueues';

vi.mock('../hooks/chatStreamStore', () => ({
  useRunningChats: () => [],
  defaultChatStreamRegistry: { peekController: () => undefined },
}));
vi.mock('../utils/sessionNameSync', () => ({ subscribeSessionNameChanges: () => () => undefined }));

/**
 * A chat's queued messages, from the side of the tab strip: they are kept while
 * the chat is open in a tab, through a trip away from /pair, and dropped — with
 * the temp images they owned — when the tab closes. Without that bound, a
 * message queued behind a turn and then abandoned by closing its tab would be
 * SENT the next time the chat was opened, possibly days later.
 */

let ctx: ReturnType<typeof useChatGroups> = null;
function Probe() {
  ctx = useChatGroups();
  const tabs = ctx ? Object.values(ctx.state.groups).flatMap((group) => group.tabs) : [];
  return <span data-testid="tabs">{tabs.map((t) => `${t.tabId}=${t.sessionId}`).join(',')}</span>;
}

const mount = () =>
  render(
    <MemoryRouter initialEntries={['/pair']}>
      <ChatGroupsProvider>
        <Probe />
      </ChatGroupsProvider>
    </MemoryRouter>
  );

const deleteTempFile = vi.fn();

const queued = (id: string, ownedImage?: string): QueuedMessage => ({
  id,
  content: `queued ${id}`,
  attachments: ownedImage ? [{ path: ownedImage, kind: 'image' }] : [],
  ownedTempAttachmentPaths: ownedImage ? [ownedImage] : [],
  timestamp: 1,
});

const park = (sessionId: string, message: QueuedMessage) =>
  parkComposerQueue(composerQueueKey(sessionId)!, {
    messages: [message],
    paused: false,
    interruption: null,
    sendWhenIdle: true,
  });

const tabOf = (sessionId: string) =>
  Object.values(ctx!.state.groups)
    .flatMap((group) => group.tabs)
    .find((tab) => tab.sessionId === sessionId)!.tabId;

beforeEach(() => {
  localStorage.clear();
  sessionStorage.clear();
  resetNewTabRegistry();
  resetComposerQueuesForTests();
  deleteTempFile.mockReset();
  Object.assign(window, { electron: { ...(window.electron ?? {}), deleteTempFile } });
});

async function twoChatsOpen() {
  const view = mount();
  act(() => ctx!.dispatch({ type: 'openTab', payload: { sessionId: 's-queued', title: 'q' } }));
  act(() => ctx!.dispatch({ type: 'openTab', payload: { sessionId: 's-other', title: 'o' } }));
  await waitFor(() => expect(screen.getByTestId('tabs').textContent).toContain('s-other'));
  return view;
}

describe("a chat's queued messages", () => {
  it('are kept while the chat is open in a tab, including through a layout change', async () => {
    await twoChatsOpen();
    park('s-queued', queued('m1', '/tmp/queued-capture.png'));

    act(() => ctx!.dispatch({ type: 'activateTab', tabId: tabOf('s-other') }));
    act(() => ctx!.dispatch({ type: 'activateTab', tabId: tabOf('s-queued') }));

    expect(readParkedComposerQueue(composerQueueKey('s-queued')!)?.messages).toHaveLength(1);
    expect(deleteTempFile).not.toHaveBeenCalled();
  });

  it('are kept through leaving /pair and coming back', async () => {
    const view = await twoChatsOpen();
    park('s-queued', queued('m1'));

    act(() => view.unmount());
    mount();
    await waitFor(() => expect(screen.getByTestId('tabs').textContent).toContain('s-queued'));

    expect(readParkedComposerQueue(composerQueueKey('s-queued')!)?.messages).toHaveLength(1);
  });

  it('are dropped, with the temp images they owned, when the tab is closed', async () => {
    await twoChatsOpen();
    park('s-queued', queued('m1', '/tmp/queued-capture.png'));
    park('s-other', queued('m2', '/tmp/other-capture.png'));

    act(() => ctx!.dispatch({ type: 'closeTab', tabId: tabOf('s-queued') }));

    await waitFor(() =>
      expect(readParkedComposerQueue(composerQueueKey('s-queued')!)).toBeUndefined()
    );
    expect(deleteTempFile).toHaveBeenCalledWith('/tmp/queued-capture.png');
    // Only that chat's.
    expect(readParkedComposerQueue(composerQueueKey('s-other')!)?.messages).toHaveLength(1);
    expect(deleteTempFile).not.toHaveBeenCalledWith('/tmp/other-capture.png');
  });
});
