import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  ExtensionsPanel,
  useExtensionPicker,
} from '../../bottom_menu/BottomMenuExtensionSelection';
import { useChatCrewAccess } from './chatCrewAccess';
import { connection, grantRow } from './testing';

/**
 * Crew ships with Biorouter, so it is a capability: it is switched on or off in Settings → Chat →
 * Capabilities and is not one of the chat's extensions. The Tools popover used to list it, and then
 * had to lock its row while a grant was active (NOVICE-BASELINE F3: "crew · Extension removed" left
 * the grant standing). The row is gone, so neither the row nor Disable all can touch a granted
 * chat's Crew tools, and only Revoke ends that access.
 */

const mocks = vi.hoisted(() => ({
  crewHttp: vi.fn(),
  getSessionExtensions: vi.fn(),
  addToAgent: vi.fn(async (): Promise<void> => undefined),
  removeFromAgent: vi.fn(async (): Promise<void> => undefined),
}));

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: mocks.crewHttp };
});
vi.mock('../../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({ currentProvider: 'versa_azure' }),
}));
vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({
    getProviders: async () => [
      { name: 'versa_azure', is_configured: true, resolved_tier: 'private' },
    ],
    extensionsList: [
      { type: 'platform', name: 'crew', description: 'Crew', bundled: true, enabled: true },
      { type: 'stdio', name: 'spoke', description: 'Spoke', cmd: 'spoke', args: [], enabled: true },
    ],
  }),
}));
vi.mock('../../settings/extensions/subcomponents/ExtensionList', () => ({
  isBuiltInExtension: () => false,
}));
vi.mock('../../../api', () => ({ getSessionExtensions: mocks.getSessionExtensions }));
vi.mock('../../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-proof' }),
}));
vi.mock('../../settings/extensions/agent-api', () => ({
  addToAgent: mocks.addToAgent,
  removeFromAgent: mocks.removeFromAgent,
}));
vi.mock('../../../toasts', () => ({ toastService: { success: vi.fn(), error: vi.fn() } }));

function ChatWithMenu({ sessionId }: { sessionId: string | null }) {
  // What BaseChat does: the chat looks its grant up.
  const access = useChatCrewAccess(sessionId);
  // The Tools popover's extensions list, as ToolsChip holds it (spec 3.7).
  const picker = useExtensionPicker(sessionId, { open: true });
  return (
    <>
      <p data-testid="state">{access.state}</p>
      <p data-testid="count">{picker.answered ? picker.activeCount : ''}</p>
      <ExtensionsPanel picker={picker} />
    </>
  );
}

function renderMenu(sessionId: string | null) {
  mocks.crewHttp.mockImplementation(async (path: string) => {
    if (path === '/connections') return { connections: [connection] };
    if (path === '/connections/conn-1/grants')
      return { grants: [grantRow({ session_id: 'chat-1' })] };
    return {};
  });
  render(
    <MemoryRouter>
      <ChatWithMenu sessionId={sessionId} />
    </MemoryRouter>
  );
}

describe('Crew in the Tools popover', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.getSessionExtensions.mockResolvedValue({
      data: {
        extensions: [
          { type: 'platform', name: 'crew' },
          { type: 'stdio', name: 'spoke' },
        ],
      },
    });
  });

  it('lists no Crew row in a granted chat, and Disable all leaves its tools alone', async () => {
    renderMenu('chat-1');
    await waitFor(() => expect(screen.getByTestId('state')).toHaveTextContent('active'));
    const spoke = await screen.findByRole('option', { name: /^spoke/i });
    await waitFor(() => expect(spoke).toHaveAttribute('aria-selected', 'true'));
    expect(screen.queryByRole('option', { name: /crew/i })).toBeNull();
    expect(screen.getAllByRole('option')).toHaveLength(1);

    // The chip counts what the person added: spoke, not the Crew capability the chat also holds.
    await waitFor(() => expect(screen.getByTestId('count')).toHaveTextContent('1'));

    fireEvent.click(screen.getByRole('button', { name: 'Disable all (1)' }));
    await waitFor(() => expect(mocks.removeFromAgent).toHaveBeenCalledTimes(1));
    expect(mocks.removeFromAgent).toHaveBeenCalledWith('spoke', 'chat-1', true);
    expect(mocks.removeFromAgent).not.toHaveBeenCalledWith('crew', expect.anything(), true);
  });

  it('lists no Crew row on Home either, and does not count it for the next chat', async () => {
    renderMenu(null);
    await screen.findByRole('option', { name: /^spoke/i });
    expect(screen.queryByRole('option', { name: /crew/i })).toBeNull();
    expect(screen.getByTestId('count')).toHaveTextContent('1');
  });
});
