import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ModelsBottomBar from './ModelsBottomBar';
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
 * and its padlock. R-26 (owner message 13): the menu sets the effort with ONE
 * slider of three stops, Quick, Normal and Deep, where it had three radio rows.
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

  it('chooses the effort with one three-stop slider in the model menu (R-26)', async () => {
    renderChip();
    const chip = await screen.findByTestId('model-chip');
    expect(chip).toHaveAttribute('aria-expanded', 'false');
    await openMenu();
    // The chip's open ground follows `aria-expanded`, not `data-state`, which
    // the tooltip around the trigger overwrites.
    expect(chip).toHaveAttribute('aria-expanded', 'true');

    // One slider, named by the "Effort" label, speaking the word for its stop.
    const section = await screen.findByRole('group', { name: MODEL_COPY.effortGroup });
    const slider = within(section).getByRole('slider', { name: MODEL_COPY.effortGroup });
    expect(slider).toHaveAttribute('aria-valuemin', '0');
    expect(slider).toHaveAttribute('aria-valuemax', '2');
    expect(slider).toHaveAttribute('aria-valuenow', '1');
    expect(slider).toHaveAttribute('aria-valuetext', 'Normal');
    // The radio rows are gone, and so are their glyphs.
    expect(screen.queryByRole('menuitemradio')).toBeNull();
    expect(section.querySelectorAll('svg')).toHaveLength(1); // the InfoTip's glyph only

    // The current stop beside "Effort", for the eye (the slider says it already).
    const value = within(section).getByTestId('effort-value');
    expect(value).toHaveTextContent('Normal');
    expect(value).toHaveAttribute('aria-hidden', 'true');
    // What the levels mean: in the InfoTip, and on the thumb without hovering.
    expect(
      within(section).getByRole('button', { name: `About ${MODEL_COPY.aboutEffort}` })
    ).toBeInTheDocument();
    expect(slider).toHaveAccessibleDescription(REASONING_EFFORT_HELP);

    // Keys apply at once, through the same store and scope the rows wrote.
    act(() => slider.focus());
    fireEvent.keyDown(slider, { key: 'ArrowRight' });
    expect(getReasoningEffort(SCOPE)).toBe('deep');
    expect(reasoningEffortForRequest(getReasoningEffort(SCOPE))).toBe('deep');
    await waitFor(() => expect(slider).toHaveAttribute('aria-valuetext', 'Deep'));
    expect(value).toHaveTextContent('Deep');
    await waitFor(() => expect(chip).toHaveTextContent('gpt-5.6-sol · Deep'));
    fireEvent.keyDown(slider, { key: 'Home' });
    expect(getReasoningEffort(SCOPE)).toBe('quick');
    fireEvent.keyDown(slider, { key: 'End' });
    expect(getReasoningEffort(SCOPE)).toBe('deep');
    fireEvent.keyDown(slider, { key: 'ArrowLeft' });
    expect(getReasoningEffort(SCOPE)).toBe('normal');

    // The menu stays open: a slider is set, not picked.
    expect(screen.getByTestId('effort-section')).toBeInTheDocument();
  });

  it('sets the stop a press lands on, and keeps the menu open', async () => {
    renderChip();
    await openMenu();
    const section = await screen.findByTestId('effort-section');
    const root = within(section).getByTestId('effort-slider');
    const rail = root.querySelector<HTMLElement>('[data-slot="slider-rail"]')!;
    // jsdom lays nothing out; give the rail the 240px it has in the menu.
    vi.spyOn(rail, 'getBoundingClientRect').mockReturnValue({
      left: 0,
      right: 240,
      width: 240,
      top: 0,
      bottom: 24,
      height: 24,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    });
    fireEvent.pointerDown(root, { clientX: 236, pointerId: 1, pointerType: 'mouse', button: 0 });
    fireEvent.pointerUp(root, { clientX: 236, pointerId: 1, pointerType: 'mouse' });
    expect(getReasoningEffort(SCOPE)).toBe('deep');
    fireEvent.pointerDown(root, { clientX: 4, pointerId: 1, pointerType: 'mouse', button: 0 });
    fireEvent.pointerUp(root, { clientX: 4, pointerId: 1, pointerType: 'mouse' });
    expect(getReasoningEffort(SCOPE)).toBe('quick');
    expect(screen.getByTestId('effort-section')).toBeInTheDocument();
    // A press focuses the thumb, so the next key keeps going from there.
    expect(within(section).getByRole('slider')).toHaveFocus();
  });

  it('follows an effort set elsewhere while the menu is open', async () => {
    renderChip();
    await openMenu();
    const slider = await screen.findByRole('slider', { name: MODEL_COPY.effortGroup });
    act(() => setReasoningEffort(SCOPE, 'quick'));
    await waitFor(() => expect(slider).toHaveAttribute('aria-valuetext', 'Quick'));
    expect(screen.getByTestId('effort-value')).toHaveTextContent('Quick');
  });

  /**
   * A Radix menu's arrow keys visit its rows only, and Tab leaves the menu, so a
   * slider among the rows was out of the keyboard's reach. Up from the row under
   * it lands on the thumb; on the thumb, Up and Down walk the menu and Left and
   * Right move the effort.
   */
  it('reaches the slider with the arrow keys and leaves it the same way', async () => {
    const user = userEvent.setup();
    renderChip();
    await openMenu();
    const slider = await screen.findByRole('slider', { name: MODEL_COPY.effortGroup });
    const change = screen.getByRole('menuitem', { name: MODEL_COPY.changeModel });

    act(() => change.focus());
    await user.keyboard('{ArrowUp}');
    expect(slider).toHaveFocus();
    expect(getReasoningEffort(SCOPE)).toBe('normal');

    // Nothing above it: Up stays, and it is not read as "more effort".
    await user.keyboard('{ArrowUp}');
    expect(slider).toHaveFocus();
    expect(getReasoningEffort(SCOPE)).toBe('normal');

    await user.keyboard('{ArrowRight}');
    expect(getReasoningEffort(SCOPE)).toBe('deep');
    expect(slider).toHaveFocus();

    await user.keyboard('{ArrowDown}');
    expect(change).toHaveFocus();
    expect(getReasoningEffort(SCOPE)).toBe('deep');

    // Row to row is still the menu's own.
    await user.keyboard('{ArrowDown}');
    expect(screen.getByRole('menuitem', { name: MODEL_COPY.leadWorker })).toHaveFocus();
  });

  it('closes on Escape from the slider and gives focus back to the chip', async () => {
    const user = userEvent.setup();
    renderChip();
    const chip = await openMenu();
    const slider = await screen.findByRole('slider', { name: MODEL_COPY.effortGroup });
    act(() => slider.focus());
    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByTestId('effort-section')).toBeNull());
    await waitFor(() => expect(chip).toHaveFocus());
  });

  it('is a model picker only when no effort scope is given', async () => {
    renderChip(null);
    await openMenu();
    await screen.findByText(MODEL_COPY.changeModel);
    expect(screen.queryByRole('slider')).toBeNull();
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
