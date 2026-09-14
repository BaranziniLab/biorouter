import React, { StrictMode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor, act } from '@testing-library/react';

/**
 * Item 10 (1.90.4): opening a chat sent the SAME full `GET /sessions/{id}` about
 * ten times — measured with CDP in the running app on 2026-09-13 as ten identical
 * requests inside two milliseconds on a plain reload into a chat.
 *
 * This spec mounts what `BaseChat` mounts for a chat's row, from the real
 * modules: the chat store (`useChatStream`, whose `/agent/resume` loads the
 * chat), the subagent header (`useSubagentSession`), the composer slot's own
 * decision (`subagentComposerKind` → `SubagentComposerSlot`) and the composer
 * (`ChatInput`, which reads the row for its working directory and its tier) —
 * through the real generated client and the transport `renderer.tsx` installs
 * (`daemonClientConfig`), with only the network faked. It counts what is SENT.
 *
 * ⚠ **Both surfaces, because they time the composer differently**, and a count
 * that holds on one says nothing about the other. The desktop mounts the
 * composer with the chat; a browser WITHHOLDS it until the store's row has
 * landed (`composerSlotMode`). An independent tester measured the second under
 * `biorouter serve`: the header's own read went out at 16 ms and the composer's
 * 55–290 ms later, after `/agent/resume` answered — two requests on almost every
 * open, where the desktop sent one. So `/agent/resume` answers here after
 * {@link RESUME_MS}, well past the coalescer's hold, as it did there. With it at
 * 0 the browser case passes against the header that read for itself, which is
 * why the delay is asserted rather than assumed.
 */

vi.mock('./ConfigContext', () => ({
  useConfig: () => ({
    getProviders: vi.fn(async () => []),
    read: vi.fn(async () => null),
  }),
}));
vi.mock('./ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    getCurrentModelAndProvider: vi.fn(async () => ({ model: null, provider: null })),
    currentModel: null,
    currentProvider: null,
    currentModelSupportsVision: false,
    currentModelSupportedInputMimeTypes: null,
  }),
}));
vi.mock('../hooks/useDiverge', () => ({
  useDiverge: () => ({ diverge: vi.fn() }),
}));
vi.mock('./settings/models/bottom_bar/ModelsBottomBar', () => ({
  default: ({ privacyTier }: { privacyTier?: string }) => (
    <div data-testid="tier-probe">{privacyTier ?? 'unresolved'}</div>
  ),
}));
vi.mock('./bottom_menu/BottomMenuExtensionSelection', () => ({
  BottomMenuExtensionSelection: () => null,
}));
vi.mock('./bottom_menu/BottomMenuSkillSelection', () => ({
  BottomMenuSkillSelection: () => null,
}));
vi.mock('./bottom_menu/BottomMenuKnowledgeSelection', () => ({
  BottomMenuKnowledgeSelection: () => null,
}));
vi.mock('./bottom_menu/BottomMenuReasoningEffort', () => ({
  BottomMenuReasoningEffort: () => null,
}));
vi.mock('./bottom_menu/CostTracker', () => ({
  CostTracker: () => null,
}));
vi.mock('./MessageQueue', () => ({
  default: () => null,
}));
vi.mock('./MentionPopover', () => {
  const MentionPopoverMock = React.forwardRef(() => null);
  MentionPopoverMock.displayName = 'MentionPopoverMock';
  return { default: MentionPopoverMock };
});

import ChatInput from './ChatInput';
import { ChatState } from '../types/chatState';
import { client } from '../api/client.gen';
import { daemonClientConfig } from '../utils/daemonClient';
import { SESSION_READ_HOLD_MS } from '../utils/sessionReadCoalescing';
import { useSubagentSession } from './subagent/useSubagentSession';
import { subagentComposerKind } from './subagent/subagentReadOnly';
import { SubagentComposerSlot } from './subagent/SubagentComposerSlot';
import { useChatStream } from '../hooks/useChatStream';
import { defaultChatStreamRegistry } from '../hooks/chatStreamStore';
import { BROWSER_SURFACE_MARKER } from '../utils/surface';
import { resetHostProviderForTests } from '../utils/userAction';

const DAEMON = 'http://127.0.0.1:4711';

/** How long `/agent/resume` takes — inside the 12–232 ms measured under serve, never 0. */
const RESUME_MS = 80;

/**
 * A fresh id per test: the transcript LRU (`utils/sessionNameSync`) is
 * module-level and keyed by session id, and a reused id would load from it
 * without the `/agent/resume` whose timing this spec is about.
 */
let seq = 0;
const nextChat = () => `20260913_${++seq}`;

type Sent = {
  method: string;
  url: URL;
  proof: string | null;
  callerProvider: string | null;
};

type Row = Record<string, unknown> & { id: string; session_type: string };

