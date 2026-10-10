import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ModelsBottomBar from './ModelsBottomBar';
import { __resetDisclosureStoreForTests } from '../../../privacy/disclosureCopy';
import { MODEL_COPY } from '../../../bottom_menu/copy';
import {
  getReasoningEffort,
  reasoningEffortForRequest,
  resetReasoningEffortForTests,
  setReasoningEffort,
} from '../../../../store/reasoningEffort';

/**
 * Spec 3.7: the model and the effort are ONE picker. The chip names the model
 * the way a person says it, plus a non-default effort ("gpt-5.6-sol · Deep"),
 * and its padlock; the menu carries Quick, Normal and Deep as a radio group.
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
vi.mock('../../../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-key' }),
}));
vi.mock('../../../ConfigContext', () => ({
  useConfig: () => ({ read: mocks.read, getProviders: mocks.getProviders }),
  usePrivacyTiersEnabled: () => true,
}));
vi.mock('../../../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    currentModel: 'gpt-5.6-sol-2026-07-09',
    currentProvider: 'versa_azure',
    getCurrentModelAndProviderForDisplay: async () => ({
      model: 'gpt-5.6-sol-2026-07-09',
      provider: 'Versa API Azure',
    }),
    getCurrentModelDisplayName: async () => 'gpt-5.6-sol-2026-07-09',
    getCurrentProviderDisplayName: async () => 'Versa API Azure',
  }),
}));
vi.mock('../subcomponents/SwitchModelModal', () => ({ SwitchModelModal: () => null }));
vi.mock('../subcomponents/LeadWorkerSettings', () => ({ LeadWorkerSettings: () => null }));

const SCOPE = 'session:s1';

function renderChip(reasoningScope: string | null = SCOPE) {
  return render(
    <ModelsBottomBar
      sessionId="s1"
      privacyTier="private"
      setView={vi.fn()}
      reasoningScope={reasoningScope ?? undefined}
      hideAlertPopover
    />
  );
}

async function openMenu() {
  const chip = await screen.findByTestId('model-chip');
  fireEvent.pointerDown(chip, { button: 0, ctrlKey: false });
  return chip;
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  resetReasoningEffortForTests();
  __resetDisclosureStoreForTests();
  mocks.getProviders.mockResolvedValue([
    {
      name: 'versa_azure',
      is_configured: true,
      provider_type: 'Builtin',
      metadata: {
        name: 'versa_azure',
        display_name: 'Versa API Azure',
        tier: 'private',
        runs_locally: false,
      },
      affiliation: null,
      resolved_tier: 'private',
    },
  ]);
  mocks.getPrivacyDisclosure.mockResolvedValue({
    data: { title_template: '{provider}', long: 'LONG', short: 'SHORT', acknowledged: true },
  });
});

describe('the model and effort chip', () => {
  it('names the model without its date stamp, keeps the full id in its name', async () => {
    renderChip();
    const chip = await screen.findByTestId('model-chip');
    await waitFor(() => expect(chip).toHaveTextContent('gpt-5.6-sol'));
    expect(chip).not.toHaveTextContent('2026-07-09');
    expect(chip).toHaveAccessibleName(/^Current model: gpt-5\.6-sol-2026-07-09/);
    // The padlock is the one mark, and the model's tier drives it.
    expect(await screen.findByTestId('privacy-badge')).toHaveAttribute('data-privacy', 'private');
  });

  it('draws no Brain glyph and no affiliation glyph on the chip', async () => {
    renderChip();
    const chip = await screen.findByTestId('model-chip');
    await screen.findByTestId('privacy-badge');
    // Two svgs at most: the padlock and the chevron.
    expect(chip.querySelector('.lucide-brain')).toBeNull();
    expect(chip.querySelector('[data-testid="affiliation-badge"]')).toBeNull();
  });

  it('keeps the default effort quiet and names a non-default one', async () => {
    renderChip();
    const chip = await screen.findByTestId('model-chip');
    expect(chip).not.toHaveTextContent('Normal');
    expect(chip).toHaveAccessibleName(/Effort: Normal$/);

    setReasoningEffort(SCOPE, 'deep');
    await waitFor(() => expect(chip).toHaveTextContent('gpt-5.6-sol · Deep'));
    expect(chip).toHaveAccessibleName(/Effort: Deep$/);
  });

  it('chooses the effort from a radio group in the model menu', async () => {
    renderChip();
    await openMenu();

    expect(await screen.findByText(MODEL_COPY.effortGroup)).toBeInTheDocument();
    const radios = screen.getAllByRole('menuitemradio');
    expect(radios.map((radio) => radio.textContent)).toEqual(['Quick', 'Normal', 'Deep']);
    expect(screen.getByRole('menuitemradio', { name: 'Normal' })).toHaveAttribute(
      'aria-checked',
      'true'
    );

    fireEvent.click(screen.getByRole('menuitemradio', { name: 'Deep' }));
    expect(getReasoningEffort(SCOPE)).toBe('deep');
    expect(reasoningEffortForRequest(getReasoningEffort(SCOPE))).toBe('deep');
  });

  it('is a model picker only when no effort scope is given', async () => {
    renderChip(null);
    await openMenu();
    await screen.findByText(MODEL_COPY.changeModel);
    expect(screen.queryByRole('menuitemradio')).toBeNull();
    expect(screen.queryByText(MODEL_COPY.effortGroup)).toBeNull();
  });

  it('offers the two actions with an ellipsis and no trailing icons', async () => {
    renderChip();
    await openMenu();
    const change = await screen.findByRole('menuitem', { name: MODEL_COPY.changeModel });
    const leadWorker = screen.getByRole('menuitem', { name: MODEL_COPY.leadWorker });
    expect(change.querySelector('svg')).toBeNull();
    expect(leadWorker.querySelector('svg')).toBeNull();
    expect(change).not.toHaveAttribute('title');
  });

  it('keeps the chat’s privacy line visible in the menu', async () => {
    renderChip();
    await openMenu();
    expect(await screen.findByTestId('chat-privacy-line')).toHaveTextContent(/^Private chat/);
  });
});
