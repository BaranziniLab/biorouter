import { describe, it, expect, vi, beforeEach, beforeAll, afterAll } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

// #44 — the empty-chat-only working-directory rule at the chip level. While
// the chat is completely empty the chip is the chooser (both the pre-session
// #39 path and a fresh zero-message session); once the chat has messages it
// is a read-only label — basename only, full path on hover — that can never
// reach updateWorkingDir.

vi.mock('../../api', () => ({
  updateWorkingDir: vi.fn(async () => ({ data: {} })),
}));
vi.mock('../../toasts', () => ({
  toastError: vi.fn(),
}));
// Issue #56 Task 58 / #47: `/agent/update_working_dir` names a chat, and
// repointing a private one needs the proof-of-user. The assertion below names
// the header rather than tolerating whatever is there.
vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-key' }),
}));

import { DirSwitcher, workingDirLabel, deriveWorkingDirLocked } from './DirSwitcher';
import { ChatState } from '../../types/chatState';
import { updateWorkingDir } from '../../api';
import { toastError } from '../../toasts';

const WORKING_DIR = '/Users/wgu/Desktop';
const CHOSEN_DIR = '/Users/wgu/Desktop/data';

beforeAll(() => {
  // Radix tooltip measures its trigger.
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
  );
});

afterAll(() => {
  vi.unstubAllGlobals();
});

beforeEach(() => {
  vi.clearAllMocks();
  Object.assign(window, {
    electron: {
      directoryChooser: vi.fn(async () => ({ canceled: false, filePaths: [CHOSEN_DIR] })),
      addRecentDir: vi.fn(),
      openDirectoryInExplorer: vi.fn(),
    },
  });
});

describe('workingDirLabel', () => {
  it('shows only the basename of a directory', () => {
    expect(workingDirLabel('/Users/wgu/Desktop')).toBe('Desktop');
    expect(workingDirLabel('/Users/wgu/Desktop/')).toBe('Desktop');
    expect(workingDirLabel('C:\\Users\\wgu\\Desktop')).toBe('Desktop');
  });

  it('shows the home directory as its own basename', () => {
    expect(workingDirLabel('/Users/wgu')).toBe('wgu');
  });

  it('shows a rootlike path as-is when there is no basename', () => {
    expect(workingDirLabel('/')).toBe('/');
    expect(workingDirLabel('')).toBe('');
  });

  it('shows a Windows drive root as-is, not as a folder named "C:"', () => {
    expect(workingDirLabel('C:\\')).toBe('C:\\');
    expect(workingDirLabel('C:/')).toBe('C:/');
    expect(workingDirLabel('d:\\\\')).toBe('d:\\\\');
  });
});

/**
 * Spec 3.7: the footer names the FOLDER, never the path, in both states (names,
 * not IDs or paths, principle 10). The path is one hover away, in the tooltip.
 * The name sits in a `<bdi>` so a right-to-left folder name cannot reorder the
 * line around it.
 */
describe('the chip names the folder, inside its bidi isolate', () => {
  it.each([
    ['an ordinary path', '/Users/wgu/Downloads', 'Downloads'],
    ['a path with a trailing slash', '/Users/wgu/Downloads/', 'Downloads'],
    ['a leading-slash root', '/', '/'],
    ['a Windows path', 'C:\\Users\\wgu\\Desktop', 'Desktop'],
    ['a Windows drive root', 'C:\\', 'C:\\'],
  ])('shows %s by its name', (_label, dir, name) => {
    const { container } = render(
      <DirSwitcher sessionId={undefined} workingDir={dir} locked={false} />
    );
    const chip = screen.getByRole('button');
    expect(chip).toHaveTextContent(name);
    expect(chip).toHaveAccessibleName(`Working folder: ${dir}`);
    expect(container.querySelector('bdi')?.textContent).toBe(name);
  });

  it('shows the same name on the locked chip', () => {
    const { container } = render(
      <DirSwitcher sessionId="session-1" workingDir="/Users/wgu/Downloads" locked />
    );
    expect(screen.getByTestId('dir-switcher-locked')).toHaveTextContent('Downloads');
    expect(container.querySelector('bdi')?.textContent).toBe('Downloads');
  });
});

