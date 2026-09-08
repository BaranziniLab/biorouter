import { describe, expect, it } from 'vitest';
import type { ProviderDetails, ProviderTier } from '../../../api';
import {
  AI_AGENT_PROVIDER_IDS,
  getOrderedProviderGroups,
  institutionsForProvider,
} from './providerOrdering';
import { AGENT_COMMAND_CONFIG } from '../../onboarding/codingAgentStatus';

/**
 * `tier` and `runs_locally` are what the daemon sends for this provider — the
 * grouping is derived from them, so the fixtures state them rather than letting
 * the renderer recognise a name.
 */
function provider(
  name: string,
  backend: {
    tier?: ProviderTier;
    runs_locally?: boolean;
    institutions?: { id: string; display_name?: string | null }[];
    affiliation?: ProviderDetails['affiliation'];
    resolved_tier?: ProviderTier | null;
  } = {},
  displayName = name
): ProviderDetails {
  return {
    name,
    is_configured: true,
    provider_type: 'Builtin',
    affiliation: backend.affiliation,
    resolved_tier: backend.resolved_tier ?? null,
    metadata: {
      config_keys: [],
      default_model: '',
      description: '',
      display_name: displayName,
      known_models: [],
      model_doc_link: '',
      name,
      tier: backend.tier ?? 'public',
      runs_locally: backend.runs_locally ?? false,
      institutions: backend.institutions ?? [],
    },
  } as ProviderDetails;
}

const PRIVATE_LOCAL = { tier: 'private', runs_locally: true } as const;
const PRIVATE_REMOTE = { tier: 'private', runs_locally: false } as const;
const UCSF = [{ id: 'ucsf', display_name: 'UCSF' }];
const names = (rows: ProviderDetails[]) => rows.map((row) => row.name);

describe('getOrderedProviderGroups', () => {
  it('groups providers into the three tabs, local first', () => {
    const groups = getOrderedProviderGroups([
      provider('openai'),
      provider('ollama', PRIVATE_LOCAL),
      provider('versa_bedrock', { ...PRIVATE_REMOTE, institutions: UCSF }),
      provider('llamacpp', PRIVATE_LOCAL),
      provider('anthropic'),
      provider('versa_azure', { ...PRIVATE_REMOTE, institutions: UCSF }),
    ]);

    expect(groups.map((group) => group.key)).toEqual(['local', 'institutional', 'commercial']);
    expect(groups.map((group) => group.tabLabel)).toEqual(['Local', 'Institutional', 'Public']);
    expect(names(groups[0]!.providers)).toEqual(['llamacpp', 'ollama']);
    expect(names(groups[1]!.providers)).toEqual(['versa_azure', 'versa_bedrock']);
    expect(names(groups[2]!.providers)).toEqual(['anthropic', 'openai']);
  });

  it('ranks Llama Server before Ollama within local models', () => {
    const groups = getOrderedProviderGroups([
      provider('ollama', PRIVATE_LOCAL),
      provider('llamacpp', PRIVATE_LOCAL),
    ]);
    expect(groups[0]?.key).toBe('local');
    expect(names(groups[0]!.providers)).toEqual(['llamacpp', 'ollama']);
  });

  it('demotes a private provider to commercial when the daemon says it is public', () => {
    // The real shape this covers: a custom_providers/*.json named `ollama`
    // shadows the built-in registry entry (registration is a plain insert by
    // config.name, after the built-ins), and the declarative path defaults the
    // tier to public rather than inheriting the engine's. The renderer must
    // follow the backend rather than recognising the name, or a provider that
    // is not the real ollama keeps a "Private · Local" badge it never earned.
    const groups = getOrderedProviderGroups([provider('ollama', { tier: 'public' })]);
    expect(groups[0]?.providers).toEqual([]);
    expect(names(groups[2]!.providers)).toEqual(['ollama']);
  });

  it('hides nothing: every provider the daemon serves reaches a group', () => {
    // This replaced a hide-list test for `codex` / `cursor-agent` / `claude-code`.
    // Two of those names are back — the daemon serves `claude_code` and `codex`
    // deliberately now — and the assertion is written this way precisely so that
    // it keeps holding: what must not come back is a renderer-side list of
    // names deciding *whether* a provider is shown.
    const groups = getOrderedProviderGroups([
      provider('openai'),
      provider('ollama', PRIVATE_LOCAL),
      provider('versa_azure', { ...PRIVATE_REMOTE, institutions: UCSF }),
    ]);

    expect(groups.flatMap((group) => names(group.providers)).sort()).toEqual([
      'ollama',
      'openai',
      'versa_azure',
    ]);
  });
});

