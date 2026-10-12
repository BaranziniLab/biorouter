import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { useState } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProviderDetails } from '../../../../../../api';
import DefaultProviderSetupForm, {
  providerFieldName,
  type ConfigInput,
} from './DefaultProviderSetupForm';
import CustomProviderForm from './CustomProviderForm';
import { customProviderCopy, providerSetupCopy } from '../../../copy';
import { BROWSER_SURFACE_MARKER } from '../../../../../../utils/surface';

const mocks = vi.hoisted(() => ({ read: vi.fn() }));

vi.mock('../../../../../ConfigContext', () => ({
  useConfig: () => ({ read: mocks.read }),
}));

/**
 * F5 of the 2026-09-10 provider QA run: the Versa Bedrock card rendered its
 * Secret Access Key as `<input type="text">` with spellcheck on — on screen and
 * in the DOM as it was typed. The fixture is that card's real key set
 * (`versa_bedrock.rs`): two required secrets, two optional non-secrets with
 * defaults behind "Advanced".
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
const SECRET_ACCESS_KEY = 'Secret access key';
const ACCESS_KEY_ID = 'Access key ID';

beforeEach(() => {
  vi.clearAllMocks();
  // Nothing stored yet — the state the QA run typed into.
  mocks.read.mockResolvedValue(null);
});

// V4-R3-2: the labels were Title Case ("Anthropic API Key") beside the custom form's "API key", and
// the required error repeated it. One name per key, in sentence case, with acronyms and product
// names kept: it is the label, the placeholder fallback, the reveal toggle and the error.
describe('providerFieldName: a key named in sentence case', () => {
  it.each([
    ['anthropic', 'ANTHROPIC_API_KEY', 'Anthropic API key'],
    ['anthropic', 'ANTHROPIC_HOST', 'Anthropic host'],
    ['azure_openai', 'AZURE_OPENAI_DEPLOYMENT_NAME', 'Azure OpenAI deployment name'],
    ['llamacpp', 'LLAMACPP_CONTEXT_SIZE', 'Llama Server context size'],
    ['claude_code', 'CLAUDE_CODE_COMMAND', 'Claude CLI command'],
    ['zai', 'ZAI_API_KEY', 'z.ai API key'],
    ['xiaomi_mimo', 'XIAOMI_MIMO_HOST', 'Xiaomi MiMo host (region endpoint)'],
    ['gcp_vertex_ai', 'GCP_PROJECT_ID', 'GCP project ID'],
    // The role names, for a key with no curated label.
    ['tetrate', 'TETRATE_API_KEY', 'API key'],
    ['tetrate', 'TETRATE_HOST', 'API host'],
    // The prefix-stripped fallback, with ID kept as an acronym.
    ['versa_bedrock', 'VERSA_BEDROCK_ACCESS_KEY_ID', 'Access key ID'],
    ['versa_bedrock', 'VERSA_BEDROCK_SECRET_ACCESS_KEY', 'Secret access key'],
    ['litellm', 'LITELLM_CUSTOM_HEADERS', 'Custom headers'],
  ])('names %s %s as "%s"', (provider, key, name) => {
    expect(providerFieldName(provider, key)).toBe(name);
  });

  it('never leaves a plain later word capitalised', () => {
    for (const key of [
      'OPENAI_API_KEY',
      'OPENAI_BASE_PATH',
      'AZURE_OPENAI_API_VERSION',
      'LLAMACPP_ENABLE_THINKING',
      'SNOWFLAKE_TOKEN',
      'CODEX_COMMAND',
    ]) {
      const later = providerFieldName('x', key).split(' ').slice(1);
      for (const word of later) {
        if (/^(Server|OpenAI|Llama|Azure)$/.test(word)) continue;
        expect(word, key).not.toMatch(/^[A-Z][a-z]+$/);
      }
    }
  });
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
    fireEvent.click(await screen.findByRole('button', { name: providerSetupCopy.advanced }));

    expect(screen.getByDisplayValue('us-west-2')).toHaveAttribute('type', 'text');
    expect(screen.getByDisplayValue('https://unified-api.ucsf.edu/general/awsai')).toHaveAttribute(
      'type',
      'text'
    );
  });

  // ⚠ An EXACT label: the reveal toggle's accessible name is "Show Secret access key" and the
  // label's InfoTip is "About Secret access key", so a pattern would match all three.
  it('labels each field, so a screen reader names the masked one', async () => {
    render(<Harness />);
    const input = await screen.findByPlaceholderText(SECRET_ACCESS_KEY);
    expect(screen.getByLabelText('Secret access key')).toBe(input);
  });

  // The config key a field writes is machine text: it is the label's InfoTip, and the field is
  // described by it, so a screen reader hears it on reaching the field (principle 10).
  it('keeps the config key behind the label, reachable from the field', async () => {
    render(<Harness />);
    const input = await screen.findByPlaceholderText(SECRET_ACCESS_KEY);
    expect(input).toHaveAccessibleDescription('Saved as VERSA_BEDROCK_SECRET_ACCESS_KEY.');
    expect(screen.queryByText('(VERSA_BEDROCK_SECRET_ACCESS_KEY)')).toBeNull();
    expect(screen.getByRole('button', { name: 'About Secret access key' })).toBeInTheDocument();
  });

  it('stays masked while typing, and reveals only on an explicit toggle', async () => {
    render(<Harness />);
    const input = await screen.findByPlaceholderText(SECRET_ACCESS_KEY);

    fireEvent.change(input, { target: { value: 'dummy-not-a-real-secret' } });
    expect(input).toHaveAttribute('type', 'password');

    const reveal = screen.getByRole('button', { name: 'Show Secret access key' });
    expect(reveal).toHaveAttribute('aria-pressed', 'false');
    fireEvent.click(reveal);
    expect(input).toHaveAttribute('type', 'text');
    // Revealed is exactly when a spellchecker would otherwise see the value.
    expect(input).toHaveAttribute('spellcheck', 'false');

    fireEvent.click(screen.getByRole('button', { name: 'Hide Secret access key' }));
    expect(input).toHaveAttribute('type', 'password');
    expect(input).toHaveValue('dummy-not-a-real-secret');
  });
});

describe('the two provider forms mask a key the same way', () => {
  it('gives the custom-provider key the same masked field and the same toggle', () => {
    const { container } = render(
      <CustomProviderForm onSubmit={vi.fn()} onCancel={vi.fn()} initialData={null} />
    );
    const input = container.querySelector('#api-key') as HTMLInputElement;
    expect(input).toHaveAttribute('type', 'password');
    expect(input).toHaveAttribute('autocomplete', 'off');
    expect(input).toHaveAttribute('spellcheck', 'false');

    fireEvent.click(screen.getByRole('button', { name: 'Show API key' }));
    expect(input).toHaveAttribute('type', 'text');
  });
});

/**
 * Vision V4-R7-5: Configure Claude Code opened with its "Claude CLI command" field focused on
 * `claude`, and Chromium drew a red dotted misspelling line under it. Every parameter of a provider
 * is a machine string (a command, an endpoint, a region, a profile), so none is spellchecked,
 * auto-corrected or auto-capitalised, secret or not, above Advanced or inside it.
 */
