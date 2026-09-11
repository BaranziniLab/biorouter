import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import PrivacyPanel, { DISABLE_PHRASE, PRIVACY_TIERS_KEY } from './PrivacyPanel';
import { PRIVACY_TIERS_RECORD_KEY } from './privacyTiers';
import { __resetDisclosureStoreForTests } from '../../privacy/disclosureCopy';

const mocks = vi.hoisted(() => ({
  value: undefined as unknown,
  read: vi.fn(),
  upsert: vi.fn(),
  getPrivacyDisclosure: vi.fn(),
  ackPrivacyDisclosure: vi.fn(),
}));

vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({ read: mocks.read, upsert: mocks.upsert }),
}));

// Task 30A: the panel's permanent statement is SERVED, so the test serves it.
// ⚠ The fixture is deliberately not the product's sentence — Step 5's gate (1)
// counts definitions of that sentence across `ui/desktop/src/` and expects one,
// and a test file is not exempt from a `--include='*.tsx'` grep.
vi.mock('../../../api', () => ({
  getPrivacyDisclosure: mocks.getPrivacyDisclosure,
  ackPrivacyDisclosure: mocks.ackPrivacyDisclosure,
}));
vi.mock('../../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-key' }),
}));

const SERVED_LONG =
  'SERVED-COPY-MARKER — this model can read files on this computer.\n\nBiorouter does stop three things.';

