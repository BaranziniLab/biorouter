import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProviderDetails, ProviderTier } from '../../../../api';
import { SwitchModelModal } from './SwitchModelModal';

const mocks = vi.hoisted(() => ({
  getProviders: vi.fn(),
  getProviderModels: vi.fn(),
  read: vi.fn(),
  changeModel: vi.fn(),
  // Mutable so the predefined-models branch can be exercised in the same file
  // as the option-list one; `usePredefinedModels` is read once at mount, so
  // flipping this before `render` is enough.
  showPredefined: false,
  predefinedModels: [] as { name: string; provider: string; subtext?: string }[],
}));

vi.mock('../../../ConfigContext', () => ({
  useConfig: () => ({
    getProviders: mocks.getProviders,
    getProviderModels: mocks.getProviderModels,
    read: mocks.read,
  }),
}));

vi.mock('../../../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    changeModel: mocks.changeModel,
    // `null`, so the modal's own auto-select effect fills the field from the
    // fetched list — which is also how we know the options finished loading.
    currentModel: null,
    currentProvider: 'anthropic',
  }),
}));

vi.mock('../predefinedModelsUtils', () => ({
  getPredefinedModelsFromEnv: () => mocks.predefinedModels,
  shouldShowPredefinedModels: () => mocks.showPredefined,
}));

// ⚠ The sibling suite `SwitchModelModal.test.tsx` replaces `ui/Select` with a
// stub that renders only the *value*. Nothing in it can see an option row, let
// alone a disabled one, so this file deliberately runs the REAL react-select:
// `role="option"` and `aria-disabled` are react-select's own output, and they
// are exactly what the pre-flight state has to produce.

function provider(
  name: string,
  tier: ProviderTier | undefined,
  displayName = name,
  // Issue #56 DR-26. Served BESIDE the metadata, never inside it: the metadata's
  // tier is the type-level claim (which is why this file has a whole test about
  // an absent one), and the affiliation is resolved by the daemon from a live
  // instance. `undefined` is a provider with none — a public model.
  affiliation?: unknown
): ProviderDetails {
  return {
    name,
    is_configured: true,
    provider_type: 'Builtin',
    affiliation: affiliation ?? null,
    metadata: {
      config_keys: [],
      default_model: '',
      description: '',
      display_name: displayName,
      known_models: [],
      model_doc_link: '',
      name,
      tier,
      runs_locally: false,
    },
  } as ProviderDetails;
}

/**
 * Open the model menu. The provider combobox is first, the model one second.
 *
 * ⚠ Wait for the auto-selected model to appear first. react-select is
 * `isDisabled` while `loadingModels`, and an ArrowDown fired before the fetch
 * settles opens a menu with no options — which reads exactly like a missing
 * feature.
 */
async function openModelMenu() {
  await screen.findByText('Claude Opus 4.8');
  const combos = screen.getAllByRole('combobox');
  fireEvent.keyDown(combos[combos.length - 1], { key: 'ArrowDown', code: 'ArrowDown' });
}

