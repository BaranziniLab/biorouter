import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ComponentProps } from 'react';

/**
 * The working folder's card, driven through the real chip (`DirSwitcher`), which owns the
 * Popover root and its anchor. When a footer card opens and closes is pinned once for all three
 * items, in one table, in `footerCards.contract.test.tsx` (the primitive's own corners are
 * `useFooterCard.test.tsx`'s); this file pins what is the folder's own: the chip's click pins the
 * card in EVERY state (the contract renders a chat with messages only), Choose folder… is the only
 * way to the chooser and only while the chat is empty (#44), the card stays shut while the chooser
 * is open (#50), Cmd or Ctrl-click, and the grants. It replaced the footer's "Folder access"
 * button and dialog, so every behaviour `FolderAccess.test.tsx` pinned lives here now: grants load
 * only when the card opens, Revoke acts on the selected grant in its own chat with the proof
 * header, a refused revoke stays visible, and the browser's approval key is labelled, described,
 * sent only as `X-Folder-Access-Key` and cleared after every attempt.
 */

const mocks = vi.hoisted(() => ({
  list: vi.fn(),
  revoke: vi.fn(),
  update: vi.fn(),
  browser: false,
}));
vi.mock('../../api', () => ({
  listFolderAccess: mocks.list,
  revokeFolderAccess: mocks.revoke,
  updateWorkingDir: mocks.update,
}));
vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'desktop-human-proof' }),
}));
vi.mock('../../utils/surface', () => ({ isBrowserSurface: () => mocks.browser }));
vi.mock('../../toasts', () => ({ toastError: vi.fn() }));

import { DirSwitcher } from './DirSwitcher';
import { FOLDER_ACCESS_COPY, FOOTER_COPY } from './copy';
import { FOOTER_CARD_CLOSE_GRACE_MS, FOOTER_CARD_OPEN_DELAY_MS } from './useFooterCard';

const WORKING_DIR = '/Users/wgu/Projects/kdps';
const RESULTS = {
  id: 'grant-1',
  folder: '/synthetic/results',
  access: 'read-only',
  descendants: true,
};
const NOTES = {
  id: 'grant-2',
  folder: '/synthetic/notes',
  access: 'read/write',
  descendants: false,
};
const PROOF = { 'X-User-Action': 'desktop-human-proof' };

beforeAll(() => {
  // Radix measures the anchor and the surface.
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
  );
});
afterAll(() => vi.unstubAllGlobals());

beforeEach(() => {
  mocks.browser = false;
  mocks.list.mockReset().mockResolvedValue({ data: [RESULTS] });
  mocks.revoke.mockReset().mockResolvedValue({ data: { revoked: true } });
  mocks.update.mockReset().mockResolvedValue({ data: {} });
  Object.assign(window, {
    appConfig: undefined,
    electron: {
      directoryChooser: vi.fn(async () => ({ canceled: true, filePaths: [] })),
      addRecentDir: vi.fn(),
      openDirectoryInExplorer: vi.fn(),
    },
  });
});

afterEach(() => {
  vi.useRealTimers();
});

type ChipProps = Partial<ComponentProps<typeof DirSwitcher>>;

function Page(props: ChipProps) {
  return (
    <div>
      <button type="button">Before</button>
      <DirSwitcher sessionId="chat-a" workingDir={WORKING_DIR} locked {...props} />
      <button type="button">Outside</button>
    </div>
  );
}

const card = () => document.querySelector<HTMLElement>('[data-testid="folder-access-card"]');
const isOpen = () => card()?.getAttribute('data-state') === 'open';
const chip = () => screen.getByRole('button', { name: /^Working folder/ });
const chooseButton = () => screen.queryByRole('button', { name: FOLDER_ACCESS_COPY.chooseFolder });

/** Let the grant read (a resolved mock) land inside act. */
const flush = () => act(async () => {});

