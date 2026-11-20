import { describe, it, expect, vi, beforeEach, beforeAll, afterAll } from 'vitest';
import { act, render, screen, fireEvent, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

// The folder chip is a CONTROL (owner, 2026-11-19): a hover shows the app's
// label tooltip (the model chip's), naming the folder as `~/…` with one line of
// status, and a click does the chip's one job with no card in between. #44 —
// the empty-chat-only working-directory rule at the chip level: while the chat
// is completely empty (both the pre-session #39 path and a fresh zero-message
// session) the click opens the native chooser itself; once the chat has
// messages the click opens the folder access card (the chat's other folders
// and Revoke), which offers no chooser, so the chip can never reach
// updateWorkingDir. What the card says is pinned in FolderAccessCard.test.tsx,
// and when a footer card opens and closes in useFooterCard.test.tsx.

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
import { FOOTER_CARD_OPEN_DELAY_MS } from './useFooterCard';

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
 * The path is one hover away, in the label tooltip. The name sits in a `<bdi>`
 * so a right-to-left folder name cannot reorder the line around it.
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
 * The chip's click opens the chooser itself while the folder can change. The
 * press is inside act with the chooser's answer, so the chip's state updates
 * after it land there too.
 */
async function clickChip() {
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: /^Working folder/ }), { detail: 1 });
  });
}

/** A chooser that answers only when the test says so. */
function pendingChooser() {
  let answer: (value: { canceled: boolean; filePaths: string[] }) => void = () => {};
  const chooser = vi.fn(
    () =>
      new Promise<{ canceled: boolean; filePaths: string[] }>((resolve) => {
        answer = resolve;
      })
  );
  window.electron.directoryChooser = chooser as unknown as typeof window.electron.directoryChooser;
  return { chooser, answer: (value: { canceled: boolean; filePaths: string[] }) => answer(value) };
}

/** The label tooltip Radix shows on a hover, past its 500ms delay. */
const findTooltip = () => screen.findByRole('tooltip', {}, { timeout: 3000 });