// #44 — the authoritative lock derivation the chip's `locked` prop is fed
// from. The two store-shaped scenarios that motivated it: a resumed transcript
// that has not hydrated yet (messages.length 0 for a non-empty session) must
// be locked from first paint, and a failed optimistic first submit (transcript
// retains the unsent message) must stay unlocked.
describe('deriveWorkingDirLocked', () => {
  const base = {
    sessionId: 'session-1' as string | null | undefined,
    persistedMessageCount: 0 as number | undefined,
    hasAssistantMessage: false,
    chatState: ChatState.Idle,
  };

  it('never locks before a session exists (#39 pre-session chooser)', () => {
    expect(deriveWorkingDirLocked({ ...base, sessionId: null })).toBe(false);
    expect(deriveWorkingDirLocked({ ...base, sessionId: undefined })).toBe(false);
  });

  it('locks a resumed session from first paint until history proves empty', () => {
    // Metadata still loading: assume locked.
    expect(deriveWorkingDirLocked({ ...base, persistedMessageCount: undefined })).toBe(true);
    // Metadata arrived and reports messages, even though the transcript is
    // still hydrating (the store loads it 0 -> N atomically).
    expect(
      deriveWorkingDirLocked({
        ...base,
        persistedMessageCount: 3,
        chatState: ChatState.LoadingConversation,
      })
    ).toBe(true);
    // History proved empty: unlock.
    expect(deriveWorkingDirLocked({ ...base, persistedMessageCount: 0 })).toBe(false);
  });

  it('stays unlocked after a failed first submit whose optimistic message never reached the server', () => {
    // The store keeps the unsent message in the transcript by design (so it
    // goes out when the agent lands); the turn errored back to Idle and the
    // server still reports zero messages -> the dir is still choosable.
    expect(
      deriveWorkingDirLocked({
        ...base,
        persistedMessageCount: 0,
        hasAssistantMessage: false,
        chatState: ChatState.Idle,
      })
    ).toBe(false);
  });

  it('locks while the first turn is in flight and once any assistant reply exists', () => {
    // A turn actively streaming means the first message is reaching the
    // server now, even though the last session fetch said zero.
    expect(deriveWorkingDirLocked({ ...base, chatState: ChatState.Streaming })).toBe(true);
    expect(deriveWorkingDirLocked({ ...base, chatState: ChatState.Thinking })).toBe(true);
    // An assistant reply proves a message reached the server (stale count).
    expect(deriveWorkingDirLocked({ ...base, hasAssistantMessage: true })).toBe(true);
  });
});

describe('DirSwitcher while the chat is empty', () => {
  it('opens the chooser and persists the choice for a zero-message session', async () => {
    const onWorkingDirChange = vi.fn();
    render(
      <DirSwitcher
        sessionId="session-1"
        workingDir={WORKING_DIR}
        locked={false}
        onWorkingDirChange={onWorkingDirChange}
      />
    );

    // Interactive chip: a real button naming the folder; the path is its name.
    const chip = screen.getByRole('button');
    expect(chip).toHaveTextContent('Desktop');
    expect(chip).toHaveAccessibleName(`Working folder: ${WORKING_DIR}`);
    fireEvent.click(chip);

    // throwOnError is required: without it the generated client resolves with
    // an error object on a 409 and the failure path would never run.
    await waitFor(() =>
      expect(updateWorkingDir).toHaveBeenCalledWith({
        headers: { 'X-User-Action': 'test-key' },
        body: { session_id: 'session-1', working_dir: CHOSEN_DIR },
        throwOnError: true,
      })
    );
    // The displayed dir and the recents list update only after success.
    expect(onWorkingDirChange).toHaveBeenCalledWith(CHOSEN_DIR);
    expect(window.electron.addRecentDir).toHaveBeenCalledWith(CHOSEN_DIR);
  });

  it('leaves the displayed dir and recents untouched when the server refuses (409)', async () => {
    vi.mocked(updateWorkingDir).mockRejectedValueOnce({
      message: 'the working directory is fixed once a chat has messages',
    });
    const onWorkingDirChange = vi.fn();
    const onRestartEnd = vi.fn();
    render(
      <DirSwitcher
        sessionId="session-1"
        workingDir={WORKING_DIR}
        locked={false}
        onWorkingDirChange={onWorkingDirChange}
        onRestartEnd={onRestartEnd}
      />
    );

    fireEvent.click(screen.getByRole('button'));

    await waitFor(() => expect(updateWorkingDir).toHaveBeenCalled());
    await waitFor(() => expect(onRestartEnd).toHaveBeenCalled());

    // Refused server-side: nothing may change client-side.
    expect(onWorkingDirChange).not.toHaveBeenCalled();
    expect(window.electron.addRecentDir).not.toHaveBeenCalled();
    expect(toastError).toHaveBeenCalledWith({
      title: 'Working directory update failed',
      msg: 'the working directory is fixed once a chat has messages',
    });
  });

  it('keeps the #39 pre-session path: forwards the choice without persisting', async () => {
    const onWorkingDirChange = vi.fn();
    render(
      <DirSwitcher
        sessionId={undefined}
        workingDir={WORKING_DIR}
        onWorkingDirChange={onWorkingDirChange}
      />
    );

    fireEvent.click(screen.getByRole('button'));

    await waitFor(() => expect(onWorkingDirChange).toHaveBeenCalledWith(CHOSEN_DIR));
    expect(updateWorkingDir).not.toHaveBeenCalled();
  });
});

