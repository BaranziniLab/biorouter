import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ModelsBottomBar, { joinSentences } from './ModelsBottomBar';
import { __resetDisclosureStoreForTests } from '../../../privacy/disclosureCopy';
import { MODEL_COPY } from '../../../bottom_menu/copy';
import {
  getReasoningEffort,
  REASONING_EFFORT_HELP,
  reasoningEffortForRequest,
  resetReasoningEffortForTests,
  setReasoningEffort,
} from '../../../../store/reasoningEffort';

/**
 * Spec 3.7: the model and the effort are ONE picker. The chip names the model
 * the way a person says it, plus a non-default effort ("gpt-5.6-sol · Deep"),
 * and its padlock. The menu sets effort with labelled Quick, Normal and Deep segments.
 */
const mocks = vi.hoisted(() => ({
  model: 'gpt-5.6-sol-2026-07-09',
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
    currentModel: mocks.model,
    currentProvider: 'versa_azure',
    getCurrentModelAndProviderForDisplay: async () => ({
      model: mocks.model,
      provider: 'Versa API Azure',
    }),
    getCurrentModelDisplayName: async () => mocks.model,
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
  mocks.model = 'gpt-5.6-sol-2026-07-09';
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

  it('names a Bedrock model, not its inference-profile id (V2-R6-2)', async () => {
    mocks.model = 'us.anthropic.claude-sonnet-4-5-20250929-v1:0';
    renderChip();
    const chip = await screen.findByTestId('model-chip');
    await waitFor(() => expect(chip).toHaveTextContent('claude-sonnet-4-5'));
    expect(chip).not.toHaveTextContent('us.anthropic');
    expect(chip).not.toHaveTextContent('v1:0');
    expect(chip).toHaveAccessibleName(
      /^Current model: us\.anthropic\.claude-sonnet-4-5-20250929-v1:0/
    );
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

    act(() => setReasoningEffort(SCOPE, 'deep'));
    await waitFor(() => expect(chip).toHaveTextContent('gpt-5.6-sol · Deep'));
    expect(chip).toHaveAccessibleName(/Effort: Deep$/);
  });

  it('reads its name as sentences with one period each, never two (V2-R8-5)', async () => {
    renderChip();
    const chip = await screen.findByTestId('model-chip');
    await waitFor(() => expect(chip).toHaveAccessibleName(/Private model/));
    // The chat's tier sentence ends with its own period; the effort follows it
    // after one, not after a second one the join used to add.
    expect(chip).toHaveAccessibleName(
      'Current model: gpt-5.6-sol-2026-07-09 (Private model). Private chat. Biorouter only lets a private model open it. Effort: Normal'
    );
    expect(chip.getAttribute('aria-label')).not.toContain('..');
  });

  it('ends each part of the name once, and leaves the parts as written (V2-R8-5)', () => {
    expect(
      joinSentences([
        'Current model: m (Private model)',
        'Private chat. Biorouter only lets a private model open it.',
        'Effort: Normal',
      ])
    ).toBe(
      'Current model: m (Private model). Private chat. Biorouter only lets a private model open it. Effort: Normal'
    );
    expect(joinSentences(['Current model: m', 'Public chat', 'Effort: Deep'])).toBe(
      'Current model: m. Public chat. Effort: Deep'
    );
    // Parts left out leave no stray period; the last part is not given one.
    expect(joinSentences(['Current model: m', null, false, ''])).toBe('Current model: m');
    expect(joinSentences(['Current model: m', 'Private chat. It ends here.'])).toBe(
      'Current model: m. Private chat. It ends here.'
    );
  });

  it('shows all effort choices inside one segmented control, with no separate current value', async () => {
    const user = userEvent.setup();
    renderChip();
    const chip = await screen.findByTestId('model-chip');
    expect(chip).toHaveAttribute('aria-expanded', 'false');
    await openMenu();
    expect(chip).toHaveAttribute('aria-expanded', 'true');

    const section = await screen.findByRole('group', { name: MODEL_COPY.effortGroup });
    const control = within(section).getByRole('radiogroup', { name: MODEL_COPY.effortGroup });
    expect(
      within(control)
        .getAllByRole('radio')
        .map((radio) => radio.textContent)
    ).toEqual(['Quick', 'Normal', 'Deep']);
    expect(within(control).getByRole('radio', { name: 'Normal' })).toBeChecked();
    expect(screen.queryByRole('slider')).toBeNull();
    expect(screen.queryByRole('menuitemradio')).toBeNull();
    expect(within(section).queryByTestId('effort-value')).toBeNull();
    expect(section.querySelectorAll('svg')).toHaveLength(1);
    expect(control.querySelectorAll('[data-slot="segmented-thumb"]')).toHaveLength(1);
    expect(control).toHaveAccessibleDescription(REASONING_EFFORT_HELP);
    expect(
      within(section).getByRole('button', { name: `About ${MODEL_COPY.aboutEffort}` })
    ).toBeInTheDocument();

    act(() => within(control).getByRole('radio', { name: 'Normal' }).focus());
    await user.keyboard('{ArrowRight}');
    await waitFor(() => expect(getReasoningEffort(SCOPE)).toBe('deep'));
    expect(reasoningEffortForRequest(getReasoningEffort(SCOPE))).toBe('deep');
    expect(within(control).getByRole('radio', { name: 'Deep' })).toBeChecked();
    await waitFor(() => expect(chip).toHaveTextContent('gpt-5.6-sol · Deep'));
    await user.keyboard('{Home}');
    expect(getReasoningEffort(SCOPE)).toBe('quick');
    await user.keyboard('{End}');
    expect(getReasoningEffort(SCOPE)).toBe('deep');
    await user.keyboard('{ArrowLeft}');
    await waitFor(() => expect(getReasoningEffort(SCOPE)).toBe('normal'));
    expect(screen.getByTestId('effort-section')).toBeInTheDocument();
  });

  it('applies a clicked segment immediately and keeps the menu open', async () => {
    const user = userEvent.setup();
    renderChip();
    await openMenu();
    const control = await screen.findByRole('radiogroup', { name: MODEL_COPY.effortGroup });
    await user.click(within(control).getByRole('radio', { name: 'Deep' }));
    expect(getReasoningEffort(SCOPE)).toBe('deep');
    await user.click(within(control).getByRole('radio', { name: 'Quick' }));
    expect(getReasoningEffort(SCOPE)).toBe('quick');
    expect(within(control).getByRole('radio', { name: 'Quick' })).toHaveFocus();
    expect(screen.getByTestId('effort-section')).toBeInTheDocument();
  });

  it('follows an effort set elsewhere while the menu is open', async () => {
    renderChip();
    await openMenu();
    const control = await screen.findByRole('radiogroup', { name: MODEL_COPY.effortGroup });
    act(() => setReasoningEffort(SCOPE, 'quick'));
    await waitFor(() =>
      expect(within(control).getByRole('radio', { name: 'Quick' })).toBeChecked()
    );
    expect(within(control).getByRole('radio', { name: 'Normal' })).not.toBeChecked();
  });

  it('reaches the selected segment with vertical arrows and leaves it the same way', async () => {
    const user = userEvent.setup();
    renderChip();
    await openMenu();
    const control = await screen.findByRole('radiogroup', { name: MODEL_COPY.effortGroup });
    const normal = within(control).getByRole('radio', { name: 'Normal' });
    const deep = within(control).getByRole('radio', { name: 'Deep' });
    const change = screen.getByRole('menuitem', { name: MODEL_COPY.changeModel });

    act(() => change.focus());
    await user.keyboard('{ArrowUp}');
    expect(normal).toHaveFocus();
    expect(getReasoningEffort(SCOPE)).toBe('normal');
    await user.keyboard('{ArrowUp}');
    expect(normal).toHaveFocus();
    expect(getReasoningEffort(SCOPE)).toBe('normal');

    await user.keyboard('{ArrowRight}');
    await waitFor(() => expect(deep).toHaveFocus());
    expect(getReasoningEffort(SCOPE)).toBe('deep');
    await user.keyboard('{ArrowDown}');
    expect(change).toHaveFocus();
    expect(getReasoningEffort(SCOPE)).toBe('deep');
    await user.keyboard('{ArrowUp}');
    expect(deep).toHaveFocus();
    expect(getReasoningEffort(SCOPE)).toBe('deep');
    await user.keyboard('{ArrowDown}{ArrowDown}');
    await waitFor(() =>
      expect(screen.getByRole('menuitem', { name: MODEL_COPY.leadWorker })).toHaveFocus()
    );
  });

  it('closes on Escape from a segment and gives focus back to the chip', async () => {
    const user = userEvent.setup();
    renderChip();
    const chip = await openMenu();
    const normal = await screen.findByRole('radio', { name: 'Normal' });
    act(() => normal.focus());
    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByTestId('effort-section')).toBeNull());
    await waitFor(() => expect(chip).toHaveFocus());
  });

  it('is a model picker only when no effort scope is given', async () => {
    renderChip(null);
    await openMenu();
    await screen.findByText(MODEL_COPY.changeModel);
    expect(screen.queryByRole('radiogroup')).toBeNull();
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
