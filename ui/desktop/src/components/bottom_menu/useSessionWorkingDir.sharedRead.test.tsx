import { render, waitFor } from '@testing-library/react';
import { useLayoutEffect } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Session } from '../../api';
import { useLoadingChatRow } from '../summary/useLoadingChatRow';
import { useSessionWorkingDir } from './useSessionWorkingDir';

/**
 * A chat's first open after a reload (vision V2-R5-6 round 7, V2-R7-3).
 *
 * BaseChat holds the composer bar at opacity 0 until the chat's row answers
 * (`summary/useLoadingChat`), and the composer, mounted in that bar, names the
 * chat's folder from the same row (`useSessionWorkingDir`). The bar appearing and
 * the folder appearing must be ONE step: a bar that arrives with its folder line
 * empty and fills it a round trip later is the blink the hold exists to prevent.
 *
 * That holds only while the two readers are one request. The renderer sends a
 * chat's identical row reads issued in the same moment as a single request
 * (`utils/sessionReadCoalescing`), and "identical" is method, URL and every
 * header. So these cases pin the two things the hand-off depends on: the reads
 * carry the same path, query and headers, and the two answers reach the screen
 * in the same commit when the daemon answers them together.
 *
 * jsdom has no network; `getSession` hands both readers the one pending answer,
 * which is what the coalesced response is to each of them.
 */
const mocks = vi.hoisted(() => ({
  getSession: vi.fn(),
  headers: vi.fn(),
  rows: null as Session[] | null,
}));
vi.mock('../../api', () => ({ getSession: mocks.getSession }));
vi.mock('../../utils/userAction', () => ({ userActionHeaders: mocks.headers }));
vi.mock('../../utils/sessionListCache', () => ({
  getCachedSessionList: () => mocks.rows,
  subscribeSessionList: () => () => {},
}));
vi.mock('../../utils/workingDir', () => ({ getInitialWorkingDir: () => '/home' }));

const row = (over: Partial<Session> = {}) =>
  ({
    id: 'a',
    name: 'chat a',
    message_count: 12,
    working_dir: '/Users/x/kdps',
    ...over,
  }) as Session;

/** What the screen holds after each commit: the bar's hold and the folder line. */
type Frame = { bar: 'held' | 'shown'; folder: string | null };

function Bar({ id, frames }: { id: string; frames: Frame[] }) {
  // What BaseChat does with the row: no row, the composer bar is held.
  const loadingRow = useLoadingChatRow(id, true);
  const { workingDir } = useSessionWorkingDir(id);
  useLayoutEffect(() => {
    frames.push({ bar: loadingRow ? 'shown' : 'held', folder: workingDir });
  });
  return null;
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

beforeEach(() => {
  mocks.rows = null;
  mocks.getSession.mockReset();
  mocks.headers.mockReset().mockResolvedValue({ 'X-User-Action': 'synthetic-proof' });
});

describe('the loading chat and the composer read the same row', () => {
  it('ask for it in the same way, so the renderer sends them as one request', async () => {
    mocks.getSession.mockReturnValue(new Promise(() => {}));
    render(<Bar id="a" frames={[]} />);
    await waitFor(() => expect(mocks.getSession).toHaveBeenCalledTimes(2));

    const [first, second] = mocks.getSession.mock.calls.map(([options]) => options);
    // The coalescing key is the method, the URL and every header; `signal` is
    // the one thing the loading chat's read adds, and it is not part of it.
    expect(first.path).toEqual({ session_id: 'a' });
    expect(second.path).toEqual(first.path);
    expect(second.query).toEqual(first.query);
    expect(second.query).toEqual({ metadata_only: true });
    expect(second.headers).toEqual(first.headers);
    expect(second.headers).toEqual({ 'X-User-Action': 'synthetic-proof' });
  });

  it('put the bar and its folder on screen in the same commit when the row answers', async () => {
    const answer = deferred<{ data: Session }>();
    mocks.getSession.mockReturnValue(answer.promise);
    const frames: Frame[] = [];
    render(<Bar id="a" frames={frames} />);
    await waitFor(() => expect(mocks.getSession).toHaveBeenCalledTimes(2));

    // Held, folder not known: nothing for the person to see yet.
    expect(frames[frames.length - 1]).toEqual({ bar: 'held', folder: null });

    answer.resolve({ data: row() });
    await waitFor(() =>
      expect(frames[frames.length - 1]).toEqual({ bar: 'shown', folder: '/Users/x/kdps' })
    );

    // Never the bar without its folder, and never a folder in a held bar.
    expect(frames.some((f) => f.bar === 'shown' && f.folder === null)).toBe(false);
    expect(frames.some((f) => f.bar === 'held' && f.folder !== null)).toBe(false);
    // And never the app's default folder, in any commit.
    expect(frames.some((f) => f.folder === '/home')).toBe(false);
  });

  it('are already together in the first frame when the listing holds the chat', () => {
    mocks.rows = [row()];
    mocks.getSession.mockReturnValue(new Promise(() => {}));
    const frames: Frame[] = [];
    render(<Bar id="a" frames={frames} />);

    expect(frames[0]).toEqual({ bar: 'shown', folder: '/Users/x/kdps' });
    expect(frames.every((f) => f.bar === 'shown' && f.folder === '/Users/x/kdps')).toBe(true);
  });
});
