import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProviderDetails } from '../../../api';
import { IngestModelPicker } from './IngestModelPicker';
import { knowledgeCopy } from '../copy';

const mocks = vi.hoisted(() => ({
  config: { getProviders: vi.fn(), getProviderModels: vi.fn() },
}));

// One stable object: `useConfig()` handing back a fresh one on every call would
// change the picker's effect dependencies on every render. Same reasoning as
// `IngestPanel.test.tsx`, which is the other suite that renders this component.
vi.mock('../../ConfigContext', () => ({
  useConfig: () => mocks.config,
}));

/**
 * A configured provider with a curated model list.
 *
 * `known_models` is deliberately non-empty: `fetchModelsForProviders` short-
 * circuits to the curated list when there is one, so a fixture built this way
 * exercises the real model-loading path without a second mock standing in for
 * it. `tier: 'public'` puts every fixture in one group, which keeps the
 * assertions about *which providers appear* from doubling as assertions about
 * `providerOrdering.ts`'s grouping.
 */
function provider(name: string, displayName: string, models: string[]): ProviderDetails {
  return {
    is_configured: true,
    name,
    provider_type: 'Builtin',
    metadata: {
      name,
      display_name: displayName,
      description: '',
      default_model: models[0] ?? '',
      model_doc_link: '',
      config_keys: [],
      known_models: models.map((model) => ({ name: model, context_limit: 128_000 })),
      tier: 'public',
    },
  };
}

const OPENAI = provider('openai', 'OpenAI', ['gpt-5.5']);
const OLLAMA = provider('ollama', 'Ollama', ['qwen3.6:latest']);
const CLAUDE_CODE = provider('claude_code', 'Claude Code', ['opus-5']);
const CODEX = provider('codex', 'Codex', ['gpt-5.4-codex']);

function renderPicker() {
  return render(
    <MemoryRouter>
      <IngestModelPicker value={null} onChange={vi.fn()} />
    </MemoryRouter>
  );
}

/** Open the popover and wait for the model rows to have loaded into it. */
async function openPicker() {
  fireEvent.click(await screen.findByTestId('knowledge-model-picker-trigger'));
  return screen.findByRole('listbox', { name: 'Knowledge models' });
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.config.getProviderModels.mockResolvedValue([]);
});

describe('IngestModelPicker offers every configured provider (#109)', () => {
  it('lists the coding-agent providers alongside the rest', async () => {
    mocks.config.getProviders.mockResolvedValue([OPENAI, CLAUDE_CODE, CODEX, OLLAMA]);
    renderPicker();
    const list = await openPicker();

    // These two used to be filtered out here, because their
    // `complete_with_model` dropped the `tools` argument a macro passes and a
    // digest run against one narrated its calls and wrote nothing. #109 fixed
    // that at the source — a macro turn goes through `ProviderToolTurnContext`,
    // which issues the MCP bridge grant those providers need — so hiding them
    // would now hide a provider that works.
    await waitFor(() => expect(list).toHaveTextContent('OpenAI'));
    expect(list).toHaveTextContent('Claude Code');
    expect(list).toHaveTextContent('opus-5');
    expect(list).toHaveTextContent('Codex');
    expect(list).toHaveTextContent('gpt-5.4-codex');
    expect(list).toHaveTextContent('Ollama');
  });

  it('no longer explains an absence, because there is none', async () => {
    mocks.config.getProviders.mockResolvedValue([OPENAI, CLAUDE_CODE, CODEX]);
    renderPicker();
    const list = await openPicker();

    // The footer note named which providers had been left out and said they
    // still worked in chat. Nothing is left out now, so the note would be a
    // sentence about an absence that does not exist.
    await waitFor(() => expect(list).toHaveTextContent('gpt-5.5'));
    expect(screen.queryByTestId('knowledge-model-picker-excluded')).toBeNull();
  });

  it('still says the plain thing when nothing is configured at all', async () => {
    mocks.config.getProviders.mockResolvedValue([]);
    renderPicker();
    await openPicker();

    // The one verdict this popover may reach on its own: the user really has no
    // provider. It must survive the removal above — a rewrite that lost it
    // would leave a user with an empty setup staring at an empty list.
    expect(await screen.findByText('No models available')).toBeInTheDocument();
    expect(screen.getByText('Configure a provider in Settings.')).toBeInTheDocument();
    expect(screen.queryByTestId('knowledge-model-picker-excluded')).toBeNull();
  });
});

