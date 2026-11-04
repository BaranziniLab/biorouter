import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ComponentProps } from 'react';

/**
 * The working folder's card, driven through the real chip (`DirSwitcher`), which owns the
 * Popover root and its anchor. It replaced the footer's "Folder access" button and dialog, so
 * every behaviour `FolderAccess.test.tsx` pinned lives here now: grants load only when the card
 * opens, Revoke acts on the selected grant in its own chat with the proof header, a refused
 * revoke stays visible, and the browser's approval key is labelled, described, sent only as
 * `X-Folder-Access-Key` and cleared after every attempt.
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
import {
  FOLDER_CARD_CLOSE_GRACE_MS,
  FOLDER_CARD_OPEN_DELAY_MS,
  folderCardAnchorRect,
  isPointInPolygon,
  pointerGraceArea,
} from './useFolderCard';

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
      <div data-testid="holds-chip">
        <button type="button">Before</button>
        <DirSwitcher sessionId="chat-a" workingDir={WORKING_DIR} locked {...props} />
        <button type="button">Outside</button>
      </div>
      <div data-testid="beside">
        <p>Plain text</p>
      </div>
    </div>
  );
}

const card = () => document.querySelector<HTMLElement>('[data-testid="folder-access-card"]');
const isOpen = () => card()?.getAttribute('data-state') === 'open';
const lockedChip = () => screen.getByTestId('dir-switcher-locked');
const unlockedChip = () => screen.getByTestId('dir-switcher');

/** Let the grant read (a resolved mock) land inside act. */
const flush = () => act(async () => {});

function hoverOpen(chip: HTMLElement) {
  fireEvent.pointerEnter(chip, { pointerType: 'mouse' });
  act(() => {
    vi.advanceTimersByTime(FOLDER_CARD_OPEN_DELAY_MS + 10);
  });
}

describe('opening and closing', () => {
  it('opens on hover after the delay, not before', async () => {
    vi.useFakeTimers();
    render(<Page />);
    fireEvent.pointerEnter(lockedChip(), { pointerType: 'mouse' });
    act(() => {
      vi.advanceTimersByTime(FOLDER_CARD_OPEN_DELAY_MS - 10);
    });
    expect(isOpen()).toBe(false);
    act(() => {
      vi.advanceTimersByTime(20);
    });
    expect(isOpen()).toBe(true);
    expect(screen.getByRole('dialog', { name: FOLDER_ACCESS_COPY.cardLabel })).toBe(card());
    await flush();
  });

  it('stays open while the pointer crosses into the card, and closes when it leaves both', async () => {
    vi.useFakeTimers();
    render(<Page />);
    hoverOpen(lockedChip());
    await flush();
    fireEvent.pointerLeave(lockedChip(), { pointerType: 'mouse' });
    fireEvent.pointerEnter(card()!, { pointerType: 'mouse' });
    act(() => {
      vi.advanceTimersByTime(FOLDER_CARD_CLOSE_GRACE_MS * 4);
    });
    expect(isOpen()).toBe(true);

    fireEvent.pointerLeave(card()!, { pointerType: 'mouse' });
    act(() => {
      vi.advanceTimersByTime(FOLDER_CARD_CLOSE_GRACE_MS - 10);
    });
    expect(isOpen()).toBe(true);
    act(() => {
      vi.advanceTimersByTime(20);
    });
    expect(isOpen()).toBe(false);
  });

  it('never opens on a touch hover', () => {
    vi.useFakeTimers();
    render(<Page />);
    fireEvent.pointerEnter(lockedChip(), { pointerType: 'touch' });
    act(() => {
      vi.advanceTimersByTime(FOLDER_CARD_OPEN_DELAY_MS * 3);
    });
    expect(isOpen()).toBe(false);
    expect(mocks.list).not.toHaveBeenCalled();
  });

  it('opens at once when Tab moves focus onto the chip, and closes when Tab moves on', async () => {
    const user = userEvent.setup();
    render(<Page />);
    act(() => screen.getByRole('button', { name: 'Before' }).focus());
    await user.tab();
    expect(lockedChip()).toHaveFocus();
    expect(isOpen()).toBe(true);
    // Focus stays on the chip: a hover card does not take it.
    expect(lockedChip()).toHaveFocus();
    await user.tab();
    expect(isOpen()).toBe(false);
  });

  it('stays shut when a program focuses the chip (a focus restore)', async () => {
    render(<Page />);
    act(() => lockedChip().focus());
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    expect(isOpen()).toBe(false);
    expect(mocks.list).not.toHaveBeenCalled();
  });

  it('closes on Escape', async () => {
    const user = userEvent.setup();
    render(<Page />);
    act(() => screen.getByRole('button', { name: 'Before' }).focus());
    await user.tab();
    expect(isOpen()).toBe(true);
    await user.keyboard('{Escape}');
    expect(isOpen()).toBe(false);
  });

  it('closes on a press outside, but a press on the chip is not outside', async () => {
    const user = userEvent.setup();
    render(<Page />);
    await user.click(lockedChip());
    expect(isOpen()).toBe(true);
    await user.click(screen.getByRole('button', { name: 'Outside' }));
    expect(isOpen()).toBe(false);
  });

  it('leaves an unpinned card open for a scroll beside the chip or inside the card', async () => {
    vi.useFakeTimers();
    render(<Page />);
    hoverOpen(lockedChip());
    await flush();
    // A scroller that does not hold the chip (the transcript streaming a reply, say) and the
    // card's own list do not move the chip.
    act(() => {
      fireEvent.scroll(screen.getByTestId('beside'));
    });
    expect(isOpen()).toBe(true);
    act(() => {
      fireEvent.scroll(card()!);
    });
    expect(isOpen()).toBe(true);
    // One that holds the chip does.
    act(() => {
      fireEvent.scroll(screen.getByTestId('holds-chip'));
    });
    expect(isOpen()).toBe(false);
  });

  it('closes an unpinned card on a scroll that moves the chip, and keeps a pinned one', async () => {
    vi.useFakeTimers();
    render(<Page />);
    hoverOpen(lockedChip());
    await flush();
    act(() => {
      fireEvent.scroll(window);
    });
    expect(isOpen()).toBe(false);

    vi.useRealTimers();
    fireEvent.click(lockedChip(), { detail: 1 });
    await flush();
    expect(isOpen()).toBe(true);
    act(() => {
      fireEvent.scroll(window);
    });
    expect(isOpen()).toBe(true);
  });
});

