import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SetupPopover } from '../dictation/SetupPopover';
import type { DictationApi, DictationStatus } from '../dictation/dictationApi';
import { refreshDictationStatus, resetDictationStoreForTests } from '../dictation/dictationStore';
import { speechStatus } from '../dictation/dictationTestKit';
import SpeechModelSection from '../settings/models/SpeechModelSection';
import { DICTIONARY_LABELS } from './copy';
import { DICTIONARY_ROUTE } from './navigation';

const activationReads = vi.hoisted(() => ({
  config: vi.fn(),
  settings: vi.fn(),
}));

vi.mock('../../api', async () => ({
  ...(await vi.importActual<typeof import('../../api')>('../../api')),
  readAllConfig: activationReads.config,
  getDictionarySettings: activationReads.settings,
}));

/**
 * The **Edit your dictionary** links (DD-67): the setup popover offers it once a model is
 * ready, and **Settings > Models > Speech models** always does unless managed policy turns the
 * dictionary off. Both change the hash route to `/dictionary`, which also opens the sidebar's
 * Components group by the current route rule.
 */

const notInstalled = () =>
  speechStatus({ active_model: null }, { installed: false, recommended: true });

beforeEach(() => {
  activationReads.config.mockReset().mockResolvedValue({
    data: { config: { dictation_dictionary_activation_version: 1 } },
  });
  activationReads.settings.mockReset().mockResolvedValue({
    data: {
      suggest_from_chats: false,
      learn_from_chats: false,
      agent_writes: 'on_request',
      authority: { raise_settings: { allowed: false } },
    },
    response: { ok: true, status: 200 },
  });
  Object.assign(window, {
    appConfig: { get: (key: string) => (key === 'BIOROUTER_LOCAL_BACKEND' ? true : undefined) },
  });
  window.location.hash = '';
});

afterEach(() => {
  cleanup();
  resetDictationStoreForTests();
  window.location.hash = '';
});

describe('the setup popover’s link', () => {
  it('is absent before a model is installed and while one installs', () => {
    const { rerender } = render(
      <SetupPopover
        status={notInstalled()}
        progress={null}
        onNotNow={vi.fn()}
        onMoreInSettings={vi.fn()}
      />
    );
    expect(screen.queryByTestId('dictation-setup-edit-dictionary')).toBeNull();
    rerender(
      <SetupPopover
        status={notInstalled()}
        progress={40}
        onNotNow={vi.fn()}
        onMoreInSettings={vi.fn()}
      />
    );
    expect(screen.queryByTestId('dictation-setup-edit-dictionary')).toBeNull();
  });

  it('closes the popover and opens the Dictionary page once a model is ready', () => {
    const onNotNow = vi.fn();
    render(
      <SetupPopover
        status={speechStatus()}
        progress={null}
        onNotNow={onNotNow}
        onMoreInSettings={vi.fn()}
      />
    );
    const link = screen.getByTestId('dictation-setup-edit-dictionary');
    expect(link).toHaveTextContent(DICTIONARY_LABELS.editYourDictionary);
    fireEvent.click(link);
    expect(onNotNow).toHaveBeenCalled();
    expect(window.location.hash).toBe(`#${DICTIONARY_ROUTE}`);
  });

  it('is absent where managed policy turns the dictionary off, as the sidebar row is', () => {
    const status = speechStatus();
    status.policy = {
      ...status.policy,
      managed: true,
      dictionary: { allowed: false, reason: null },
    };
    render(
      <SetupPopover status={status} progress={null} onNotNow={vi.fn()} onMoreInSettings={vi.fn()} />
    );
    expect(screen.getByTestId('dictation-setup')).toBeInTheDocument();
    expect(screen.queryByTestId('dictation-setup-edit-dictionary')).toBeNull();
  });
});

describe('the Settings row', () => {
  async function serve(status: DictationStatus) {
    resetDictationStoreForTests({
      status: vi.fn(async () => ({ ok: true as const, status: 200, data: status })),
    } as unknown as DictationApi);
    await act(async () => {
      await refreshDictationStatus();
    });
  }

  it('links to the Dictionary page', async () => {
    await serve(speechStatus());
    render(<SpeechModelSection />);
    const button = await screen.findByTestId('speech-edit-dictionary');
    expect(button).toHaveTextContent(DICTIONARY_LABELS.editYourDictionary);
    fireEvent.click(button);
    expect(window.location.hash).toBe(`#${DICTIONARY_ROUTE}`);
  });

  it('is absent where managed policy turns the dictionary off', async () => {
    const status = speechStatus();
    status.policy = {
      ...status.policy,
      managed: true,
      dictionary: { allowed: false, reason: null },
    };
    await serve(status);
    render(<SpeechModelSection />);
    await screen.findByTestId('speech-models-section');
    expect(screen.queryByTestId('speech-edit-dictionary')).toBeNull();
  });
});
