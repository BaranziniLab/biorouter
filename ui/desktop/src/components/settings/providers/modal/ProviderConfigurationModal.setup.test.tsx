import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProviderDetails } from '../../../../api';
import type {
  CodingAgentAuth,
  CodingAgentAvailability,
  CodingAgentKind,
} from '../../../onboarding/codingAgentStatus';
import ProviderConfigurationModal from './ProviderConfiguationModal';

const mocks = vi.hoisted(() => ({
  submit: vi.fn(),
  status: vi.fn(),
  read: vi.fn(),
  upsert: vi.fn(),
}));
vi.mock('./subcomponents/handlers/DefaultSubmitHandler', () => ({
  providerConfigSubmitHandler: mocks.submit,
}));
vi.mock('../../../ConfigContext', () => ({
  useConfig: () => ({ read: mocks.read, upsert: mocks.upsert, remove: vi.fn() }),
}));
vi.mock('../../../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({ getCurrentModelAndProvider: vi.fn() }),
}));
vi.mock('../../../onboarding/codingAgentStatus', async () => ({
  ...(await vi.importActual<typeof import('../../../onboarding/codingAgentStatus')>(
    '../../../onboarding/codingAgentStatus'
  )),
  fetchCodingAgentStatus: mocks.status,
}));
vi.mock('../../../InAppTerminalDock', () => ({
  default: () => <div data-testid="setup-terminal" />,
}));
vi.mock('./subcomponents/ProviderLogo', () => ({ default: () => null }));

function fixture(kind: CodingAgentKind, auth: CodingAgentAuth): CodingAgentAvailability {
  return {
    kind,
    providerId: kind,
    displayName: kind === 'codex' ? 'Codex' : 'Claude Code',
    auth,
    path: auth.state === 'not_installed' ? null : `/tools/${kind}`,
    loginCommand: kind === 'codex' ? 'codex login' : 'claude auth login',
    installHint:
      kind === 'codex'
        ? 'npm install -g @openai/codex@latest'
        : 'curl -fsSL https://claude.ai/install.sh | bash',
  };
}
function provider(kind: CodingAgentKind): ProviderDetails {
  return {
    name: kind,
    is_configured: false,
    provider_type: 'Builtin',
    metadata: {
      name: kind,
      display_name: kind === 'codex' ? 'Codex' : 'Claude Code',
      description: '',
      default_model: '',
      known_models: [],
      model_doc_link: '',
      config_keys: [
        {
          name: kind === 'codex' ? 'CODEX_COMMAND' : 'CLAUDE_CODE_COMMAND',
          required: true,
          secret: false,
          oauth_flow: false,
          default: `/custom/${kind}`,
        },
      ],
    },
  } as ProviderDetails;
}
async function openFailure(kind: CodingAgentKind, onConfigured = vi.fn()) {
  render(
    <ProviderConfigurationModal
      provider={provider(kind)}
      onClose={vi.fn()}
      onConfigured={onConfigured}
    />
  );
  await screen.findByDisplayValue(`/custom/${kind}`);
  fireEvent.click(screen.getByRole('button', { name: 'Save' }));
  await screen.findByTestId('coding-agent-setup-recovery');
  return onConfigured;
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.submit.mockReset();
  mocks.status.mockReset();
  mocks.read.mockResolvedValue(null);
  mocks.submit.mockRejectedValueOnce(new Error('raw error with private-token-DO-NOT-DISPLAY'));
});

