import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ConfigData } from '../../../types/config';

const mocks = vi.hoisted(() => ({
  upsert: vi.fn(),
  toastError: vi.fn(),
  toastSuccess: vi.fn(),
  config: {} as Record<string, unknown>,
}));

vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({
    config: mocks.config,
    upsert: mocks.upsert,
    refreshConfig: async () => {},
  }),
}));
vi.mock('../../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    currentProvider: 'versa_azure',
    refreshCurrentModelAndProvider: async () => {},
  }),
}));
vi.mock('../../../toasts', () => ({
  toastError: mocks.toastError,
  toastSuccess: mocks.toastSuccess,
}));

import ConfigSettings, { PRIVACY_SETTINGS_PATH } from './ConfigSettings';

// The config `GET /config` serves: the record is an OBJECT (W2-PRV-8).
const served: ConfigData = {
  BIOROUTER_MODE: 'auto',
  BIOROUTER_PRIVACY_TIERS: 'on',
  BIOROUTER_PRIVACY_MIXING_POLICY: 'standard',
  BIOROUTER_PRIVACY_TIERS_RECORD: {
    enabled: true,
    origin: 'settings',
    path: '/home/u/.config/biorouter/privacy-tiers.json',
    last_change: {
      via: 'settings',
      set_to: true,
      at: '2026-09-27T10:00:00Z',
      system_authenticated: false,
      user_action: true,
    },
  } as unknown as ConfigData[string],
};

function openEditor() {
  render(<ConfigSettings />);
  fireEvent.click(screen.getByRole('button', { name: /Edit configuration/ }));
  return screen.getByRole('dialog');
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.config = { ...served };
});

// W2-PRV-8: the editor showed the privacy record as "[object Object]" with a
// Save, and the master switch and mixing policy as free-text fields that can
// never save from there.
describe('ConfigSettings and the privacy settings', () => {
  it('shows the record read-only as fields, never "[object Object]"', () => {
    const dialog = openEditor();
    expect(within(dialog).queryByDisplayValue('[object Object]')).toBeNull();
    expect(dialog).not.toHaveTextContent('[object Object]');

    const summary = within(dialog).getByTestId('config-privacy-summary');
    expect(summary).toHaveTextContent('Privacy tiers');
    expect(summary).toHaveTextContent('On');
    expect(summary).toHaveTextContent('2026-09-27T10:00:00Z');
    expect(summary).toHaveTextContent(`Changed in${PRIVACY_SETTINGS_PATH}`);
    expect(summary).toHaveTextContent('Cross-institution mixingstandard');
    expect(summary).toHaveTextContent(`These are changed in ${PRIVACY_SETTINGS_PATH}`);
  });

  it('offers none of the three as a field with a Save', () => {
    const dialog = openEditor();
    for (const key of [
      'BIOROUTER_PRIVACY_TIERS',
      'BIOROUTER_PRIVACY_TIERS_RECORD',
      'BIOROUTER_PRIVACY_MIXING_POLICY',
    ]) {
      expect(within(dialog).queryByTitle(key)).toBeNull();
    }
    // An ordinary key is still editable.
    expect(within(dialog).getByTitle('BIOROUTER_MODE')).toBeInTheDocument();
  });

  it('links to the Privacy section it names', () => {
    const panel = document.createElement('section');
    panel.setAttribute('data-privacy-panel', '');
    panel.scrollIntoView = vi.fn();
    document.body.appendChild(panel);
    try {
      const dialog = openEditor();
      fireEvent.click(within(dialog).getByRole('button', { name: 'Go to Privacy' }));
      expect(panel.scrollIntoView).toHaveBeenCalled();
    } finally {
      panel.remove();
    }
  });

  it('shows any other structured value read-only', () => {
    mocks.config = { ...served, SOME_REPORT: { a: 1 } as unknown as ConfigData[string] };
    const dialog = openEditor();
    const value = within(dialog).getByTestId('config-readonly-SOME_REPORT');
    expect(value).toHaveTextContent('"a": 1');
    const row = value.closest('div.grid') as HTMLElement;
    expect(within(row).getByRole('button')).toBeDisabled();
  });

  it('puts the daemon’s refusal sentence in the toast body', async () => {
    const sentence =
      "'BIOROUTER_MODE' cannot be saved: the daemon said so, in a sentence for a person.";
    mocks.upsert.mockRejectedValue(sentence);
    const dialog = openEditor();
    const field = within(dialog).getByDisplayValue('auto');
    fireEvent.change(field, { target: { value: 'approve' } });
    const row = field.closest('div.grid') as HTMLElement;
    fireEvent.click(within(row).getByRole('button'));

    await waitFor(() => expect(mocks.toastError).toHaveBeenCalled());
    expect(mocks.toastError.mock.calls[0][0].msg).toContain(sentence);
  });
});
