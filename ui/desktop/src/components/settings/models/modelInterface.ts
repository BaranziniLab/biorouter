import type { ModelInfo, ProviderDetails, ProviderMetadata } from '../../../api';

export default interface Model {
  id?: number; // Make `id` optional to allow user-defined models
  name: string;
  provider: string;
  lastUsed?: string;
  alias?: string; // optional model display name
  subtext?: string; // goes below model name if not the provider
  context_limit?: number; // optional context limit override
  request_params?: Record<string, unknown>; // provider-specific request parameters
}

export function createModelStruct(
  modelName: string,
  provider: string,
  id?: number, // Make `id` optional to allow user-defined models
  lastUsed?: string,
  alias?: string, // optional model display name
  subtext?: string
): Model {
  // use the metadata to create a Model
  return {
    name: modelName,
    provider: provider,
    alias: alias,
    id: id,
    lastUsed: lastUsed,
    subtext: subtext,
  };
}

export async function getProviderMetadata(
  providerName: string,
  getProvidersFunc: (b: boolean) => Promise<ProviderDetails[]>
) {
  const providers = await getProvidersFunc(false);
  const matches = providers.find((providerMatch) => providerMatch.name === providerName);
  if (!matches) {
    throw Error(`No match for provider: ${providerName}`);
  }
  return matches.metadata;
}

function normalizedModelCandidates(modelName: string): Set<string> {
  const base = modelName
    .toLowerCase()
    .trim()
    .replace(/^([^/\s]+\/)/, '')
    .replace(/^(?:us|eu|apac)\.anthropic\./, '')
    .replace(/^anthropic\./, '')
    .replace(/^databricks-/, '')
    .replace(/@(\d{8})$/, '-$1')
    .replace(/-v\d+(?::\d+)?$/, '');

  const candidates = new Set<string>([base]);
  candidates.add(base.replace(/\./g, '-'));

  const numericDot = base.replace(/-(\d+)-(\d+)(?=-|$)/g, '-$1.$2');
  candidates.add(numericDot);

  const numericHyphen = base.replace(/-(\d+)\.(\d+)(?=-|$)/g, '-$1-$2');
  candidates.add(numericHyphen);

  for (const candidate of [...candidates]) {
    candidates.add(candidate.replace(/-v\d+(?::\d+)?$/, ''));
  }

  return candidates;
}

const DEFAULT_IMAGE_INPUT_MIME_TYPES = [
  'image/png',
  'image/jpeg',
  'image/jpg',
  'image/gif',
  'image/webp',
];

export function resolveKnownModelInfo(
  metadata: ProviderMetadata,
  modelName: string
): ModelInfo | undefined {
  const normalizedName = modelName.toLowerCase().trim();
  const exact = metadata.known_models.find(
    (model) => model.name.toLowerCase().trim() === normalizedName
  );
  if (exact) return exact;

  if (normalizedName.includes('codex')) return undefined;

  const activeCandidates = normalizedModelCandidates(normalizedName);

  return [...metadata.known_models]
    .sort((a, b) => b.name.length - a.name.length)
    .find((model) => {
      const knownCandidates = normalizedModelCandidates(model.name);
      for (const active of activeCandidates) {
        for (const known of knownCandidates) {
          if (
            active === known ||
            active.startsWith(`${known}-`) ||
            active.startsWith(`${known}.`) ||
            active.startsWith(`${known}@`)
          ) {
            return true;
          }
        }
      }
      return false;
    });
}

export function modelSupportsVision(metadata: ProviderMetadata, modelName: string): boolean {
  return resolveKnownModelInfo(metadata, modelName)?.supports_vision === true;
}

export function modelSupportedInputMimeTypes(
  metadata: ProviderMetadata,
  modelName: string
): string[] | null {
  const modelInfo = resolveKnownModelInfo(metadata, modelName);
  if (!modelInfo) return null;
  if (Array.isArray(modelInfo.supported_input_mime_types)) {
    return modelInfo.supported_input_mime_types;
  }
  return modelInfo.supports_vision === true ? DEFAULT_IMAGE_INPUT_MIME_TYPES : null;
}

export interface ProviderModelsResult {
  provider: ProviderDetails;
  models: string[] | null;
  error: string | null;
}

/**
 * Fetches models for all active providers in parallel.
 * Curated catalogues keep unsupported live models out and put their available
 * declared default first. Live lists retain the provider's own priority order.
 * Coding-agent account catalogues take precedence over bundled fallback models.
 */
export async function fetchModelsForProviders(
  activeProviders: ProviderDetails[],
  getProviderModelsFunc: (providerName: string) => Promise<string[]>
): Promise<ProviderModelsResult[]> {
  const modelPromises = activeProviders.map(async (p) => {
    const providerName = p.name;
    const knownModels = p.metadata.known_models?.map((m) => m.name) || [];
    const defaultIndex = knownModels.indexOf(p.metadata.default_model);
    const declaredModels =
      defaultIndex > 0
        ? [
            knownModels[defaultIndex],
            ...knownModels.slice(0, defaultIndex),
            ...knownModels.slice(defaultIndex + 1),
          ]
        : knownModels;
    const usesAccountCatalogue = providerName === 'codex' || providerName === 'github_copilot';

    // When a curated known_models list exists, use it exclusively so only
    // tested/supported models are shown (live API includes old deprecated models).
    if (declaredModels.length > 0 && !usesAccountCatalogue) {
      return { provider: p, models: declaredModels, error: null };
    }

    try {
      const models = await getProviderModelsFunc(providerName);
      if (providerName === 'codex' && !models?.length) {
        return { provider: p, models: declaredModels, error: null };
      }
      return { provider: p, models: models || [], error: null };
    } catch (e: unknown) {
      if (providerName === 'github_copilot') {
        // Capability metadata also serves saved models; it is not an account catalogue.
        return { provider: p, models: [p.metadata.default_model], error: null };
      }
      if (usesAccountCatalogue && declaredModels.length > 0) {
        return { provider: p, models: declaredModels, error: null };
      }
      const errorMessage = `Failed to fetch models for ${providerName}${e instanceof Error ? `: ${e.message}` : ''}`;
      return {
        provider: p,
        models: null,
        error: errorMessage,
      };
    }
  });

  return await Promise.all(modelPromises);
}