const claudeCode = {
  name: 'claude_code',
  is_configured: false,
  provider_type: 'Builtin',
  metadata: {
    name: 'claude_code',
    display_name: 'Claude Code',
    description: '',
    default_model: '',
    known_models: [],
    model_doc_link: '',
    config_keys: [
      { name: 'CLAUDE_CODE_COMMAND', required: true, secret: false, default: 'claude' },
    ],
  },
} as unknown as ProviderDetails;

const MACHINE_ATTRIBUTES = { spellcheck: 'false', autocorrect: 'off', autocapitalize: 'off' };

function expectMachineString(input: HTMLElement) {
  for (const [attribute, value] of Object.entries(MACHINE_ATTRIBUTES)) {
    expect(input, `${input.id} ${attribute}`).toHaveAttribute(attribute, value);
  }
}

describe('DefaultProviderSetupForm: machine strings are not spellchecked (V4-R7-5)', () => {
  it("leaves the coding agent's command field alone", async () => {
    const { container } = render(<Harness provider={claudeCode} />);
    await screen.findByDisplayValue('claude');
    const command = container.querySelector<HTMLInputElement>(
      '#provider-config-CLAUDE_CODE_COMMAND'
    );
    expect(command).not.toBeNull();
    expect(command).toHaveValue('claude');
    expectMachineString(command!);
  });

  it('leaves every field of a provider alone, the ones behind Advanced and the secrets too', async () => {
    const { container } = render(<Harness />);
    fireEvent.click(await screen.findByRole('button', { name: providerSetupCopy.advanced }));

    const fields = Array.from(container.querySelectorAll<HTMLInputElement>('input'));
    // Two secrets, an endpoint and a region.
    expect(fields).toHaveLength(4);
    for (const field of fields) expectMachineString(field);
  });
});

