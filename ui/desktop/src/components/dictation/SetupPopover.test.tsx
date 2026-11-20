import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { speechStatus } from './dictationTestKit';

const install = vi.hoisted(() => vi.fn(async () => undefined));
const activation = vi.hoisted(() => ({
  ready: true,
  checked: true,
  disabled: false,
  showChoice: true,
  needsSave: false,
  help: 'Terms from chats are suggested.',
  setChecked: vi.fn(),
  startInstall: vi.fn(),
  activationFailed: false,
  savedChoiceDiffered: false,
  saving: false,
  retryActivation: vi.fn(),
}));
vi.mock('../dictionary/activationChoice', () => ({
  useDictionaryActivationChoice: () => activation,
}));
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
  activation.ready = true;
  activation.activationFailed = false;
  activation.savedChoiceDiffered = false;
  activation.saving = false;
  activation.needsSave = false;
  activation.retryActivation.mockClear();
  Object.assign(window, {
    appConfig: { get: (key: string) => (key === 'BIOROUTER_LOCAL_BACKEND' ? true : undefined) },
  });
});

afterEach(() => {
  cleanup();
  install.mockClear();
});

describe('the setup popover', () => {
  it('keeps a retry action visible after the model installs but activation saving fails', () => {
    activation.activationFailed = true;
    activation.needsSave = true;
    const status = speechStatus({ active_model: turbo.id }, { ...turbo, installed: true });
    render(
      <SetupPopover status={status} progress={null} onNotNow={vi.fn()} onMoreInSettings={vi.fn()} />
    );
    expect(
      screen.getByText('Dictionary choice could not be saved. Retry to finish setting it up.')
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(activation.retryActivation).toHaveBeenCalledTimes(1);
    // Inline in the sentence's own size, so its line is not taller than the others.
    expect(screen.getByRole('button', { name: 'Retry' })).toHaveClass('h-auto', 'text-supporting');
  });
  it('draws the command in an install refusal as code', () => {
    const status = speechStatus(
      {
        active_model: null,
        install: {
          allowed: false,
          reason:
            'Dictation is not set up on this server. Ask the person who runs it to run `biorouter dictation install`.',
        },
      },
      turbo
    );
    render(
      <SetupPopover status={status} progress={null} onNotNow={vi.fn()} onMoreInSettings={vi.fn()} />
    );
    const line = screen.getByText(/Dictation is not set up on this server/);
    expect(line.textContent).not.toContain('`');
    expect(line.querySelector('code')).toHaveTextContent('biorouter dictation install');
  });
  it('shows when the server kept a different saved choice', () => {
    activation.savedChoiceDiffered = true;
    const status = speechStatus({ active_model: turbo.id }, { ...turbo, installed: true });
    render(
      <SetupPopover status={status} progress={null} onNotNow={vi.fn()} onMoreInSettings={vi.fn()} />
    );
    expect(screen.getByRole('status')).toHaveTextContent(
      'Dictionary kept the choice already saved here. Review it in Dictionary settings.'
    );
  });
  it('holds the first Install until the dictionary activation choice is resolved', () => {
    activation.ready = false;
    const status = speechStatus({ active_model: null }, turbo);
    render(
      <SetupPopover status={status} progress={null} onNotNow={vi.fn()} onMoreInSettings={vi.fn()} />
    );
    const button = screen.getByRole('button', { name: 'Install' });
    expect(button).toBeDisabled();
    fireEvent.click(button);
    expect(install).not.toHaveBeenCalled();
  });
  it('keeps the dictionary choice to one line, its explanation in an InfoTip', () => {
    const status = speechStatus({ active_model: null }, turbo);
    render(
      <SetupPopover status={status} progress={null} onNotNow={vi.fn()} onMoreInSettings={vi.fn()} />
    );
    const choice = screen.getByTestId('dictation-setup-suggest');
    expect(
      screen.getByRole('checkbox', { name: 'Suggest dictionary words from my chats' })
    ).toBeChecked();
    expect(
      screen.getByRole('button', { name: 'About Suggest dictionary words from my chats' })
    ).toBeInTheDocument();
    // The help is the InfoTip's description, never a visible paragraph under the box.
    expect(choice.querySelector('.text-supporting')).toBeNull();
  });

  it('names the model, its size and its folder, and says where speech is transcribed', () => {
    const status = speechStatus({ active_model: null }, turbo);
    render(
      <SetupPopover status={status} progress={null} onNotNow={vi.fn()} onMoreInSettings={vi.fn()} />
    );
    expect(screen.getByTestId('dictation-setup')).toHaveTextContent(
      "Dictation turns your speech into text on this computer. It needs a 574 MB speech model, Whisper large-v3 turbo, downloaded once from the Baranzini Lab's model repository on GitHub into /Users/wgu/.local/share/biorouter/dictation/models."
    );
    expect(screen.getByTestId('dictation-setup-privacy')).toHaveTextContent(
      'Speech is transcribed by Biorouter on this computer. Audio is not saved.'
    );
    // The privacy line says it once; MSG-SETUP no longer repeats it two lines above.
    const text = screen.getByTestId('dictation-setup').textContent ?? '';
    expect(text.split('Audio is not saved.').length - 1).toBe(1);
    // A display name that carries its own parentheses gets no second pair for the size.
    expect(text).not.toMatch(/\) \(/);
    // The name is drawn whole, so it never breaks at a hyphen of "large-v3-turbo".
    expect(screen.getByText('Whisper large-v3 turbo')).toHaveClass('br-dictation-setup__name');
  });

  it('draws a command in a failure as code, never with its backticks', () => {
    const offline =
      "This computer cannot reach Biorouter's model repository. On a computer that can, run `biorouter dictation urls` and download the files it lists.";
    const status = speechStatus(
      {
        active_model: null,
        install_job: {
          model: turbo.id,
          bytes_done: 0,
          bytes_total: turbo.bytes,
          bytes_per_second: 0,
          phase: 'failed',
          error: offline,
        },
      },
      turbo
    );
    render(
      <SetupPopover status={status} progress={null} onNotNow={vi.fn()} onMoreInSettings={vi.fn()} />
    );
    const alert = screen.getByRole('alert');
    expect(alert).toHaveTextContent('run biorouter dictation urls and download');
    expect(alert.textContent).not.toContain('`');
    expect(alert.querySelector('code')).toHaveTextContent('biorouter dictation urls');
    expect(alert).toHaveClass('br-dictation-setup__warning');
  });

  it('says a cancelled install as a record in the muted line, not as a failure', () => {
    const status = speechStatus(
      {
        active_model: null,
        install_job: {
          model: turbo.id,
          bytes_done: 16_000_000,
          bytes_total: turbo.bytes,
          bytes_per_second: 0,
          phase: 'failed',
          error: 'The install was canceled.',
        },
      },
      turbo
    );
    render(
      <SetupPopover status={status} progress={null} onNotNow={vi.fn()} onMoreInSettings={vi.fn()} />
    );
    expect(screen.queryByRole('alert')).toBeNull();
    const note = screen.getByText('The install was canceled.');
    expect(note).toHaveAttribute('role', 'status');
    expect(note).toHaveClass('br-dictation-setup__note');
    // Install is still offered, so the person can pick up where the partial stopped.
    expect(screen.getByRole('button', { name: 'Install' })).toBeEnabled();
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
    // The thin 4px bar Settings draws for the same install.
    expect(screen.getByRole('progressbar')).toHaveAttribute('data-size', 'thin');
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
