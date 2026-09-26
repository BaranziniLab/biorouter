import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { accessCopy } from '../access/copy';
import { forgetUnconfirmedRevocations, UNCONFIRMED_REVOKE_WATCH_MS } from '../access/useCrewGrants';
import { CrewHttpError } from '../crewApi';
import { paneCopy } from '../pane/copy';
import { OFFLINE_FOLLOW_INTERVAL_MS } from '../state/useCrewConnections';
import { installResizeObserverStub } from '../test/crewTestUtils';
import {
  channelReady,
  connection,
  currentCrew,
  ids,
  installDaemon,
  mocked,
  renderCrew,
  richMessages,
  type ScriptedDaemon,
} from './harness';

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: vi.fn(), crewRequest: vi.fn(), observeCrew: vi.fn() };
});
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
 * Final polish NEW-4 (P3). Live (R1), Bob revoked a chat while the workspace was unreachable (503,
 * "Stopped on this device"), the daemon confirmed it by itself once the network was back, and the
 * pane said "Confirmed. The workspace has stopped this chat's access too." — for seven seconds.
 * Then Crew reconnected, the offline screen gave way to the channel, the pane's body mounted again,
 * and it forgot it had ever waited: a person who looked away for ten seconds never saw the
 * confirmation. The note now stays until the person dismisses it or leaves the pane.
 */

const REVOKE = `/connections/${connection.id}/sessions/chat-1/revoke`;
const GRANTS = `/connections/${connection.id}/grants`;
const DAEMON_SENTENCE =
  'Room observation ended. Clear cached room content and refresh authorized access; a stale cursor requires an explicit fresh history selection.';

let daemon: ScriptedDaemon;
let saved: Record<string, unknown>;
let standing: 'active' | 'unconfirmed' | 'confirmed';

function grant() {
  return {
    session_id: 'chat-1',
    run_id: 'run-1',
    connection_id: connection.id,
    channel_id: ids.general,
    source_channels: [ids.general],
    policy_epoch: 1,
    expired: standing !== 'active',
    kind: 'chat',
    session_name: 'Crew context check',
    expires_at: Math.floor(Date.now() / 1000) + 3600,
    labels: { destination: { channel_id: ids.general, label: '#general' } },
    ...(standing === 'active' ? {} : { revocation: standing }),
  };
}

