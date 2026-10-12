import type { ProviderDetails } from '../../../api';
import { getModelDisplayName, getProviderDisplayName } from './predefinedModelsUtils';

/**
 * The Model row's value, "gpt-5.5-2026-04-24 · Versa API Azure", the words the row shows.
 *
 * Both halves are known without asking the daemon anything. The app already holds the model and
 * provider in force (`useModelAndProvider`), and the provider list (`useConfig().providersList`)
 * has been fetched by the time a person is in Settings: the first-run guard, the composer and
 * the privacy hooks all read it. So a tab switch back to Models can draw the value in its first
 * frame instead of a skeleton bar that the daemon's two config reads then replace 100 to 250 ms
 * later (vision V4-R7-1, principle 9: a surface mounted by a tab switch appears at rest).
 */
export const joinModelValue = (model: string, provider: string | null): string =>
  [model, provider].filter(Boolean).join(' · ');

/**
 * The value the row would show once the daemon had answered, or `null` when it cannot be said
 * from what the app already holds: no model in force yet, or a provider that is not in the list
 * (the list not fetched, or a name it does not know). `null` is "not known", never "empty": the
 * caller keeps its skeleton and lets the daemon's answer decide.
 *
 * This follows `getCurrentModelDisplayName` and `getCurrentProviderDisplayName` step for step
 * (a pinned model's alias and subtext first, then the provider's display name), so the value it
 * returns is the one they resolve to and the row never changes words when they land.
 */
export function knownDefaultModelValue(
  currentModel: string | null,
  currentProvider: string | null,
  providers: readonly ProviderDetails[] | undefined
): string | null {
  if (!currentModel || !currentProvider) return null;
  const model = getModelDisplayName(currentModel) || currentModel;
  const pinned = getProviderDisplayName(currentModel);
  if (pinned) return joinModelValue(model, pinned);
  const listed = providers?.find((candidate) => candidate.name === currentProvider);
  const displayName = listed?.metadata.display_name;
  return displayName ? joinModelValue(model, displayName) : null;
}
