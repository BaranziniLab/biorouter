import { ChatKindIcon } from '../chats/ChatKindIcon';
import { plainExcerpt } from './plainExcerpt';
import {
  FooterCardAction,
  FooterCardActions,
  FooterCardDetail,
  FooterCardSection,
  FooterCardValue,
} from '../bottom_menu/FooterCard';
import { usePrivacyTiersEnabled } from '../ConfigContext';
import { Disclosure } from '../ui/disclosure';
import { useOptionalKnowledge } from '../knowledge/KnowledgeContext';
import { extensionDisplayName } from '../settings/extensions/extensionDisplayName';
import { decisionModelDisplay, joinWithAnd } from '../decisions/decisionWords';
import { isDecisionProvider } from '../decisions/decisionWire';
import { SUBAGENT_STATUS_WORDS } from '../subagent/subagentStatus';
import { formatCount, formatTokensCompact } from '../../utils/usageStats';
import {
  formatElapsed,
  isActiveStatus,
  isQuiet,
  shortModelLabel,
  subagentTitle,
  type SubagentRow,
} from '../../utils/sessionSubagents';
import { summaryCopy } from './copy';

const copy = summaryCopy.agents.peek;

/** At most this many names per list in the peek, then "and N more". */
export const PEEK_NAME_LIMIT = 8;

/** `Developer, Knowledge, Todo and 3 more`; `None` for an empty list. */
export function peekNameList(names: readonly string[], limit = PEEK_NAME_LIMIT): string {
  if (names.length === 0) return copy.none;
  const shown = names.slice(0, limit);
  const more = names.length - shown.length;
  return more > 0 ? `${shown.join(', ')} ${copy.more(more)}` : shown.join(', ');
}

/**
 * A knowledge base's name for an id the list does not know: the id read as words
 * with a capital first letter (`soul` → `Soul`, `trial-notes` → `Trial notes`),
 * never the raw id (R6).
 */
export function knowledgeBaseWords(id: string): string {
  const words = id.replace(/[-_]+/g, ' ').trim();
  return words ? words.charAt(0).toUpperCase() + words.slice(1) : id;
}

/**
 * The model as a person names it (R3, C-9): a decision model's alias and where it
 * runs; any other model's short name, never the provider key or a dated slug.
 */
export function peekModelName(row: Pick<SubagentRow, 'modelName' | 'providerName'>): string | null {
  if (row.modelName && isDecisionProvider(row.providerName)) {
    return decisionModelDisplay({ provider: row.providerName!, model: row.modelName });
  }
  return shortModelLabel(row.modelName);
}

/**
 * The card's line under its title (owner request, 2026-11-11): the model and where
 * it runs. A decision model says so first, with its alias and its place
 * (`Decision model · tev1:0.8b · this computer`); a language model is its short
 * name and, while privacy tiers are on, the chat tier it runs at
 * (`sonnet-4-5 · Public`). `null` when the row names no model.
 */
export function peekModelLine(
  row: Pick<SubagentRow, 'modelName' | 'providerName' | 'privacyTier'>,
  tiersEnabled: boolean
): string | null {
  const model = peekModelName(row);
  if (isDecisionProvider(row.providerName)) {
    return [summaryCopy.agents.decisionModel, model].filter(Boolean).join(' · ');
  }
  if (!model) return null;
  const tier = tiersEnabled && row.privacyTier ? copy.tier[row.privacyTier] : null;
  return tier ? `${model} · ${tier}` : model;
}

/** `Watched by Latch stop and Scope check`, or `null` when no monitor watches it. */
export function peekWatchedLine(watchedBy: readonly string[]): string | null {
  return watchedBy.length > 0 ? copy.watchedBy(joinWithAnd([...watchedBy])) : null;
}

/** `Completed · 3s`, `Running · 4m`, `Quiet for 4m`, `Stopped · 1m`. */
export function peekResultLabel(row: SubagentRow): string {
  if (isQuiet(row)) return summaryCopy.agents.quietFor(formatElapsed(row.quietSeconds) ?? '');
  const elapsed = formatElapsed(row.elapsedSeconds);
  const word = SUBAGENT_STATUS_WORDS[row.status];
  return elapsed ? `${word} · ${elapsed}` : word;
}

