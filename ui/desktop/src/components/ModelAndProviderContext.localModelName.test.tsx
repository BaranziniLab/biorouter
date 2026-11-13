/**
 * @vitest-environment jsdom
 */
import { render, screen, waitFor } from '@testing-library/react';
import { useEffect, useState } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ModelAndProviderProvider, useModelAndProvider } from './ModelAndProviderContext';

/**
 * W2-5. A Llama Server model is named the way the model picker lists it.
 *
 * The picker reads Llama Server's catalog and lists `qwen3.8-27b` as "Qwen3.8 27B", but
 * `getCurrentModelDisplayName` only knew predefined-model aliases, so the chip and Settings
 * then named the model `qwen3.8-27b`: two names for one choice, a click apart.
 */

const mocks = vi.hoisted(() => ({
  llamacppStatus: vi.fn(),
  llamacppWarmup: vi.fn(),
  setConfigProvider: vi.fn(),
  updateAgentProvider: vi.fn(),
  read: vi.fn(),
  getProviders: vi.fn(),
  refreshConfig: vi.fn(),
}));

vi.mock('../api', () => ({
  llamacppStatus: mocks.llamacppStatus,
  llamacppWarmup: mocks.llamacppWarmup,
  setConfigProvider: mocks.setConfigProvider,
  updateAgentProvider: mocks.updateAgentProvider,
}));

vi.mock('../toasts', () => ({ toastError: vi.fn(), toastSuccess: vi.fn() }));

vi.mock('./ConfigContext', () => ({
  useConfig: () => ({
    read: mocks.read,
    getProviders: mocks.getProviders,
    refreshConfig: mocks.refreshConfig,
  }),
}));

Object.defineProperty(window, 'appConfig', {
  writable: true,
  value: { get: () => undefined },
});

const catalogEntry = (name: string, displayName: string) => ({
  name,
  display_name: displayName,
  download_status: 'downloaded',
});

const status = {
  data: {
    sidecar: { state: 'stopped', model: null, warmed: false, detail: null },
    catalog: [
      catalogEntry('qwen3.8-27b', 'Qwen3.8 27B'),
      catalogEntry('gemma4-31b', 'Gemma 4 31B'),
    ],
    system: {},
  },
};

let selection = { model: 'qwen3.8-27b', provider: 'llamacpp' };

function NameProbe({ pinned }: { pinned?: { provider: string; model: string } }) {
  const { currentModel, getCurrentModelDisplayName, localModelDisplayName, loadLocalModelNames } =
    useModelAndProvider();
  const [name, setName] = useState('');
  useEffect(() => {
    void getCurrentModelDisplayName().then(setName);
  }, [currentModel, getCurrentModelDisplayName, setName]);
  useEffect(() => {
    if (pinned) void loadLocalModelNames();
  }, [pinned, loadLocalModelNames]);
  return (
    <>
      <div data-testid="display-name">{name}</div>
      <div data-testid="selection-name">
        {localModelDisplayName(selection.provider, selection.model) ?? 'none'}
      </div>
      {pinned && (
        <div data-testid="pinned-name">
          {localModelDisplayName(pinned.provider, pinned.model) ?? 'none'}
        </div>
      )}
    </>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  selection = { model: 'qwen3.8-27b', provider: 'llamacpp' };
  mocks.read.mockImplementation(async (key: string) => {
    if (key === 'BIOROUTER_MODEL') return selection.model;
    if (key === 'BIOROUTER_PROVIDER') return selection.provider;
    return null;
  });
  mocks.getProviders.mockResolvedValue([]);
  mocks.llamacppStatus.mockResolvedValue(status);
});

describe('a Llama Server model goes by its catalog name', () => {
  it("names the selection's local model as the picker does, from one catalog read", async () => {
    render(
      <ModelAndProviderProvider>
        <NameProbe />
      </ModelAndProviderProvider>
    );
    await waitFor(() =>
      expect(screen.getByTestId('display-name')).toHaveTextContent('Qwen3.8 27B')
    );
    expect(screen.getByTestId('selection-name')).toHaveTextContent('Qwen3.8 27B');
    // The mount, the selection's effect and the display name all asked; one read answered.
    expect(mocks.llamacppStatus).toHaveBeenCalledTimes(1);
  });

  it('keeps the id for a local model the catalog does not list', async () => {
    selection = { model: 'unsloth/Some-Model-GGUF:Q4_K_M', provider: 'llamacpp' };
    render(
      <ModelAndProviderProvider>
        <NameProbe />
      </ModelAndProviderProvider>
    );
    await waitFor(() =>
      expect(screen.getByTestId('display-name')).toHaveTextContent('unsloth/Some-Model-GGUF:Q4_K_M')
    );
    expect(screen.getByTestId('selection-name')).toHaveTextContent('none');
  });

  it('reads nothing from Llama Server for any other provider', async () => {
    selection = { model: 'qwen3.8-27b', provider: 'openrouter' };
    render(
      <ModelAndProviderProvider>
        <NameProbe />
      </ModelAndProviderProvider>
    );
    await waitFor(() =>
      expect(screen.getByTestId('display-name')).toHaveTextContent('qwen3.8-27b')
    );
    expect(screen.getByTestId('selection-name')).toHaveTextContent('none');
    expect(mocks.llamacppStatus).not.toHaveBeenCalled();
  });

  it('names a chat bound to a local model while the selection is elsewhere, once asked', async () => {
    selection = { model: 'gpt-5.5-2026-04-24', provider: 'versa_azure' };
    render(
      <ModelAndProviderProvider>
        <NameProbe pinned={{ provider: 'llamacpp', model: 'gemma4-31b' }} />
      </ModelAndProviderProvider>
    );
    await waitFor(() => expect(screen.getByTestId('pinned-name')).toHaveTextContent('Gemma 4 31B'));
    expect(screen.getByTestId('display-name')).toHaveTextContent('gpt-5.5-2026-04-24');
  });

  it('falls back to the id when the catalog cannot be read, and asks again next time', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    mocks.llamacppStatus.mockRejectedValue(new Error('daemon starting'));
    const { rerender } = render(
      <ModelAndProviderProvider>
        <NameProbe />
      </ModelAndProviderProvider>
    );
    await waitFor(() =>
      expect(screen.getByTestId('display-name')).toHaveTextContent('qwen3.8-27b')
    );
    expect(screen.getByTestId('selection-name')).toHaveTextContent('none');

    // The next surface that asks reads it again, and gets the name.
    mocks.llamacppStatus.mockResolvedValue(status);
    rerender(
      <ModelAndProviderProvider>
        <NameProbe pinned={{ provider: 'llamacpp', model: 'qwen3.8-27b' }} />
      </ModelAndProviderProvider>
    );
    await waitFor(() => expect(screen.getByTestId('pinned-name')).toHaveTextContent('Qwen3.8 27B'));
    warn.mockRestore();
  });
});
