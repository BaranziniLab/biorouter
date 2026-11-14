import { useEffect, useId, useRef, useState, type RefObject } from 'react';
import type {
  SessionUsageLimitResponse,
  SetSessionUsageLimitRequest,
  UsageLimitAmounts,
} from '../../api';
import { formatUsdPair } from '../../utils/formatUsd';
import { isBrowserSurface } from '../../utils/surface';
import { Input } from '../ui/input';
import { SegmentedControl } from '../ui/segmented-control';
import {
  formatLimitTokens,
  formatLimitUsd,
  limitInputValue,
  parseLimitInput,
  type LimitUnit,
} from '../usage/usageLimits';
import { USAGE_LIMIT_COPY as COPY } from './copy';
import {
  FooterCardAction,
  FooterCardActions,
  FooterCardDetail,
  FooterCardNote,
  FooterCardSection,
  FooterCardValue,
} from './FooterCard';
import type { SessionUsageLimitController } from './useSessionUsageLimit';

/**
 * The limit in force, as one reading: "$5.00", "500,000 tokens", both, or "No limit". A dollar
 * limit is shown with Show costs off too: that switch hides what was spent, not the limit.
 */
export function limitReading(amounts: UsageLimitAmounts): string {
  const parts: string[] = [];
  if (amounts.maxUsd != null) parts.push(formatLimitUsd(amounts.maxUsd));
  if (amounts.maxTokens != null) parts.push(formatLimitTokens(amounts.maxTokens));
  return parts.length > 0 ? parts.join(' · ') : COPY.noLimit;
}

/** Where the limit in force came from. */
export function limitSourceReading(state: SessionUsageLimitResponse): string {
  if (state.limit.source === 'chat') return COPY.source.chat;
  if (state.limit.source === 'default') return COPY.source.default;
  return state.chatOverride?.mode === 'unlimited' ? COPY.source.unlimited : COPY.source.none;
}

/**
 * "This chat and its subagents: $1.20 of $5.00 used · 12,000 of 500,000 tokens used", or what has
 * been used with no limit. The daemon counts a chat's limit over the chat and every subagent it
 * started, while the card's cost above counts the chat's own requests only (side calls such as the
 * title included; both are read again when the card opens), so the line names what it counts: the
 * two figures differ by the subagents' share. `ownChat` is false in a subagent's chat, whose line
 * is its parent's.
 */
export function chatUsedReading(
  state: SessionUsageLimitResponse,
  showCosts: boolean,
  ownChat = true
): string {
  const { limit, used } = state;
  const parts: string[] = [];
  if (showCosts && limit.maxUsd != null && used.chatUsd != null) {
    parts.push(COPY.used(...formatUsdPair(used.chatUsd, limit.maxUsd)));
  }
  if (limit.maxTokens != null) {
    parts.push(COPY.usedTokens(used.chatTokens, limit.maxTokens));
  }
  const reading =
    parts.length === 0 ? COPY.usedOnly(formatLimitTokens(used.chatTokens)) : parts.join(' · ');
  return ownChat ? COPY.chatUsed(reading) : COPY.parentUsed(reading);
}

/**
 * The month's line when a monthly limit is in force, or `null`. Also `null` when the
 * daemon withheld the month's totals (`monthTokens: null`, a request without the
 * user-action proof): a reading of nothing would read as nothing used.
 */
export function monthUsedReading(
  state: SessionUsageLimitResponse,
  showCosts: boolean
): string | null {
  const { monthly, used } = state;
  if (used.monthTokens == null) return null;
  const parts: string[] = [];
  if (showCosts && monthly.maxUsd != null) {
    parts.push(COPY.used(...formatUsdPair(used.monthUsd ?? 0, monthly.maxUsd)));
  }
  if (monthly.maxTokens != null) {
    parts.push(COPY.usedTokens(used.monthTokens, monthly.maxTokens));
  }
  return parts.length > 0 ? COPY.month(parts.join(' · ')) : null;
}

