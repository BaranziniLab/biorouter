import { SlidersHorizontal, Brain } from '../../../icons/app-icons';
import React, { useEffect, useState } from 'react';
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
import { useCurrentModelInfo } from '../../../BaseChat';
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
  const currentModelInfo = useCurrentModelInfo();
  const { read, getProviders } = useConfig();
  const [displayProvider, setDisplayProvider] = useState<string | null>(null);
  const [displayModelName, setDisplayModelName] = useState<string>('Select Model');
  const [isAddModelModalOpen, setIsAddModelModalOpen] = useState(false);
  const [isLeadWorkerModalOpen, setIsLeadWorkerModalOpen] = useState(false);
  const [isLeadWorkerActive, setIsLeadWorkerActive] = useState(false);
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

  /**
   * Issue #56 Gate B. What actually runs in THIS chat: the pin when there is
   * one, the app's global selection otherwise.
   *
   * ⚠ The chat's own binding outranks lead/worker below. Such a chat runs the
   * single provider its session row names, so a lead/worker pair configured
   * globally is not what answers here, and labelling the chip `(lead)` would
   * name a mechanism that is not in play.
   */
  const effectiveProvider = effectiveModel?.provider ?? currentProvider;

  // Check if lead/worker mode is active
  useEffect(() => {
    const checkLeadWorker = async () => {
      try {
        const leadModel = await read('BIOROUTER_LEAD_MODEL', false);
        setIsLeadWorkerActive(!!leadModel);
      } catch (error) {
        console.error('Error checking lead model:', error);
        setIsLeadWorkerActive(false);
      }
    };
    checkLeadWorker();
  }, [read]);

  // Refresh lead/worker status when modal closes
  const handleLeadWorkerModalClose = () => {
    setIsLeadWorkerModalOpen(false);
    // Refresh the lead/worker status after modal closes
    const checkLeadWorker = async () => {
      try {
        const leadModel = await read('BIOROUTER_LEAD_MODEL', false);
        const currentModel = await read('BIOROUTER_MODEL', false);
        setIsLeadWorkerActive(!!leadModel);
        setLeadModelName((leadModel as string) || '');
        setCurrentActiveModel((currentModel as string) || '');
      } catch (error) {
        console.error('Error checking lead model after modal close:', error);
        setIsLeadWorkerActive(false);
      }
    };
    checkLeadWorker();
  };

  // Since currentModelInfo.mode is not working, let's determine mode differently
  // We'll need to get the lead model and compare it with the current model
  const [leadModelName, setLeadModelName] = useState<string>('');
  const [currentActiveModel, setCurrentActiveModel] = useState<string>('');

  // Get lead model name and current model for comparison
  useEffect(() => {
    const getModelInfo = async () => {
      try {
        const leadModel = await read('BIOROUTER_LEAD_MODEL', false);
        const currentModel = await read('BIOROUTER_MODEL', false);
        setLeadModelName((leadModel as string) || '');
        setCurrentActiveModel((currentModel as string) || '');
      } catch (error) {
        console.error('Error getting model info:', error);
      }
    };
    getModelInfo();
  }, [read]);

  // Determine the mode based on which model is currently active
  const modelMode = isLeadWorkerActive
    ? currentActiveModel === leadModelName
      ? 'lead'
      : 'worker'
    : undefined;

  // Determine which model to display - activeModel takes priority when lead/worker is active
  const displayModel =
    effectiveModel?.model ??
    (isLeadWorkerActive && currentModelInfo?.model
      ? currentModelInfo.model
      : currentModel || providerDefaultModel || displayModelName);
  const fullModelLabel =
    !effectiveModel && isLeadWorkerActive && modelMode
      ? `${displayModel} (${modelMode})`
      : displayModel;
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
   * The two names in the dropdown's "Current model" block, pinned binding first.
   *
   * Layered here rather than inside the effects that fill `displayModelName` /
   * `displayProvider`: those two describe the app's GLOBAL selection, which is
   * still the right answer for every chat that is not pinned, and having two
   * effects race to own one state was how the earlier drafts of this went
   * wrong.
   */
  const shownModelName = effectiveModel?.model ?? displayModelName;
  const shownProviderName = effectiveModel
    ? (pinnedProviderName ?? effectiveModel.provider)
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
   * ⚠ A plain button rather than a disabled dropdown: the menu's items are
   * "Change Model" and "Lead/Worker Settings", both of which read as adjustments
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
            <div className="text-sm font-medium text-text-default">Current model</div>
            <div className="mt-0.5 text-supporting leading-4 text-text-muted">
              {shownModelName}
              {shownProviderName && ` · ${shownProviderName}`}
            </div>
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
              <span>Change Model</span>
              <SlidersHorizontal className="ml-auto size-3.5" />
            </DropdownMenuItem>
            <DropdownMenuItem
              className="h-auto rounded-element px-2 py-1.5 text-xs font-medium text-text-default"
              disabled={hostManaged}
              title={hostManaged ? HOST_MANAGED_MODEL_REASON : undefined}
              onClick={hostManaged ? undefined : () => setIsLeadWorkerModalOpen(true)}
            >
              <span>Lead/Worker Settings</span>
              <SlidersHorizontal className="ml-auto size-3.5" />
            </DropdownMenuItem>
          </div>
        </DropdownMenuContent>
      </DropdownMenu>

      {isAddModelModalOpen ? (
        <SwitchModelModal
          sessionId={sessionId}
          privacyTier={privacyTier}
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