/** `9.8k tokens · 0 tool calls · 3 messages`. */
export function peekUsedLine(row: SubagentRow): string {
  return [
    row.totalTokens !== null ? copy.tokens(formatTokensCompact(row.totalTokens)) : null,
    copy.toolCalls(formatCount(row.toolCallCount), row.toolCallCount),
    copy.messages(formatCount(row.messageCount), row.messageCount),
  ]
    .filter((part): part is string => !!part)
    .join(' · ');
}

/**
 * The content of a subagent's peek (spec 4.3), built only from the footer card's
 * parts and one Disclosure, so a preview and a kept peek look the same and the
 * peek reads like every other card the app hangs off a control: its title, the
 * model and where it runs (and, for a decision model or a watched subagent, that
 * it is decision-model work), the task when the title is a role, the status and
 * run time over the result, "Open in a tab", then Details with the setup by
 * display name (no provider keys, dated slugs or raw ids).
 */
export function SubagentPeek({
  row,
  onOpen,
  watchedBy = [],
}: {
  row: SubagentRow;
  /** "Open in a tab": open the child as a tab, then close the peek. */
  onOpen: () => void;
  /** The chat's monitors that watch its subagents (`subagentWatchers`). */
  watchedBy?: readonly string[];
}) {
  const knowledge = useOptionalKnowledge();
  const tiersEnabled = usePrivacyTiersEnabled();
  const modelLine = peekModelLine(row, tiersEnabled);
  const watchedLine = peekWatchedLine(watchedBy);
  const title = subagentTitle(row);
  const model = peekModelName(row);
  const knowledgeNames = row.knowledgeBases.map(
    (id) => knowledge?.bases.find((base) => base.id === id)?.name || knowledgeBaseWords(id)
  );
  // R2-5: plain words, never the answer's raw markdown (`plainExcerpt`).
  const excerpt = plainExcerpt(row.resultLine);
  const result = excerpt
    ? excerpt
    : isActiveStatus(row.status)
      ? summaryCopy.agents.working
      : copy.noResult;
  return (
    <>
      <FooterCardValue className="br-agents-peek__title">
        <ChatKindIcon
          session={{ session_type: 'sub_agent' }}
          tier={row.privacyTier}
          testId={`subagent-peek-glyph-${row.sessionId}`}
        />
        <span>{title}</span>
      </FooterCardValue>
      {modelLine || watchedLine ? (
        <div className="br-agents-peek__lines">
          {modelLine ? <FooterCardDetail>{modelLine}</FooterCardDetail> : null}
          {watchedLine ? <FooterCardDetail>{watchedLine}</FooterCardDetail> : null}
        </div>
      ) : null}
      {row.role && row.name ? (
        <FooterCardSection label={copy.task}>
          <FooterCardValue className="br-agents-peek__clamp" data-lines="3">
            {row.name}
          </FooterCardValue>
        </FooterCardSection>
      ) : null}
      <FooterCardSection label={peekResultLabel(row)}>
        <FooterCardValue className="br-agents-peek__clamp" data-lines="4">
          {result}
        </FooterCardValue>
      </FooterCardSection>
      <FooterCardActions>
        <FooterCardAction onClick={onOpen} data-testid={`subagent-peek-open-${row.sessionId}`}>
          {copy.openInTab}
        </FooterCardAction>
      </FooterCardActions>
      <Disclosure
        label={copy.details}
        summary={model ?? undefined}
        className="br-agents-peek__details"
      >
        <dl className="br-agents-peek__facts">
          <dt>{copy.model}</dt>
          <dd>{model ?? copy.none}</dd>
          <dt>{copy.extensions}</dt>
          <dd>{peekNameList(row.extensions.map((name) => extensionDisplayName(name)))}</dd>
          <dt>{copy.skills}</dt>
          <dd>{peekNameList(row.skills)}</dd>
          <dt>{copy.knowledgeBases}</dt>
          <dd>{peekNameList(knowledgeNames)}</dd>
          <dt>{copy.used}</dt>
          <dd>{peekUsedLine(row)}</dd>
        </dl>
      </Disclosure>
    </>
  );
}
