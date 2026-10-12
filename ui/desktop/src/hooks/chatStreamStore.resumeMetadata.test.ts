import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ChatStreamRegistry } from './chatStreamStore';
import type { Message, Session } from '../api';
import { resumeAgent, updateFromSession } from '../api';

/**
 * Ruling 28 (V2-R9-3): the agent-loading resume answers without the transcript.
 *
 * Opening a chat sends `POST /agent/resume` twice, on purpose: the paint resume
 * (`load_model_and_extensions: false`) carries the transcript, and
 * `ensureAgentLoaded`'s resume (`true`) starts the agent. The daemon used to
 * answer the second one with the whole transcript as well, which on a large
 * chat was megabytes nobody read. It now answers with the chat's row and
 * `conversation: null` (`read_resume_session` in `routes/agent.rs`).
 *
 * That is safe only while nothing in the store reads `session.conversation`
 * from that second answer. These tests hand the store a row that records every
 * property read from it and assert `conversation` is never one of them, on the
 * cold open and on the two paths where the agent load is the only resume (a
 * transcript served from the LRU, a controller reused with its transcript up).
 * A spread, a cache write or a snapshot of that row would read it and fail here.
 */
vi.mock('../api', () => ({
  cancelTurn: vi.fn(),
  editMessage: vi.fn(),
  getSession: vi.fn(async () => ({ data: null })),
  interrupt: vi.fn(),
  listSessions: vi.fn(async () => ({ data: { sessions: [] } })),
  reply: vi.fn(),
  resumeAgent: vi.fn(),
  updateFromSession: vi.fn(async () => ({ data: {} })),
  updateSessionUserWorkflowValues: vi.fn(async () => ({ data: {} })),
}));

vi.mock('../utils/continuationLease', () => ({
  abandonContinuationLease: vi.fn(async () => undefined),
  getContinuationOwnerId: vi.fn(() => 'test-window-owner'),
  recoverContinuationGroup: vi.fn(),
}));

vi.mock('../utils/userAction', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../utils/userAction')>()),
  userActionHeaders: async () => ({ 'X-User-Action': 'test-key' }),
}));

function message(id: string, text: string): Message {
  return {
    id,
    role: 'user',
    created: 1,
    content: [{ type: 'text', text }],
    metadata: { userVisible: true, agentVisible: true },
  };
}

/** What the paint resume answers with: the row and its transcript. */
function paintedSession(id: string, conversation: Message[]): Session {
  return {
    id,
    name: `Session ${id}`,
    working_dir: '/tmp',
    conversation,
    message_count: conversation.length,
    total_tokens: 0,
    created_at: '',
    updated_at: '',
    extension_data: {},
    user_set_name: false,
  } as Session;
}

/**
 * What the agent load answers with now: the same row, `conversation: null`,
 * behind a proxy that records each property the store reads from it.
 */
function metadataRow(id: string, messageCount: number, reads: string[]): Session {
  const row = { ...paintedSession(id, []), conversation: null, message_count: messageCount };
  return new Proxy(row, {
    get(target, key, receiver) {
      if (typeof key === 'string') reads.push(key);
      return Reflect.get(target, key, receiver);
    },
  }) as Session;
}

type ResumeBody = { body: { load_model_and_extensions: boolean } };

function answerResumes(id: string, transcript: Message[], reads: string[]) {
  vi.mocked(resumeAgent).mockImplementation(async (options: unknown) => {
    const { load_model_and_extensions } = (options as ResumeBody).body;
    if (!load_model_and_extensions) {
      return { data: { session: paintedSession(id, transcript), initializing: false } } as never;
    }
    return {
      data: {
        session: metadataRow(id, transcript.length, reads),
        initializing: false,
        extension_results: [],
      },
    } as never;
  });
}

function agentLoads(): number {
  return vi
    .mocked(resumeAgent)
    .mock.calls.filter(([options]) => (options as ResumeBody).body.load_model_and_extensions)
    .length;
}

beforeEach(() => {
  vi.mocked(resumeAgent).mockReset();
  vi.mocked(updateFromSession).mockClear();
  Object.assign(window, {
    electron: {
      openExternal: vi.fn(async () => undefined),
      showNotification: vi.fn(),
      logInfo: vi.fn(),
    },
  });
});