describe('DirSwitcher once the chat has messages (locked, #44)', () => {
  it('renders a read-only basename label with the full path on hover', async () => {
    const user = userEvent.setup();
    render(<DirSwitcher sessionId="session-1" workingDir={WORKING_DIR} locked={true} />);

    // Basename only, and no interactive affordance at all.
    const label = screen.getByTestId('dir-switcher-locked');
    expect(label).toHaveTextContent('Desktop');
    expect(label).not.toHaveTextContent(WORKING_DIR);
    expect(screen.queryByRole('button')).not.toBeInTheDocument();

    // The full path is still discoverable on hover.
    await user.hover(label);
    expect(await screen.findByRole('tooltip')).toHaveTextContent(WORKING_DIR);
  });

  it('never opens the chooser or calls updateWorkingDir', async () => {
    const onWorkingDirChange = vi.fn();
    render(
      <DirSwitcher
        sessionId="session-1"
        workingDir={WORKING_DIR}
        locked={true}
        onWorkingDirChange={onWorkingDirChange}
      />
    );

    fireEvent.click(screen.getByTestId('dir-switcher-locked'));

    // Nothing may happen — no chooser, no persistence, no local change.
    await waitFor(() => expect(window.electron.directoryChooser).not.toHaveBeenCalled());
    expect(updateWorkingDir).not.toHaveBeenCalled();
    expect(onWorkingDirChange).not.toHaveBeenCalled();
  });
});

// #50 — the chip's Tooltip must keep ONE control mode for its whole life.
// Both branches render `<TooltipProvider><Tooltip>` in the same position, so
// React reconciles them into a single Tooltip instance; when the working
// directory locks after the first message the instance would flip from
// controlled (`open`/`onOpenChange`) to uncontrolled. Radix warns about it,
// and the state it warns about is the one that later yields a stuck-open or
// unresponsive tooltip.
describe('DirSwitcher tooltip control mode (#50)', () => {
  const CONTROL_MODE_WARNING = /changing from (controlled|uncontrolled) to/;

  it('does not flip the tooltip between controlled and uncontrolled when the dir locks', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});

    const { rerender } = render(
      <DirSwitcher sessionId="session-1" workingDir={WORKING_DIR} locked={false} />
    );
    // The chat gets its first message: the same chip becomes read-only.
    rerender(<DirSwitcher sessionId="session-1" workingDir={WORKING_DIR} locked={true} />);
    // ...and a later unlock (new empty session in the same chip) must not flip back.
    rerender(<DirSwitcher sessionId="session-2" workingDir={WORKING_DIR} locked={false} />);

    await waitFor(() => expect(screen.getByRole('button')).toBeInTheDocument());

    const controlModeWarnings = warn.mock.calls.filter((call) =>
      call.some((arg) => typeof arg === 'string' && CONTROL_MODE_WARNING.test(arg))
    );
    expect(controlModeWarnings).toEqual([]);

    warn.mockRestore();
  });

  it('still reveals the full path on hover once locked', async () => {
    const user = userEvent.setup();
    const { rerender } = render(
      <DirSwitcher sessionId="session-1" workingDir={WORKING_DIR} locked={false} />
    );
    rerender(<DirSwitcher sessionId="session-1" workingDir={WORKING_DIR} locked={true} />);

    await user.hover(screen.getByTestId('dir-switcher-locked'));
    expect(await screen.findByRole('tooltip')).toHaveTextContent(WORKING_DIR);
  });
});

/// The footer is sans (principle 3: mono is for machine strings). The PATH is a
/// machine string, so the tooltip that shows it is mono, as it is in the
/// sidebar and in History. jsdom never runs Tailwind, so these assert classes.
describe('DirSwitcher — the folder name is sans, the path in its tooltip is mono', () => {
  it('sets neither chip in monospace', () => {
    const { rerender } = render(
      <DirSwitcher sessionId="session-1" workingDir={WORKING_DIR} locked={false} />
    );
    expect(screen.getByText('Desktop').closest('.font-mono')).toBeNull();
    rerender(<DirSwitcher sessionId="session-1" workingDir={WORKING_DIR} locked={true} />);
    expect(screen.getByText('Desktop').closest('.font-mono')).toBeNull();
  });

  it('sets the hover tooltip in monospace, overriding the tooltip font-sans', async () => {
    const user = userEvent.setup();
    render(<DirSwitcher sessionId="session-1" workingDir={WORKING_DIR} locked={true} />);

    await user.hover(screen.getByTestId('dir-switcher-locked'));
    const tooltip = await screen.findByRole('tooltip');

    // `ui/Tooltip.tsx` puts `font-sans` on the content box itself, so the
    // override must be on a descendant — asserting on the box would pass while
    // the path still rendered in the body font.
    const path = [...tooltip.querySelectorAll('*')].find(
      (node) => node.textContent === WORKING_DIR
    );
    expect(path).toBeDefined();
    expect(path!.className).toMatch(/font-mono/);
  });
});
