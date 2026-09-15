import { act, render } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Session } from '../../api';

/**
 * A tab that is NOT active must still learn its chat's real name.
 *
 * # The bug
 *
 * Found by driving the app on 2026-09-12. The daemon renames a chat after each
 * of its first few turns, with no signal (`SessionManager::maybe_update_name`
 * runs after the reply stream has closed). A tab title is the one name the
 * renderer PERSISTS, and only two things could ever correct one: the name
 * channel, and `handleSessionLoaded` — which comes from BaseChat, and only the
 * active tab mounts a BaseChat. So the sidebar read "Instruction-following
 * tests" while the same chat's background tab read "Penguin prompt test", on one
 * screen, unchanged across three reloads; making that tab active fixed it
 * instantly.
 *
 * `renameTab` already mirrored into every tab of that session — the reducer was
 * never the problem. What was missing is any path from the server's row to a tab
 * nobody is looking at. This pins that path: the shell reconciles its titles
 * against the session list it ALREADY reads and warms for the privacy dots.
 *
 * # Why it asserts dispatches, not DOM
 *
 * The strip reaches the DOM through BaseChat's `renderSessionTitle` render prop
 * and BaseChat is unmountable here (~10 providers), exactly as
 * `ChatGroupsShell.sessionName.test.tsx` and `.privacy.test.tsx` already
 * document. The dispatch IS the wiring that was absent.
 */

const dispatch = vi.fn();
let cachedList: Session[] | null = null;
// Every subscriber, as the real cache keeps them: the shell has more than one
// (the privacy tiers read the same list), and a single slot would hand an emit
// to whichever subscribed last.
const listeners = new Set<() => void>();
let emit: (() => void) | null = null;

vi.mock('../BaseChat', () => ({
  default: () => <div data-testid="basechat" />,
}));

vi.mock('../../utils/sessionListCache', () => ({
  getCachedSessionList: () => cachedList,
  subscribeSessionList: (listener: () => void) => {
    listeners.add(listener);
    emit = () => listeners.forEach((l) => l());
    return () => {
      listeners.delete(listener);
    };
  },
  preloadSessionList: () => {},
}));

vi.mock('../../hooks/chatStreamStore', () => ({
  useLiveSessionTiers: () => ({}),
  useLiveSessionTypes: () => ({}),
}));

/**
 * The single-row read, `GET /sessions/{id}`. Every call is recorded and left
 * pending, so a test decides when each read lands and what it says.
 */
type ReadCall = {
  options: { path: { session_id: string }; query?: unknown; headers?: unknown };
  resolve: (row: Partial<Session>) => void;
  /**
   * The daemon's refusal as the client hands it over: the read passes no
   * `throwOnError`, so a 403 resolves with the status rather than throwing.
   */
  refuse: () => void;
  reject: (error: unknown) => void;
};
let reads: ReadCall[] = [];
vi.mock('../../api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../api')>()),
  getSession: (options: ReadCall['options']) =>
    new Promise((resolve, reject) => {
      reads.push({
        options,
        resolve: (row) => resolve({ data: row }),
        refuse: () =>
          resolve({
            data: undefined,
            error: 'That chat is private, or there is no chat with that id.',
            response: { status: 403 },
          }),
        reject,
      });
    }),
}));

/** The proof-of-user this window attaches. Every read must carry exactly this. */
const PROOF = { 'X-User-Action': 'proof-of-user' };
vi.mock('../../utils/userAction', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../utils/userAction')>()),
  userActionHeaders: async () => PROOF,
}));

/**
 * `t-active` is the group's `activeTabId`; `t-background` and `t-mine` are not.
 * The background tab is the whole point — on `main` nothing could reach it.
 */
let tabs = [
  {
    tabId: 't-active',
    sessionId: 'sess-active',
    title: 'DELTA instruction test',
    userSetName: false,
  },
  { tabId: 't-background', sessionId: 'sess-bg', title: 'Penguin prompt test', userSetName: false },
];