describe('where focus goes when the card closes', () => {
  async function pinFromKeyboard(user: ReturnType<typeof userEvent.setup>) {
    act(() => lockedChip().focus());
    await user.keyboard('{Enter}');
    await waitFor(() => expect(card()).toHaveFocus());
    await screen.findByText(RESULTS.folder);
  }

  it('sends Shift+Tab off the card’s own surface back to the chip, and closes the card', async () => {
    const user = userEvent.setup();
    render(<Page />);
    await pinFromKeyboard(user);
    await user.keyboard('{Shift>}{Tab}{/Shift}');
    expect(lockedChip()).toHaveFocus();
    expect(isOpen()).toBe(false);
    // Tab was down when focus came back, yet that focus is the card's own restore: it opens
    // nothing.
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    expect(isOpen()).toBe(false);
  });

  it('does the same after a press on the card’s background put focus on its surface', async () => {
    const user = userEvent.setup();
    render(<Page />);
    await user.click(lockedChip());
    await screen.findByText(RESULTS.folder);
    await user.click(card()!);
    expect(card()).toHaveFocus();
    await user.keyboard('{Shift>}{Tab}{/Shift}');
    expect(lockedChip()).toHaveFocus();
    expect(isOpen()).toBe(false);
  });

  it('wraps Shift+Tab from the first Revoke inside the card, as Radix does', async () => {
    mocks.list.mockResolvedValue({ data: [RESULTS, NOTES] });
    const user = userEvent.setup();
    render(<Page />);
    await pinFromKeyboard(user);
    await user.tab();
    const first = screen.getByRole('button', {
      name: FOLDER_ACCESS_COPY.revokeLabel(RESULTS.folder),
    });
    expect(first).toHaveFocus();
    await user.keyboard('{Shift>}{Tab}{/Shift}');
    expect(
      screen.getByRole('button', { name: FOLDER_ACCESS_COPY.revokeLabel(NOTES.folder) })
    ).toHaveFocus();
    expect(isOpen()).toBe(true);
  });

  it('leaves focus to an outside press, never handing it to the chip mid-click', async () => {
    const user = userEvent.setup();
    render(<Page />);
    await pinFromKeyboard(user);
    // A chip focus here would fire every focus handler around the chip (a split pane's
    // onFocusCapture makes its pane the active one) during a press meant for somewhere else.
    const chipFocused = vi.fn();
    lockedChip().addEventListener('focus', chipFocused);
    await user.click(screen.getByText('Plain text'));
    expect(isOpen()).toBe(false);
    expect(chipFocused).not.toHaveBeenCalled();
    expect(lockedChip()).not.toHaveFocus();
  });

  it('still hands focus back to the chip on Escape and on the chip’s own click', async () => {
    const user = userEvent.setup();
    render(<Page />);
    await pinFromKeyboard(user);
    await user.keyboard('{Escape}');
    expect(isOpen()).toBe(false);
    expect(lockedChip()).toHaveFocus();

    // A keyboard pin again, then the chip's own click (the press lands on the chip, which is not
    // outside) closes it, with focus on the chip.
    await user.keyboard('{Enter}');
    await waitFor(() => expect(card()).toHaveFocus());
    await user.click(lockedChip());
    expect(isOpen()).toBe(false);
    expect(lockedChip()).toHaveFocus();
  });
});

