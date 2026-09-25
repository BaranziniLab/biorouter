import { screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { CrewSessionGrant } from '../api/grants';
import type { CrewMessage } from '../crewApi';
import {
  channelReady,
  connection,
  ids,
  installDaemon,
  mocked,
  renderCrew,
} from '../integration/harness';
import { installResizeObserverStub } from '../test/crewTestUtils';
import {
  AGENT_CHAT_MEMORY_LIMIT,
  agentChatMemoryKey,
  rememberAgentChats,
  rememberedAgentChats,
  withRememberedAgentChats,
} from './agentChatMemory';
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

  it('leaves a task out: its post reads “Your agent” alone, not the task a third time (Q4-20)', () => {
    const chats = ownAgentChatsFrom(
      [
        grant(),
        grant({ run_id: 'task-run', kind: 'task', session_name: 'Crew · #imaging · Sum it' }),
        // A daemon that predates `kind` lists chats only.
        grant({ run_id: 'older', kind: undefined, session_name: 'Plot summary' }),
      ],
      connection.id
    );
    expect([...chats.keys()]).toEqual([chatRun, 'older']);
  });
});

describe('the byline memory (Q4-12)', () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  const chat = (title: string, sessionId = chatSession) => ({ title, sessionId });

  it('remembers every chat by run, per connection, and lays the current ones over it', () => {
    rememberAgentChats(connection.id, new Map([['run-1', chat('First chat')]]));
    rememberAgentChats(connection.id, new Map([['run-2', chat('Second chat')]]));
    rememberAgentChats('conn-2', new Map([['run-9', chat('Elsewhere')]]));
    expect([...rememberedAgentChats(connection.id).keys()]).toEqual(['run-1', 'run-2']);
    const merged = withRememberedAgentChats(
      connection.id,
      new Map([['run-1', chat('First chat, renamed')]])
    );
    expect(merged.get('run-1')?.title).toBe('First chat, renamed');
    expect(merged.get('run-2')?.title).toBe('Second chat');
    expect(merged.has('run-9')).toBe(false);
  });

  it('keeps at most 200 runs, dropping the oldest', () => {
    for (let index = 0; index < AGENT_CHAT_MEMORY_LIMIT + 5; index += 1) {
      rememberAgentChats(connection.id, new Map([[`run-${index}`, chat(`Chat ${index}`)]]));
    }
    const kept = [...rememberedAgentChats(connection.id).keys()];
    expect(kept).toHaveLength(AGENT_CHAT_MEMORY_LIMIT);
    expect(kept[0]).toBe('run-5');
    expect(kept[kept.length - 1]).toBe(`run-${AGENT_CHAT_MEMORY_LIMIT + 4}`);
  });

  it('reads nothing it cannot trust, and survives storage that throws', () => {
    window.localStorage.setItem(
      agentChatMemoryKey(connection.id),
      JSON.stringify([
        { run: 'run-1', title: 'Kept', session: 's-1' },
        { run: 'run-2', title: 42, session: 's-2' },
        { run: '', title: 'No run', session: 's-3' },
        'nonsense',
      ])
    );
    expect([...rememberedAgentChats(connection.id).keys()]).toEqual(['run-1']);
    window.localStorage.setItem(agentChatMemoryKey(connection.id), '{not json');
    expect(rememberedAgentChats(connection.id).size).toBe(0);

    const setItem = vi.spyOn(window.localStorage, 'setItem').mockImplementation(() => {
      throw new Error('QuotaExceededError');
    });
    const getItem = vi.spyOn(window.localStorage, 'getItem').mockImplementation(() => {
      throw new Error('SecurityError');
    });
    try {
      expect(() =>
        rememberAgentChats(connection.id, new Map([['run-3', chat('Third')]]))
      ).not.toThrow();
      expect(rememberedAgentChats(connection.id).size).toBe(0);
    } finally {
      setItem.mockRestore();
      getItem.mockRestore();
    }
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

  it('keeps “Your agent · {chat title}” once the grant is revoked, re-granted or expired (Q4-12)', async () => {
    window.localStorage.clear();
    const daemon = installDaemon({
      messages: [post(ids.messages[0], ids.alice, chatRun, 'Summary of the assay.')],
    });
    let grants: CrewSessionGrant[] = [grant()];
    daemon.state.http = (path, method) =>
      path === `/connections/${connection.id}/grants` && method === 'GET' ? { grants } : undefined;
    const first = renderCrew();
    await channelReady();
    const byline = async () =>
      (await screen.findByText('Summary of the assay.')).closest('article') as HTMLElement;
    await waitFor(async () =>
      expect(await byline()).toHaveTextContent(/^Your agent · Assay results summary/)
    );
    first.unmount();

    // Re-granted: the daemon lists the chat under a new run, and the old run is gone.
    grants = [grant({ run_id: 'e6f7a8b9-c0d1-4e2f-9a3b-4c5d6e7f8a9b' })];
    renderCrew();
    await channelReady();
    await waitFor(() =>
      expect(mocked.crewHttp.mock.calls.some(([path]) => String(path).endsWith('/grants'))).toBe(
        true
      )
    );
    const mine = await byline();
    await waitFor(() =>
      expect(
        within(mine).getByRole('button', { name: 'Assay results summary' })
      ).toBeInTheDocument()
    );
    expect(mine).toHaveTextContent(/^Your agent · Assay results summary/);
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