describe('SwitchModelModal — pre-flight, not post-refusal', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.getProviders.mockResolvedValue([
      provider('anthropic', 'public', 'Anthropic'),
      provider('versa_azure', 'private', 'Versa'),
    ]);
    mocks.getProviderModels.mockResolvedValue(['Claude Opus 4.8']);
    mocks.read.mockResolvedValue('');
    mocks.changeModel.mockResolvedValue(true);
    mocks.showPredefined = false;
    mocks.predefinedModels = [];
  });

  it('a public model is disabled with its reason in a private chat', async () => {
    render(
      <SwitchModelModal sessionId="s1" privacyTier="private" onClose={vi.fn()} setView={vi.fn()} />
    );

    await openModelMenu();
    const row = await screen.findByRole('option', { name: /Claude Opus/ });
    expect(row).toHaveAttribute('aria-disabled', 'true');
    expect(row).toHaveTextContent(/private chat/i);
  });

  /**
   * F3 (QA of 7c96d796, 2026-09-10). A disabled ROW is not a disabled SELECTION.
   * The auto-select fills the field with the provider's first model without
   * asking `isOptionDisabled`, so opening this on a public provider in a private
   * chat put a barred model in the field — and "Select model" stayed live,
   * because validity started `true` and was first computed inside the click.
   * The click was refused, so the gate held; the pre-flight did not.
   *
   * ⚠ Fails against the code before the fix: the confirm is enabled and the
   * reason is nowhere on screen until something is clicked.
   */
  it('disables the confirm, with the reason beside it, before any click on a barred selection', async () => {
    render(
      <SwitchModelModal sessionId="s1" privacyTier="private" onClose={vi.fn()} setView={vi.fn()} />
    );

    // The auto-selected model: every row of this provider is barred here.
    await screen.findByText('Claude Opus 4.8');

    const confirm = screen.getByRole('button', { name: 'Select model' });
    expect(confirm).toBeDisabled();
    // The reason is on screen with the menu closed, and it is the confirm's
    // own description rather than a sentence that merely happens to be nearby.
    const reason = screen.getByText(/private chat, so only private models/i);
    expect(reason.id).not.toBe('');
    expect(confirm).toHaveAttribute('aria-describedby', reason.id);

    fireEvent.click(confirm);
    expect(mocks.changeModel).not.toHaveBeenCalled();
  });

  // "Every selection change", not "whatever was there at mount": moving the
  // same dialog onto a private provider has to bring the confirm back.
  it('re-validates when the selection moves off the barred provider', async () => {
    render(
      <SwitchModelModal sessionId="s1" privacyTier="private" onClose={vi.fn()} setView={vi.fn()} />
    );

    await screen.findByText('Claude Opus 4.8');
    const confirm = screen.getByRole('button', { name: 'Select model' });
    expect(confirm).toBeDisabled();

    fireEvent.change(screen.getAllByRole('combobox')[0], { target: { value: 'Versa' } });
    fireEvent.click(await screen.findByRole('option', { name: 'Versa' }));

    await waitFor(() => expect(confirm).toBeEnabled());
    expect(confirm).not.toHaveAttribute('aria-describedby');
    expect(screen.queryByText(/private chat/i)).toBeNull();
  });

  // The control for the two above: a private model in the same private chat
  // leaves the confirm live and says nothing, so the fix cannot have been
  // "disable the confirm in every private chat".
  it('leaves the confirm live for a private model in a private chat', async () => {
    render(
      <SwitchModelModal
        sessionId="s1"
        privacyTier="private"
        initialProvider="versa_azure"
        onClose={vi.fn()}
        setView={vi.fn()}
      />
    );

    await screen.findByText('Claude Opus 4.8');
    const confirm = screen.getByRole('button', { name: 'Select model' });
    await waitFor(() => expect(confirm).toBeEnabled());
    expect(screen.queryByText(/private chat/i)).toBeNull();

    fireEvent.click(confirm);
    await waitFor(() => expect(mocks.changeModel).toHaveBeenCalledTimes(1));
  });

  // Without this the assertion above passes for a modal that disables EVERY
  // row, which would be a worse bug than the one it is meant to catch.
  it('leaves the same row selectable in a public chat', async () => {
    render(
      <SwitchModelModal sessionId="s1" privacyTier="public" onClose={vi.fn()} setView={vi.fn()} />
    );

    await openModelMenu();
    const row = await screen.findByRole('option', { name: /Claude Opus/ });
    expect(row).toHaveAttribute('aria-disabled', 'false');
    expect(row).not.toHaveTextContent(/private chat/i);
  });

  // A tier the caller could not resolve is not an assertion that the chat is
  // public: the settings grid opens this modal with no session at all.
  it('judges nothing when the chat tier is unknown', async () => {
    render(<SwitchModelModal sessionId={null} onClose={vi.fn()} setView={vi.fn()} />);

    await openModelMenu();
    const row = await screen.findByRole('option', { name: /Claude Opus/ });
    expect(row).toHaveAttribute('aria-disabled', 'false');
  });

  // ⚠ An ABSENT tier is not an unknown one — it is Public, and it has to be
  // read that way here or the pre-flight silently stops covering it.
  //
  // `ProviderMetadata::tier` is `#[serde(default)]` over a `ProviderTier` whose
  // `Default` is deliberately `Public` ("fail-safe, not fail-open: a provider
  // module that forgets `tier()` gets less reach, never more"), which is why
  // the generated client types the field as optional. A daemon predating the
  // field, or any provider whose metadata omits it, therefore arrives here with
  // `tier === undefined` while the daemon resolves it to Public and
  // `available_private_providers` — which filters on `is_private()` — declines
  // to offer it. A `=== 'public'` test would leave that row selectable, then
  // hand the user a Gate A 409: the false-negative this whole task exists to
  // replace.
  it('treats a provider with no declared tier as public, exactly as the daemon does', async () => {
    mocks.getProviders.mockResolvedValue([
      provider('mystery', undefined, 'Mystery'),
      provider('versa_azure', 'private', 'Versa'),
    ]);

    render(
      <SwitchModelModal
        sessionId="s1"
        privacyTier="private"
        initialProvider="mystery"
        onClose={vi.fn()}
        setView={vi.fn()}
      />
    );

    await openModelMenu();
    const row = await screen.findByRole('option', { name: /Claude Opus/ });
    expect(row).toHaveAttribute('aria-disabled', 'true');
    expect(row).toHaveTextContent(/private chat/i);
  });

  /**
   * Issue #56, DR-26 — the third axis, stated in the picker BEFORE the switch.
   *
   * ⚠ The tier is deliberately NOT badged on this surface: the only tier the
   * modal has is `metadata.tier`, the type-level claim, and a Private pill hung
   * on it would read Private for an `ollama` re-pointed off this machine. The
   * affiliation is resolved by the daemon from a live instance, so it can be
   * stated outright — and its absence for a public provider is equally
   * meaningful.
   */
  it('states the selected provider’s institution before the switch', async () => {
    mocks.getProviders.mockResolvedValue([
      provider('anthropic', 'public', 'Anthropic'),
      provider('versa_azure', 'private', 'Versa', {
        kind: 'institutions',
        institutions: [{ id: 'ucsf', display_name: 'UCSF' }],
      }),
    ]);

    render(
      <SwitchModelModal
        sessionId="s1"
        privacyTier="private"
        initialProvider="versa_azure"
        onClose={vi.fn()}
        setView={vi.fn()}
      />
    );

    const row = await screen.findByTestId('switch-model-affiliation');
    expect(row).toHaveTextContent('UCSF');
    expect(row).toHaveTextContent(/compliance does not transfer/i);
  });

  it('does not submit the old model while a different provider search is uncommitted', async () => {
    mocks.getProviders.mockResolvedValue([
      provider('versa_azure', 'private', 'Versa API Azure', {
        kind: 'institutions',
        institutions: [{ id: 'ucsf', display_name: 'UCSF' }],
      }),
      provider('codex', 'public', 'Codex'),
    ]);
    mocks.getProviderModels.mockResolvedValue(['gpt-5.5-2026-04-24']);

    render(
      <SwitchModelModal
        sessionId="s1"
        privacyTier="public"
        initialProvider="versa_azure"
        initialModel="gpt-5.5-2026-04-24"
        onClose={vi.fn()}
        setView={vi.fn()}
      />
    );

    expect(await screen.findByTestId('switch-model-affiliation')).toHaveTextContent('UCSF');
    const providerInput = screen.getAllByRole('combobox')[0];
    fireEvent.change(providerInput, { target: { value: 'Codex' } });
    expect(await screen.findByRole('option', { name: 'Codex' })).toBeInTheDocument();

    const confirm = screen.getByRole('button', { name: 'Select model' });
    expect(confirm).toBeDisabled();
    fireEvent.click(confirm);
    expect(mocks.changeModel).not.toHaveBeenCalled();
  });

  it('submits the exact provider and model after the filtered provider is committed', async () => {
    mocks.getProviders.mockResolvedValue([
      provider('versa_azure', 'private', 'Versa API Azure', {
        kind: 'institutions',
        institutions: [{ id: 'ucsf', display_name: 'UCSF' }],
      }),
      provider('codex', 'public', 'Codex'),
    ]);
    let resolveCodex!: (models: string[]) => void;
    mocks.getProviderModels.mockImplementation((providerName: string) => {
      if (providerName !== 'codex') return Promise.resolve(['gpt-5.5-2026-04-24']);
      return new Promise<string[]>((resolve) => {
        resolveCodex = resolve;
      });
    });

    render(
      <SwitchModelModal
        sessionId="s1"
        privacyTier="public"
        initialProvider="versa_azure"
        initialModel="gpt-5.5-2026-04-24"
        onClose={vi.fn()}
        setView={vi.fn()}
      />
    );

    expect(await screen.findByTestId('switch-model-affiliation')).toHaveTextContent('UCSF');
    const providerInput = screen.getAllByRole('combobox')[0];
    fireEvent.change(providerInput, { target: { value: 'Codex' } });
    fireEvent.click(await screen.findByRole('option', { name: 'Codex' }));

    await waitFor(() => expect(screen.queryByTestId('switch-model-affiliation')).toBeNull());
    const confirm = screen.getByRole('button', { name: 'Select model' });
    expect(confirm).toBeDisabled();

    await act(async () => resolveCodex(['gpt-5.6-sol']));
    expect(await screen.findByText('gpt-5.6-sol')).toBeInTheDocument();
    expect(confirm).toBeEnabled();
    fireEvent.click(confirm);

    await waitFor(() => expect(mocks.changeModel).toHaveBeenCalledTimes(1));
    expect(mocks.changeModel).toHaveBeenCalledWith('s1', {
      name: 'gpt-5.6-sol',
      provider: 'codex',
      subtext: 'Codex',
    });
  });

  // The control case: a public provider has no affiliation at all, so the row is
  // absent rather than empty. Without this, the assertion above passes for a
  // modal that prints a chip for every provider.
  it('says nothing about affiliation for a public provider', async () => {
    render(
      <SwitchModelModal
        sessionId="s1"
        privacyTier="public"
        initialProvider="anthropic"
        onClose={vi.fn()}
        setView={vi.fn()}
      />
    );

    await openModelMenu();
    expect(screen.queryByTestId('switch-model-affiliation')).toBeNull();
  });

  /**
   * ⚠ **`local` is the MOST permissive affiliation, not the narrowest.** A local
   * model reaches every private extension, because no transfer occurs at all. A
   * picker that rendered it as a narrower-sounding institution beside UCSF would
   * invert the axis for the one user who most needs it right.
   */
  it('does not render a local model as a narrower institution', async () => {
    mocks.getProviders.mockResolvedValue([
      provider('llamacpp', 'private', 'Llama Server', { kind: 'local', institutions: [] }),
      provider('versa_azure', 'private', 'Versa'),
    ]);

    render(
      <SwitchModelModal
        sessionId="s1"
        privacyTier="private"
        initialProvider="llamacpp"
        onClose={vi.fn()}
        setView={vi.fn()}
      />
    );

    const row = await screen.findByTestId('switch-model-affiliation');
    expect(row).toHaveTextContent('On this machine');
    expect(row).toHaveTextContent(/least restricted/i);
    expect(row).toHaveTextContent(/every private extension/i);
  });

  // The other half of the same predicate: a declared-private provider stays
  // selectable, so the change above cannot have been "block everything".
  it('leaves a declared-private provider selectable in a private chat', async () => {
    render(
      <SwitchModelModal
        sessionId="s1"
        privacyTier="private"
        initialProvider="versa_azure"
        onClose={vi.fn()}
        setView={vi.fn()}
      />
    );

    await openModelMenu();
    const row = await screen.findByRole('option', { name: /Claude Opus/ });
    expect(row).toHaveAttribute('aria-disabled', 'false');
  });
});

