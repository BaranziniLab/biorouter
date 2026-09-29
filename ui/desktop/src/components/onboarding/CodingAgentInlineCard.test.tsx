import { describe, it, expect, vi, beforeEach } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import CodingAgentInlineCard, {
  CODING_AGENT_TERMINAL_HEIGHT_PX,
  CODING_AGENT_TERMINAL_RESERVE_PX,
} from './CodingAgentInlineCard';
import type { CodingAgentAuth, CodingAgentAvailability } from './codingAgentStatus';

const mockFetchStatus = vi.fn();
vi.mock('./codingAgentStatus', async () => {
  const actual = await vi.importActual<typeof import('./codingAgentStatus')>('./codingAgentStatus');
  return {
    ...actual,
    fetchCodingAgentStatus: () => mockFetchStatus(),
  };
});

vi.mock('react-router-dom', () => ({
  useNavigate: () => vi.fn(),
}));

const mockUpsert = vi.fn();
vi.mock('../ConfigContext', () => ({
  useConfig: () => ({ upsert: mockUpsert }),
}));

const mockToastError = vi.fn();
const mockToastSuccess = vi.fn();
vi.mock('../../toasts', () => ({
  toastService: {
    error: (...args: unknown[]) => mockToastError(...args),
    success: (...args: unknown[]) => mockToastSuccess(...args),
  },
}));

// The real dock wires xterm and the terminal:create IPC, neither of which exists
// in jsdom. Every other suite in the repo mocks it the same way.
vi.mock('../InAppTerminalDock', () => ({
  default: () => <div data-testid="in-app-terminal-dock" />,
}));

const claude = (auth: CodingAgentAuth, over: Partial<CodingAgentAvailability> = {}) =>
  ({
    kind: 'claude_code',
    providerId: 'claude_code',
    displayName: 'Claude Code',
    path: null,
    version: null,
    auth,
    loginCommand: 'claude auth login',
    installHint: 'npm install -g @anthropic-ai/claude-code@latest',
    ...over,
  }) satisfies CodingAgentAvailability;

const codex = (auth: CodingAgentAuth, over: Partial<CodingAgentAvailability> = {}) =>
  ({
    kind: 'codex',
    providerId: 'codex',
    displayName: 'Codex',
    path: null,
    version: null,
    auth,
    loginCommand: 'codex login',
    installHint: 'npm install -g @openai/codex@latest',
    ...over,
  }) satisfies CodingAgentAvailability;

const respondWith = (...agents: CodingAgentAvailability[]) =>
  mockFetchStatus.mockResolvedValue({ agents });

