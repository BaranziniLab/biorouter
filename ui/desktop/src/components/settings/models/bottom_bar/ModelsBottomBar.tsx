import '../../../bottom_menu/pickers.css';
import { ChevronDown } from '../../../icons/app-icons';
import React, { useEffect, useRef, useState } from 'react';
import { useModelAndProvider } from '../../../ModelAndProviderContext';
import { NO_MODEL_CHIP_LABEL, hasNoModelConfigured } from '../../../composerNoProvider';
import { SwitchModelModal } from '../subcomponents/SwitchModelModal';
import { usePendingChatModel } from '../pendingChatModel';
import { View, type ViewOptions } from '../../../../utils/navigationUtils';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '../../../ui/dropdown-menu';
import { InfoTip, useInfoTipId } from '../../../ui/info-tip';
import { MODEL_COPY } from '../../../bottom_menu/copy';
import { friendlyModelName } from '../../../bottom_menu/modelLabel';
import { useReasoningEffortValue } from '../../../bottom_menu/BottomMenuReasoningEffort';
import { EffortSection, moveFocusAcrossMenuStops } from '../../../bottom_menu/EffortSection';
import {
  DEFAULT_REASONING_EFFORT,
  REASONING_EFFORT_LABELS,
} from '../../../../store/reasoningEffort';
import { useConfig } from '../../../ConfigContext';
import { getProviderMetadata } from '../modelInterface';
import { Alert } from '../../../alerts';
import BottomMenuAlertPopover from '../../../bottom_menu/BottomMenuAlertPopover';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../../ui/Tooltip';
import { useSurfaceTriggerHint } from '../../../ui/surfaceTriggerHint';
import { PrivacyBadge } from '../../../ui/PrivacyBadge';
import { AffiliationBadge } from '../../../ui/AffiliationBadge';
import {
  affiliationPresentation,
  readProviderAffiliation,
  type ProviderAffiliation,
} from '../../../privacy/providerAffiliation';
import { disclosureRequiredForTier, useDisclosure } from '../../../privacy/disclosureCopy';
import { readResolvedProviderTier } from '../../../privacy/useBoundProviderTier';
import { HostManagedModelNote } from '../../../privacy/HostManagedModelNote';
import { isDecisionProvider } from '../../../decisions/decisionWire';
import { decisionModelPlace, privacyBasisChipLine } from '../../../decisions/decisionWords';
import { decisionModelLabel, type PrivacyBasis } from '../../../../utils/sessionDecisions';
import { switchModelCopy } from '../copy';
import { isBrowserSurface } from '../../../../utils/surface';
import type {
  ProviderDetails,
  ProviderTier,
  SessionClassification,
} from '../../../../api/types.gen';
import type { PinnedModelView } from '../../../../hooks/chatStreamStore';

/**
 * Round 3 / N1 — the one line explaining why the chip may not name the model the
 * user last chose.
 *
 * Two plain facts and no instruction: what governs this chat, and what the
 * app-wide choice governs instead. It deliberately says nothing about privacy —
 * that is a different cause with its own sentence
 * (`privacy/pinnedModel.pinnedModelNotice`), and it is true of only some of the
 * chats this line appears in.
 */
export const CHAT_KEEPS_ITS_MODEL_NOTE =
  'This chat keeps the model it was last set to. A model chosen elsewhere applies to new chats.';

/**
 * F3 — the heading and line this chip's dropdown carries where there is no chat
 * yet (Home; an unsent chat is a chat, see `pendingChatModel.ts`).
 *
 * There the chip names the APP-WIDE selection — the pair `/agent/start` will
 * bind — and switching from it changes that pair for every window. "Current
 * model" read as a property of this screen; the heading says whose model it is,
 * and the line says how far a change reaches, beside the control that makes it.
 */
export const NEW_CHATS_MODEL_HEADING = 'Model for new chats';
export const NEW_CHATS_MODEL_NOTE =
  'New chats in every window start on this model. Existing chats keep their own.';

/**
 * V2-R8-1. Everything this chip states about a provider, as one catalog row
 * said it. One row of one read, so the tier, the affiliation, the disclosure
 * and the display name can never come from two different providers.
 */
interface ProviderSample {
  displayName: string | null;
  needsDisclosure: boolean;
  affiliation: ProviderAffiliation | null;
  tier: ProviderTier | undefined;
}

