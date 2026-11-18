import { act, cleanup, fireEvent, render, within } from '@testing-library/react';
import { StrictMode, type ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  ChatGroupsProvider,
  resetLiveChatTabsForTests,
  useChatGroups,
} from '../../contexts/ChatGroupsContext';
import { TerminalDockProvider, useTerminalDock } from '../../contexts/TerminalDockContext';
import { clearSessionListCache } from '../../utils/sessionListCache';
import { resetSystemMonoFontForTest } from '../../utils/systemMonoFont';
import {
  resetCloseTerminalPaneRegistry,
  resetNewTerminalPaneRegistry,
} from '../../utils/terminalFocus';
import { SidebarProvider } from '../ui/sidebar';
import ChatGroupsShell from './ChatGroupsShell';
import { getWindowId, saveChatGroups } from './chatGroupsStorage';
import type { ChatGroupsState, GroupLayout } from './chatGroupsTypes';
import { leafGroupIds } from './chatGroupsTypes';

/**
 * The real shell, reducer/providers, compact action, tab strip and terminal dock own these
 * lifetimes. The approved BaseChat VIEW boundary renders its actual title seam; it neither owns
 * nor stands in for a terminal. Existing BaseChat suites avoid its unrelated jsdom integrations.
 * Its composer mirrors ChatInput's mount focus so a remount cannot conceal focus stealing.
 * The remaining doubles are xterm and the Electron/HTTP backend. A real BaseChat + PTY check remains
 * necessary alongside these renderer lifecycle regressions.
 */
vi.mock('../BaseChat', async () => {
  const React = await import('react');
  const { isTerminalFocused } = await import('../../utils/terminalFocus');
  return {
    default: function BaseChatView({
      renderSessionTitle,
      autoFocusComposer = true,
    }: {
      renderSessionTitle?: () => ReactNode;
      autoFocusComposer?: boolean;
    }) {
      const textAreaRef = React.useRef<HTMLTextAreaElement | null>(null);
      const autoFocusAtMountRef = React.useRef(autoFocusComposer);
      React.useEffect(() => {
        if (autoFocusAtMountRef.current && textAreaRef.current && !isTerminalFocused()) {
          textAreaRef.current.focus();
        }
      }, []);
      return (
        <div className="flex min-h-0 flex-1 flex-col">
          {renderSessionTitle?.()}
          <textarea ref={textAreaRef} className="br-composer-input" aria-label="Test composer" />
        </div>
      );
    },
  };
});

interface TerminalData {
  sessionId: string;
  data: string;
}

interface TerminalExit {
  sessionId: string;
  exitCode: number;
}

interface TerminalDouble {
  node: HTMLDivElement;
  host: HTMLElement | null;
  options: Record<string, unknown>;
  dispose: ReturnType<typeof vi.fn>;
  write: ReturnType<typeof vi.fn>;
}

const boundary = vi.hoisted(() => ({
  terminals: [] as TerminalDouble[],
  data: new Set<(event: TerminalData) => void>(),
  exits: new Set<(event: TerminalExit) => void>(),
  create: vi.fn(),
  dispose: vi.fn(),
  write: vi.fn(),
}));

vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    node = document.createElement('div');
    host: HTMLElement | null = null;
    modes = { bracketedPasteMode: false };
    dispose = vi.fn();
    focus = vi.fn();
    loadAddon = vi.fn();
    onData = vi.fn(() => ({ dispose: vi.fn() }));
    open = vi.fn((host: HTMLElement) => {
      this.host = host;
      this.node.tabIndex = 0;
      this.node.setAttribute('data-testid', 'xterm-boundary');
      host.append(this.node);
    });
    write = vi.fn((text: string, parsed?: () => void) => {
      this.node.textContent += text;
      parsed?.();
    });
    writeln = vi.fn((text: string) => {
      this.node.textContent += `${text}\n`;
    });
    constructor(public options: Record<string, unknown>) {
      boundary.terminals.push(this);
    }
  },
}));

vi.mock('@xterm/addon-fit', () => ({
  FitAddon: class {
    fit = vi.fn();
    proposeDimensions = vi.fn(() => ({ cols: 80, rows: 18 }));
  },
}));

type Groups = NonNullable<ReturnType<typeof useChatGroups>>;
type Docks = NonNullable<ReturnType<typeof useTerminalDock>>;
let controls: { groups: Groups; docks: Docks };

function Controls() {
  const groups = useChatGroups();
  const docks = useTerminalDock();
  if (!groups || !docks) throw new Error('Missing real ownership providers.');
  controls = { groups, docks };
  return null;
}