vi.mock('../../contexts/ChatGroupsContext', () => ({
  useChatGroups: () => ({
    dispatch,
    state: {
      activeGroupId: 'g1',
      layout: { kind: 'leaf', groupId: 'g1' },
      groups: { g1: { id: 'g1', activeTabId: 't-active', tabs } },
    },
  }),
}));

vi.mock('../ui/sidebar', () => ({ useSidebar: () => ({ state: 'expanded', isMobile: false }) }));

import ChatGroupsShell from './ChatGroupsShell';

function row(id: string, name: string, userSetName = false): Session {
  return {
    id,
    name,
    user_set_name: userSetName,
    working_dir: '/tmp',
    message_count: 2,
    total_tokens: 0,
    created_at: '',
    updated_at: '',
    extension_data: {},
  } as unknown as Session;
}

function renameDispatches() {
  return dispatch.mock.calls.map((call) => call[0]).filter((a) => a?.type === 'renameTab');
}

describe('ChatGroupsShell — tab titles are reconciled against the session list', () => {
  beforeEach(() => {
    dispatch.mockClear();
    cachedList = null;
    emit = null;
    reads = [];
    tabs = [
      {
        tabId: 't-active',
        sessionId: 'sess-active',
        title: 'DELTA instruction test',
        userSetName: false,
      },
      {
        tabId: 't-background',
        sessionId: 'sess-bg',
        title: 'Penguin prompt test',
        userSetName: false,
      },
    ];
  });

  it('renames a tab that is not the active one when the row says so', () => {
    cachedList = [
      row('sess-active', 'DELTA instruction test'),
      row('sess-bg', 'Instruction-following tests'),
    ];

    render(<ChatGroupsShell onChatChange={() => {}} />);

    expect(renameDispatches()).toEqual([
      {
        type: 'renameTab',
        sessionId: 'sess-bg',
        title: 'Instruction-following tests',
        userSetName: false,
      },
    ]);
  });

  it('reconciles when the list arrives after the mount, not only on it', () => {
    // The cold-start order: the strip warms the cache, so the first read is
    // empty and the correction has to ride the subscription.
    cachedList = null;
    render(<ChatGroupsShell onChatChange={() => {}} />);
    expect(renameDispatches()).toEqual([]);

    cachedList = [row('sess-bg', 'Instruction-following tests')];
    emit?.();

    expect(renameDispatches()).toEqual([
      {
        type: 'renameTab',
        sessionId: 'sess-bg',
        title: 'Instruction-following tests',
        userSetName: false,
      },
    ]);
  });

  /**
   * Guard 1 — the case the old code was protecting. A user rename sets
   * `userSetName` on the tab optimistically and `user_set_name` on the row, and
   * the daemon never auto-renames such a chat again. A user-named tab is skipped
   * outright, so no late row can snap it back and no keystroke can flicker.
   */
  it('never touches a tab the user named', () => {
    tabs = [{ tabId: 't-mine', sessionId: 'sess-bg', title: 'Penguin notes', userSetName: true }];
    cachedList = [row('sess-bg', 'Instruction-following tests')];

    render(<ChatGroupsShell onChatChange={() => {}} />);

    expect(renameDispatches()).toEqual([]);
  });

  /** Guard 2 — the placeholder is a lower bound on a name, never a correction. */
  it('never downgrades a named tab to the "New chat" placeholder', () => {
    cachedList = [row('sess-bg', 'New chat')];

    render(<ChatGroupsShell onChatChange={() => {}} />);

    expect(renameDispatches()).toEqual([]);
  });

  /** …but a tab still ON the placeholder does adopt the row's real name. */
  it('names a placeholder tab from the row', () => {
    tabs = [{ tabId: 't-new', sessionId: 'sess-bg', title: 'New chat', userSetName: false }];
    cachedList = [row('sess-bg', 'Instruction-following tests')];

    render(<ChatGroupsShell onChatChange={() => {}} />);

    expect(renameDispatches()).toEqual([
      {
        type: 'renameTab',
        sessionId: 'sess-bg',
        title: 'Instruction-following tests',
        userSetName: false,
      },
    ]);
  });

  /** A row that agrees with the tab must not dispatch — the strip re-renders on
   *  every streamed token, and a dispatch per emit would be a render loop. */
  it('dispatches nothing when every title already agrees', () => {
    cachedList = [
      row('sess-active', 'DELTA instruction test'),
      row('sess-bg', 'Penguin prompt test'),
    ];

    render(<ChatGroupsShell onChatChange={() => {}} />);
    emit?.();

    expect(renameDispatches()).toEqual([]);
  });

  /**
   * A chat the list does not carry is asked about on its own (see the suite
   * below). When that read is refused, the tab keeps what it has: silence, not a
   * guess.
   */
  it('leaves a tab alone when the list has no row for it and its own read is refused', async () => {
    cachedList = [row('sess-active', 'DELTA instruction test')];

    render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    for (const read of reads) read.refuse();
    await flush();

    expect(renameDispatches()).toEqual([]);
  });
});

