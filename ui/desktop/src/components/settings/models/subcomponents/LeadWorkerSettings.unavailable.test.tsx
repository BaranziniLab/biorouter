import { render, screen, fireEvent } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProviderDetails } from '../../../../api';
import { LeadWorkerSettings } from './LeadWorkerSettings';

/**
 * D4 of the 2026-09-12 model-controls run — a provider the user set up that
 * cannot run must be EXPLAINED in these two selects, not omitted from them.
 *
 * Measured live (dev GUI, sandboxed config, `CODEX_COMMAND: /nope/codex`,
 * 2026-09-12) against `origin/main`: Settings → the composer chip → Lead/worker
 * settings, switch on, open the lead select — **25** options,
 * `anyCodex=false`, `anyUnavailable=false`, while `GET /config/providers` served
 * `codex | is_configured=False | unavailable_reason='Codex is not installed, or
 * is not on a path Biorouter searches' | 6 known models`. One menu item away,
 * the Switch-models picker rendered Codex disabled with that same sentence on
 * the row. Same catalog, same machine, two different stories.
 *
 * The real `Select` (react-select) is used deliberately, exactly as
 * `SwitchModelModal.unavailable.test.tsx` does: `role="option"` and
 * `aria-disabled` are react-select's own output, and they are what this
 * pre-flight has to produce. A stubbed Select can see no option at all.
 */

const mocks = vi.hoisted(() => ({
  read: vi.fn(),
  upsert: vi.fn(),
  remove: vi.fn(),
  getProviders: vi.fn(),
  getProviderModels: vi.fn(),
}));

vi.mock('../predefinedModelsUtils', () => ({
  shouldShowPredefinedModels: () => false,
  getPredefinedModelsFromEnv: () => [],
}));

vi.mock('../../../ConfigContext', () => ({
  useConfig: () => ({
    read: mocks.read,
    upsert: mocks.upsert,
    remove: mocks.remove,
    getProviders: mocks.getProviders,
    getProviderModels: mocks.getProviderModels,
  }),
}));

vi.mock('../../../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({ currentModel: null }),
}));

/** The daemon's sentence for a coding agent whose CLI does not resolve. */
const NOT_INSTALLED = 'Codex is not installed, or is not on a path Biorouter searches';

function provider(
  name: string,
  displayName: string,
  models: string[],
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
      default_model: models[0] ?? '',
      description: '',
      display_name: displayName,
      known_models: models.map((model) => ({ name: model })),
      model_doc_link: '',
      name,
      tier: 'public',
      runs_locally: false,
    },
  } as unknown as ProviderDetails;
}

/** Versa usable, Codex set up but unable to run, OpenAI never set up. */
const ROWS = [
  provider('versa_azure', 'Versa API Azure', ['gpt-5.5-2026-04-24'], { is_configured: true }),
  provider('codex', 'Codex', ['gpt-6-astra'], {
    is_configured: false,
    unavailable_reason: NOT_INSTALLED,
  }),
  provider('openai', 'OpenAI', ['gpt-4o'], { is_configured: false }),
];

