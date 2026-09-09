import { useEffect, useState } from 'react';
import { useConfig } from '../ConfigContext';
import { useModelAndProvider } from '../ModelAndProviderContext';
import { readResolvedProviderTier } from './useBoundProviderTier';
import type { PinnedModelView } from '../../hooks/chatStreamStore';
import type { ProviderTier, Session } from '../../api/types.gen';
import {
  bindingDiffersFromSelection,
  bindingLabel,
  chatBinding,
  pinnedModelNotice,
  selectionBarredByPrivacy,
} from './pinnedModel';

export interface PinnedModelPresentation {
  /**
   * What the composer's model chip and context gauge should state, or
   * `undefined` to keep stating the app-wide selection.
   */
  effectiveModel?: PinnedModelView;
  /**
   * The one line to show the user, or `null` when there is nothing to say —
   * nothing differs, or the difference is not the barrier's doing, or the facts
   * have not resolved yet.
   */
  notice: string | null;
}

/**
 * What this chat runs on, and whether that needs explaining.
 *
 * # Two rules, one per statement
 *
 * 1. **The chip and gauge state the chat's own binding whenever it differs from
 *    the app-wide selection** — for every chat, private or not. That is simply
 *    what runs: `restore_provider_from_session` binds the row's own
 *    `provider_name`/`model_config` whenever the row names them, and only falls
 *    back to the selection for a chat that names neither.
 * 2. **The sentence is reserved for the case the privacy barrier causes.**
 *    {@link selectionBarredByPrivacy} is the whole test.
 *
 * ⚠ Rule 1 is what PR #192 could not yet ship, and the reason is worth keeping:
 * `ModelAndProviderContext.changeModel` writes BOTH the session row
 * (`updateAgentProvider`) and the global default (`setConfigProvider`), so after
 * a per-chat model switch the daemon had them in step while this client's copy
 * of the row — cached in the `/agent/resume` payload the chat stream holds, and
 * refetched by nothing — was the value from BEFORE the switch. Preferring the
 * row on every disagreement then answered a switch by showing the model the
 * user had just switched away from. #192 narrowed the rule to the barred case,
 * where Gate A proves the selection cannot be what runs here.
 *
 * That staleness is now fixed at its source rather than routed around, which is
 * what lets rule 1 be the general rule:
 *
 * - `utils/sessionBindingSync` — `changeModel` announces the binding the daemon
 *   accepted, and the controller patches its row BEFORE the global selection
 *   moves. So in the only render where the two can disagree, the row already
 *   holds the new binding.
 * - `ChatStreamController.refreshSessionBinding` — after every turn the row's
 *   provider, model, classification and reason are re-read from
 *   `GET /sessions/{id}?metadata_only=true`. A turn is the only other thing
 *   that changes them, and it changes them in ways no client can compute.
 *
 * ⚠ **No sentence for the public case, deliberately.** A public chat on a
 * different public model needs the chip and the gauge to be right, and once they
 * are, the chip IS the statement — a standing banner above the composer would
 * appear in every chat older than the user's last model switch, which is most of
 * them, to say something the control beside it already says. The chip's own
 * dropdown carries the one line that the chip cannot ("a model chosen elsewhere
 * applies to new chats"), where a reader who wonders why the two disagree is
 * already looking. The privacy sentence stays because it answers a different
 * question — *why your choice is not in effect here* — which nothing else on
 * screen answers.
 *
 * ⚠ The provider **display names** are resolved through `getProviders`, which
 * `ConfigContext` caches. `versa_azure` is not what that provider is called
 * anywhere else in the app. The selected provider's tier comes off the same
 * fetch, and off the ROW rather than `row.metadata` — `metadata.tier` is the
 * type-level claim and `resolved_tier` the instance-resolved one (DR-26);
 * reading the type-level field would mis-classify an `ollama` re-pointed off
 * this machine, the exact demotion the tier exists to catch.
 */
export function usePinnedModel(
  session: Session | undefined,
  reportedByTurn: PinnedModelView | undefined
): PinnedModelPresentation {
  const { getProviders } = useConfig();
  const { currentModel, currentProvider } = useModelAndProvider();
  const [displayNames, setDisplayNames] = useState<Record<string, string>>({});
  const [selectedTier, setSelectedTier] = useState<ProviderTier | undefined>(undefined);

  const binding = chatBinding(session, reportedByTurn);
  const chatTier = session?.privacy_tier;
  const bindingProvider = binding?.provider;
  const selectedProvider = currentProvider ?? undefined;

  useEffect(() => {
    // ⚠ Nothing is fetched for a chat that cannot be barred, and that survives
    // rule 1: the catalog is read only for the SENTENCE — the selected
    // provider's tier, and the two display names the sentence names — and only a
    // PRIVATE chat can ever earn one. The chip resolves its own provider's
    // display name from the row it already reads (`ModelsBottomBar`), so a
    // public chat still costs no catalog read and no state update here.
    if (chatTier !== 'private' || !selectedProvider) {
      setSelectedTier(undefined);
      return;
    }
    const wanted = [bindingProvider, selectedProvider].filter((id): id is string => !!id);
    let cancelled = false;
    (async () => {
      try {
        const rows = await getProviders(false);
        if (cancelled) return;
        // The tier and both display names come from ONE pass over ONE fetch, so
        // they cannot be taken from different reads of the catalog.
        const resolved: Record<string, string> = {};
        for (const id of wanted) {
          const name = rows.find((row) => row.name === id)?.metadata?.display_name;
          if (name) resolved[id] = name;
        }
        setDisplayNames((prev) => {
          const unchanged = wanted.every((id) => prev[id] === resolved[id]);
          return unchanged ? prev : { ...prev, ...resolved };
        });
        const selectedRow = rows.find((row) => row.name === selectedProvider);
        setSelectedTier(selectedRow ? readResolvedProviderTier(selectedRow) : undefined);
      } catch {
        // A catalog we cannot read leaves the tier unresolved, and unresolved
        // means "say nothing and change nothing": both the note and the chip
        // override claim to know that the selection is not what runs here.
        if (!cancelled) setSelectedTier(undefined);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [chatTier, getProviders, bindingProvider, selectedProvider]);

  // A chat that has never named a provider genuinely has no binding of its own
  // and correctly runs on — and states — the app-wide selection.
  if (!binding) return { notice: null };

  // Rule 1. `bindingDiffersFromSelection` is also `false` on the first render of
  // every chat, where the config has not landed and `currentProvider` /
  // `currentModel` are still `null`. Overriding there would state something this
  // hook cannot yet know, and would do it as a flash.
  const differs = bindingDiffersFromSelection(binding, {
    provider: currentProvider,
    model: currentModel,
  });
  if (!differs) return { notice: null };

  // Rule 2. The chip is already stating the binding by this point; the sentence
  // is the separate claim that PRIVACY is why the user's choice is not in
  // effect, and only a public selection on a private chat earns it.
  const barred = selectionBarredByPrivacy(chatTier, selectedTier);

  return {
    effectiveModel: binding,
    notice: barred
      ? pinnedModelNotice(
          bindingLabel(displayNames[binding.provider], binding.model),
          bindingLabel(displayNames[selectedProvider!], currentModel!)
        )
      : null,
  };
}
