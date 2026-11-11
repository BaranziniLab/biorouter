import { useEffect, useId, useRef, useState } from 'react';
import {
  putUsageLimits,
  type UpdateUsageLimitsRequest,
  type UsageLimitsResponse,
} from '../../../api';
import { formatUsdPair } from '../../../utils/formatUsd';
import { isBrowserSurface } from '../../../utils/surface';
import { userActionHeaders } from '../../../utils/userAction';
import { Button } from '../../ui/button';
import { Input } from '../../ui/input';
import { SegmentedControl } from '../../ui/segmented-control';
import { SettingRow } from '../../ui/setting-row';
import { settingsLimitsCopy as copy } from '../../usage/copy';
import '../../usage/usage.css';
import {
  announceUsageLimitsChanged,
  formatLimitTokens,
  formatLimitUsd,
  limitInputValue,
  parseLimitInput,
  type LimitUnit,
} from '../../usage/usageLimits';

/** The four configured limits, as `GET /usage/summary` and `PUT /usage/limits` carry them. */
export type ConfiguredLimits = Required<{
  [K in keyof UsageLimitsResponse]: number | null;
}>;

/**
 * Where the month stands against its limit, as the reply loop reads it (`GET /usage/summary`'s
 * `monthlyReached`, `monthlyNear`, `monthlyUsedTokens`, `monthlyUsedUsd`).
 */
export interface MonthProgress {
  usedTokens: number | null;
  usedUsd: number | null;
  reached: boolean;
  near: boolean;
}

type Which = 'monthly' | 'defaultPerChat';

const FIELDS: Record<Which, { tokens: keyof ConfiguredLimits; usd: keyof ConfiguredLimits }> = {
  monthly: { tokens: 'monthlyTokenLimit', usd: 'monthlyDollarLimit' },
  defaultPerChat: { tokens: 'defaultSessionTokenLimit', usd: 'defaultSessionDollarLimit' },
};

/** A limit in force is above 0; anything else reads as unlimited, as the daemon reads it. */
function inForce(value: number | null | undefined): number | null {
  return value != null && Number.isFinite(value) && value > 0 ? value : null;
}

/**
 * "No limit", "$50.00", "5,000,000 tokens", or both. A dollar limit is shown with Show costs off
 * too: that switch hides what was spent, not the limit a person set.
 */
export function configuredLimitReading(limits: ConfiguredLimits, which: Which): string {
  const usd = inForce(limits[FIELDS[which].usd]);
  const tokens = inForce(limits[FIELDS[which].tokens]);
  const parts: string[] = [];
  if (usd !== null) parts.push(formatLimitUsd(usd));
  if (tokens !== null) parts.push(formatLimitTokens(tokens));
  return parts.length > 0 ? parts.join(' · ') : copy.noLimit;
}

/**
 * The monthly limit's progress lines: what was used against each limit in force ("$4.01 of $4.02
 * used this month"; dollars only while Show costs is on), or `[]` with no monthly limit or no
 * reading.
 */
export function monthProgressLines(
  limits: ConfiguredLimits,
  progress: MonthProgress | null | undefined,
  showCosts: boolean
): string[] {
  if (!progress) return [];
  const lines: string[] = [];
  const usd = inForce(limits.monthlyDollarLimit);
  const tokens = inForce(limits.monthlyTokenLimit);
  if (showCosts && usd !== null) {
    lines.push(copy.monthUsed(...formatUsdPair(progress.usedUsd ?? 0, usd)));
  }
  if (tokens !== null && progress.usedTokens != null) {
    lines.push(copy.monthUsedTokens(progress.usedTokens, tokens));
  }
  return lines;
}

/**
 * The write for one row. A value in one unit clears the other unit's limit, so the row holds the
 * one limit the person set. "No limit" clears both.
 */
export function limitsUpdate(
  which: Which,
  next: { unit: LimitUnit; value: number } | null
): UpdateUsageLimitsRequest {
  const { tokens, usd } = FIELDS[which];
  if (next === null) return { [tokens]: null, [usd]: null };
  if (next.unit === 'usd') return { [usd]: next.value, [tokens]: null };
  return { [tokens]: next.value, [usd]: null };
}

interface LimitRowProps {
  which: Which;
  label: string;
  help: string;
  /** `null` while the first read runs: the value is pending and Change waits. */
  limits: ConfiguredLimits | null;
  showCosts: boolean;
  /** The current model has a price, so a new limit starts in dollars. */
  priced: boolean;
  /** The monthly row's progress against its limit. */
  progress?: MonthProgress | null;
  disabled: boolean;
  onSave: (body: UpdateUsageLimitsRequest) => Promise<'saved' | 'needUser' | 'failed'>;
}