describe('DirSwitcher while the chat is empty', () => {
  it('opens the chooser on the click itself, with no card in between, and persists the choice', async () => {
    const onWorkingDirChange = vi.fn();
    render(
      <DirSwitcher
        sessionId="session-1"
        workingDir={WORKING_DIR}
        locked={false}
        onWorkingDirChange={onWorkingDirChange}
      />
    );

    // A real button naming the folder; the path is its name. It opens the
    // system's own dialog, not a popup in the page.
    const chip = screen.getByRole('button');
    expect(chip).toHaveTextContent('Desktop');
    expect(chip).toHaveAccessibleName(`Working folder: ${WORKING_DIR}`);
    expect(chip).not.toHaveAttribute('aria-haspopup');
    await clickChip();

    expect(window.electron.directoryChooser).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole('dialog')).toBeNull();
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

  it('opens the chooser from the keyboard too', async () => {
    const user = userEvent.setup();
    render(<DirSwitcher sessionId={undefined} workingDir={WORKING_DIR} locked={false} />);
    act(() => screen.getByRole('button').focus());
    await user.keyboard('{Enter}');
    await waitFor(() => expect(window.electron.directoryChooser).toHaveBeenCalledTimes(1));
    expect(screen.queryByRole('dialog')).toBeNull();
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

    await clickChip();

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
    await clickChip();
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

    await clickChip();

    await waitFor(() => expect(onWorkingDirChange).toHaveBeenCalledWith(CHOSEN_DIR));
    expect(updateWorkingDir).not.toHaveBeenCalled();
  });

  it('opens one chooser at a time, and a click while it is up arms nothing for after it (#50)', async () => {
    const { chooser, answer } = pendingChooser();
    const onWorkingDirChange = vi.fn();
    render(
      <DirSwitcher
        sessionId={undefined}
        workingDir={WORKING_DIR}
        locked={false}
        onWorkingDirChange={onWorkingDirChange}
      />
    );
    await clickChip();
    await clickChip();
    expect(chooser).toHaveBeenCalledTimes(1);

    await act(async () => answer({ canceled: true, filePaths: [] }));
    // Nothing was chosen, nothing opened after the chooser, and nothing is marked.
    expect(onWorkingDirChange).not.toHaveBeenCalled();
    expect(chooser).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(screen.getByRole('button')).not.toHaveAttribute('data-pinned');
  });

  it('shows the folder and what a click does in the label tooltip on a hover, and no card', async () => {
    const user = userEvent.setup();
    render(<DirSwitcher sessionId="session-1" workingDir={WORKING_DIR} locked={false} />);
    await user.hover(screen.getByRole('button'));

    const tip = await findTooltip();
    expect(tip).toHaveTextContent(`${FOOTER_COPY.folderTooltipLabel} ${WORKING_DIR}`);
    expect(tip).toHaveTextContent(FOOTER_COPY.folderChangeHint);
    expect(tip).not.toHaveTextContent(FOOTER_COPY.folderFixedHint);
    // A hover only shows: past the footer cards' delay, still no card and no chooser.
    await act(() => new Promise((done) => setTimeout(done, FOOTER_CARD_OPEN_DELAY_MS + 50)));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(window.electron.directoryChooser).not.toHaveBeenCalled();
  });

  it('shows the label tooltip on a Tab onto the chip, and opens nothing', async () => {
    const user = userEvent.setup();
    render(
      <>
        <button type="button">Before</button>
        <DirSwitcher sessionId={undefined} workingDir={WORKING_DIR} locked={false} />
      </>
    );
    act(() => screen.getByRole('button', { name: 'Before' }).focus());
    await user.tab();
    expect(screen.getByRole('button', { name: /^Working folder/ })).toHaveFocus();
    expect(await findTooltip()).toHaveTextContent(FOOTER_COPY.folderChangeHint);
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(window.electron.directoryChooser).not.toHaveBeenCalled();
  });

  it('writes a folder under home as ~/… in the tooltip', async () => {
    Object.assign(window, {
      appConfig: {
        get: (key: string) =>
          ({ BIOROUTER_HOME_DIR: '/Users/wgu', BIOROUTER_LOCAL_BACKEND: true })[key],
      },
    });
    const user = userEvent.setup();
    render(<DirSwitcher sessionId={undefined} workingDir={WORKING_DIR} locked={false} />);
    await user.hover(screen.getByRole('button'));
    const tip = await findTooltip();
    expect(tip).toHaveTextContent(`${FOOTER_COPY.folderTooltipLabel} ~/Desktop`);
    expect(tip).not.toHaveTextContent('/Users/wgu');
  });
});

describe('DirSwitcher once the chat has messages (locked, #44)', () => {
  it('names the folder on a button that opens the folder access card, which offers no chooser', async () => {
    render(<DirSwitcher sessionId="session-1" workingDir={WORKING_DIR} locked={true} />);

    // Basename only, on the one button there is.
    const label = screen.getByTestId('dir-switcher-locked');
    expect(label).toHaveTextContent('Desktop');
    expect(label).not.toHaveTextContent(WORKING_DIR);
    expect(screen.getAllByRole('button')).toEqual([label]);
    expect(screen.queryByTestId('dir-switcher')).not.toBeInTheDocument();
    expect(label).toHaveAttribute('aria-haspopup', 'dialog');
    expect(label).toHaveAttribute('aria-expanded', 'false');
    // A screen reader gets the path without the tooltip.
    expect(label).toHaveAccessibleName(`Working folder: ${WORKING_DIR}`);

    // A click opens the card, which says the folder is fixed and holds no
    // control that chooses a folder; the chip shows it kept.
    await clickChip();
    const card = await screen.findByRole('dialog', { name: FOLDER_ACCESS_COPY.cardLabel });
    expect(card).toHaveTextContent(WORKING_DIR);
    expect(card).toHaveTextContent(FOLDER_ACCESS_COPY.lockedNote);
    expect(within(card).queryByRole('button', { name: /choose/i })).toBeNull();
    expect(label).toHaveAttribute('aria-expanded', 'true');
    expect(label).toHaveAttribute('data-pinned', 'true');

    // A second click closes it.
    await clickChip();
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(label).not.toHaveAttribute('data-pinned');
  });

  it('shows the folder and that it is fixed in the label tooltip on a hover, and no card', async () => {
    const user = userEvent.setup();
    render(<DirSwitcher sessionId="session-1" workingDir={WORKING_DIR} locked={true} />);
    await user.hover(screen.getByTestId('dir-switcher-locked'));

    const tip = await findTooltip();
    expect(tip).toHaveTextContent(`${FOOTER_COPY.folderTooltipLabel} ${WORKING_DIR}`);
    expect(tip).toHaveTextContent(FOOTER_COPY.folderFixedHint);
    expect(tip).not.toHaveTextContent(FOOTER_COPY.folderChangeHint);
    await act(() => new Promise((done) => setTimeout(done, FOOTER_CARD_OPEN_DELAY_MS + 50)));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(screen.getByTestId('dir-switcher-locked')).not.toHaveAttribute('data-pinned');
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

  it('opens the card from the keyboard and moves focus into it; Escape gives it back', async () => {
    const user = userEvent.setup();
    render(<DirSwitcher sessionId="session-1" workingDir={WORKING_DIR} locked={true} />);
    const chip = screen.getByTestId('dir-switcher-locked');
    act(() => chip.focus());
    await user.keyboard('{Enter}');
    const card = await screen.findByRole('dialog', { name: FOLDER_ACCESS_COPY.cardLabel });
    await waitFor(() => expect(card).toHaveFocus());
    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(chip).toHaveFocus();
    // Focus handed back by a program shows no tooltip.
    await act(() => new Promise((done) => setTimeout(done, 700)));
    expect(screen.queryByRole('tooltip')).toBeNull();
  });

  it('says the folder is not known yet, and still opens the card on a click', async () => {
    const user = userEvent.setup();
    render(<DirSwitcher sessionId="session-1" workingDir={null} locked={true} />);
    const chip = screen.getByTestId('dir-switcher-unknown');
    expect(chip).toHaveAccessibleName(FOOTER_COPY.folderUnknownLabel);
    await user.hover(chip);
    expect(await findTooltip()).toHaveTextContent(FOOTER_COPY.folderUnknownLabel);
    await clickChip();
    expect(
      await screen.findByRole('dialog', { name: FOLDER_ACCESS_COPY.cardLabel })
    ).toHaveTextContent(FOLDER_ACCESS_COPY.workingFolderUnknown);
    expect(window.electron.directoryChooser).not.toHaveBeenCalled();
  });
});

describe('the reveal chord (Cmd-click on macOS, Ctrl-click elsewhere)', () => {
  it.each([
    ['an empty chat', false],
    ['a chat with messages', true],
  ])('opens the folder in the file manager in %s and does nothing else', async (_case, locked) => {
    Object.assign(window.electron, { platform: 'darwin' });
    render(<DirSwitcher sessionId="session-1" workingDir={WORKING_DIR} locked={locked} />);
    await act(async () => {
      fireEvent.click(screen.getByRole('button'), { detail: 1, metaKey: true });
    });
    expect(window.electron.openDirectoryInExplorer).toHaveBeenCalledWith(WORKING_DIR);
    expect(window.electron.directoryChooser).not.toHaveBeenCalled();
    expect(screen.queryByRole('dialog')).toBeNull();
  });
});

// #50 — the chip's card and its tooltip must each keep ONE control mode for
// their whole life. Both states render the same roots in the same position, so
// React reconciles them into a single instance; when the working directory
// locks after the first message that instance must not flip from controlled
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

  it('still shows the full path in the tooltip on hover once locked', async () => {
    const user = userEvent.setup();
    const { rerender } = render(
      <DirSwitcher sessionId="session-1" workingDir={WORKING_DIR} locked={false} />
    );
    rerender(<DirSwitcher sessionId="session-1" workingDir={WORKING_DIR} locked={true} />);

    await user.hover(screen.getByTestId('dir-switcher-locked'));
    const tip = await findTooltip();
    expect(tip).toHaveTextContent(WORKING_DIR);
    expect(tip).toHaveTextContent(FOOTER_COPY.folderFixedHint);
  });
});

/// The footer is sans (principle 3: mono is for machine strings), and so is the
/// folder in its tooltip and its card: a reading written as the app writes a
/// folder everywhere (`PathSegments`, `FooterCardPath`). jsdom never runs
/// Tailwind, so these assert classes and markup; the face itself is measured in
/// Chromium (footerCard.browser.test.tsx).
describe('DirSwitcher: the folder name, the path in its tooltip and in its card are sans', () => {
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

  it('writes the path in the tooltip name by name, with nothing in monospace', async () => {
    const user = userEvent.setup();
    render(<DirSwitcher sessionId="session-1" workingDir={WORKING_DIR} locked={false} />);
    await user.hover(screen.getByRole('button'));
    await findTooltip();
    // Radix draws the visible tooltip and a hidden copy for assistive technology; both are drawn
    // from the same markup.
    const paths = [...document.querySelectorAll<HTMLElement>('.br-folder-tip__path')];
    expect(paths.length).toBeGreaterThan(0);
    for (const path of paths) {
      expect(path).toHaveAttribute('dir', 'ltr');
      expect([...path.querySelectorAll('bdi')].map((bdi) => bdi.textContent)).toEqual([
        'Users',
        'wgu',
        'Desktop',
      ]);
      expect(path.closest('.font-mono')).toBeNull();
    }
  });

  it('writes the path in the card as a value, with nothing in monospace', async () => {
    render(<DirSwitcher sessionId="session-1" workingDir={WORKING_DIR} locked={true} />);

    await clickChip();
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