function hoverOpen(target: HTMLElement) {
  fireEvent.pointerEnter(target, { pointerType: 'mouse' });
  act(() => {
    vi.advanceTimersByTime(FOOTER_CARD_OPEN_DELAY_MS + 10);
  });
}

/** A chooser that answers only when the test says so. */
function holdChooser() {
  let answer: ((value: { canceled: boolean; filePaths: string[] }) => void) | undefined;
  window.electron.directoryChooser = vi.fn(
    () =>
      new Promise((resolve) => {
        answer = resolve;
      })
  ) as typeof window.electron.directoryChooser;
  return (value: { canceled: boolean; filePaths: string[] }) => act(async () => answer?.(value));
}

describe('the chip pins its card in every state', () => {
  it.each([
    ['on Home', { sessionId: undefined, locked: false }, 'dir-switcher'],
    ['in an empty chat', { locked: false }, 'dir-switcher'],
    ['in a chat with messages (#44)', { locked: true }, 'dir-switcher-locked'],
    ['while the folder is not known yet', { workingDir: null }, 'dir-switcher-unknown'],
    [
      'while the folder of an empty chat is not known yet',
      { workingDir: null, locked: false },
      'dir-switcher-unknown',
    ],
  ] as const)(
    'pins on a click %s, and a second click closes it, choosing nothing',
    async (_state, props, testId) => {
      const onWorkingDirChange = vi.fn();
      const user = userEvent.setup();
      render(<Page {...props} onWorkingDirChange={onWorkingDirChange} />);
      const target = screen.getByTestId(testId);
      expect(target).toBe(chip());
      expect(target.tagName).toBe('BUTTON');
      expect(target).toHaveAttribute('type', 'button');
      expect(target).toHaveClass('br-footline__item');
      expect(target).toHaveAttribute('aria-haspopup', 'dialog');
      expect(target).toHaveAttribute('aria-expanded', 'false');

      await user.click(target);
      expect(isOpen()).toBe(true);
      expect(target).toHaveAttribute('aria-expanded', 'true');
      expect(target).toHaveAttribute('aria-controls', card()!.id);

      // Pinned: the pointer wandering off does not close it.
      await user.unhover(target);
      await act(
        () => new Promise((resolve) => setTimeout(resolve, FOOTER_CARD_CLOSE_GRACE_MS * 2))
      );
      expect(isOpen()).toBe(true);

      await user.click(target);
      expect(isOpen()).toBe(false);
      expect(window.electron.directoryChooser).not.toHaveBeenCalled();
      expect(mocks.update).not.toHaveBeenCalled();
      expect(onWorkingDirChange).not.toHaveBeenCalled();
    }
  );
});