const NO_CATALOG_SAMPLES: ReadonlyMap<string, ProviderSample> = new Map();

/**
 * V2-R8-5. The chip's accessible name is several sentences: the model, the
 * chat's tier, the effort. Joined with ". ", a part that already ends its own
 * sentence ("…only lets a private model open it.") was read out with a double
 * period. Each part is ended with exactly one period, never a second, and the
 * parts themselves (the chat's tier sentence among them) are left as written.
 */
export function joinSentences(parts: ReadonlyArray<string | null | undefined | false>): string {
  return parts
    .filter((part): part is string => typeof part === 'string' && part.length > 0)
    .map((part, index, kept) =>
      index < kept.length - 1 && !/[.!?]$/.test(part) ? `${part}.` : part
    )
    .join(' ');
}

/**
 * A provider the catalog does not list, or a read that failed. Fail-safe on the
 * disclosure means saying more; on the affiliation and the tier it means saying
 * nothing, because each of those is a claim (whose agreements cover a
 * transcript; whether the model is private), and a catalog we cannot read is
 * evidence of neither.
 */
const UNVOUCHED_PROVIDER: ProviderSample = {
  displayName: null,
  needsDisclosure: true,
  affiliation: null,
  tier: undefined,
};

function sampleProviderRow(row: ProviderDetails): ProviderSample {
  return {
    displayName: row.metadata.display_name ?? null,
    needsDisclosure: disclosureRequiredForTier(row.metadata.tier),
    // Read off the ROW, never `row.metadata`: the metadata's tier is the
    // type-level claim, and affiliation is served beside it precisely because
    // DR-26 requires an instance-resolved value.
    affiliation: readProviderAffiliation(row),
    tier: readResolvedProviderTier(row),
  };
}

/** Every row of one catalog read, each under the provider it was read for. */
function sampleCatalog(
  rows: ReadonlyArray<ProviderDetails> | undefined
): ReadonlyMap<string, ProviderSample> {
  if (!rows || rows.length === 0) return NO_CATALOG_SAMPLES;
  return new Map(rows.map((row) => [row.name, sampleProviderRow(row)]));
}

interface ModelsBottomBarProps {
  sessionId: string | null;
  /** Legacy: nothing reads it. Kept optional so older callers still compile. */
  dropdownRef?: React.RefObject<HTMLDivElement>;
  setView: (view: View, options?: ViewOptions) => void;
  /** Only read when the inline alert popover is shown (`hideAlertPopover` false). */
  alerts?: Alert[];
  /** Hide the inline alert green-dot when the context window indicator is
   * surfaced separately (e.g. in the picker popover's dedicated row). */
  hideAlertPopover?: boolean;
  /**
   * The focused chat's privacy tier (issue #56, R10 / §14.2).
   *
   * ⚠ This is the SESSION's ratcheted classification, not the bound provider's
   * `metadata.tier`. `providerOrdering.ts` records why, and it is not a
   * preference: `GET /config/providers` serves the *type-level* tier, so an
   * `ollama` re-pointed off this machine by `OLLAMA_HOST` still arrives here
   * claiming `private` while its instance resolves `public`. A badge hung on
   * that field would read Private in exactly the demotion case the tier exists
   * to catch. The session classification is computed server-side from the
   * instance and only ever ratchets upward, so it can be asserted.
   *
   * `undefined` — a chat whose session has not loaded — renders nothing rather
   * than asserting Public, matching `SessionNamePill`.
   */
  privacyTier?: SessionClassification;
  /**
   * DR-33: why this public chat is public when a decision model made it so (the
   * daemon's `privacyBasis`, read by the chat). The chip's tip gains one line, and
   * the model switch says the chat stays public.
   */
  privacyBasis?: PrivacyBasis | null;
  /**
   * Issue #56 / F2 — what THIS chat actually runs on: the session row's own
   * provider and model (`restore_provider_from_session` binds exactly those),
   * or the one a turn reported when the privacy barrier repaired the binding
   * mid-turn.
   *
   * ⚠ This chip states the app's global selection, and that is exactly what
   * made the defect invisible: a user switched to a public model, watched this
   * chip change, sent into a private chat, and got an answer from a different
   * model — with this chip still naming the one that was not used. When set,
   * every fact this chip states (the name, the tier padlock, the affiliation)
   * is about the binding that actually runs here.
   *
   * ⚠ It is NOT a signal that anything is wrong, and this component must not
   * editorialise. Most chats are bound to exactly what is selected, and a chat
   * bound to something else may simply have been switched by hand. The sentence
   * explaining a difference the privacy barrier caused is
   * `privacy/PinnedModelNote`, which decides for itself whether there is one.
   */
  effectiveModel?: PinnedModelView;
  /**
   * Where this chat's reasoning effort is kept (`store/reasoningEffort.ts`).
   * Given, the menu carries the Quick, Normal and Deep choice and the chip names
   * a non-default level; omitted, the chip is a model picker only.
   */
  reasoningScope?: string;
}

