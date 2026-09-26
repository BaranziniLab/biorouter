import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { accessCopy } from '../access/copy';
import { paneCopy } from '../pane/copy';
import { forgetUnconfirmedRevocations } from '../access/useCrewGrants';
import { CrewHttpError } from '../crewApi';
import { installResizeObserverStub } from '../test/crewTestUtils';
import {
  connection,
  currentCrew,
  installDaemon,
  mocked,
  renderCrew,
  richMessages,
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
 * Final acceptance F2: while the workspace was offline, the Crew view offered no Revoke. The
 * Agents section's chat row set the Chat access pane's intent, and no screen outside a channel drew
 * the pane, so the click did nothing. A revoke needs no connection — the daemon stops the grant on
 * this device at once and confirms it with the workspace once it is back — so the pane now opens
 * beside the offline screen, with Revoke.
 */

const REVOKE = `/connections/${connection.id}/sessions/chat-1/revoke`;
const DAEMON_SENTENCE =
  'Room observation ended. Clear cached room content and refresh authorized access; a stale cursor requires an explicit fresh history selection.';

let expired: boolean;

function grant() {
  return {
    session_id: 'chat-1',
    run_id: 'run-1',
    connection_id: connection.id,
    channel_id: 'channel-general',
    source_channels: ['channel-general'],
    policy_epoch: 1,
    expired,
    kind: 'chat',
    session_name: 'Slack data summary request',
    expires_at: Math.floor(Date.now() / 1000) + 3600,
    labels: { destination: { channel_id: 'channel-general', label: '#data' } },
    ...(expired ? { revocation: 'unconfirmed' } : {}),
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  forgetUnconfirmedRevocations();
  window.localStorage.clear();
  expired = false;
  installDaemon({
    messages: richMessages(),
    connections: [{ ...connection, status: 'disconnected' }],
    http: (path, method) => {
      if (path === `/connections/${connection.id}/grants` && method === 'GET')
        return { grants: [grant()] };
      if (path === REVOKE && method === 'POST') {
        expired = true;
        throw new CrewHttpError(
          'Stopped on this device. The workspace has not confirmed yet.',
          503,
          'crew_revocation_unconfirmed'
        );
      }
      return undefined;
    },
  });
  // The daemon refuses to observe a connection it calls disconnected, as it does.
  mocked.observeCrew.mockImplementation(
    async (
      _connection: string,
      _channel: string | undefined,
      _after: string | null,
      _signal: AbortSignal,
      deliver: (frame: unknown) => void
    ) => {
      deliver({ type: 'error', code: 'observation_refused', clear: true, error: DAEMON_SENTENCE });
      return 'terminal';
    }
  );
});

describe('Revoke in the Crew view while the workspace is offline (F2)', () => {
  it('opens the chat’s access beside the offline screen, and revokes on this device at once', async () => {
    renderCrew();
    await waitFor(() => expect(currentCrew().screen).toBe('offline'));
    const row = await screen.findByTestId('crew-agents-chat');
    expect(row).toHaveTextContent('Slack data summary request');

    fireEvent.click(row);
    const pane = await waitFor(() => {
      const aside = document.querySelector<HTMLElement>('aside.crew-pane');
      expect(aside).toHaveAttribute('data-state', 'open');
      return aside!;
    });
    // The offline screen and its Connect stay beside it.
    expect(currentCrew().screen).toBe('offline');
    expect(
      within(pane).getByRole('heading', { name: paneCopy.chatAccessTitle })
    ).toBeInTheDocument();
    // Named as the person granted it, with no dangling "as" for a person no view names.
    expect(pane).toHaveTextContent('Posts in #data');
    expect(pane).not.toHaveTextContent(/as\s*$/);

    fireEvent.click(within(pane).getByRole('button', { name: accessCopy.revokeButton }));
    fireEvent.click(within(pane).getByRole('button', { name: accessCopy.confirmRevoke }));

    await waitFor(() =>
      expect(
        mocked.crewHttp.mock.calls.some(([path, method]) => path === REVOKE && method === 'POST')
      ).toBe(true)
    );
    // Stopped here at once; the workspace confirms when Crew is back — said without telling the
    // person to do anything.
    expect(await within(pane).findByText(accessCopy.unconfirmed)).toBeInTheDocument();
    expect(within(pane).queryByText(/Access revoked/)).toBeNull();
    expect(within(pane).getByRole('button', { name: accessCopy.retry })).toBeInTheDocument();
  });
});