describe('the public tab: AI agents first, then pinned, then alphabetical', () => {
  it('gives the coding agents their own section, ahead of the API providers', () => {
    const groups = getOrderedProviderGroups([
      provider('openai', {}, 'OpenAI'),
      provider('codex', {}, 'Codex'),
      provider('anthropic', {}, 'Anthropic'),
      provider('claude_code', {}, 'Claude Code'),
    ]);

    const commercial = groups[2]!;
    expect(commercial.sections.map((section) => section.key)).toEqual(['agents', 'api']);
    expect(names(commercial.sections[0]!.providers)).toEqual(['claude_code', 'codex']);
    expect(names(commercial.sections[1]!.providers)).toEqual(['anthropic', 'openai']);
    // The flattened view follows the sections, so a consumer that ignores
    // sections still sees the agents first.
    expect(names(commercial.providers)).toEqual(['claude_code', 'codex', 'anthropic', 'openai']);
  });

  it('omits the agents section entirely when the daemon serves no agent providers', () => {
    const groups = getOrderedProviderGroups([provider('openai')]);
    expect(groups[2]!.sections.map((section) => section.key)).toEqual(['api']);
  });

  /**
   * ⚠ The set of agent ids is imported from `codingAgentStatus.ts`. Asserted
   * against that module's own config map, so adding a third agent there and
   * forgetting this catalog fails here rather than shipping a provider with a
   * status pill and no section.
   */
  it('takes the agent ids from the one module that defines them', () => {
    expect([...AI_AGENT_PROVIDER_IDS].sort()).toEqual(Object.keys(AGENT_COMMAND_CONFIG).sort());
  });

  it('pins Anthropic, OpenAI and Google ahead of an alphabetical tail', () => {
    const groups = getOrderedProviderGroups([
      provider('openrouter', {}, 'OpenRouter'),
      provider('google', {}, 'Google Gemini'),
      provider('databricks', {}, 'Databricks'),
      provider('openai', {}, 'OpenAI'),
      provider('anthropic', {}, 'Anthropic'),
      provider('zai', {}, 'z.ai'),
    ]);
    expect(names(groups[2]!.sections[0]!.providers)).toEqual([
      'anthropic',
      'openai',
      'google',
      'databricks',
      'openrouter',
      'zai',
    ]);
  });

  /**
   * ⚠ **By DISPLAY name, not by id, and custom providers are in the same sort.**
   * `zai` next to `openrouter` sorts nowhere near "z.ai" next to "OpenRouter" on
   * screen, and a list whose order the eye cannot follow is what this replaced.
   */
  it('sorts the tail by what the row prints, custom providers included', () => {
    const groups = getOrderedProviderGroups([
      provider('zzz_internal', {}, 'Aardvark AI'),
      provider('custom_deepseek', {}, 'DeepSeek'),
      provider('aaa_vendor', {}, 'Zebra Labs'),
    ]);
    expect(names(groups[2]!.sections[0]!.providers)).toEqual([
      'zzz_internal',
      'custom_deepseek',
      'aaa_vendor',
    ]);
  });
});

