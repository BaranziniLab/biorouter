import { render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { LeadWorkerSettings } from './LeadWorkerSettings';
import { leadWorkerCopy } from '../copy';

/**
 * The Lead/worker dialog's anatomy (codex simplicity redesign, spec 3.13): a 480px form dialog,
 * the switch row with its explanation in an InfoTip, labelled fields (each with its own InfoTip),
 * and no boxes inside the dialog. The save/disable/unavailable logic has its own files.
 */

// Stable identities: the dialog reloads its configuration whenever one of these changes.
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

describe('LeadWorkerSettings anatomy', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.read.mockResolvedValue(null);
    mocks.getProviders.mockResolvedValue([]);
    mocks.getProviderModels.mockResolvedValue([]);
  });

  it('is a 480px form dialog with a single title and a named switch', async () => {
    render(<LeadWorkerSettings isOpen onClose={vi.fn()} />);
    const dialog = await screen.findByRole('dialog', { name: leadWorkerCopy.title });
    await screen.findByRole('switch', { name: leadWorkerCopy.mode });
    expect(dialog).toHaveAttribute('data-size', 'md');
    // The old description paragraph and the switch's second line are InfoTips now.
    expect(within(dialog).queryByText(/Configure a lead model for planning/)).toBeNull();
    expect(dialog.querySelector('.biorouter-modal-panel')).toBeNull();
  });

  it('reaches the switch’s explanation from the switch itself', async () => {
    render(<LeadWorkerSettings isOpen onClose={vi.fn()} />);
    const toggle = await screen.findByRole('switch', { name: leadWorkerCopy.mode });
    expect(
      screen.getByRole('button', { name: `About ${leadWorkerCopy.mode}` })
    ).toBeInTheDocument();
    // The InfoTip is a sibling of the label, so it cannot flip the switch.
    expect(toggle.closest('label')).toBeNull();
    // And it is described by it, so a screen reader hears the help on reaching the switch.
    expect(toggle).toHaveAccessibleDescription(leadWorkerCopy.modeHelp);
  });

  it('labels both model fields and gives each its own InfoTip', async () => {
    render(<LeadWorkerSettings isOpen onClose={vi.fn()} />);
    await screen.findByRole('switch', { name: leadWorkerCopy.mode });
    // react-select drops the combobox role from a disabled input, and the mode starts off, so the
    // label is the way to the field.
    expect(screen.getByLabelText(leadWorkerCopy.leadModel)).toBeInstanceOf(HTMLInputElement);
    expect(screen.getByLabelText(leadWorkerCopy.workerModel)).toBeInstanceOf(HTMLInputElement);
    for (const name of [leadWorkerCopy.leadModel, leadWorkerCopy.workerModel]) {
      expect(screen.getByRole('button', { name: `About ${name}` })).toBeInTheDocument();
    }
  });

  it('puts the three counts on rows with their own labels and InfoTips', async () => {
    render(<LeadWorkerSettings isOpen onClose={vi.fn()} />);
    await screen.findByRole('switch', { name: leadWorkerCopy.mode });
    for (const name of [
      leadWorkerCopy.leadTurns,
      leadWorkerCopy.failureThreshold,
      leadWorkerCopy.fallbackTurns,
    ]) {
      expect(screen.getByRole('spinbutton', { name })).toBeInTheDocument();
      expect(screen.getByRole('button', { name: `About ${name}` })).toBeInTheDocument();
    }
  });

  it('goes quiet, not away, while the mode is off', async () => {
    const { container } = render(<LeadWorkerSettings isOpen onClose={vi.fn()} />);
    await screen.findByRole('switch', { name: leadWorkerCopy.mode });
    await waitFor(() =>
      expect(document.querySelector('.br-model-fields')).toHaveAttribute('data-enabled', 'false')
    );
    expect(container).toBeTruthy();
    expect(screen.getByRole('spinbutton', { name: leadWorkerCopy.leadTurns })).toBeDisabled();
  });
});
