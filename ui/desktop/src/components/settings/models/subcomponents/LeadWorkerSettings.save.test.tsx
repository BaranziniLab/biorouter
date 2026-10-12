import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProviderDetails } from '../../../../api';
import { leadWorkerCopy } from '../copy';
import { LeadWorkerSettings } from './LeadWorkerSettings';

/**
 * Save waits for a change (V4-R7-4's note, coordinator ruling 22). The dialog fills its fields
 * from the config on open, so a Save that is coral on open offers to write back what is already
 * saved. Disabled until a change exists, it draws what a disabled secondary draws
 * (`.br-button-inert`), and the primitive turns it coral once it can act; the Chromium half of
 * that is in `LeadWorkerSettings.fill.browser.test.tsx`.
 *
 * What counts as a change is what Save would write: switched on, the seven values it upserts;
 * switched off, only the removal, so the inert fields of a pair that was off and is off again do
 * not count. The real `Select` (react-select) is used, as the unavailable cases do.
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

function provider(name: string, displayName: string, models: string[]): ProviderDetails {
  return {
    name,
    provider_type: 'Builtin',
    affiliation: null,
    resolved_tier: null,
    unavailable_reason: null,
    is_configured: true,
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

const ROWS = [
  provider('versa_azure', 'Versa API Azure', ['gpt-5.5', 'gpt-5.5-mini']),
  provider('anthropic', 'Anthropic', ['claude-sonnet']),
];

/** No pair saved: the worker is the current selection and the mode is off. */
function daemonHoldsNoPair() {
  mocks.read.mockImplementation(async (key: string) => {
    if (key === 'BIOROUTER_MODEL') return 'gpt-5.5';
    if (key === 'BIOROUTER_PROVIDER') return 'versa_azure';
    return null;
  });
}

