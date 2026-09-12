import { SlidersHorizontal, Brain } from '../../../icons/app-icons';
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useModelAndProvider } from '../../../ModelAndProviderContext';
import { NO_MODEL_CHIP_LABEL, hasNoModelConfigured } from '../../../composerNoProvider';
import { SwitchModelModal } from '../subcomponents/SwitchModelModal';
import { LeadWorkerSettings } from '../subcomponents/LeadWorkerSettings';
import { View } from '../../../../utils/navigationUtils';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../../../ui/dropdown-menu';
import { useConfig } from '../../../ConfigContext';
import { getProviderMetadata } from '../modelInterface';
import { Alert } from '../../../alerts';
import BottomMenuAlertPopover from '../../../bottom_menu/BottomMenuAlertPopover';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../../ui/Tooltip';
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
import { HOST_MANAGED_MODEL_REASON } from '../../../privacy/hostManagedModelCopy';
import { isBrowserSurface } from '../../../../utils/surface';
import type { ProviderTier, SessionClassification } from '../../../../api/types.gen';
import type { PinnedModelView } from '../../../../hooks/chatStreamStore';
import { subscribeAppModelSelectionChanges } from '../../../../utils/sessionBindingSync';
import {
  DEFAULT_LEAD_TURNS,
  leadWorkerChip,
  leadWorkerHandoverNote,
  readLeadTurns,
  type LeadWorkerChip,
} from './leadWorkerLabel';

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
 * yet (Home, a chat not started).
 *
 * There the chip names the APP-WIDE selection — the pair `/agent/start` will
 * bind — and switching from it changes that pair for every window. "Current
 * model" read as a property of this screen; the heading says whose model it is,
 * and the line says how far a change reaches, beside the control that makes it.
 */
export const NEW_CHATS_MODEL_HEADING = 'Model for new chats';
export const NEW_CHATS_MODEL_NOTE =
  'New chats in every window start on this model. Existing chats keep their own.';

interface ModelsBottomBarProps {
  sessionId: string | null;
  dropdownRef: React.RefObject<HTMLDivElement>;
  setView: (view: View) => void;
  alerts: Alert[];
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
}

const MAX_INLINE_MODEL_LABEL_CHARS = 24;