describe('choosing a folder (only from the card, only while the chat is empty)', () => {
  it('offers Choose folder… in an empty chat and on Home, and nowhere else', async () => {
    const user = userEvent.setup();
    const view = render(<Page locked={false} />);
    await user.click(chip());
    expect(chooseButton()).toBeInTheDocument();
    expect(card()).not.toHaveTextContent(FOLDER_ACCESS_COPY.lockedNote);
    await user.click(chip());

    view.rerender(<Page locked={false} sessionId={undefined} />);
    await user.click(chip());
    expect(chooseButton()).toBeInTheDocument();
    await user.click(chip());

    // #44: once the chat has messages the card says the folder is fixed, and offers no chooser.
    view.rerender(<Page locked />);
    await user.click(chip());
    expect(chooseButton()).toBeNull();
    expect(card()).toHaveTextContent(FOLDER_ACCESS_COPY.lockedNote);
    await user.click(chip());

    // No folder known: nothing to choose from, in either state.
    view.rerender(<Page locked={false} workingDir={null} />);
    await user.click(chip());
    expect(chooseButton()).toBeNull();
    expect(card()).not.toHaveTextContent(FOLDER_ACCESS_COPY.lockedNote);
    await user.click(chip());
    view.rerender(<Page locked workingDir={null} />);
    await user.click(chip());
    expect(chooseButton()).toBeNull();
    expect(card()).toHaveTextContent(FOLDER_ACCESS_COPY.lockedNote);
    await flush();
    expect(window.electron.directoryChooser).not.toHaveBeenCalled();
  });

  it('closes the card, opens the chooser, and keeps the card shut while the chooser is open (#50)', async () => {
    vi.useFakeTimers();
    const answer = holdChooser();
    render(<Page locked={false} sessionId={undefined} />);
    fireEvent.click(chip(), { detail: 1 });
    expect(isOpen()).toBe(true);
    fireEvent.click(chooseButton()!);
    expect(window.electron.directoryChooser).toHaveBeenCalledTimes(1);
    expect(isOpen()).toBe(false);

    // The native chooser is up: a hover, a click or a focus must not bring the card back over it.
    hoverOpen(chip());
    expect(isOpen()).toBe(false);
    fireEvent.click(chip(), { detail: 1 });
    expect(isOpen()).toBe(false);
    fireEvent.click(chip(), { detail: 1, metaKey: true });
    expect(window.electron.openDirectoryInExplorer).not.toHaveBeenCalled();

    await answer({ canceled: true, filePaths: [] });
    // Nothing armed during the chooser opens the card now that it is gone, nor once any open
    // delay has run out.
    expect(isOpen()).toBe(false);
    act(() => {
      vi.advanceTimersByTime(FOOTER_CARD_OPEN_DELAY_MS * 2);
    });
    expect(isOpen()).toBe(false);
    expect(mocks.list).not.toHaveBeenCalled();

    // A fresh hover once the chooser has closed opens it again.
    fireEvent.pointerLeave(chip(), { pointerType: 'mouse' });
    hoverOpen(chip());
    expect(isOpen()).toBe(true);
  });

  it('hands focus back to the chip when Choose folder… is pressed from the keyboard', async () => {
    const answer = holdChooser();
    const user = userEvent.setup();
    render(<Page locked={false} />);
    act(() => chip().focus());
    await user.keyboard('{Enter}');
    await waitFor(() => expect(card()).toHaveFocus());
    await user.tab();
    expect(chooseButton()).toHaveFocus();
    await user.keyboard('{Enter}');
    expect(window.electron.directoryChooser).toHaveBeenCalledTimes(1);
    expect(isOpen()).toBe(false);
    expect(chip()).toHaveFocus();
    await answer({ canceled: true, filePaths: [] });
    expect(isOpen()).toBe(false);
  });

  it('persists the choice for an empty chat, bracketed by the restart, with the proof header', async () => {
    window.electron.directoryChooser = vi.fn(async () => ({
      canceled: false,
      filePaths: ['/Users/wgu/Projects/other'],
    }));
    const onWorkingDirChange = vi.fn();
    const onRestartStart = vi.fn();
    const onRestartEnd = vi.fn();
    const user = userEvent.setup();
    render(
      <Page
        locked={false}
        onWorkingDirChange={onWorkingDirChange}
        onRestartStart={onRestartStart}
        onRestartEnd={onRestartEnd}
      />
    );
    await user.click(chip());
    await user.click(chooseButton()!);
    await waitFor(() =>
      expect(mocks.update).toHaveBeenCalledWith({
        headers: PROOF,
        body: { session_id: 'chat-a', working_dir: '/Users/wgu/Projects/other' },
        throwOnError: true,
      })
    );
    expect(onRestartStart).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(onRestartEnd).toHaveBeenCalledTimes(1));
    expect(onWorkingDirChange).toHaveBeenCalledWith('/Users/wgu/Projects/other');
  });

  it('never reaches the chooser or updateWorkingDir from a locked chip, by any press (#44)', async () => {
    const onWorkingDirChange = vi.fn();
    const user = userEvent.setup();
    render(<Page onWorkingDirChange={onWorkingDirChange} />);
    await user.click(chip());
    await user.click(chip());
    act(() => chip().focus());
    await user.keyboard('{Enter}');
    await user.keyboard('{Escape}');
    await user.keyboard(' ');
    await flush();
    for (const button of screen.queryAllByRole('button')) {
      expect(button).not.toHaveAccessibleName(FOLDER_ACCESS_COPY.chooseFolder);
    }
    expect(window.electron.directoryChooser).not.toHaveBeenCalled();
    expect(mocks.update).not.toHaveBeenCalled();
    expect(onWorkingDirChange).not.toHaveBeenCalled();
  });
});