describe('the locked chip (#44)', () => {
  it('is a real button that pins the card and never opens the chooser', async () => {
    const onWorkingDirChange = vi.fn();
    const user = userEvent.setup();
    render(<Page onWorkingDirChange={onWorkingDirChange} />);
    const chip = lockedChip();
    expect(chip.tagName).toBe('BUTTON');
    expect(chip).toHaveAttribute('type', 'button');
    expect(chip).toHaveAccessibleName(FOOTER_COPY.folderLabel(WORKING_DIR));
    expect(chip).toHaveAttribute('aria-haspopup', 'dialog');
    expect(chip).toHaveAttribute('aria-expanded', 'false');

    await user.click(chip);
    expect(isOpen()).toBe(true);
    expect(chip).toHaveAttribute('aria-expanded', 'true');
    expect(chip).toHaveAttribute('aria-controls', card()!.id);

    // Pinned: the pointer wandering off does not close it.
    await user.unhover(chip);
    await act(() => new Promise((resolve) => setTimeout(resolve, FOLDER_CARD_CLOSE_GRACE_MS * 2)));
    expect(isOpen()).toBe(true);

    // A second click closes it.
    await user.click(chip);
    expect(isOpen()).toBe(false);
    expect(window.electron.directoryChooser).not.toHaveBeenCalled();
    expect(mocks.update).not.toHaveBeenCalled();
    expect(onWorkingDirChange).not.toHaveBeenCalled();
  });

  it('pins on Enter and moves focus into the card, so Revoke is reachable by keyboard', async () => {
    const user = userEvent.setup();
    render(<Page />);
    act(() => lockedChip().focus());
    await user.keyboard('{Enter}');
    expect(isOpen()).toBe(true);
    await waitFor(() => expect(card()).toHaveFocus());
    await screen.findByText(RESULTS.folder);
    await user.tab();
    expect(
      screen.getByRole('button', { name: FOLDER_ACCESS_COPY.revokeLabel(RESULTS.folder) })
    ).toHaveFocus();
    // Escape hands focus back to the chip, and that restore does not reopen the card.
    await user.keyboard('{Escape}');
    expect(isOpen()).toBe(false);
    expect(lockedChip()).toHaveFocus();
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    expect(isOpen()).toBe(false);
    expect(window.electron.directoryChooser).not.toHaveBeenCalled();
  });

  it('pins a card Tab already opened and moves focus into it on Enter', async () => {
    const user = userEvent.setup();
    render(<Page />);
    act(() => screen.getByRole('button', { name: 'Before' }).focus());
    await user.tab();
    expect(isOpen()).toBe(true);
    expect(lockedChip()).toHaveFocus();
    await user.keyboard('{Enter}');
    expect(isOpen()).toBe(true);
    expect(card()).toHaveFocus();
    // Pinned now: Tab stays in the card instead of closing it.
    await screen.findByText(RESULTS.folder);
    await user.tab();
    expect(
      screen.getByRole('button', { name: FOLDER_ACCESS_COPY.revokeLabel(RESULTS.folder) })
    ).toHaveFocus();
    expect(isOpen()).toBe(true);
  });

  it('pins a card a hover opened instead of closing it, and a press inside pins it too', async () => {
    vi.useFakeTimers();
    render(<Page />);
    hoverOpen(lockedChip());
    await flush();
    fireEvent.pointerDown(card()!);
    fireEvent.pointerLeave(lockedChip(), { pointerType: 'mouse' });
    act(() => {
      vi.advanceTimersByTime(FOLDER_CARD_CLOSE_GRACE_MS * 4);
    });
    expect(isOpen()).toBe(true);
    fireEvent.pointerUp(card()!);
  });

  it('says the folder is fixed', async () => {
    const user = userEvent.setup();
    render(<Page />);
    await user.click(lockedChip());
    expect(card()).toHaveTextContent(FOLDER_ACCESS_COPY.lockedHint);
    expect(card()).not.toHaveTextContent(FOLDER_ACCESS_COPY.chooseHint);
    await flush();
  });
});