describe('Settings > Privacy', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // The served copy is held in module state (one install, one disclosure), so
    // it outlives `cleanup()` and has to be dropped between tests.
    __resetDisclosureStoreForTests();
    mocks.value = undefined;
    mocks.read.mockImplementation(async () => mocks.value);
    // A STATEFUL stand-in for the daemon: a write that resolves is a write that
    // landed, so a later read sees it. The panel now reads the key back after
    // every write (P-05), and a mock whose `read` ignored its own `upsert` would
    // make every success look like a refusal.
    mocks.upsert.mockImplementation(async (_key: string, value: unknown) => {
      mocks.value = value;
    });
    mocks.getPrivacyDisclosure.mockResolvedValue({
      data: {
        title_template: '{provider} is not hosted by your institution.',
        long: SERVED_LONG,
        short: 'SERVED-SHORT-MARKER',
        acknowledged: false,
      },
    });
    mocks.ackPrivacyDisclosure.mockResolvedValue({ data: undefined });
  });

  it('defaults to on when the key is absent', async () => {
    render(<PrivacyPanel />);
    await waitFor(() =>
      expect(screen.getByRole('switch', { name: /Privacy tiers/ })).toBeChecked()
    );
  });

  it('does not turn off on a single click — it asks for the phrase first', async () => {
    const user = userEvent.setup();
    render(<PrivacyPanel />);
    await waitFor(() => screen.getByRole('switch', { name: /Privacy tiers/ }));

    await user.click(screen.getByRole('switch', { name: /Privacy tiers/ }));
    expect(mocks.upsert).not.toHaveBeenCalled();

    // All four sentences, and the two a user cannot reconstruct for themselves.
    const dialog = screen.getByTestId('privacy-disable-confirm');
    expect(dialog).toHaveTextContent(/every.*privacy guardrail on this machine/i);
    expect(dialog).toHaveTextContent(/read and write your knowledge bases/i);
    expect(dialog).toHaveTextContent(/stops recording which chats touched private/i);
    expect(dialog).toHaveTextContent(/cannot go back and mark anything that happened/i);
  });

  it('compares the phrase exactly, then writes with the confirmation', async () => {
    const user = userEvent.setup();
    render(<PrivacyPanel />);
    await waitFor(() => screen.getByRole('switch', { name: /Privacy tiers/ }));
    await user.click(screen.getByRole('switch', { name: /Privacy tiers/ }));

    const field = screen.getByLabelText('Confirmation phrase');
    const button = screen.getByRole('button', { name: /Turn off privacy tiers/ });

    // Lower case is NOT the phrase. This is the assertion that fails a
    // `toLowerCase()` or a `trim()` creeping into the comparison.
    await user.type(field, DISABLE_PHRASE.toLowerCase());
    expect(button).toBeDisabled();

    await user.clear(field);
    await user.type(field, DISABLE_PHRASE);
    expect(button).toBeEnabled();

    await user.click(button);
    await waitFor(() =>
      expect(mocks.upsert).toHaveBeenCalledWith(PRIVACY_TIERS_KEY, 'off', false, DISABLE_PHRASE)
    );
  });

  it('shows the persistent strip while enforcement is off, and turning it back on needs no phrase', async () => {
    const user = userEvent.setup();
    mocks.value = 'off';
    render(<PrivacyPanel />);

    await waitFor(() =>
      expect(screen.getByRole('switch', { name: /Privacy tiers/ })).not.toBeChecked()
    );
    expect(screen.getByTestId('privacy-enforcement-off-strip')).toHaveTextContent(
      /Privacy tiers are off/i
    );

    await user.click(screen.getByRole('switch', { name: /Privacy tiers/ }));
    await waitFor(() =>
      expect(mocks.upsert).toHaveBeenCalledWith(PRIVACY_TIERS_KEY, 'on', false, DISABLE_PHRASE)
    );
  });

  it('a refused write leaves the switch showing what is true, not what was asked', async () => {
    const user = userEvent.setup();
    mocks.upsert.mockRejectedValue(new Error('403 Forbidden'));
    render(<PrivacyPanel />);
    await waitFor(() => screen.getByRole('switch', { name: /Privacy tiers/ }));

    await user.click(screen.getByRole('switch', { name: /Privacy tiers/ }));
    await user.type(screen.getByLabelText('Confirmation phrase'), DISABLE_PHRASE);
    await user.click(screen.getByRole('button', { name: /Turn off privacy tiers/ }));

    await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent('403'));
    expect(screen.getByRole('switch', { name: /Privacy tiers/ })).toBeChecked();
  });

  /**
   * P-05. The reported defect was that this control had no end state at all —
   * it sat in `busy` forever, because the daemon never answered. The daemon's
   * half of that is fixed where it belongs (a bounded system-auth prompt), and
   * cannot be exercised here: jsdom has no macOS XPC to leave unanswered.
   *
   * What IS the renderer's own responsibility, and is what these three cover:
   * once an answer arrives, the control must land on a state that is TRUE, in
   * both directions, and must always come back out of `busy`.
   */
  describe('the control always ends in a definite state', () => {
    it('reports a write the daemon accepted but did not apply, instead of showing it as done', async () => {
      const user = userEvent.setup();
      // Resolves — no throw — but the value does not move. This is precisely
      // what the generated API client did on a 403 before `throwOnError` was
      // set on it, and the panel must not be the thing that depends on that.
      mocks.upsert.mockResolvedValue(undefined);
      render(<PrivacyPanel />);
      await waitFor(() => screen.getByRole('switch', { name: /Privacy tiers/ }));

      await user.click(screen.getByRole('switch', { name: /Privacy tiers/ }));
      await user.type(screen.getByLabelText('Confirmation phrase'), DISABLE_PHRASE);
      await user.click(screen.getByRole('button', { name: /Turn off privacy tiers/ }));

      await waitFor(() =>
        expect(screen.getByTestId('privacy-toggle-error')).toHaveTextContent(/did not apply/i)
      );
      // The switch shows what is TRUE, and the "tiers are off" strip — the
      // thing that would tell a user their machine is unprotected — is absent.
      expect(screen.getByRole('switch', { name: /Privacy tiers/ })).toBeChecked();
      expect(screen.queryByTestId('privacy-enforcement-off-strip')).toBeNull();
    });

    it('says so when turning protection back ON fails', async () => {
      const user = userEvent.setup();
      mocks.value = 'off';
      // The ON direction opens no confirmation dialog, which is exactly why
      // this needs its own test: the refusal used to be rendered INSIDE that
      // dialog, so this whole direction failed in silence.
      mocks.upsert.mockRejectedValue(new Error('the daemon refused'));
      render(<PrivacyPanel />);
      await waitFor(() =>
        expect(screen.getByRole('switch', { name: /Privacy tiers/ })).not.toBeChecked()
      );

      await user.click(screen.getByRole('switch', { name: /Privacy tiers/ }));

      await waitFor(() =>
        expect(screen.getByTestId('privacy-toggle-error')).toHaveTextContent(/refused/i)
      );
      expect(screen.queryByTestId('privacy-disable-confirm')).toBeNull();
    });

    it('comes back out of busy on every path, so the control is never left inert', async () => {
      const user = userEvent.setup();
      mocks.upsert.mockRejectedValue(new Error('403 Forbidden'));
      render(<PrivacyPanel />);
      await waitFor(() => screen.getByRole('switch', { name: /Privacy tiers/ }));

      await user.click(screen.getByRole('switch', { name: /Privacy tiers/ }));
      await user.type(screen.getByLabelText('Confirmation phrase'), DISABLE_PHRASE);
      await user.click(screen.getByRole('button', { name: /Turn off privacy tiers/ }));
      await waitFor(() => expect(screen.getByTestId('privacy-toggle-error')).toBeInTheDocument());

      // `disabled={busy}` on both, so a control still disabled after the answer
      // arrived is the permanent-spinner bug in its renderer-side form.
      expect(screen.getByRole('switch', { name: /Privacy tiers/ })).toBeEnabled();
      expect(screen.getByRole('button', { name: /Turn off privacy tiers/ })).toBeEnabled();
    });
  });

  /**
   * Task 30A (issue #56, DR-17 requirement 3). The panel is the surface anyone
   * can go back to, so it carries the disclosure permanently — in BOTH toggle
   * positions, because DR-15 turns off enforcement and not the truth.
   */
  describe('the non-private-model disclosure', () => {
    it('states what a public model can reach, above the switch, with enforcement ON', async () => {
      render(<PrivacyPanel />);
      const statement = await screen.findByTestId('non-private-model-statement');
      expect(statement).toHaveTextContent(/can read files on this computer/i);

      // Above the switch, not below it: a user who reads the first thing on the
      // screen must meet the limit before the control.
      const row = screen.getByRole('switch', { name: /Privacy tiers/ }).closest('div');
      expect(
        statement.compareDocumentPosition(row!) & Node.DOCUMENT_POSITION_FOLLOWING
      ).toBeTruthy();
    });

    it('the panel states the limit even when enforcement is off', async () => {
      mocks.value = 'off';
      render(<PrivacyPanel />);

      expect(await screen.findByText(/enforcement off/i)).toBeInTheDocument();
      expect(await screen.findByTestId('non-private-model-statement')).toHaveTextContent(
        /can read files on this computer/i
      );
    });

    it('with the feature off, it does not repeat the guarantee as though it still held', async () => {
      // The served copy says Biorouter "does stop three things". With the master
      // switch off it stops none of them, so a panel that reprinted that
      // paragraph unqualified would be making a false statement on the very
      // screen the user just used to turn it off.
      mocks.value = 'off';
      render(<PrivacyPanel />);
      const statement = await screen.findByTestId('non-private-model-statement');
      expect(statement).toHaveTextContent(/not being stopped/i);
    });

    it('renders nothing rather than inventing prose when the copy cannot be fetched', async () => {
      mocks.getPrivacyDisclosure.mockRejectedValue(new Error('offline'));
      render(<PrivacyPanel />);
      await waitFor(() => screen.getByRole('switch', { name: /Privacy tiers/ }));
      expect(screen.queryByTestId('non-private-model-statement')).toBeNull();
    });
  });

  /**
   * H3 (2026-09-10 security test drive). The composer's off-state note sends
   * the user here, so this strip carries the same account of how the switch
   * got to off and where it is recorded — and re-reads it after the user's own
   * flip, so it never quotes a stale "outside the app" beside a change they
   * just made.
   */
  describe('where the switch is recorded, and how it got to off', () => {
    const PATH = '/Users/someone/.config/biorouter/privacy-tiers.json';
    let record: unknown = null;

    beforeEach(() => {
      record = null;
      // Key-aware: the switch and its record are two keys on one surface.
      mocks.read.mockImplementation(async (key: string) =>
        key === PRIVACY_TIERS_RECORD_KEY ? record : mocks.value
      );
    });

    it('says the switch was turned off outside the app, and where it is recorded', async () => {
      mocks.value = 'off';
      record = { enabled: false, origin: 'unrecorded', path: PATH, last_change: null };
      render(<PrivacyPanel />);

      const strip = await screen.findByTestId('privacy-enforcement-off-strip');
      await waitFor(() => expect(strip).toHaveTextContent(/turned off outside the app/i));
      expect(strip).toHaveTextContent(PATH);
    });

    it('re-reads the record after the user turns the tiers off here', async () => {
      const user = userEvent.setup();
      mocks.upsert.mockImplementation(async (_key: string, value: unknown) => {
        mocks.value = value;
        // What the daemon's confirmed arm now remembers beside the value.
        record = {
          enabled: false,
          origin: 'settings',
          path: PATH,
          last_change: {
            via: 'settings',
            set_to: false,
            at: '2026-09-10T18:04:00+00:00',
            system_authenticated: true,
            user_action: true,
          },
        };
      });
      render(<PrivacyPanel />);
      await waitFor(() => screen.getByRole('switch', { name: /Privacy tiers/ }));

      await user.click(screen.getByRole('switch', { name: /Privacy tiers/ }));
      await user.type(screen.getByLabelText('Confirmation phrase'), DISABLE_PHRASE);
      await user.click(screen.getByRole('button', { name: /Turn off privacy tiers/ }));

      const strip = await screen.findByTestId('privacy-enforcement-off-strip');
      await waitFor(() => expect(strip).toHaveTextContent(/turned off in Settings → Privacy/i));
      expect(strip).not.toHaveTextContent(/outside the app/i);
      expect(strip).toHaveTextContent(PATH);
    });
  });
});
