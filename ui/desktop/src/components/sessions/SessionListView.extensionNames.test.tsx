/**
 * Vision round 2, V3-R2-6 (principle 10: names, not ids).
 *
 * A History row's extension cell lists the chat's extensions. It read the config KEY and title-cased
 * it, so the row said "Computercontroller, Cdwagent" for what the catalogue, the composer and the
 * Extensions page call "Biorouter Copilot" and "CDWAgent". The row now asks the one resolver,
 * `extensionDisplayName`, and hands it the session's config ENTRY, so an installed extension
 * (`type: 'stdio'`) is never renamed as a bundled capability that happens to share its key.
 */
import { render, screen } from '@testing-library/react';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
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

const session = (extensions: unknown[]) =>
  ({
    id: 'session-1',
    name: 'Cohort analysis',
    working_dir: '/Users/someone/Desktop',
    created_at: '2026-09-05T17:49:17Z',
    updated_at: '2026-09-05T17:49:17Z',
    message_count: 2,
    extension_data: { 'enabled_extensions.v0': { extensions } },
  }) as unknown as Session;

beforeEach(() => {
  vi.clearAllMocks();
  clearSessionListCache();
  (window as unknown as { electron: unknown }).electron = {
    platform: 'darwin',
    on: () => () => {},
  };
});

/** The text of the extensions stat cell, which is where a row names its extensions. */
async function extensionsCell(extensions: unknown[]): Promise<string> {
  mocks.listSessions.mockResolvedValue({ data: { sessions: [session(extensions)] } });
  render(
    <MemoryRouter>
      <SessionListView onSelectSession={vi.fn()} />
    </MemoryRouter>
  );
  await screen.findByText('Cohort analysis');
  const cell = [...document.querySelectorAll('.br-history-stat')]
    .map((el) => el.textContent ?? '')
    .find((text) => text.startsWith('Extensions:'));
  expect(cell, 'the row has no extensions cell').toBeDefined();
  return cell!;
}

describe('a History row names its extensions as the catalogue does (V3-R2-6)', () => {
  it('reads a bundled key and an installed one by their catalogue names, not title-cased keys', async () => {
    const cell = await extensionsCell([
      { type: 'builtin', name: 'computercontroller' },
      { type: 'stdio', name: 'cdwagent' },
    ]);
    expect(cell).toMatch(/Biorouter Copilot, CDWAgent/);
    expect(cell).not.toMatch(/Computercontroller|Cdwagent/);
  });

  it('keeps the names an entry without a type already had', async () => {
    const cell = await extensionsCell([{ name: 'developer' }, { name: 'memory' }]);
    expect(cell).toMatch(/Developer, Memory/);
  });

  it('never renames an installed extension as the bundled capability that shares its key', async () => {
    // A stdio entry keyed `computercontroller` is somebody's own server, not the bundled Biorouter
    // Copilot, so it is not called that. This is why the row passes the entry and not its name.
    const cell = await extensionsCell([{ type: 'stdio', name: 'computercontroller' }]);
    expect(cell).not.toMatch(/Biorouter Copilot/);
    expect(cell).toMatch(/Computercontroller/);
  });
});
