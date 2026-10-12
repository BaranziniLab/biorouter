import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { SwitchModelModal } from './SwitchModelModal';
import { switchModelCopy } from '../copy';

/**
 * Vision V4-R7-5: "Enter a model not listed" opens a field for a model id (`gpt-5.5-2026-04-24`).
 * That is a machine string, not words, so Chromium's red dotted misspelling line under a correct
 * id was a lie, and the platform must not "correct" it or capitalise its first letter. The field
 * is `<Input machine />`. Everything else about the dialog has its own files.
 *
 * The model Select is replaced by one button that picks the "not listed" entry, because the
 * react-select menu is not what this file is about.
 */

const mocks = vi.hoisted(() => ({
  getProviders: vi.fn(),
  getProviderModels: vi.fn(),
  read: vi.fn(),
  changeModel: vi.fn(),
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
    currentModel: 'gpt-4o',
    currentProvider: 'openai',
  }),
}));

vi.mock('../predefinedModelsUtils', () => ({
  getPredefinedModelsFromEnv: () => [],
  shouldShowPredefinedModels: () => false,
}));

vi.mock('../../../ui/Select', () => ({
  Select: ({
    placeholder,
    onChange,
  }: {
    placeholder?: string;
    onChange?: (value: unknown) => void;
  }) =>
    placeholder?.startsWith('Provider') ? (
      <div data-testid="provider-select" />
    ) : (
      <button
        type="button"
        onClick={() => onChange?.({ value: 'custom', label: 'Not listed', provider: 'openai' })}
      >
        pick the model that is not listed
      </button>
    ),
}));

describe('SwitchModelModal: the custom model id is a machine string (V4-R7-5)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.getProviders.mockResolvedValue([
      {
        name: 'openai',
        is_configured: true,
        provider_type: 'Commercial',
        metadata: {
          name: 'openai',
          display_name: 'OpenAI',
          default_model: 'gpt-4o',
          known_models: [{ name: 'gpt-4o' }],
          allows_unlisted_models: true,
          config_keys: [],
        },
      },
    ]);
    mocks.getProviderModels.mockResolvedValue(['gpt-4o']);
    mocks.read.mockResolvedValue('');
    mocks.changeModel.mockResolvedValue(true);
  });

  it('leaves the field unspellchecked, uncorrected and uncapitalised', async () => {
    render(
      <SwitchModelModal
        sessionId={null}
        onClose={vi.fn()}
        setView={vi.fn()}
        initialProvider="openai"
        initialModel="gpt-4o"
      />
    );

    fireEvent.click(
      await screen.findByRole('button', { name: /pick the model that is not listed/ })
    );

    const field = await screen.findByPlaceholderText(switchModelCopy.customPlaceholder);
    expect(field).toHaveAttribute('spellcheck', 'false');
    expect(field).toHaveAttribute('autocorrect', 'off');
    expect(field).toHaveAttribute('autocapitalize', 'off');
  });
});
