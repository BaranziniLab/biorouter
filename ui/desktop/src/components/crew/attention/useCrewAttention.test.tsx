import { act, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ATTENTION_POLL_MS } from './crewAttention';
import { useCrewAttention } from './useCrewAttention';

const mocks = vi.hoisted(() => ({
  crewHttp: vi.fn(),
  crewRequest: vi.fn(),
  rememberLastChannel: vi.fn(),
  browser: false,
}));

vi.mock('../crewApi', () => ({ crewHttp: mocks.crewHttp, crewRequest: mocks.crewRequest }));
vi.mock('../state/draftStash', () => ({ rememberLastChannel: mocks.rememberLastChannel }));
vi.mock('../../../utils/surface', () => ({ isBrowserSurface: () => mocks.browser }));

let unread: Record<string, number> = {};
const snapshot = () => ({
  workspace: { name: 'chen-lab' },
  channels: [{ id: 'c-general', name: 'general' }],
  actor: { username: 'crew_bob' },
  unread,
});

function installBridge() {
  let open: (target: { connectionId: string; channelId: string }) => void = () => undefined;
  const bridge = {
    setCrewAttentionBadge: vi.fn(),
    notifyCrewAttention: vi.fn(),
    onCrewAttentionOpen: vi.fn((callback: typeof open) => {
      open = callback;
      return vi.fn();
    }),
  };
  Object.defineProperty(window, 'electron', { configurable: true, writable: true, value: bridge });
  return { bridge, open: (target: Parameters<typeof open>[0]) => act(() => open(target)) };
}

function Probe(props: { onCrewRoute: boolean; onOpenChannel(id: string, channel: string): void }) {
  const total = useCrewAttention(props);
  return <span data-testid="total">{total}</span>;
}

describe('useCrewAttention (M2)', () => {
  const original = window.electron;
  beforeEach(() => {
    vi.useFakeTimers();
    mocks.browser = false;
    mocks.crewHttp.mockReset().mockResolvedValue({
      connections: [{ id: 'conn-1', status: 'connected' }],
    });
    mocks.crewRequest.mockReset().mockImplementation(async (_id: string, method: string) =>
      method === 'workspace.snapshot'
        ? snapshot()
        : {
            messages: [{ actor_id: 'p1', body: 'hello @crew_bob' }],
            people: { p1: { username: 'crew_alice', display_name: 'Alice Chen' } },
          }
    );
    mocks.rememberLastChannel.mockReset();
    unread = { 'c-general': 2 };
    vi.spyOn(document, 'hasFocus').mockReturnValue(false);
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
    Object.defineProperty(window, 'electron', {
      configurable: true,
      writable: true,
      value: original,
    });
  });

  const settle = () => act(async () => vi.advanceTimersByTimeAsync(0));
  const nextRound = () => act(async () => vi.advanceTimersByTimeAsync(ATTENTION_POLL_MS));

  it('counts unread messages for the Crew item and the dock, reading as the person', async () => {
    const { bridge } = installBridge();
    const view = render(<Probe onCrewRoute={false} onOpenChannel={vi.fn()} />);
    await settle();
    expect(view.getByTestId('total')).toHaveTextContent('2');
    expect(bridge.setCrewAttentionBadge).toHaveBeenLastCalledWith(2);
    expect(mocks.crewHttp).toHaveBeenCalledWith(
      '/connections',
      'GET',
      undefined,
      expect.anything()
    );
    expect(mocks.crewRequest).toHaveBeenCalledWith(
      'conn-1',
      'workspace.snapshot',
      {},
      false,
      expect.anything()
    );
    view.unmount();
    expect(bridge.setCrewAttentionBadge).toHaveBeenLastCalledWith(0);
  });

  it('asks for a notification when a mention arrives while the window is elsewhere', async () => {
    const { bridge } = installBridge();
    render(<Probe onCrewRoute={false} onOpenChannel={vi.fn()} />);
    await settle();
    unread = { 'c-general': 3 };
    await nextRound();
    expect(mocks.crewRequest).toHaveBeenCalledWith(
      'conn-1',
      'messages.history',
      { channel_id: 'c-general', limit: 1, latest: true },
      false,
      expect.anything()
    );
    expect(bridge.notifyCrewAttention).toHaveBeenCalledWith({
      title: 'Alice Chen mentioned you in #general',
      body: 'chen-lab',
      key: 'conn-1:c-general',
      connectionId: 'conn-1',
      channelId: 'c-general',
    });
  });

  it('says nothing while this window shows Crew in front', async () => {
    const { bridge } = installBridge();
    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    render(<Probe onCrewRoute onOpenChannel={vi.fn()} />);
    await settle();
    unread = { 'c-general': 5 };
    await nextRound();
    expect(bridge.setCrewAttentionBadge).toHaveBeenLastCalledWith(5);
    expect(bridge.notifyCrewAttention).not.toHaveBeenCalled();
  });

  it('opens Crew on the clicked channel', async () => {
    const { open } = installBridge();
    const onOpenChannel = vi.fn();
    render(<Probe onCrewRoute={false} onOpenChannel={onOpenChannel} />);
    await settle();
    open({ connectionId: 'conn-1', channelId: 'c-general' });
    expect(mocks.rememberLastChannel).toHaveBeenCalledWith('conn-1', 'c-general');
    expect(onOpenChannel).toHaveBeenCalledWith('conn-1', 'c-general');
  });

  it('does nothing on a browser surface', async () => {
    installBridge();
    mocks.browser = true;
    const view = render(<Probe onCrewRoute={false} onOpenChannel={vi.fn()} />);
    await settle();
    expect(mocks.crewHttp).not.toHaveBeenCalled();
    expect(view.getByTestId('total')).toHaveTextContent('0');
  });
});
