import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ModelsBottomBar from './ModelsBottomBar';
import { __resetDisclosureStoreForTests } from '../../../privacy/disclosureCopy';

const mocks = vi.hoisted(() => ({
  read: vi.fn(async () => ''),
  getProviders: vi.fn(async () => [] as unknown[]),
  // Task 30A: mutable, because the disclosure line depends on the tier of the
  // PROVIDER bound to the chat, not on the chat's own classification — a fresh
  // chat on Versa is classified `public` and its model is emphatically not.
  currentProvider: 'versa_azure',
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

// ⚠ `usePrivacyTiersEnabled` as well as `useConfig`. `PrivacyBadge` reads the
// master switch itself (issue #56, DR-15) rather than making its nine call
// sites remember a prop, so any test that mocks this module AND renders a badge
// owes both. Omitting it fails loudly — "usePrivacyTiersEnabled is not a
// function" at the badge's own line — which is the failure mode to want; the
// alternative was a prop that ships unpassed, and it did.
vi.mock('../../../ConfigContext', () => ({
  useConfig: () => ({ read: mocks.read, getProviders: mocks.getProviders }),
  usePrivacyTiersEnabled: () => true,
}));

vi.mock('../../../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    currentModel: 'claude-opus-4',
    currentProvider: mocks.currentProvider,
    getCurrentModelAndProviderForDisplay: async () => ({
      model: 'claude-opus-4',
      provider: 'Versa',
    }),
    getCurrentModelDisplayName: async () => 'claude-opus-4',
    getCurrentProviderDisplayName: async () => 'Versa',
  }),
}));

vi.mock('../subcomponents/SwitchModelModal', () => ({ SwitchModelModal: () => null }));
vi.mock('../subcomponents/LeadWorkerSettings', () => ({ LeadWorkerSettings: () => null }));

// `RefObject<HTMLDivElement>` is non-nullable in this React version, and the
// bar only forwards it to a wrapper `div`.
const dropdownRef = { current: null } as unknown as React.RefObject<HTMLDivElement>;

function renderBar(privacyTier?: 'public' | 'private') {
  return render(
    <ModelsBottomBar
      sessionId="s1"
      privacyTier={privacyTier}
      dropdownRef={dropdownRef}
      setView={vi.fn()}
      alerts={[]}
    />
  );
}

const providerEntry = (
  name: string,
  tier: 'private' | 'public',
  // Issue #56 DR-26: served BESIDE the metadata, never inside it. The metadata's
  // tier is the type-level claim; the affiliation is resolved by the daemon from
  // a live instance, which is what makes it safe to state as a badge.
  affiliation?: unknown,
  // The instance-resolved tier, likewise beside the metadata. Defaults to
  // agreeing with the type-level claim because that is the ordinary case; pass
  // it explicitly to build the row where they DISAGREE, which is the only shape
  // that can tell the two fields apart.
  resolvedTier?: 'private' | 'public' | null
) => ({
  name,
  is_configured: true,
  provider_type: 'Builtin',
  metadata: { name, display_name: name, tier, runs_locally: tier === 'private' },
  affiliation: affiliation ?? null,
  resolved_tier: resolvedTier === undefined ? tier : resolvedTier,
});

const ucsfAffiliation = {
  kind: 'institutions',
  institutions: [{ id: 'ucsf', display_name: 'UCSF' }],
};

