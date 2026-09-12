import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { diagnostics, getSession } from '../../api';
import { toastError, toastSuccess } from '../../toasts';
import { userActionHeaders } from '../../utils/userAction';
import { DiagnosticsModal } from './Diagnostics';

vi.mock('../../api', () => ({
  diagnostics: vi.fn(),
  getSession: vi.fn(),
  systemInfo: vi.fn(),
}));

vi.mock('../../toasts', () => ({
  toastError: vi.fn(),
  toastSuccess: vi.fn(),
}));

vi.mock('../../utils/userAction', () => ({
  userActionHeaders: vi.fn(),
}));

const diagnosticsMock = vi.mocked(diagnostics);
const getSessionMock = vi.mocked(getSession);
const toastErrorMock = vi.mocked(toastError);
const toastSuccessMock = vi.mocked(toastSuccess);
const userActionHeadersMock = vi.mocked(userActionHeaders);
const saveDiagnosticsBundle = vi.fn();

const diagnosticsResponse = () => ({
  data: {
    arrayBuffer: vi.fn().mockResolvedValue(Uint8Array.from([0x50, 0x4b, 0x03, 0x04]).buffer),
  },
});

describe('DiagnosticsModal', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    (window as unknown as { electron: unknown }).electron = { saveDiagnosticsBundle };
    userActionHeadersMock.mockResolvedValue({ 'X-User-Action': 'proof-of-user' });
    diagnosticsMock.mockResolvedValue(diagnosticsResponse() as never);
    // Default: the daemon confirms whatever the caller seeded, so the existing
    // prop-driven assertions keep meaning what they did before the modal
    // started reading for itself.
    getSessionMock.mockResolvedValue({ data: {} } as unknown as Awaited<
      ReturnType<typeof getSession>
    >);
  });

  it('generates and saves the archive through the native diagnostics IPC', async () => {
    const onClose = vi.fn();
    saveDiagnosticsBundle.mockResolvedValue({
      canceled: false,
      filePath: '/Users/test/Downloads/diagnostics_20260716_27.zip',
    });

    render(<DiagnosticsModal isOpen onClose={onClose} sessionId="20260716_27" />);

    expect(screen.getByRole('dialog', { name: 'Report a problem' })).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Generate diagnostics' }));

    await waitFor(() => {
      expect(saveDiagnosticsBundle).toHaveBeenCalledWith('20260716_27', expect.any(ArrayBuffer));
    });
    expect(diagnosticsMock).toHaveBeenCalledWith({
      headers: { 'X-User-Action': 'proof-of-user' },
      path: { session_id: '20260716_27' },
      throwOnError: true,
    });
    expect(toastSuccessMock).toHaveBeenCalledWith({
      title: 'Diagnostics saved',
      msg: 'The diagnostics bundle was saved to /Users/test/Downloads/diagnostics_20260716_27.zip.',
    });
    expect(onClose).toHaveBeenCalledOnce();
  });

  it('keeps the dialog open and recovers its controls when saving is canceled', async () => {
    const onClose = vi.fn();
    let finishSave: ((value: { canceled: true }) => void) | undefined;
    saveDiagnosticsBundle.mockImplementation(
      () =>
        new Promise((resolve) => {
          finishSave = resolve;
        })
    );

    render(<DiagnosticsModal isOpen onClose={onClose} sessionId="20260716_27" />);
    fireEvent.click(screen.getByRole('button', { name: 'Generate diagnostics' }));

    expect(await screen.findByRole('status')).toHaveTextContent('Preparing diagnostics bundle');
    expect(screen.getByRole('button', { name: 'Generating...' })).toBeDisabled();

    finishSave?.({ canceled: true });

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Generate diagnostics' })).toBeEnabled();
    });
    expect(screen.getByRole('dialog', { name: 'Report a problem' })).toBeVisible();
    expect(onClose).not.toHaveBeenCalled();
    expect(toastSuccessMock).not.toHaveBeenCalled();
  });

  it('shows the native save failure without dismissing the dialog', async () => {
    const onClose = vi.fn();
    saveDiagnosticsBundle.mockResolvedValue({
      canceled: false,
      error: 'The diagnostics response is not a valid ZIP archive.',
    });

    render(<DiagnosticsModal isOpen onClose={onClose} sessionId="20260716_27" />);
    fireEvent.click(screen.getByRole('button', { name: 'Generate diagnostics' }));

    await waitFor(() => {
      expect(toastErrorMock).toHaveBeenCalledWith({
        title: 'Diagnostics error',
        msg: 'The diagnostics response is not a valid ZIP archive.',
      });
    });
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByRole('dialog', { name: 'Report a problem' })).toBeVisible();
  });

  /**
   * A private chat must still be able to produce a diagnostics bundle. Being
   * unable to report a bug is the worst possible consequence of a chat being
   * private: the person who most needs support is the one who cannot ask for
   * it, and the workaround is to reproduce the problem in a public chat, which
   * is exactly the thing they were avoiding.
   *
   * So the warning is a warning and nothing more. It does not disable the
   * button, it does not gate the download behind a confirmation, and this test
   * asserts the bundle is produced with the private warning on screen.
   */
  it('still generates the bundle for a private chat, and warns about the contents', async () => {
    render(
      <DiagnosticsModal isOpen onClose={vi.fn()} sessionId="20260716_27" privacyTier="private" />
    );

    const warning = screen.getByTestId('diagnostics-private-warning');
    expect(warning).toBeVisible();
    expect(warning.textContent).toContain('This chat is private.');
    // ⚠ The point of the sentence is what the reader must DO with the file.
    // A warning that only says "this is private" tells them something they
    // already know and nothing they can act on.
    expect(warning.textContent).toMatch(/read the file before you send it/i);

    const button = screen.getByRole('button', { name: 'Generate diagnostics' });
    expect(button).toBeEnabled();
    fireEvent.click(button);

    await waitFor(() => {
      expect(saveDiagnosticsBundle).toHaveBeenCalledWith('20260716_27', expect.any(ArrayBuffer));
    });
  });

  /**
   * ⚠ REGRESSION (found in the running app, not by a test). Every assertion
   * above hands the tier in as a prop, so all of them passed while the warning
   * was missing from every real private chat.
   *
   * `privacyTier` reaches this modal from the session `useChatStream` loaded
   * when the chat OPENED, and the classification ratchets to private DURING a
   * turn. So the case that matters most — a fresh chat that just became private
   * by talking to a private model — arrives here still carrying the pre-ratchet
   * value. The composer's chip was right beside it saying "Private chat",
   * because ChatInput re-reads the tier after each turn; this modal did not.
   *
   * The fix is that the modal asks the daemon itself when it opens. This test
   * is that fix's negative control: it hands in the STALE prop and requires the
   * warning anyway.
   */
  it('warns when the daemon says private even though the prop is a stale public', async () => {
    getSessionMock.mockResolvedValue({
      data: { privacy_tier: 'private' },
    } as unknown as Awaited<ReturnType<typeof getSession>>);

    render(
      <DiagnosticsModal isOpen onClose={vi.fn()} sessionId="20260716_27" privacyTier="public" />
    );

    const warning = await screen.findByTestId('diagnostics-private-warning');
    expect(warning).toBeVisible();
    expect(warning.textContent).toContain('This chat is private.');
  });

  it('keeps the seeded tier when the daemon read fails, inventing nothing', async () => {
    getSessionMock.mockRejectedValue(new Error('refused'));
    const error = vi.spyOn(console, 'error').mockImplementation(() => {});

    render(
      <DiagnosticsModal isOpen onClose={vi.fn()} sessionId="20260716_27" privacyTier="private" />
    );
    // A failed read must not erase a warning the caller already justified...
    expect(screen.getByTestId('diagnostics-private-warning')).toBeVisible();
    await waitFor(() => expect(error).toHaveBeenCalled());
    expect(screen.getByTestId('diagnostics-private-warning')).toBeVisible();
    error.mockRestore();
  });

  it('does not warn on a chat that is not private', () => {
    const { rerender } = render(
      <DiagnosticsModal isOpen onClose={vi.fn()} sessionId="20260716_27" privacyTier="public" />
    );
    expect(screen.queryByTestId('diagnostics-private-warning')).toBeNull();

    // An unknown tier reads as public. A warning shown on every chat is one
    // nobody reads by the third time, which costs the private case its warning.
    rerender(<DiagnosticsModal isOpen onClose={vi.fn()} sessionId="20260716_27" />);
    expect(screen.queryByTestId('diagnostics-private-warning')).toBeNull();
  });

  it('shows a server refusal instead of replacing it with a generic error', async () => {
    const onClose = vi.fn();
    diagnosticsMock.mockRejectedValue('The diagnostics request was refused.');

    render(<DiagnosticsModal isOpen onClose={onClose} sessionId="20260716_27" />);
    fireEvent.click(screen.getByRole('button', { name: 'Generate diagnostics' }));

    await waitFor(() => {
      expect(toastErrorMock).toHaveBeenCalledWith({
        title: 'Diagnostics error',
        msg: 'The diagnostics request was refused.',
      });
    });
    expect(saveDiagnosticsBundle).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });
});