describe('CustomProviderForm: machine strings are not spellchecked (V4-R7-5)', () => {
  it('leaves the name, the URL, the key and the model list alone', () => {
    const { container } = render(
      <CustomProviderForm onSubmit={vi.fn()} onCancel={vi.fn()} initialData={null} isEditable />
    );
    for (const id of ['display-name', 'api-url', 'api-key', 'available-models']) {
      const field = container.querySelector<HTMLInputElement>(`#${id}`);
      expect(field, id).not.toBeNull();
      expectMachineString(field!);
    }
  });

  it('keeps the same attributes on an edit', () => {
    const { container } = render(
      <CustomProviderForm
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        isEditable
        initialData={{
          engine: 'openai',
          display_name: 'My provider',
          api_url: 'https://api.example.com/v1',
          api_key: '',
          models: ['model-a'],
          supports_streaming: true,
        }}
      />
    );
    for (const id of ['display-name', 'api-url', 'available-models']) {
      expectMachineString(container.querySelector<HTMLInputElement>(`#${id}`)!);
    }
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

    const endpoint = await screen.findByLabelText('Azure OpenAI endpoint');
    expect(endpoint).toHaveValue('');
    expect(endpoint).toHaveAttribute('placeholder', 'https://<your-resource>.openai.azure.com');
  });

  it('fills in no value that points at the UCSF gateway', async () => {
    render(<Harness provider={azure} />);
    await screen.findByLabelText('Azure OpenAI endpoint');

    expect(screen.queryByDisplayValue(/unified-api\.ucsf\.edu/)).toBeNull();
  });

  // The control: a default that IS right for every user still arrives as a
  // value, so the case above is about this one key, not about defaults.
  it('still fills in the API version, which is the same for everyone', async () => {
    render(<Harness provider={azure} />);

    expect(await screen.findByLabelText('Azure OpenAI API version')).toHaveValue(
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
    const input = await screen.findByLabelText('Llama Server external host');
    expect(input).toHaveAttribute('placeholder', 'http://127.0.0.1:8080');
    expect(input).toHaveAccessibleDescription('Saved as LLAMACPP_EXTERNAL_HOST.');
    expect(screen.queryByPlaceholderText('https://api.example.com')).toBeNull();
  });

  it('gives every field its config key in the label’s InfoTip, the role-named ones too', async () => {
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
    expect(await screen.findByLabelText('API key')).toHaveAccessibleDescription(
      'Saved as VERSA_AZURE_API_KEY.'
    );
  });

  // Vision FR-R4-5: the first-run path's disclosures said "Details", "Advanced" and "Show 4
  // options"; every one now says "Advanced", with its hidden fields' labels joined by "·" as the
  // closed summary, which is also the trigger's description.
  it('keeps the optional fields behind Advanced, summarised by their labels', async () => {
    render(<Harness />);
    const trigger = await screen.findByRole('button', { name: providerSetupCopy.advanced });
    const summary = [
      providerFieldName('versa_bedrock', 'AWS_ENDPOINT_URL_BEDROCK'),
      providerFieldName('versa_bedrock', 'AWS_REGION'),
    ].join(' · ');
    expect(trigger).toHaveAccessibleDescription(summary);
    expect(screen.queryByText(/Show \d|Hide \d/)).toBeNull();

    fireEvent.click(trigger);
    // Open, the fields say it themselves.
    expect(trigger).toHaveAccessibleName(providerSetupCopy.advanced);
    expect(screen.queryByText(summary)).toBeNull();
  });

  it('summarises one optional field by its label alone', async () => {
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
    expect(
      await screen.findByRole('button', { name: providerSetupCopy.advanced })
    ).toHaveAccessibleDescription(providerFieldName('versa_bedrock', 'AWS_REGION'));
    expect(screen.queryByText(/·/)).toBeNull();
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
    fireEvent.click(screen.getByRole('button', { name: customProviderCopy.update }));
    expect(onSubmit).toHaveBeenCalled();
  });
});

/**
 * Vision FR-R3-3, on the same path: Add provider's "Create provider" was coral with nothing typed,
 * beside a Configure dialog whose Save is now secondary until its key is in. The submit follows the
 * one rule: secondary until every field it would ask for has a value, coral once it can act.
 */
describe('CustomProviderForm submit is coral only once it can act', () => {
  const coral = (button: HTMLElement) => button.classList.contains('bg-background-accent');

  it('stays secondary on an empty form and turns coral once every field is in', () => {
    const { container } = render(
      <CustomProviderForm onSubmit={vi.fn()} onCancel={vi.fn()} initialData={null} isEditable />
    );
    const create = screen.getByRole('button', { name: customProviderCopy.create });
    expect(coral(create)).toBe(false);
    expect(create).toHaveClass('br-fill-rest');
    const type = (selector: string, value: string) =>
      fireEvent.change(container.querySelector(selector) as HTMLInputElement, {
        target: { value },
      });
    type('#display-name', 'Lab gateway');
    type('#api-url', 'https://gateway.example.org/v1');
    type('#available-models', 'model-a');
    expect(coral(create), 'still no key').toBe(false);
    type('#api-key', 'sk-test');
    expect(coral(create)).toBe(true);
  });

  it('counts a local model as needing no key', () => {
    const { container } = render(
      <CustomProviderForm onSubmit={vi.fn()} onCancel={vi.fn()} initialData={null} isEditable />
    );
    for (const [selector, value] of [
      ['#display-name', 'Local'],
      ['#api-url', 'http://127.0.0.1:8080/v1'],
      ['#available-models', 'm'],
    ]) {
      fireEvent.change(container.querySelector(selector) as HTMLInputElement, {
        target: { value },
      });
    }
    fireEvent.click(container.querySelector('#local-model') as HTMLInputElement);
    expect(coral(screen.getByRole('button', { name: customProviderCopy.create }))).toBe(true);
  });

  it('draws an edit with a saved key coral, from its definition', () => {
    render(
      <CustomProviderForm
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        initialData={{
          engine: 'openai',
          display_name: 'DeepSeek',
          api_url: 'https://api.deepseek.com',
          api_key: '',
          models: ['deepseek-chat'],
          supports_streaming: true,
        }}
        isEditable
        hasSavedKey
      />
    );
    expect(coral(screen.getByRole('button', { name: customProviderCopy.update }))).toBe(true);
    expect(screen.getByDisplayValue('DeepSeek')).toBeInTheDocument();
  });
});

/**
 * W2-PRV-2, round 4. In a browser served by `biorouter serve`, a setting that
 * decides where a provider sends its requests and key cannot be changed: the
 * daemon asks for a proof of a person a browser can never send. The field is
 * shown, not editable, and the form says where it is changed, before a Save
 * meets the refusal.
 */
describe('DefaultProviderSetupForm in a browser', () => {
  const openai = {
    name: 'openai',
    is_configured: true,
    provider_type: 'Builtin',
    metadata: {
      name: 'openai',
      display_name: 'OpenAI',
      description: '',
      default_model: '',
      known_models: [],
      model_doc_link: '',
      config_keys: [
        { name: 'OPENAI_API_KEY', required: true, secret: true, default: null },
        { name: 'OPENAI_HOST', required: true, secret: false, default: 'https://api.openai.com' },
        { name: 'OPENAI_ORGANIZATION', required: true, secret: false, default: null },
      ],
    },
  } as unknown as ProviderDetails;

  afterEach(() => {
    delete document.documentElement.dataset.biorouterSurface;
  });

  it('shows the host and endpoint read-only, with the reason', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    const { container } = render(<Harness provider={openai} />);

    const host = await screen.findByDisplayValue('https://api.openai.com');
    expect(host).toBeDisabled();
    // The host-owned note first, then the label's own InfoTip (Field merges both).
    expect(host.getAttribute('aria-describedby')).toContain('provider-config-host-owned-note');
    const note = screen.getByTestId('host-managed-destination-note');
    expect(note.textContent).toContain('biorouter configure');
    // The key and a setting that goes nowhere stay editable.
    expect(container.querySelector('#provider-config-OPENAI_API_KEY')).toBeEnabled();
    expect(container.querySelector('#provider-config-OPENAI_ORGANIZATION')).toBeEnabled();
  });

  it('leaves every field editable in the desktop app', async () => {
    render(<Harness provider={openai} />);
    expect(await screen.findByDisplayValue('https://api.openai.com')).toBeEnabled();
    expect(screen.queryByTestId('host-managed-destination-note')).toBeNull();
  });
});

