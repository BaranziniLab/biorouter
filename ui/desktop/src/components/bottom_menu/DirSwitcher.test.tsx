import { describe, it, expect, vi, beforeEach, beforeAll, afterAll } from 'vitest';
import { act, render, screen, fireEvent, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

// #44 — the empty-chat-only working-directory rule at the chip level. The
// chip's click pins its card in every state; while the chat is completely
// empty (both the pre-session #39 path and a fresh zero-message session) the
// card's Choose folder… opens the chooser, and once the chat has messages the
// card offers none, so the chip can never reach updateWorkingDir. The card's
// own behaviour is pinned in FolderAccessCard.test.tsx, and when every footer
// card opens and closes in useFooterCard.test.tsx.

vi.mock('../../api', () => ({
  updateWorkingDir: vi.fn(async () => ({ data: {} })),
  // The card reads the chat's folder grants when it opens.
  listFolderAccess: vi.fn(async () => ({ data: [] })),
  revokeFolderAccess: vi.fn(async () => ({ data: { revoked: true } })),
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

import { DirSwitcher, deriveWorkingDirLocked } from './DirSwitcher';
import { ChatState } from '../../types/chatState';
import { updateWorkingDir } from '../../api';
import { toastError } from '../../toasts';
import { FOLDER_ACCESS_COPY, FOOTER_COPY } from './copy';

const WORKING_DIR = '/Users/wgu/Desktop';
const CHOSEN_DIR = '/Users/wgu/Desktop/data';

beforeAll(() => {
  // Radix's popover measures its anchor.
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
    // No home folder known unless a test says so: paths stay absolute.
    appConfig: undefined,
    electron: {
      directoryChooser: vi.fn(async () => ({ canceled: false, filePaths: [CHOSEN_DIR] })),
      addRecentDir: vi.fn(),
      openDirectoryInExplorer: vi.fn(),
    },
  });
});

/**
 * Spec 3.7: the footer names the FOLDER, never the path, in both states (names,
 * not IDs or paths, principle 10), with the word the sidebar's folder groups use
 * (`folderName`, utils/folderPath.ts, whose own cases are folderPath.test.ts's).
 * The path is one hover away, in the card. The name sits in a `<bdi>` so a
 * right-to-left folder name cannot reorder the line around it.
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

  describe('with the home folder known (the desktop’s own daemon)', () => {
    beforeEach(() => {
      Object.assign(window, {
        appConfig: {
          get: (key: string) =>
            ({ BIOROUTER_HOME_DIR: '/Users/wgu', BIOROUTER_LOCAL_BACKEND: true })[key],
        },
      });
    });

    it.each([
      // The visible word is not in the path, so the name says it first (WCAG 2.5.3).
      ['the home folder', '/Users/wgu', '~', 'Working folder: ~ (/Users/wgu)'],
      [
        'the Crew task folder',
        '/Users/wgu/Library/Application Support/Biorouter/crew/tasks',
        'Crew',
        'Working folder: Crew (/Users/wgu/Library/Application Support/Biorouter/crew/tasks)',
      ],
      // The visible word is in the path: the name is the path alone, as before.
      [
        'a folder under home',
        '/Users/wgu/Desktop',
        'Desktop',
        'Working folder: /Users/wgu/Desktop',
      ],
    ])('names %s as the sidebar does, and its name holds that word', (_case, dir, name, label) => {
      const { container } = render(<DirSwitcher sessionId="session-1" workingDir={dir} locked />);
      const chip = screen.getByTestId('dir-switcher-locked');
      expect(container.querySelector('bdi')?.textContent).toBe(name);
      expect(chip).toHaveAccessibleName(label);
      expect(chip.getAttribute('aria-label')).toContain(name);
      expect(FOOTER_COPY.folderLabel(dir, name)).toBe(label);
    });

    it('reads the home folder by its own name for a daemon that may be elsewhere', () => {
      Object.assign(window, {
        appConfig: { get: (key: string) => ({ BIOROUTER_HOME_DIR: '/Users/wgu' })[key] },
      });
      render(<DirSwitcher sessionId="session-1" workingDir="/Users/wgu" locked />);
      const chip = screen.getByTestId('dir-switcher-locked');
      expect(chip).toHaveTextContent('wgu');
      expect(chip).toHaveAccessibleName('Working folder: /Users/wgu');
    });
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

/**
 * The chip's click pins its card; the card's Choose folder… opens the chooser.
 * The press is inside act with the chooser's answer, so the chip's state
 * updates after it land there too.
 */
async function chooseFromCard() {
  fireEvent.click(screen.getByRole('button', { name: /^Working folder/ }), { detail: 1 });
  const choose = await screen.findByRole('button', { name: FOLDER_ACCESS_COPY.chooseFolder });
  await act(async () => {
    fireEvent.click(choose);
  });
}

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

    // A real button naming the folder; the path is its name.
    const chip = screen.getByRole('button');
    expect(chip).toHaveTextContent('Desktop');
    expect(chip).toHaveAccessibleName(`Working folder: ${WORKING_DIR}`);
    await chooseFromCard();

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

    await chooseFromCard();

    await waitFor(() => expect(updateWorkingDir).toHaveBeenCalled());
    await waitFor(() => expect(onRestartEnd).toHaveBeenCalled());

    // Refused server-side: nothing may change client-side.
    expect(onWorkingDirChange).not.toHaveBeenCalled();
    expect(window.electron.addRecentDir).not.toHaveBeenCalled();
    expect(toastError).toHaveBeenCalledWith({
      title: FOOTER_COPY.folderUpdateFailedTitle,
      msg: 'the working directory is fixed once a chat has messages',
    });
  });

  it('says "folder", like the rest of the line, when the server gives no reason', async () => {
    vi.mocked(updateWorkingDir).mockRejectedValueOnce({});
    render(<DirSwitcher sessionId="session-1" workingDir={WORKING_DIR} locked={false} />);
    await chooseFromCard();
    await waitFor(() => expect(toastError).toHaveBeenCalled());
    expect(toastError).toHaveBeenCalledWith({
      title: FOOTER_COPY.folderUpdateFailedTitle,
      msg: FOOTER_COPY.folderUpdateFailed,
    });
    expect(`${FOOTER_COPY.folderUpdateFailedTitle} ${FOOTER_COPY.folderUpdateFailed}`).not.toMatch(
      /director/i
    );
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

    await chooseFromCard();

    await waitFor(() => expect(onWorkingDirChange).toHaveBeenCalledWith(CHOSEN_DIR));
    expect(updateWorkingDir).not.toHaveBeenCalled();
  });
});