function fakeDaemon(row: Row, { resumeRefused = false } = {}) {
  const sent: Sent[] = [];
  const transcript = [
    {
      role: 'user',
      created: 1,
      content: [{ type: 'text', text: '## Subagent spawn context\ntask: count the files' }],
      metadata: { userVisible: true, agentVisible: false, provenance: { kind: 'spawn_context' } },
    },
  ];
  const json = (body: unknown, status = 200) =>
    new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
  const fetchMock = vi.fn(async (input: Parameters<typeof fetch>[0], init?: RequestInit) => {
    const request = input instanceof Request ? input : new Request(input, init);
    const url = new URL(request.url);
    sent.push({
      method: request.method,
      url,
      proof: request.headers.get('X-User-Action'),
      callerProvider: request.headers.get('X-Caller-Provider'),
    });
    if (url.pathname === '/agent/resume') {
      await new Promise((resolve) => setTimeout(resolve, RESUME_MS));
      if (resumeRefused) {
        return new Response('This daemon was started without a user-action key.', {
          status: 403,
        });
      }
      return json({ session: { ...row, conversation: transcript } });
    }
    if (url.pathname === `/sessions/${row.id}`) {
      const metadataOnly = url.searchParams.get('metadata_only') === 'true';
      return json({ ...row, conversation: metadataOnly ? null : transcript });
    }
    if (url.pathname === `/sessions/${row.id}/extensions`) {
      return json({ extensions: [{ type: 'platform', name: 'developer' }] });
    }
    if (url.pathname === `/sessions/${row.id}/events`) {
      // An observer's feed: stays open until the store lets go of it.
      return new Promise<Response>((_, reject) => {
        request.signal.addEventListener('abort', () =>
          reject(new DOMException('The operation was aborted.', 'AbortError'))
        );
      });
    }
    if (url.pathname === '/config/read') return json('versa_azure');
    return json({});
  });
  const rowReads = () =>
    sent.filter((s) => s.method === 'GET' && s.url.pathname === `/sessions/${row.id}`);
  return { sent, fetchMock, rowReads };
}

const describeRead = (s: Sent) => `${s.url.pathname}${s.url.search}`;

function Composer({ sessionId }: { sessionId: string }) {
  return (
    <ChatInput
      sessionId={sessionId}
      handleSubmit={vi.fn()}
      chatState={ChatState.Idle}
      onStop={vi.fn()}
      initialValue=""
      setView={vi.fn()}
      totalTokens={0}
      accumulatedInputTokens={0}
      accumulatedOutputTokens={0}
      droppedFiles={[]}
      onFilesProcessed={vi.fn()}
      messagesLength={0}
      disableAnimation={false}
      toolCount={0}
      onWorkingDirChange={vi.fn()}
    />
  );
}

const onStreamFinish = () => {};

/**
 * `BaseChat`'s reads of a chat's row, in `BaseChat`'s arrangement: the store
 * loads it, the header hook and the slot decide from it, and the composer sits
 * in one of two layouts — the empty-chat one, then the transcript's, which
 * remounts it (`isCleanConversation`).
 */
function ChatOpen({ sessionId }: { sessionId: string }) {
  const { session, messages, chatState, sessionLoadError } = useChatStream({
    sessionId,
    onStreamFinish,
  });
  const subagent = useSubagentSession(sessionId);
  const kind = subagentComposerKind({
    sessionId,
    loadedSessionId: session?.id,
    loadedSessionType: session?.session_type,
    hookSaysSubagent: subagent.isSubagent,
    loadFailed: sessionLoadError !== undefined,
  });
  const slot = (
    <SubagentComposerSlot kind={kind}>
      <Composer sessionId={sessionId} />
    </SubagentComposerSlot>
  );
  return (
    <div>
      <div data-testid="subagent-probe">
        {subagent.isSubagent
          ? `subagent of ${subagent.parentSessionId} with ${subagent.extensions.join(',')}`
          : 'ordinary'}
      </div>
      {messages.length === 0 && chatState === ChatState.Idle ? (
        <div data-layout="clean">{slot}</div>
      ) : (
        <section data-layout="transcript">{slot}</section>
      )}
    </div>
  );
}

async function settle(ms = RESUME_MS + 150) {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
  });
}

beforeEach(() => {
  Object.assign(window, {
    appConfig: { get: () => '/default/workdir' },
    electron: {
      directoryChooser: vi.fn(),
      addRecentDir: vi.fn(),
      logInfo: vi.fn(),
      showNotification: vi.fn(),
      getPathForFile: vi.fn(() => ''),
      on: vi.fn(),
      off: vi.fn(),
      getUserActionKey: vi.fn(async () => 'proof-of-person'),
    },
  });
  client.setConfig(daemonClientConfig(DAEMON, 'daemon-secret'));
});