export default function ModelsBottomBar({
  sessionId,
  dropdownRef,
  setView,
  alerts = [],
  hideAlertPopover = false,
  privacyTier,
  privacyBasis = null,
  effectiveModel,
  reasoningScope,
}: ModelsBottomBarProps) {
  const effort = useReasoningEffortValue(reasoningScope);
  const notesId = useInfoTipId();
  // The menu's Up and Down keys enter effort at its selected segment.
  const effortControlRef = useRef<HTMLDivElement>(null);
  const {
    currentModel,
    currentProvider,
    modelConfigStatus,
    getCurrentModelAndProviderForDisplay,
    getCurrentModelDisplayName,
    getCurrentProviderDisplayName,
    localModelDisplayName,
    loadLocalModelNames,
  } = useModelAndProvider();
  const { getProviders, providersList } = useConfig();
  /**
   * W2-PRV-6. A chat that has not been sent yet is still a chat: its switch
   * holds a model for it (see `pendingChatModel.ts`) instead of rewriting the
   * model every new chat starts on. Home provides no such chat and keeps its
   * explicit new-chats scope.
   */
  const pendingChat = usePendingChatModel();
  const unsentChat = !sessionId && pendingChat !== null;
  const [displayProvider, setDisplayProvider] = useState<string | null>(null);
  const [displayModelName, setDisplayModelName] = useState<string>('Select Model');
  const [isAddModelModalOpen, setIsAddModelModalOpen] = useState(false);
  // The model menu, controlled for the chip's whole life so its hint can stay shut over it: the
  // chip's tooltip names the model on a hover and never comes back over the open menu (H2).
  const [menuOpen, setMenuOpen] = useState(false);
  const menuHint = useSurfaceTriggerHint(menuOpen);
  const [providerDefaultModel, setProviderDefaultModel] = useState<string | null>(null);
  /**
   * V2-R8-1. What the catalog said about each provider, KEYED BY THE PROVIDER
   * IT WAS READ FOR. The four facts this chip states about its model (the
   * disclosure, the provider's display name, the affiliation and the tier
   * padlock) are derived from it below, and only from the entry for the
   * provider the chip names in THIS render.
   *
   * ⚠ They used to be four state hooks the effect below overwrote once its read
   * landed. Between a binding changing and that read, the chip paired the new
   * model's name with the PREVIOUS provider's padlock: measured after a reload,
   * Home on a private model, then a public chat (`aws_bedrock`) opened from the
   * sidebar read "claude-sonnet-4-6" with a Private padlock for about 70 ms,
   * then dropped it and the name moved 24px. A privacy mark that is false and
   * then withdrawn is the one thing this chip must never draw. Keyed, a binding
   * this mount has not read yet has no entry and the chip says nothing.
   *
   * ⚠ V2-R8-3: a mount starts from the catalog the app already holds. The
   * effect's own read is `getProviders(false)`, which answers from that very
   * list whenever it holds one (`ConfigContext`), so this is the same rows one
   * commit sooner, not an older answer. It matters because a new chat's first
   * send REMOUNTS the composer (`composer/composerHandOff.ts`): started empty,
   * the new chip drew the model without its padlock until its read landed, and
   * the padlock went and came back within 50 ms of Enter.
   */
  const [catalogSamples, setCatalogSamples] = useState<ReadonlyMap<string, ProviderSample>>(() =>
    sampleCatalog(providersList)
  );

  /**
   * SD-1 — is this chip's model chosen here, or on the machine running
   * `biorouter serve`?
   *
   * ⚠ **Not state, and not fetched.** The surface a renderer is running on
   * cannot change while it is running, so this is read straight from the DOM
   * marker `renderer.tsx` stamps; a state hook here would add a render in which
   * the two menu items are still offered.
   */
  const hostManaged = isBrowserSurface();

  /**
   * Issue #56 Gate B. What actually runs in THIS chat: the pin when there is
   * one, then the app's global selection.
   *
   * ⚠ **It must follow the name the chip prints, not the app-wide selection.**
   * Every other fact below — the tier padlock, the affiliation glyph, the
   * disclosure line — is read off ONE catalog row for this provider, so a chip
   * naming one provider's model must hang that provider's tier on it.
   */
  const effectiveProvider = effectiveModel?.provider ?? currentProvider;

  // Determine which model to display. A chat's own binding outranks the app-wide
  // selection.
  //
  // ⚠ A branch used to sit here reading the live model from a React context in
  // `BaseChat.tsx` that nothing ever provided, so it could not run on any
  // surface; `src/test/contextProviders.test.ts` fails on the next context
  // created without a provider.
  const displayModel =
    effectiveModel?.model ?? (currentModel || providerDefaultModel || displayModelName);
  const fullModelLabel = displayModel;

  /**
   * W2-5. A Llama Server model goes by its catalog name, the one the model
   * picker lists it under ("Qwen3.8 27B", not `qwen3.8-27b`), in the chip and
   * the menu. The id stays in the tooltip and closes the accessible name. `null`
   * for every other model, and until the catalog has been read. Called
   * optionally, so a context that predates the two members still names the id.
   */
  const localModelName = localModelDisplayName?.(effectiveProvider, displayModel) ?? null;
  const spokenModelLabel = localModelName ?? fullModelLabel;
  // The selection's own local model is read by the model context; a chat bound
  // to one while the selection is elsewhere asks for the catalog here.
  useEffect(() => {
    if (effectiveProvider === 'llamacpp') void loadLocalModelNames?.();
  }, [effectiveProvider, loadLocalModelNames]);

  // Update display provider when current provider changes
  useEffect(() => {
    if (currentProvider) {
      (async () => {
        const providerDisplayName = await getCurrentProviderDisplayName();
        if (providerDisplayName) {
          setDisplayProvider(providerDisplayName);
        } else {
          const modelProvider = await getCurrentModelAndProviderForDisplay();
          setDisplayProvider(modelProvider.provider);
        }
      })();
    }
  }, [currentProvider, getCurrentProviderDisplayName, getCurrentModelAndProviderForDisplay]);

  // Fetch provider default model when provider changes and no current model
  useEffect(() => {
    if (currentProvider && !currentModel) {
      (async () => {
        try {
          const metadata = await getProviderMetadata(currentProvider, getProviders);
          setProviderDefaultModel(metadata.default_model);
        } catch (error) {
          console.error('Failed to get provider default model:', error);
          setProviderDefaultModel(null);
        }
      })();
    } else if (currentModel) {
      // Clear provider default when we have a current model
      setProviderDefaultModel(null);
    }
  }, [currentProvider, currentModel, getProviders]);

  // Update display model name when current model changes
  useEffect(() => {
    (async () => {
      const displayName = await getCurrentModelDisplayName();
      setDisplayModelName(displayName);
    })();
  }, [currentModel, getCurrentModelDisplayName]);

  // Task 30A. The bound provider's own tier, resolved from the registry the
  // daemon serves — never a list kept here.
  //
  // ⚠ Issue #56 DR-26: the same fetch now also yields the bound model's
  // AFFILIATION. One fetch and one row, deliberately — the two axes are decided
  // by one endpoint resolution in the daemon (`ProviderAffiliation::of` takes
  // both off a single `&dyn Provider`), and two fetches here could pair one
  // provider's tier with another's institution across a model switch.
  //
  // ⚠ V2-R8-1: every row of the read is kept, each under its own provider, so a
  // later switch to a provider this mount has already read is stated in the
  // very render that switches, from that provider's own row. Nothing read for
  // one provider is ever shown under another's name. The sibling effect that
  // fills `displayProvider` asks the global model context, which by definition
  // cannot name a provider this chat was pinned to, so the display name comes
  // off the same row too.
  useEffect(() => {
    if (!effectiveProvider) return;
    let cancelled = false;
    (async () => {
      try {
        const rows = await getProviders(false);
        if (cancelled) return;
        const next = new Map(sampleCatalog(rows));
        // A provider Biorouter cannot classify is one it cannot vouch for.
        if (!next.has(effectiveProvider)) next.set(effectiveProvider, UNVOUCHED_PROVIDER);
        setCatalogSamples(next);
      } catch {
        if (!cancelled) {
          setCatalogSamples((previous) =>
            new Map(previous).set(effectiveProvider, UNVOUCHED_PROVIDER)
          );
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [effectiveProvider, getProviders]);

  // The entry for the provider this render names, or none. No entry is
  // *unresolved*, and every fact below then says nothing.
  const boundSample = effectiveProvider ? catalogSamples.get(effectiveProvider) : undefined;
  /**
   * Task 30A (issue #56, DR-17 requirement 3). Does the model bound to this
   * chat need the one-line disclosure?
   *
   * ⚠ It hangs off the bound PROVIDER's tier, never off {@link privacyTier}.
   * That prop is the chat's ratcheted CLASSIFICATION, and a fresh chat on Versa
   * is classified `public` while its model is emphatically not a public model —
   * so a line keyed on it would tell the user something false about the one
   * provider this whole feature exists to make safe to use.
   *
   * `null` while unresolved: say nothing rather than guess, in a chip that is
   * re-rendered on every keystroke in the composer.
   */
  const needsDisclosure: boolean | null = boundSample ? boundSample.needsDisclosure : null;
  /**
   * The display name of the provider this chat is PINNED to, read off the same
   * catalog row as the tier and affiliation below. `null` until it resolves, and
   * for a provider the catalog cannot name — in both cases the chip falls back
   * to the provider's id, which is true rather than invented.
   */
  const pinnedProviderName: string | null = boundSample?.displayName ?? null;
  /**
   * Issue #56, DR-26 — *under whose agreements?* for the model bound to this
   * chat. `null` renders nothing, which is both the "not resolved yet" answer
   * and the correct answer for a public model.
   */
  const affiliation: ProviderAffiliation | null = boundSample?.affiliation ?? null;
  /**
   * Issue #56, DR-26 — the bound model's OWN tier, instance-resolved.
   *
   * ⚠ This chip is a MODEL chip (a brain glyph and a model name), and until
   * this field existed the only tier it could state was {@link privacyTier} —
   * the chat's. So a UCSF Versa model rendered with the chat's public dot
   * beside its name, above a tooltip reading `gpt-5.5-… · Public chat`, and the
   * operator read the only available subject: the model. The reasoning was
   * already written out above, for the disclosure line, and simply never
   * applied to the badge or the word.
   *
   * Sampled in the SAME row of the SAME fetch as {@link affiliation}, not via
   * `useBoundProviderTier` — a second fetch here could pair one provider's tier
   * with another's institution across a model switch, which is the pairing this
   * chip exists to state.
   *
   * `undefined` is *unresolved* and renders nothing. It is the answer for an
   * unconfigured provider, a construction failure, a daemon older than the
   * field, and a provider this mount has not read yet — never "public".
   */
  const boundTier: ProviderTier | undefined = boundSample?.tier;

  // ⚠ Unconditional on the master privacy switch — DR-15 turns off enforcement,
  // not the truth. See `privacy/disclosureCopy.ts`.
  const { copy: disclosure } = useDisclosure(needsDisclosure === true);
  // A decision chat (B-3) runs a model that reads only the chat it answers:
  // it calls no tools, so "this model can read files on this computer" is not
  // true of it, and the line is not drawn. The DR-17 text itself is unchanged.
  const decisionChat = isDecisionProvider(effectiveProvider);
  const disclosureLine =
    needsDisclosure === true && !decisionChat ? (disclosure?.short ?? null) : null;

  // §14.2's line for the chat's tier. A "Private" PILL cannot fit in this chip
  // — the trigger is `max-w-[120px]` and the label is already truncated at 24
  // characters — so the chip carries the dense padlock and the WORD goes where
  // there is room for it: the tooltip and the dropdown header.
  const privacyLine =
    privacyTier === 'private'
      ? 'Private chat. Biorouter only lets private models, decision models included, read it.'
      : privacyTier === 'public'
        ? privacyBasis
          ? privacyBasisChipLine(privacyBasis)
          : 'Public chat'
        : null;

  // The MODEL's tier, in the model's own words. Every surface below states
  // this one FIRST and adjacent to the model name, and states `privacyLine`
  // after it and separately — because the two are different subjects that
  // disagree routinely, and the whole defect here was one standing where the
  // other belonged.
  const modelTierWords =
    boundTier === 'private' ? 'Private model' : boundTier === 'public' ? 'Public model' : null;

  // DR-26's third axis, in the same two places the tier's WORD goes: this chip
  // has room for a glyph and nothing more. `null` for a public model, which has
  // no affiliation, so a public chat's chip is byte-for-byte what it was.
  const affiliationWords = affiliationPresentation(affiliation);

  /**
   * The two names in the dropdown's "Current model" block, pinned binding first,
   * then the app's global selection.
   *
   * Layered here rather than inside the effects that fill `displayModelName` /
   * `displayProvider`: those two describe the app's GLOBAL selection, which is
   * still the right answer for every chat that is not pinned, and having two
   * effects race to own one state was how the earlier drafts of this went
   * wrong.
   *
   * ⚠ **The same model as the chip, always.**
   */
  const shownModelName = effectiveModel?.model ?? displayModelName;
  const shownProviderName = effectiveModel
    ? (pinnedProviderName ?? effectiveProvider)
    : displayProvider;

  // What is true of the MODEL, as one clause: its tier, then who covers it.
  // Both axes come off one sample of one endpoint, so they can be said in one
  // breath without risking a cross-provider pairing.
  const modelClause = [modelTierWords, affiliationWords?.label].filter(Boolean).join(', ');

  /**
   * Nothing is bound, so there is no "current model" for the dropdown to be
   * about. The chip becomes the way IN to the catalog instead — reachable since
   * a user can now enter the app before configuring anything.
   *
   * ⚠ A plain button rather than a disabled dropdown: the menu's items adjust a
   * model, and there is no model to adjust. One control, one meaning.
   */
  if (hasNoModelConfigured(modelConfigStatus, effectiveModel?.provider ?? currentProvider)) {
    return (
      <div className="relative flex min-w-0 items-center" ref={dropdownRef}>
        {!hideAlertPopover && <BottomMenuAlertPopover alerts={alerts} />}
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              onClick={() => setView?.('ConfigureProviders')}
              data-testid="model-chip-choose-model"
              aria-label={NO_MODEL_CHIP_LABEL}
              className="br-picker-chip"
              data-kind="model"
            >
              <span className="br-picker-chip__name">{NO_MODEL_CHIP_LABEL}</span>
            </button>
          </TooltipTrigger>
          <TooltipContent side="top">{MODEL_COPY.noModelTooltip}</TooltipContent>
        </Tooltip>
      </div>
    );
  }

  // Spec 3.7: the chip names the model the way a person says it, so the date
  // stamp comes off (`gpt-5.6-sol-2026-07-09` reads `gpt-5.6-sol`). The full id
  // stays in the tooltip, the menu and the accessible name.
  // A decision model goes by its alias (R3): `joey-1`, never `decision-models/joey-1`.
  const decisionAlias =
    decisionChat && effectiveProvider && displayModel
      ? decisionModelLabel({ provider: effectiveProvider, model: displayModel })
      : null;
  const chipModelLabel = decisionAlias ?? localModelName ?? friendlyModelName(displayModel);
  // A decision model has no reasoning effort to set: it answers in one step.
  const showEffort = !decisionChat && effort !== null && effort !== DEFAULT_REASONING_EFFORT;
  // The InfoTip's notes: where a choice here reaches. At most one applies (Home
  // vs a chat with its own binding).
  const notes = [
    !sessionId && !unsentChat ? NEW_CHATS_MODEL_NOTE : null,
    effectiveModel ? CHAT_KEEPS_ITS_MODEL_NOTE : null,
  ]
    .filter(Boolean)
    .join(' ');
  // R1-7 and R1-9: a decision model's line says where it runs, in the words every
  // decision surface uses ("Runs on this computer", "Runs on Baseten"), and wraps
  // rather than cutting off. A local model's affiliation ("On this machine") says
  // the same thing again, so it is not added; an institutional one still is.
  const decisionPlace = decisionChat
    ? decisionModelPlace(effectiveProvider ?? '', { tier: boundTier })
    : null;
  const providerLine = (
    decisionPlace
      ? [switchModelCopy.decisionRunsOn(decisionPlace), modelTierWords]
      : [shownProviderName, modelTierWords]
  )
    .filter(Boolean)
    .join(' · ');
  const lineAffiliationWords =
    decisionPlace && affiliation?.kind === 'local' ? null : affiliationWords;
  const lineModelName =
    decisionChat && effectiveProvider && shownModelName
      ? (decisionModelLabel({ provider: effectiveProvider, model: shownModelName }) ??
        shownModelName)
      : (localModelName ?? shownModelName);
  // DR-17 and §14.2, kept VISIBLE as one line (principle 2): the chat's tier,
  // then the standing disclosure the daemon serves for a non-private model.
  const showPrivacyBlock = Boolean(privacyLine || disclosureLine);

  return (
    <div className="relative flex min-w-0 items-center" ref={dropdownRef}>
      {!hideAlertPopover && <BottomMenuAlertPopover alerts={alerts} />}
      <DropdownMenu open={menuOpen} onOpenChange={setMenuOpen}>
        <Tooltip {...menuHint.tooltip}>
          <TooltipTrigger asChild {...menuHint.trigger}>
            <DropdownMenuTrigger
              // The parenthetical belongs to the MODEL; the sentence after the
              // period belongs to the CHAT. The old label put the chat's tier
              // between the model's name and the model's affiliation, where the
              // only subject in reach was the model.
              aria-label={joinSentences([
                `Current model: ${spokenModelLabel}${modelClause ? ` (${modelClause})` : ''}`,
                privacyLine,
                !decisionChat && effort !== null && `Effort: ${REASONING_EFFORT_LABELS[effort]}`,
                // W2-5. The id the visible name stands for, in the tooltip's words.
                localModelName && MODEL_COPY.fullIdTooltip(displayModel),
              ])}
              data-testid="model-chip"
              data-kind="model"
              className="br-picker-chip"
            >
              <span className="br-picker-chip__name">
                {chipModelLabel}
                {showEffort && ` · ${REASONING_EFFORT_LABELS[effort!]}`}
              </span>
              {/* The MODEL's tier, as the padlock every private thing in the app
                  carries (`PrivacyBadge`). The chat's own tier has its own
                  surface at the top of the chat. The affiliation is in the
                  tooltip and the menu: this chip has room for one mark. */}
              {boundTier && (
                <span className="br-picker-chip__badge">
                  <PrivacyBadge tier={boundTier} dense />
                </span>
              )}
              <ChevronDown className="br-picker-chip__chevron" aria-hidden="true" />
            </DropdownMenuTrigger>
          </TooltipTrigger>
          <TooltipContent side="top">
            {MODEL_COPY.fullIdTooltip(fullModelLabel)}
            {modelTierWords && ` · ${modelTierWords}`}
            {affiliationWords && ` · ${affiliationWords.label}`}
            {/* On its own line: the chat's tier is a different subject from the
                model's, and a `·` would make it read as one. */}
            {privacyLine && <span className="mt-1 block">{privacyLine}</span>}
            {disclosureLine && (
              <span className="mt-1 block max-w-[280px] [overflow-wrap:anywhere]">
                {disclosureLine}
              </span>
            )}
          </TooltipContent>
        </Tooltip>
        <DropdownMenuContent
          side="top"
          align="end"
          className="w-72"
          aria-describedby={notes ? notesId : undefined}
          // The menu's arrow keys visit rows only, and Tab leaves the menu; this adds the selected
          // effort segment and the InfoTips (the two About glyphs, the affiliation badge).
          onKeyDownCapture={(event) => moveFocusAcrossMenuStops(event, effortControlRef.current)}
        >
          <div className="px-3 pt-2 pb-1.5">
            <div className="flex items-center text-supporting text-text-muted">
              <span>
                {sessionId || unsentChat ? MODEL_COPY.currentModel : NEW_CHATS_MODEL_HEADING}
              </span>
              {notes && (
                <InfoTip
                  id={notesId}
                  label={MODEL_COPY.aboutModel}
                  help={notes}
                  data-testid="model-notes-info"
                />
              )}
            </div>
            <div className="mt-0.5 truncate text-label text-text-default">{lineModelName}</div>
            {providerLine && (
              // Read whole at the menu's 288px, never cut (live QA, 2026-11-12: "Llama
              // Server · Private …" and "On this mac…"). The provider and the tier are one
              // line that wraps; who covers the model is its own line, glyph first, so a
              // wrap never strands a "·" at the start of a line.
              <div
                data-testid="model-provider-line"
                className="flex min-w-0 flex-col text-supporting text-text-muted [overflow-wrap:anywhere]"
              >
                <span className="min-w-0">{providerLine}</span>
                {lineAffiliationWords && (
                  <span
                    className="flex min-w-0 items-center gap-1.5"
                    data-testid="model-affiliation-line"
                  >
                    <AffiliationBadge affiliation={affiliation} dense />
                    <span className="min-w-0">{lineAffiliationWords.label}</span>
                  </span>
                )}
              </div>
            )}
            {/* A decision model answers each message with a choice and its
                probabilities, not with text; said here so the chip's menu names
                what kind of model the chat runs on. */}
            {decisionChat && (
              <div
                data-testid="model-decision-line"
                className="text-supporting text-text-muted [overflow-wrap:anywhere]"
              >
                {`${switchModelCopy.decisionModel}: ${switchModelCopy.decisionDetail.toLowerCase()}`}
              </div>
            )}
            {showPrivacyBlock && (
              // Two subjects, two lines (V2-05). They used to be two spans in one
              // `<p>` joined by a space, which read as the single run-on "Public
              // chat Not HIPAA-compliant, …": the chat's tier is a label with no
              // full stop, and the disclosure is a sentence about the MODEL. The
              // texts are unchanged (§14.2 and DR-17 are locked); only the break
              // between them is new. Still one `text-supporting` block (spec 3.7).
              <div className="mt-1.5 flex flex-col gap-1 text-supporting text-text-muted [overflow-wrap:anywhere]">
                {privacyLine && <p data-testid="chat-privacy-line">{privacyLine}</p>}
                {disclosureLine && (
                  <p data-testid="non-private-model-chip-note">{disclosureLine}</p>
                )}
              </div>
            )}
          </div>
          {/*
            SD-1. Both items below write `BIOROUTER_PROVIDER`, and a browser-served
            daemon refuses them with a 409. The note says so inside the same block
            as the items, so the reason is visible where they are grey.
          */}
          <HostManagedModelNote variant="inset" />
          {effort !== null && reasoningScope && !decisionChat && (
            <>
              <DropdownMenuSeparator />
              {/* Effort applies immediately to this scope while the menu stays open. */}
              <EffortSection scope={reasoningScope} effort={effort} controlRef={effortControlRef} />
            </>
          )}
          <DropdownMenuSeparator />
          <DropdownMenuItem
            disabled={hostManaged}
            onClick={hostManaged ? undefined : () => setIsAddModelModalOpen(true)}
          >
            {MODEL_COPY.changeModel}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      {isAddModelModalOpen ? (
        /*
          D6 — the dialog is headed "Select a provider and model for this chat",
          so it must OPEN on that chat's own binding. Left to its own fallback it
          pre-fills `ModelAndProviderContext`'s app-wide selection, and pressing
          "Select model" without touching anything then moved the chat to a model
          the user never chose.

          Measured (dev GUI, 2026-09-12): chat `20260610_28`, row
          `provider_name = versa_azure`, `model_name = gpt-5.2-2025-12-11`,
          `privacy_tier = private`; `BIOROUTER_MODEL = gpt-5.5-2026-04-24`. The
          chip read `gpt-5.2-2025-12-11`, the dialog opened on
          `gpt-5.5-2026-04-24`. Model identity decides the privacy tier, so a
          silent move is a correctness bug, not a cosmetic one.

          ⚠ `effectiveModel` is the right source and `undefined` is the right
          pass-through. `usePinnedModel` sets it exactly when the chat's binding
          DIFFERS from the selection; where it is unset the two agree (or the chat
          has no binding of its own and genuinely runs the selection), and the
          modal's own fallback to `currentProvider`/`currentModel` is then the
          same pair.
        */
        <SwitchModelModal
          sessionId={sessionId}
          privacyTier={privacyTier}
          privacyBasis={privacyBasis}
          initialProvider={effectiveModel?.provider}
          initialModel={effectiveModel?.model}
          setView={setView}
          onChooseForUnsentChat={unsentChat ? pendingChat?.choose : undefined}
          unsentChatTabId={unsentChat ? pendingChat?.tabId : undefined}
          onClose={() => setIsAddModelModalOpen(false)}
        />
      ) : null}
    </div>
  );
}
