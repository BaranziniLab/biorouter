import { act, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ModelsBottomBar from './ModelsBottomBar';
import { __resetDisclosureStoreForTests } from '../../../privacy/disclosureCopy';

/**
 * V2-R8-1. The model chip states its model's tier off the catalog row of the
 * provider it NAMES, in every render.
 *
 * Measured in the running app: after a reload, Home on a private model
 * (`versa_azure`), a public chat opened from the sidebar (`aws_bedrock`) read
 * "claude-sonnet-4-6" with the Private padlock for about 70 ms, then dropped it
 * and the name moved 24px. The tier was a state hook the catalog read
 * overwrote once it landed, so between the binding changing and that read the
 * chip hung the previous provider's padlock on the new model's name.
 */
const mocks = vi.hoisted(() => ({
  read: vi.fn(async () => ''),
  getProviders: vi.fn(async (_force?: boolean) => [] as unknown[]),
  getPrivacyDisclosure: vi.fn(),
  ackPrivacyDisclosure: vi.fn(),
}));

vi.mock('../../../../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  getPrivacyDisclosure: mocks.getPrivacyDisclosure,
  ackPrivacyDisclosure: mocks.ackPrivacyDisclosure,
}));
vi.mock('../../../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-key' }),
}));
vi.mock('../../../ConfigContext', () => ({
  useConfig: () => ({ read: mocks.read, getProviders: mocks.getProviders }),
  usePrivacyTiersEnabled: () => true,
}));
// The app-wide selection, as on Home in the measured case: a private model.
vi.mock('../../../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    currentModel: 'gpt-5.5-2026-04-24',
    currentProvider: 'versa_azure',
    getCurrentModelAndProviderForDisplay: async () => ({
      model: 'gpt-5.5-2026-04-24',
      provider: 'Versa API Azure',
    }),
    getCurrentModelDisplayName: async () => 'gpt-5.5-2026-04-24',
    getCurrentProviderDisplayName: async () => 'Versa API Azure',
  }),
}));
vi.mock('../subcomponents/SwitchModelModal', () => ({ SwitchModelModal: () => null }));
vi.mock('../subcomponents/LeadWorkerSettings', () => ({ LeadWorkerSettings: () => null }));

const dropdownRef = { current: null } as unknown as React.RefObject<HTMLDivElement>;

const providerEntry = (name: string, display: string, tier: 'private' | 'public') => ({
  name,
  is_configured: true,
  provider_type: 'Builtin',
  metadata: { name, display_name: display, tier, runs_locally: false },
  affiliation: null,
  resolved_tier: tier,
});

const VERSA = providerEntry('versa_azure', 'Versa API Azure', 'private');
const BEDROCK = providerEntry('aws_bedrock', 'Amazon Bedrock', 'public');
const OLLAMA = providerEntry('ollama', 'Ollama', 'private');

const GWAS = { provider: 'aws_bedrock', model: 'claude-sonnet-4-6' };
const LOCAL = { provider: 'ollama', model: 'qwen3.6' };

/** A read the test answers by hand, so a render can be inspected before it lands. */
function heldRead() {
  let answer!: (rows: unknown[]) => void;
  const promise = new Promise<unknown[]>((resolve) => {
    answer = resolve;
  });
  return { promise, answer };
}

function bar(sessionId: string | null, effectiveModel?: { provider: string; model: string }) {
  return (
    <ModelsBottomBar
      sessionId={sessionId}
      privacyTier={sessionId ? 'public' : undefined}
      effectiveModel={effectiveModel}
      dropdownRef={dropdownRef}
      setView={vi.fn()}
      alerts={[]}
    />
  );
}

/** Home on the private selection, its padlock settled. */
async function mountOnPrivateHome() {
  const view = render(bar(null));
  const chip = await screen.findByTestId('model-chip');
  await waitFor(() => expect(chip).toHaveAccessibleName(/Private model/));
  expect(screen.getByTestId('privacy-badge')).toBeInTheDocument();
  return view;
}

/** Every committed state of the chip: its name and whether it carries the padlock. */
function recordChip() {
  const frames: string[] = [];
  const snap = () => {
    const chip = screen.queryByTestId('model-chip');
    if (!chip) return;
    const name = chip.querySelector('.br-picker-chip__name')?.textContent ?? '';
    const lock = chip.querySelector('[data-testid="privacy-badge"]') ? ' +lock' : '';
    const frame = `${name}${lock}`;
    if (frames[frames.length - 1] !== frame) frames.push(frame);
  };
  snap();
  const observer = new MutationObserver(snap);
  observer.observe(document.body, { subtree: true, childList: true, characterData: true });
  return { frames, stop: () => observer.disconnect() };
}