/** Let the proof's async hop, the read and its answer settle. */
async function flush() {
  await act(async () => {
    for (let i = 0; i < 5; i++) await Promise.resolve();
  });
}

function readIds() {
  return reads.map((read) => read.options.path.session_id);
}

/**
 * A delegated subagent's tab read "New chat" forever.
 *
 * # The bug
 *
 * Measured 2026-09-13 on `main` @ 35757426, sandbox `fx-subtab`: a Versa GPT-5.5
 * chat (private) delegated one task, and the daemon opened the child's tab in
 * the background (`announce_open_frame` sends `open_tab` with `focus: false`,
 * and no title). sqlite named the child `Subagent: Reply with exactly the word
 * ECHOSUB and nothing else.`; the tab said `New chat`, and still did after a
 * reload.
 *
 * The reconcile above could never correct it. It reads the shared session list,
 * which is `GET /sessions?include_subagents=false` — 5543 rows on that machine,
 * and the child was not one of them. Leaving `sub_agent` rows out of that list
 * is deliberate (they are not sidebar chats), so the fix is not to put them in.
 *
 * # The fix
 *
 * A tab whose chat the list does not carry is read on its own, with
 * `GET /sessions/{id}?metadata_only=true` — the singular read, behind the same
 * reach gate as every other read of a chat, carrying the user's proof. Measured
 * against that daemon: without the proof the private child answers 403; with
 * it, 200 and its name.
 */
