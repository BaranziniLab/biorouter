import React, { StrictMode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor, act } from '@testing-library/react';

/**
 * Item 10 (1.90.4): opening a chat sent the SAME full `GET /sessions/{id}` about
 * ten times — measured with CDP in the running app on 2026-09-13 as ten identical
 * requests inside two milliseconds on a plain reload into a chat.
 *
 * This spec drives the real readers through the real generated client and the
 * transport `renderer.tsx` installs (`daemonClientConfig`), with only the network
 * faked, and counts what reaches it:
 *
 *   - `ChatInput` reads the row twice per mount (working directory, privacy tier);
 *   - `useSubagentSession` reads it once per mount;
 *   - React's StrictMode doubles every mount in development, and `BaseChat`
 *     remounts the composer when the transcript replaces the empty-chat layout.
 *
 * That is 2 x 2 x 2 + 1 x 2 = 10 reads issued, which is what the app sent. The
 * assertion is on what was SENT: one request, for the row only, carrying the
 * proof of a person.
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
import { useSubagentSession } from './subagent/useSubagentSession';

const DAEMON = 'http://127.0.0.1:4711';
const CHAT = '20260913_1';

type Sent = { url: URL; proof: string | null };

function fakeDaemon(row: Record<string, unknown>) {
  const sent: Sent[] = [];
  const fetchMock = vi.fn(async (input: Parameters<typeof fetch>[0], init?: RequestInit) => {
    const request = input instanceof Request ? input : new Request(input, init);
    const url = new URL(request.url);
    sent.push({ url, proof: request.headers.get('X-User-Action') });
    if (url.pathname === `/sessions/${CHAT}`) {
      const metadataOnly = url.searchParams.get('metadata_only') === 'true';
      return new Response(
        JSON.stringify({
          ...row,
          id: CHAT,
          conversation: metadataOnly ? null : [{ role: 'user', created: 1, content: [] }],
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } }
      );
    }
    return new Response('{}', { status: 200, headers: { 'Content-Type': 'application/json' } });
  });
  return { sent, fetchMock };
}

const sessionReads = (sent: Sent[]) => sent.filter((s) => s.url.pathname === `/sessions/${CHAT}`);

function SubagentHeaderProbe({ sessionId }: { sessionId: string }) {
  const info = useSubagentSession(sessionId);
  return <div data-testid="subagent-probe">{info.isSubagent ? 'subagent' : 'ordinary'}</div>;
}

function Composer() {
  return (
    <ChatInput
      sessionId={CHAT}
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

/** `BaseChat`'s two composer slots: the empty-chat layout, then the transcript's. */
function ChatOpen({ transcript }: { transcript: boolean }) {
  return (
    <div>
      <SubagentHeaderProbe sessionId={CHAT} />
      {transcript ? (
        <section data-layout="transcript">
          <Composer />
        </section>
      ) : (
        <div data-layout="clean">
          <Composer />
        </div>
      )}
    </div>
  );
}

beforeEach(() => {
  Object.assign(window, {
    appConfig: { get: () => '/default/workdir' },
    electron: {
      directoryChooser: vi.fn(),
      addRecentDir: vi.fn(),
      logInfo: vi.fn(),
      getPathForFile: vi.fn(() => ''),
      on: vi.fn(),
      off: vi.fn(),
      getUserActionKey: vi.fn(async () => 'proof-of-person'),
    },
  });
  client.setConfig(daemonClientConfig(DAEMON, 'daemon-secret'));
});

afterEach(() => {
  client.setConfig({ baseUrl: 'http://localhost', headers: {}, fetch: undefined });
  vi.unstubAllGlobals();
});

describe('opening a chat reads its row once', () => {
  it('sends ONE metadata read, with the proof, where ten reads are issued', async () => {
    const daemon = fakeDaemon({ privacy_tier: 'private', working_dir: '/w', session_type: 'user' });
    vi.stubGlobal('fetch', daemon.fetchMock);

    const { rerender } = render(
      <StrictMode>
        <ChatOpen transcript={false} />
      </StrictMode>
    );
    // The transcript lands and the composer moves to its other slot — a remount.
    rerender(
      <StrictMode>
        <ChatOpen transcript />
      </StrictMode>
    );

    await waitFor(() => expect(screen.getByTestId('tier-probe')).toHaveTextContent('private'));
    // Let any straggler that would have gone out separately do so.
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 100));
    });

    const reads = sessionReads(daemon.sent);
    expect(reads.map((r) => `${r.url.pathname}${r.url.search}`)).toEqual([
      `/sessions/${CHAT}?metadata_only=true`,
    ]);
    // ⚠ A read of a private chat without the proof is answered with nothing, and
    // the interface then treats the chat as gone.
    expect(reads[0].proof).toBe('proof-of-person');
    expect(screen.getByTestId('subagent-probe')).toHaveTextContent('ordinary');
  });

  it('reads the transcript only for a subagent`s chat, which needs its spawn record', async () => {
    const daemon = fakeDaemon({
      privacy_tier: 'public',
      working_dir: '/w',
      session_type: 'sub_agent',
      parent_session_id: '20260913_0',
    });
    vi.stubGlobal('fetch', daemon.fetchMock);

    render(<ChatOpen transcript />);

    await waitFor(() => expect(screen.getByTestId('subagent-probe')).toHaveTextContent('subagent'));
    const reads = sessionReads(daemon.sent).map((r) => `${r.url.pathname}${r.url.search}`);
    expect(reads).toEqual([`/sessions/${CHAT}?metadata_only=true`, `/sessions/${CHAT}`]);
    expect(sessionReads(daemon.sent).every((r) => r.proof === 'proof-of-person')).toBe(true);
  });
});
