import * as React from 'react';
import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ExtensionsPanel, useExtensionPicker } from './BottomMenuExtensionSelection';
import { EXTENSIONS_COPY } from './copy';

/**
 * The extensions list as the Tools chip holds it (spec 3.7). The session tier
 * prop is kept on the harness so each test still states the CHAT's tier beside
 * the model's: the list must judge on the model's and ignore the chat's.
 */
function BottomMenuExtensionSelection({
  sessionId,
}: {
  sessionId: string | null;
  privacyTier?: 'public' | 'private';
}) {
  const [open, setOpen] = React.useState(false);
  const picker = useExtensionPicker(sessionId, { open });
  return (
    <>
      <button
        type="button"
        aria-label={`Manage extensions (${picker.activeCount} added)`}
        onClick={() => setOpen(true)}
      />
      {open && <ExtensionsPanel picker={picker} />}
    </>
  );
}

const mocks = vi.hoisted(() => ({
  overrides: new Map<string, boolean>(),
  getSessionExtensions: vi.fn(async () => ({ data: { extensions: [] } })),
  addToAgent: vi.fn(async (): Promise<void> => undefined),
  removeFromAgent: vi.fn(async (): Promise<void> => undefined),
  /**
   * The `ProviderDetails` row `GET /config/providers` serves for the bound
   * provider. `resolved_tier` is the instance-resolved field the composer now
   * judges pairings on; `metadata.tier` is deliberately set to the OPPOSITE
   * value in the tests that care, so a component that read the type-level claim
   * instead would fail rather than coincidentally pass.
   */
  providerRow: undefined as unknown,
}));

vi.mock('../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({ currentProvider: 'versa_azure' }),
}));

vi.mock('../ConfigContext', () => ({
  useConfig: () => ({
    getProviders: async () => (mocks.providerRow ? [mocks.providerRow] : []),
    extensionsList: [
      {
        type: 'stdio',
        name: 'ucsfomopagent',
        display_name: 'UCSFOMOPAgent',
        description: 'UCSF OMOP clinical database',
        cmd: 'uv',
        args: [],
        enabled: true,
      },
      {
        type: 'stdio',
        name: 'example',
        display_name: 'Example',
        description: 'Example extension',
        cmd: 'example',
        args: [],
        enabled: true,
      },
    ],
  }),
}));

// Only the built-in predicate is stubbed: the NAME a row reads is the real resolver's, as in
// the sibling test file.
vi.mock('../settings/extensions/subcomponents/ExtensionList', () => ({
  isBuiltInExtension: () => false,
}));

vi.mock('../../api', () => ({ getSessionExtensions: mocks.getSessionExtensions }));

vi.mock('../settings/extensions/agent-api', () => ({
  addToAgent: mocks.addToAgent,
  removeFromAgent: mocks.removeFromAgent,
}));

vi.mock('../../store/extensionOverrides', () => ({
  setExtensionOverride: (name: string, enabled: boolean) => mocks.overrides.set(name, enabled),
  getExtensionOverrides: () => mocks.overrides,
}));

vi.mock('../../toasts', () => ({
  toastService: { success: vi.fn(), error: vi.fn() },
}));

/**
 * ⚠ A row writes `aria-disabled={disabled || undefined}`, so an ENABLED row
 * carries no such attribute at all — `toHaveAttribute('aria-disabled', 'false')`
 * fails against a perfectly good row. The negative assertions below therefore
 * read "not disabled", which is what the DOM can actually express.
 */
async function openMenu() {
  fireEvent.click(screen.getByLabelText(/Manage extensions/));
  await screen.findAllByRole('option');
}

/**
 * A bound provider whose INSTANCE resolved `resolved_tier`, with `metadata.tier`
 * pinned to the opposite value on purpose — see `mocks.providerRow`.
 */
function boundProvider(resolvedTier: 'public' | 'private' | undefined) {
  return {
    name: 'versa_azure',
    is_configured: true,
    resolved_tier: resolvedTier,
    metadata: { tier: resolvedTier === 'private' ? 'public' : 'private' },
  };
}

