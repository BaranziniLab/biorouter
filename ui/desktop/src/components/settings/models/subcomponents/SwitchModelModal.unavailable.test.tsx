import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProviderDetails } from '../../../../api';
import { SwitchModelModal } from './SwitchModelModal';

const mocks = vi.hoisted(() => ({
  getProviders: vi.fn(),
  getProviderModels: vi.fn(),
  read: vi.fn(),
  changeModel: vi.fn(),
  currentProvider: 'versa_azure' as string,
  currentModel: null as string | null,
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
    currentModel: mocks.currentModel,
    currentProvider: mocks.currentProvider,
  }),
}));

vi.mock('../predefinedModelsUtils', () => ({
  getPredefinedModelsFromEnv: () => [],
  shouldShowPredefinedModels: () => false,
}));

// ⚠ The REAL react-select, as in `SwitchModelModal.privacy.test.tsx`:
// `role="option"` and `aria-disabled` are react-select's own output, and they
// are exactly what this pre-flight has to produce. The sibling
// `SwitchModelModal.test.tsx` stubs the Select and can see no option at all.

/** The daemon's sentence for a coding agent whose CLI does not resolve. */
const NOT_INSTALLED = 'Codex is not installed, or is not on a path Biorouter searches';

function provider(
  name: string,
  displayName: string,
  readiness: { is_configured: boolean; unavailable_reason?: string | null }
): ProviderDetails {
  return {
    name,
    provider_type: 'Builtin',
    affiliation: null,
    resolved_tier: null,
    unavailable_reason: null,
    ...readiness,
    metadata: {
      config_keys: [],
      default_model: '',
      description: '',
      display_name: displayName,
      known_models: [],
      model_doc_link: '',
      name,
      tier: 'public',
      runs_locally: false,
    },
  } as ProviderDetails;
}

/**
 * F6 of the 2026-09-10 provider QA run: with `CODEX_COMMAND` pointed at a path
 * that did not exist, Codex stayed selectable here. What the daemon now serves
 * for that machine: Versa usable, Codex set up but unable to run, OpenAI never
 * set up.
 */
const ROWS = [
  provider('versa_azure', 'Versa API Azure', { is_configured: true }),
  provider('codex', 'Codex', { is_configured: false, unavailable_reason: NOT_INSTALLED }),
  provider('openai', 'OpenAI', { is_configured: false }),
];

beforeEach(() => {
  vi.clearAllMocks();
  mocks.getProviders.mockResolvedValue(ROWS);
  mocks.getProviderModels.mockResolvedValue(['gpt-5.5-2026-04-24']);
  mocks.read.mockResolvedValue('');
  mocks.changeModel.mockResolvedValue(true);
  mocks.currentProvider = 'versa_azure';
  mocks.currentModel = null;
});

/** The provider combobox is the first one; open it from the keyboard. */
async function openProviderMenu() {
  // The bound provider's model loads first, which is also how we know the
  // provider list has arrived.
  await screen.findByText('gpt-5.5-2026-04-24');
  fireEvent.keyDown(screen.getAllByRole('combobox')[0], { key: 'ArrowDown', code: 'ArrowDown' });
}

describe('SwitchModelModal — a provider that cannot run', () => {
  it('lists it disabled, with the reason on the row', async () => {
    render(<SwitchModelModal sessionId="s1" onClose={vi.fn()} setView={vi.fn()} />);
    await openProviderMenu();

    const codex = await screen.findByRole('option', { name: /Codex/ });
    expect(codex).toHaveAttribute('aria-disabled', 'true');
    expect(codex).toHaveTextContent(`Unavailable: ${NOT_INSTALLED}`);
  });

  // Without this the case above passes for a picker that disables every row.
  it('leaves a usable provider selectable, and still omits one never set up', async () => {
    render(<SwitchModelModal sessionId="s1" onClose={vi.fn()} setView={vi.fn()} />);
    await openProviderMenu();

    const versa = await screen.findByRole('option', { name: /Versa API Azure/ });
    expect(versa).toHaveAttribute('aria-disabled', 'false');
    expect(versa).not.toHaveTextContent(/Unavailable/);
    expect(screen.queryByRole('option', { name: /OpenAI/ })).toBeNull();
  });

  /**
   * The dialog can OPEN on a provider it would never let you pick: the bound
   * one, after its CLI went missing. It says why before anything is tried, and
   * the switch cannot be submitted — the bind would only be refused by
   * `from_env`, with the explanation in a toast in the far corner.
   */
  it('explains, and refuses to switch, when it opens on an unavailable provider', async () => {
    mocks.currentProvider = 'codex';
    mocks.currentModel = 'gpt-6-astra';
    render(<SwitchModelModal sessionId="s1" onClose={vi.fn()} setView={vi.fn()} />);

    expect(await screen.findByTestId('switch-model-provider-error')).toHaveTextContent(
      `Unavailable: ${NOT_INSTALLED}`
    );
    const confirm = screen.getByRole('button', { name: 'Select model' });
    expect(confirm).toBeDisabled();
    fireEvent.click(confirm);
    expect(mocks.changeModel).not.toHaveBeenCalled();
  });
});