beforeEach(() => {
  vi.clearAllMocks();
  __resetDisclosureStoreForTests();
  mocks.getProviders.mockResolvedValue([VERSA, BEDROCK, OLLAMA]);
  mocks.getPrivacyDisclosure.mockResolvedValue({
    data: {
      title_template: '{provider} is not hosted by your institution.',
      long: 'LONG',
      short: 'SHORT',
      acknowledged: true,
    },
  });
});

describe('V2-R8-1: a changed binding never wears the previous provider’s tier', () => {
  it('a public chat opened from a private Home has no padlock in the render that opens it', async () => {
    const view = await mountOnPrivateHome();
    // The chat's own read is held: whatever the chip says now, it says before it.
    const held = heldRead();
    mocks.getProviders.mockReturnValueOnce(held.promise);

    view.rerender(bar('gwas', GWAS));

    const chip = screen.getByTestId('model-chip');
    expect(chip).toHaveAccessibleName(/Current model: claude-sonnet-4-6/);
    expect(screen.queryByTestId('privacy-badge')).toBeNull();
    expect(chip).not.toHaveAccessibleName(/Private model/);
    // Home's read already listed Bedrock, so its own row is stated at once.
    expect(chip).toHaveAccessibleName(/Public model/);

    await act(async () => held.answer([VERSA, BEDROCK, OLLAMA]));
    expect(screen.queryByTestId('privacy-badge')).toBeNull();
    expect(chip).toHaveAccessibleName(/Public model/);
  });

  it('a provider this mount has not read yet states no tier until its own row answers', async () => {
    // Home's catalog does not list the chat's provider yet (configured since).
    mocks.getProviders.mockResolvedValueOnce([VERSA]);
    const view = await mountOnPrivateHome();
    const held = heldRead();
    mocks.getProviders.mockReturnValueOnce(held.promise);

    view.rerender(bar('gwas', GWAS));

    const chip = screen.getByTestId('model-chip');
    expect(chip).toHaveAccessibleName(/Current model: claude-sonnet-4-6/);
    expect(screen.queryByTestId('privacy-badge')).toBeNull();
    expect(chip).not.toHaveAccessibleName(/Private model/);
    expect(chip).not.toHaveAccessibleName(/Public model/);

    await act(async () => held.answer([VERSA, BEDROCK]));
    await waitFor(() => expect(chip).toHaveAccessibleName(/Public model/));
    expect(screen.queryByTestId('privacy-badge')).toBeNull();
  });

  it('a private provider not read yet gains its padlock from its own row, never from Home’s', async () => {
    mocks.getProviders.mockResolvedValueOnce([VERSA]);
    const view = await mountOnPrivateHome();
    const held = heldRead();
    mocks.getProviders.mockReturnValueOnce(held.promise);
    const record = recordChip();

    view.rerender(bar('local', LOCAL));
    expect(screen.queryByTestId('privacy-badge')).toBeNull();

    await act(async () => held.answer([VERSA, OLLAMA]));
    await waitFor(() =>
      expect(screen.getByTestId('model-chip')).toHaveAccessibleName(/Private model/)
    );
    record.stop();
    // Home's padlock, the chat's name with none, then the chat's own padlock.
    expect(record.frames).toEqual(['gpt-5.5 +lock', 'qwen3.6', 'qwen3.6 +lock']);
  });

  it('never commits the public chat’s name beside a padlock, there or back', async () => {
    const view = await mountOnPrivateHome();
    const record = recordChip();

    view.rerender(bar('gwas', GWAS));
    await act(async () => {});
    // Back to Home: Versa's own row, already read, so the padlock is back at once.
    const held = heldRead();
    mocks.getProviders.mockReturnValueOnce(held.promise);
    view.rerender(bar(null));
    expect(screen.getByTestId('privacy-badge')).toBeInTheDocument();
    await act(async () => held.answer([VERSA, BEDROCK, OLLAMA]));
    record.stop();

    expect(record.frames).toEqual(['gpt-5.5 +lock', 'claude-sonnet-4-6', 'gpt-5.5 +lock']);
  });
});