describe('the unlocked chip', () => {
  it('keeps the chooser on click, closes the card and keeps it shut while the chooser is open', async () => {
    vi.useFakeTimers();
    let answer: ((value: { canceled: boolean; filePaths: string[] }) => void) | undefined;
    window.electron.directoryChooser = vi.fn(
      () =>
        new Promise((resolve) => {
          answer = resolve;
        })
    ) as typeof window.electron.directoryChooser;
    render(<Page locked={false} sessionId={undefined} />);
    hoverOpen(unlockedChip());
    expect(isOpen()).toBe(true);
    expect(card()).toHaveTextContent(FOLDER_ACCESS_COPY.chooseHint);

    fireEvent.click(unlockedChip());
    expect(window.electron.directoryChooser).toHaveBeenCalledTimes(1);
    expect(isOpen()).toBe(false);

    // The native chooser is up: a hover or a focus must not bring the card back over it.
    hoverOpen(unlockedChip());
    expect(isOpen()).toBe(false);

    await act(async () => answer?.({ canceled: true, filePaths: [] }));
    expect(unlockedChip()).toBeEnabled();
    // #50: the hover during the chooser armed nothing. The card does not pop open now that the
    // chooser is gone, nor once any open delay has run out.
    expect(isOpen()).toBe(false);
    act(() => {
      vi.advanceTimersByTime(FOLDER_CARD_OPEN_DELAY_MS * 2);
    });
    expect(isOpen()).toBe(false);
    expect(mocks.list).not.toHaveBeenCalled();
  });

  it('opens again on a fresh hover once the chooser has closed', async () => {
    vi.useFakeTimers();
    render(<Page locked={false} sessionId={undefined} />);
    fireEvent.click(unlockedChip());
    await flush();
    expect(unlockedChip()).toBeEnabled();
    fireEvent.pointerLeave(unlockedChip(), { pointerType: 'mouse' });
    hoverOpen(unlockedChip());
    expect(isOpen()).toBe(true);
  });

  it('opens the folder in the file manager on Cmd-click', () => {
    render(<Page locked={false} />);
    fireEvent.click(unlockedChip(), { metaKey: true });
    expect(window.electron.openDirectoryInExplorer).toHaveBeenCalledWith(WORKING_DIR);
    expect(window.electron.directoryChooser).not.toHaveBeenCalled();
  });
});