type Box = { width: number; height: number };
let tree: Box;
let paneBoxes: Record<string, Box>;
let sessions: Array<Record<string, unknown>>;
let nextPty: number;
const originalElectron = Object.getOwnPropertyDescriptor(window, 'electron');
const originalAppConfig = Object.getOwnPropertyDescriptor(window, 'appConfig');
const originalClientWidth = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientWidth');
const nativeRect = HTMLElement.prototype.getBoundingClientRect;

class LayoutObserver {
  static live = new Set<LayoutObserver>();
  targets = new Set<Element>();
  constructor(private callback: ConstructorParameters<typeof ResizeObserver>[0]) {
    LayoutObserver.live.add(this);
  }
  observe(target: Element) {
    this.targets.add(target);
  }
  unobserve(target: Element) {
    this.targets.delete(target);
  }
  disconnect() {
    LayoutObserver.live.delete(this);
    this.targets.clear();
  }
  sample() {
    this.callback([], this as unknown as ResizeObserver);
  }
}

function rect(box: Box): DOMRect {
  return {
    x: 0,
    y: 0,
    left: 0,
    top: 0,
    right: box.width,
    bottom: box.height,
    ...box,
    toJSON: () => ({}),
  };
}

beforeEach(() => {
  localStorage.clear();
  sessionStorage.clear();
  resetLiveChatTabsForTests();
  clearSessionListCache();
  resetSystemMonoFontForTest();
  resetCloseTerminalPaneRegistry();
  resetNewTerminalPaneRegistry();
  LayoutObserver.live.clear();
  boundary.terminals.length = 0;
  boundary.data.clear();
  boundary.exits.clear();
  boundary.create.mockReset();
  boundary.dispose.mockReset();
  boundary.write.mockReset();
  nextPty = 0;
  tree = { width: 1000, height: 600 };
  paneBoxes = {};
  sessions = [];
  boundary.create.mockImplementation(async ({ workingDir }: { workingDir?: string }) => ({
    success: true,
    backend: 'pty',
    sessionId: `pty-${++nextPty}`,
    cwd: workingDir,
  }));
  boundary.dispose.mockResolvedValue({ success: true });
  boundary.write.mockResolvedValue({ success: true });
  Object.defineProperty(window, 'electron', {
    configurable: true,
    value: {
      platform: 'darwin',
      createTerminalSession: boundary.create,
      disposeTerminalSession: boundary.dispose,
      writeTerminalSession: boundary.write,
      resizeTerminalSession: vi.fn(async () => ({ success: true })),
      onTerminalData: (handler: (event: TerminalData) => void) => {
        boundary.data.add(handler);
        return () => boundary.data.delete(handler);
      },
      onTerminalExit: (handler: (event: TerminalExit) => void) => {
        boundary.exits.add(handler);
        return () => boundary.exits.delete(handler);
      },
    },
  });
  Object.defineProperty(window, 'appConfig', {
    configurable: true,
    value: {
      get: (key: string) =>
        ({
          BIOROUTER_API_HOST: 'http://terminal-lifetime.invalid',
          BIOROUTER_HOME_DIR: '/synthetic-home',
          BIOROUTER_LOCAL_BACKEND: true,
        })[key as 'BIOROUTER_API_HOST' | 'BIOROUTER_HOME_DIR' | 'BIOROUTER_LOCAL_BACKEND'],
    },
  });
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: Parameters<typeof fetch>[0]) => {
      const url = new URL(
        typeof input === 'string' ? input : input instanceof URL ? input.href : input.url
      );
      if (url.pathname === '/sessions') return Response.json({ sessions });
      const id = /^\/sessions\/([^/]+)$/.exec(url.pathname)?.[1];
      const session = sessions.find((row) => row.id === id);
      if (session) return Response.json({ session });
      throw new Error(`Unexpected terminal-lifetime backend request: ${url.pathname}`);
    })
  );
  vi.stubGlobal('ResizeObserver', LayoutObserver);
  Object.defineProperty(HTMLElement.prototype, 'clientWidth', {
    configurable: true,
    get() {
      return this.hasAttribute('data-chat-groups-tree') ? tree.width : 0;
    },
  });
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (
    this: HTMLElement
  ) {
    if (this.hasAttribute('data-chat-groups-tree')) return rect(tree);
    const groupId =
      this.getAttribute('data-chat-group-id') ??
      (this.classList.contains('br-compact-pane')
        ? this.closest('[data-chat-group-id]')?.getAttribute('data-chat-group-id')
        : null);
    if (groupId) return rect(paneBoxes[groupId] ?? { width: 600, height: 600 });
    return nativeRect.call(this);
  });
});