async function wait(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

const openPane = () => document.querySelector<HTMLElement>('aside.crew-pane[data-state="open"]');
const confirmedNote = () => screen.queryByTestId('crew-access-confirmed');

/** Open the chat's Chat access pane from its Agents row, as Bob did. */
async function openChatAccess(): Promise<HTMLElement> {
  fireEvent.click(await screen.findByTestId('crew-agents-chat'));
  return waitFor(() => {
    const aside = openPane();
    expect(aside).not.toBeNull();
    return aside!;
  });
}

/**
 * R1, as it happened: Crew is on #general; the bridge drops; the person revokes the chat from the
 * pane beside the offline screen (503); the daemon confirms by itself; then it re-dials, and Crew
 * follows it back to the channel.
 */
async function revokeOfflineThenConfirm() {
  renderCrew();
  await channelReady();

  saved = { ...connection, status: 'disconnected' };
  act(() =>
    daemon.emit({ type: 'error', code: 'observation_refused', clear: true, error: DAEMON_SENTENCE })
  );
  await waitFor(() => expect(currentCrew().screen).toBe('offline'));

  const pane = await openChatAccess();
  fireEvent.click(within(pane).getByRole('button', { name: accessCopy.revokeButton }));
  fireEvent.click(within(pane).getByRole('button', { name: accessCopy.confirmRevoke }));
  expect(await within(pane).findByText(accessCopy.unconfirmed)).toBeInTheDocument();

  // The workspace confirms (the daemon asked again by itself); the pane's watch reads it.
  standing = 'confirmed';
  await wait(UNCONFIRMED_REVOKE_WATCH_MS);
  await waitFor(() => expect(confirmedNote()).not.toBeNull());
  expect(currentCrew().screen).toBe('offline');

  // The daemon's re-dial: Crew follows it back to the channel, and the pane's body mounts again.
  saved = { ...connection, status: 'connected' };
  await wait(OFFLINE_FOLLOW_INTERVAL_MS);
  await channelReady();
  await waitFor(() => expect(currentCrew().screen).toBe('channel'));
}

beforeEach(() => {
  vi.clearAllMocks();
  forgetUnconfirmedRevocations();
  window.localStorage.clear();
  saved = { ...connection, status: 'connected' };
  standing = 'active';
  daemon = installDaemon({
    messages: richMessages(),
    http: (path, method) => {
      if (path === '/connections' && method === 'GET') return { connections: [saved] };
      if (path === GRANTS && method === 'GET') return { grants: [grant()] };
      if (path === REVOKE && method === 'POST') {
        standing = 'unconfirmed';
        throw new CrewHttpError(
          'Stopped on this device. The workspace hasn’t confirmed yet; Biorouter confirms it by itself when the connection is back.',
          503,
          'crew_revocation_unconfirmed'
        );
      }
      return undefined;
    },
  });
  // The daemon refuses to observe a connection it calls disconnected, as it does.
  const answer = mocked.observeCrew.getMockImplementation()!;
  mocked.observeCrew.mockImplementation(
    async (
      connectionId: string,
      channelId: string | undefined,
      after: string | null,
      signal: AbortSignal,
      deliver: (frame: unknown) => void
    ) => {
      if (signal.aborted) return 'terminal';
      if (saved.status !== 'connected') {
        deliver({
          type: 'error',
          code: 'observation_refused',
          clear: true,
          error: DAEMON_SENTENCE,
        });
        return 'terminal';
      }
      return answer(connectionId, channelId, after, signal, deliver);
    }
  );
  vi.useFakeTimers({ shouldAdvanceTime: true });
});
afterEach(() => {
  vi.useRealTimers();
});

describe('“Confirmed.” stays until the person is done with it (NEW-4)', () => {
  it('outlives Crew reconnecting and the pane mounting again, and minutes after', async () => {
    await revokeOfflineThenConfirm();

    // The same pane, now beside the channel: the confirmation is still there.
    const pane = openPane();
    expect(pane).not.toBeNull();
    expect(within(pane!).getByTestId('crew-access-confirmed')).toHaveTextContent(
      accessCopy.confirmed
    );
    // And it stays: no timer, no refresh, no re-read takes it away.
    await wait(5 * 60_000);
    expect(confirmedNote()).toHaveTextContent(accessCopy.confirmed);
    // Nothing connected but the daemon: the person never pressed Connect.
    expect(
      mocked.crewHttp.mock.calls.filter(([path]) => String(path).endsWith('/connect'))
    ).toHaveLength(0);
  });

  it('goes when the person dismisses it, and does not come back', async () => {
    await revokeOfflineThenConfirm();
    const note = await waitFor(() => {
      const shown = confirmedNote();
      expect(shown).not.toBeNull();
      return shown!;
    });
    fireEvent.click(within(note).getByRole('button', { name: accessCopy.confirmedDismissName }));
    await waitFor(() => expect(confirmedNote()).toBeNull());
    await wait(2 * UNCONFIRMED_REVOKE_WATCH_MS);
    expect(confirmedNote()).toBeNull();
  });

  it('goes when the person leaves the pane: opened again, it is a new look', async () => {
    await revokeOfflineThenConfirm();
    await waitFor(() => expect(confirmedNote()).not.toBeNull());
    fireEvent.click(screen.getByRole('button', { name: paneCopy.closeChatAccess }));
    await waitFor(() => expect(openPane()).toBeNull());

    // The Agents section hides a revoked chat at rest; the person opens its access again from
    // another row (the Access tab's, say), which is a new pane intent.
    act(() => currentCrew().openPane({ mode: 'chat-access', sessionId: 'chat-1' }));
    await waitFor(() =>
      expect(openPane()).toHaveTextContent(accessCopy.paneRevoked('Crew context check'))
    );
    expect(confirmedNote()).toBeNull();
  });
});