afterEach(() => {
  delete document.documentElement.dataset.biorouterSurface;
  resetHostProviderForTests();
  defaultChatStreamRegistry.resetForTests();
  client.setConfig({ baseUrl: 'http://localhost', headers: {}, fetch: undefined });
  vi.unstubAllGlobals();
});

describe('opening a chat reads its row once', () => {
  it('desktop: ONE metadata read, with the proof, however many readers mount', async () => {
    const chat = nextChat();
    const daemon = fakeDaemon({
      id: chat,
      privacy_tier: 'private',
      working_dir: '/w',
      session_type: 'user',
    });
    vi.stubGlobal('fetch', daemon.fetchMock);

    const { unmount } = render(
      <StrictMode>
        <ChatOpen sessionId={chat} />
      </StrictMode>
    );
    await waitFor(() => expect(screen.getByTestId('tier-probe')).toHaveTextContent('private'));
    await settle();

    expect(daemon.rowReads().map(describeRead)).toEqual([`/sessions/${chat}?metadata_only=true`]);
    // ⚠ A read of a private chat without the proof is answered with nothing, and
    // the interface then treats the chat as gone.
    expect(daemon.rowReads()[0].proof).toBe('proof-of-person');
    expect(screen.getByTestId('subagent-probe')).toHaveTextContent('ordinary');
    unmount();
  });

  it('browser: ONE metadata read, though the composer mounts only after the chat has loaded', async () => {
    // The tester's D1. The composer is withheld until `/agent/resume` answers,
    // so a reader that asks at mount cannot share the composer's request — not
    // within the coalescer's hold, and not by any rule that keeps a read fresh.
    expect(RESUME_MS).toBeGreaterThan(SESSION_READ_HOLD_MS * 3);
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    const chat = nextChat();
    const daemon = fakeDaemon({
      id: chat,
      privacy_tier: 'private',
      working_dir: '/w',
      session_type: 'user',
    });
    vi.stubGlobal('fetch', daemon.fetchMock);

    const { unmount } = render(<ChatOpen sessionId={chat} />);
    // Withheld while the answer is in flight — the arrangement that separates
    // the two readers in time.
    expect(screen.queryByTestId('tier-probe')).toBeNull();
    await waitFor(() => expect(screen.getByTestId('tier-probe')).toHaveTextContent('private'));
    await settle();

    const reads = daemon.rowReads();
    expect(reads.map(describeRead)).toEqual([`/sessions/${chat}?metadata_only=true`]);
    // A browser states the host's model instead of a proof it cannot hold (SD-12).
    expect(reads[0].callerProvider).toBe('versa_azure');
    unmount();
  });

  it("desktop, a subagent's chat: the header comes from the resume, and no transcript is read", async () => {
    const chat = nextChat();
    const daemon = fakeDaemon({
      id: chat,
      privacy_tier: 'public',
      working_dir: '/w',
      session_type: 'sub_agent',
      parent_session_id: '20260913_0',
    });
    vi.stubGlobal('fetch', daemon.fetchMock);

    const { unmount } = render(<ChatOpen sessionId={chat} />);
    await waitFor(() =>
      expect(screen.getByTestId('subagent-probe')).toHaveTextContent(
        'subagent of 20260913_0 with developer'
      )
    );
    await settle();

    expect(daemon.rowReads().map(describeRead)).toEqual([`/sessions/${chat}?metadata_only=true`]);
    const grants = daemon.sent.filter((s) => s.url.pathname === `/sessions/${chat}/extensions`);
    expect(grants).toHaveLength(1);
    expect(grants[0].proof).toBe('proof-of-person');
    unmount();
  });

  it("browser, a subagent's chat: the store's one full read is the header's too", async () => {
    // `/agent/resume` is refused for a subagent's chat on a keyless daemon, and
    // the store reads the chat itself to paint it read-only
    // (`loadReadOnlySubagentChat`). The composer never mounts there.
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    const chat = nextChat();
    const daemon = fakeDaemon(
      {
        id: chat,
        privacy_tier: 'public',
        working_dir: '/w',
        session_type: 'sub_agent',
        parent_session_id: '20260913_0',
      },
      { resumeRefused: true }
    );
    vi.stubGlobal('fetch', daemon.fetchMock);

    const { unmount } = render(<ChatOpen sessionId={chat} />);
    await waitFor(() =>
      expect(screen.getByTestId('subagent-probe')).toHaveTextContent(
        'subagent of 20260913_0 with developer'
      )
    );
    await settle();

    expect(daemon.rowReads().map(describeRead)).toEqual([`/sessions/${chat}`]);
    expect(daemon.rowReads()[0].callerProvider).toBe('versa_azure');
    expect(screen.getByTestId('subagent-read-only-note')).toBeInTheDocument();
    defaultChatStreamRegistry.peekController(chat)?.releaseOwnership();
    unmount();
  });
});