describe('ModelsBottomBar — the chip carries the dense padlock, never a pill', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // The served copy is held in module state (one install, one disclosure), so
    // it outlives `cleanup()` and has to be dropped between tests.
    __resetDisclosureStoreForTests();
    mocks.currentProvider = 'versa_azure';
    mocks.getProviders.mockResolvedValue([
      providerEntry('versa_azure', 'private', ucsfAffiliation),
      providerEntry('openai', 'public'),
    ]);
    mocks.getPrivacyDisclosure.mockResolvedValue({
      data: {
        title_template: '{provider} is not hosted by your institution.',
        long: 'SERVED-LONG-MARKER',
        short: 'SERVED-SHORT-MARKER — this model can read files on this computer.',
        acknowledged: true,
      },
    });
  });

  it('marks a private MODEL with the dense badge and no added text', async () => {
    renderBar('private');

    const badge = await screen.findByTestId('privacy-badge');
    expect(badge).toHaveAttribute('data-privacy', 'private');
    // The trigger is `max-w-[120px]` with a 24-character truncation: a "Private"
    // pill cannot fit, so the dense form is the only one this surface may use.
    expect(badge).not.toHaveTextContent(/Private/);
  });

  /**
   * ⚠ Rendered into a PRIVATE chat deliberately. A public model in a public
   * chat draws nothing under either the old rule or the new one, so it cannot
   * tell them apart; a public model in a private chat is the one arrangement
   * where the old rule drew a Private dot beside a public model's name — and it
   * is also the pairing Gate C refuses, so it is the arrangement most worth
   * showing honestly.
   */
  it('leaves a public model unmarked even in a private chat', async () => {
    mocks.currentProvider = 'openai';
    renderBar('private');
    fireEvent.pointerDown(screen.getByLabelText(/Current model/), { button: 0, ctrlKey: false });
    // Settle the same effect the dot would have arrived on, so this asserts the
    // resolved state rather than winning a race against it.
    await screen.findByTestId('non-private-model-chip-note');
    expect(screen.queryByTestId('privacy-badge')).toBeNull();
  });

  /**
   * ⚠ **The regression this chip shipped with, and the reason the badge moved.**
   *
   * A session is created `public` and its classification ratchets at the start
   * of its FIRST turn. So in every brand-new chat — and every chat predating
   * the feature — a UCSF Versa model resolving Private was drawn with the
   * chat's public dot beside its name, under a tooltip reading
   * `gpt-5.5-… · Public chat`, and the only subject in reach was the model.
   *
   * The mark is the model's now. `SessionNamePill` carries the chat's, in the
   * full pill, at the top of the chat — so nothing was lost and the duplicate
   * that was misattributing itself is gone.
   *
   * ⚠ This test fails against the old implementation, which is the whole point:
   * `privacyTier='public'` with a private model is exactly the disagreement.
   */
  it('draws the MODEL’s tier when the chat’s ratchet has not caught up', async () => {
    mocks.currentProvider = 'versa_azure';
    renderBar('public');

    const badge = await screen.findByTestId('privacy-badge');
    expect(badge).toHaveAttribute('data-privacy', 'private');
  });

  /**
   * ⚠ **`resolved_tier`, never `metadata.tier`.** The metadata's tier is what
   * the provider MODULE ships; `providers/base.rs` says in as many words not to
   * hang a badge on it, because an `ollama` re-pointed off this machine by
   * `OLLAMA_HOST` still ships `private` there while its instance resolves
   * `public`. A badge on the type-level field reads Private in precisely the
   * demotion case the tier exists to catch.
   */
  it('honours the instance’s demotion over the module’s claim', async () => {
    mocks.currentProvider = 'ollama';
    mocks.getProviders.mockResolvedValue([
      // Ships `private`, resolves `public` — a re-pointed endpoint.
      providerEntry('ollama', 'private', null, 'public'),
    ]);
    renderBar('private');

    fireEvent.pointerDown(screen.getByLabelText(/Current model/), { button: 0, ctrlKey: false });
    expect(await screen.findByText(/Public model/i)).toBeInTheDocument();
    expect(screen.queryByTestId('privacy-badge')).toBeNull();
  });

  /**
   * ⚠ **Unresolved is not public.** A daemon older than `resolved_tier`, a
   * provider that failed to construct, and a catalog that would not load all
   * arrive as the same absent field. Drawing the public dot for any of them
   * would state a fact about a model nobody resolved — and would do it in the
   * permissive direction.
   */
  it('draws no tier at all when the daemon does not serve one', async () => {
    mocks.currentProvider = 'versa_azure';
    mocks.getProviders.mockResolvedValue([
      providerEntry('versa_azure', 'private', ucsfAffiliation, null),
    ]);
    renderBar('public');

    // The affiliation off the same row still lands, so this waits on the very
    // effect that would have set a tier had one been served.
    await screen.findByTestId('affiliation-badge');
    expect(screen.queryByTestId('privacy-badge')).toBeNull();

    // ⚠ The dense badge alone cannot carry this assertion: its public form is
    // nothing at all, so a parser defaulting `undefined` to 'public' would draw
    // exactly the same emptiness. The WORD is what separates unresolved from
    // resolved-public, so the header is where the claim has to be checked.
    fireEvent.pointerDown(screen.getByLabelText(/Current model/), { button: 0, ctrlKey: false });
    await screen.findByText(/Current model/);
    expect(screen.queryByText(/Public model|Private model/i)).toBeNull();
  });

  /**
   * The accessible name is where the misattribution was most literal:
   * `Current model: … (Public chat) (Affiliation: UCSF)` put the CHAT's tier
   * between the model's name and the model's institution. The parenthetical is
   * the model's now, and the chat's tier follows a full stop.
   */
  it('attributes each tier to its own subject in the accessible name', async () => {
    mocks.currentProvider = 'versa_azure';
    renderBar('public');

    const trigger = await screen.findByLabelText(/Private model/);
    const label = trigger.getAttribute('aria-label') ?? '';
    expect(label).toBe('Current model: claude-opus-4 (Private model, UCSF). Public chat');
    // The exact failure: the chat's tier standing inside the model's clause.
    expect(label).not.toMatch(/\(Public chat/);
  });

  it('says both words in the dropdown header, where there is room for them', async () => {
    renderBar('private');
    fireEvent.pointerDown(screen.getByLabelText(/Current model/), {
      button: 0,
      ctrlKey: false,
    });

    // The model's tier, under the heading that reads "Current model"...
    expect(await screen.findByText(/Private model/i)).toBeInTheDocument();
    // ...and the chat's, set apart below it.
    expect(await screen.findByText(/Private chat/i)).toBeInTheDocument();
  });

  /**
   * Task 30A (issue #56, DR-17 requirement 3). The chip is where a user looks
   * to see which model they are talking to, so it is where the one-line
   * disclosure belongs.
   *
   * ⚠ It hangs off the bound PROVIDER's tier, never off `privacyTier`. That
   * prop is the chat's ratcheted CLASSIFICATION: a fresh chat on Versa is
   * classified `public` and its model is emphatically not a public model, so a
   * line keyed on it would tell the user something false about Versa.
   */
  it('a public model gets the one-line disclosure in the dropdown', async () => {
    mocks.currentProvider = 'openai';
    renderBar('public');
    fireEvent.pointerDown(screen.getByLabelText(/Current model/), { button: 0, ctrlKey: false });
    expect(await screen.findByTestId('non-private-model-chip-note')).toHaveTextContent(
      /SERVED-SHORT-MARKER/
    );
  });

  it('an institutional model does NOT, even while the chat itself is still public', async () => {
    mocks.currentProvider = 'versa_azure';
    renderBar('public');
    fireEvent.pointerDown(screen.getByLabelText(/Current model/), { button: 0, ctrlKey: false });
    await screen.findByText(/Current model/);
    expect(screen.queryByTestId('non-private-model-chip-note')).toBeNull();
  });

  /**
   * Issue #56, DR-26 — the second axis on the composer.
   *
   * ⚠ It hangs off the bound PROVIDER's row, exactly as the disclosure above
   * does and for the same reason: `privacyTier` is the chat's ratcheted
   * classification, and a fresh chat on Versa is classified `public` while its
   * model is covered by UCSF's agreements. A chip keyed on the chat's tier would
   * say nothing about the institution on precisely the chat where it matters.
   */
  it('marks the bound model’s institution on the chip, and says it in the dropdown', async () => {
    mocks.currentProvider = 'versa_azure';
    renderBar('public');

    const dense = await screen.findByTestId('affiliation-badge');
    expect(dense).toHaveAttribute('data-affiliation', 'institutions');
    expect(dense.getAttribute('aria-label')).toContain('UCSF');

    fireEvent.pointerDown(screen.getByLabelText(/Current model/), { button: 0, ctrlKey: false });
    // The dropdown header is where the WORD goes — the trigger is
    // `max-w-[120px]` and already truncating the model name.
    const badges = await screen.findAllByTestId('affiliation-badge');
    expect(badges.some((badge) => /UCSF/.test(badge.textContent ?? ''))).toBe(true);
  });

  /**
   * ⚠ **A public model has no affiliation at all, and renders none.** Not an
   * empty chip: a constraint-shaped ornament on the one kind of model that has
   * none of the private tier's protections is worse than silence.
   */
  it('shows no affiliation for a public model', async () => {
    mocks.currentProvider = 'openai';
    renderBar('public');
    // Wait for the same effect the affiliation would have arrived on, so this is
    // an assertion about the resolved state rather than about a race.
    fireEvent.pointerDown(screen.getByLabelText(/Current model/), { button: 0, ctrlKey: false });
    await screen.findByTestId('non-private-model-chip-note');
    expect(screen.queryByTestId('affiliation-badge')).toBeNull();
  });
});