function share(used: number | null | undefined, limit: number | null | undefined): number {
  if (used == null || limit == null || !(limit > 0)) return 0;
  return used / limit;
}

/** Which limit a warning is about: the chat's when it is at 80% or more, else the month's. */
export function nearLimit(state: SessionUsageLimitResponse): 'chat' | 'month' {
  const { limit, monthly, used } = state;
  const chat = Math.max(share(used.chatTokens, limit.maxTokens), share(used.chatUsd, limit.maxUsd));
  if (chat >= 0.8) return 'chat';
  const month = Math.max(
    share(used.monthTokens, monthly.maxTokens),
    share(used.monthUsd, monthly.maxUsd)
  );
  return month >= 0.8 ? 'month' : 'chat';
}

/**
 * What a pause on an estimate adds: the next request's estimate on the limit's axis and what is
 * left, or that one request alone is past the whole limit. `null` for a limit that is reached, or
 * when the axis of the estimate is not shown (dollars with Show costs off).
 */
export function nextRequestReading(
  state: SessionUsageLimitResponse,
  showCosts: boolean
): string | null {
  const next = state.nextRequest;
  if (!state.paused || !state.pausedByEstimate || !next) return null;
  const month = state.reason === 'month';
  const limits = month ? state.monthly : state.limit;
  const usedTokens = month ? state.used.monthTokens : state.used.chatTokens;
  const usedUsd = month ? state.used.monthUsd : state.used.chatUsd;
  // The axis the estimate passes: tokens when it passes the token limit, else dollars.
  if (
    limits.maxTokens != null &&
    usedTokens != null &&
    usedTokens + next.tokens > limits.maxTokens
  ) {
    if (next.tokens > limits.maxTokens)
      return COPY.nextRequestTooBig(formatLimitTokens(next.tokens));
    return COPY.nextRequest(
      formatLimitTokens(next.tokens),
      formatLimitTokens(Math.max(0, limits.maxTokens - usedTokens))
    );
  }
  if (showCosts && limits.maxUsd != null && next.usd != null) {
    if (next.usd > limits.maxUsd) return COPY.nextRequestTooBig(formatLimitUsd(next.usd));
    return COPY.nextRequest(
      formatLimitUsd(next.usd),
      formatLimitUsd(Math.max(0, limits.maxUsd - (usedUsd ?? 0)))
    );
  }
  return null;
}

export interface UsageLimitSectionProps {
  controller: SessionUsageLimitController;
  /** Whether the card is open: an edit in progress is dropped when it closes. */
  open: boolean;
  /**
   * Settings > App > Show costs: off, no dollar spend is shown. A dollar limit is still offered and
   * shown, because the switch hides the running cost, not the limit.
   */
  showCosts: boolean;
  /**
   * The chat's model has a price, so a new limit starts in dollars. Unpriced, it starts in tokens,
   * and choosing dollars says a dollar limit cannot stop this chat.
   */
  priced: boolean;
  /** Opens Settings > App > Usage, for a chat the monthly limit holds. */
  onOpenUsageSettings?: () => void;
  /** The card's surface: focus lands there when the control that held it goes away. */
  contentRef: RefObject<HTMLDivElement | null>;
}

/**
 * The cost card's Limit section: the limit in force for this chat and where it comes from, what
 * the chat has used against it (and the month, when a monthly limit is set), why the chat is
 * paused, and the three writes a person can make here: set this chat's own limit (one value, in
 * dollars or tokens), no limit for this chat, or back to the default. The monthly and default
 * limits are Settings > App > Usage's.
 *
 * A write this surface can never make says so instead of offering the control: a `biorouter
 * serve` browser holds no proof of a person (SD-8), and a subagent's chat counts toward its
 * parent's limit, which the daemon refuses to change from the child.
 */
