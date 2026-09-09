import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { PinnedModelNote } from './PinnedModelNote';
import { bindingLabel, pinContradictsSelection, pinnedModelNotice } from './pinnedModel';

const mocks = vi.hoisted(() => ({
  getProviders: vi.fn(),
  currentModel: 'claude-opus-5' as string | null,
  currentProvider: 'claude_code' as string | null,
}));

vi.mock('../ConfigContext', () => ({
  useConfig: () => ({ getProviders: mocks.getProviders }),
}));

vi.mock('../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    currentModel: mocks.currentModel,
    currentProvider: mocks.currentProvider,
  }),
}));

const CATALOG = [
  { name: 'versa_azure', metadata: { display_name: 'Versa API Azure' } },
  { name: 'claude_code', metadata: { display_name: 'Claude Code' } },
];

const PINNED = { provider: 'versa_azure', model: 'gpt-5.5-2026-04-24' };

beforeEach(() => {
  vi.clearAllMocks();
  mocks.getProviders.mockResolvedValue(CATALOG);
  mocks.currentModel = 'claude-opus-5';
  mocks.currentProvider = 'claude_code';
});

/**
 * F2, the verbatim repro: Settings → Models → Claude Code / claude-opus-5, then
 * send into a chat whose `privacy_tier` is `private`. The turn answers from
 * Versa; before this note, nothing on screen said so.
 */
describe('PinnedModelNote', () => {
  it('names both bindings by their display names, not their ids', async () => {
    render(<PinnedModelNote pinnedModel={PINNED} />);

    const note = await screen.findByTestId('pinned-model-note');
    await waitFor(() =>
      expect(note).toHaveTextContent(
        'This chat is marked private, so it stays on Versa API Azure / gpt-5.5-2026-04-24. ' +
          'Claude Code / claude-opus-5 is not used here.'
      )
    );
    // `versa_azure` is not what that provider is called anywhere else in the
    // app; leaking the registry id here would read as a bug in the one place
    // the user is being told something new.
    expect(note.textContent).not.toContain('versa_azure');
    expect(note.textContent).not.toContain('claude_code');
  });

  /**
   * The daemon sends the frame on EVERY repaired bind, including the ordinary
   * ones (an LRU-rehydrated agent, a legacy row) where it names exactly what is
   * already selected. A note there would be permanent and meaningless.
   */
  it('says nothing when the pin names what is already selected', () => {
    mocks.currentProvider = 'versa_azure';
    mocks.currentModel = 'gpt-5.5-2026-04-24';
    const { container } = render(<PinnedModelNote pinnedModel={PINNED} />);
    expect(container).toBeEmptyDOMElement();
  });

  it('says nothing when no turn has been pinned', () => {
    const { container } = render(<PinnedModelNote pinnedModel={undefined} />);
    expect(container).toBeEmptyDOMElement();
  });

  /**
   * `currentProvider` / `currentModel` are `null` on the first render of every
   * chat while the config loads. A note that flashed on and off there would be
   * asserting something the component cannot yet know.
   */
  it('says nothing while the selection is still unresolved', () => {
    mocks.currentProvider = null;
    mocks.currentModel = null;
    const { container } = render(<PinnedModelNote pinnedModel={PINNED} />);
    expect(container).toBeEmptyDOMElement();
  });

  /**
   * A catalog that cannot be read costs the display names and nothing else.
   * Staying silent about the pin is the one outcome that reintroduces the
   * defect, so the ids stand in.
   */
  it('falls back to ids rather than going quiet when the catalog fails', async () => {
    mocks.getProviders.mockRejectedValue(new Error('offline'));
    render(<PinnedModelNote pinnedModel={PINNED} />);

    const note = await screen.findByTestId('pinned-model-note');
    expect(note).toHaveTextContent(
      'This chat is marked private, so it stays on gpt-5.5-2026-04-24. ' +
        'claude-opus-5 is not used here.'
    );
  });

  it('is a neutral note, not a warning: nothing has gone wrong', async () => {
    render(<PinnedModelNote pinnedModel={PINNED} />);
    const note = await screen.findByTestId('pinned-model-note');
    expect(note).toHaveAttribute('role', 'status');
    expect(note.className).toContain('bg-background-muted');
    expect(note.className).not.toContain('wash-warning');
    expect(note.className).not.toContain('wash-danger');
  });
});

describe('the pure half', () => {
  it('reports a contradiction only when both sides are known and differ', () => {
    const selected = { provider: 'claude_code', model: 'claude-opus-5' };
    expect(pinContradictsSelection(PINNED, selected)).toBe(true);
    // Same provider, different model, and the reverse: both are contradictions.
    expect(
      pinContradictsSelection({ provider: 'claude_code', model: 'claude-sonnet-5' }, selected)
    ).toBe(true);
    expect(pinContradictsSelection({ ...selected }, selected)).toBe(false);
    expect(pinContradictsSelection(undefined, selected)).toBe(false);
    expect(pinContradictsSelection(PINNED, { provider: null, model: null })).toBe(false);
    expect(pinContradictsSelection(PINNED, { provider: 'claude_code', model: null })).toBe(false);
  });

  it('drops the second clause when the selection cannot be named', () => {
    expect(pinnedModelNotice('Versa API Azure / gpt-5.5')).toBe(
      'This chat is marked private, so it stays on Versa API Azure / gpt-5.5.'
    );
  });

  it('labels a binding by provider and model, or by model alone', () => {
    expect(bindingLabel('Versa API Azure', 'gpt-5.5')).toBe('Versa API Azure / gpt-5.5');
    expect(bindingLabel(undefined, 'gpt-5.5')).toBe('gpt-5.5');
    expect(bindingLabel('   ', 'gpt-5.5')).toBe('gpt-5.5');
  });
});
