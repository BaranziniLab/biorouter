import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { speechStatus } from '../dictation/dictationTestKit';
import { settings } from './dictionaryTestKit';

const api = vi.hoisted(() => ({
  readAllConfig: vi.fn(),
  activateDictionary: vi.fn(),
}));
const dictionary = vi.hoisted(() => ({
  loadDictionarySettings: vi.fn(),
  refreshDictionary: vi.fn(),
}));
vi.mock('../../api', () => api);
vi.mock('./useDictionary', () => dictionary);
vi.mock('../../utils/userAction', () => ({
  userActionHeaders: vi.fn(async () => ({ 'X-User-Action': 'proof' })),
}));

import { useDictionaryActivationChoice } from './activationChoice';

const modelId = 'whisper-small.en-q8_0';
const beforeInstall = speechStatus({}, { installed: false });
const afterInstall = speechStatus({}, { installed: true });

beforeEach(() => {
  for (const mock of Object.values(api)) mock.mockReset();
  for (const mock of Object.values(dictionary)) mock.mockReset();
  dictionary.loadDictionarySettings.mockResolvedValue(settings({ suggest_from_chats: false }));
  dictionary.refreshDictionary.mockResolvedValue(undefined);
  api.activateDictionary.mockResolvedValue({ data: settings({ suggest_from_chats: true }) });
});