describe('the institutional tab: grouped by the daemon-supplied institution', () => {
  it('heads a group with the display name from the payload, never a literal', () => {
    const groups = getOrderedProviderGroups([
      provider('versa_azure', { ...PRIVATE_REMOTE, institutions: UCSF }, 'Versa API Azure'),
      provider('versa_bedrock', { ...PRIVATE_REMOTE, institutions: UCSF }, 'Versa API Bedrock'),
    ]);
    const sections = groups[1]!.sections;
    expect(sections).toHaveLength(1);
    expect(sections[0]!.key).toBe('ucsf');
    expect(sections[0]!.label).toBe('UCSF');
    // The sub-line names what the institution hosts, built from the providers'
    // own display names rather than from a sentence written here.
    expect(sections[0]!.note).toBe('Versa API Azure · Versa API Bedrock');
    expect(names(sections[0]!.providers)).toEqual(['versa_azure', 'versa_bedrock']);
  });

  it('lists a provider under every institution that covers it', () => {
    const groups = getOrderedProviderGroups([
      provider('shared_gateway', {
        ...PRIVATE_REMOTE,
        resolved_tier: 'private',
        affiliation: {
          kind: 'institutions',
          institutions: [
            { id: 'ucsf', display_name: 'UCSF' },
            { id: 'stanford', display_name: 'Stanford' },
          ],
        },
      }),
    ]);
    const sections = groups[1]!.sections;
    // Alphabetical by the institution's own label.
    expect(sections.map((section) => section.label)).toEqual(['Stanford', 'UCSF']);
    for (const section of sections) expect(names(section.providers)).toEqual(['shared_gateway']);
  });

  it('falls back to a named group rather than hiding an unaffiliated gateway', () => {
    const groups = getOrderedProviderGroups([provider('mystery_gateway', PRIVATE_REMOTE)]);
    const sections = groups[1]!.sections;
    expect(sections.map((section) => section.key)).toEqual(['unaffiliated']);
    expect(sections[0]!.label).toBe('Unaffiliated private gateways');
  });

  it('puts the unaffiliated group last, after every named institution', () => {
    const groups = getOrderedProviderGroups([
      provider('mystery_gateway', PRIVATE_REMOTE),
      provider('versa_azure', { ...PRIVATE_REMOTE, institutions: UCSF }),
    ]);
    expect(groups[1]!.sections.map((section) => section.key)).toEqual(['ucsf', 'unaffiliated']);
  });

  it('keeps the bare id when the registry publishes no display name', () => {
    const groups = getOrderedProviderGroups([
      provider('gw', { ...PRIVATE_REMOTE, institutions: [{ id: 'not-published' }] }),
    ]);
    expect(groups[1]!.sections[0]!.label).toBe('not-published');
  });
});

describe('institutionsForProvider — instance answer first, shipped claim only as a fallback', () => {
  it('reads the shipped metadata for a row the daemon did not resolve', () => {
    // `resolved_tier: null` is how `GET /config/providers` reports "no instance
    // was built" — which is every provider on a machine where nothing is
    // configured, i.e. first-run onboarding.
    expect(
      institutionsForProvider(provider('versa_azure', { ...PRIVATE_REMOTE, institutions: UCSF }))
    ).toEqual(UCSF);
  });

  it('prefers the instance-resolved affiliation when the daemon sent one', () => {
    expect(
      institutionsForProvider(
        provider('versa_azure', {
          ...PRIVATE_REMOTE,
          institutions: UCSF,
          resolved_tier: 'private',
          affiliation: {
            kind: 'institutions',
            institutions: [{ id: 'elsewhere', display_name: 'Elsewhere' }],
          },
        })
      )
    ).toEqual([{ id: 'elsewhere', display_name: 'Elsewhere' }]);
  });

  /**
   * ⚠ **The case the precedence exists for.** A Versa module repointed off the
   * UCSF gateway loses Private *and* `ucsf` together in the daemon; a catalog
   * that read the shipped metadata in preference would keep printing the
   * institution's name over a gateway that is no longer theirs.
   */
  it('drops the shipped claim once an instance resolved without one', () => {
    expect(
      institutionsForProvider(
        provider('versa_azure', {
          ...PRIVATE_REMOTE,
          institutions: UCSF,
          resolved_tier: 'public',
        })
      )
    ).toEqual([]);
  });

  it('is empty for a provider that ships pointed at no institution', () => {
    expect(institutionsForProvider(provider('anthropic'))).toEqual([]);
  });
});