describe('Cmd or Ctrl-click (an accelerator)', () => {
  it.each([
    ['an empty chat', { locked: false }],
    ['a chat with messages', { locked: true }],
  ] as const)('opens the folder of %s in the file manager, and pins nothing', (_state, props) => {
    render(<Page {...props} />);
    fireEvent.click(chip(), { metaKey: true, detail: 1 });
    expect(window.electron.openDirectoryInExplorer).toHaveBeenCalledWith(WORKING_DIR);
    fireEvent.click(chip(), { ctrlKey: true, detail: 1 });
    expect(window.electron.openDirectoryInExplorer).toHaveBeenCalledTimes(2);
    expect(isOpen()).toBe(false);
    expect(window.electron.directoryChooser).not.toHaveBeenCalled();
    expect(mocks.update).not.toHaveBeenCalled();
  });

  it('has no folder to open while the folder is not known, so it is an ordinary click', async () => {
    render(<Page workingDir={null} locked={false} />);
    fireEvent.click(chip(), { metaKey: true, detail: 1 });
    expect(window.electron.openDirectoryInExplorer).not.toHaveBeenCalled();
    expect(isOpen()).toBe(true);
    await flush();
  });
});

describe('a chat whose working folder is not known yet', () => {
  it('keeps a nameless chip whose card lists and revokes the chat’s grants', async () => {
    const user = userEvent.setup();
    render(<Page workingDir={null} />);
    const unknown = screen.getByTestId('dir-switcher-unknown');
    expect(unknown).toHaveAccessibleName(FOOTER_COPY.folderUnknownLabel);
    expect(unknown.querySelector('.br-footline__name')).toBeNull();
    await user.click(unknown);
    const dialog = screen.getByRole('dialog', { name: FOLDER_ACCESS_COPY.cardLabel });
    expect(dialog).toHaveTextContent(FOLDER_ACCESS_COPY.workingFolderUnknown);
    await screen.findByText(RESULTS.folder);
    mocks.list.mockResolvedValueOnce({ data: [] });
    await user.click(
      screen.getByRole('button', { name: FOLDER_ACCESS_COPY.revokeLabel(RESULTS.folder) })
    );
    await waitFor(() =>
      expect(mocks.revoke).toHaveBeenCalledWith({
        body: { sessionId: 'chat-a', grantId: 'grant-1' },
        headers: PROOF,
      })
    );
    expect(await screen.findByText(FOLDER_ACCESS_COPY.none)).toBeInTheDocument();
  });
});