describe('first dictionary activation choice', () => {
  it('waits for the marker and authority decision before a first install can be armed', async () => {
    let resolveConfig!: (value: { data: { config: Record<string, unknown> } }) => void;
    api.readAllConfig.mockReturnValue(
      new Promise((resolve) => {
        resolveConfig = resolve;
      })
    );
    const { result, rerender } = renderHook(({ status }) => useDictionaryActivationChoice(status), {
      initialProps: { status: beforeInstall },
    });
    expect(result.current.ready).toBe(false);
    expect(result.current.showChoice).toBe(false);
    act(() => result.current.startInstall(modelId, true));
    rerender({ status: afterInstall });
    expect(api.activateDictionary).not.toHaveBeenCalled();
    rerender({ status: beforeInstall });
    await act(async () => resolveConfig({ data: { config: {} } }));
    await waitFor(() => expect(result.current.showChoice).toBe(true));
    expect(result.current.ready).toBe(true);
    act(() => result.current.startInstall(modelId, true));
    rerender({ status: afterInstall });
    await waitFor(() => expect(api.activateDictionary).toHaveBeenCalledTimes(1));
  });

  it('starts ticked only with no activation marker and posts after the first verified install', async () => {
    api.readAllConfig.mockResolvedValue({ data: { config: {} } });
    const { result, rerender } = renderHook(({ status }) => useDictionaryActivationChoice(status), {
      initialProps: { status: beforeInstall },
    });
    await waitFor(() => expect(result.current.showChoice).toBe(true));
    expect(result.current.checked).toBe(true);
    act(() => result.current.startInstall(modelId, true));
    expect(api.activateDictionary).not.toHaveBeenCalled();
    rerender({ status: afterInstall });
    await waitFor(() =>
      expect(api.activateDictionary).toHaveBeenCalledWith(
        expect.objectContaining({ body: { suggest_from_chats: true, first_install: true } })
      )
    );
    await waitFor(() => expect(result.current.showChoice).toBe(false));
  });

  it('saves the choice when Install is pressed again after a failed or cancelled install', async () => {
    // Walking setup in the dev app (2026-11-19): an install that failed offline, then Install
    // again and a finished download, left Settings asking "Finish setting up Dictionary for new
    // chats" with Save choice. The status still held the first attempt's failed job when the
    // second was armed, and that old failure disarmed it before the download even began.
    api.readAllConfig.mockResolvedValue({ data: { config: {} } });
    const job = (phase: 'failed' | 'downloading', bytes_done: number, error: string | null) => ({
      model: modelId,
      bytes_done,
      bytes_total: 264_477_561,
      bytes_per_second: 0,
      phase,
      error,
    });
    const offline = speechStatus(
      { install_job: job('failed', 0, "This computer cannot reach Biorouter's model repository.") },
      { installed: false }
    );
    const downloading = speechStatus(
      { install_job: job('downloading', 1_000_000, null) },
      { installed: false }
    );
    const { result, rerender } = renderHook(({ status }) => useDictionaryActivationChoice(status), {
      initialProps: { status: offline },
    });
    await waitFor(() => expect(result.current.showChoice).toBe(true));
    act(() => result.current.startInstall(modelId, true));
    rerender({ status: offline });
    rerender({ status: downloading });
    rerender({ status: afterInstall });
    await waitFor(() =>
      expect(api.activateDictionary).toHaveBeenCalledWith(
        expect.objectContaining({ body: { suggest_from_chats: true, first_install: true } })
      )
    );
    await waitFor(() => expect(result.current.showChoice).toBe(false));
  });

  it('lets a new failure end the armed install, so a later model never saves the choice by surprise', async () => {
    api.readAllConfig.mockResolvedValue({ data: { config: {} } });
    const job = (phase: 'failed' | 'downloading', bytes_done: number, error: string | null) => ({
      model: modelId,
      bytes_done,
      bytes_total: 264_477_561,
      bytes_per_second: 0,
      phase,
      error,
    });
    const { result, rerender } = renderHook(({ status }) => useDictionaryActivationChoice(status), {
      initialProps: { status: beforeInstall },
    });
    await waitFor(() => expect(result.current.showChoice).toBe(true));
    act(() => result.current.startInstall(modelId, true));
    rerender({
      status: speechStatus({ install_job: job('downloading', 5, null) }, { installed: false }),
    });
    rerender({
      status: speechStatus(
        { install_job: job('failed', 9, 'The install was canceled.') },
        { installed: false }
      ),
    });
    rerender({ status: afterInstall });
    // The choice was armed for an install that ended; a model that appears later (a Rescan, the
    // CLI) leaves the choice to the person, as Save choice.
    await waitFor(() => expect(result.current.needsSave).toBe(true));
    expect(api.activateDictionary).not.toHaveBeenCalled();
  });

  it('keeps a verified installation retryable when saving the activation choice fails', async () => {
    api.readAllConfig.mockResolvedValue({ data: { config: {} } });
    api.activateDictionary
      .mockResolvedValueOnce({ error: { message: 'Config could not be written.' } })
      .mockResolvedValueOnce({ data: settings({ suggest_from_chats: true }) });
    const { result, rerender } = renderHook(({ status }) => useDictionaryActivationChoice(status), {
      initialProps: { status: beforeInstall },
    });
    await waitFor(() => expect(result.current.showChoice).toBe(true));
    act(() => result.current.startInstall(modelId, true));
    rerender({ status: afterInstall });
    await waitFor(() => expect(result.current.activationFailed).toBe(true));
    expect(result.current.showChoice).toBe(true);
    expect(api.activateDictionary).toHaveBeenCalledTimes(1);
    await act(async () => result.current.retryActivation());
    await waitFor(() => expect(api.activateDictionary).toHaveBeenCalledTimes(2));
    expect(result.current.activationFailed).toBe(false);
    expect(result.current.showChoice).toBe(false);
  });

  it('locks a lost-response choice and reconciles the saved setting without re-enabling it', async () => {
    api.readAllConfig.mockResolvedValue({ data: { config: {} } });
    api.activateDictionary
      .mockRejectedValueOnce(new Error('response lost after commit'))
      .mockResolvedValueOnce({ data: settings({ suggest_from_chats: false }) });
    const { result, rerender } = renderHook(({ status }) => useDictionaryActivationChoice(status), {
      initialProps: { status: beforeInstall },
    });
    await waitFor(() => expect(result.current.showChoice).toBe(true));
    act(() => result.current.startInstall(modelId, true));
    rerender({ status: afterInstall });
    await waitFor(() => expect(result.current.activationFailed).toBe(true));
    expect(result.current.disabled).toBe(true);
    act(() => result.current.setChecked(false));
    expect(result.current.checked).toBe(true);
    await act(async () => result.current.retryActivation());
    expect(api.activateDictionary).toHaveBeenNthCalledWith(
      2,
      expect.objectContaining({ body: { suggest_from_chats: true, first_install: true } })
    );
    expect(result.current.checked).toBe(false);
    expect(result.current.savedChoiceDiffered).toBe(true);
    expect(result.current.showChoice).toBe(false);
  });

  it('offers Save choice after remount when the model is installed but the marker is absent', async () => {
    api.readAllConfig.mockResolvedValue({ data: { config: {} } });
    const { result } = renderHook(() => useDictionaryActivationChoice(afterInstall));
    await waitFor(() => expect(result.current.needsSave).toBe(true));
    expect(api.activateDictionary).not.toHaveBeenCalled();
    await act(async () => result.current.retryActivation());
    expect(api.activateDictionary).toHaveBeenCalledWith(
      expect.objectContaining({ body: { suggest_from_chats: true, first_install: true } })
    );
  });

  it('preserves saved off after all models are removed and a model is reinstalled', async () => {
    api.readAllConfig.mockResolvedValue({
      data: { config: { dictation_dictionary_activation_version: 1 } },
    });
    const { result, rerender } = renderHook(({ status }) => useDictionaryActivationChoice(status), {
      initialProps: { status: beforeInstall },
    });
    await waitFor(() => expect(result.current.ready).toBe(true));
    expect(result.current.disabled).toBe(true);
    expect(result.current.showChoice).toBe(false);
    expect(result.current.checked).toBe(false);
    act(() => result.current.startInstall(modelId, true));
    rerender({ status: afterInstall });
    expect(api.activateDictionary).not.toHaveBeenCalled();
  });

  it('does not arm activation if settings, marker, or authority is unknown', async () => {
    api.readAllConfig.mockRejectedValue(new Error('config unavailable'));
    const { result, rerender } = renderHook(({ status }) => useDictionaryActivationChoice(status), {
      initialProps: { status: beforeInstall },
    });
    await waitFor(() => expect(result.current.disabled).toBe(true));
    act(() => result.current.startInstall(modelId, true));
    rerender({ status: afterInstall });
    expect(api.activateDictionary).not.toHaveBeenCalled();
  });

  it('keeps the first install unarmed when settings fail to load', async () => {
    dictionary.loadDictionarySettings.mockResolvedValue(null);
    api.readAllConfig.mockResolvedValue({ data: { config: {} } });
    const { result, rerender } = renderHook(({ status }) => useDictionaryActivationChoice(status), {
      initialProps: { status: beforeInstall },
    });
    await waitFor(() => expect(dictionary.loadDictionarySettings).toHaveBeenCalled());
    expect(result.current.showChoice).toBe(false);
    act(() => result.current.startInstall(modelId, true));
    rerender({ status: afterInstall });
    expect(api.activateDictionary).not.toHaveBeenCalled();
  });

  it('keeps a keyless or otherwise unproven caller unarmed', async () => {
    dictionary.loadDictionarySettings.mockResolvedValue(
      settings({
        authority: {
          ...settings().authority,
          raise_settings: { allowed: false, reason: 'Use the host command.' },
        },
      })
    );
    api.readAllConfig.mockResolvedValue({ data: { config: {} } });
    const { result, rerender } = renderHook(({ status }) => useDictionaryActivationChoice(status), {
      initialProps: { status: beforeInstall },
    });
    await waitFor(() => expect(api.readAllConfig).toHaveBeenCalled());
    expect(result.current.showChoice).toBe(false);
    act(() => result.current.startInstall(modelId, true));
    rerender({ status: afterInstall });
    expect(api.activateDictionary).not.toHaveBeenCalled();
  });
});
