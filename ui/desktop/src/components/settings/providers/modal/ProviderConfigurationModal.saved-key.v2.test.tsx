import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProviderDetails, ReadConfigData, RemoveConfigData } from '../../../../api';
import { BROWSER_SURFACE_MARKER } from '../../../../utils/surface';
import {
  loadProviderSetupAccess,
  resetProviderSetupAccessForTests,
} from '../../../../utils/providerSetupAccess';
import ProviderConfigurationModal from './ProviderConfiguationModal';

const mocks = vi.hoisted(() => ({
  checkProvider: vi.fn(),
  previewProviderSecret: vi.fn(),
  removeConfig: vi.fn(),
  readConfig: vi.fn(),
  getProviderSetupAccess: vi.fn(),
  headers: vi.fn(),
  read: vi.fn(),
  upsert: vi.fn(),
  remove: vi.fn(),
}));

vi.mock('../../../../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  checkProvider: mocks.checkProvider,
  previewProviderSecret: mocks.previewProviderSecret,
  removeConfig: mocks.removeConfig,
  readConfig: mocks.readConfig,
  getProviderSetupAccess: mocks.getProviderSetupAccess,
}));
vi.mock('../../../../utils/userAction', () => ({ userActionHeaders: mocks.headers }));
vi.mock('../../../ConfigContext', () => ({
  useConfig: () => ({
    read: mocks.read,
    upsert: mocks.upsert,
    remove: mocks.remove,
    getProviders: async () => [],
  }),
}));
vi.mock('../../../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    getCurrentModelAndProvider: async () => ({ provider: 'other', model: 'fixture' }),
  }),
}));

const KEY = 'ANTHROPIC_API_KEY';
const LABEL = 'Anthropic API key';
const MASK = '••••••••';
const WORKING_KEY = 'synthetic-working-anthropic-key';
const CANDIDATE = 'synthetic-candidate-anthropic-key';
const saved = new Map<string, unknown>();
const savedCases = [
  ['one character', 'x'],
  ['seventeen characters', 'abcdefghijklmnopq'],
  ['long ASCII', 'synthetic-saved-credential-with-a-deliberately-different-length'],
  ['Unicode', '甲乙丙丁戊己庚辛壬癸子丑寅卯辰巳午未申酉戌亥'],
] as const;

function providerDetails(configured = true): ProviderDetails {
  return {
    name: 'anthropic',
    is_configured: configured,
    provider_type: 'Builtin',
    metadata: {
      name: 'anthropic',
      display_name: 'Anthropic',
      description: '',
      default_model: '',
      known_models: [],
      model_doc_link: '',
      config_keys: [{ name: KEY, required: true, secret: true, oauth_flow: false, default: null }],
    },
  } as ProviderDetails;
}

function OpenProvider({
  provider,
  onClose,
  onConfigured,
}: {
  provider: ProviderDetails;
  onClose: () => void;
  onConfigured: (configured: ProviderDetails) => void;
}) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button type="button" onClick={() => setOpen(true)}>
        Configure Anthropic
      </button>
      {open && (
        <ProviderConfigurationModal
          provider={provider}
          onClose={() => {
            setOpen(false);
            onClose();
          }}
          onConfigured={(configured) => {
            setOpen(false);
            onConfigured(configured);
          }}
        />
      )}
    </>
  );
}

async function openProvider(configured = true) {
  const provider = providerDetails(configured);
  const onClose = vi.fn();
  const onConfigured = vi.fn();
  const user = userEvent.setup();
  render(<OpenProvider provider={provider} onClose={onClose} onConfigured={onConfigured} />);
  const opener = screen.getByRole('button', { name: 'Configure Anthropic' });
  await user.click(opener);
  const input = (await screen.findByLabelText(LABEL)) as HTMLInputElement;
  return { input, opener, provider, onClose, onConfigured, user };
}

function savedKeyEditButton() {
  const input = screen.getByLabelText(LABEL) as HTMLInputElement;
  return within(keyField(input)).getByRole('button', { name: /\b(?:edit|replace)\b/i });
}

function keyField(input: HTMLInputElement) {
  const field = input.closest<HTMLElement>('[data-slot="field"]');
  if (!field) throw new Error('The key must remain in the existing labelled Field.');
  return field;
}