describe('what the card says', () => {
  it('shows the full working folder in mono, and the chip only its name', async () => {
    const user = userEvent.setup();
    render(<Page />);
    expect(chip()).toHaveTextContent('kdps');
    expect(chip()).not.toHaveTextContent(WORKING_DIR);
    await user.click(chip());
    const dialog = screen.getByRole('dialog', { name: FOLDER_ACCESS_COPY.cardLabel });
    expect(FOLDER_ACCESS_COPY.cardLabel).toBe('Working folder');
    expect(dialog).toHaveTextContent(FOLDER_ACCESS_COPY.workingFolder);
    const path = within(dialog).getByText(WORKING_DIR);
    expect(path).toHaveClass('font-mono');
    expect(path).toHaveClass('break-all');
    await flush();
  });

  it('builds the card from the shared pieces: labelled sections, then the note last', async () => {
    const user = userEvent.setup();
    render(<Page />);
    await user.click(chip());
    await screen.findByText(RESULTS.folder);
    const surface = card()!;
    expect(surface).toHaveClass('br-footer-card');
    const labels = [
      ...surface.querySelectorAll('.br-footer-card__section > .br-footer-card__label'),
    ];
    expect(labels.map((label) => label.textContent)).toEqual([
      FOLDER_ACCESS_COPY.workingFolder,
      FOLDER_ACCESS_COPY.otherFolders,
    ]);
    expect(surface.lastElementChild).toHaveTextContent(FOLDER_ACCESS_COPY.whoAsks);
    expect(surface.lastElementChild).toHaveClass('br-footer-card__note');
  });

  it('says exactly which tools ask for folder approval, and never that everything asks', async () => {
    // Folder approval gates the Developer and Files extensions' editor, analysis and image tools
    // only (`file_access_plan`); a shell command or a document tool reads another folder without
    // one. So no sentence may say Biorouter as a whole asks.
    expect(FOLDER_ACCESS_COPY.whoAsks).toMatch(/file editor, file analysis and image tools ask/);
    expect(FOLDER_ACCESS_COPY.whoAsks).toMatch(/Shell commands and document tools do not ask/);
    expect(FOLDER_ACCESS_COPY.whoAsks).not.toMatch(/Biorouter (asks|needs)|Built-in file tools/);
    mocks.list.mockResolvedValue({ data: [] });
    const user = userEvent.setup();
    render(<Page />);
    await user.click(chip());
    expect(await screen.findByText(FOLDER_ACCESS_COPY.none)).toBeInTheDocument();
    expect(card()).toHaveTextContent(FOLDER_ACCESS_COPY.whoAsks);
  });

  it('reads nothing on Home, lists no other folders there, and still says which tools ask', async () => {
    vi.useFakeTimers();
    render(<Page sessionId={undefined} locked={false} />);
    hoverOpen(chip());
    await flush();
    expect(isOpen()).toBe(true);
    expect(card()).toHaveTextContent(WORKING_DIR);
    expect(card()).toHaveTextContent(FOLDER_ACCESS_COPY.whoAsks);
    expect(card()).not.toHaveTextContent(FOLDER_ACCESS_COPY.otherFolders);
    expect(mocks.list).not.toHaveBeenCalled();
  });

  it('treats a new chat with no session yet like Home, then lists its grants once it has one', async () => {
    // A new chat (`#/pair` before its first message) reaches the chip with an EMPTY session id,
    // not `undefined`. Such a chat can hold no grant, so the card must not wait for a read that
    // is never sent: it once said "Checking folder access…" for as long as it stayed open.
    const user = userEvent.setup();
    const view = render(<Page sessionId="" locked={false} />);
    await user.click(chip());
    await flush();
    expect(isOpen()).toBe(true);
    expect(card()).toHaveTextContent(WORKING_DIR);
    expect(chooseButton()).toBeInTheDocument();
    expect(card()).toHaveTextContent(FOLDER_ACCESS_COPY.whoAsks);
    expect(card()).not.toHaveTextContent(FOLDER_ACCESS_COPY.loading);
    expect(card()).not.toHaveTextContent(FOLDER_ACCESS_COPY.otherFolders);
    expect(mocks.list).not.toHaveBeenCalled();

    // The chat gets its session while the card is open: the section appears and reads it.
    view.rerender(<Page sessionId="chat-a" locked={false} />);
    expect(await within(card()!).findByText(RESULTS.folder)).toBeInTheDocument();
    expect(card()).toHaveTextContent(FOLDER_ACCESS_COPY.otherFolders);
    expect(mocks.list).toHaveBeenCalledTimes(1);
    expect(mocks.list).toHaveBeenCalledWith({ query: { session_id: 'chat-a' }, headers: PROOF });
  });

  it('reads the grants only once the card opens, with the proof header', async () => {
    vi.useFakeTimers();
    render(<Page />);
    expect(mocks.list).not.toHaveBeenCalled();
    fireEvent.pointerEnter(chip(), { pointerType: 'mouse' });
    act(() => {
      vi.advanceTimersByTime(FOOTER_CARD_OPEN_DELAY_MS - 10);
    });
    expect(mocks.list).not.toHaveBeenCalled();
    act(() => {
      vi.advanceTimersByTime(20);
    });
    await flush();
    expect(mocks.list).toHaveBeenCalledTimes(1);
    expect(mocks.list).toHaveBeenCalledWith({
      query: { session_id: 'chat-a' },
      headers: PROOF,
    });
  });

  it('lists each grant with its access in words and a Revoke that names the folder', async () => {
    mocks.list.mockResolvedValue({
      data: [
        RESULTS,
        NOTES,
        { id: 'grant-3', folder: '/synthetic/odd', access: 'append', descendants: true },
      ],
    });
    const user = userEvent.setup();
    render(<Page />);
    await user.click(chip());
    const list = await screen.findByRole('list', { name: FOLDER_ACCESS_COPY.otherFolders });
    const rows = within(list).getAllByRole('listitem');
    expect(rows).toHaveLength(3);
    expect(rows[0]).toHaveTextContent('/synthetic/results');
    expect(rows[0]).toHaveTextContent('Read only · includes subfolders');
    expect(rows[1]).toHaveTextContent('Read and write · listed files only');
    // A value this build does not know is shown as the backend wrote it.
    expect(rows[2]).toHaveTextContent('append · includes subfolders');
    expect(within(rows[0]).getByText('/synthetic/results')).toHaveClass('font-mono');
    expect(
      within(rows[1]).getByRole('button', { name: 'Revoke access to /synthetic/notes' })
    ).toHaveTextContent(FOLDER_ACCESS_COPY.revoke);
    expect(card()).toHaveTextContent(FOLDER_ACCESS_COPY.onceNotListed);
  });

  it('says "Checking…" on the first read only, and keeps the last list during a refresh', async () => {
    let answer: ((value: unknown) => void) | undefined;
    mocks.list.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          answer = resolve;
        })
    );
    const user = userEvent.setup();
    render(<Page />);
    await user.click(chip());
    expect(screen.getByRole('status')).toHaveTextContent(FOLDER_ACCESS_COPY.loading);
    await act(async () => answer?.({ data: [RESULTS] }));
    expect(screen.getByText(RESULTS.folder)).toBeInTheDocument();
    await user.click(chip());
    expect(isOpen()).toBe(false);

    // Reopened: the read runs again, and the last list shows while it does.
    mocks.list.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          answer = resolve;
        })
    );
    await user.click(chip());
    expect(mocks.list).toHaveBeenCalledTimes(2);
    expect(screen.getByText(RESULTS.folder)).toBeInTheDocument();
    expect(screen.queryByRole('status')).toBeNull();
    await act(async () => answer?.({ data: [] }));
    expect(card()).toHaveTextContent(FOLDER_ACCESS_COPY.none);
  });

  it('says so when there are none', async () => {
    mocks.list.mockResolvedValue({ data: [] });
    const user = userEvent.setup();
    render(<Page />);
    await user.click(chip());
    expect(await screen.findByText(FOLDER_ACCESS_COPY.none)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Revoke/ })).toBeNull();
  });

  it('says so when the read fails', async () => {
    mocks.list.mockResolvedValue({ error: 'refused' });
    const user = userEvent.setup();
    render(<Page />);
    await user.click(chip());
    expect(await screen.findByRole('alert')).toHaveTextContent(FOLDER_ACCESS_COPY.loadFailed);
    expect(screen.queryByRole('list')).toBeNull();
  });

  it('states the desktop lifetime only for the managed desktop backend', async () => {
    Object.assign(window, {
      appConfig: { get: (key: string) => (key === 'BIOROUTER_LOCAL_BACKEND' ? true : undefined) },
    });
    const user = userEvent.setup();
    render(<Page />);
    await user.click(chip());
    await screen.findByText(RESULTS.folder);
    expect(card()).toHaveTextContent('desktop app quits');
  });
});

