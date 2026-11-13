import type { ReactNode } from 'react';
import { Lock } from '../icons/app-icons';
import {
  decisionModelLabel,
  type DecisionModelEntry,
  type DecisionModelRef,
  type DecisionTier,
} from '../../utils/sessionDecisions';
import { decisionModelPlace } from './decisionWords';

/**
 * One row of a decision model picker (the Monitors editor and the phase gate
 * editor), so the two pickers read one way (monitors spec 1.5, R3, R4):
 *
 * - one line: the alias in default ink, then muted secondary text after an 8px gap,
 *   wrapping under the name only when it cannot fit;
 * - a private model draws the `Lock` shape with a visually hidden "Private model"
 *   ("Private is a shape, not a tint"); the words "Private" and "Public" leave the row;
 * - where it runs (`decisionModelPlace`), and the window only when it is a risk
 *   (under 8,000 tokens);
 * - a model this chat may not use is disabled and says why: the R4 line for a public
 *   model in a private chat, whatever the daemon said, else the daemon's own reason
 *   with backticks removed (R5);
 * - the selected value (`context: 'value'`) is the name alone, except the default row's,
 *   which says what the default resolves to when that is known: "Default model (tev1:0.8b)".
 *   In the menu the model sits in the secondary text; once picked, the name is all that
 *   shows, and "Default model" alone left the gate editor's person guessing which model ran.
 */

export const DECISION_MODEL_OPTION_COPY = {
  defaultModel: 'Default model',
  defaultModelHint: "the chat's decision model",
  /** The default row as the selected value, when the model it resolves to is known. */
  defaultModelNamed: (alias: string) => `Default model (${alias})`,
  privateModel: 'Private model',
  publicRefused: "Public model: it can't read a private chat.",
} as const;

/** A window at or above this many tokens is not named: a person cannot act on it. */
export const SMALL_WINDOW_TOKENS = 8_000;

/** `512 tokens`, `2K tokens` (2,048), `33K tokens`. `null` for no usable number. */
export function formatWindow(tokens: number | null | undefined): string | null {
  if (tokens === null || tokens === undefined || !Number.isFinite(tokens) || tokens <= 0) {
    return null;
  }
  if (tokens < 1000) return `${Math.round(tokens)} tokens`;
  return `${Math.round(tokens / 1000)}K tokens`;
}

/** The window as a picker row names it: only under {@link SMALL_WINDOW_TOKENS}. */
export function riskyWindow(tokens: number | null | undefined): string | null {
  return typeof tokens === 'number' && tokens < SMALL_WINDOW_TOKENS ? formatWindow(tokens) : null;
}

export interface DecisionModelOption {
  /** `provider/model`, or `''` for the chat's default model. */
  value: string;
  /** What the row reads as, spoken: `tev1:0.8b, Private model, this computer · 2K tokens`. */
  label: string;
  /** The alias, or "Default model". Also the selected value's text, unless `selectedName`. */
  name: string;
  /** The selected value's text when it differs from `name`: "Default model (tev1:0.8b)". */
  selectedName?: string;
  /** Muted text after the name: the place and window, the default's model, or the reason. */
  secondary: string | null;
  /** Draws the lock with the hidden "Private model". Never on a disabled row. */
  isPrivate: boolean;
  /** `null` for the chat's default model. */
  model: DecisionModelRef | null;
  /** Why this chat may not use it; a row with a reason is disabled. */
  reason: string | null;
}

/** The picker's key for a model: `provider/model`, `''` for the default. */
export function decisionModelKey(
  model: Pick<DecisionModelRef, 'provider' | 'model'> | null
): string {
  return model ? `${model.provider}/${model.model}` : '';
}