describe('CodingAgentInlineCard', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockUpsert.mockResolvedValue(undefined);
    respondWith(claude({ state: 'not_installed' }), codex({ state: 'not_installed' }));
  });

  /**
   * ⚠ **A successful connect must re-enable the buttons.**
   *
   * `connect` sets `connectingProviderId` on entry and used to clear it only in
   * the catch arm, on the reasoning that a success hands off and never comes
   * back. It does come back: `ProviderGuard` opens the Choose Model modal
   * BESIDE the still-mounted onboarding tree rather than unmounting it, and
   * closing that modal without picking a model returns the user to this card.
   *
   * Both buttons are `disabled={connectingProviderId !== null}` and `connect`
   * early-returns while it is set, so the card wedged: "Connecting…" forever on
   * the one they clicked, greyed out on the other, no re-check path that resets
   * it. And because the config write already happened, relaunching skips
   * onboarding rather than offering a second chance.
   */
  it('re-enables both agents after a successful connect, since the card stays mounted', async () => {
    respondWith(
      claude({ state: 'signed_in_subscription' }),
      codex({ state: 'signed_in_subscription' })
    );
    const onSuccess = vi.fn();
    render(<CodingAgentInlineCard onSuccess={onSuccess} />);

    const use = await screen.findByRole('button', { name: /Use Claude Code/i });
    fireEvent.click(use);

    await waitFor(() => expect(onSuccess).toHaveBeenCalledWith('claude_code'));

    // The user closes the model modal without choosing; this card never
    // unmounted, so both buttons have to be usable again.
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /Use Claude Code/i })).toBeEnabled()
    );
    expect(screen.getByRole('button', { name: /Use Codex/i })).toBeEnabled();
  });

  it('renders a row for every agent the daemon reports', async () => {
    render(<CodingAgentInlineCard onSuccess={vi.fn()} />);
    expect(await screen.findByTestId('coding-agent-row-claude_code')).toBeInTheDocument();
    expect(screen.getByTestId('coding-agent-row-codex')).toBeInTheDocument();
    // Display names come from the wire, never from a literal in the component.
    expect(screen.getByText('Claude Code')).toBeInTheDocument();
    expect(screen.getByText('Codex')).toBeInTheDocument();
  });

  describe('not_installed', () => {
    it('shows the install hint as a copyable command and never offers to run it', async () => {
      respondWith(claude({ state: 'not_installed' }));
      render(<CodingAgentInlineCard onSuccess={vi.fn()} />);

      expect(
        await screen.findByText('npm install -g @anthropic-ai/claude-code@latest')
      ).toBeInTheDocument();
      expect(screen.getByText('Not installed')).toBeInTheDocument();
      // No button anywhere volunteers to install another vendor's toolchain.
      for (const button of screen.getAllByRole('button')) {
        expect(button.textContent ?? '').not.toMatch(/^install/i);
      }
      // Nor does it tell a user with no binary to sign in.
      expect(screen.queryByText('claude auth login')).not.toBeInTheDocument();
      // A shell is still offered — the user typing the install line is the user
      // installing it; only Biorouter running it is out of bounds.
      expect(screen.getByTestId('coding-agent-terminal-toggle-claude_code')).toBeInTheDocument();
    });

    it('copies the install hint to the clipboard', async () => {
      respondWith(claude({ state: 'not_installed' }));
      render(<CodingAgentInlineCard onSuccess={vi.fn()} />);

      const copy = await screen.findByRole('button', {
        name: 'Copy npm install -g @anthropic-ai/claude-code@latest',
      });
      fireEvent.click(copy);

      await waitFor(() =>
        expect(navigator.clipboard.writeText).toHaveBeenCalledWith(
          'npm install -g @anthropic-ai/claude-code@latest'
        )
      );
    });

    it('re-fetches status from "Check again"', async () => {
      respondWith(claude({ state: 'not_installed' }));
      render(<CodingAgentInlineCard onSuccess={vi.fn()} />);

      const recheck = await screen.findByTestId('coding-agent-recheck-claude_code');
      expect(recheck).toHaveTextContent('Check again');

      mockFetchStatus.mockResolvedValue({
        agents: [claude({ state: 'signed_in_subscription', plan: 'max', account: null })],
      });
      fireEvent.click(recheck);

      expect(await screen.findByTestId('coding-agent-connect-claude_code')).toBeInTheDocument();
      expect(mockFetchStatus).toHaveBeenCalledTimes(2);
    });
  });

  describe('signed_out', () => {
    it('shows the login command and says Biorouter never brokers the credential', async () => {
      respondWith(claude({ state: 'signed_out' }, { path: '/opt/homebrew/bin/claude' }));
      render(<CodingAgentInlineCard onSuccess={vi.fn()} />);

      expect(await screen.findByText('claude auth login')).toBeInTheDocument();
      expect(screen.getByText('Installed · not signed in')).toBeInTheDocument();
      expect(
        screen.getByText(/never passes through Biorouter/i, { exact: false })
      ).toBeInTheDocument();
      // The re-check reads as an acknowledgement here, not a poll.
      expect(screen.getByTestId('coding-agent-recheck-claude_code')).toHaveTextContent(
        "I've signed in"
      );
    });

    it('offers an in-app terminal alongside the copyable command, not instead of it', async () => {
      respondWith(claude({ state: 'signed_out' }));
      render(<CodingAgentInlineCard onSuccess={vi.fn()} />);

      const toggle = await screen.findByTestId('coding-agent-terminal-toggle-claude_code');
      expect(screen.queryByTestId('in-app-terminal-dock')).not.toBeInTheDocument();

      fireEvent.click(toggle);

      expect(screen.getByTestId('in-app-terminal-dock')).toBeInTheDocument();
      // The command stays reachable with the terminal open.
      expect(screen.getByText('claude auth login')).toBeInTheDocument();

      fireEvent.click(screen.getByTestId('coding-agent-terminal-toggle-claude_code'));
      expect(screen.queryByTestId('in-app-terminal-dock')).not.toBeInTheDocument();
    });

    it('mounts the dock in a box taller than the visible region by exactly the reserve', async () => {
      respondWith(claude({ state: 'signed_out' }));
      render(<CodingAgentInlineCard onSuccess={vi.fn()} />);

      fireEvent.click(await screen.findByTestId('coding-agent-terminal-toggle-claude_code'));

      // The dock sizes itself as `calc(100% - 200px)` of its parent, so the mount
      // box must be that much taller than what the user sees, and the outer box
      // clips the difference. A drifted reserve silently shrinks the terminal.
      const visible = screen.getByTestId('coding-agent-terminal-claude_code');
      expect(visible).toHaveStyle({ height: `${CODING_AGENT_TERMINAL_HEIGHT_PX}px` });
      expect(visible.firstElementChild).toHaveStyle({
        height: `${CODING_AGENT_TERMINAL_HEIGHT_PX + CODING_AGENT_TERMINAL_RESERVE_PX}px`,
      });
    });
  });

  describe('signed_in_with_api_key', () => {
    it('is not presented as a failure, and names both ways out', async () => {
      respondWith(claude({ state: 'signed_in_with_api_key' }));
      render(<CodingAgentInlineCard onSuccess={vi.fn()} />);

      // Reads as a billing mismatch, not an error: no "not installed"/"failed"
      // framing, and the info dot rather than the warning one.
      expect(await screen.findByText('Signed in with an API key')).toBeInTheDocument();
      expect(screen.getByText(/The CLI works/)).toBeInTheDocument();
      expect(screen.getByText(/bill per token/)).toBeInTheDocument();
      expect(screen.getByText(/removes API credentials from the environment/)).toBeInTheDocument();

      // Way out 1: sign in with the subscription.
      expect(screen.getByText('claude auth login')).toBeInTheDocument();
      // Way out 2: use the vendor's metered provider instead.
      expect(screen.getByText('Anthropic')).toBeInTheDocument();

      // Not selectable — this state cannot serve a turn.
      expect(screen.queryByTestId('coding-agent-connect-claude_code')).not.toBeInTheDocument();
    });

    it('names the right metered sibling for Codex', async () => {
      respondWith(codex({ state: 'signed_in_with_api_key' }));
      render(<CodingAgentInlineCard onSuccess={vi.fn()} />);

      expect(await screen.findByText('OpenAI')).toBeInTheDocument();
      expect(screen.queryByText('Anthropic')).not.toBeInTheDocument();
    });
  });

  describe('signed_in_subscription', () => {
    it('shows plan, account, version and path when present', async () => {
      respondWith(
        claude(
          { state: 'signed_in_subscription', plan: 'max', account: 'ada@example.edu' },
          {
            version: '2.1.4 (Claude Code)',
            path: '/opt/homebrew/bin/claude',
          }
        )
      );
      render(<CodingAgentInlineCard onSuccess={vi.fn()} />);

      expect(await screen.findByText(/Plan: max · Account: ada@example.edu/)).toBeInTheDocument();
      expect(
        screen.getByText('2.1.4 (Claude Code) · /opt/homebrew/bin/claude')
      ).toBeInTheDocument();
      expect(screen.getByText('Ready · signed in on your subscription')).toBeInTheDocument();
    });

    it('persists the defaulted command key, and only that, then hands off', async () => {
      const onSuccess = vi.fn();
      respondWith(claude({ state: 'signed_in_subscription', plan: null, account: null }));
      render(<CodingAgentInlineCard onSuccess={onSuccess} />);

      fireEvent.click(await screen.findByTestId('coding-agent-connect-claude_code'));

      await waitFor(() => expect(onSuccess).toHaveBeenCalledWith('claude_code'));
      // Writing the defaulted key is what makes check_provider_configured report
      // the provider as configured — the whole reason this call exists.
      expect(mockUpsert).toHaveBeenNthCalledWith(1, 'CLAUDE_CODE_COMMAND', 'claude', false);
      // W2-PRV-5: NOT the model every new chat starts on. That moved on this
      // click, before any model was chosen; the model step that follows writes
      // the provider and model together, to the scope picked there.
      expect(mockUpsert).toHaveBeenCalledTimes(1);
      expect(mockUpsert).not.toHaveBeenCalledWith('BIOROUTER_PROVIDER', expect.anything(), false);
      // The command name, never the resolved absolute path.
      expect(mockUpsert).not.toHaveBeenCalledWith(
        'CLAUDE_CODE_COMMAND',
        expect.stringContaining('/'),
        false
      );
      expect(mockToastSuccess).toHaveBeenCalled();
    });

    it('writes CODEX_COMMAND for Codex', async () => {
      const onSuccess = vi.fn();
      respondWith(codex({ state: 'signed_in_subscription', plan: 'plus', account: null }));
      render(<CodingAgentInlineCard onSuccess={onSuccess} />);

      fireEvent.click(await screen.findByTestId('coding-agent-connect-codex'));

      await waitFor(() => expect(onSuccess).toHaveBeenCalledWith('codex'));
      expect(mockUpsert).toHaveBeenNthCalledWith(1, 'CODEX_COMMAND', 'codex', false);
    });

    it('surfaces a failed config write instead of claiming success', async () => {
      const onSuccess = vi.fn();
      respondWith(claude({ state: 'signed_in_subscription', plan: null, account: null }));
      mockUpsert.mockRejectedValueOnce(new Error('keyring locked'));
      render(<CodingAgentInlineCard onSuccess={onSuccess} />);

      fireEvent.click(await screen.findByTestId('coding-agent-connect-claude_code'));

      await waitFor(() => expect(mockToastError).toHaveBeenCalled());
      expect(onSuccess).not.toHaveBeenCalled();
    });
  });

  describe('indeterminate', () => {
    it('shows the detail and does NOT tell a possibly-signed-in user to sign in', async () => {
      respondWith(
        claude({ state: 'indeterminate', detail: 'credentials.json parsed but had no plan field' })
      );
      render(<CodingAgentInlineCard onSuccess={vi.fn()} />);

      expect(await screen.findByTestId('coding-agent-detail-claude_code')).toHaveTextContent(
        'credentials.json parsed but had no plan field'
      );
      expect(screen.getByText('Status unclear')).toBeInTheDocument();
      // The one thing this arm must not do.
      expect(screen.queryByText('claude auth login')).not.toBeInTheDocument();
      expect(screen.getByTestId('coding-agent-recheck-claude_code')).toHaveTextContent(
        'Check again'
      );
    });
  });

  it('offers a retry when the status route itself fails', async () => {
    mockFetchStatus.mockRejectedValueOnce(new Error('404 Not Found'));
    render(<CodingAgentInlineCard onSuccess={vi.fn()} />);

    expect(await screen.findByTestId('coding-agent-load-error')).toHaveTextContent('404 Not Found');

    respondWith(claude({ state: 'signed_out' }));
    fireEvent.click(screen.getByTestId('coding-agent-retry'));

    expect(await screen.findByText('claude auth login')).toBeInTheDocument();
  });
});
