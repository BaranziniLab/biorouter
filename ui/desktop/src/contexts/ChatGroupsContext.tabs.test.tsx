import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, act, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { ChatGroupsProvider, resetLiveChatTabsForTests, useChatGroups } from './ChatGroupsContext';
import { requestNewTab, resetNewTabRegistry } from '../components/chatGroups/newTabRegistry';
import { ARTIFACT_PANEL_ATTR } from '../utils/tabCycle';

// The provider reads BOTH the hook and the registry: closing a tab detaches
// that session's observer stream (§4.3), which happens on any tab close, not
// only on a daemon frame. Supplying only `useRunningChats` leaves the registry
// undefined and the first close in this file throws.
vi.mock('../hooks/chatStreamStore', () => ({
  useRunningChats: () => [],
  defaultChatStreamRegistry: { peekController: () => undefined },
}));
vi.mock('../utils/sessionNameSync', () => ({ subscribeSessionNameChanges: () => () => undefined }));

/** Renders the strip's identity so a keystroke's effect is observable. */
function Probe() {
  const ctx = useChatGroups();
  const group = ctx?.activeGroup;
  return (
    <div>
      <span data-testid="tabs">{group?.tabs.map((t) => t.tabId).join(',') ?? ''}</span>
      <span data-testid="active">{group?.activeTabId ?? ''}</span>
    </div>
  );
}

function mount() {
  return render(
    <MemoryRouter initialEntries={['/pair']}>
      <ChatGroupsProvider>
        <Probe />
      </ChatGroupsProvider>
    </MemoryRouter>
  );
}

const ctrlTab = (target: EventTarget, shiftKey = false) => {
  const event = new KeyboardEvent('keydown', {
    key: 'Tab',
    ctrlKey: true,
    shiftKey,
    bubbles: true,
    cancelable: true,
  });
  act(() => {
    target.dispatchEvent(event);
  });
  return event;
};

const openTabs = async (n: number) => {
  for (let i = 0; i < n; i++) act(() => void requestNewTab());
  await waitFor(() =>
    expect(screen.getByTestId('tabs').textContent!.split(',').filter(Boolean)).toHaveLength(n)
  );
  return screen.getByTestId('tabs').textContent!.split(',').filter(Boolean);
};

describe('ChatGroupsProvider — the browser tab keyboard', () => {
  beforeEach(() => {
    localStorage.clear();
    resetLiveChatTabsForTests();
    resetNewTabRegistry();
  });

  it('Cmd+T opens a blank tab, and twice opens TWO (a browser never dedupes blanks)', async () => {
    mount();
    const tabs = await openTabs(2);
    expect(tabs).toHaveLength(2);
    // The newest takes focus, as a browser's new tab does.
    expect(screen.getByTestId('active')).toHaveTextContent(tabs[1]);
  });

  it('preserves empty tab identities through Settings but drops them on renderer restart', async () => {
    const view = mount();
    const tabs = await openTabs(2);
    view.unmount();
    const returned = mount();
    expect(screen.getByTestId('tabs').textContent).toBe(tabs.join(','));
    expect(screen.getByTestId('active').textContent).toBe(tabs[1]);
    returned.unmount();
    resetLiveChatTabsForTests();
    mount();
    expect(screen.getByTestId('tabs').textContent).toBe('');
  });

  it('Ctrl+Tab cycles left-to-right and wraps; Ctrl+Shift+Tab goes back', async () => {
    mount();
    const tabs = await openTabs(3);
    expect(screen.getByTestId('active')).toHaveTextContent(tabs[2]);

    // Wrap forward off the last tab onto the first.
    ctrlTab(document.body);
    await waitFor(() => expect(screen.getByTestId('active')).toHaveTextContent(tabs[0]));

    ctrlTab(document.body);
    await waitFor(() => expect(screen.getByTestId('active')).toHaveTextContent(tabs[1]));

    // Backwards, including the wrap off the first tab onto the last.
    ctrlTab(document.body, true);
    await waitFor(() => expect(screen.getByTestId('active')).toHaveTextContent(tabs[0]));
    ctrlTab(document.body, true);
    await waitFor(() => expect(screen.getByTestId('active')).toHaveTextContent(tabs[2]));
  });

  it('Ctrl+Tab still switches with the cursor in a text field (no text-input guard)', async () => {
    mount();
    const tabs = await openTabs(2);
    const textarea = document.createElement('textarea');
    document.body.appendChild(textarea);
    textarea.focus();

    ctrlTab(textarea);

    // A browser switches tabs while you type; Ctrl+Tab has no editing meaning.
    await waitFor(() => expect(screen.getByTestId('active')).toHaveTextContent(tabs[0]));
    textarea.remove();
  });

  it('does NOT answer Ctrl+Tab aimed at the preview panel — that strip owns it', async () => {
    // The arbitration, from the chat side. Both listeners are on window in the
    // capture phase, so without the focus check BOTH would answer one key: the
    // chat would jump a tab behind the user's back while they cycled previews.
    mount();
    const tabs = await openTabs(3);
    const activeBefore = screen.getByTestId('active').textContent;

    const panel = document.createElement('aside');
    panel.setAttribute(ARTIFACT_PANEL_ATTR, '');
    const insidePanel = document.createElement('button');
    panel.appendChild(insidePanel);
    document.body.appendChild(panel);

    const event = ctrlTab(insidePanel);

    expect(screen.getByTestId('active')).toHaveTextContent(activeBefore!);
    // And it must not be swallowed — the preview's own listener still needs it.
    expect(event.defaultPrevented).toBe(false);
    expect(tabs).toHaveLength(3);
    panel.remove();
  });

  it('plain Tab is left alone, so focus still moves normally', async () => {
    mount();
    const tabs = await openTabs(2);
    const activeBefore = screen.getByTestId('active').textContent;

    const event = new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true });
    act(() => void document.body.dispatchEvent(event));

    expect(screen.getByTestId('active')).toHaveTextContent(activeBefore!);
    expect(event.defaultPrevented).toBe(false);
    expect(tabs).toHaveLength(2);
  });

  it('a strip with ONE tab does not swallow Ctrl+Tab', async () => {
    // Nothing to cycle to. preventDefault here would rob Tab of its focus
    // behaviour for no gain.
    mount();
    await openTabs(1);
    const event = ctrlTab(document.body);
    expect(event.defaultPrevented).toBe(false);
  });
});