/** The daemon's reason as a person reads it: no backticks (R5). */
function plainReason(reason: string): string {
  return reason.replace(/`/g, '').trim();
}

/**
 * Why the chat may not use `entry`, or `null`: the R4 line when a private chat meets
 * a public model (it overrides the daemon's agent-facing sentence), else the
 * daemon's own reason.
 */
export function decisionModelReason(
  entry: Pick<DecisionModelEntry, 'tier' | 'unavailableReason'>,
  chatTier: DecisionTier | null
): string | null {
  if (chatTier === 'private' && entry.tier === 'public') {
    return DECISION_MODEL_OPTION_COPY.publicRefused;
  }
  const reason = entry.unavailableReason ? plainReason(entry.unavailableReason) : '';
  return reason || null;
}

function spoken(name: string, isPrivate: boolean, secondary: string | null): string {
  return [name, isPrivate ? DECISION_MODEL_OPTION_COPY.privateModel : null, secondary]
    .filter(Boolean)
    .join(', ');
}

export interface DecisionModelOptionsInput {
  models: readonly DecisionModelEntry[];
  chatTier: DecisionTier | null;
  /** Why this chat may use no decision model at all; disables the default row. */
  chatRefusal?: string | null;
  /** The chat's stored default model, named on the default row when set. */
  defaultModel?: Pick<DecisionModelRef, 'provider' | 'model'> | null;
  /** A `biorouter serve` browser (default: the surface the renderer is on). */
  browser?: boolean;
}

/** The picker's rows: the chat's default first, then each listed model. */
export function decisionModelOptions({
  models,
  chatTier,
  chatRefusal = null,
  defaultModel = null,
  browser,
}: DecisionModelOptionsInput): DecisionModelOption[] {
  const defaultReason = chatRefusal ? plainReason(chatRefusal) || null : null;
  const defaultAlias = decisionModelLabel(defaultModel);
  const defaultSecondary =
    defaultReason ?? defaultAlias ?? DECISION_MODEL_OPTION_COPY.defaultModelHint;
  const rows: DecisionModelOption[] = [
    {
      value: '',
      name: DECISION_MODEL_OPTION_COPY.defaultModel,
      ...(defaultAlias
        ? { selectedName: DECISION_MODEL_OPTION_COPY.defaultModelNamed(defaultAlias) }
        : {}),
      label: spoken(DECISION_MODEL_OPTION_COPY.defaultModel, false, defaultSecondary),
      secondary: defaultSecondary,
      isPrivate: false,
      model: null,
      reason: defaultReason,
    },
  ];
  for (const entry of models) {
    const model: DecisionModelRef = { provider: entry.provider, model: entry.model, adapter: null };
    const name = decisionModelLabel(model) || entry.displayName;
    const reason = decisionModelReason(entry, chatTier);
    const place = decisionModelPlace(entry.provider, {
      local: entry.local,
      tier: entry.tier,
      browser,
    });
    const where = [place, riskyWindow(entry.contextTokens)].filter(Boolean).join(' · ') || null;
    const secondary = reason ?? where;
    const isPrivate = !reason && entry.tier === 'private';
    rows.push({
      value: decisionModelKey(model),
      name,
      label: spoken(name, isPrivate, secondary),
      secondary,
      isPrivate,
      model,
      reason,
    });
  }
  return rows;
}

/** `isOptionDisabled` for the picker's `Select`. */
export function isDecisionModelOptionDisabled(raw: unknown): boolean {
  return !!(raw as DecisionModelOption).reason;
}

/**
 * `formatOptionLabel` for the picker's `Select`: in the menu the name, then the
 * muted secondary text 8px after it (wrapping under it only when it cannot fit);
 * as the selected value, the name alone. Separators the eye does not need are
 * visually hidden, so the row reads "tev1:0.8b, Private model, this computer · 2K tokens".
 */
export function formatDecisionModelOption(
  raw: unknown,
  meta?: { context?: 'menu' | 'value' }
): ReactNode {
  const option = raw as DecisionModelOption;
  if (meta?.context === 'value') {
    return <span className="truncate">{option.selectedName ?? option.name}</span>;
  }
  return (
    <span
      className="flex min-w-0 flex-wrap items-baseline gap-x-2"
      data-testid="decision-model-option"
    >
      <span className="min-w-0 text-text-default">{option.name}</span>
      {option.secondary || option.isPrivate ? (
        <span className="inline-flex min-w-0 items-center gap-1 text-supporting text-text-muted">
          <span className="sr-only">, </span>
          {option.isPrivate ? (
            <>
              <Lock aria-hidden="true" className="size-3 shrink-0" />
              <span className="sr-only">{`${DECISION_MODEL_OPTION_COPY.privateModel}, `}</span>
            </>
          ) : null}
          {option.secondary ? <span className="min-w-0">{option.secondary}</span> : null}
        </span>
      ) : null}
    </span>
  );
}