describe.each(['codex', 'claude_code'] as const)('%s configuration recovery', (kind) => {
  it.each(['not_installed', 'signed_out'] as const)(
    'offers appropriate steps for %s without executing commands',
    async (state) => {
      const agent = fixture(kind, { state });
      mocks.status.mockResolvedValue({ agents: [agent] });
      await openFailure(kind);
      const command = state === 'not_installed' ? agent.installHint : agent.loginCommand;
      expect(await screen.findByText(command)).toBeInTheDocument();
      expect(screen.getByRole('button', { name: `Copy ${command}` })).toBeEnabled();
      expect(screen.queryByText(/private-token-DO-NOT-DISPLAY/)).not.toBeInTheDocument();
      expect(screen.queryByRole('button', { name: 'Retry configuration' })).not.toBeInTheDocument();
      expect(screen.queryByTestId('setup-terminal')).not.toBeInTheDocument();
      expect(screen.getByRole('link', { name: /Open official/ })).toHaveAttribute(
        'href',
        kind === 'codex'
          ? 'https://developers.openai.com/codex/cli'
          : 'https://code.claude.com/docs/en/setup'
      );
      expect(mocks.upsert).not.toHaveBeenCalled();
      fireEvent.click(screen.getByRole('button', { name: `Copy ${command}` }));
      await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenCalledWith(command));
      fireEvent.click(screen.getByRole('button', { name: 'Open a terminal here' }));
      expect(screen.getByTestId('setup-terminal')).toBeInTheDocument();
    }
  );

  it('rechecks after sign-in and retries the original custom command only on request', async () => {
    mocks.read.mockResolvedValue(`/custom/${kind}`);
    mocks.status
      .mockResolvedValueOnce({ agents: [fixture(kind, { state: 'signed_out' })] })
      .mockResolvedValue({ agents: [fixture(kind, { state: 'signed_in_subscription' })] });
    mocks.submit.mockResolvedValueOnce(undefined);
    const onConfigured = await openFailure(kind);
    fireEvent.click(await screen.findByRole('button', { name: "I've signed in" }));
    const retry = await screen.findByRole('button', { name: 'Retry configuration' });
    expect(mocks.submit).toHaveBeenCalledTimes(1);
    fireEvent.click(retry);
    await waitFor(() => expect(onConfigured).toHaveBeenCalledWith(provider(kind)));
    expect(mocks.submit).toHaveBeenLastCalledWith(mocks.upsert, provider(kind), {
      [kind === 'codex' ? 'CODEX_COMMAND' : 'CLAUDE_CODE_COMMAND']: `/custom/${kind}`,
    });
    expect(mocks.upsert).not.toHaveBeenCalled();
  });

  it('keeps unknown status distinct from signed-out and uses normal text', async () => {
    mocks.status.mockResolvedValue({
      agents: [fixture(kind, { state: 'indeterminate', detail: 'The status check timed out.' })],
    });
    await openFailure(kind);
    const detail = await screen.findByText('The status check timed out.');
    expect(detail).not.toHaveClass('font-mono');
    expect(detail.tagName).toBe('P');
    expect(
      screen.queryByText(fixture(kind, { state: 'signed_out' }).loginCommand)
    ).not.toBeInTheDocument();
  });
});

it('can retry a failed status request without showing raw transport errors', async () => {
  mocks.status
    .mockRejectedValueOnce(new Error('private-token-DO-NOT-DISPLAY'))
    .mockResolvedValue({ agents: [fixture('codex', { state: 'signed_out' })] });
  await openFailure('codex');
  fireEvent.click(await screen.findByRole('button', { name: 'Check again' }));
  expect(await screen.findByText('codex login')).toBeInTheDocument();
  expect(screen.queryByText(/private-token-DO-NOT-DISPLAY/)).not.toBeInTheDocument();
});

it('rechecks a failed configuration retry and offers the new sign-in state', async () => {
  mocks.status
    .mockResolvedValueOnce({ agents: [fixture('codex', { state: 'signed_in_subscription' })] })
    .mockResolvedValue({ agents: [fixture('codex', { state: 'signed_out' })] });
  mocks.submit.mockRejectedValueOnce(new Error('session expired'));
  const onConfigured = await openFailure('codex');
  fireEvent.click(await screen.findByRole('button', { name: 'Retry configuration' }));
  expect(await screen.findByText('codex login')).toBeInTheDocument();
  expect(onConfigured).not.toHaveBeenCalled();
});

it('explains Claude Code subscription setup without asking for an API key', async () => {
  render(<ProviderConfigurationModal provider={provider('claude_code')} onClose={vi.fn()} />);
  expect(screen.getByText(/No API key is needed here/)).toBeInTheDocument();
  expect(screen.queryByText(/Add your API key/)).not.toBeInTheDocument();
});

