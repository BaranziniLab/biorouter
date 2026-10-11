import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ConfigData } from '../../../types/config';
import { configFieldLabel } from './configLabels';

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
    getProviders: async () => [],
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
import { configCopy } from './copy';

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

// The provider name is looked up after the first paint, so the render is awaited inside `act`.
async function openEditor() {
  await act(async () => {
    render(<ConfigSettings />);
  });
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
  it('shows the record read-only as fields, never "[object Object]"', async () => {
    const dialog = await openEditor();
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

  it('offers none of the three as a field with a Save', async () => {
    const dialog = await openEditor();
    for (const key of [
      'BIOROUTER_PRIVACY_TIERS',
      'BIOROUTER_PRIVACY_TIERS_RECORD',
      'BIOROUTER_PRIVACY_MIXING_POLICY',
    ]) {
      // A field is found by its label (the key itself is that label's tooltip, not a `title`).
      expect(within(dialog).queryByLabelText(configFieldLabel(key))).toBeNull();
    }
    // An ordinary key is still editable.
    expect(within(dialog).getByLabelText(configFieldLabel('BIOROUTER_MODE'))).toBeInTheDocument();
  });

  it('links to the Privacy section it names', async () => {
    const panel = document.createElement('section');
    panel.setAttribute('data-privacy-panel', '');
    panel.scrollIntoView = vi.fn();
    document.body.appendChild(panel);
    try {
      const dialog = await openEditor();
      fireEvent.click(within(dialog).getByRole('button', { name: 'Go to Privacy' }));
      expect(panel.scrollIntoView).toHaveBeenCalled();
    } finally {
      panel.remove();
    }
  });

  it('shows any other structured value read-only', async () => {
    mocks.config = { ...served, SOME_REPORT: { a: 1 } as unknown as ConfigData[string] };
    const dialog = await openEditor();
    const value = within(dialog).getByTestId('config-readonly-SOME_REPORT');
    expect(value).toHaveTextContent('"a": 1');
    const row = value.closest('div.grid') as HTMLElement;
    expect(within(row).getByRole('button')).toBeDisabled();
  });

  it('puts the daemon’s refusal sentence in the toast body', async () => {
    const sentence =
      "'BIOROUTER_MODE' cannot be saved: the daemon said so, in a sentence for a person.";
    mocks.upsert.mockRejectedValue(sentence);
    const dialog = await openEditor();
    const field = within(dialog).getByDisplayValue('auto');
    fireEvent.change(field, { target: { value: 'approve' } });
    const row = field.closest('div.grid') as HTMLElement;
    fireEvent.click(within(row).getByRole('button'));

    await waitFor(() => expect(mocks.toastError).toHaveBeenCalled());
    expect(mocks.toastError.mock.calls[0][0].msg).toContain(sentence);
  });
});

// Spec 3.14: the editor is a ModalShell, titled "Configuration", with no icon and no lowercase
// brand, and its fields are named by their labels.
describe('ConfigSettings editor', async () => {
  it('is a large dialog titled Configuration, with no description and no lowercase brand', async () => {
    const dialog = await openEditor();
    expect(dialog).toHaveAttribute('data-size', 'lg');
    expect(
      within(dialog).getByRole('heading', { name: configCopy.dialogTitle })
    ).toBeInTheDocument();
    expect(dialog.textContent).not.toMatch(/biorouter configuration/);
    expect(dialog.textContent).not.toMatch(/current settings for/);
    expect(dialog.querySelector('svg[class*="lucide-file"]')).toBeNull();
  });

  it('names each field by its label and keeps no native title or placeholder help', async () => {
    const dialog = await openEditor();
    const field = within(dialog).getByLabelText(configFieldLabel('BIOROUTER_MODE'));
    expect(field).toHaveValue('auto');
    expect(field).not.toHaveAttribute('placeholder');
    expect(dialog.querySelector('[title]')).toBeNull();
  });

  it('keeps exactly one button in each field row', async () => {
    const dialog = await openEditor();
    const field = within(dialog).getByLabelText(configFieldLabel('BIOROUTER_MODE'));
    const row = field.closest('div.grid') as HTMLElement;
    expect(within(row).getAllByRole('button')).toHaveLength(1);
    expect(within(row).getByRole('button')).toHaveAccessibleName(
      configCopy.saveLabel(configFieldLabel('BIOROUTER_MODE'))
    );
  });

  it('closes with Done and gives focus back to the Edit button', async () => {
    render(<ConfigSettings />);
    const edit = screen.getByRole('button', { name: configCopy.editName });
    fireEvent.click(edit);
    fireEvent.click(
      within(screen.getByRole('dialog')).getByRole('button', { name: configCopy.done })
    );
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(edit).toHaveFocus();
  });

  it('leaves with Escape, discarding unsaved edits, and focus returns to Edit', async () => {
    const user = userEvent.setup();
    render(<ConfigSettings />);
    const edit = screen.getByRole('button', { name: configCopy.editName });
    fireEvent.click(edit);
    const field = within(screen.getByRole('dialog')).getByLabelText(
      configFieldLabel('BIOROUTER_MODE')
    );
    fireEvent.change(field, { target: { value: 'approve' } });

    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(edit).toHaveFocus();

    // Reopened, the field shows what is saved, not what was typed.
    fireEvent.click(edit);
    expect(
      within(screen.getByRole('dialog')).getByLabelText(configFieldLabel('BIOROUTER_MODE'))
    ).toHaveValue('auto');
  });

  it('shows the key itself as the label’s tooltip, in the mono face', async () => {
    const user = userEvent.setup();
    const dialog = await openEditor();
    const label = within(dialog).getByText(configFieldLabel('BIOROUTER_MODE'));

    await user.hover(label);
    const tip = await screen.findByRole('tooltip');
    expect(tip).toHaveTextContent('BIOROUTER_MODE');
    expect(tip.querySelector('.font-mono')).not.toBeNull();
  });

  // V4-13: the labels were Title Case beside lower-case runs ("computer use capabilities
  // version", "Aws Endpoint Url Bedrock", "brsdk tracing"), two wrapping to two lines.
  it('names every field in sentence case, with acronyms and the brand spelled out', async () => {
    mocks.config = {
      ...served,
      BIOROUTER_AUTO_COMPACT_THRESHOLD: 0.8,
      computer_use_capabilities_version: 2,
      AWS_ENDPOINT_URL_BEDROCK: 'https://example.edu/awsai',
      context_about_biorouter: true,
      brsdk_tracing: '',
    };
    const dialog = await openEditor();

    for (const name of [
      'Auto compact threshold',
      'Computer use capabilities version',
      'AWS endpoint URL Bedrock',
      'Context about Biorouter',
      'App SDK tracing',
    ]) {
      expect(within(dialog).getByLabelText(name)).toBeInTheDocument();
    }
    expect(within(dialog).queryByText(/Aws Endpoint|Biorouter Auto|brsdk/)).toBeNull();
  });

  it('keeps the key as each label’s tooltip, whatever the label says', async () => {
    mocks.config = { ...served, computer_use_capabilities_version: 2 };
    const user = userEvent.setup();
    const dialog = await openEditor();

    await user.hover(within(dialog).getByText('Computer use capabilities version'));
    expect(await screen.findByRole('tooltip')).toHaveTextContent(
      'computer_use_capabilities_version'
    );
  });
});

// V4-13: the privacy summary was a bordered box inside the dialog, a box in a box. It is plain
// rows on the fields' own label column now.
describe('ConfigSettings privacy summary', async () => {
  it('is plain rows: no border, no fill, no rounded box', async () => {
    const summary = within(await openEditor()).getByTestId('config-privacy-summary');
    const own = [summary, ...summary.querySelectorAll('dl, dl > div, dt, dd')]
      .map((node) => node.getAttribute('class') ?? '')
      .join(' ');
    expect(own).not.toMatch(/\bborder\b|border-|\brounded|\bbg-|\bshadow/);
  });

  it('lists its rows as a definition list, label then value', async () => {
    const summary = within(await openEditor()).getByTestId('config-privacy-summary');
    const terms = [...summary.querySelectorAll('dt')].map((node) => node.textContent);
    expect(terms).toEqual([
      'Privacy tiers',
      'Recorded as',
      'Last changed',
      'Changed in',
      'Cross-institution mixing',
    ]);
    expect(summary.querySelectorAll('dd')).toHaveLength(terms.length);
  });

  it('shares one label column with the fields, so a value starts where the inputs do', async () => {
    const here = dirname(fileURLToPath(import.meta.url));
    const css = readFileSync(resolve(here, 'config.css'), 'utf8');
    const rule = (selector: string) => css.slice(css.indexOf(selector)).split('}')[0] ?? '';

    expect(rule('.br-config-row {')).toContain('var(--br-config-label-col)');
    expect(rule('.br-config-summary-list {')).toContain('var(--br-config-label-col)');
    // The column is declared once, on the body both lists sit in.
    expect(css.match(/--br-config-label-col:/g)).toHaveLength(1);
    expect(rule('.br-config-body {')).toContain('--br-config-label-col');

    const dialog = await openEditor();
    expect(dialog.querySelector('.br-config-body .br-config-summary-list')).not.toBeNull();
    expect(dialog.querySelector('.br-config-body .br-config-row')).not.toBeNull();
  });
});

// R-27 (fills before lines): the editor draws no outline of its own. A field rests on the filled
// field recipe, a structured value is the subtle fill with a transparent edge, and an edit that is
// not saved yet is a wash instead of a blue outline (which also hid the focus edge: a utility
// beats the field's layered `border-color`). jsdom runs no Tailwind, so this pins the classes;
// `settingsBFill.browser.test.tsx` measures what Chromium paints from them.
describe('ConfigSettings under the fill rule', async () => {
  it('marks an unsaved edit with the info wash, never an outline', async () => {
    const dialog = await openEditor();
    const field = within(dialog).getByLabelText(configFieldLabel('BIOROUTER_MODE'));
    expect(field.className).not.toMatch(/\bbg-wash-info\b/);

    fireEvent.change(field, { target: { value: 'approve' } });
    expect(field.className).toMatch(/\bbg-wash-info\b/);
    // No border colour of its own: the layered field recipe draws the focus and danger edges.
    expect(field.className).not.toMatch(/(^|\s)border-border-/);

    // Back to the saved value: the wash goes with the edit.
    fireEvent.change(field, { target: { value: 'auto' } });
    expect(field.className).not.toMatch(/\bbg-wash-info\b/);
  });

  it('leaves every field to the shared field recipe: no resting outline of its own', async () => {
    const dialog = await openEditor();
    const field = within(dialog).getByLabelText(configFieldLabel('BIOROUTER_MODE'));
    expect(field).toHaveClass('br-field');
    // (`aria-invalid:border-border-danger` is the primitive's own variant, so only a bare one counts.)
    expect(field.className).not.toMatch(/(^|\s)border-(border-|[a-z]+-\d)/);
  });

  it('draws a structured value as the subtle fill with a transparent 1px edge', async () => {
    mocks.config = { ...served, SOME_REPORT: { a: 1 } as unknown as ConfigData[string] };
    const value = within(await openEditor()).getByTestId('config-readonly-SOME_REPORT');
    expect(value).toHaveClass('bg-background-fill', 'border', 'border-transparent');
    // The muted ground it used to take is a different token on a recessed pane, and `border-0`
    // would leave forced colours with no edge to draw.
    expect(value.className).not.toMatch(/bg-background-muted|\bborder-0\b/);
  });
});