afterEach(() => {
  cleanup();
  clearSessionListCache();
  resetLiveChatTabsForTests();
  resetCloseTerminalPaneRegistry();
  resetNewTerminalPaneRegistry();
  vi.unstubAllGlobals();
  if (originalClientWidth)
    Object.defineProperty(HTMLElement.prototype, 'clientWidth', originalClientWidth);
  else Reflect.deleteProperty(HTMLElement.prototype, 'clientWidth');
  if (originalElectron) Object.defineProperty(window, 'electron', originalElectron);
  else Reflect.deleteProperty(window, 'electron');
  if (originalAppConfig) Object.defineProperty(window, 'appConfig', originalAppConfig);
  else Reflect.deleteProperty(window, 'appConfig');
});

function initialState(layout: GroupLayout, tabsPerGroup = 1): ChatGroupsState {
  const groups: ChatGroupsState['groups'] = {};
  let tabIndex = 0;
  for (const groupId of leafGroupIds(layout)) {
    const tabs = Array.from({ length: tabsPerGroup }, () => {
      const tabId = `tab-${++tabIndex}`;
      const sessionId = `session-${tabIndex}`;
      sessions.push({
        id: sessionId,
        name: `Chat ${tabIndex}`,
        user_set_name: true,
        privacy_tier: 'public',
        session_type: 'user',
        working_dir: `/work/${tabId}`,
        updated_at: '2026-11-18T00:00:00Z',
      });
      return { tabId, sessionId, title: `Chat ${tabIndex}`, userSetName: true };
    });
    groups[groupId] = { groupId, tabs, activeTabId: tabs[0].tabId };
  }
  return { version: 1, seq: 20, activeGroupId: leafGroupIds(layout)[0], layout, groups };
}

async function mount(state: ChatGroupsState, { strict = false }: { strict?: boolean } = {}) {
  saveChatGroups(state, getWindowId());
  const app = (
    <MemoryRouter initialEntries={['/pair']}>
      <SidebarProvider>
        <ChatGroupsProvider>
          <TerminalDockProvider>
            <Controls />
            <ChatGroupsShell onChatChange={() => {}} />
          </TerminalDockProvider>
        </ChatGroupsProvider>
      </SidebarProvider>
    </MemoryRouter>
  );
  const view = render(strict ? <StrictMode>{app}</StrictMode> : app);
  await act(async () => {});
  return view;
}

function resizeTree(next: Box) {
  tree = next;
  act(() => {
    for (const observer of [...LayoutObserver.live]) {
      if (!LayoutObserver.live.has(observer)) continue;
      if (
        [...observer.targets].some(
          (target) =>
            target.hasAttribute('data-chat-groups-tree') ||
            target.hasAttribute('data-chat-group-id') ||
            target.classList.contains('br-compact-pane')
        )
      ) {
        observer.sample();
      }
    }
  });
}

async function openTerminal(tabId: string, workingDir = `/work/${tabId}`) {
  act(() => controls.groups.dispatch({ type: 'activateTab', tabId }));
  await act(async () => controls.docks.setOpen(tabId, true, workingDir));
}

function dispatch(action: Parameters<Groups['dispatch']>[0]) {
  act(() => controls.groups.dispatch(action));
}

function runtime(index: number, sessionId = `pty-${index + 1}`) {
  const terminal = boundary.terminals[index];
  if (!terminal) throw new Error(`Missing xterm ${index}.`);
  act(() => {
    for (const handler of boundary.data) handler({ sessionId, data: `history-${sessionId}\n$ ` });
  });
  return { terminal, node: terminal.node, host: terminal.host, sessionId };
}

function expectKept(saved: ReturnType<typeof runtime>, creations: number) {
  expect(boundary.create).toHaveBeenCalledTimes(creations);
  expect(boundary.dispose).not.toHaveBeenCalled();
  expect(saved.terminal.dispose).not.toHaveBeenCalled();
  expect(saved.node.isConnected).toBe(true);
  expect(saved.terminal.node).toBe(saved.node);
  expect(saved.terminal.host).toBe(saved.host);
  expect(saved.node).toHaveTextContent(`history-${saved.sessionId}`);
}

function compactMerge(groupId: string) {
  const pane = document.querySelector<HTMLElement>(`[data-chat-group-id="${groupId}"]`);
  if (!pane) throw new Error(`Missing ${groupId}.`);
  const button = within(pane).getByRole('button', { name: /^Merge panes,/ });
  fireEvent.click(button);
}

