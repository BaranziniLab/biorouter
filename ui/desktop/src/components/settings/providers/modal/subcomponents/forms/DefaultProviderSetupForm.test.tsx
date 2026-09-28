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

function Harness({ provider = bedrock }: { provider?: ProviderDetails }) {
  const [values, setValues] = useState<Record<string, ConfigInput>>({});
  return (
    <DefaultProviderSetupForm
      configValues={values}
      setConfigValues={setValues}
      provider={provider}
      validationErrors={{}}
    />
  );
}

/**
 * Found by what the QA screenshot shows in each empty field — its placeholder —
 * so the same queries run unchanged against the form before the fix, and the
 * assertion that fails there is the one about masking rather than a lookup.
 */
// W2-PRV-13: the placeholder is the field's name in words. It was the env var
// with spaces ('VERSA BEDROCK SECRET ACCESS KEY').
const SECRET_ACCESS_KEY = 'Secret Access Key';
const ACCESS_KEY_ID = 'Access Key Id';

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

/**
 * PROVIDERS-4 of the 2026-09-27 Crew QA audit. The public Azure OpenAI card is
 * for the user's OWN Azure resource, but its endpoint field came up filled in
 * with UCSF's Versa gateway, and a field left alone is saved. A non-UCSF user
 * who typed only a key and a deployment sent both, with the transcript, to
 * UCSF. The fixture is that card's key set as the daemon now serves it
 * (`azure.rs`): the endpoint is required and has no default.
 */
const azure = {
  name: 'azure_openai',
  is_configured: false,
  provider_type: 'Builtin',
  metadata: {
    name: 'azure_openai',
    display_name: 'Azure OpenAI',
    description: '',
    default_model: 'gpt-6-sol-2026-09-22',
    known_models: [],
    model_doc_link: '',
    config_keys: [
      { name: 'AZURE_OPENAI_ENDPOINT', required: true, secret: false, default: null },
      { name: 'AZURE_OPENAI_DEPLOYMENT_NAME', required: true, secret: false, default: null },
      {
        name: 'AZURE_OPENAI_API_VERSION',
        required: true,
        secret: false,
        default: '2025-01-01-preview',
      },
      { name: 'AZURE_OPENAI_API_KEY', required: false, secret: true, default: '' },
    ],
  },
} as unknown as ProviderDetails;

describe('DefaultProviderSetupForm — the Azure OpenAI endpoint is the user’s own', () => {
  it('leaves the endpoint empty, with a placeholder that says whose it is', async () => {
    render(<Harness provider={azure} />);

    const endpoint = await screen.findByLabelText(/\(AZURE_OPENAI_ENDPOINT\)/);
    expect(endpoint).toHaveValue('');
    expect(endpoint).toHaveAttribute('placeholder', 'https://<your-resource>.openai.azure.com');
  });

  it('fills in no value that points at the UCSF gateway', async () => {
    render(<Harness provider={azure} />);
    await screen.findByLabelText(/\(AZURE_OPENAI_ENDPOINT\)/);

    expect(screen.queryByDisplayValue(/unified-api\.ucsf\.edu/)).toBeNull();
  });

  // The control: a default that IS right for every user still arrives as a
  // value, so the case above is about this one key, not about defaults.
  it('still fills in the API version, which is the same for everyone', async () => {
    render(<Harness provider={azure} />);

    expect(await screen.findByLabelText(/\(AZURE_OPENAI_API_VERSION\)/)).toHaveValue(
      '2025-01-01-preview'
    );
  });
});

// W2-PRV-13: human labels, one chip rule, a loopback hint for Llama Server's
// external host, and "1 option", not "1 options".
describe('DefaultProviderSetupForm — provider-appropriate copy', () => {
  const llama = {
    name: 'llamacpp',
    is_configured: true,
    provider_type: 'Builtin',
    metadata: {
      name: 'llamacpp',
      display_name: 'Llama Server',
      description: '',
      default_model: '',
      known_models: [],
      model_doc_link: '',
      config_keys: [
        { name: 'LLAMACPP_EXTERNAL_HOST', required: false, secret: false, default: null },
      ],
    },
  } as unknown as ProviderDetails;

  it('names Llama Server\u2019s external host and suggests this machine for it', async () => {
    render(<Harness provider={llama} />);
    const input = await screen.findByLabelText(/Llama Server External Host/);
    expect(input).toHaveAttribute('placeholder', 'http://127.0.0.1:8080');
    expect(screen.getByText('(LLAMACPP_EXTERNAL_HOST)')).toBeInTheDocument();
    expect(screen.queryByPlaceholderText('https://api.example.com')).toBeNull();
  });

  it('gives every field its env-var chip, the role-named ones too', async () => {
    const azure = {
      ...bedrock,
      name: 'versa_azure',
      metadata: {
        ...bedrock.metadata,
        name: 'versa_azure',
        config_keys: [{ name: 'VERSA_AZURE_API_KEY', required: true, secret: true, default: null }],
      },
    } as unknown as ProviderDetails;
    render(<Harness provider={azure} />);
    expect(await screen.findByText('(VERSA_AZURE_API_KEY)')).toBeInTheDocument();
  });

  it('counts one option as one option', async () => {
    const one = {
      ...bedrock,
      metadata: {
        ...bedrock.metadata,
        config_keys: [
          bedrock.metadata.config_keys[0],
          { name: 'AWS_REGION', required: false, secret: false, default: 'us-west-2' },
        ],
      },
    } as unknown as ProviderDetails;
    render(<Harness provider={one} />);
    expect(await screen.findByText(/Show 1 option\b/)).toBeInTheDocument();
    expect(screen.queryByText(/1 options/)).toBeNull();
  });
});

// W2-PRV-13: a built-in declarative provider (DeepSeek, Groq, Mistral...) opens
// in the custom-provider form with its definition filled in whether or not a key
// was ever saved, and the form read that as "a key is saved".
describe('CustomProviderForm for a provider with no key saved', () => {
  const definition = {
    engine: 'openai',
    display_name: 'DeepSeek',
    api_url: 'https://api.deepseek.com',
    api_key: '',
    models: ['deepseek-chat'],
    supports_streaming: true,
  };

  it('asks for the key, and refuses to save without one', () => {
    const onSubmit = vi.fn();
    const { container } = render(
      <CustomProviderForm
        onSubmit={onSubmit}
        onCancel={vi.fn()}
        initialData={definition}
        isEditable={false}
        hasSavedKey={false}
      />
    );
    const input = container.querySelector('#api-key') as HTMLInputElement;
    expect(input).toHaveAttribute('placeholder', 'Your API key');
    expect(screen.queryByText(/keep existing key/)).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(onSubmit).not.toHaveBeenCalled();
    expect(screen.getByText('API key is required')).toBeInTheDocument();
  });

  it('still keeps a saved key when one is saved', () => {
    const onSubmit = vi.fn();
    const { container } = render(
      <CustomProviderForm
        onSubmit={onSubmit}
        onCancel={vi.fn()}
        initialData={definition}
        isEditable={false}
        hasSavedKey
      />
    );
    const input = container.querySelector('#api-key') as HTMLInputElement;
    expect(input).toHaveAttribute('placeholder', 'Leave blank to keep existing key');
    fireEvent.click(screen.getByRole('button', { name: 'Update Provider' }));
    expect(onSubmit).toHaveBeenCalled();
  });
});