describe('a chat whose working folder is not known yet', () => {
  const unknownChip = () => screen.getByTestId('dir-switcher-unknown');

  it('keeps a nameless chip whose card lists and revokes the chat’s grants', async () => {
    const user = userEvent.setup();
    render(<Page workingDir={null} />);
    const chip = unknownChip();
    expect(chip.tagName).toBe('BUTTON');
    expect(chip).toHaveAccessibleName(FOOTER_COPY.folderUnknownLabel);
    expect(chip.querySelector('.br-footline__name')).toBeNull();
    await user.click(chip);
    const dialog = screen.getByRole('dialog', { name: FOLDER_ACCESS_COPY.cardLabel });
    expect(dialog).toHaveTextContent(FOLDER_ACCESS_COPY.workingFolderUnknown);
    expect(dialog).not.toHaveTextContent(FOLDER_ACCESS_COPY.workingFolderNote);
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

  it('never opens the chooser, even before the chat has messages, and promises no click', async () => {
    const user = userEvent.setup();
    render(<Page workingDir={null} locked={false} />);
    await user.click(unknownChip());
    expect(isOpen()).toBe(true);
    expect(card()).not.toHaveTextContent(FOLDER_ACCESS_COPY.chooseHint);
    expect(card()).not.toHaveTextContent(FOLDER_ACCESS_COPY.lockedHint);
    fireEvent.click(unknownChip(), { metaKey: true });
    expect(window.electron.directoryChooser).not.toHaveBeenCalled();
    expect(window.electron.openDirectoryInExplorer).not.toHaveBeenCalled();
    expect(mocks.update).not.toHaveBeenCalled();
    await flush();
  });
});

describe('what the card says', () => {
  it('shows the full working folder in mono, and the chip only its name', async () => {
    const user = userEvent.setup();
    render(<Page />);
    expect(lockedChip()).toHaveTextContent('kdps');
    expect(lockedChip()).not.toHaveTextContent(WORKING_DIR);
    await user.click(lockedChip());
    const dialog = screen.getByRole('dialog', { name: FOLDER_ACCESS_COPY.cardLabel });
    expect(dialog).toHaveTextContent(FOLDER_ACCESS_COPY.workingFolder);
    expect(dialog).toHaveTextContent(FOLDER_ACCESS_COPY.workingFolderNote);
    const path = within(dialog).getByText(WORKING_DIR);
    expect(path).toHaveClass('font-mono');
    expect(path).toHaveClass('break-all');
    await flush();
  });

  it('says which tools ask for folder approval, and never that everything asks', async () => {
    // Folder approval gates Biorouter's built-in file tools only (`file_access_plan`); a shell
    // command reads another folder without one. So no sentence may say Biorouter as a whole asks.
    for (const line of [
      FOLDER_ACCESS_COPY.none,
      FOLDER_ACCESS_COPY.noChatYet,
      FOLDER_ACCESS_COPY.workingFolderNote,
    ]) {
      expect(line).toMatch(/Built-in file tools/);
      expect(line).not.toMatch(/Biorouter (asks|needs)/);
    }
    expect(FOLDER_ACCESS_COPY.workingFolderNote).toMatch(/Shell commands never ask/);
    mocks.list.mockResolvedValue({ data: [] });
    const user = userEvent.setup();
    render(<Page />);
    await user.click(lockedChip());
    expect(await screen.findByText(FOLDER_ACCESS_COPY.none)).toBeInTheDocument();
    expect(card()).toHaveTextContent(FOLDER_ACCESS_COPY.workingFolderNote);
  });

  it('reads nothing on Home and says that any other folder is asked for', async () => {
    vi.useFakeTimers();
    render(<Page sessionId={undefined} locked={false} />);
    hoverOpen(unlockedChip());
    await flush();
    expect(isOpen()).toBe(true);
    expect(card()).toHaveTextContent(WORKING_DIR);
    expect(card()).toHaveTextContent(FOLDER_ACCESS_COPY.noChatYet);
    expect(card()).toHaveTextContent(FOLDER_ACCESS_COPY.chooseHint);
    expect(card()).not.toHaveTextContent(FOLDER_ACCESS_COPY.otherFolders);
    expect(mocks.list).not.toHaveBeenCalled();
  });

  it('reads the grants only once the card opens, with the proof header', async () => {
    vi.useFakeTimers();
    render(<Page />);
    expect(mocks.list).not.toHaveBeenCalled();
    fireEvent.pointerEnter(lockedChip(), { pointerType: 'mouse' });
    act(() => {
      vi.advanceTimersByTime(FOLDER_CARD_OPEN_DELAY_MS - 10);
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
    await user.click(lockedChip());
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
    await user.click(lockedChip());
    expect(screen.getByRole('status')).toHaveTextContent(FOLDER_ACCESS_COPY.loading);
    await act(async () => answer?.({ data: [RESULTS] }));
    expect(screen.getByText(RESULTS.folder)).toBeInTheDocument();
    await user.click(lockedChip());
    expect(isOpen()).toBe(false);

    // Reopened: the read runs again, and the last list shows while it does.
    mocks.list.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          answer = resolve;
        })
    );
    await user.click(lockedChip());
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
    await user.click(lockedChip());
    expect(await screen.findByText(FOLDER_ACCESS_COPY.none)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Revoke/ })).toBeNull();
  });

  it('says so when the read fails', async () => {
    mocks.list.mockResolvedValue({ error: 'refused' });
    const user = userEvent.setup();
    render(<Page />);
    await user.click(lockedChip());
    expect(await screen.findByRole('alert')).toHaveTextContent(FOLDER_ACCESS_COPY.loadFailed);
    expect(screen.queryByRole('list')).toBeNull();
  });

  it('states the desktop lifetime only for the managed desktop backend', async () => {
    Object.assign(window, {
      appConfig: { get: (key: string) => (key === 'BIOROUTER_LOCAL_BACKEND' ? true : undefined) },
    });
    const user = userEvent.setup();
    render(<Page />);
    await user.click(lockedChip());
    await screen.findByText(RESULTS.folder);
    expect(card()).toHaveTextContent('desktop app quits');
  });
});