describe('CustomProviderForm in a browser', () => {
  const lab = {
    engine: 'openai',
    display_name: 'Lab gateway',
    api_url: 'https://lab.example/v1',
    api_key: '',
    models: ['lab-model'],
    supports_streaming: true,
  };

  afterEach(() => {
    delete document.documentElement.dataset.biorouterSurface;
  });

  function edit(onSubmit: () => void) {
    return render(
      <CustomProviderForm
        onSubmit={onSubmit}
        onCancel={vi.fn()}
        initialData={lab}
        isEditable
        hasSavedKey
      />
    );
  }

  it('asks for the key again before moving a saved key to a new URL', () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    const onSubmit = vi.fn();
    const { container } = edit(onSubmit);
    expect(screen.getByTestId('host-managed-custom-url')).toBeInTheDocument();

    fireEvent.change(container.querySelector('#api-url') as HTMLInputElement, {
      target: { value: 'https://elsewhere.example/v1' },
    });
    fireEvent.click(screen.getByRole('button', { name: customProviderCopy.update }));
    expect(onSubmit).not.toHaveBeenCalled();
    expect(screen.getByText(/Type the key again/)).toBeInTheDocument();

    fireEvent.change(container.querySelector('#api-key') as HTMLInputElement, {
      target: { value: 'sk-typed' },
    });
    fireEvent.click(screen.getByRole('button', { name: customProviderCopy.update }));
    expect(onSubmit).toHaveBeenCalled();
  });

  it('keeps the saved key for an edit that does not move the URL', () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    const onSubmit = vi.fn();
    edit(onSubmit);
    fireEvent.click(screen.getByRole('button', { name: customProviderCopy.update }));
    expect(onSubmit).toHaveBeenCalled();
  });

  it('moves the URL with the saved key in the desktop app', () => {
    const onSubmit = vi.fn();
    const { container } = edit(onSubmit);
    expect(screen.queryByTestId('host-managed-custom-url')).toBeNull();
    fireEvent.change(container.querySelector('#api-url') as HTMLInputElement, {
      target: { value: 'https://elsewhere.example/v1' },
    });
    fireEvent.click(screen.getByRole('button', { name: customProviderCopy.update }));
    expect(onSubmit).toHaveBeenCalled();
  });
});