export default function ModelsBottomBar({
  sessionId,
  dropdownRef,
  setView,
  alerts,
  hideAlertPopover = false,
  privacyTier,
  effectiveModel,
}: ModelsBottomBarProps) {
  const {
    currentModel,
    currentProvider,
    modelConfigStatus,
    getCurrentModelAndProviderForDisplay,
    getCurrentModelDisplayName,
    getCurrentProviderDisplayName,
  } = useModelAndProvider();
  const { read, getProviders } = useConfig();
  const [displayProvider, setDisplayProvider] = useState<string | null>(null);
  const [displayModelName, setDisplayModelName] = useState<string>('Select Model');
  const [isAddModelModalOpen, setIsAddModelModalOpen] = useState(false);
  const [isLeadWorkerModalOpen, setIsLeadWorkerModalOpen] = useState(false);
  const [providerDefaultModel, setProviderDefaultModel] = useState<string | null>(null);
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
  const [needsDisclosure, setNeedsDisclosure] = useState<boolean | null>(null);
  /**
   * The display name of the provider this chat is PINNED to, read off the same
   * catalog row as the tier and affiliation below. `null` until it resolves, and
   * for a provider the catalog cannot name — in both cases the chip falls back
   * to the provider's id, which is true rather than invented.
   */
  const [pinnedProviderName, setPinnedProviderName] = useState<string | null>(null);
  /**
   * Issue #56, DR-26 — *under whose agreements?* for the model bound to this
   * chat. `null` renders nothing, which is both the "not resolved yet" answer
   * and the correct answer for a public model.
   */
  const [affiliation, setAffiliation] = useState<ProviderAffiliation | null>(null);
  /**
   * Issue #56, DR-26 — the bound model's OWN tier, instance-resolved.
   *
   * ⚠ This chip is a MODEL chip (a brain glyph and a model name), and until
   * this field existed the only tier it could state was {@link privacyTier} —
   * the chat's. So a UCSF Versa model rendered with the chat's public dot
   * beside its name, above a tooltip reading `gpt-5.5-… · Public chat`, and the
   * operator read the only available subject: the model. The reasoning was
   * already written out one state hook above, for the disclosure line, and
   * simply never applied to the badge or the word.
   *
   * Sampled in the SAME row of the SAME fetch as {@link affiliation}, not via
   * `useBoundProviderTier` — a second fetch here could pair one provider's tier
   * with another's institution across a model switch, which is the pairing this
   * chip exists to state.
   *
   * `undefined` is *unresolved* and renders nothing. It is the answer for an
   * unconfigured provider, a construction failure, and a daemon older than the
   * field — never "public".
   */
  const [boundTier, setBoundTier] = useState<ProviderTier | undefined>(undefined);

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

  // The app-wide lead/worker selection, as its three config keys state it.
  // `BIOROUTER_MODEL` IS the worker while a pair is on, so there is no
  // "which half is live" key to read — see `leadWorkerLabel.ts` for what that
  // leaves knowable, and for the false `(worker)` it used to produce.
  const [leadModelName, setLeadModelName] = useState<string>('');
  const [leadProviderName, setLeadProviderName] = useState<string>('');
  const [currentActiveModel, setCurrentActiveModel] = useState<string>('');
  const [leadTurns, setLeadTurns] = useState<number>(DEFAULT_LEAD_TURNS);

  /**
   * One read for the whole lead/worker question: is a pair configured, what are
   * the lead's model and provider, what does `BIOROUTER_MODEL` name, and how many
   * turns the lead opens with.
   *
   * ⚠ **The answers move together or not at all.** They used to be three reads
   * across two effects and a modal-close handler, so "is a pair configured" could
   * be refreshed while `currentActiveModel` stayed behind — and a half-refreshed
   * chip puts the wrong role on the right model, which is worse than carrying no
   * role. D7's two new answers joined the same `Promise.all` for that reason
   * rather than taking reads of their own.
   */
  const refreshLeadWorker = useCallback(async () => {
    try {
      const [leadModel, leadProvider, activeModel, turns] = await Promise.all([
        read('BIOROUTER_LEAD_MODEL', false),
        read('BIOROUTER_LEAD_PROVIDER', false),
        read('BIOROUTER_MODEL', false),
        read('BIOROUTER_LEAD_TURNS', false),
      ]);
      setLeadModelName((leadModel as string) || '');
      setLeadProviderName((leadProvider as string) || '');
      setCurrentActiveModel((activeModel as string) || '');
      setLeadTurns(readLeadTurns(turns));
    } catch (error) {
      console.error('Error reading the lead/worker selection:', error);
      // A selection we could not read is not a pair, and must not leave a stale
      // `(lead)` on a name from the last successful read.
      setLeadModelName('');
      setLeadProviderName('');
    }
  }, [read]);

  /**
   * F3's other half. Saving a lead/worker pair rewrites `BIOROUTER_MODEL` to
   * the WORKER, and that write is announced on `sessionBindingSync`, so since
   * #247 every window's chip already follows the new *value*. Nothing told the
   * other windows about the *role*: they kept the `isLeadWorkerActive: false`
   * they had read at mount, so the window that saved drew
   * `gpt-4.1-mini-2025-04-14 (worker)` and every other window drew a bare
   * `gpt-4.1-mini-2025-04-14`.
   *
   * Two windows of one app then said different things about one selection, and
   * the one saying less was saying the more misleading thing: a bare name reads
   * as "this is simply the model", with nothing to suggest a lead is configured
   * at all.
   *
   * ⚠ Mount-once, with the handler reached through a ref — the subscription
   * belongs to the mount, not to a callback's identity. Hanging it off
   * `refreshLeadWorker` would tear it down and remake it every time `read`'s
   * identity moved (the rule `ConfigContext`'s catalogue subscription and
   * `ModelAndProviderContext`'s selection subscription both record).
   */
  const refreshLeadWorkerRef = useRef(refreshLeadWorker);
  useEffect(() => {
    refreshLeadWorkerRef.current = refreshLeadWorker;
  }, [refreshLeadWorker]);

  useEffect(() => {
    void refreshLeadWorkerRef.current();
    return subscribeAppModelSelectionChanges(() => {
      void refreshLeadWorkerRef.current();
    });
  }, []);

  // Refresh when the modal closes as well. The save inside it announces, so
  // this covers a close that wrote nothing through that channel — a cancel
  // after an edit, or a save that failed part-way.
  const handleLeadWorkerModalClose = () => {
    setIsLeadWorkerModalOpen(false);
    void refreshLeadWorker();
  };

  /**
   * D7 — the name and role a lead/worker pair earns on THIS surface. `sessionId`
   * is the whole input: with no chat the next message opens a new session and the
   * lead answers it, so the chip names the lead; inside a chat the live half
   * depends on a turn count the renderer is never served, so no role is claimed.
   * `leadWorkerLabel.ts` carries the measurement and the reasoning.
   *
   * A chat with its own binding runs a single provider, so the pair is not in
   * play at all and is not asked about — the rule {@link effectiveProvider}
   * below also records.
   */
  const pair = {
    leadModel: leadModelName,
    // `create_lead_worker_from_env` falls back to the default provider for an
    // unset `BIOROUTER_LEAD_PROVIDER`, so the fallback is resolved here and
    // nothing downstream has to know about it.
    leadProvider: leadProviderName || currentProvider || '',
    workerModel: currentActiveModel,
    leadTurns,
  };
  const chipPair: LeadWorkerChip = effectiveModel ? {} : leadWorkerChip(pair, !!sessionId);
  const handoverNote = effectiveModel ? null : leadWorkerHandoverNote(pair);

  /**
   * Issue #56 Gate B. What actually runs in THIS chat: the pin when there is
   * one, then the half of a lead/worker pair the chip is naming, then the app's
   * global selection.
   *
   * ⚠ The chat's own binding outranks lead/worker. Such a chat runs the single
   * provider its session row names, so a lead/worker pair configured globally is
   * not what answers here, and labelling the chip `(lead)` would name a mechanism
   * that is not in play.
   *
   * ⚠ **It must follow the name the chip prints, not the app-wide selection.**
   * Every other fact below — the tier padlock, the affiliation glyph, the
   * disclosure line — is read off ONE catalog row for this provider. When the chip
   * names the lead (D7, no chat) and this still named the worker's provider, the
   * chip hung the worker's tier on the lead's name: a public lead wore the
   * worker's Private padlock over turns that really go to a public endpoint.
   */
  const effectiveProvider = effectiveModel?.provider ?? chipPair.provider ?? currentProvider;

  // Determine which model to display. The pair's own answer outranks the app-wide
  // selection; a chat's own binding outranks both.
  //
  // ⚠ `useCurrentModelInfo()` is NOT consulted: `CurrentModelContext` is created
  // and read in `BaseChat.tsx` and never provided, so the branch that used to sit
  // here could not run on any surface. See `leadWorkerLabel.ts`.
  const displayModel =
    effectiveModel?.model ??
    chipPair.model ??
    (currentModel || providerDefaultModel || displayModelName);
  const fullModelLabel = chipPair.role ? `${displayModel} (${chipPair.role})` : displayModel;
  const inlineModelLabel =
    fullModelLabel.length > MAX_INLINE_MODEL_LABEL_CHARS
      ? `${fullModelLabel.slice(0, MAX_INLINE_MODEL_LABEL_CHARS - 3)}...`
      : fullModelLabel;

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
  useEffect(() => {
    if (!effectiveProvider) {
      setNeedsDisclosure(null);
      setAffiliation(null);
      setBoundTier(undefined);
      setPinnedProviderName(null);
      return;
    }
    let cancelled = false;
    (async () => {
      try {
        const rows = await getProviders(false);
        const row = rows.find((candidate) => candidate.name === effectiveProvider);
        if (!row) throw new Error(`No match for provider: ${effectiveProvider}`);
        if (cancelled) return;
        // Off the SAME row as the tier and affiliation below. The sibling
        // effect that fills `displayProvider` asks the global model context,
        // which by definition cannot name a provider this chat was pinned to.
        setPinnedProviderName(row.metadata.display_name ?? null);
        setNeedsDisclosure(disclosureRequiredForTier(row.metadata.tier));
        // Read off the ROW, never `row.metadata`: the metadata's tier is the
        // type-level claim, and affiliation is served beside it precisely
        // because DR-26 requires an instance-resolved value.
        setAffiliation(readProviderAffiliation(row));
        setBoundTier(readResolvedProviderTier(row));
      } catch {
        // A provider Biorouter cannot classify is one it cannot vouch for.
        // Fail-safe here means fail towards telling the user.
        if (!cancelled) {
          setPinnedProviderName(null);
          setNeedsDisclosure(true);
          // ...but NOT towards claiming an affiliation. Failing safe on the
          // disclosure means saying more; failing safe on the third axis means
          // saying nothing, because every value here is a claim about whose
          // agreements cover a transcript.
          setAffiliation(null);
          // Same direction, same reason. A catalog we cannot read is not
          // evidence that the model is public.
          setBoundTier(undefined);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [effectiveProvider, getProviders]);

  // ⚠ Unconditional on the master privacy switch — DR-15 turns off enforcement,
  // not the truth. See `privacy/disclosureCopy.ts`.
  const { copy: disclosure } = useDisclosure(needsDisclosure === true);
  const disclosureLine = needsDisclosure === true ? (disclosure?.short ?? null) : null;

  // §14.2's line for the chat's tier. A "Private" PILL cannot fit in this chip
  // — the trigger is `max-w-[120px]` and the label is already truncated at 24
  // characters — so the chip carries the dense padlock and the WORD goes where
  // there is room for it: the tooltip and the dropdown header.
  const privacyLine =
    privacyTier === 'private'
      ? 'Private chat. Biorouter only lets a private model open it.'
      : privacyTier === 'public'
        ? 'Public chat'
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
   * then the lead the chip above is naming, then the app's global selection.
   *
   * Layered here rather than inside the effects that fill `displayModelName` /
   * `displayProvider`: those two describe the app's GLOBAL selection, which is
   * still the right answer for every chat that is not pinned, and having two
   * effects race to own one state was how the earlier drafts of this went
   * wrong.
   *
   * ⚠ **The same model as the chip, always.** The note under these names says
   * "New chats in every window start on this model" — so with a pair configured
   * it had to name the lead too, or that sentence pointed at the worker, which
   * is not what a new chat starts on.
   */
  const shownModelName = effectiveModel?.model ?? chipPair.model ?? displayModelName;
  const shownProviderName =
    effectiveModel || chipPair.model ? (pinnedProviderName ?? effectiveProvider) : displayProvider;

  // What is true of the MODEL, as one clause: its tier, then who covers it.
  // Both axes come off one sample of one endpoint, so they can be said in one
  // breath without risking a cross-provider pairing.
  const modelClause = [modelTierWords, affiliationWords?.label].filter(Boolean).join(', ');

  /**
   * Nothing is bound, so there is no "current model" for the dropdown to be
   * about. The chip becomes the way IN to the catalog instead — reachable since
   * a user can now enter the app before configuring anything.
   *
   * ⚠ A plain button rather than a disabled dropdown: the menu's items are
   * "Change model" and "Lead/worker settings", both of which read as adjustments
   * to a model that does not exist. One control, one meaning.
   */
  if (hasNoModelConfigured(modelConfigStatus, currentProvider)) {
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
              className="flex h-7 min-w-0 max-w-[220px] items-center rounded-element px-0.5 hover:cursor-pointer text-text-default/70 tint-interactive hover:text-text-default transition-colors"
            >
              <div className="flex min-w-0 max-w-full items-center gap-0.5 truncate">
                <Brain className="size-[18px] flex-shrink-0" />
                <span className="truncate text-supporting">{NO_MODEL_CHIP_LABEL}</span>
              </div>
            </button>
          </TooltipTrigger>
          <TooltipContent side="top">
            No model is configured yet. Opens the provider catalog.
          </TooltipContent>
        </Tooltip>
      </div>
    );
  }

  return (
    <div className="relative flex min-w-0 items-center" ref={dropdownRef}>
      {!hideAlertPopover && <BottomMenuAlertPopover alerts={alerts} />}
      <DropdownMenu>
        <Tooltip>
          <TooltipTrigger asChild>
            <DropdownMenuTrigger
              // The parenthetical belongs to the MODEL; the sentence after the
              // period belongs to the CHAT. The old label put the chat's tier
              // between the model's name and the model's affiliation —
              // `gpt-5.5-… (Public chat) (Affiliation: UCSF)` — where the only
              // subject in reach was the model.
              aria-label={`Current model: ${fullModelLabel}${modelClause ? ` (${modelClause})` : ''}${
                privacyLine ? `. ${privacyLine}` : ''
              }`}
              // A CAP, and one the label can actually reach. At 120px the
              // name got an 80px span after the glyph and the badges, so
              // `gpt-5.5-2026-04-24` rendered as `gpt-5.5-202…` — a truncation
              // that keeps only the part every model shares — while ~250px of
              // the composer's footer row sat empty immediately to its right.
              // 220px fits the 24-character ceiling `MAX_INLINE_MODEL_LABEL_CHARS`
              // already imposes, so the two limits now agree instead of the CSS
              // one silently biting first.
              //
              // Shrinkable, deliberately: the chip was `flex-shrink-0`, which is
              // safe at 120px and would push the row at 220px. Letting it give
              // way means the cap costs nothing when the composer is narrow.
              className="flex h-7 min-w-0 max-w-[220px] items-center rounded-element px-0.5 hover:cursor-pointer text-text-default/70 tint-interactive hover:text-text-default transition-colors"
            >
              <div className="flex min-w-0 max-w-full items-center gap-0.5 truncate">
                <Brain className="size-[18px] flex-shrink-0" />
                {/* `text-supporting`, the composer rails' role — see the note in
                    ChatInput.tsx, "THE RAILS' TYPE". It was `text-xs`: the same
                    12px by coincidence, not by role, so it would not have
                    followed the rails when they moved. */}
                <span className="truncate text-supporting">{inlineModelLabel}</span>
                {/* The MODEL's tier, not the chat's.
                    The chat's ratcheted classification has its own surface —
                    `SessionNamePill`, at the top of the chat, in the full pill —
                    so this mark repeating it was both duplicative and, next to a
                    model name under a brain glyph, misattributed. On a private
                    chat holding a public model the two now disagree visibly,
                    which is exactly the pairing Gate C refuses.

                    ⚠ It is a PADLOCK, the same one a private conversation and a
                    private extension carry — see `PrivacyBadge`. It was a bare
                    dot, which is the one form of the mark that connected to
                    nothing else in the app. */}
                {boundTier && <PrivacyBadge tier={boundTier} dense className="ml-1" />}
                {/* Beside the tier padlock, in its dense form — the chip is
                    `max-w-[120px]` with an already-truncated label, so the WORDS
                    go where there is room for them (the tooltip and the dropdown
                    header below), exactly as the tier's do. */}
                <AffiliationBadge affiliation={affiliation} dense className="ml-0.5" />
              </div>
            </DropdownMenuTrigger>
          </TooltipTrigger>
          <TooltipContent side="top">
            Model: {fullModelLabel}
            {modelTierWords && ` · ${modelTierWords}`}
            {affiliationWords && ` · ${affiliationWords.label}`}
            {/* On its own line, below — a `·` separator put the chat's tier in
                the same run of dot-joined facts as the model's, which is how
                `gpt-5.5-… · Public chat` came to describe a private model. */}
            {privacyLine && <span className="mt-1 block">{privacyLine}</span>}
            {disclosureLine && (
              <span className="mt-1 block max-w-[280px] [overflow-wrap:anywhere]">
                {disclosureLine}
              </span>
            )}
          </TooltipContent>
        </Tooltip>
        <DropdownMenuContent side="top" align="center" className="w-64 p-0 font-sans">
          <div className="border-b border-border-subtle px-3 py-2.5">
            <div className="text-sm font-medium text-text-default">
              {sessionId ? 'Current model' : NEW_CHATS_MODEL_HEADING}
            </div>
            <div className="mt-0.5 text-supporting leading-4 text-text-muted">
              {shownModelName}
              {shownProviderName && ` · ${shownProviderName}`}
            </div>
            {!sessionId && (
              <div
                data-testid="new-chats-model-note"
                className="mt-1 text-[11px] leading-4 text-text-muted"
              >
                {NEW_CHATS_MODEL_NOTE}
              </div>
            )}
            {/*
              D7 — the pair's whole truth, in the one surface with room for a
              sentence. The chip above can name one half and (off Home) cannot
              even say which half is live, because the turn count that decides it
              is daemon state the renderer is never served. This line states the
              HANDOVER instead of claiming a side of it, so it is true of every
              chat and every surface — the same split this block already uses for
              the tier word, the affiliation word and `CHAT_KEEPS_ITS_MODEL_NOTE`.
            */}
            {handoverNote && (
              <div
                data-testid="lead-worker-handover-note"
                className="mt-1 text-[11px] leading-4 text-text-muted [overflow-wrap:anywhere]"
              >
                {handoverNote}
              </div>
            )}
            {/* Under the heading "Current model", so it must be about the
                model. It used to be `privacyLine`. */}
            {modelTierWords && (
              <div className="mt-1 text-[11px] leading-4 text-text-muted">{modelTierWords}</div>
            )}
            {/*
              DR-26's third axis, with room for the full pill — the one place on
              this chip where the affiliation gets its word rather than its
              glyph. It sits directly under the tier line so the two axes read as
              one statement about the bound model.
            */}
            {affiliationWords && (
              <div className="mt-1.5 flex items-center gap-1.5">
                <AffiliationBadge affiliation={affiliation} className="max-w-full" />
              </div>
            )}
            {/*
              Round 3 / N1 — why the two names above may not be the model the
              user last chose in Settings.

              ⚠ **Here and nowhere else.** The chip and gauge now state the
              chat's own binding for EVERY chat that has one, which means they
              disagree with the app-wide selection in every chat older than the
              user's last model switch — the ordinary case, not an edge one. A
              standing note above the composer would then be near-permanent
              chrome restating what the control beside it already says. This is
              the surface a reader reaches by asking the chip what model this
              chat is on, so it is where the answer to "did my switch fail?"
              belongs.

              It names no cause and gives no instruction: the two ways to move
              this chat are directly below it, and nothing here is broken.
            */}
            {effectiveModel && (
              <div
                data-testid="chat-binding-note"
                className="mt-1.5 text-[11px] leading-4 text-text-muted"
              >
                {CHAT_KEEPS_ITS_MODEL_NOTE}
              </div>
            )}
            {/*
              The CHAT's tier, last and set apart by a rule, because everything
              above it is about the model and this is not. Its copy names its
              own subject ("Public chat" / "Private chat — …"), which is what
              makes it safe to sit in the same block at all.
            */}
            {privacyLine && (
              <div className="mt-2 border-t border-border-subtle pt-2 text-[11px] leading-4 text-text-muted">
                {privacyLine}
              </div>
            )}
            {/*
              Issue #56, DR-17 requirement 3 — the standing one-line disclosure,
              in the one place on this chip with room for a sentence. The words
              come from the daemon; a literal here would be a second definition
              and would be the one that shipped stale.
            */}
            {disclosureLine && (
              <div
                data-testid="non-private-model-chip-note"
                className="mt-1 text-[11px] leading-4 text-text-muted [overflow-wrap:anywhere]"
              >
                {disclosureLine}
              </div>
            )}
          </div>
          {/*
            SD-1. Both items below write `BIOROUTER_PROVIDER` — the first through
            `/config/set_provider`, the second through `/config/upsert` on that
            key and on `BIOROUTER_LEAD_*` — and a browser-served daemon refuses
            all three with a 409 addressed to an AI agent. The note is inside the
            same block as the items, so the reason is visible in the act of
            reading why they are grey.
          */}
          <HostManagedModelNote variant="inset" />
          <div className="p-1.5">
            <DropdownMenuItem
              className="h-auto rounded-element px-2 py-1.5 text-xs font-medium text-text-default"
              disabled={hostManaged}
              title={hostManaged ? HOST_MANAGED_MODEL_REASON : undefined}
              onClick={hostManaged ? undefined : () => setIsAddModelModalOpen(true)}
            >
              <span>Change model</span>
              <SlidersHorizontal className="ml-auto size-3.5" />
            </DropdownMenuItem>
            <DropdownMenuItem
              className="h-auto rounded-element px-2 py-1.5 text-xs font-medium text-text-default"
              disabled={hostManaged}
              title={hostManaged ? HOST_MANAGED_MODEL_REASON : undefined}
              onClick={hostManaged ? undefined : () => setIsLeadWorkerModalOpen(true)}
            >
              <span>Lead/worker settings</span>
              <SlidersHorizontal className="ml-auto size-3.5" />
            </DropdownMenuItem>
          </div>
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
          initialProvider={effectiveModel?.provider}
          initialModel={effectiveModel?.model}
          setView={setView}
          onClose={() => setIsAddModelModalOpen(false)}
        />
      ) : null}

      {isLeadWorkerModalOpen ? (
        <LeadWorkerSettings isOpen={isLeadWorkerModalOpen} onClose={handleLeadWorkerModalClose} />
      ) : null}
    </div>
  );
}