function LimitRow({
  which,
  label,
  help,
  limits,
  showCosts,
  priced,
  progress,
  disabled,
  onSave,
}: LimitRowProps) {
  const fieldId = useId();
  const errorId = useId();
  const inputRef = useRef<HTMLInputElement>(null);
  const changeRef = useRef<HTMLButtonElement>(null);
  const [editing, setEditing] = useState(false);
  const [unit, setUnit] = useState<LimitUnit>('usd');
  const [text, setText] = useState('');
  const [invalid, setInvalid] = useState<false | 'amount' | 'precision'>(false);
  const [saving, setSaving] = useState(false);
  const [failure, setFailure] = useState<'needUser' | 'failed' | null>(null);

  useEffect(() => {
    if (editing) inputRef.current?.focus();
  }, [editing]);

  const startEditing = () => {
    if (!limits) return;
    const usd = inForce(limits[FIELDS[which].usd]);
    const tokens = inForce(limits[FIELDS[which].tokens]);
    // Dollars first: the stored unit, else dollars for a priced model.
    const startUnit: LimitUnit =
      usd !== null ? 'usd' : tokens !== null ? 'tokens' : priced ? 'usd' : 'tokens';
    setUnit(startUnit);
    setText(limitInputValue(startUnit === 'usd' ? usd : tokens, startUnit));
    setInvalid(false);
    setFailure(null);
    setEditing(true);
  };

  const finish = () => {
    setEditing(false);
    setInvalid(false);
    requestAnimationFrame(() => changeRef.current?.focus());
  };

  const write = async (next: { unit: LimitUnit; value: number } | null) => {
    setSaving(true);
    setFailure(null);
    try {
      const outcome = await onSave(limitsUpdate(which, next));
      if (outcome === 'saved') finish();
      else setFailure(outcome);
    } finally {
      setSaving(false);
    }
  };

  const submit = () => {
    const parsed = parseLimitInput(text, unit);
    if (!parsed.ok) {
      setInvalid(parsed.reason === 'precision' ? 'precision' : 'amount');
      inputRef.current?.focus();
      return;
    }
    void write({ unit, value: parsed.value });
  };

  const progressLines = limits ? monthProgressLines(limits, progress, showCosts) : [];

  return (
    <div data-testid={`usage-limit-row-${which}`}>
      <SettingRow
        label={label}
        help={help}
        value={limits ? configuredLimitReading(limits, which) : copy.pending}
        status={disabled ? copy.browserNote : undefined}
      >
        <Button
          ref={changeRef}
          type="button"
          size="sm"
          variant="secondary"
          disabled={disabled || editing || !limits}
          aria-label={copy.changeName(label)}
          onClick={startEditing}
        >
          {copy.change}
        </Button>
      </SettingRow>
      {(progressLines.length > 0 || progress?.reached || progress?.near) && (
        <div
          className="flex flex-col gap-0.5 px-3 pb-2 text-supporting text-text-muted"
          data-testid={`usage-limit-progress-${which}`}
          data-limit-state={progress?.reached ? 'paused' : progress?.near ? 'warn' : undefined}
        >
          {progressLines.map((line) => (
            <p key={line}>{line}</p>
          ))}
          {progress?.reached && (
            <p role="status" className="text-text-danger">
              {copy.monthReached}
            </p>
          )}
          {!progress?.reached && progress?.near && <p role="status">{copy.monthNear}</p>}
        </div>
      )}
      {editing && (
        <div className="br-usage-limit-editor" data-testid={`usage-limit-editor-${which}`}>
          <label htmlFor={fieldId} className="sr-only">
            {copy.amountLabel(label)}
          </label>
          <div className="br-usage-limit-editor__fields">
            <Input
              ref={inputRef}
              id={fieldId}
              className="br-usage-limit-editor__input"
              inputMode={unit === 'usd' ? 'decimal' : 'numeric'}
              autoComplete="off"
              machine
              value={text}
              aria-label={copy.amountName(label, unit === 'usd' ? copy.dollars : copy.tokens)}
              aria-invalid={invalid ? true : undefined}
              aria-describedby={invalid ? errorId : undefined}
              onChange={(event) => {
                setText(event.target.value);
                setInvalid(false);
              }}
              onKeyDown={(event) => {
                if (event.key === 'Enter') {
                  event.preventDefault();
                  submit();
                } else if (event.key === 'Escape') {
                  event.preventDefault();
                  finish();
                }
              }}
            />
            <SegmentedControl
              aria-label={copy.unitLabel}
              value={unit}
              onValueChange={(next) => {
                setUnit(next);
                setInvalid(false);
              }}
              options={[
                { value: 'usd', label: copy.dollars, testId: `usage-limit-${which}-usd` },
                { value: 'tokens', label: copy.tokens, testId: `usage-limit-${which}-tokens` },
              ]}
            />
          </div>
          {unit === 'usd' && !priced && (
            <p
              className="text-supporting text-text-muted"
              data-testid={`usage-limit-${which}-unpriced`}
            >
              {copy.unpricedDollar}
            </p>
          )}
          {invalid && (
            <p id={errorId} role="alert" className="text-supporting text-text-danger">
              {unit === 'tokens'
                ? copy.invalidTokens
                : invalid === 'precision'
                  ? copy.invalidUsdPrecision
                  : copy.invalidUsd}
            </p>
          )}
          {failure && (
            <p role="alert" className="text-supporting text-text-danger">
              {failure === 'needUser' ? copy.needUser : copy.saveFailed}
            </p>
          )}
          <div className="br-usage-limit-editor__actions">
            <Button type="button" size="sm" disabled={saving} onClick={submit}>
              {copy.save}
            </Button>
            <Button
              type="button"
              size="sm"
              variant="secondary"
              disabled={saving}
              aria-label={copy.removeLimitName(label)}
              onClick={() => void write(null)}
            >
              {copy.removeLimit}
            </Button>
            <Button type="button" size="sm" variant="ghost" disabled={saving} onClick={finish}>
              {copy.cancel}
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}

export interface UsageLimitsRowsProps {
  /** `null` while Settings' first read of the month runs. */
  limits: ConfiguredLimits | null;
  showCosts: boolean;
  /** The current model has a price, so a new limit starts in dollars (default `true`). */
  priced?: boolean;
  /** Where the month stands against the monthly limit, or `null` before the first read. */
  monthProgress?: MonthProgress | null;
  /** The limits as stored after a write, so the section and its gauges read them at once. */
  onSaved: (limits: ConfiguredLimits) => void;
}

/**
 * Settings > App > Usage's Limits group: the monthly limit and the default limit per chat, each
 * "No limit" until set, in dollars (first, for a priced model) or tokens, whether or not Show
 * costs is on. Under the monthly limit, what the month has used against it and whether it is near
 * or reached, as the reply loop reads it. Saved through `PUT /usage/limits` with the person's
 * proof; a `biorouter serve` browser has none, so its rows say where to change them instead of
 * offering a control that cannot work (SD-8). A save tells every open composer to read its chat's
 * limit again (`announceUsageLimitsChanged`).
 */
export function UsageLimitsRows({
  limits,
  showCosts,
  priced = true,
  monthProgress,
  onSaved,
}: UsageLimitsRowsProps) {
  const browser = isBrowserSurface();

  const save = async (body: UpdateUsageLimitsRequest): Promise<'saved' | 'needUser' | 'failed'> => {
    try {
      const result = await putUsageLimits({
        body,
        headers: await userActionHeaders(),
        throwOnError: false,
      });
      if (!result.data) return result.response?.status === 403 ? 'needUser' : 'failed';
      onSaved({
        monthlyTokenLimit: inForce(result.data.monthlyTokenLimit),
        monthlyDollarLimit: inForce(result.data.monthlyDollarLimit),
        defaultSessionTokenLimit: inForce(result.data.defaultSessionTokenLimit),
        defaultSessionDollarLimit: inForce(result.data.defaultSessionDollarLimit),
      });
      announceUsageLimitsChanged();
      return 'saved';
    } catch {
      return 'failed';
    }
  };

  return (
    <section className="flex flex-col gap-1" data-testid="usage-limits">
      <h3 className="px-3 pt-3 text-caps text-text-muted">{copy.groupTitle}</h3>
      <LimitRow
        which="monthly"
        label={copy.monthly}
        help={copy.monthlyHelp}
        limits={limits}
        showCosts={showCosts}
        priced={priced}
        progress={monthProgress}
        disabled={browser}
        onSave={save}
      />
      <LimitRow
        which="defaultPerChat"
        label={copy.defaultPerChat}
        help={copy.defaultPerChatHelp}
        limits={limits}
        showCosts={showCosts}
        priced={priced}
        disabled={browser}
        onSave={save}
      />
    </section>
  );
}
