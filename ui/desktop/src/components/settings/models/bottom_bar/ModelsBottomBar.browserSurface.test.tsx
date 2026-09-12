import { fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import ModelsBottomBar from './ModelsBottomBar';
import { __resetDisclosureStoreForTests } from '../../../privacy/disclosureCopy';
import { BROWSER_SURFACE_MARKER } from '../../../../utils/surface';

/**
 * SD-1 at the composer's model chip — the surface a browser user is most likely
 * to reach first, because it is the only model control visible without opening
 * Settings. Both of its menu items write capability config keys:
 * "Change model" through `/config/set_provider`, "Lead/worker settings" through
 * `/config/upsert` on `BIOROUTER_PROVIDER` and `BIOROUTER_LEAD_*`.
 */

const mocks = vi.hoisted(() => ({
  read: vi.fn(async () => ''),
  getProviders: vi.fn(async () => [] as unknown[]),
  getPrivacyDisclosure: vi.fn(),
  ackPrivacyDisclosure: vi.fn(),
}));

vi.mock('../../../../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  getPrivacyDisclosure: mocks.getPrivacyDisclosure,
  ackPrivacyDisclosure: mocks.ackPrivacyDisclosure,
}));

// `PrivacyBadge` reads the master switch itself, so a suite that mocks
// ConfigContext and renders a badge owes both exports.
vi.mock('../../../ConfigContext', () => ({
  useConfig: () => ({ read: mocks.read, getProviders: mocks.getProviders }),
  usePrivacyTiersEnabled: () => true,
}));

vi.mock('../../../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    currentModel: 'gpt-5.5',
    currentProvider: 'openai',
    getCurrentModelAndProviderForDisplay: async () => ({ model: 'gpt-5.5', provider: 'OpenAI' }),
    getCurrentModelDisplayName: async () => 'gpt-5.5',
    getCurrentProviderDisplayName: async () => 'OpenAI',
  }),
}));

// Markers rather than `null`: the assertion that matters is whether a click
// OPENED the dialog, which an empty stub cannot report.
vi.mock('../subcomponents/SwitchModelModal', () => ({
  SwitchModelModal: () => <div>SWITCH-MODEL-MODAL</div>,
}));
vi.mock('../subcomponents/LeadWorkerSettings', () => ({
  LeadWorkerSettings: () => <div>LEAD-WORKER-MODAL</div>,
}));

const dropdownRef = { current: null } as unknown as React.RefObject<HTMLDivElement>;

function renderBar() {
  return render(
    <ModelsBottomBar
      sessionId="s1"
      privacyTier="public"
      dropdownRef={dropdownRef}
      setView={vi.fn()}
      alerts={[]}
    />
  );
}

function openChip() {
  fireEvent.pointerDown(screen.getByLabelText(/Current model/), { button: 0, ctrlKey: false });
}

describe('ModelsBottomBar on a browser-served surface', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    __resetDisclosureStoreForTests();
    mocks.getProviders.mockResolvedValue([
      {
        name: 'openai',
        is_configured: true,
        provider_type: 'Builtin',
        affiliation: null,
        resolved_tier: 'public',
        metadata: { name: 'openai', display_name: 'OpenAI', tier: 'public', runs_locally: false },
      },
    ]);
    mocks.getPrivacyDisclosure.mockResolvedValue({
      data: { title_template: '{provider}', long: 'L', short: 'S', acknowledged: true },
    });
  });

  afterEach(() => {
    delete document.documentElement.dataset.biorouterSurface;
  });

  /**
   * ⚠ **Fails against today's code**: neither item carries `disabled`, so
   * neither renders `aria-disabled`, and `findByTestId` has no note to resolve.
   * Today the chip offers "Change model", the dialog opens, and the refusal
   * arrives as a toast written for an AI agent.
   */
  it('greys out both menu items and says who chose the model', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    renderBar();
    openChip();

    const note = await screen.findByTestId('host-managed-model-note');
    expect(note.textContent).toMatch(/biorouter serve/);

    expect(screen.getByRole('menuitem', { name: /Change model/ })).toHaveAttribute(
      'aria-disabled',
      'true'
    );
    expect(screen.getByRole('menuitem', { name: /Lead\/worker settings/ })).toHaveAttribute(
      'aria-disabled',
      'true'
    );
  });

  /**
   * `aria-disabled` is a claim about the item; this is a claim about what the
   * click does. Radix blocks selection on a disabled item, but the `onClick`
   * would still be wired without the ternary in the component — so this pins
   * the behaviour rather than the markup.
   *
   * ⚠ Fails against today's code, where the click mounts the dialog.
   */
  it('does not open the switch-model dialog when the item is clicked', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    renderBar();
    openChip();

    await screen.findByTestId('host-managed-model-note');
    fireEvent.click(screen.getByRole('menuitem', { name: /Change model/ }));

    expect(screen.queryByText('SWITCH-MODEL-MODAL')).toBeNull();
  });

  /**
   * ⚠ **The control.** Passes before and after. Taking the model picker away
   * from the desktop application would be a far worse regression than the 409
   * this change replaces, so the desktop path is asserted positively rather
   * than left to the absence of a failure.
   */
  it('leaves the desktop chip fully usable', async () => {
    renderBar();
    openChip();

    const item = await screen.findByRole('menuitem', { name: /Change model/ });
    expect(item).not.toHaveAttribute('aria-disabled', 'true');
    expect(screen.queryByTestId('host-managed-model-note')).toBeNull();

    fireEvent.click(item);
    expect(await screen.findByText('SWITCH-MODEL-MODAL')).toBeInTheDocument();
  });
});