describe('revoking (the only revoke control in the interface)', () => {
  it('revokes only the selected grant, in its own chat, with the proof header, then rereads', async () => {
    mocks.list.mockResolvedValueOnce({ data: [RESULTS, NOTES] });
    const user = userEvent.setup();
    render(<Page />);
    await user.click(chip());
    await screen.findByText(NOTES.folder);
    mocks.list.mockResolvedValueOnce({ data: [RESULTS] });
    await user.click(
      screen.getByRole('button', { name: FOLDER_ACCESS_COPY.revokeLabel(NOTES.folder) })
    );
    await waitFor(() =>
      expect(mocks.revoke).toHaveBeenCalledWith({
        body: { sessionId: 'chat-a', grantId: 'grant-2' },
        headers: PROOF,
      })
    );
    expect(mocks.revoke).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(screen.queryByText(NOTES.folder)).toBeNull());
    expect(screen.getByText(RESULTS.folder)).toBeInTheDocument();
    expect(mocks.list).toHaveBeenLastCalledWith({
      query: { session_id: 'chat-a' },
      headers: PROOF,
    });
    // The card is still open: a press inside it pinned it.
    expect(isOpen()).toBe(true);

    mocks.list.mockResolvedValueOnce({ data: [] });
    await user.click(
      screen.getByRole('button', { name: FOLDER_ACCESS_COPY.revokeLabel(RESULTS.folder) })
    );
    expect(await screen.findByText(FOLDER_ACCESS_COPY.none)).toBeInTheDocument();
  });

  it('keeps a failed revocation visible instead of pretending it succeeded', async () => {
    mocks.revoke.mockResolvedValueOnce({ error: 'refused' });
    const user = userEvent.setup();
    render(<Page />);
    await user.click(chip());
    await screen.findByText(RESULTS.folder);
    await user.click(
      screen.getByRole('button', { name: FOLDER_ACCESS_COPY.revokeLabel(RESULTS.folder) })
    );
    expect(await screen.findByRole('alert')).toHaveTextContent(FOLDER_ACCESS_COPY.revokeFailed);
    expect(screen.getByText(RESULTS.folder)).toBeInTheDocument();
    expect(isOpen()).toBe(true);
  });

  it('disables Revoke while a revocation is running', async () => {
    let answer: ((value: unknown) => void) | undefined;
    mocks.list.mockResolvedValueOnce({ data: [RESULTS, NOTES] });
    mocks.revoke.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          answer = resolve;
        })
    );
    const user = userEvent.setup();
    render(<Page />);
    await user.click(chip());
    await screen.findByText(NOTES.folder);
    await user.click(
      screen.getByRole('button', { name: FOLDER_ACCESS_COPY.revokeLabel(NOTES.folder) })
    );
    for (const button of screen.getAllByRole('button', { name: /^Revoke access to/ })) {
      expect(button).toBeDisabled();
    }
    await act(async () => answer?.({ data: { revoked: true } }));
  });

  it('labels the browser key apart from its help, sends it only as its header and clears it', async () => {
    mocks.browser = true;
    const user = userEvent.setup();
    render(<Page />);
    await user.click(chip());
    await screen.findByText(RESULTS.folder);
    // A browser's grants live as long as the backend, not the tab.
    expect(card()).toHaveTextContent('backend restarts');
    expect(card()).not.toHaveTextContent('desktop app quits');
    const key = screen.getByLabelText(FOLDER_ACCESS_COPY.approvalKey);
    expect(key).toHaveAttribute('type', 'password');
    expect(key).toHaveAttribute('autocomplete', 'off');
    expect(key).toHaveAccessibleName(FOLDER_ACCESS_COPY.approvalKey);
    expect(key).toHaveAccessibleDescription(/Use the separate key configured with/);
    const revoke = screen.getByRole('button', {
      name: FOLDER_ACCESS_COPY.revokeLabel(RESULTS.folder),
    });
    expect(revoke).toBeDisabled();

    await user.type(key, 'synthetic-folder-key');
    expect(revoke).toBeEnabled();
    mocks.revoke.mockResolvedValueOnce({ error: 'refused' });
    await user.click(revoke);
    await waitFor(() => expect(mocks.revoke).toHaveBeenCalledTimes(1));
    const call = mocks.revoke.mock.calls[0][0];
    expect(call.headers).toEqual({ ...PROOF, 'X-Folder-Access-Key': 'synthetic-folder-key' });
    expect(JSON.stringify(call.body)).not.toContain('synthetic-folder-key');
    // Cleared after the attempt, even a refused one, so Revoke needs the key again.
    await waitFor(() => expect(key).toHaveValue(''));
    expect(revoke).toBeDisabled();
  });

  it('shows no key field in a browser while there is nothing to revoke', async () => {
    mocks.browser = true;
    mocks.list.mockResolvedValue({ data: [] });
    const user = userEvent.setup();
    render(<Page />);
    await user.click(chip());
    await screen.findByText(FOLDER_ACCESS_COPY.none);
    expect(screen.queryByLabelText(FOLDER_ACCESS_COPY.approvalKey)).toBeNull();
  });

  it('clears the browser key when the card closes', async () => {
    mocks.browser = true;
    const user = userEvent.setup();
    render(<Page />);
    await user.click(chip());
    await screen.findByText(RESULTS.folder);
    await user.type(screen.getByLabelText(FOLDER_ACCESS_COPY.approvalKey), 'typed-then-left');
    await user.keyboard('{Escape}');
    await user.click(chip());
    await screen.findByText(RESULTS.folder);
    expect(screen.getByLabelText(FOLDER_ACCESS_COPY.approvalKey)).toHaveValue('');
  });
});

