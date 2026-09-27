import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { BottomMenuExtensionSelection } from '../../bottom_menu/BottomMenuExtensionSelection';
import { useChatCrewAccess } from './chatCrewAccess';
import { accessCopy } from './copy';
import { connection, grantRow } from './testing';

/**
 * The chat's extension menu must not let the Crew row read as "removed" while the chat's grant is
 * still active (NOVICE-BASELINE F3: "crew · Extension removed" left `expired: false`). The row stays
 * on, says where Revoke is, and Disable all leaves it alone. It reads the chat's own lookup, so the
 * menu itself asks the daemon nothing.
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
      { type: 'platform', name: 'crew', description: 'Crew', enabled: true },
      { type: 'stdio', name: 'spoke', description: 'Spoke', cmd: 'spoke', args: [], enabled: true },
    ],
  }),
}));
vi.mock('../../settings/extensions/subcomponents/ExtensionList', () => ({
  formatExtensionName: (name: string) => name,
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

function ChatWithMenu({ grant }: { grant: boolean }) {
  // What BaseChat does: the chat looks its grant up; the menu only reads the answer.
  const access = useChatCrewAccess(grant ? 'chat-1' : null);
  return (
    <>
      <p data-testid="state">{access.state}</p>
      <BottomMenuExtensionSelection sessionId="chat-1" />
    </>
  );
}

function renderMenu(grants: unknown[], lookUp = true) {
  mocks.crewHttp.mockImplementation(async (path: string) => {
    if (path === '/connections') return { connections: [connection] };
    if (path === '/connections/conn-1/grants') return { grants };
    return {};
  });
  render(
    <MemoryRouter>
      <ChatWithMenu grant={lookUp} />
    </MemoryRouter>
  );
}

async function openMenu() {
  fireEvent.pointerDown(screen.getByLabelText(/Manage extensions/), { button: 0, ctrlKey: false });
  return screen.findByRole('menuitemcheckbox', { name: /crew/ });
}

describe('the Crew row of a chat’s extension menu', () => {
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

  it('stays on and points to Revoke while the chat’s grant is active', async () => {
    renderMenu([grantRow({ session_id: 'chat-1' })]);
    await waitFor(() => expect(screen.getByTestId('state')).toHaveTextContent('active'));
    const crew = await openMenu();
    await waitFor(() => expect(crew).toHaveAttribute('aria-checked', 'true'));
    expect(crew).toHaveAttribute('aria-disabled', 'true');
    expect(crew).toHaveTextContent(accessCopy.extensionLocked);

    fireEvent.click(crew);
    expect(mocks.removeFromAgent).not.toHaveBeenCalled();

    // Disable all leaves the Crew row alone.
    fireEvent.click(screen.getByRole('button', { name: 'Disable all (1)' }));
    await waitFor(() => expect(mocks.removeFromAgent).toHaveBeenCalledTimes(1));
    expect(mocks.removeFromAgent).toHaveBeenCalledWith('spoke', 'chat-1', true);
  });

  it('toggles as before once the grant is revoked', async () => {
    renderMenu([grantRow({ session_id: 'chat-1', expired: true })]);
    await waitFor(() => expect(screen.getByTestId('state')).toHaveTextContent('revoked'));
    const crew = await openMenu();
    await waitFor(() => expect(crew).toHaveAttribute('aria-checked', 'true'));
    expect(crew).not.toHaveAttribute('aria-disabled', 'true');
    fireEvent.click(crew);
    await waitFor(() => expect(mocks.removeFromAgent).toHaveBeenCalledWith('crew', 'chat-1', true));
  });

  it('toggles as before in a chat that never looked anything up', async () => {
    renderMenu([grantRow({ session_id: 'chat-1' })], false);
    const crew = await openMenu();
    await waitFor(() => expect(crew).toHaveAttribute('aria-checked', 'true'));
    expect(crew).not.toHaveAttribute('aria-disabled', 'true');
    expect(mocks.crewHttp).not.toHaveBeenCalled();
  });
});