/** No pair configured, so the dialog opens with the worker on the selection. */
function daemonHoldsNoPair() {
  mocks.read.mockImplementation(async (key: string) => {
    if (key === 'BIOROUTER_MODEL') return 'gpt-5.5-2026-04-24';
    if (key === 'BIOROUTER_PROVIDER') return 'versa_azure';
    return null;
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.getProviders.mockResolvedValue(ROWS);
  mocks.getProviderModels.mockResolvedValue([]);
  mocks.upsert.mockResolvedValue(undefined);
  mocks.remove.mockResolvedValue(undefined);
  daemonHoldsNoPair();
});

/** The switch gates both selects (`isDisabled={!isEnabled}`), so turn it on. */
async function enablePair() {
  const toggle = await screen.findByRole('switch', { name: 'Lead/worker mode' });
  if (toggle.getAttribute('aria-checked') === 'false') fireEvent.click(toggle);
}

/** The lead select is the first combobox and the worker select the second. */
async function openSelect(which: 'lead' | 'worker') {
  await enablePair();
  const boxes = await screen.findAllByRole('combobox');
  fireEvent.keyDown(boxes[which === 'lead' ? 0 : 1], { key: 'ArrowDown', code: 'ArrowDown' });
}

describe('LeadWorkerSettings — a provider that cannot run', () => {
  it.each(['lead', 'worker'] as const)(
    'lists its models in the %s select, disabled, with the reason on the row',
    async (which) => {
      render(<LeadWorkerSettings isOpen onClose={vi.fn()} />);
      await openSelect(which);

      const codex = await screen.findByRole('option', { name: /gpt-6-astra/ });
      expect(codex).toHaveAttribute('aria-disabled', 'true');
      expect(codex).toHaveTextContent(`Unavailable: ${NOT_INSTALLED}`);
    }
  );

  // Without this the case above passes for a picker that disables every row —
  // and a picker that had simply stopped filtering would offer a provider the
  // user never set up.
  it('leaves a usable provider selectable, and still omits one never set up', async () => {
    render(<LeadWorkerSettings isOpen onClose={vi.fn()} />);
    await openSelect('lead');

    const versa = await screen.findByRole('option', { name: /gpt-5\.5-2026-04-24/ });
    expect(versa).toHaveAttribute('aria-disabled', 'false');
    expect(versa).not.toHaveTextContent(/Unavailable/);
    expect(screen.queryByRole('option', { name: /gpt-4o/ })).toBeNull();
  });

  /**
   * The reason has to reach a field that is ALREADY filled: a pair saved while
   * the CLI resolved reopens here after it moved, and nothing in this dialog
   * re-picks anything on open. A disabled menu row says nothing about that.
   */
  it('explains a saved lead whose provider can no longer run, and refuses the save', async () => {
    mocks.read.mockImplementation(async (key: string) => {
      if (key === 'BIOROUTER_LEAD_MODEL') return 'gpt-6-astra';
      if (key === 'BIOROUTER_LEAD_PROVIDER') return 'codex';
      if (key === 'BIOROUTER_MODEL') return 'gpt-5.5-2026-04-24';
      if (key === 'BIOROUTER_PROVIDER') return 'versa_azure';
      return null;
    });

    render(<LeadWorkerSettings isOpen onClose={vi.fn()} />);

    expect(await screen.findByTestId('lead-worker-lead-unavailable')).toHaveTextContent(
      `Unavailable: ${NOT_INSTALLED}`
    );
    // The worker half is fine, so it says nothing — otherwise this passes for a
    // dialog that shouts the reason at both fields.
    expect(screen.queryByTestId('lead-worker-worker-unavailable')).toBeNull();

    const save = screen.getByRole('button', { name: 'Save settings' });
    expect(save).toBeDisabled();
    fireEvent.click(save);
    expect(mocks.upsert).not.toHaveBeenCalled();
  });

  // The fields are inert while the pair is off, so a verdict about them would be
  // a refusal of something nobody is doing.
  it('says nothing about either half while the pair is switched off', async () => {
    mocks.read.mockImplementation(async (key: string) => {
      if (key === 'BIOROUTER_LEAD_MODEL') return 'gpt-6-astra';
      if (key === 'BIOROUTER_LEAD_PROVIDER') return 'codex';
      if (key === 'BIOROUTER_MODEL') return 'gpt-5.5-2026-04-24';
      if (key === 'BIOROUTER_PROVIDER') return 'versa_azure';
      return null;
    });

    render(<LeadWorkerSettings isOpen onClose={vi.fn()} />);
    await screen.findByTestId('lead-worker-lead-unavailable');

    fireEvent.click(screen.getByRole('switch', { name: 'Lead/worker mode' }));

    expect(screen.queryByTestId('lead-worker-lead-unavailable')).toBeNull();
    expect(screen.getByRole('button', { name: 'Save settings' })).toBeEnabled();
  });
});
