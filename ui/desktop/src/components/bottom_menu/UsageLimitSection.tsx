import { useEffect, useId, useRef, useState, type RefObject } from 'react';
import type {
  SessionUsageLimitResponse,
  SetSessionUsageLimitRequest,
  UsageLimitAmounts,
} from '../../api';
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

/** The limit in force, as one reading: "$5.00", "500,000 tokens", both, or "No limit". */
export function limitReading(amounts: UsageLimitAmounts, showCosts: boolean): string {
  const parts: string[] = [];
  if (amounts.maxUsd != null) {
    parts.push(showCosts ? formatLimitUsd(amounts.maxUsd) : COPY.dollarLimitHidden);
  }
  if (amounts.maxTokens != null) parts.push(formatLimitTokens(amounts.maxTokens));
  return parts.length > 0 ? parts.join(' · ') : COPY.noLimit;
}

/** Where the limit in force came from. */
export function limitSourceReading(state: SessionUsageLimitResponse): string {
  if (state.limit.source === 'chat') return COPY.source.chat;
  if (state.limit.source === 'default') return COPY.source.default;
  return state.chatOverride?.mode === 'unlimited' ? COPY.source.unlimited : COPY.source.none;
}

/** "$1.20 of $5.00 used · 12,000 of 500,000 tokens used", or what has been used with no limit. */
export function chatUsedReading(state: SessionUsageLimitResponse, showCosts: boolean): string {
  const { limit, used } = state;
  const parts: string[] = [];
  if (showCosts && limit.maxUsd != null && used.chatUsd != null) {
    parts.push(COPY.used(formatLimitUsd(used.chatUsd), formatLimitUsd(limit.maxUsd)));
  }
  if (limit.maxTokens != null) {
    parts.push(COPY.usedTokens(used.chatTokens, limit.maxTokens));
  }
  if (parts.length === 0) return COPY.usedOnly(formatLimitTokens(used.chatTokens));
  return parts.join(' · ');
}

/** The month's line when a monthly limit is in force, or `null`. */
export function monthUsedReading(
  state: SessionUsageLimitResponse,
  showCosts: boolean
): string | null {
  const { monthly, used } = state;
  const parts: string[] = [];
  if (showCosts && monthly.maxUsd != null) {
    const spent = used.monthUsd != null ? formatLimitUsd(used.monthUsd) : formatLimitUsd(0);
    parts.push(COPY.used(spent, formatLimitUsd(monthly.maxUsd)));
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

export interface UsageLimitSectionProps {
  controller: SessionUsageLimitController;
  /** Whether the card is open: an edit in progress is dropped when it closes. */
  open: boolean;
  /** Settings > App > Show costs: off, no dollar figure is shown and no dollar limit offered. */
  showCosts: boolean;
  /** The chat's model has a price, so a new limit starts in dollars. */
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
  const [invalid, setInvalid] = useState(false);

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
      own?.maxUsd != null && showCosts ? 'usd' : own?.maxTokens != null ? 'tokens' : null;
    const startUnit: LimitUnit = ownUnit ?? (showCosts && priced ? 'usd' : 'tokens');
    setUnit(startUnit);
    setText(limitInputValue(startUnit === 'usd' ? own?.maxUsd : own?.maxTokens, startUnit));
    setInvalid(false);
    setEditing(true);
  };

  const submit = () => {
    const parsed = parseLimitInput(text, unit);
    if (!parsed.ok) {
      setInvalid(true);
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
    const ownChat = state.billingSessionId === sessionId;
    const browser = isBrowserSurface();
    const canWrite = ownChat && !browser;
    const override = state.chatOverride ?? null;

    return (
      <>
        <FooterCardValue data-testid="usage-limit-value">
          {limitReading(state.limit, showCosts)}
        </FooterCardValue>
        <FooterCardDetail>{limitSourceReading(state)}</FooterCardDetail>
        <FooterCardDetail>{chatUsedReading(state, showCosts)}</FooterCardDetail>
        {monthLine && <FooterCardDetail>{monthLine}</FooterCardDetail>}

        {state.paused && state.reason === 'month' && (
          <FooterCardNote role="alert" tone="danger">
            {COPY.pausedMonth}
          </FooterCardNote>
        )}
        {state.paused && state.reason !== 'month' && (
          <FooterCardNote role="alert" tone="danger">
            {COPY.pausedChat}
          </FooterCardNote>
        )}
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
                aria-invalid={invalid || undefined}
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
              {showCosts && (
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
              )}
            </div>
            {invalid && (
              <FooterCardNote id={errorId} role="alert" tone="danger">
                {unit === 'usd' ? COPY.invalidUsd : COPY.invalidTokens}
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