/**
 * Spec 3.12: the footer says what the pick is for in one line, with the rest in
 * an InfoTip. The "Set as default" chip it replaces looked like a button and did
 * nothing, because picking a row already sets the base's default.
 */
describe('IngestModelPicker footer', () => {
  it('says what the model is for in one line, with the rest as help', async () => {
    mocks.config.getProviders.mockResolvedValue([OPENAI]);
    renderPicker();
    const list = await openPicker();
    await waitFor(() => expect(list).toHaveTextContent('gpt-5.5'));

    expect(screen.getByText(knowledgeCopy.ingest.modelUse)).toBeVisible();
    expect(
      screen.getByRole('button', { name: `About ${knowledgeCopy.ingest.modelUseLabel}` })
    ).toHaveAccessibleDescription(knowledgeCopy.ingest.modelUseInfo);
    expect(screen.queryByText('Set as default')).toBeNull();
  });
});

/**
 * V3-03: the trigger shows the model's name and nothing else. The uppercase "MODEL" label took 45px
 * of a 268px field (the name truncated to "Versa API Azure / g…"), it was the only caps text on the
 * page that was not a section label, and the Brain glyph said the same thing again. The provider
 * and the release stamp ride in the Tooltip and the accessible name.
 */
describe('IngestModelPicker trigger', () => {
  const VERSA = provider('versa_azure', 'Versa API Azure', ['gpt-5.5-2026-04-24']);

  function renderWith(value: { provider: string; model: string } | null) {
    mocks.config.getProviders.mockResolvedValue([VERSA]);
    return render(
      <MemoryRouter>
        <IngestModelPicker value={value} onChange={vi.fn()} />
      </MemoryRouter>
    );
  }

  it("names the model as the composer's chip does, without a caps label or a glyph of its own", async () => {
    renderWith({ provider: 'versa_azure', model: 'gpt-5.5-2026-04-24' });
    const trigger = await screen.findByTestId('knowledge-model-picker-trigger');
    await waitFor(() =>
      expect(trigger).toHaveAccessibleName(
        knowledgeCopy.ingest.modelTriggerName('gpt-5.5-2026-04-24', 'Versa API Azure')
      )
    );

    expect(trigger).toHaveTextContent(/^gpt-5\.5$/);
    expect(trigger.querySelector('.text-caps')).toBeNull();
    // The chevron is the only glyph in it.
    expect(trigger.querySelectorAll('svg')).toHaveLength(1);
  });

  it('carries the provider and the full id in its Tooltip', async () => {
    const user = userEvent.setup();
    renderWith({ provider: 'versa_azure', model: 'gpt-5.5-2026-04-24' });
    const trigger = await screen.findByTestId('knowledge-model-picker-trigger');
    await waitFor(() => expect(trigger).toHaveAccessibleName(/Versa API Azure/));

    await user.hover(trigger);
    expect(await screen.findByRole('tooltip')).toHaveTextContent(
      knowledgeCopy.ingest.modelTriggerTip('gpt-5.5-2026-04-24', 'Versa API Azure')
    );
  });

  it('keeps the plain name, and no Tooltip, when there is no model to name', async () => {
    renderWith(null);
    const trigger = await screen.findByTestId('knowledge-model-picker-trigger');
    expect(trigger).toHaveAccessibleName(knowledgeCopy.ingest.modelName);
    expect(trigger).toHaveTextContent('No model configured');
  });
});