describe('DirSwitcher once the chat has messages (locked, #44)', () => {
  it('names the folder on a button that opens its card, which offers no chooser', async () => {
    const user = userEvent.setup();
    render(<DirSwitcher sessionId="session-1" workingDir={WORKING_DIR} locked={true} />);

    // Basename only, on the one button there is.
    const label = screen.getByTestId('dir-switcher-locked');
    expect(label).toHaveTextContent('Desktop');
    expect(label).not.toHaveTextContent(WORKING_DIR);
    expect(screen.getAllByRole('button')).toEqual([label]);
    expect(screen.queryByTestId('dir-switcher')).not.toBeInTheDocument();
    expect(label).toHaveAttribute('aria-haspopup', 'dialog');
    // A screen reader gets the path without opening the card.
    expect(label).toHaveAccessibleName(`Working folder: ${WORKING_DIR}`);

    // The full path is one hover away, in the card, which says the folder is
    // fixed and has no Choose folder….
    await user.hover(label);
    const card = await screen.findByRole('dialog', { name: FOLDER_ACCESS_COPY.cardLabel });
    expect(card).toHaveTextContent(WORKING_DIR);
    expect(card).toHaveTextContent(FOLDER_ACCESS_COPY.lockedNote);
    expect(screen.queryByRole('button', { name: FOLDER_ACCESS_COPY.chooseFolder })).toBeNull();
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

    fireEvent.click(screen.getByTestId('dir-switcher-locked'), { detail: 1 });
    fireEvent.click(screen.getByTestId('dir-switcher-locked'), { detail: 0 });

    // Nothing may happen — no chooser, no persistence, no local change.
    await waitFor(() => expect(window.electron.directoryChooser).not.toHaveBeenCalled());
    expect(updateWorkingDir).not.toHaveBeenCalled();
    expect(onWorkingDirChange).not.toHaveBeenCalled();
  });
});

// #50 — the chip's card must keep ONE control mode for its whole life. Both
// branches render the same Popover root in the same position, so React
// reconciles them into a single instance; when the working directory locks
// after the first message that instance must not flip from controlled
// (`open`/`onOpenChange`) to uncontrolled. Radix warns about it, and the
// state it warns about is the one that once yielded a stuck-open or
// unresponsive tooltip on this chip.
describe('DirSwitcher card control mode (#50)', () => {
  const CONTROL_MODE_WARNING = /changing from (controlled|uncontrolled) to/;

  it('does not flip the card between controlled and uncontrolled when the dir locks', async () => {
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
    expect(
      await screen.findByRole('dialog', { name: FOLDER_ACCESS_COPY.cardLabel })
    ).toHaveTextContent(WORKING_DIR);
  });
});

/// The footer is sans (principle 3: mono is for machine strings), and so is the
/// folder in its card: it is a reading like the card's others, written as the
/// app writes a folder everywhere (`FooterCardPath`). jsdom never runs Tailwind,
/// so these assert classes and markup; the face itself is measured in Chromium
/// (footerCard.browser.test.tsx).
describe('DirSwitcher: the folder name and the path in its card are both sans', () => {
  /** A folder the card writes (`FooterCardPath`), found by the absolute path it stands for. */
  const byPath = (container: Element | Document, absolute: string) =>
    [...container.querySelectorAll<HTMLElement>('[data-path]')].find(
      (node) => node.dataset.path === absolute
    ) ?? null;

  it('sets neither chip in monospace', () => {
    const { rerender } = render(
      <DirSwitcher sessionId="session-1" workingDir={WORKING_DIR} locked={false} />
    );
    expect(screen.getByText('Desktop').closest('.font-mono')).toBeNull();
    rerender(<DirSwitcher sessionId="session-1" workingDir={WORKING_DIR} locked={true} />);
    expect(screen.getByText('Desktop').closest('.font-mono')).toBeNull();
  });

  it('writes the path in the card as a value, with nothing in monospace', async () => {
    const user = userEvent.setup();
    render(<DirSwitcher sessionId="session-1" workingDir={WORKING_DIR} locked={true} />);

    await user.hover(screen.getByTestId('dir-switcher-locked'));
    const card = await screen.findByRole('dialog', { name: FOLDER_ACCESS_COPY.cardLabel });

    const path = byPath(card, WORKING_DIR);
    expect(path).not.toBeNull();
    expect(path).toHaveTextContent(WORKING_DIR);
    expect(path).toHaveClass('br-footer-card__value', 'br-footer-card__path');
    expect(path).not.toHaveClass('break-all');
    expect(card.querySelector('.font-mono')).toBeNull();
    expect(card.className).not.toMatch(/font-mono/);
  });
});