it('advances only after a successful configuration check and ready subscription probe', async () => {
  mocks.submit.mockReset().mockResolvedValue(undefined);
  mocks.status.mockResolvedValue({
    agents: [fixture('codex', { state: 'signed_in_subscription' })],
  });
  const onConfigured = vi.fn();
  render(
    <ProviderConfigurationModal
      provider={provider('codex')}
      onClose={vi.fn()}
      onConfigured={onConfigured}
    />
  );
  await screen.findByDisplayValue('/custom/codex');
  fireEvent.click(screen.getByRole('button', { name: 'Save' }));
  await waitFor(() => expect(onConfigured).toHaveBeenCalled());
  expect(mocks.status).toHaveBeenCalledTimes(1);
  expect(screen.queryByTestId('coding-agent-setup-recovery')).not.toBeInTheDocument();
});

it('keeps Anthropic API configuration separate from Claude Code setup', async () => {
  const apiProvider = {
    ...provider('claude_code'),
    name: 'anthropic',
    metadata: {
      ...provider('claude_code').metadata,
      name: 'anthropic',
      display_name: 'Anthropic',
      config_keys: [
        {
          name: 'ANTHROPIC_API_KEY',
          required: true,
          secret: true,
          oauth_flow: false,
          default: null,
        },
      ],
    },
  } as ProviderDetails;
  render(<ProviderConfigurationModal provider={apiProvider} onClose={vi.fn()} />);
  expect(screen.getByText(/Add your API key/)).toBeInTheDocument();
  expect(screen.queryByText(/subscription sign-in/)).not.toBeInTheDocument();
  expect(mocks.status).not.toHaveBeenCalled();
});

describe.each(['codex', 'claude_code'] as const)(
  '%s readiness after a successful provider check',
  (kind) => {
    it.each<CodingAgentAuth>([
      { state: 'not_installed' },
      { state: 'signed_out' },
      { state: 'indeterminate', detail: 'The check did not finish.' },
      { state: 'signed_in_with_api_key' },
    ])('keeps $state in setup instead of opening model selection', async (auth) => {
      mocks.submit.mockReset().mockResolvedValue(undefined);
      mocks.status.mockResolvedValue({ agents: [fixture(kind, auth)] });
      const onConfigured = await openFailure(kind);
      expect(onConfigured).not.toHaveBeenCalled();
      expect(mocks.status).toHaveBeenCalledTimes(1);
      expect(screen.queryByRole('button', { name: 'Retry configuration' })).not.toBeInTheDocument();
      if (auth.state === 'signed_out') {
        expect(screen.getByText(fixture(kind, auth).loginCommand)).toBeInTheDocument();
      }
    });

    it('requires another ready probe when retrying after sign-in', async () => {
      mocks.submit.mockReset().mockResolvedValue(undefined);
      mocks.status
        .mockResolvedValueOnce({ agents: [fixture(kind, { state: 'signed_out' })] })
        .mockResolvedValueOnce({ agents: [fixture(kind, { state: 'signed_in_subscription' })] })
        .mockResolvedValue({ agents: [fixture(kind, { state: 'signed_out' })] });
      const onConfigured = await openFailure(kind);
      fireEvent.click(screen.getByRole('button', { name: "I've signed in" }));
      fireEvent.click(await screen.findByRole('button', { name: 'Retry configuration' }));
      expect(
        await screen.findByText(fixture(kind, { state: 'signed_out' }).loginCommand)
      ).toBeInTheDocument();
      expect(onConfigured).not.toHaveBeenCalled();
      expect(mocks.status).toHaveBeenCalledTimes(3);
    });
  }
);

it('does not advance after the configuration modal was closed during its readiness probe', async () => {
  mocks.submit.mockReset().mockResolvedValue(undefined);
  let finish!: (value: { agents: CodingAgentAvailability[] }) => void;
  mocks.status.mockReturnValue(
    new Promise((resolve) => {
      finish = resolve;
    })
  );
  const onConfigured = vi.fn();
  const { unmount } = render(
    <ProviderConfigurationModal
      provider={provider('codex')}
      onClose={vi.fn()}
      onConfigured={onConfigured}
    />
  );
  await screen.findByDisplayValue('/custom/codex');
  fireEvent.click(screen.getByRole('button', { name: 'Save' }));
  await waitFor(() => expect(mocks.status).toHaveBeenCalledTimes(1));
  unmount();
  finish({ agents: [fixture('codex', { state: 'signed_in_subscription' })] });
  await Promise.resolve();
  expect(onConfigured).not.toHaveBeenCalled();
});