export function UsageLimitSection({
  controller,
  open,
  showCosts,
  priced,
  onOpenUsageSettings,
  contentRef,
}: UsageLimitSectionProps) {
  const { state, sessionId, loadFailed, saving, saveError } = controller;
  const fieldId = useId();
  const errorId = useId();
  const inputRef = useRef<HTMLInputElement>(null);
  const [editing, setEditing] = useState(false);
  const [unit, setUnit] = useState<LimitUnit>('tokens');
  const [text, setText] = useState('');
  const [invalid, setInvalid] = useState<false | 'amount' | 'precision'>(false);

  useEffect(() => {
    if (!open) {
      setEditing(false);
      setInvalid(false);
    }
  }, [open]);

  useEffect(() => {
    if (editing) inputRef.current?.focus();
  }, [editing]);

  const keepFocusInCard = () => {
    const content = contentRef.current;
    const active = document.activeElement;
    if (content?.isConnected && (!active || active === document.body)) content.focus();
  };

  const write = async (body: SetSessionUsageLimitRequest) => {
    const saved = await controller.save(body);
    if (saved) {
      setEditing(false);
      setInvalid(false);
    }
    // The pressed control may be gone (a "No limit" that no longer applies, the form's Save).
    requestAnimationFrame(keepFocusInCard);
  };

  const startEditing = () => {
    if (!state) return;
    const own = state.chatOverride?.mode === 'limit' ? state.chatOverride : null;
    const ownUnit: LimitUnit | null =
      own?.maxUsd != null ? 'usd' : own?.maxTokens != null ? 'tokens' : null;
    // Dollars first: people think of a limit as money, so a priced chat starts there.
    const startUnit: LimitUnit = ownUnit ?? (priced ? 'usd' : 'tokens');
    setUnit(startUnit);
    setText(limitInputValue(startUnit === 'usd' ? own?.maxUsd : own?.maxTokens, startUnit));
    setInvalid(false);
    setEditing(true);
  };

  const submit = () => {
    const parsed = parseLimitInput(text, unit);
    if (!parsed.ok) {
      setInvalid(parsed.reason === 'precision' ? 'precision' : 'amount');
      inputRef.current?.focus();
      return;
    }
    void write(
      unit === 'usd'
        ? { mode: 'limit', maxUsd: parsed.value }
        : { mode: 'limit', maxTokens: parsed.value }
    );
  };

  const cancel = () => {
    setEditing(false);
    setInvalid(false);
    requestAnimationFrame(keepFocusInCard);
  };

  const body = () => {
    if (!sessionId) return <FooterCardNote>{COPY.noChatYet}</FooterCardNote>;
    if (!state) {
      return loadFailed ? (
        <FooterCardNote role="alert" tone="danger">
          {COPY.loadFailed}
        </FooterCardNote>
      ) : (
        <FooterCardNote role="status">{COPY.checking}</FooterCardNote>
      );
    }

    const monthLine = monthUsedReading(state, showCosts);
    const nextLine = nextRequestReading(state, showCosts);
    const ownChat = state.billingSessionId === sessionId;
    const browser = isBrowserSurface();
    const canWrite = ownChat && !browser;
    const override = state.chatOverride ?? null;

    return (
      <>
        <FooterCardValue data-testid="usage-limit-value">
          {limitReading(state.limit)}
        </FooterCardValue>
        <FooterCardDetail>{limitSourceReading(state)}</FooterCardDetail>
        <FooterCardDetail>{chatUsedReading(state, showCosts, ownChat)}</FooterCardDetail>
        {monthLine && <FooterCardDetail>{monthLine}</FooterCardDetail>}

        {state.paused && (
          <FooterCardNote role="alert" tone="danger">
            {state.reason === 'month'
              ? state.pausedByEstimate
                ? COPY.pausedMonthNext
                : COPY.pausedMonth
              : state.pausedByEstimate
                ? COPY.pausedChatNext
                : COPY.pausedChat}
          </FooterCardNote>
        )}
        {nextLine && <FooterCardDetail>{nextLine}</FooterCardDetail>}
        {!state.paused && state.warn && (
          <FooterCardNote>
            {nearLimit(state) === 'month' ? COPY.warnMonth : COPY.warnChat}
          </FooterCardNote>
        )}
        {showCosts && state.chatDollarLimitInert && (
          <FooterCardNote>{COPY.chatDollarInert}</FooterCardNote>
        )}
        {showCosts && state.monthDollarLimitInert && (
          <FooterCardNote>{COPY.monthDollarInert}</FooterCardNote>
        )}
        {!ownChat && <FooterCardNote>{COPY.parentChat}</FooterCardNote>}
        {ownChat && browser && <FooterCardNote>{COPY.browserNote}</FooterCardNote>}

        {canWrite && editing && (
          <div className="br-footer-card__limit-form">
            <label htmlFor={fieldId} className="br-footer-card__label">
              {COPY.fieldLabel}
            </label>
            <div className="br-footer-card__limit-fields">
              <Input
                ref={inputRef}
                id={fieldId}
                className="br-footer-card__limit-input"
                inputMode={unit === 'usd' ? 'decimal' : 'numeric'}
                autoComplete="off"
                machine
                value={text}
                aria-label={unit === 'usd' ? COPY.fieldNameDollars : COPY.fieldNameTokens}
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
                  }
                }}
                data-testid="usage-limit-input"
              />
              <SegmentedControl
                aria-label={COPY.unitLabel}
                value={unit}
                onValueChange={(next) => {
                  setUnit(next);
                  setInvalid(false);
                }}
                options={[
                  { value: 'usd', label: COPY.dollars, testId: 'usage-limit-unit-usd' },
                  { value: 'tokens', label: COPY.tokens, testId: 'usage-limit-unit-tokens' },
                ]}
              />
            </div>
            {unit === 'usd' && !priced && (
              <FooterCardNote data-testid="usage-limit-unpriced">
                {COPY.unpricedDollar}
              </FooterCardNote>
            )}
            {invalid && (
              <FooterCardNote id={errorId} role="alert" tone="danger">
                {unit === 'tokens'
                  ? COPY.invalidTokens
                  : invalid === 'precision'
                    ? COPY.invalidUsdPrecision
                    : COPY.invalidUsd}
              </FooterCardNote>
            )}
            <FooterCardActions>
              <FooterCardAction disabled={saving} onClick={submit}>
                {COPY.save}
              </FooterCardAction>
              <FooterCardAction disabled={saving} onClick={cancel}>
                {COPY.cancel}
              </FooterCardAction>
            </FooterCardActions>
          </div>
        )}

        {canWrite && !editing && (
          <FooterCardActions>
            <FooterCardAction disabled={saving} onClick={startEditing}>
              {override?.mode === 'limit' ? COPY.changeLimit : COPY.setLimit}
            </FooterCardAction>
            {override?.mode !== 'unlimited' && (
              <FooterCardAction
                disabled={saving}
                aria-label={COPY.noLimitForChatName}
                onClick={() => void write({ mode: 'unlimited' })}
              >
                {COPY.noLimitForChat}
              </FooterCardAction>
            )}
            {override !== null && (
              <FooterCardAction disabled={saving} onClick={() => void write({ mode: 'inherit' })}>
                {COPY.useDefault}
              </FooterCardAction>
            )}
          </FooterCardActions>
        )}

        {state.paused && state.reason === 'month' && onOpenUsageSettings && (
          <FooterCardActions>
            <FooterCardAction onClick={onOpenUsageSettings}>{COPY.openSettings}</FooterCardAction>
          </FooterCardActions>
        )}

        {saveError && (
          <FooterCardNote role="alert" tone="danger">
            {saveError === 'needUser' ? COPY.needUser : COPY.saveFailed}
          </FooterCardNote>
        )}
      </>
    );
  };

  return (
    <FooterCardSection label={COPY.section}>
      <div data-testid="usage-limit-section" className="br-footer-card__limit">
        {body()}
      </div>
    </FooterCardSection>
  );
}
