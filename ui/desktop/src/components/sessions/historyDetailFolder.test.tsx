/**
 * "This chat ran in …", the InfoTip on a transcript's title (`ranIn`, sessions/copy.ts).
 *
 * - History's own transcript (`SessionHistoryView`) is the person's chat, on this machine, so
 *   its folder is written as everywhere else in the app: `~` for the home folder, absolute
 *   outside it, and absolute whenever the daemon is not this machine's (`homeFolder`).
 * - A shared chat (`SharedSessionView`) ran on the SHARER's machine. Its folder stays absolute
 *   even when it happens to start with the reader's home folder: `~` there would name the
 *   reader's home, a folder the chat never saw.
 *
 * The page header is drawn as its props here, so the help text is read as the InfoTip would
 * show it, in its own face (a plain sentence, no markup and no mono).
 */
import { render, screen } from '@testing-library/react';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import SessionHistoryView from './SessionHistoryView';
import SharedSessionView from './SharedSessionView';
import type { Session } from '../../api';
import type { SharedSessionDetails } from '../../sharedSessions';
import { SHARED_CHAT_INFO } from './copy';

vi.mock('../../api', () => ({ declassifySession: vi.fn(), getSession: vi.fn() }));

vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-key' }),
}));

vi.mock('../ProgressiveMessageList', () => ({ default: () => <div data-testid="messages" /> }));

vi.mock('./SessionViewComponents', () => ({ SessionMessages: () => <div /> }));

vi.mock('../artifacts/ArtifactViewer', () => ({ default: () => <div /> }));

vi.mock('../conversation/SearchView', () => ({
  SearchView: ({ children }: { children: ReactNode }) => <>{children}</>,
}));

vi.mock('../ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: ReactNode }) => <div>{children}</div>,
}));

vi.mock('../Layout/PageHeader', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../Layout/PageHeader')>();
  return {
    ...actual,
    PageHeader: ({ title, info }: { title: string; info?: ReactNode }) => (
      <div data-band="">
        <h1>{title}</h1>
        <div data-testid="page-info">{info}</div>
      </div>
    ),
  };
});

const HOME = '/Users/someone';
const FOLDER = `${HOME}/Desktop/BioRouter`;

function setAppConfig(config: Record<string, unknown>) {
  Object.defineProperty(window, 'appConfig', {
    configurable: true,
    value: { get: (key: string) => config[key] },
  });
}

let saved: typeof window.appConfig;

beforeEach(() => {
  saved = window.appConfig;
  setAppConfig({ BIOROUTER_HOME_DIR: HOME, BIOROUTER_LOCAL_BACKEND: true });
});

afterEach(() => {
  Object.defineProperty(window, 'appConfig', { configurable: true, value: saved });
});

function renderHistoryDetail(workingDir: string) {
  return render(
    <MemoryRouter>
      <SessionHistoryView
        session={
          {
            id: 'session-1',
            name: 'Cohort query',
            created_at: '2026-07-14T12:00:00Z',
            updated_at: '2026-07-14T12:00:00Z',
            working_dir: workingDir,
            message_count: 0,
            extension_data: {},
            conversation: [],
          } as Session
        }
        isLoading={false}
        error={null}
        onBack={vi.fn()}
        onRetry={vi.fn()}
      />
    </MemoryRouter>
  );
}

function renderShared(workingDir: string) {
  const session: SharedSessionDetails = {
    share_token: 'token-1',
    created_at: 1757200000,
    base_url: 'https://example.invalid',
    description: 'Shared cohort walkthrough',
    working_dir: workingDir,
    message_count: 2,
    total_tokens: 4096,
    messages: [],
  };
  return render(
    <SharedSessionView session={session} isLoading={false} error={null} onRetry={vi.fn()} />
  );
}

const info = () => screen.getByTestId('page-info');

describe('where a transcript ran', () => {
  it('writes a History chat’s folder from `~`, as a plain sentence', () => {
    renderHistoryDetail(FOLDER);
    expect(info().textContent).toBe('This chat ran in ~/Desktop/BioRouter.');
    // A sentence in the InfoTip's own face: no element, so no mono, inside it.
    expect(info().children).toHaveLength(0);
  });

  it('writes the home folder itself as `~`, and a folder outside home in full', () => {
    const view = renderHistoryDetail(HOME);
    expect(info().textContent).toBe('This chat ran in ~.');
    view.unmount();

    renderHistoryDetail('/Volumes/lab/data');
    expect(info().textContent).toBe('This chat ran in /Volumes/lab/data.');
  });

  it('keeps a History chat’s folder absolute when the daemon is not this machine’s', () => {
    setAppConfig({ BIOROUTER_HOME_DIR: HOME, BIOROUTER_LOCAL_BACKEND: false });
    renderHistoryDetail(FOLDER);
    expect(info().textContent).toBe(`This chat ran in ${FOLDER}.`);
  });

  it('keeps a shared chat’s folder absolute, even one under the reader’s home folder', () => {
    renderShared(FOLDER);
    expect(info().textContent).toBe(`${SHARED_CHAT_INFO} This chat ran in ${FOLDER}.`);
    expect(info().textContent).not.toMatch(/~/);
  });
});
