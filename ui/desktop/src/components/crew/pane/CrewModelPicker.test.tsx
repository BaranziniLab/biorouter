import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProviderDetails } from '../../../api';
import { agentCopy } from './copy';
import { CrewModelPicker } from './CrewModelPicker';
import type { ModelChoice } from './useConfiguredModels';

const mocks = vi.hoisted(() => ({ getProviderModels: vi.fn() }));

vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({ getProviderModels: mocks.getProviderModels }),
  usePrivacyTiersEnabled: () => true,
}));

const versa = {
  name: 'versa_azure',
  is_configured: true,
  resolved_tier: 'private',
  affiliation: { kind: 'institutions', institutions: [{ id: 'ucsf', display_name: 'UCSF' }] },
  metadata: {
    display_name: 'Versa',
    known_models: [{ name: 'gpt-5.5' }, { name: 'gpt-5.5-mini' }],
  },
} as unknown as ProviderDetails;
const ollama = {
  name: 'ollama',
  is_configured: true,
  resolved_tier: 'private',
  affiliation: { kind: 'local', institutions: [] },
  metadata: { display_name: 'Ollama', known_models: [] },
} as unknown as ProviderDetails;
const openRouter = {
  name: 'openrouter',
  is_configured: true,
  resolved_tier: 'public',
  metadata: { display_name: 'OpenRouter', known_models: [{ name: 'free-model' }] },
} as unknown as ProviderDetails;

function Picker({
  providers = [versa, ollama, openRouter],
  initial = null,
  onChange = () => undefined,
  unavailableReason,
}: {
  providers?: ProviderDetails[] | null;
  initial?: ModelChoice | null;
  onChange?: (choice: ModelChoice) => void;
  unavailableReason?: (provider: ProviderDetails) => string | null;
}) {
  const [value, setValue] = useState<ModelChoice | null>(initial);
  return (
    <CrewModelPicker
      providers={providers}
      provider={value?.provider ?? ''}
      model={value?.model ?? ''}
      unavailableReason={unavailableReason}
      onChange={(choice) => {
        setValue(choice);
        onChange(choice);
      }}
    />
  );
}

const trigger = () => screen.getByRole('button', { name: /^Model/ });

beforeEach(() => {
  vi.clearAllMocks();
  mocks.getProviderModels.mockImplementation(async (name: string) =>
    name === 'ollama' ? ['qwen3.6', 'gemma4'] : []
  );
});

afterEach(() => {
  delete (window as unknown as { appConfig?: unknown }).appConfig;
});