/**
 * `shouldShowPredefinedModels()` swaps the provider/model selects for a flat
 * radio list, and that branch reaches the SAME `changeModel` call. The custom
 * model field was guarded in `validateForm` precisely because it bypassed the
 * option list; this list bypasses it in exactly the same way, so leaving it
 * unguarded reopens the hole on the sibling path.
 *
 * Gate A still refuses the bind, so this is a missing warning rather than a
 * leak — but a missing warning is the entire defect this task set out to fix.
 */
describe('SwitchModelModal — the predefined-model list is the same pre-flight', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.getProviders.mockResolvedValue([
      provider('anthropic', 'public', 'Anthropic'),
      provider('versa_azure', 'private', 'Versa'),
    ]);
    mocks.getProviderModels.mockResolvedValue(['Claude Opus 4.8']);
    mocks.read.mockResolvedValue('');
    mocks.changeModel.mockResolvedValue(true);
    mocks.showPredefined = true;
    mocks.predefinedModels = [
      { name: 'claude-opus-4-8', provider: 'anthropic' },
      { name: 'versa-gpt', provider: 'versa_azure' },
    ];
  });

  it('refuses a public predefined model in a private chat, with the same reason', async () => {
    render(
      <SwitchModelModal sessionId="s1" privacyTier="private" onClose={vi.fn()} setView={vi.fn()} />
    );

    fireEvent.click(await screen.findByText('claude-opus-4-8'));
    fireEvent.click(screen.getByRole('button', { name: 'Select model' }));

    expect(await screen.findByText(/private chat/i)).toBeInTheDocument();
    expect(mocks.changeModel).not.toHaveBeenCalled();
  });

  // The negative: the guard must key on the SELECTED model's provider, not
  // simply refuse every submit while the chat is private.
  it('accepts a private predefined model in the same private chat', async () => {
    render(
      <SwitchModelModal sessionId="s1" privacyTier="private" onClose={vi.fn()} setView={vi.fn()} />
    );

    fireEvent.click(await screen.findByText('versa-gpt'));
    fireEvent.click(screen.getByRole('button', { name: 'Select model' }));

    await waitFor(() => expect(mocks.changeModel).toHaveBeenCalled());
    expect(screen.queryByText(/private chat/i)).toBeNull();
  });
});
