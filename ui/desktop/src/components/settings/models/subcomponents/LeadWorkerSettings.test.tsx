import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/react';
import { LeadWorkerSettings } from './LeadWorkerSettings';

// Mock predefined models utils to force provider-based options (no predefined list)
vi.mock('../predefinedModelsUtils', () => ({
  shouldShowPredefinedModels: () => false,
  getPredefinedModelsFromEnv: () => [],
}));

// Mocks for useConfig
const mockRead = vi.fn();
const mockUpsert = vi.fn();
const mockRemove = vi.fn();
const mockGetProviders = vi.fn();

vi.mock('../../../ConfigContext', () => ({
  useConfig: () => ({
    read: mockRead,
    upsert: mockUpsert,
    remove: mockRemove,
    getProviders: mockGetProviders,
  }),
}));

// Minimal mock for useModelAndProvider
vi.mock('../../../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    currentModel: null,
  }),
}));

describe('LeadWorkerSettings', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  const setupHappyPathMocks = () => {
    // reads
    mockRead.mockImplementation(async (key: string) => {
      switch (key) {
        case 'BIOROUTER_LEAD_MODEL':
          return 'my-custom-lead';
        case 'BIOROUTER_LEAD_PROVIDER':
          return 'anthropic';
        case 'BIOROUTER_LEAD_TURNS':
          return 3;
        case 'BIOROUTER_LEAD_FAILURE_THRESHOLD':
          return 2;
        case 'BIOROUTER_LEAD_FALLBACK_TURNS':
          return 2;
        case 'BIOROUTER_MODEL':
          return 'my-custom-worker';
        case 'BIOROUTER_PROVIDER':
          return 'openai';
        default:
          return null;
      }
    });

    // providers (options do NOT include the custom models above)
    mockGetProviders.mockResolvedValue([
      {
        is_configured: true,
        name: 'openai',
        metadata: {
          display_name: 'OpenAI',
          known_models: [{ name: 'gpt-4o' }, { name: 'gpt-4o-mini' }],
        },
      },
      {
        is_configured: true,
        name: 'anthropic',
        metadata: {
          display_name: 'Anthropic',
          known_models: [{ name: 'claude-3-5-sonnet' }],
        },
      },
    ]);

    // writers
    mockUpsert.mockResolvedValue(undefined);
    mockRemove.mockResolvedValue(undefined);
  };

  it('shows custom inputs for lead/worker when current models are unknown and saves them', async () => {
    setupHappyPathMocks();

    const onClose = vi.fn();
    render(<LeadWorkerSettings isOpen={true} onClose={onClose} />);

    // Wait for modal content (not loading)
    await waitFor(() => {
      expect(screen.getByText('Lead/worker mode')).toBeInTheDocument();
    });

    // Labels should be present with back-to-list controls
    await waitFor(() => {
      expect(screen.getByText('Lead model')).toBeInTheDocument();
      expect(screen.getByText('Worker model')).toBeInTheDocument();
      // Back to model list appears for each section when in custom mode
      const backLinks = screen.getAllByText('Back to model list');
      expect(backLinks.length).toBeGreaterThanOrEqual(2);
    });

    const inputs = screen.getAllByPlaceholderText('Type model name here') as HTMLInputElement[];
    expect(inputs.length).toBe(2);
    const [leadInput, workerInput] = inputs;
    expect(leadInput.value).toBe('my-custom-lead');
    expect(workerInput.value).toBe('my-custom-worker');

    // Save waits for a change: the saved pair as it was is nothing to write.
    const saveBtn = screen.getByRole('button', { name: 'Save settings' });
    expect(saveBtn).toBeDisabled();
    fireEvent.change(leadInput, { target: { value: 'my-custom-lead-2' } });
    expect(saveBtn).toBeEnabled();
    fireEvent.click(saveBtn);

    // Assert upserts for models (providers are optional but present in this setup)
    await waitFor(() => {
      expect(mockUpsert).toHaveBeenCalledWith('BIOROUTER_LEAD_MODEL', 'my-custom-lead-2', false);
      expect(mockUpsert).toHaveBeenCalledWith('BIOROUTER_MODEL', 'my-custom-worker', false);
      expect(mockUpsert).toHaveBeenCalledWith('BIOROUTER_LEAD_PROVIDER', 'anthropic', false);
      expect(mockUpsert).toHaveBeenCalledWith('BIOROUTER_PROVIDER', 'openai', false);
    });
  });

  // Vision V4-R7-5: a model id is a machine string. Chromium drew its red dotted misspelling line
  // under `my-custom-lead` on a correct value, and the platform may "correct" or capitalise it.
  // The number fields beside them are numbers and take nothing.
  it('leaves the two custom model fields unspellchecked, uncorrected and uncapitalised', async () => {
    setupHappyPathMocks();
    const { container } = render(<LeadWorkerSettings isOpen={true} onClose={vi.fn()} />);
    await waitFor(() => expect(screen.getAllByText('Back to model list')).toHaveLength(2));

    for (const id of ['lead-model', 'worker-model']) {
      const field = container.ownerDocument.getElementById(id);
      expect(field, id).not.toBeNull();
      expect(field, `${id} spellcheck`).toHaveAttribute('spellcheck', 'false');
      expect(field, `${id} autocorrect`).toHaveAttribute('autocorrect', 'off');
      expect(field, `${id} autocapitalize`).toHaveAttribute('autocapitalize', 'off');
    }
  });

  it('disables lead/worker and removes config when toggled off', async () => {
    setupHappyPathMocks();

    const onClose = vi.fn();
    render(<LeadWorkerSettings isOpen={true} onClose={onClose} />);

    await waitFor(() => {
      expect(screen.getByText('Lead/worker mode')).toBeInTheDocument();
    });

    // Toggle off. The enable control is a Radix Switch (role="switch"),
    // labelled "Lead/worker mode" via htmlFor/id, with state on aria-checked.
    const toggle = screen.getByRole('switch', { name: 'Lead/worker mode' });
    expect(toggle).toHaveAttribute('aria-checked', 'true');
    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute('aria-checked', 'false');

    const saveBtn = screen.getByRole('button', { name: 'Save settings' });
    expect(saveBtn).toBeEnabled();
    fireEvent.click(saveBtn);

    await waitFor(() => {
      expect(mockRemove).toHaveBeenCalledWith('BIOROUTER_LEAD_MODEL', false);
      expect(mockRemove).toHaveBeenCalledWith('BIOROUTER_LEAD_PROVIDER', false);
      expect(mockRemove).toHaveBeenCalledWith('BIOROUTER_LEAD_TURNS', false);
      expect(mockRemove).toHaveBeenCalledWith('BIOROUTER_LEAD_FAILURE_THRESHOLD', false);
      expect(mockRemove).toHaveBeenCalledWith('BIOROUTER_LEAD_FALLBACK_TURNS', false);
    });
  });
});
