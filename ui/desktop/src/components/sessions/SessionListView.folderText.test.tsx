/**
 * How a History row writes the folder a chat ran in: as the sidebar and the composer's folder
 * chip write it (utils/folderPath.ts).
 *
 * - The row names the folder (`~` for the home folder, `Crew` for the Crew task folder, else its
 *   own name), and its tooltip holds the whole location from `~` (`~/Desktop/BioRouter`), in the
 *   tooltip's own sans. It used to be the absolute path in mono, the one place the app wrote the
 *   same folder that way, beside a row that read the home folder as "someone".
 * - `~` only while the daemon is this machine's (`homeFolder`): with an external backend the
 *   folder may be on another machine, so the tooltip stays absolute.
 * - The filter matches the folder as stored and as the tooltip writes it.
 *
 * Tooltips are drawn inline here (their content beside their trigger), so what a hover would
 * show can be read without driving the pointer timers.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import SessionListView from './SessionListView';
import { clearSessionListCache } from '../../utils/sessionListCache';
import type { Session } from '../../api';

const mocks = vi.hoisted(() => ({ listSessions: vi.fn() }));

vi.mock('../../api', () => ({
  listSessions: mocks.listSessions,
  deleteSession: vi.fn(),
  exportSession: vi.fn(),
  importSession: vi.fn(),
  updateSessionName: vi.fn(),
  declassifySession: vi.fn(),
}));

vi.mock('../../toasts', () => ({ toastSuccess: vi.fn(), toastError: vi.fn() }));

vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-proof' }),
}));

vi.mock('../ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: ReactNode }) => <div>{children}</div>,
}));

vi.mock('../ui/Tooltip', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../ui/Tooltip')>();
  return {
    ...actual,
    Tooltip: ({ children }: { children: ReactNode }) => <>{children}</>,
    TooltipTrigger: ({ children }: { children: ReactNode }) => <>{children}</>,
    TooltipContent: ({ children }: { children: ReactNode }) => (
      <span data-tooltip-content="">{children}</span>
    ),
  };
});

const HOME = '/Users/someone';

const session = (id: string, name: string, workingDir: string): Session =>
  ({
    id,
    name,
    working_dir: workingDir,
    created_at: '2026-09-12T12:00:00Z',
    updated_at: '2026-09-12T12:00:00Z',
    extension_data: {},
    message_count: 2,
  }) as Session;

const SESSIONS = [
  session('s1', 'Under home', `${HOME}/Desktop/BioRouter`),
  session('s2', 'At home', HOME),
  session('s3', 'Elsewhere', '/Volumes/lab/data'),
  session('s4', 'Crew task', '/Users/someone/.local/share/biorouter/crew/tasks'),
  session('s5', 'Sibling', '/Users/someone2/work'),
];

function setAppConfig(config: Record<string, unknown>) {
  Object.defineProperty(window, 'appConfig', {
    configurable: true,
    value: { get: (key: string) => config[key] },
  });
}

let saved: typeof window.appConfig;

beforeEach(() => {
  vi.clearAllMocks();
  clearSessionListCache();
  saved = window.appConfig;
  setAppConfig({ BIOROUTER_HOME_DIR: HOME, BIOROUTER_LOCAL_BACKEND: true });
  mocks.listSessions.mockResolvedValue({ data: { sessions: SESSIONS } });
  (window as unknown as { electron: unknown }).electron = {
    platform: 'darwin',
    on: () => () => {},
  };
});

afterEach(() => {
  Object.defineProperty(window, 'appConfig', { configurable: true, value: saved });
});

async function renderHistory() {
  render(
    <MemoryRouter>
      <SessionListView onSelectSession={vi.fn()} />
    </MemoryRouter>
  );
  await screen.findByText('Under home');
}

/** A row's folder as the row names it, and as its tooltip writes it. */
function folderOf(name: string) {
  const row = screen
    .getByRole('button', { name: `Open chat ${name}` })
    .closest<HTMLElement>('[data-session-id]')!;
  const trigger = row.querySelector<HTMLElement>('[data-working-dir]')!;
  const tooltip = trigger.nextElementSibling as HTMLElement;
  expect(tooltip).toHaveAttribute('data-tooltip-content');
  return { trigger, tooltip };
}

/** Every element from `node` down, `node` included. */
const subtree = (node: Element) => [node, ...node.querySelectorAll('*')];

describe('a History row’s folder', () => {
  it('reads from `~` in sans in its tooltip, and names the folder as the sidebar does', async () => {
    await renderHistory();

    const under = folderOf('Under home');
    expect(under.trigger).toHaveTextContent(/^BioRouter$/);
    expect(under.trigger).toHaveAttribute('data-working-dir', `${HOME}/Desktop/BioRouter`);
    expect(under.tooltip).toHaveTextContent(/^~\/Desktop\/BioRouter$/);
    // The tooltip's own face: no mono anywhere in it (it was `font-mono`).
    for (const node of subtree(under.tooltip)) {
      expect(node.getAttribute('class') ?? '').not.toMatch(/font-mono/);
    }

    // The home folder is `~` on the row and in the tooltip (the row used to read "someone").
    const home = folderOf('At home');
    expect(home.trigger).toHaveTextContent(/^~$/);
    expect(home.tooltip).toHaveTextContent(/^~$/);

    // Outside home the location stays absolute, and a folder that only shares the home
    // folder's prefix is not under it.
    expect(folderOf('Elsewhere').tooltip).toHaveTextContent(/^\/Volumes\/lab\/data$/);
    expect(folderOf('Sibling').tooltip).toHaveTextContent(/^\/Users\/someone2\/work$/);

    // The Crew task folder is named "Crew", as the sidebar's folder groups name it.
    const crew = folderOf('Crew task');
    expect(crew.trigger).toHaveTextContent(/^Crew$/);
    expect(crew.tooltip).toHaveTextContent(/^~\/\.local\/share\/biorouter\/crew\/tasks$/);
  });

  it('stays absolute when the daemon is not this machine’s', async () => {
    setAppConfig({ BIOROUTER_HOME_DIR: HOME, BIOROUTER_LOCAL_BACKEND: false });
    await renderHistory();

    expect(folderOf('Under home').tooltip).toHaveTextContent(
      new RegExp(`^${HOME}/Desktop/BioRouter$`)
    );
    // With no home to write, the home folder is named by its own name.
    const home = folderOf('At home');
    expect(home.trigger).toHaveTextContent(/^someone$/);
    expect(home.tooltip).toHaveTextContent(new RegExp(`^${HOME}$`));
  });

  it('is found by the folder as its tooltip writes it, and as it is stored', async () => {
    await renderHistory();
    const filter = screen.getByRole('searchbox', { name: 'Search history' });
    const rowNames = () =>
      screen
        .getAllByRole('button', { name: /^Open chat / })
        .map((button) => button.getAttribute('aria-label')?.replace('Open chat ', ''));

    fireEvent.change(filter, { target: { value: '~/desktop' } });
    await waitFor(() => expect(rowNames()).toEqual(['Under home']));

    fireEvent.change(filter, { target: { value: '/users/someone/desktop' } });
    await waitFor(() => expect(rowNames()).toEqual(['Under home']));
  });
});