describe('BottomMenuExtensionSelection — the pairing, not the extension', () => {
  beforeEach(() => {
    mocks.overrides.clear();
    mocks.providerRow = boundProvider('public');
    vi.clearAllMocks();
  });

  it('a private extension is visible-but-disabled in the composer, never omitted', async () => {
    // Omission is what produces "the OMOP tool is broken". Gate C is invisible
    // in the GUI by construction — it returns ErrorData from inside
    // dispatch_tool_call, so it never enters PermissionCheckResult and produces
    // no approval card and no denial record.
    //
    // ⚠ VISIBLE is asserted first and on its own: an implementation that
    // FILTERED the row out would pass every "is it disabled" check by default.
    // The refused row must still be in the list, and the list must still be
    // whole.
    render(<BottomMenuExtensionSelection sessionId="s1" privacyTier="public" />);
    await openMenu();

    expect(screen.getAllByRole('option')).toHaveLength(2);
    expect(screen.getByText('UCSFOMOPAgent')).toBeInTheDocument();

    const item = screen.getByText('UCSFOMOPAgent').closest('[role="option"]')!;
    expect(item).toHaveAttribute('aria-disabled', 'true');
    // The refusal is a visible line on the row, never tooltip-only (principle 2).
    expect(item).toHaveTextContent(EXTENSIONS_COPY.pairingRefused);
    // A click on a refused row changes nothing.
    fireEvent.click(item);
    expect(mocks.addToAgent).not.toHaveBeenCalled();
    expect(mocks.removeFromAgent).not.toHaveBeenCalled();
  });

  it('leaves every other extension alone in the same chat', async () => {
    render(<BottomMenuExtensionSelection sessionId="s1" privacyTier="public" />);
    await openMenu();

    const item = screen.getByText('Example').closest('[role="option"]')!;
    expect(item).not.toHaveAttribute('aria-disabled', 'true');
    expect(item).not.toHaveTextContent(/public model/i);
  });

  it('leaves the private extension usable once the bound model is private', async () => {
    mocks.providerRow = boundProvider('private');
    render(<BottomMenuExtensionSelection sessionId="s1" privacyTier="private" />);
    await openMenu();

    const item = screen.getByText('UCSFOMOPAgent').closest('[role="option"]')!;
    expect(item).not.toHaveAttribute('aria-disabled', 'true');
    expect(item).not.toHaveTextContent(/public model/i);
  });

  /**
   * The reported defect, as a test: a chat the daemon still classifies `public`
   * — every chat that has not yet run a turn, because a session is created
   * public and the ratchet fires at the START of the first turn — bound to UCSF
   * Versa, whose instance resolves **Private**.
   *
   * Gate C judges `privacy_refusal(extension, ext_tier, cap.tier())` where
   * `cap.tier()` is `Provider::tier()` off the bound instance; the session row
   * is never consulted. So the daemon dispatches `ucsfomopagent` here without
   * complaint, and the composer that greyed it out with "(public model)" was
   * telling the user something false about UCSF's own model.
   *
   * ⚠ The session tier is passed as `public` DELIBERATELY. An implementation
   * that went back to reading the prop passes every other test in this file and
   * fails only this one.
   */
  it('does not call a private model public just because the chat has not ratcheted yet', async () => {
    mocks.providerRow = boundProvider('private');
    render(<BottomMenuExtensionSelection sessionId="s1" privacyTier="public" />);
    await openMenu();

    const item = screen.getByText('UCSFOMOPAgent').closest('[role="option"]')!;
    expect(item).not.toHaveAttribute('aria-disabled', 'true');
    expect(item).not.toHaveTextContent(/public model/i);
  });

  // A tier nobody could resolve is not a claim that the model is public. Walling
  // a working tool on a missing read is the failure this whole state exists to
  // prevent — and failing an unresolvable model over to "public" is the wrong
  // direction twice over.
  it('judges nothing when the bound model tier is unknown', async () => {
    mocks.providerRow = boundProvider(undefined);
    render(<BottomMenuExtensionSelection sessionId="s1" privacyTier="public" />);
    await openMenu();

    const item = screen.getByText('UCSFOMOPAgent').closest('[role="option"]')!;
    expect(item).not.toHaveAttribute('aria-disabled', 'true');
  });

  // The row may predate `resolved_tier`, or the provider may be one the daemon
  // could not construct. Neither is evidence of a tier.
  it('judges nothing when the provider row is absent entirely', async () => {
    mocks.providerRow = undefined;
    render(<BottomMenuExtensionSelection sessionId="s1" privacyTier="public" />);
    await openMenu();

    const item = screen.getByText('UCSFOMOPAgent').closest('[role="option"]')!;
    expect(item).not.toHaveAttribute('aria-disabled', 'true');
  });
});