describe('CrewModelPicker', () => {
  it('reads as a field named Model, with "Choose a model" until one is chosen', () => {
    render(<Picker />);
    expect(trigger()).toHaveAccessibleName(`Model ${agentCopy.modelEmpty}`);
    expect(trigger()).toHaveAttribute('aria-haspopup', 'dialog');
  });

  it('groups models by configured provider under headings with their privacy marks', async () => {
    const user = userEvent.setup();
    render(<Picker />);
    await user.click(trigger());
    const list = await screen.findByRole('listbox', { name: agentCopy.modelsLabel });
    const versaGroup = await within(list).findByRole('group', { name: /Versa/ });
    expect(
      within(versaGroup)
        .getAllByRole('option')
        .map((option) => option.textContent)
    ).toEqual(['gpt-5.5', 'gpt-5.5-mini']);
    // One mark with words, "Private · UCSF", never two bare glyphs or the chat badge's
    // "Private chat" (Q2-67).
    expect(versaGroup).toHaveAccessibleName(/Private/);
    const versaMark = within(versaGroup).getByTitle(agentCopy.privateModel);
    expect(versaMark).toHaveTextContent('Private · UCSF');
    expect(within(versaGroup).queryByText(/Private chat/)).toBeNull();
    expect(within(versaGroup).queryByTestId('affiliation-badge')).toBeNull();
    const ollamaGroup = within(list).getByRole('group', { name: /Ollama/ });
    expect(within(ollamaGroup).getAllByRole('option')).toHaveLength(2);
    expect(within(ollamaGroup).getByTitle(agentCopy.privateModel)).toHaveTextContent(
      'Private · On this machine'
    );
    // A curated list is used as is; only a provider without one is asked.
    expect(mocks.getProviderModels).toHaveBeenCalledWith('ollama');
    expect(mocks.getProviderModels).not.toHaveBeenCalledWith('versa_azure');
    const openRouterGroup = within(list).getByRole('group', { name: /OpenRouter/ });
    expect(openRouterGroup).not.toHaveAccessibleName(/Private/);
    expect(within(openRouterGroup).getByTitle(agentCopy.publicModel)).toHaveTextContent('Public');
  });

  it('says only private models can run the task where its context is protected (Q2-67)', async () => {
    const user = userEvent.setup();
    render(
      <CrewModelPicker
        providers={[versa, openRouter]}
        provider="versa_azure"
        model="gpt-5.5"
        privateOnly
        onChange={() => undefined}
      />
    );
    const onTrigger = within(trigger()).getByTitle(agentCopy.privateOnly);
    expect(onTrigger).toHaveTextContent('Private · UCSF');
    expect(agentCopy.privateOnly).toBe('Private. Only private models can run this task.');
    // The mark is not part of the field's name, which stays "Model {choice}".
    expect(trigger()).toHaveAccessibleName('Model gpt-5.5 · Versa');
    await user.click(trigger());
    const list = await screen.findByRole('listbox', { name: agentCopy.modelsLabel });
    const versaGroup = await within(list).findByRole('group', { name: /Versa/ });
    expect(within(versaGroup).getByTitle(agentCopy.privateOnly)).toBeInTheDocument();
    // A public model's mark says what it cannot read, not that it is private.
    expect(
      within(within(list).getByRole('group', { name: /OpenRouter/ })).getByTitle(
        agentCopy.publicModel
      )
    ).toBeInTheDocument();
  });

  it('chooses a model and names it on the trigger', async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<Picker onChange={onChange} />);
    await user.click(trigger());
    await user.click(await screen.findByRole('option', { name: /^qwen3\.6/ }));
    expect(onChange).toHaveBeenCalledWith({ provider: 'ollama', model: 'qwen3.6' });
    expect(trigger()).toHaveAccessibleName('Model qwen3.6 · Ollama');
    expect(screen.queryByRole('listbox')).toBeNull();
  });

  it('filters by model or provider and offers the typed name with each provider', async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<Picker onChange={onChange} />);
    await user.click(trigger());
    const search = await screen.findByRole('combobox', { name: agentCopy.searchModels });
    await user.type(search, 'mini');
    const list = screen.getByRole('listbox');
    expect(
      within(list)
        .getAllByRole('option')
        .map((option) => option.textContent)
    ).toEqual([
      'gpt-5.5-mini',
      agentCopy.modelUse('mini', 'Versa'),
      agentCopy.modelUse('mini', 'Ollama'),
      agentCopy.modelUse('mini', 'OpenRouter'),
    ]);

    await user.clear(search);
    await user.type(search, 'llama3.3');
    await user.click(
      screen.getByRole('option', { name: agentCopy.modelUse('llama3.3', 'Ollama') })
    );
    expect(onChange).toHaveBeenCalledWith({ provider: 'ollama', model: 'llama3.3' });
  });

  it('does not offer free text for a name that is already listed', async () => {
    const user = userEvent.setup();
    render(<Picker />);
    await user.click(trigger());
    await user.type(await screen.findByRole('combobox'), 'GPT-5.5-MINI');
    expect(
      screen.queryByRole('option', { name: agentCopy.modelUse('GPT-5.5-MINI', 'Versa') })
    ).toBeNull();
  });

  it('works with a provider row that carries no metadata', async () => {
    const user = userEvent.setup();
    mocks.getProviderModels.mockResolvedValue(['fixture-model']);
    render(
      <Picker
        providers={[
          { name: 'fixture-provider', is_configured: true } as unknown as ProviderDetails,
        ]}
      />
    );
    await user.click(trigger());
    await user.click(await screen.findByRole('option', { name: /^fixture-model/ }));
    expect(trigger()).toHaveAccessibleName('Model fixture-model · fixture-provider');
  });

  it('marks the models this workspace’s institution has not approved, and still lets them be chosen (T-47)', async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    const reason = agentCopy.notApproved('foreign-synthetic');
    render(
      <Picker
        onChange={onChange}
        unavailableReason={(provider) => (provider.name === 'versa_azure' ? reason : null)}
      />
    );
    await user.click(trigger());
    const list = await screen.findByRole('listbox', { name: agentCopy.modelsLabel });
    const versaGroup = await within(list).findByRole('group', { name: /Versa/ });
    expect(versaGroup).toHaveAccessibleName(new RegExp(reason));
    for (const option of within(versaGroup).getAllByRole('option')) {
      expect(option).toHaveTextContent(reason);
    }
    const ollamaGroup = within(list).getByRole('group', { name: /Ollama/ });
    expect(ollamaGroup).not.toHaveTextContent(reason);

    // The pane explains the choice and the daemon decides; the picker only marks it.
    await user.click(within(versaGroup).getByRole('option', { name: /^gpt-5\.5-mini/ }));
    expect(onChange).toHaveBeenCalledWith({ provider: 'versa_azure', model: 'gpt-5.5-mini' });
  });

  it('names the choice as the chat composer does, and wraps it rather than cutting it (T-47)', async () => {
    (window as unknown as { appConfig: unknown }).appConfig = {
      get: (key: string) =>
        key === 'BIOROUTER_PREDEFINED_MODELS'
          ? JSON.stringify([
              {
                id: 1,
                name: 'gpt-5.5-2026-04-24',
                provider: 'versa_azure',
                alias: 'GPT-5.5',
                subtext: 'Versa',
              },
            ])
          : undefined,
    };
    render(<Picker initial={{ provider: 'versa_azure', model: 'gpt-5.5-2026-04-24' }} />);
    expect(trigger()).toHaveAccessibleName('Model GPT-5.5 · Versa');
    const value = within(trigger()).getByText('GPT-5.5 · Versa');
    expect(value).not.toHaveClass('truncate');
    expect(trigger()).toHaveClass('min-h-control-md');
    expect(trigger()).not.toHaveClass('h-control-md');
  });

  it('says so while providers are loading', async () => {
    const user = userEvent.setup();
    render(<Picker providers={null} />);
    await user.click(trigger());
    expect(await screen.findByText(agentCopy.loadingModels)).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByRole('option')).toBeNull());
  });
});
