import { act, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { announceSameRouteReset } from '../../../hooks/useSameRouteReset';
import { installResizeObserverStub } from '../test/crewTestUtils';
import { channelReady, currentCrew, installDaemon, renderCrew, richMessages } from './harness';

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: vi.fn(), crewRequest: vi.fn(), observeCrew: vi.fn() };
});
// Stable across renders, as the real context's callbacks are.
const config = vi.hoisted(() => ({
  getProviders: async () => [],
  read: async () => '',
  getProviderModels: async () => [],
}));
vi.mock('../../ConfigContext', async () => {
  const actual = await vi.importActual<typeof import('../../ConfigContext')>('../../ConfigContext');
  return { ...actual, useConfig: () => config };
});
vi.mock('../CrewAuthentication', () => ({ default: () => <div /> }));

installResizeObserverStub();

/**
 * SF-F5: choosing Crew in the app's sidebar on a chat's access link (`/crew?sessionId=…`) kept the
 * link's chat, so its connect note and refusal stayed until the person went Home and back. The
 * sidebar announces a same-route reset for a path it is already on; Crew now hears it.
 */
describe('choosing Crew in the sidebar on a chat-access link (SF-F5)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    installDaemon({ messages: richMessages() });
  });

  it('opens plain Crew: the link’s chat and its Chat access pane go', async () => {
    renderCrew('/crew?sessionId=agent-1');
    await channelReady();
    expect(currentCrew().grantSessionId).toBe('agent-1');
    act(() => currentCrew().openPane({ mode: 'chat-access', sessionId: 'agent-1' }));
    expect(currentCrew().ui.pane?.mode).toBe('chat-access');

    act(() => announceSameRouteReset('/crew'));
    await waitFor(() => expect(currentCrew().grantSessionId).toBeNull());
    expect(currentCrew().ui.pane).toBeNull();
    // Still Crew, on the channel it showed.
    await channelReady();
  });

  it('keeps the link when another page is re-selected', async () => {
    renderCrew('/crew?sessionId=agent-1');
    await channelReady();
    act(() => announceSameRouteReset('/schedules'));
    expect(currentCrew().grantSessionId).toBe('agent-1');
  });

  it('leaves plain Crew as it is', async () => {
    renderCrew('/crew');
    await channelReady();
    act(() => currentCrew().openPane({ mode: 'details' }));
    act(() => announceSameRouteReset('/crew'));
    expect(currentCrew().ui.pane?.mode).toBe('details');
  });
});