describe('one chat at a time', () => {
  it('drops the old chat’s grants when the chip moves to another chat, and its late answer too', async () => {
    let answerA: ((value: unknown) => void) | undefined;
    mocks.list.mockImplementation(({ query }: { query: { session_id: string } }) =>
      query.session_id === 'chat-a'
        ? new Promise((resolve) => {
            answerA = resolve;
          })
        : Promise.resolve({ data: [NOTES] })
    );
    const user = userEvent.setup();
    const view = render(<Page />);
    await user.click(chip());
    expect(mocks.list).toHaveBeenLastCalledWith({
      query: { session_id: 'chat-a' },
      headers: PROOF,
    });

    view.rerender(<Page sessionId="chat-b" />);
    expect(await screen.findByText(NOTES.folder)).toBeInTheDocument();
    expect(mocks.list).toHaveBeenLastCalledWith({
      query: { session_id: 'chat-b' },
      headers: PROOF,
    });

    // Chat A's answer arrives after the switch: it is not shown under chat B.
    await act(async () => answerA?.({ data: [RESULTS] }));
    expect(screen.queryByText(RESULTS.folder)).toBeNull();

    // And Revoke acts on chat B.
    await user.click(
      screen.getByRole('button', { name: FOLDER_ACCESS_COPY.revokeLabel(NOTES.folder) })
    );
    await waitFor(() =>
      expect(mocks.revoke).toHaveBeenCalledWith({
        body: { sessionId: 'chat-b', grantId: 'grant-2' },
        headers: PROOF,
      })
    );
  });

  it('never shows one chat’s list under another while the new read runs', async () => {
    const user = userEvent.setup();
    const view = render(<Page />);
    await user.click(chip());
    await screen.findByText(RESULTS.folder);

    mocks.list.mockImplementationOnce(() => new Promise(() => {}));
    view.rerender(<Page sessionId="chat-b" />);
    await flush();
    expect(screen.queryByText(RESULTS.folder)).toBeNull();
    expect(screen.getByRole('status')).toHaveTextContent(FOLDER_ACCESS_COPY.loading);
  });
});