describe('revoking (the only revoke control in the interface)', () => {
  it('revokes only the selected grant, in its own chat, with the proof header, then rereads', async () => {
    mocks.list.mockResolvedValueOnce({ data: [RESULTS, NOTES] });
    const user = userEvent.setup();
    render(<Page />);
    await user.click(lockedChip());
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
    await user.click(lockedChip());
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
    await user.click(lockedChip());
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
    await user.click(lockedChip());
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
    await user.click(lockedChip());
    await screen.findByText(FOLDER_ACCESS_COPY.none);
    expect(screen.queryByLabelText(FOLDER_ACCESS_COPY.approvalKey)).toBeNull();
  });

  it('clears the browser key when the card closes', async () => {
    mocks.browser = true;
    const user = userEvent.setup();
    render(<Page />);
    await user.click(lockedChip());
    await screen.findByText(RESULTS.folder);
    await user.type(screen.getByLabelText(FOLDER_ACCESS_COPY.approvalKey), 'typed-then-left');
    await user.keyboard('{Escape}');
    await user.click(lockedChip());
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
    await user.click(lockedChip());
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
    await user.click(lockedChip());
    await screen.findByText(RESULTS.folder);

    mocks.list.mockImplementationOnce(() => new Promise(() => {}));
    view.rerender(<Page sessionId="chat-b" />);
    await flush();
    expect(screen.queryByText(RESULTS.folder)).toBeNull();
    expect(screen.getByRole('status')).toHaveTextContent(FOLDER_ACCESS_COPY.loading);
  });
});

describe('where the card hangs, and the pointer’s way to it', () => {
  it('hangs from the chip stretched up to the composer’s top, keeping the chip’s left and width', () => {
    const chip = new DOMRect(40, 560, 80, 20);
    const composer = new DOMRect(24, 440, 700, 110);
    const anchor = folderCardAnchorRect(chip, composer);
    expect(anchor.left).toBe(40);
    expect(anchor.width).toBe(80);
    expect(anchor.top).toBe(440);
    expect(anchor.bottom).toBe(580);
    // No composer, or one with no layout yet: the chip itself.
    expect(folderCardAnchorRect(chip, null)).toBe(chip);
    expect(folderCardAnchorRect(chip, new DOMRect())).toBe(chip);
  });

  it('counts a pointer between the chip and the card as on its way, and one beside it as not', () => {
    // The chip at the bottom, the card above the composer.
    const exit = { x: 60, y: 560 };
    const cardBox = { top: 160, right: 360, bottom: 430, left: 40 };
    const area = pointerGraceArea(exit, cardBox);
    // Straight up across the composer, and into the card.
    expect(isPointInPolygon({ x: 62, y: 500 }, area)).toBe(true);
    expect(isPointInPolygon({ x: 120, y: 440 }, area)).toBe(true);
    expect(isPointInPolygon({ x: 200, y: 300 }, area)).toBe(true);
    // Off along the composer's text, or below the chip.
    expect(isPointInPolygon({ x: 600, y: 520 }, area)).toBe(false);
    expect(isPointInPolygon({ x: 340, y: 550 }, area)).toBe(false);
    expect(isPointInPolygon({ x: 60, y: 600 }, area)).toBe(false);
  });
});
