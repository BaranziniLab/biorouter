import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import ResetPanel from './ResetPanel';
import { RESET_NEEDS_HOST_REASON } from './resetOnBrowser';
import { resetCopy } from './copy';
import { BROWSER_SURFACE_MARKER } from '../../../utils/surface';

const mocks = vi.hoisted(() => ({
  previewReset: vi.fn(),
  resetAppData: vi.fn(),
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
  userActionHeaders: vi.fn(),
}));

vi.mock('../../../api', () => ({
  previewReset: mocks.previewReset,
  resetAppData: mocks.resetAppData,
}));

vi.mock('../../../toasts', () => ({
  toastService: {
    success: mocks.toastSuccess,
    error: mocks.toastError,
  },
}));

vi.mock('../../../utils/userAction', () => ({
  userActionHeaders: mocks.userActionHeaders,
}));

/** What the desktop's `userActionHeaders()` resolves to: the person's proof. */
const PROOF = { 'X-User-Action': 'desktop-user-action-key' };

/** Open the dialog from the Danger zone row; the row's button is named by the row label. */
function openDialog() {
  fireEvent.click(screen.getByRole('button', { name: resetCopy.row }));
  return screen.getByRole('dialog');
}

describe('ResetPanel', () => {
  afterEach(() => {
    delete document.documentElement.dataset.biorouterSurface;
  });

  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    mocks.userActionHeaders.mockResolvedValue(PROOF);
    mocks.previewReset.mockResolvedValue({
      data: {
        counts: {
          applications: 2,
          knowledgeBases: 3,
          skills: 4,
          extensions: 1,
          schedules: 5,
          workflows: 6,
          conversations: 12,
        },
      },
    });
    mocks.resetAppData.mockResolvedValue({ data: { reset: [], removed: {} } });
  });

  /**
   * The page shows ONE row and one small destructive button (Crew's danger zone). The
   * checklist, the counts and the consequence live in the dialog, at the moment of decision.
   */
  it('puts one row on the page and the whole decision in the dialog', async () => {
    render(<ResetPanel />);

    expect(screen.getByRole('heading', { name: resetCopy.section })).toBeInTheDocument();
    expect(screen.queryByRole('checkbox')).toBeNull();
    expect(mocks.previewReset).not.toHaveBeenCalled();

    const dialog = openDialog();
    expect(dialog).toHaveTextContent(resetCopy.dialogTitle);
    expect(dialog).toHaveTextContent(resetCopy.permanence);
    expect(dialog).toHaveTextContent('Models, credentials and preferences are kept');
    expect(await within(dialog).findByText('2 built')).toBeInTheDocument();
    expect(within(dialog).getAllByRole('checkbox')).toHaveLength(7);
  });

  it('keeps each category’s meaning as help its checkbox hears', async () => {
    render(<ResetPanel />);
    const dialog = openDialog();
    const apps = within(dialog).getByRole('checkbox', { name: 'Built apps' });
    expect(within(dialog).getByRole('button', { name: 'About Built apps' })).toBeInTheDocument();
    // The sentence is not on the page as a paragraph: only the hidden description carries it.
    expect(within(dialog).getByText('Delete every app created with Agent Drafter.')).toHaveClass(
      'sr-only'
    );
    expect(apps).not.toBeChecked();
  });

  it('requires a deliberate selection, and reads "Reset everything" only when all are checked', async () => {
    render(<ResetPanel />);
    const dialog = openDialog();
    await within(dialog).findByText('12 chats');

    const confirm = within(dialog).getByRole('button', { name: resetCopy.resetSelected });
    expect(confirm).toBeDisabled();

    fireEvent.click(within(dialog).getByRole('checkbox', { name: 'Built apps' }));
    expect(confirm).toBeEnabled();
    expect(within(dialog).getByText(resetCopy.selectedCount(1, 7))).toBeInTheDocument();

    fireEvent.click(within(dialog).getByRole('button', { name: resetCopy.selectAll }));
    expect(within(dialog).getByRole('button', { name: resetCopy.resetEverything })).toBeEnabled();
    fireEvent.click(within(dialog).getByRole('button', { name: resetCopy.clear }));
    expect(within(dialog).getByRole('button', { name: resetCopy.resetSelected })).toBeDisabled();
  });

  it('submits only the selected categories', async () => {
    const onReset = vi.fn();
    render(<ResetPanel onReset={onReset} />);
    const dialog = openDialog();
    await within(dialog).findByText('12 chats');

    fireEvent.click(within(dialog).getByRole('checkbox', { name: 'Chat & usage history' }));
    fireEvent.click(within(dialog).getByRole('button', { name: resetCopy.resetSelected }));

    await waitFor(() => {
      expect(mocks.resetAppData).toHaveBeenCalledWith({
        body: { categories: ['history'] },
        headers: PROOF,
        throwOnError: true,
      });
    });
    await waitFor(() => expect(onReset).toHaveBeenCalledWith(['history']));
    expect(await screen.findByText(resetCopy.complete)).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(mocks.toastSuccess).toHaveBeenCalledWith({
      title: resetCopy.completeToastTitle,
      msg: resetCopy.completeToast(1),
    });
  });

  /**
   * The daemon answers `GET /reset/preview` and `POST /reset` only for a request
   * carrying the person's proof. A caller holding just the daemon secret had
   * been able to empty History, private chats included. Without the header the
   * desktop's own panel would show no counts and refuse every reset.
   */
  it('sends the user-action proof on the preview and on the reset', async () => {
    render(<ResetPanel />);
    const dialog = openDialog();
    await within(dialog).findByText('12 chats');
    expect(mocks.previewReset).toHaveBeenCalledWith({ headers: PROOF, throwOnError: true });

    fireEvent.click(within(dialog).getByRole('checkbox', { name: 'Workflows' }));
    fireEvent.click(within(dialog).getByRole('button', { name: resetCopy.resetSelected }));

    await waitFor(() => expect(mocks.resetAppData).toHaveBeenCalledTimes(1));
    expect(mocks.resetAppData.mock.calls[0][0].headers).toEqual(PROOF);
    for (const call of mocks.previewReset.mock.calls) {
      expect(call[0].headers).toEqual(PROOF);
    }
  });

  /**
   * Under `throwOnError` the client throws the parsed JSON body, not an `Error`,
   * so the refusal's own sentence has to be read off `message`. It used to be
   * replaced with the generic fallback.
   */
  it("shows the daemon's own sentence when a reset is refused, in the dialog that caused it", async () => {
    const refusal =
      'This daemon was started without a user-action key, so it cannot verify that a request ' +
      'came from the person at the keyboard.';
    mocks.resetAppData.mockRejectedValueOnce({ message: refusal });
    render(<ResetPanel />);
    const dialog = openDialog();
    await within(dialog).findByText('12 chats');

    fireEvent.click(within(dialog).getByRole('button', { name: resetCopy.selectAll }));
    fireEvent.click(within(dialog).getByRole('button', { name: resetCopy.resetEverything }));

    await waitFor(() =>
      expect(mocks.toastError).toHaveBeenCalledWith({
        title: resetCopy.failedToastTitle,
        msg: refusal,
      })
    );
    expect(within(dialog).getByRole('alert')).toHaveTextContent(refusal);
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
  });

  /**
   * SD-8 on `biorouter serve`: that daemon holds no key, so it refuses every
   * reset. The row says so in place of a working control, before the person can
   * select, confirm, and only then read a refusal, and it never asks for the
   * preview it would be refused.
   */
  it('explains before the click on a browser-served page, and never asks', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    render(<ResetPanel />);

    expect(await screen.findByTestId('reset-needs-host-note')).toHaveTextContent(
      RESET_NEEDS_HOST_REASON
    );
    const open = screen.getByRole('button', { name: resetCopy.row });
    expect(open).toBeDisabled();
    fireEvent.click(open);
    expect(screen.queryByRole('dialog')).toBeNull();

    expect(mocks.previewReset).not.toHaveBeenCalled();
    expect(mocks.resetAppData).not.toHaveBeenCalled();
    expect(mocks.userActionHeaders).not.toHaveBeenCalled();
  });
});