async function enterReplacement(user: ReturnType<typeof userEvent.setup>) {
  await user.click(savedKeyEditButton());
  const candidate = (await screen.findByLabelText(LABEL)) as HTMLInputElement;
  expect(candidate).not.toHaveAttribute('readonly');
  expect(candidate).toHaveValue('');
  await waitFor(() => expect(candidate).toHaveFocus());
  expect(saved.get(KEY)).toBe(WORKING_KEY);
  expect(mocks.upsert).not.toHaveBeenCalled();
  expect(mocks.remove).not.toHaveBeenCalled();
  expect(mocks.removeConfig).not.toHaveBeenCalled();
  return candidate;
}

function modalCancel(input: HTMLInputElement) {
  const field = keyField(input);
  const cancel = screen
    .getAllByRole('button', { name: /^cancel$/i })
    .find((button) => !field.contains(button));
  if (!cancel) throw new Error('The existing modal Cancel action must remain available.');
  return cancel;
}

function holdNextCheck() {
  let release!: () => void;
  const pending = new Promise<void>((resolve) => {
    release = resolve;
  });
  mocks.checkProvider.mockImplementationOnce(async () => {
    await pending;
    return { data: {} };
  });
  return release;
}

function expectNoCredentialMutation() {
  expect(mocks.upsert).not.toHaveBeenCalled();
  expect(mocks.remove).not.toHaveBeenCalled();
  expect(mocks.removeConfig).not.toHaveBeenCalled();
}

beforeEach(() => {
  vi.resetAllMocks();
  saved.clear();
  saved.set(KEY, WORKING_KEY);
  mocks.headers.mockResolvedValue({ 'X-User-Action': 'synthetic-user-action' });
  mocks.checkProvider.mockResolvedValue({ data: {} });
  mocks.previewProviderSecret.mockRejectedValue(new Error('Saved-key retrieval is forbidden.'));
  mocks.getProviderSetupAccess.mockResolvedValue({ data: { allowed: true } });
  mocks.read.mockImplementation(async (key: string, secret: boolean) =>
    saved.has(key) ? (secret ? { maskedValue: MASK } : saved.get(key)) : null
  );
  mocks.upsert.mockImplementation(async (key: string, value: unknown) => {
    saved.set(key, value);
  });
  mocks.remove.mockImplementation(async (key: string) => {
    saved.delete(key);
  });
  mocks.removeConfig.mockImplementation(async ({ body }: RemoveConfigData) => {
    saved.delete(body.key);
    return { data: 'removed' };
  });
  mocks.readConfig.mockImplementation(async ({ body }: ReadConfigData) => ({
    data:
      body.key === 'BIOROUTER_PROVIDER'
        ? 'anthropic'
        : saved.has(body.key)
          ? { maskedValue: MASK }
          : null,
  }));
});

afterEach(() => {
  delete document.documentElement.dataset.biorouterSurface;
  resetProviderSetupAccessForTests();
});

describe.each(['desktop', 'serve'] as const)('%s saved-key display', (surface) => {
  it.each(savedCases)(
    'uses only the same fixed mask for %s, without retrieval',
    async (_, value) => {
      saved.set(KEY, value);
      if (surface === 'serve') {
        document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
        await loadProviderSetupAccess();
        mocks.headers.mockResolvedValue({ 'X-Caller-Provider': 'anthropic' });
      }
      const { input } = await openProvider();
      await act(async () => {});

      expect(input.value || input.placeholder).toBe(MASK);
      expect(mocks.previewProviderSecret).not.toHaveBeenCalled();
      expect(screen.queryByRole('alert')).toBeNull();
      expect(saved.get(KEY)).toBe(value);
      expectNoCredentialMutation();
    }
  );
});

