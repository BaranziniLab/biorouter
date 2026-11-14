import { useCallback, useState, useSyncExternalStore } from 'react';
import type {
  DecisionModelEntry,
  DecisionModelRef,
  DecisionSettings,
} from '../../utils/sessionDecisions';

/**
 * DR-33 (design §4) — the decision settings a chat that has not been sent yet will
 * start with: the Monitors dialog's draft mode, from either composer (Home and a
 * blank chat), before a session row exists.
 *
 * Kept beside the held chat model (`settings/models/pendingChatModel.ts`) and on its
 * terms: renderer memory, keyed by the composer's identity (a chat TAB, or Home's
 * draft key), so the draft outlives a tab switch and a trip to Settings, and is
 * never written to disk. The entry goes when the chat its first message started
 * exists (the start carried it as `decisionSettings`), or when the person empties
 * it. A tab closed while unsent leaves one small object behind that is never read
 * again, as the held model does.
 *
 * `acceptedFor` is the chat model under which the person saw "This chat will be
 * public: …" and saved anyway (`chatModelKey`). Send passes
 * `acceptPublicDecisionModels` only while it still names the model the chat starts
 * on: a model changed after the draft was saved re-asks at Send, from the daemon's
 * own 409 question, because the effect the person accepted was worked out for the
 * other model.
 */
export interface DraftDecisionSettings {
  settings: DecisionSettings;
  /** The chat model key the person accepted a public chat under, or `null`. */
  acceptedFor: string | null;
}

/** `provider/model`, the key a draft's acceptance is held under; `null` with either missing. */
export function chatModelKey(
  provider: string | null | undefined,
  model: string | null | undefined
) {
  return provider && model ? `${provider}/${model}` : null;
}

/** A draft with nothing in it is no draft: the composer sends no `decisionSettings`. */
export function isEmptyDraft(settings: DecisionSettings | null | undefined): boolean {
  return !settings || (settings.monitors.length === 0 && !settings.defaultModel);
}

/**
 * The listed models a draft's settings would make the chat public with: the
 * default decision model, and each enabled monitor's model (or, for a monitor with
 * none, the default), wherever the daemon marked the row `makesPublic`. Each model
 * once, in the order first named. The daemon computes the marking; this only reads
 * which of the draft's names carry it.
 */
export function loweringModels(
  settings: DecisionSettings | null | undefined,
  models: readonly DecisionModelEntry[]
): DecisionModelEntry[] {
  if (!settings) return [];
  const found: DecisionModelEntry[] = [];
  const consider = (ref: { provider: string; model: string } | null) => {
    if (!ref) return;
    const entry = models.find(
      (candidate) =>
        candidate.provider === ref.provider &&
        (candidate.model === ref.model || candidate.displayName === ref.model)
    );
    if (entry?.chatEffect === 'makesPublic' && !found.includes(entry)) found.push(entry);
  };
  consider(settings.defaultModel);
  for (const monitor of settings.monitors) {
    if (monitor.enabled) consider(monitor.model ?? settings.defaultModel);
  }
  return found;
}

/**
 * Every decision model a draft names, once each: its default and each monitor's, a
 * monitor that is off included. A new chat's dictation sends these
 * (`new_chat_decision_models`), so the daemon judges the chat as the start will make it
 * (DR-33). Which of them make the chat public is the daemon's decision, made with the
 * same check the start runs; a monitor that is off counts there too, which can only
 * make the dictation more public. Empty for no draft.
 */
export function draftDecisionModels(
  settings: DecisionSettings | null | undefined
): DecisionModelRef[] {
  if (isEmptyDraft(settings) || !settings) return [];
  const found: DecisionModelRef[] = [];
  const add = (ref: DecisionModelRef | null) => {
    if (!ref) return;
    const seen = found.some(
      (known) =>
        known.provider === ref.provider &&
        known.model === ref.model &&
        known.adapter === ref.adapter
    );
    if (!seen) found.push(ref);
  };
  add(settings.defaultModel);
  for (const monitor of settings.monitors) add(monitor.model);
  return found;
}

const held = new Map<string, DraftDecisionSettings>();
const listeners = new Set<() => void>();

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function notify() {
  for (const listener of [...listeners]) listener();
}

/** Hold `draft` for the unsent chat under `key`, or drop it with `null`. */
export function holdDraftDecisionSettings(key: string, draft: DraftDecisionSettings | null): void {
  if (draft && !isEmptyDraft(draft.settings)) {
    if (held.get(key) === draft) return;
    held.set(key, draft);
  } else {
    if (!held.has(key)) return;
    held.delete(key);
  }
  notify();
}

/** The draft held for the unsent chat under `key`, or `null`. */
export function heldDraftDecisionSettings(key: string): DraftDecisionSettings | null {
  return held.get(key) ?? null;
}

/** Tests only: the map is module state and outlives a `cleanup()`. */
export function __resetDraftDecisionSettingsForTests() {
  held.clear();
  listeners.clear();
}

/**
 * The draft held under `key`, and its setter. With no key (a composer outside the
 * tabbed shell) it lives in this component's own state, as the held model does.
 */
export function useDraftDecisionSettings(
  key: string | undefined
): [DraftDecisionSettings | null, (draft: DraftDecisionSettings | null) => void] {
  const [local, setLocal] = useState<DraftDecisionSettings | null>(null);
  const stored = useSyncExternalStore(subscribe, () =>
    key ? heldDraftDecisionSettings(key) : null
  );
  const set = useCallback(
    (draft: DraftDecisionSettings | null) => {
      if (key) holdDraftDecisionSettings(key, draft);
      else setLocal(draft && !isEmptyDraft(draft.settings) ? draft : null);
    },
    [key]
  );
  return [key ? stored : local, set];
}

/**
 * What a composer's start sends for its draft: `decisionSettings` when there is
 * one, and `acceptPublicDecisionModels` only while the person's acceptance names
 * the model the chat starts on.
 */
export function draftStartFields(
  draft: DraftDecisionSettings | null,
  startsOn: string | null
): { decisionSettings: DecisionSettings; acceptPublicDecisionModels: boolean } | null {
  if (!draft || isEmptyDraft(draft.settings)) return null;
  return {
    decisionSettings: draft.settings,
    acceptPublicDecisionModels: !!startsOn && draft.acceptedFor === startsOn,
  };
}
