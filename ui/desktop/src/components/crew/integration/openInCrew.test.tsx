import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { crewSelectRouteState } from '../state/useCrewConnections';
import { installResizeObserverStub } from '../test/crewTestUtils';
import {
  channelReady,
  connection,
  currentCrew,
  ids,
  installDaemon,
  mocked,
  ownedRun,
  renderCrew,
  richMessages,
  type ScriptedDaemon,
} from './harness';

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

// jsdom has no layout, so it has no `scrollIntoView`; the timeline brings the row into view.
const scrollIntoView = vi.fn();
const original = Element.prototype.scrollIntoView;
beforeAll(() => {
  Element.prototype.scrollIntoView = scrollIntoView;
});
afterAll(() => {
  Element.prototype.scrollIntoView = original;
});

/**
 * Chat parity WI-16, SECURITY-SENSITIVE (human review). A chat's "Open in Crew", and a clicked
 * Crew notification, arrive with `crewSelect`: the connection, and the channel, message or task
 * the daemon's answer named. Crew selects them and brings the row into view with the one-shot
 * wash. It never connects: a connection the daemon calls disconnected opens offline, and
 * connecting stays the person's click.
 */

/** A second saved connection, which the daemon calls disconnected. */
const offline = {
  ...connection,
  id: 'conn-2',
  name: 'Offline lab',
  workspace_id: 'b9a8c7d6-e5f4-4a3b-9c2d-1e0f9a8b7c6d',
  status: 'disconnected' as const,
};

/** A message in #methods, which the request points at. */
const methodsNote = {
  channel_id: ids.methods,
  restricted: false,
  source_channels: [ids.methods],
  attachments: [] as string[],
  id: '7d6e5f4a-3b2c-4d1e-8f0a-9b8c7d6e5f4a',
  sequence: '7d6e5f4a-3b2c-4d1e-8f0a-9b8c7d6e5f4a',
  actor_id: ids.alice,
  body: 'status update',
  created_at: Math.floor(Date.now() / 1000) - 60,
};

let daemon: ScriptedDaemon;

function connectsTo(id: string): number {
  return mocked.crewHttp.mock.calls.filter(
    ([path, method]) => path === `/connections/${id}/connect` && method === 'POST'
  ).length;
}

/** `/crew` with a request for `target`, as the chat's row and the notification send it. */
function arrive(target: Parameters<typeof crewSelectRouteState>[0]) {
  const state = crewSelectRouteState(target);
  if (!state) throw new Error('not a request');
  return renderCrew({ pathname: '/crew', state });
}

/** The row that draws the message with `body`. */
function messageRow(body: string): HTMLElement {
  const row = screen.getByText(body).closest<HTMLElement>('.crew-message-row');
  if (!row) throw new Error(`No message row draws ${body}.`);
  return row;
}

beforeEach(() => {
  vi.clearAllMocks();
  window.localStorage.clear();
  daemon = installDaemon({
    connections: [connection, offline],
    messages: [...richMessages(), methodsNote],
  });
  // The daemon holds no bridge for the offline connection, so it observes nothing there.
  const observe = mocked.observeCrew.getMockImplementation();
  mocked.observeCrew.mockImplementation(
    async (
      connectionId: string,
      channelId: string | undefined,
      after: string | null,
      signal: AbortSignal,
      deliver: (frame: unknown) => void
    ) => {
      if (connectionId !== offline.id)
        return observe?.(connectionId, channelId, after, signal, deliver);
      deliver({
        type: 'error',
        code: 'observation_refused',
        clear: true,
        error: 'Crew is not connected to this workspace.',
      });
      return 'terminal';
    }
  );
});

describe('Open in Crew selects where a chat’s change went (WI-16)', () => {
  it('opens the channel and washes the message once, without connecting anything', async () => {
    arrive({ connectionId: connection.id, channelId: ids.methods, messageId: methodsNote.id });
    await channelReady('methods');
    expect(currentCrew().channelId).toBe(ids.methods);
    await waitFor(() => expect(messageRow('status update')).toHaveClass('crew-highlight'));
    expect(scrollIntoView).toHaveBeenCalledWith(expect.objectContaining({ block: 'center' }));
    fireEvent.animationEnd(messageRow('status update'));
    expect(messageRow('status update')).not.toHaveClass('crew-highlight');
    expect(connectsTo(connection.id)).toBe(0);

    // Once per request: another channel and back does not wash it again.
    act(() => currentCrew().selectChannel(ids.general));
    await channelReady('general');
    act(() => currentCrew().selectChannel(ids.methods));
    await channelReady('methods');
    expect(messageRow('status update')).not.toHaveClass('crew-highlight');
  });

  it('washes the task a chat started, in its channel', async () => {
    daemon.state.runs = [ownedRun];
    arrive({ connectionId: connection.id, channelId: ids.general, runId: ownedRun.run_id });
    await channelReady('general');
    await waitFor(() =>
      expect(document.querySelector('.crew-task-row')).toHaveClass('crew-highlight')
    );
  });

  it('selects an offline connection and leaves it offline: connecting is the person’s', async () => {
    arrive({ connectionId: offline.id, channelId: ids.methods, messageId: methodsNote.id });
    await waitFor(() => expect(currentCrew().connectionId).toBe(offline.id));
    await waitFor(() => expect(currentCrew().screen).toBe('offline'));
    expect(connectsTo(offline.id)).toBe(0);
    expect(connectsTo(connection.id)).toBe(0);
    expect(document.querySelector('.crew-highlight')).toBeNull();
  });

  it('keeps the person’s own choice of connection once it moved there', async () => {
    arrive({ connectionId: offline.id, channelId: ids.methods });
    await waitFor(() => expect(currentCrew().connectionId).toBe(offline.id));
    act(() => currentCrew().selectConnection(connection.id));
    await channelReady();
    expect(currentCrew().connectionId).toBe(connection.id);
    expect(connectsTo(offline.id)).toBe(0);
  });

  it('selects nothing for a connection Crew does not list, nor for a request of the wrong shape', async () => {
    expect(crewSelectRouteState({ connectionId: '../connections' })).toBeNull();
    arrive({ connectionId: 'conn-removed', channelId: ids.methods });
    await channelReady('general');
    expect(currentCrew().connectionId).toBe(connection.id);
    expect(currentCrew().channelId).toBe(ids.general);
  });
});