describe('local replacement of a saved key', () => {
  it('supports keyboard Edit or Replace and local Cancel with focus, without treating the mask as a key', async () => {
    const { user, onConfigured, provider } = await openProvider();
    savedKeyEditButton().focus();
    await user.keyboard('{Enter}');
    const candidate = (await screen.findByLabelText(LABEL)) as HTMLInputElement;
    expect(candidate).not.toHaveAttribute('readonly');
    expect(candidate).toHaveValue('');
    await waitFor(() => expect(candidate).toHaveFocus());
    expectNoCredentialMutation();
    await user.type(candidate, CANDIDATE);
    await user.click(within(keyField(candidate)).getByRole('button', { name: /cancel/i }));

    const masked = (await screen.findByLabelText(LABEL)) as HTMLInputElement;
    expect(masked.value || masked.placeholder).toBe(MASK);
    await waitFor(() => expect(savedKeyEditButton()).toHaveFocus());
    expect(saved.get(KEY)).toBe(WORKING_KEY);
    expectNoCredentialMutation();
    expect(mocks.checkProvider).not.toHaveBeenCalled();

    await user.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(onConfigured).toHaveBeenCalledWith(provider));
    expectNoCredentialMutation();
    expect(saved.get(KEY)).toBe(WORKING_KEY);
    expect(mocks.checkProvider).not.toHaveBeenCalledWith(
      expect.objectContaining({ body: expect.objectContaining({ candidate: { [KEY]: MASK } }) })
    );
  });

  it('keeps a rejected candidate and useful recovery feedback without losing the working key', async () => {
    const refusal = 'Anthropic rejected the synthetic candidate.';
    mocks.checkProvider.mockRejectedValueOnce(refusal);
    const { user, onConfigured } = await openProvider();
    const candidate = await enterReplacement(user);
    await user.type(candidate, CANDIDATE);
    await user.click(screen.getByRole('button', { name: 'Save' }));

    expect(await screen.findByText(refusal)).toBeInTheDocument();
    expect(mocks.checkProvider).toHaveBeenCalledWith(
      expect.objectContaining({
        body: { provider: 'anthropic', live: true, candidate: { [KEY]: CANDIDATE } },
      })
    );
    expect(saved.get(KEY)).toBe(WORKING_KEY);
    expectNoCredentialMutation();
    expect(onConfigured).not.toHaveBeenCalled();
    await user.click(screen.getByRole('button', { name: 'Go back' }));
    const recovered = (await screen.findByLabelText(LABEL)) as HTMLInputElement;
    expect(recovered).toHaveValue(CANDIDATE);
    expect(recovered).not.toHaveAttribute('readonly');
    await waitFor(() => expect(recovered).toHaveFocus());
  });

  it('shows a busy check and persists only a successfully checked replacement, without deletion', async () => {
    const release = holdNextCheck();
    try {
      const { user, provider, onConfigured } = await openProvider();
      const candidate = await enterReplacement(user);
      await user.type(candidate, CANDIDATE);
      await user.click(screen.getByRole('button', { name: 'Save' }));
      await waitFor(() => expect(mocks.checkProvider).toHaveBeenCalledOnce());
      expect(screen.getByRole('button', { name: 'Checking…' })).toBeDisabled();
      expect(modalCancel(candidate)).toBeEnabled();
      expect(saved.get(KEY)).toBe(WORKING_KEY);
      expectNoCredentialMutation();

      await act(async () => release());
      await waitFor(() => expect(onConfigured).toHaveBeenCalledWith(provider));
      expect(mocks.upsert).toHaveBeenCalledExactlyOnceWith(KEY, CANDIDATE, true);
      expect(saved.get(KEY)).toBe(CANDIDATE);
      expect(mocks.remove).not.toHaveBeenCalled();
      expect(mocks.removeConfig).not.toHaveBeenCalled();
    } finally {
      await act(async () => release());
    }
  });

  it.each(['Cancel', 'Escape'] as const)(
    'abandons a pending replacement on modal %s and returns focus without a late write',
    async (close) => {
      const release = holdNextCheck();
      try {
        const { user, opener, onClose, onConfigured } = await openProvider();
        const candidate = await enterReplacement(user);
        await user.type(candidate, CANDIDATE);
        await user.click(screen.getByRole('button', { name: 'Save' }));
        await waitFor(() => expect(mocks.checkProvider).toHaveBeenCalledOnce());
        if (close === 'Cancel') {
          await user.click(modalCancel(candidate));
        } else {
          await user.keyboard('{Escape}');
        }
        await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
        expect(onClose).toHaveBeenCalledOnce();
        await waitFor(() => expect(opener).toHaveFocus());
        await act(async () => release());
        expect(saved.get(KEY)).toBe(WORKING_KEY);
        expectNoCredentialMutation();
        expect(onConfigured).not.toHaveBeenCalled();
      } finally {
        await act(async () => release());
      }
    }
  );
});

