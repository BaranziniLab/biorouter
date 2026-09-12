import { fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProviderDetails } from '../../../../../../api';
import DefaultProviderSetupForm, { type ConfigInput } from './DefaultProviderSetupForm';
import CustomProviderForm from './CustomProviderForm';

const mocks = vi.hoisted(() => ({ read: vi.fn() }));

vi.mock('../../../../../ConfigContext', () => ({
  useConfig: () => ({ read: mocks.read }),
}));

/**
 * F5 of the 2026-09-10 provider QA run: the Versa Bedrock card rendered its
 * Secret Access Key as `<input type="text">` with spellcheck on — on screen and
 * in the DOM as it was typed. The fixture is that card's real key set
 * (`versa_bedrock.rs`): two required secrets, two optional non-secrets with
 * defaults behind "Show 2 options".
 */
const bedrock = {
  name: 'versa_bedrock',
  is_configured: false,
  provider_type: 'Builtin',
  metadata: {
    name: 'versa_bedrock',
    display_name: 'Versa API Bedrock',
    description: '',
    default_model: '',
    known_models: [],
    model_doc_link: '',
    config_keys: [
      { name: 'VERSA_BEDROCK_ACCESS_KEY_ID', required: true, secret: true, default: null },
      { name: 'VERSA_BEDROCK_SECRET_ACCESS_KEY', required: true, secret: true, default: null },
      {
        name: 'AWS_ENDPOINT_URL_BEDROCK',
        required: false,
        secret: false,
        default: 'https://unified-api.ucsf.edu/general/awsai',
      },
      { name: 'AWS_REGION', required: false, secret: false, default: 'us-west-2' },
    ],
  },
} as unknown as ProviderDetails;

function Harness() {
  const [values, setValues] = useState<Record<string, ConfigInput>>({});
  return (
    <DefaultProviderSetupForm
      configValues={values}
      setConfigValues={setValues}
      provider={bedrock}
      validationErrors={{}}
    />
  );
}

/**
 * Found by what the QA screenshot shows in each empty field — its placeholder —
 * so the same queries run unchanged against the form before the fix, and the
 * assertion that fails there is the one about masking rather than a lookup.
 */
const SECRET_ACCESS_KEY = 'VERSA BEDROCK SECRET ACCESS KEY';
const ACCESS_KEY_ID = 'VERSA BEDROCK ACCESS KEY ID';

beforeEach(() => {
  vi.clearAllMocks();
  // Nothing stored yet — the state the QA run typed into.
  mocks.read.mockResolvedValue(null);
});

describe('DefaultProviderSetupForm — secrets are masked', () => {
  it('renders a secret parameter as a masked, unchecked, un-autofilled field', async () => {
    render(<Harness />);

    for (const placeholder of [SECRET_ACCESS_KEY, ACCESS_KEY_ID]) {
      const input = await screen.findByPlaceholderText(placeholder);
      expect(input).toHaveAttribute('type', 'password');
      expect(input).toHaveAttribute('autocomplete', 'off');
      expect(input).toHaveAttribute('spellcheck', 'false');
    }
  });

  // The control: without it, the case above passes for a form that masks
  // everything — an endpoint URL nobody can read back is its own defect.
  it('leaves a non-secret parameter readable', async () => {
    render(<Harness />);
    fireEvent.click(await screen.findByText(/Show 2 options/));

    expect(screen.getByDisplayValue('us-west-2')).toHaveAttribute('type', 'text');
    expect(screen.getByDisplayValue('https://unified-api.ucsf.edu/general/awsai')).toHaveAttribute(
      'type',
      'text'
    );
  });

  // ⚠ By the config key, never by the words: the reveal toggle's accessible
  // name is "Show Secret Access Key", so a query for the words would match the
  // button as well as the field.
  it('labels each field, so a screen reader names the masked one', async () => {
    render(<Harness />);
    const input = await screen.findByPlaceholderText(SECRET_ACCESS_KEY);
    expect(screen.getByLabelText(/\(VERSA_BEDROCK_SECRET_ACCESS_KEY\)/)).toBe(input);
  });

  it('stays masked while typing, and reveals only on an explicit toggle', async () => {
    render(<Harness />);
    const input = await screen.findByPlaceholderText(SECRET_ACCESS_KEY);

    fireEvent.change(input, { target: { value: 'dummy-not-a-real-secret' } });
    expect(input).toHaveAttribute('type', 'password');

    const reveal = screen.getByRole('button', { name: 'Show Secret Access Key' });
    expect(reveal).toHaveAttribute('aria-pressed', 'false');
    fireEvent.click(reveal);
    expect(input).toHaveAttribute('type', 'text');
    // Revealed is exactly when a spellchecker would otherwise see the value.
    expect(input).toHaveAttribute('spellcheck', 'false');

    fireEvent.click(screen.getByRole('button', { name: 'Hide Secret Access Key' }));
    expect(input).toHaveAttribute('type', 'password');
    expect(input).toHaveValue('dummy-not-a-real-secret');
  });
});

describe('the two provider forms mask a key the same way', () => {
  // The custom form's "local model" checkbox is Radix Themes', which measures
  // itself with a ResizeObserver jsdom does not have.
  beforeAll(() => {
    vi.stubGlobal(
      'ResizeObserver',
      class {
        observe() {}
        unobserve() {}
        disconnect() {}
      }
    );
  });
  afterAll(() => {
    vi.unstubAllGlobals();
  });

  it('gives the custom-provider key the same masked field and the same toggle', () => {
    const { container } = render(
      <CustomProviderForm onSubmit={vi.fn()} onCancel={vi.fn()} initialData={null} />
    );
    const input = container.querySelector('#api-key') as HTMLInputElement;
    expect(input).toHaveAttribute('type', 'password');
    expect(input).toHaveAttribute('autocomplete', 'off');
    expect(input).toHaveAttribute('spellcheck', 'false');

    fireEvent.click(screen.getByRole('button', { name: 'Show API Key' }));
    expect(input).toHaveAttribute('type', 'text');
  });
});