describe('the agent-loading resume answers without the transcript', () => {
  it('opens a chat without reading a transcript from the agent load', async () => {
    const transcript = [message('m1', 'first'), message('m2', 'second')];
    const reads: string[] = [];
    answerResumes('rm-cold', transcript, reads);

    const controller = new ChatStreamRegistry().getController('rm-cold');
    await controller.loadSession();
    await vi.waitFor(() => expect(controller.getSnapshot().agentReady).toBe(true));

    expect(agentLoads()).toBe(1);
    // The instrument works: the store did read the row it was handed.
    expect(reads).toContain('id');
    expect(reads).not.toContain('conversation');
    // The model was bound from the row's id, as before.
    expect(updateFromSession).toHaveBeenCalledWith(
      expect.objectContaining({ body: { session_id: 'rm-cold' } })
    );
    // The transcript the paint resume carried is what the chat shows, and the
    // row it shows is the paint resume's, not the agent load's.
    const snapshot = controller.getSnapshot();
    expect(snapshot.messages.map((m) => m.id)).toEqual(['m1', 'm2']);
    expect(snapshot.session?.conversation).toHaveLength(2);
    expect(snapshot.turnError).toBeUndefined();
  });

  it('keeps a transcript served from the cache when the agent load is the only resume', async () => {
    const transcript = [message('c1', 'cached one'), message('c2', 'cached two')];
    const firstReads: string[] = [];
    answerResumes('rm-cached', transcript, firstReads);
    const first = new ChatStreamRegistry().getController('rm-cached');
    await first.loadSession();
    await vi.waitFor(() => expect(first.getSnapshot().agentReady).toBe(true));

    vi.mocked(resumeAgent).mockClear();
    const reads: string[] = [];
    answerResumes('rm-cached', transcript, reads);
    const fresh = new ChatStreamRegistry().getController('rm-cached');
    await fresh.loadSession();
    await vi.waitFor(() => expect(fresh.getSnapshot().agentReady).toBe(true));

    // The LRU painted it, so the agent load was the only resume sent.
    expect(vi.mocked(resumeAgent).mock.calls).toHaveLength(1);
    expect(agentLoads()).toBe(1);
    expect(reads).toContain('id');
    expect(reads).not.toContain('conversation');
    expect(firstReads).not.toContain('conversation');
    expect(fresh.getSnapshot().messages.map((m) => m.id)).toEqual(['c1', 'c2']);
  });

  it("keeps a queued child's painted transcript while its reused controller loads the agent", async () => {
    // A delegated child still queued: the daemon answers both resumes with
    // `initializing: true`, the agent load with the row alone. Once the child
    // is ready, the reused controller (transcript up, agent not loaded) runs
    // the agent load again, and that is the only resume it sends.
    const transcript = [message('q1', 'delegated task')];
    const reads: string[] = [];
    let childReady = false;
    const childRow = (row: Session): Session => {
      (row as { session_type?: string }).session_type = 'sub_agent';
      return row;
    };
    vi.mocked(resumeAgent).mockImplementation(async (options: unknown) => {
      const { load_model_and_extensions } = (options as ResumeBody).body;
      if (!load_model_and_extensions) {
        return {
          data: {
            session: childRow(paintedSession('rm-queued', transcript)),
            initializing: !childReady,
          },
        } as never;
      }
      return {
        data: {
          session: childRow(metadataRow('rm-queued', transcript.length, reads)),
          initializing: !childReady,
          extension_results: childReady ? [] : null,
        },
      } as never;
    });

    const controller = new ChatStreamRegistry().getController('rm-queued');
    await controller.loadSession();
    await vi.waitFor(() => expect(agentLoads()).toBe(1));
    expect(controller.getSnapshot().agentReady).toBe(false);

    childReady = true;
    vi.mocked(resumeAgent).mockClear();
    await controller.loadSession();
    await vi.waitFor(() => expect(controller.getSnapshot().agentReady).toBe(true));

    expect(vi.mocked(resumeAgent).mock.calls).toHaveLength(1);
    expect(agentLoads()).toBe(1);
    expect(reads).toContain('id');
    expect(reads).not.toContain('conversation');
    expect(controller.getSnapshot().messages.map((m) => m.id)).toEqual(['q1']);
  });
});