const ONE: GroupLayout = { kind: 'leaf', groupId: 'g1' };
const TWO: GroupLayout = {
  kind: 'branch',
  dir: 'row',
  sizes: [0.5, 0.5],
  children: [
    { kind: 'leaf', groupId: 'g1' },
    { kind: 'leaf', groupId: 'g2' },
  ],
};
const FIVE: GroupLayout = {
  kind: 'branch',
  dir: 'col',
  sizes: Array(5).fill(0.2),
  children: Array.from({ length: 5 }, (_, index) => ({ kind: 'leaf', groupId: `g${index + 1}` })),
};

describe('terminal runtime ownership through real shell layout transitions', () => {
  it('keeps selected and hidden per-tab shells through the actual compact Merge action', async () => {
    for (const groupId of leafGroupIds(FIVE)) paneBoxes[groupId] = { width: 1000, height: 119.2 };
    await mount(initialState(FIVE));
    await openTerminal('tab-1');
    const hidden = runtime(0);
    act(() => controls.docks.setOpen('tab-1', false));
    await openTerminal('tab-3');
    const selected = runtime(1);
    compactMerge('g3');
    expect(controls.groups.state.layout).toEqual({ kind: 'leaf', groupId: 'g3' });
    expect(controls.groups.activeTab?.tabId).toBe('tab-3');
    expect(controls.groups.activeGroup?.tabs).toHaveLength(5);
    expectKept(selected, 2);
    expectKept(hidden, 2);
  });

  it('keeps the same shells through automatic width merge and restoration', async () => {
    await mount(initialState(TWO));
    await openTerminal('tab-1');
    const first = runtime(0);
    await openTerminal('tab-2');
    const second = runtime(1);
    act(() => second.node.focus());
    expect(document.activeElement).toBe(second.node);
    resizeTree({ width: 600, height: 600 });
    expect(controls.groups.state.layout.kind).toBe('leaf');
    expectKept(first, 2);
    expectKept(second, 2);
    expect(document.activeElement).toBe(second.node);
    resizeTree({ width: 1000, height: 600 });
    expect(controls.groups.state.layout.kind).toBe('branch');
    expectKept(first, 2);
    expectKept(second, 2);
    expect(document.activeElement).toBe(second.node);
  });

  it('keeps terminal focus through automatic merge and restore under the renderer root StrictMode', async () => {
    await mount(initialState(TWO), { strict: true });
    await openTerminal('tab-2');
    const liveIndex = boundary.terminals.findIndex(
      (terminal) =>
        terminal.node.isConnected &&
        terminal.dispose.mock.calls.length === 0 &&
        !terminal.node.closest('[data-terminal-pane]')?.classList.contains('invisible')
    );
    if (liveIndex === -1) throw new Error('Missing settled live StrictMode terminal.');
    const created = await Promise.all(boundary.create.mock.results.map((result) => result.value));
    act(() => {
      for (const { sessionId } of created) {
        for (const handler of boundary.data) {
          handler({ sessionId, data: `strict-session:${sessionId};\n` });
        }
      }
    });
    const connected = boundary.terminals[liveIndex];
    const associated = created.filter(({ sessionId }) =>
      connected.node.textContent?.includes(`strict-session:${sessionId};`)
    );
    expect(associated).toHaveLength(1);
    const saved = runtime(liveIndex, associated[0].sessionId);
    const creations = boundary.create.mock.calls.length;
    const disposals = boundary.dispose.mock.calls.length;
    expect(creations).toBeGreaterThan(0);
    expect(saved.terminal.dispose).not.toHaveBeenCalled();
    expect(saved.node).toHaveTextContent(`history-${saved.sessionId}`);
    act(() => saved.node.focus());
    expect(document.activeElement).toBe(saved.node);

    const expectRuntimeKept = () => {
      expect(boundary.create).toHaveBeenCalledTimes(creations);
      expect(boundary.dispose).toHaveBeenCalledTimes(disposals);
      expect(saved.terminal.dispose).not.toHaveBeenCalled();
      expect(saved.node.isConnected).toBe(true);
      expect(saved.terminal.host).toBe(saved.host);
      expect(saved.node).toHaveTextContent(`history-${saved.sessionId}`);
    };

    resizeTree({ width: 600, height: 600 });
    expect(controls.groups.state.layout.kind).toBe('leaf');
    expectRuntimeKept();
    expect(document.activeElement).toBe(saved.node);
    resizeTree({ width: 1000, height: 600 });
    expect(controls.groups.state.layout.kind).toBe('branch');
    expectRuntimeKept();
    expect(document.activeElement).toBe(saved.node);
  });

  it('keeps shells through split and tab moves, and routes moved dock input to its physical group', async () => {
    await mount(initialState(ONE, 3));
    await openTerminal('tab-1');
    const first = runtime(0);
    await openTerminal('tab-2');
    const moved = runtime(1);
    dispatch({ type: 'moveTabToGroup', tabId: 'tab-2', targetGroupId: 'g1', zone: 'right' });
    const owner = Object.values(controls.groups.state.groups).find((group) =>
      group.tabs.some((tab) => tab.tabId === 'tab-2')
    );
    if (!owner || owner.groupId === 'g1') throw new Error('The real reducer did not split.');
    expectKept(first, 2);
    expectKept(moved, 2);
    dispatch({ type: 'setActiveGroup', groupId: 'g1' });
    fireEvent.pointerDown(moved.node, { button: 0 });
    expect(controls.groups.state.activeGroupId).toBe(owner.groupId);
    dispatch({ type: 'setActiveGroup', groupId: 'g1' });
    act(() => moved.node.focus());
    expect(controls.groups.state.activeGroupId).toBe(owner.groupId);
    dispatch({ type: 'moveTabToGroup', tabId: 'tab-2', targetGroupId: 'g1', zone: 'center' });
    expect(controls.groups.state.layout.kind).toBe('leaf');
    expectKept(first, 2);
    expectKept(moved, 2);
  });

  it('keeps an inactive hidden dock and its frozen working folder through tab changes', async () => {
    await mount(initialState(ONE, 2));
    await openTerminal('tab-1', '/work/original');
    const first = runtime(0);
    act(() => controls.docks.setOpen('tab-1', false));
    await openTerminal('tab-2', '/work/other');
    const other = runtime(1);
    expect(first.node.closest('[data-testid="in-app-terminal-dock"]')).toHaveClass('hidden');
    await openTerminal('tab-1', '/work/changed');
    expect(controls.docks.terminals.find((dock) => dock.key === 'tab-1')?.workingDir).toBe(
      '/work/original'
    );
    expect(boundary.create.mock.calls[0][0]).toEqual(
      expect.objectContaining({ workingDir: '/work/original' })
    );
    expectKept(first, 2);
    expectKept(other, 2);
  });

  it('disposes once on intentional terminal close and once when a chat close retains the other dock', async () => {
    const view = await mount(initialState(ONE, 2));
    await openTerminal('tab-1');
    const first = runtime(0);
    const dock = first.node.closest<HTMLElement>('[data-testid="in-app-terminal-dock"]');
    if (!dock) throw new Error('Missing real terminal dock.');
    fireEvent.click(within(dock).getByRole('button', { name: /^Close terminal tab / }));
    expect(boundary.dispose.mock.calls).toEqual([['pty-1']]);
    expect(first.terminal.dispose).toHaveBeenCalledTimes(1);
    expect(controls.docks.terminals).toHaveLength(0);
    dispatch({ type: 'closeTab', tabId: 'tab-1' });
    expect(boundary.dispose).toHaveBeenCalledTimes(1);
    await openTerminal('tab-2');
    const second = runtime(1);
    dispatch({ type: 'closeTab', tabId: 'tab-2' });
    dispatch({ type: 'closeTab', tabId: 'tab-2' });
    expect(boundary.dispose.mock.calls).toEqual([['pty-1'], ['pty-2']]);
    expect(second.terminal.dispose).toHaveBeenCalledTimes(1);
    view.unmount();
    expect(boundary.dispose).toHaveBeenCalledTimes(2);
    expect(boundary.data.size).toBe(0);
    expect(boundary.exits.size).toBe(0);
  });

  it('keeps a pending create through Merge and disposes its eventual PTY only after the real chat closes', async () => {
    let finish!: (answer: { success: true; backend: string; sessionId: string }) => void;
    boundary.create.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        })
    );
    for (const groupId of leafGroupIds(FIVE)) paneBoxes[groupId] = { width: 1000, height: 119.2 };
    await mount(initialState(FIVE));
    await openTerminal('tab-3');
    const pending = boundary.terminals[0];
    expect(boundary.create).toHaveBeenCalledTimes(1);
    compactMerge('g3');
    expect(boundary.create).toHaveBeenCalledTimes(1);
    expect(pending.dispose).not.toHaveBeenCalled();
    dispatch({ type: 'closeTab', tabId: 'tab-3' });
    expect(pending.dispose).toHaveBeenCalledTimes(1);
    expect(boundary.dispose).not.toHaveBeenCalled();
    await act(async () => finish({ success: true, backend: 'pty', sessionId: 'pending-pty' }));
    expect(boundary.dispose.mock.calls).toEqual([['pending-pty']]);
    expect(pending.dispose).toHaveBeenCalledTimes(1);
    expect(boundary.create).toHaveBeenCalledTimes(1);
  });
});