/** A pair saved, with the counts as the config serves them (strings). */
function daemonHoldsAPair() {
  mocks.read.mockImplementation(async (key: string) => {
    switch (key) {
      case 'BIOROUTER_LEAD_MODEL':
        return 'claude-sonnet';
      case 'BIOROUTER_LEAD_PROVIDER':
        return 'anthropic';
      case 'BIOROUTER_LEAD_TURNS':
        return '3';
      case 'BIOROUTER_LEAD_FAILURE_THRESHOLD':
        return '2';
      case 'BIOROUTER_LEAD_FALLBACK_TURNS':
        return '2';
      case 'BIOROUTER_MODEL':
        return 'gpt-5.5';
      case 'BIOROUTER_PROVIDER':
        return 'versa_azure';
      default:
        return null;
    }
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.getProviders.mockResolvedValue(ROWS);
  mocks.getProviderModels.mockResolvedValue([]);
  mocks.upsert.mockResolvedValue(undefined);
  mocks.remove.mockResolvedValue(undefined);
});

const save = () => screen.getByRole('button', { name: leadWorkerCopy.save });
const modeSwitch = () => screen.getByRole('switch', { name: leadWorkerCopy.mode });

async function openDialog() {
  render(<LeadWorkerSettings isOpen onClose={vi.fn()} />);
  await screen.findByRole('switch', { name: leadWorkerCopy.mode });
}

/** The lead select is the first combobox and the worker select the second. */
async function choose(which: 'lead' | 'worker', option: RegExp) {
  const boxes = await screen.findAllByRole('combobox');
  fireEvent.keyDown(boxes[which === 'lead' ? 0 : 1], { key: 'ArrowDown', code: 'ArrowDown' });
  fireEvent.click(await screen.findByRole('option', { name: option }));
}

const setCount = (label: string, value: string) =>
  fireEvent.change(screen.getByRole('spinbutton', { name: label }), { target: { value } });

describe('LeadWorkerSettings — Save waits for a change', () => {
  it('is disabled on open when no pair is saved', async () => {
    daemonHoldsNoPair();
    await openDialog();

    expect(modeSwitch()).toHaveAttribute('aria-checked', 'false');
    expect(save()).toBeDisabled();
    // Cancel is the way out and stays available.
    expect(screen.getByRole('button', { name: leadWorkerCopy.cancel })).toBeEnabled();
  });

  it('is disabled on open when a pair is saved', async () => {
    daemonHoldsAPair();
    await openDialog();

    expect(modeSwitch()).toHaveAttribute('aria-checked', 'true');
    expect(save()).toBeDisabled();
  });

  it('turns the pair on, chooses the lead, and saves it', async () => {
    daemonHoldsNoPair();
    await openDialog();

    fireEvent.click(modeSwitch());
    // On with no lead is a change but not a pair yet, so there is still nothing Save can write.
    expect(save()).toBeDisabled();

    await choose('lead', /claude-sonnet/);
    expect(save()).toBeEnabled();

    fireEvent.click(save());
    await waitFor(() =>
      expect(mocks.upsert).toHaveBeenCalledWith('BIOROUTER_LEAD_MODEL', 'claude-sonnet', false)
    );
    expect(mocks.upsert).toHaveBeenCalledWith('BIOROUTER_LEAD_PROVIDER', 'anthropic', false);
    expect(mocks.upsert).toHaveBeenCalledWith('BIOROUTER_MODEL', 'gpt-5.5', false);
  });

  it('is disabled again when a pair that was off is switched off again', async () => {
    daemonHoldsNoPair();
    await openDialog();

    fireEvent.click(modeSwitch());
    await choose('lead', /claude-sonnet/);
    expect(save()).toBeEnabled();

    // Off writes only the removal, which a pair that was never saved does not need.
    fireEvent.click(modeSwitch());
    expect(save()).toBeDisabled();
  });

  it('counts every value it writes, and a change changed back is no change', async () => {
    daemonHoldsAPair();
    await openDialog();

    for (const [label, changed, saved] of [
      [leadWorkerCopy.leadTurns, '4', '3'],
      [leadWorkerCopy.failureThreshold, '3', '2'],
      [leadWorkerCopy.fallbackTurns, '1', '2'],
    ] as const) {
      setCount(label, changed);
      expect(save(), `${label} changed`).toBeEnabled();
      setCount(label, saved);
      expect(save(), `${label} back`).toBeDisabled();
    }

    await choose('worker', /gpt-5\.5-mini/);
    expect(save(), 'worker model changed').toBeEnabled();
    await choose('worker', /^gpt-5\.5 \(/);
    expect(save(), 'worker model back').toBeDisabled();
  });

  it('writes the one change that was made', async () => {
    daemonHoldsAPair();
    await openDialog();

    setCount(leadWorkerCopy.leadTurns, '5');
    fireEvent.click(save());

    await waitFor(() =>
      expect(mocks.upsert).toHaveBeenCalledWith('BIOROUTER_LEAD_TURNS', 5, false)
    );
    expect(mocks.upsert).toHaveBeenCalledWith('BIOROUTER_LEAD_MODEL', 'claude-sonnet', false);
  });

  it('is enabled when a saved pair is switched off, and removes it', async () => {
    daemonHoldsAPair();
    await openDialog();

    fireEvent.click(modeSwitch());
    expect(save()).toBeEnabled();

    fireEvent.click(save());
    await waitFor(() => expect(mocks.remove).toHaveBeenCalledWith('BIOROUTER_LEAD_MODEL', false));
    expect(mocks.upsert).not.toHaveBeenCalled();
  });

  // A dialog that could not read the config does not know what is saved, so it does not claim
  // there is nothing to save: Save stays as available as it was before this rule.
  it('stays enabled when the config could not be read', async () => {
    const quiet = vi.spyOn(console, 'error').mockImplementation(() => {});
    mocks.read.mockRejectedValue(new Error('daemon unreachable'));
    await openDialog();

    expect(save()).toBeEnabled();
    quiet.mockRestore();
  });
});