describe('pending replacement header preflight', () => {
  it.each(['Cancel', 'Escape'] as const)(
    'invalidates a pending replacement on modal %s without a late write',
    async (close) => {
      let releaseHeaders!: () => void;
      let markHeadersReached!: () => void;
      const headersPending = new Promise<void>((resolve) => {
        releaseHeaders = resolve;
      });
      const headersReached = new Promise<void>((resolve) => {
        markHeadersReached = resolve;
      });
      mocks.headers.mockImplementationOnce(async () => {
        markHeadersReached();
        await headersPending;
        return { 'X-User-Action': 'synthetic-user-action' };
      });

      try {
        const { user, opener, onClose, onConfigured } = await openProvider();
        const masked = screen.getByLabelText(LABEL) as HTMLInputElement;
        expect(masked.value).toBe(MASK);
        expect(masked).toHaveAttribute('readonly');
        expect(mocks.previewProviderSecret).not.toHaveBeenCalled();

        await user.click(screen.getByRole('button', { name: `Edit ${LABEL}` }));
        const candidate = (await screen.findByLabelText(LABEL)) as HTMLInputElement;
        expect(candidate).toHaveValue('');
        await user.type(candidate, CANDIDATE);
        await user.click(screen.getByRole('button', { name: 'Save' }));
        await headersReached;

        expect(mocks.checkProvider).not.toHaveBeenCalled();
        expect(screen.getByRole('button', { name: 'Checking…' })).toBeDisabled();
        expect(candidate).toBeDisabled();
        expect(screen.getByRole('button', { name: `Show ${LABEL}` })).toBeDisabled();
        expect(
          within(keyField(candidate)).getByRole('button', { name: /^Cancel$/ })
        ).toBeDisabled();
        expect(
          within(keyField(candidate)).getByRole('button', { name: `Remove ${LABEL}` })
        ).toBeDisabled();
        expect(modalCancel(candidate)).toBeEnabled();

        if (close === 'Cancel') {
          await user.click(modalCancel(candidate));
        } else {
          await user.keyboard('{Escape}');
        }
        await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
        expect(onClose).toHaveBeenCalledOnce();
        await waitFor(() => expect(opener).toHaveFocus());

        await act(async () => {
          releaseHeaders();
          await Promise.resolve();
        });
        await waitFor(() => expect(mocks.checkProvider).toHaveBeenCalledOnce());
        expect(saved.get(KEY)).toBe(WORKING_KEY);
        expectNoCredentialMutation();
        expect(onConfigured).not.toHaveBeenCalled();
        expect(screen.queryByRole('dialog')).toBeNull();
        expect(screen.queryByRole('alert')).toBeNull();
        expect(mocks.previewProviderSecret).not.toHaveBeenCalled();
      } finally {
        await act(async () => releaseHeaders());
      }
    }
  );
});

it('keeps Add usable with required feedback, candidate visibility and busy validation before saving', async () => {
  saved.clear();
  const release = holdNextCheck();
  try {
    const { input, user, provider, onConfigured } = await openProvider(false);
    expect(input).toHaveValue('');
    expect(input).not.toHaveAttribute('readonly');
    await user.click(screen.getByRole('button', { name: 'Save' }));
    expect(await screen.findByText('Anthropic API key is required')).toBeInTheDocument();
    expect(input).toHaveAttribute('aria-invalid', 'true');
    expect(mocks.checkProvider).not.toHaveBeenCalled();
    expectNoCredentialMutation();

    await user.type(input, CANDIDATE);
    expect(input).toHaveAttribute('type', 'password');
    await user.click(screen.getByRole('button', { name: 'Show Anthropic API key' }));
    expect(input).toHaveAttribute('type', 'text');
    expect(input).toHaveValue(CANDIDATE);
    await user.click(screen.getByRole('button', { name: 'Hide Anthropic API key' }));
    expect(input).toHaveAttribute('type', 'password');
    await user.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(mocks.checkProvider).toHaveBeenCalledOnce());
    expect(screen.getByRole('button', { name: 'Checking…' })).toBeDisabled();
    expect(saved.has(KEY)).toBe(false);
    expectNoCredentialMutation();

    await act(async () => release());
    await waitFor(() => expect(onConfigured).toHaveBeenCalledWith(provider));
    expect(mocks.upsert).toHaveBeenCalledExactlyOnceWith(KEY, CANDIDATE, true);
    expect(saved.get(KEY)).toBe(CANDIDATE);
    expect(mocks.remove).not.toHaveBeenCalled();
    expect(mocks.removeConfig).not.toHaveBeenCalled();
    expect(mocks.previewProviderSecret).not.toHaveBeenCalled();
  } finally {
    await act(async () => release());
  }
});