describe('ChatGroupsShell — a tab the session list leaves out is read on its own', () => {
  const CHILD_NAME = 'Subagent: Reply with exactly the word ECHOSUB and nothing else.';
  const childRow = (name = CHILD_NAME): Partial<Session> => ({
    id: 'sess-sub',
    name,
    user_set_name: false,
    session_type: 'sub_agent',
    parent_session_id: 'sess-parent',
    privacy_tier: 'private',
  });

  beforeEach(() => {
    dispatch.mockClear();
    emit = null;
    reads = [];
    tabs = [
      {
        tabId: 't-active',
        sessionId: 'sess-parent',
        title: 'Subagent delegation prompt',
        userSetName: false,
      },
      // The daemon-opened child: a background tab still on the placeholder.
      { tabId: 't-background', sessionId: 'sess-sub', title: 'New chat', userSetName: false },
    ];
    // The list the app really holds: the parent, and never the child.
    cachedList = [row('sess-parent', 'Subagent delegation prompt')];
  });

  it("names a subagent's background tab from its own row", async () => {
    render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();

    expect(readIds()).toEqual(['sess-sub']);
    reads[0].resolve(childRow());
    await flush();

    expect(renameDispatches()).toEqual([
      { type: 'renameTab', sessionId: 'sess-sub', title: CHILD_NAME, userSetName: false },
    ]);
  });

  /**
   * ⚠ A subagent of a PRIVATE chat is private. A read without the proof is
   * refused by the reach gate, and the tab would silently keep the placeholder
   * — the very bug, reintroduced by a missing header.
   */
  it("carries the user's proof, and asks for metadata only", async () => {
    render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();

    expect(reads).toHaveLength(1);
    expect(reads[0].options.headers).toEqual(PROOF);
    expect(reads[0].options.query).toEqual({ metadata_only: true });
  });

  it('never reads a chat the list already carries', async () => {
    cachedList = [row('sess-parent', 'Subagent delegation prompt'), row('sess-sub', CHILD_NAME)];

    render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();

    expect(reads).toEqual([]);
    expect(renameDispatches()).toEqual([
      { type: 'renameTab', sessionId: 'sess-sub', title: CHILD_NAME, userSetName: false },
    ]);
  });

  it('waits for the list before reading anything', async () => {
    cachedList = null;
    render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    expect(reads).toEqual([]);

    cachedList = [row('sess-parent', 'Subagent delegation prompt')];
    emit?.();
    await flush();
    expect(readIds()).toEqual(['sess-sub']);
  });

  /**
   * Rule 1 holds for a read's answer. The tab is still READ — its tier is a fact
   * the strip has no other source for (`ChatGroupsShell.privacy.test.tsx`) —
   * but its name is the user's.
   */
  it('never renames a tab the user named, though it still reads its row', async () => {
    tabs[1] = { ...tabs[1], title: 'My audit child', userSetName: true };

    render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    expect(readIds()).toEqual(['sess-sub']);
    reads[0].resolve(childRow());
    await flush();

    expect(renameDispatches()).toEqual([]);
  });

  it('keeps a real name over a row still reading the placeholder', async () => {
    tabs[1] = { ...tabs[1], title: CHILD_NAME };

    render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    reads[0].resolve(childRow('New chat'));
    await flush();

    expect(renameDispatches()).toEqual([]);
  });

  /**
   * A read answers for the title it was asked about. If the tab's name moved
   * while it was out — a rename on the name channel, the tab loading its own
   * chat — the tab already holds the later fact, and the answer is dropped.
   */
  it('drops an answer for a title the tab no longer has', async () => {
    const { rerender } = render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    expect(reads).toHaveLength(1);

    tabs[1] = { ...tabs[1], title: 'Renamed while the read was out' };
    rerender(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();

    reads[0].resolve(childRow());
    await flush();

    expect(renameDispatches()).toEqual([]);
  });

  it('reads once per list, and follows a rename the next list brings', async () => {
    const { rerender } = render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    reads[0].resolve(childRow());
    await flush();
    expect(renameDispatches()).toHaveLength(1);

    // The reducer applied the rename; the tab's signature moved. Same list — no
    // second read, however often the shell re-renders or the list re-emits.
    tabs[1] = { ...tabs[1], title: CHILD_NAME };
    rerender(<ChatGroupsShell onChatChange={() => {}} />);
    emit?.();
    emit?.();
    await flush();
    expect(reads).toHaveLength(1);

    // The list is fetched again. The child is renamed since (the CLI, another
    // window while this one was shut): the tab follows.
    cachedList = [row('sess-parent', 'Subagent delegation prompt')];
    emit?.();
    await flush();
    expect(reads).toHaveLength(2);
    reads[1].resolve(childRow('Audit of the migration'));
    await flush();

    const renames = renameDispatches();
    expect(renames[renames.length - 1]).toEqual({
      type: 'renameTab',
      sessionId: 'sess-sub',
      title: 'Audit of the migration',
      userSetName: false,
    });
  });

  it('never lets an earlier read land over a later one', async () => {
    render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    cachedList = [row('sess-parent', 'Subagent delegation prompt')];
    emit?.();
    await flush();
    expect(reads).toHaveLength(2);

    reads[1].resolve(childRow('The later name'));
    await flush();
    reads[0].resolve(childRow('The earlier name'));
    await flush();

    expect(renameDispatches()).toEqual([
      { type: 'renameTab', sessionId: 'sess-sub', title: 'The later name', userSetName: false },
    ]);
  });

  it('reads a chat open in two tabs once', async () => {
    tabs.push({ tabId: 't-again', sessionId: 'sess-sub', title: 'New chat', userSetName: false });

    render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();

    expect(readIds()).toEqual(['sess-sub']);
  });
});
