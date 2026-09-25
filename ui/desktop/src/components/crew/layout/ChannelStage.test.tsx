import { screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { CrewSessionGrant } from '../api/grants';
import type { CrewMessage } from '../crewApi';
import { channelReady, connection, ids, installDaemon, renderCrew } from '../integration/harness';
import { installResizeObserverStub } from '../test/crewTestUtils';
import { ownAgentChatsFrom } from './ChannelStage';

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
 * Q3-22: every agent post read "Your agent @crew_gina · Agent", whether a task or one of the
 * viewer's connected chats had posted it, so with several chats connected nobody could tell which
 * conversation said what. The layout now hands the timeline the viewer's OWN chats, from this
 * device's grant list, by the run each posts as.
 */

const chatRun = 'c4d5e6f7-a8b9-4c0d-9e1f-2a3b4c5d6e7f';
const chatSession = 'd5e6f7a8-b9c0-4d1e-8f2a-3b4c5d6e7f8a';

function grant(overrides: Partial<CrewSessionGrant> = {}): CrewSessionGrant {
  return {
    session_id: chatSession,
    run_id: chatRun,
    connection_id: connection.id,
    channel_id: ids.general,
    source_channels: [ids.general],
    policy_epoch: 1,
    expired: false,
    kind: 'chat',
    session_name: 'Assay results summary',
    ...overrides,
  };
}

describe('ownAgentChatsFrom', () => {
  it('keeps the titled chats of this connection, by run', () => {
    const chats = ownAgentChatsFrom(
      [
        grant(),
        grant({ run_id: 'untitled', session_name: null }),
        grant({ run_id: 'blank', session_name: '   ' }),
        grant({ run_id: 'elsewhere', connection_id: 'conn-2' }),
      ],
      connection.id
    );
    expect([...chats.entries()]).toEqual([
      [chatRun, { title: 'Assay results summary', sessionId: chatSession }],
    ]);
  });
});

describe('the viewer’s chats head their agents’ posts (Q3-22)', () => {
  const now = Math.floor(Date.now() / 1000);
  const post = (id: string, actor: string, run: string, body: string): CrewMessage => ({
    id,
    sequence: id,
    channel_id: ids.general,
    actor_id: actor,
    run_id: run,
    body,
    created_at: now - 60,
    restricted: false,
    source_channels: [ids.general],
    attachments: [],
  });

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('reads “Your agent · {chat title}” for the viewer’s chat, and nothing of anyone else’s', async () => {
    const daemon = installDaemon({
      messages: [
        post(ids.messages[0], ids.alice, chatRun, 'Summary of the assay.'),
        // Bob's agent, posting under a run this device holds no grant for.
        post(ids.messages[1], ids.bob, ids.run, 'Plot attached.'),
      ],
    });
    daemon.state.http = (path, method) =>
      path === `/connections/${connection.id}/grants` && method === 'GET'
        ? { grants: [grant()] }
        : undefined;
    renderCrew();
    await channelReady();
    const mine = (await screen.findByText('Summary of the assay.')).closest(
      'article'
    ) as HTMLElement;
    await waitFor(() =>
      expect(
        within(mine).getByRole('button', { name: 'Assay results summary' })
      ).toBeInTheDocument()
    );
    expect(mine).toHaveTextContent(/^Your agent · Assay results summary/);
    const theirs = screen.getByText('Plot attached.').closest('article') as HTMLElement;
    expect(theirs).toHaveTextContent(/Bob Lee's agent/);
    expect(theirs).not.toHaveTextContent('Assay results summary');
  });
});
