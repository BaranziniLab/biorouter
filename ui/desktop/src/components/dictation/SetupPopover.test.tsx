import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { speechStatus } from './dictationTestKit';

const install = vi.hoisted(() => vi.fn(async () => undefined));
vi.mock('../settings/models/speechModelStore', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  installSpeechModel: install,
}));

import { SetupPopover, disclosureFor, setupModel } from './SetupPopover';

const turbo = {
  id: 'whisper-large-v3-turbo-q5_0',
  display_name: 'Whisper large-v3 turbo',
  installed: false,
  bytes: 574_041_195,
  recommended: true,
};

beforeEach(() => {
  Object.assign(window, {
    appConfig: { get: (key: string) => (key === 'BIOROUTER_LOCAL_BACKEND' ? true : undefined) },
  });
});

afterEach(() => {
  cleanup();
  install.mockClear();
});

describe('the setup popover', () => {
  it('names the model, its size and its folder, and says where speech is transcribed', () => {
    const status = speechStatus({ active_model: null }, turbo);
    render(
      <SetupPopover status={status} progress={null} onNotNow={vi.fn()} onMoreInSettings={vi.fn()} />
    );
    expect(screen.getByTestId('dictation-setup')).toHaveTextContent(
      "Dictation turns your speech into text on this computer. It needs a speech model: Whisper large-v3 turbo (574 MB), downloaded once from the Baranzini Lab's model repository on GitHub into /Users/wgu/.local/share/biorouter/dictation/models. Audio is not saved."
    );
    expect(screen.getByTestId('dictation-setup-privacy')).toHaveTextContent(
      'Speech is transcribed by Biorouter on this computer. Audio is not saved.'
    );
  });

  it('installs only from Install, and Not now and More in Settings do what they say', () => {
    const status = speechStatus({ active_model: null }, turbo);
    const onNotNow = vi.fn();
    const onMoreInSettings = vi.fn();
    render(
      <SetupPopover
        status={status}
        progress={null}
        onNotNow={onNotNow}
        onMoreInSettings={onMoreInSettings}
      />
    );
    expect(install).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Install' }));
    expect(install).toHaveBeenCalledWith('whisper-large-v3-turbo-q5_0');
    fireEvent.click(screen.getByRole('button', { name: 'Not now' }));
    expect(onNotNow).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole('button', { name: 'More in Settings' }));
    expect(onMoreInSettings).toHaveBeenCalledTimes(1);
  });

  it('shows the install progress in place of Install', () => {
    const status = speechStatus({ active_model: null }, turbo);
    render(
      <SetupPopover status={status} progress={42} onNotNow={vi.fn()} onMoreInSettings={vi.fn()} />
    );
    expect(screen.getByTestId('dictation-setup-progress')).toHaveTextContent(
      'Downloading speech model… 42%'
    );
    expect(screen.queryByRole('button', { name: 'Install' })).toBeNull();
  });

  it('never says this computer when the daemon is elsewhere', () => {
    Object.assign(window, { appConfig: { get: () => false } });
    const status = speechStatus({ active_model: null, host: 'lab-server' }, turbo);
    render(
      <SetupPopover status={status} progress={null} onNotNow={vi.fn()} onMoreInSettings={vi.fn()} />
    );
    expect(screen.getByTestId('dictation-setup')).toHaveTextContent(
      'into text on the Biorouter server at lab-server.'
    );
    expect(screen.getByTestId('dictation-setup')).not.toHaveTextContent('on this computer');
    expect(disclosureFor({ kind: 'external-backend', host: 'lab-server' })).toBe(
      "Audio is transcribed on lab-server, the computer running Biorouter's service. Models install there, not on this device."
    );
  });

  it('offers the model recommended for this computer', () => {
    const status = speechStatus({ active_model: null }, turbo);
    expect(setupModel(status)?.id).toBe('whisper-large-v3-turbo-q5_0');
    expect(setupModel(null)).toBeNull();
  });
});