/**
 * T3-SH-3. The daemon now refuses a declarative or custom provider's key when
 * the provider rejects it. The refusal used to be an unhandled rejection: the
 * form stayed as it was, with nothing said.
 */
describe('CustomProviderForm when the save is refused', () => {
  const groq = {
    engine: 'openai',
    display_name: 'Groq',
    api_url: 'https://api.groq.com/openai/v1/chat/completions',
    api_key: '',
    models: ['openai/gpt-oss-120b'],
    supports_streaming: true,
  };
  const REFUSAL = 'Groq rejected this key, so it was not saved: Invalid API Key';

  it("shows the daemon's sentence in the form and keeps what was typed", async () => {
    const onSubmit = vi.fn().mockRejectedValue(REFUSAL);
    const { container } = render(
      <CustomProviderForm
        onSubmit={onSubmit}
        onCancel={vi.fn()}
        initialData={groq}
        isEditable={false}
        hasSavedKey={false}
      />
    );
    const key = container.querySelector('#api-key') as HTMLInputElement;
    fireEvent.change(key, { target: { value: 'gsk_wrong' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    expect(await screen.findByRole('alert')).toHaveTextContent(REFUSAL);
    expect(onSubmit).toHaveBeenCalledWith(expect.objectContaining({ api_key: 'gsk_wrong' }));
    expect(key).toHaveValue('gsk_wrong');
  });

  it('says nothing when the save goes through', async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const { container } = render(
      <CustomProviderForm
        onSubmit={onSubmit}
        onCancel={vi.fn()}
        initialData={groq}
        isEditable={false}
        hasSavedKey={false}
      />
    );
    fireEvent.change(container.querySelector('#api-key') as HTMLInputElement, {
      target: { value: 'gsk_right' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalled());
    await waitFor(() => expect(screen.getByRole('button', { name: 'Save' })).toBeEnabled());
    expect(screen.queryByRole('alert')).toBeNull();
  });
});
