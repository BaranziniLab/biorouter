import { render, screen } from '@testing-library/react';
import { useState } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SessionSummary } from '../../api';
import { ChatProvider } from '../../contexts/ChatContext';
import type { ChatType } from '../../types/chat';
import { SidebarProvider } from '../ui/sidebar';
import { SIDEBAR_CHAT_VIEW_STORAGE_KEY } from './sidebarChatView';

/**
 * The composer footer's folder chip names a folder the way the sidebar's folder groups name it
 * (P3), and both write `~` only where `~` is true: while the daemon is this machine's
 * (`homeFolder()`, utils/folderPath.ts). The helpers' own cases are folderPath.test.ts's; this
 * file renders the two REAL surfaces under one `window.appConfig`, so each takes its home folder
 * from wherever it really takes it. Handing both helpers the same literal home cannot see a
 * surface that reads the home folder some other way, which is how the sidebar once wrote `~` for
 * an external backend while the chip, its card, History and the terminal dock wrote `wgu`.
 */

const mocks = vi.hoisted(() => ({
  listSessions: vi.fn(),
  listSidebarSessions: vi.fn(),
}));

vi.mock('../../api', () => ({
  listSessions: mocks.listSessions,
  listSidebarSessions: mocks.listSidebarSessions,
  // The folder chip's card reads these only when it opens, which nothing here does.
  updateWorkingDir: vi.fn(async () => ({ data: {} })),
  listFolderAccess: vi.fn(async () => ({ data: [] })),
  revokeFolderAccess: vi.fn(async () => ({ data: { revoked: true } })),
}));

vi.mock('../../hooks/chatStreamStore', () => ({
  useRunningChats: () => [],
}));

vi.mock('./SidebarUpdateButton', () => ({
  default: () => null,
}));

import AppSidebar from './AppSidebar';
import { DirSwitcher } from '../bottom_menu/DirSwitcher';

const HOME = '/Users/wgu';
/** One chat per folder; every basename differs, so no label takes its parent's name. */
const FOLDERS = [
  HOME,
  `${HOME}/Projects/kdps`,
  `${HOME}/Library/Application Support/Biorouter/crew/tasks`,
  '/Volumes/Lab/data',
];

const SESSIONS: SessionSummary[] = FOLDERS.map((folder, index) => ({
  id: `chat-${index}`,
  name: `Chat ${index}`,
  created_at: '2026-07-14T12:00:00.000Z',
  updated_at: `2026-07-15T12:0${index}:00.000Z`,
  working_dir: folder,
  message_count: 2,
  user_set_name: true,
}));

function setConfig(config: Record<string, unknown> | undefined) {
  Object.assign(window, {
    appConfig: config === undefined ? undefined : { get: (key: string) => config[key] },
  });
}

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
  // Radix's popover measures its anchor.
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
  );
});

afterAll(() => {
  vi.unstubAllGlobals();
});

beforeEach(() => {
  window.localStorage.clear();
  // Group by Folder: a header per folder, its label the sidebar's name for it.
  window.localStorage.setItem(
    SIDEBAR_CHAT_VIEW_STORAGE_KEY,
    JSON.stringify({ v: 1, groupBy: 'folder', sortBy: 'activity' })
  );
  mocks.listSessions.mockResolvedValue({ data: { sessions: SESSIONS } });
  mocks.listSidebarSessions.mockResolvedValue({
    data: { sessions: SESSIONS, has_more: false, next_cursor: null },
  });
});

afterEach(() => {
  setConfig(undefined);
  window.localStorage.clear();
});

function Sidebar() {
  const [chat, setChat] = useState<ChatType>({
    sessionId: 'chat-0',
    name: 'Chat 0',
    messages: [],
    workflow: null,
  });
  return (
    <ChatProvider chat={chat} setChat={setChat}>
      <SidebarProvider>
        <AppSidebar currentPath="/" onSelectSession={vi.fn()} />
      </SidebarProvider>
    </ChatProvider>
  );
}

/** Each folder's group label, as the real sidebar draws it. */
async function sidebarLabels(): Promise<Map<string, string>> {
  const view = render(
    <MemoryRouter>
      <Sidebar />
    </MemoryRouter>
  );
  const labels = new Map<string, string>();
  for (const folder of FOLDERS) {
    const group = await screen.findByTestId(`sidebar-folder-folder:${folder}`);
    labels.set(folder, group.querySelector('.br-nav-row-label')?.textContent ?? '');
  }
  view.unmount();
  return labels;
}

/** The folder's name on the real composer chip. */
function chipName(folder: string): string | null | undefined {
  const view = render(<DirSwitcher sessionId="chat-0" workingDir={folder} locked />);
  const name = view.container.querySelector('bdi')?.textContent;
  view.unmount();
  return name;
}

describe('the folder chip and the sidebar name a folder the same way, from their own sources', () => {
  it.each([
    [
      'the desktop’s own daemon',
      { BIOROUTER_HOME_DIR: HOME, BIOROUTER_LOCAL_BACKEND: true },
      ['~', 'kdps', 'Crew', 'data'],
    ],
    // The desktop pointed at an external backend (`just debug-ui`): the daemon may run elsewhere,
    // where `~` names another home, so both read the home folder by its own name.
    [
      'an external backend',
      { BIOROUTER_HOME_DIR: HOME, BIOROUTER_LOCAL_BACKEND: false },
      ['wgu', 'kdps', 'Crew', 'data'],
    ],
    ['no config', undefined, ['wgu', 'kdps', 'Crew', 'data']],
  ] as const)('with %s', async (_case, config, expected) => {
    setConfig(config);
    const labels = await sidebarLabels();
    FOLDERS.forEach((folder, index) => {
      expect(labels.get(folder), `the sidebar’s ${folder}`).toBe(expected[index]);
      expect(chipName(folder), `the chip’s ${folder}`).toBe(labels.get(folder));
    });
  });
});
