import { beforeEach, describe, expect, it, vi } from 'vitest';
import { agentAddExtension, agentRemoveExtension } from '../../../api';
import { userActionHeaders } from '../../../utils/userAction';
import { addToAgent, removeFromAgent } from './agent-api';
import { toastService } from '../../../toasts';

vi.mock('../../../api', () => ({
  agentAddExtension: vi.fn(),
  agentRemoveExtension: vi.fn(),
}));
vi.mock('../../../utils/userAction', () => ({
  userActionHeaders: vi.fn(),
}));
vi.mock('../../../toasts', () => ({
  toastService: {
    loading: vi.fn(),
    dismiss: vi.fn(),
    success: vi.fn(),
    error: vi.fn(),
  },
}));
vi.mock('../../../utils/crossAffiliationNotice', () => ({
  showCrossAffiliationNotice: vi.fn(),
}));
vi.mock('../../../utils/extensionErrorUtils', () => ({
  createExtensionRecoverHints: vi.fn(() => ''),
  formatExtensionErrorMessage: vi.fn((message: string) => message),
}));

describe('agent extension API', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('proves a user-requested extension removal for a child session', async () => {
    vi.mocked(userActionHeaders).mockResolvedValue({ 'X-User-Action': 'proof' });
    vi.mocked(agentRemoveExtension).mockResolvedValue({ data: {} } as never);

    await removeFromAgent('developer', 'child-session', false);

    expect(agentRemoveExtension).toHaveBeenCalledWith({
      headers: { 'X-User-Action': 'proof' },
      body: { session_id: 'child-session', name: 'developer' },
      throwOnError: true,
    });
  });

  it('announces successful live attach and detach for the addressed chat', async () => {
    vi.mocked(userActionHeaders).mockResolvedValue({ 'X-User-Action': 'proof' });
    vi.mocked(agentAddExtension).mockResolvedValue({ data: '' } as never);
    vi.mocked(agentRemoveExtension).mockResolvedValue({ data: {} } as never);
    const changed = vi.fn();
    window.addEventListener('session-tools:changed', changed);

    await addToAgent(
      {
        type: 'stdio',
        name: 'notes',
        description: 'Notes',
        cmd: 'notes',
        args: [],
      },
      'chat-1',
      false
    );
    await removeFromAgent('notes', 'chat-1', false);

    expect(changed).toHaveBeenCalledTimes(2);
    expect(changed.mock.calls.map(([event]) => (event as CustomEvent).detail)).toEqual([
      { sessionId: 'chat-1' },
      { sessionId: 'chat-1' },
    ]);
    window.removeEventListener('session-tools:changed', changed);
  });

  it('does not invent proof when the user-action bridge is unavailable', async () => {
    vi.mocked(userActionHeaders).mockResolvedValue({});
    const refusal = { message: 'forbidden' };
    vi.mocked(agentRemoveExtension).mockRejectedValue(refusal);

    await expect(removeFromAgent('developer', 'child-session', false)).rejects.toBe(refusal);
    expect(agentRemoveExtension).toHaveBeenCalledWith(expect.objectContaining({ headers: {} }));
  });

  it('says what it is doing in the toast with the typographic ellipsis and no repeated name', async () => {
    vi.mocked(userActionHeaders).mockResolvedValue({});
    vi.mocked(agentAddExtension).mockResolvedValue({ data: '' } as never);
    vi.mocked(agentRemoveExtension).mockResolvedValue({ data: {} } as never);

    await addToAgent(
      { type: 'stdio', name: 'notes', description: 'Notes', cmd: 'notes', args: [] },
      'chat-1',
      true
    );
    await removeFromAgent('notes', 'chat-1', true);

    // The toast's own title is the extension's name, so the message does not say it again.
    expect(toastService.loading).toHaveBeenNthCalledWith(1, {
      title: 'notes',
      msg: 'Adding extension…',
    });
    expect(toastService.loading).toHaveBeenNthCalledWith(2, {
      title: 'notes',
      msg: 'Removing extension…',
    });
  });
});
