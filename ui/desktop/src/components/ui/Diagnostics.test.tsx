import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { diagnostics, getSession, systemInfo } from '../../api';
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
const systemInfoMock = vi.mocked(systemInfo);
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

  /**
   * Principle 2: a visible line is a label, a value or a state word; the explanation of the other
   * way to report sits behind an InfoTip. What leaves the machine (the list, the sensitive-data
   * warning, the private-chat warning) stays in the page.
   */
  it('keeps what leaves the machine visible and puts the tips behind an InfoTip', () => {
    render(<DiagnosticsModal isOpen onClose={() => {}} sessionId="20260716_27" />);
    const dialog = screen.getByRole('dialog', { name: 'Report a problem' });
    expect(dialog).toHaveTextContent('A diagnostics report contains the following:');
    expect(dialog).toHaveTextContent('Your current chat messages');
    expect(dialog).toHaveTextContent('do not share the diagnostics file publicly');
    expect(dialog).not.toHaveTextContent('consider attaching');

    const tip = screen.getByTestId('diagnostics-agent-hint');
    expect(tip).toHaveAccessibleName('About Report a problem');
    expect(tip).toHaveAccessibleDescription(
      /report a bug.*removes paths and credentials.*exact text before anything is published/
    );
    expect(tip.closest('p')).toBeNull();
  });

  /**
   * V4-R4-2: the one dialog recipe. It was a raw, centred DialogContent with a warning tile before
   * the title and 28px footer buttons; geometry is measured in `diagnosticsDialog.browser.test.tsx`.
   */
  it('is the shared dialog: ModalShell md at the top, the header rule, no tile, a word-only footer', () => {
    render(<DiagnosticsModal isOpen onClose={() => {}} sessionId="20260716_27" />);
    const dialog = screen.getByRole('dialog', { name: 'Report a problem' });
    expect(dialog).toHaveAttribute('data-size', 'md');
    expect(dialog).toHaveAttribute('data-anchor', 'top');

    const heading = screen.getByRole('heading', { name: 'Report a problem' });
    expect(heading).toHaveTextContent(/^Report a problem$/);
    const header = heading.closest('.border-b')!;
    expect(header).not.toBeNull();
    // The title row holds the title and its InfoTip, and nothing painted before them.
    expect(header.querySelectorAll('svg')).toHaveLength(1);
    expect(screen.getByTestId('diagnostics-agent-hint')).toContainElement(
      header.querySelector('svg')
    );
    expect(header.querySelector('[class*="bg-"]')).toBeNull();

    // What leaves the machine is the dialog's description, in body text.
    expect(dialog).toHaveAccessibleDescription(
      /A diagnostics report contains the following:.*Your current chat messages.*do not share the diagnostics file publicly/
    );

    const buttons = screen
      .getAllByRole('button')
      .filter((button) => button.textContent && button.textContent !== 'Close')
      .filter((button) => !button.classList.contains('br-info-tip'));
    expect(buttons.map((button) => button.textContent)).toEqual([
      'Cancel',
      'Generate diagnostics',
      'File a bug on GitHub',
    ]);
    for (const button of buttons) {
      expect(button).toHaveClass('h-control-md');
      expect(button).not.toHaveClass('h-control-sm');
      expect(button.querySelector('svg')).toBeNull();
    }
    // The dismiss and the second action are secondary; the one coral button files the bug.
    expect(buttons.filter((button) => button.classList.contains('bg-background-accent'))).toEqual([
      buttons[2],
    ]);
  });

  it('opens with focus on the dialog box, not on the title’s InfoTip', async () => {
    render(<DiagnosticsModal isOpen onClose={() => {}} sessionId="20260716_27" />);
    const dialog = screen.getByRole('dialog', { name: 'Report a problem' });
    await waitFor(() => expect(dialog).toHaveFocus());
    expect(screen.getByTestId('diagnostics-agent-hint')).not.toHaveFocus();
  });

  it('must be answered while the bundle is being made: no close button, Escape ignored', async () => {
    const onClose = vi.fn();
    saveDiagnosticsBundle.mockImplementation(() => new Promise(() => {}));
    render(<DiagnosticsModal isOpen onClose={onClose} sessionId="20260716_27" />);
    expect(screen.getByRole('button', { name: 'Close' })).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Generate diagnostics' }));
    expect(await screen.findByRole('status')).toHaveTextContent('Preparing diagnostics bundle');
    expect(screen.queryByRole('button', { name: 'Close' })).toBeNull();
    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    expect(screen.getByRole('dialog', { name: 'Report a problem' })).toBeVisible();
    expect(onClose).not.toHaveBeenCalled();
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
    expect(screen.getByRole('button', { name: 'Generating…' })).toBeDisabled();

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

  /**
   * V4-R7-6: a bare `<strong>` computes to `bolder`, 700, and the type system draws only 400 and
   * 600. Both warning leads wear `font-semibold`; `diagnosticsDialog.browser.test.tsx` measures the
   * computed weight.
   */
  it('sets both warning leads at the strong weight, 600', () => {
    render(
      <DiagnosticsModal isOpen onClose={vi.fn()} sessionId="20260716_27" privacyTier="private" />
    );
    const dialog = screen.getByRole('dialog', { name: 'Report a problem' });
    const leads = [...dialog.querySelectorAll('strong')];
    expect(leads.map((el) => el.textContent)).toEqual(['Warning:', 'This chat is private.']);
    for (const el of leads) expect(el).toHaveClass('font-semibold');
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

  it('points at the in-chat reporter, by its slash command and in words', () => {
    render(<DiagnosticsModal isOpen onClose={vi.fn()} sessionId="20260716_27" />);

    // The hint is the title's InfoTip: its words are the glyph's description, not its text.
    const hint = screen.getByTestId('diagnostics-agent-hint');
    expect(hint).toHaveAccessibleDescription(/\/bug.*report a bug/);
    expect(hint).not.toHaveAccessibleDescription(/[—–]/);
  });

  // The body is a hand copy of `.github/ISSUE_TEMPLATE/bug_report.md`, so a
  // section added there (as **Suspected cause** was) is missing here until
  // someone notices. Read the template and compare the headings.
  it("files a body with the repository template's sections, in its order", async () => {
    systemInfoMock.mockResolvedValue({
      data: {
        os: 'macOS',
        os_version: '15.6',
        architecture: 'aarch64',
        app_version: '1.92.1',
        provider: 'versa_azure',
        model: 'gpt-5.6-sol',
        enabled_extensions: ['developer'],
      },
    } as unknown as Awaited<ReturnType<typeof systemInfo>>);
    const open = vi.spyOn(window, 'open').mockImplementation(() => null);

    render(<DiagnosticsModal isOpen onClose={vi.fn()} sessionId="20260716_27" />);
    fireEvent.click(screen.getByRole('button', { name: 'File a bug on GitHub' }));
    await waitFor(() => expect(open).toHaveBeenCalledTimes(1));

    const body = new URL(String(open.mock.calls[0][0])).searchParams.get('body') ?? '';
    const template = readFileSync(
      join(__dirname, '../../../../../.github/ISSUE_TEMPLATE/bug_report.md'),
      'utf8'
    );
    const headings = (text: string) =>
      [...text.matchAll(/^\*\*([^*]+)\*\*\s*$/gm)].map((match) => match[1]);
    expect(headings(template)).toContain('Suspected cause');
    expect(headings(body)).toEqual(headings(template));
    open.mockRestore();
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
