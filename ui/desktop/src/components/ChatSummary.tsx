import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from 'react';
import {
  AlertCircle,
  Check,
  CircleIcon,
  CircleDotDashed,
  CodeAnalysis,
  Pipeline,
} from './icons/app-icons';
import { Button } from './ui/button';
import { InfoTip } from './ui/info-tip';
import { Progress } from './ui/progress';
import { summaryCopy } from './summary/copy';
import { SummarySubagents, type SummarySubagentsData } from './summary/SummarySubagents';
import { SummaryDecisions, type SummaryDecisionsData } from './summary/SummaryDecisions';
import { SummarySectionHeading, useSectionToggle } from './summary/SummarySectionHeading';
import { subagentWatchers } from './summary/agentGrid';
import { usePrivacyTiersEnabled } from './ConfigContext';
import { prefersReducedMotion } from '../styles/motion';
import { withDerivedActiveItem, type TodoItem, type TodoStatus } from '../utils/sessionTodos';
import './summary/chatSummary.css';

/**
 * Keyed by status so a status added to the backend fails the build here rather
 * than falling through a ternary chain and rendering as "Pending".
 *
 * Status is told apart by SHAPE (an open circle, a dashed circle, an alert, a
 * check), never by colour alone; the word itself is read to assistive
 * technology only, ahead of the task text.
 */
const TODO_STATUS_ICONS: Record<TodoStatus, typeof CircleIcon> = {
  pending: CircleIcon,
  in_progress: CircleDotDashed,
  blocked: AlertCircle,
  completed: Check,
};

/** At most this many new rows animate in one update; the rest appear at rest. */
const MAX_ANIMATED_ROWS = 5;
const ROW_STAGGER_MS = 30;

function Stat({ label, children }: { label: ReactNode; children: ReactNode }) {
  return (
    <div className="br-summary__stat">
      <dt className="br-summary__stat-label">{label}</dt>
      <dd className="br-summary__stat-value">{children}</dd>
    </div>
  );
}

/**
 * One to-do row. The glyph crossfades whenever the status changes after the row
 * first rendered; a row that arrives with the list stays at rest.
 *
 * Whether the row ENTERS is decided once, in the render that mounts it. The
 * list above re-derives "entering" on every render from a baseline that moves
 * on after the first, and BaseChat re-renders on every streamed chunk, so
 * reading it live would drop `.br-enter` (and its stagger delay) from a row
 * milliseconds into its 175ms entrance and pop it to full opacity.
 */
function TodoRow({
  item,
  entering,
  enterIndex,
}: {
  item: TodoItem;
  entering: boolean;
  enterIndex: number;
}) {
  const [animateIn] = useState(entering);
  const [delayIndex] = useState(enterIndex);
  // A row that has changed status once keeps its crossfade, so a return to the
  // status it began with (pending, running, pending) still plays.
  const [seenStatus, setSeenStatus] = useState(item.status);
  const [changed, setChanged] = useState(false);
  if (seenStatus !== item.status) {
    setSeenStatus(item.status);
    setChanged(true);
  }
  const Icon = TODO_STATUS_ICONS[item.status];
  const style: CSSProperties | undefined = animateIn
    ? { animationDelay: `${delayIndex * ROW_STAGGER_MS}ms` }
    : undefined;
  return (
    <li
      className={animateIn ? 'br-summary__todo br-enter' : 'br-summary__todo'}
      data-status={item.status}
      data-derived={item.derived ? '' : undefined}
      data-nested={item.parent !== undefined ? '' : undefined}
      data-todo-id={item.id}
      style={style}
    >
      <span
        key={item.status}
        className="br-summary__glyph"
        data-changed={changed ? '' : undefined}
        aria-hidden="true"
      >
        <Icon />
      </span>
      <span className="br-summary__todo-text">
        <span className="sr-only">{summaryCopy.todoStatus[item.status]}, </span>
        {item.text}
      </span>
    </li>
  );
}

/**
 * The statistics take `null` while a figure is not known yet: the transcript is
 * still loading, or (Tokens) the chat's usage ledger has not answered. The row
 * keeps its label and its 28px, with an empty value, so the figure arrives
 * once instead of a stand-in that is then replaced (vision V2-R5-5, V2-R5-6).
 */
export interface ChatSummaryProps {
  toolCalls: string | null;
  billedTokens: string | null;
  artifacts: string | null;
  codeDelta: { added: number; removed: number } | null;
  todos: { items: TodoItem[]; loading: boolean; error: boolean; refresh: () => void };
  /**
   * The chat's subagents and its phase plan (`hooks/useSessionSubagents.ts`).
   * Absent, or with nothing to list, the section draws nothing.
   */
  subagents?: SummarySubagentsData;
  /**
   * The chat's decision-model activity, one row per source
   * (`hooks/useSessionDecisions.ts`). Absent, or with no calls and no monitors,
   * the section draws nothing.
   */
  decisions?: SummaryDecisionsData;
  hasWorkflow: boolean;
  /** The chat's model by its short name, for This chat; absent or `null`, no Model row. */
  chatModel?: string | null;
  /**
   * The chat's privacy tier, for This chat while privacy tiers are on; absent or
   * `null` (not known yet), no Privacy row.
   */
  privacyTier?: 'private' | 'public' | null;
  /**
   * A decision chat (B-3): its model reads the chat and answers, and calls no
   * tools and writes no files. Details then holds Tokens only, and the footer
   * offers no Make workflow and Decisions no Monitors button.
   */
  decisionChat?: boolean;
  onWorkflow: () => void;
  onDiagnostics: () => void;
}

/**
 * The Chat summary's content: the live to-do list, the chat's subagents and
 * phases, the chat's statistics and two actions. The docked rail (`summary/ChatSummaryRail.tsx`) and the header
 * popover draw exactly this, so the two read as one component.
 *
 * A scrolling body over a pinned footer, in two type sizes (13 for rows and
 * actions, 12 for labels and counts). There is no "no tasks" sentence: with no
 * list, the statistics simply come first.
 */
export function ChatSummary({
  toolCalls,
  billedTokens,
  artifacts,
  codeDelta,
  todos,
  subagents,
  decisions,
  hasWorkflow,
  chatModel = null,
  privacyTier = null,
  decisionChat = false,
  onWorkflow,
  onDiagnostics,
}: ChatSummaryProps) {
  const tiersEnabled = usePrivacyTiersEnabled();
  // The current item reads as running although the agent never marked it so:
  // it now reports only what is done, and the next open item is current by
  // position (`withDerivedActiveItem`, the same rule the agent's copy uses).
  const items = useMemo(() => withDerivedActiveItem(todos.items), [todos.items]);
  const total = items.length;
  // F-1: with a phase plan the Phases block is what the person opened the
  // summary for, so To do starts collapsed; its count and bar stay on show.
  const [todoOpen, setTodoOpen] = useSectionToggle(!subagents?.data?.plan);
  const todoListId = useId();
  // F-1: the chat's own figures are secondary, so This chat starts closed; its
  // heading carries the token figure.
  const [detailsOpen, setDetailsOpen] = useSectionToggle(false);
  const detailsId = useId();
  const completed = items.filter((item) => item.status === 'completed').length;
  const statsPending =
    toolCalls === null || billedTokens === null || artifacts === null || codeDelta === null;

  // Rows that arrive with the first snapshot (an open, a reload, a tab switch)
  // are at rest; only rows added later enter. The baseline is taken once the
  // list has answered: either it has rows, or its first refresh has finished.
  const baselineRef = useRef<Set<string> | null>(null);
  if (baselineRef.current === null && (total > 0 || !todos.loading)) {
    baselineRef.current = new Set(items.map((item) => item.id));
  }
  const baseline = baselineRef.current;
  const entering = new Set<string>();
  if (baseline) {
    for (const item of items) {
      if (!baseline.has(item.id) && entering.size < MAX_ANIMATED_ROWS) entering.add(item.id);
    }
  }
  useEffect(() => {
    if (!baselineRef.current) return;
    for (const item of items) baselineRef.current.add(item.id);
  }, [items]);

  // Keep the running step in view, smoothly unless motion is reduced or the
  // pointer is over the summary (a list must not scroll under a reading cursor).
  const bodyRef = useRef<HTMLDivElement>(null);
  const pointerInsideRef = useRef(false);
  const runningId = items.find((item) => item.status === 'in_progress')?.id;
  const firstScrollRef = useRef(true);
  useEffect(() => {
    if (!runningId) return;
    const row = Array.from(
      bodyRef.current?.querySelectorAll<HTMLElement>('[data-todo-id]') ?? []
    ).find((element) => element.dataset.todoId === runningId);
    if (!row || typeof row.scrollIntoView !== 'function') return;
    const smooth = !firstScrollRef.current && !prefersReducedMotion() && !pointerInsideRef.current;
    firstScrollRef.current = false;
    row.scrollIntoView({ block: 'nearest', behavior: smooth ? 'smooth' : 'auto' });
  }, [runningId]);

  let enterIndex = 0;
  return (
    <div
      className="br-summary"
      onPointerEnter={() => {
        pointerInsideRef.current = true;
      }}
      onPointerLeave={() => {
        pointerInsideRef.current = false;
      }}
    >
      {/* A scroll container with a tab stop so a keyboard user can scroll it:
          a region, not a control, so it opts out of D-15's focus fill with
          `.biorouter-focus-region` (authored in main.css), which also draws
          the inset accent edge that is its only focus indicator. */}
      <div
        ref={bodyRef}
        className="br-summary__body biorouter-focus-region"
        tabIndex={0}
        data-summary-body=""
      >
        {total > 0 && (
          <section
            aria-label={summaryCopy.todo}
            aria-busy={todos.loading || undefined}
            className="br-summary__section"
          >
            {/* Visible "2 of 5"; announced "2 of 5 complete". */}
            <SummarySectionHeading
              label={summaryCopy.todo}
              count={summaryCopy.todoCount(completed, total)}
              open={todoOpen}
              onOpenChange={setTodoOpen}
              controls={todoListId}
              testId="summary-todo-toggle"
            >
              <span className="sr-only" aria-live="polite">
                {summaryCopy.todoCount(completed, total)}
                {summaryCopy.todoCountSuffix}
              </span>
            </SummarySectionHeading>
            <Progress
              size="thin"
              className="br-summary__progress"
              value={completed}
              max={total}
              label={summaryCopy.todoProgressLabel}
            />
            {todoOpen ? (
              <ol
                id={todoListId}
                aria-label={summaryCopy.todoListLabel}
                className="br-summary__todos"
              >
                {items.map((item) => {
                  const isEntering = entering.has(item.id);
                  return (
                    <TodoRow
                      key={item.id}
                      item={item}
                      entering={isEntering}
                      enterIndex={isEntering ? enterIndex++ : 0}
                    />
                  );
                })}
              </ol>
            ) : null}
          </section>
        )}
        {todos.error && (
          <div role="alert" className="br-summary__error">
            <span>{summaryCopy.todoRefreshFailed}</span>
            <Button
              type="button"
              variant="ghost"
              size="xs"
              className="br-summary__retry"
              onClick={todos.refresh}
            >
              {summaryCopy.retry}
            </Button>
          </div>
        )}
        {/* The Plan (or Agents) section, between the to-do list and the
            decisions. Its rows and squares are buttons of their own (not the
            Button primitive), in their own module, so this card keeps its
            three. A subagent's card says which of the chat's monitors watch it. */}
        {subagents && (
          <SummarySubagents
            {...subagents}
            watchedBy={subagents.watchedBy ?? subagentWatchers(decisions?.data?.settings)}
          />
        )}
        {/* Decision-model activity, aggregated per source: never one row per
            call. Its rows and its Monitors action are plain buttons in its own
            module, for the same reason as the subagents'. */}
        {decisions && <SummaryDecisions {...decisions} decisionChat={decisionChat} />}
        {/* F-1: the chat's own figures are secondary, so they sit in This chat,
            closed by default, whose heading carries the token figure. A
            decision chat calls no tools and writes no files, so it holds Tokens
            only (B-3), with its model. */}
        <section
          aria-label={summaryCopy.details}
          className="br-summary__section br-summary__details"
        >
          <SummarySectionHeading
            label={summaryCopy.details}
            count={billedTokens ? summaryCopy.detailsSummary(billedTokens) : undefined}
            open={detailsOpen}
            onOpenChange={setDetailsOpen}
            controls={detailsId}
            testId="summary-details-toggle"
          />
          {detailsOpen ? (
            <dl
              id={detailsId}
              aria-label={summaryCopy.statsLabel}
              aria-busy={statsPending || undefined}
              className="br-summary__stats"
            >
              {chatModel ? <Stat label={summaryCopy.model}>{chatModel}</Stat> : null}
              {tiersEnabled && privacyTier ? (
                <Stat label={summaryCopy.privacy}>{summaryCopy.tierWords[privacyTier]}</Stat>
              ) : null}
              {!decisionChat && <Stat label={summaryCopy.toolCalls}>{toolCalls}</Stat>}
              <Stat
                label={
                  <>
                    <span>{summaryCopy.tokens}</span>
                    <InfoTip label={summaryCopy.tokens} help={summaryCopy.tokensHelp} />
                  </>
                }
              >
                {billedTokens}
              </Stat>
              {!decisionChat && <Stat label={summaryCopy.artifacts}>{artifacts}</Stat>}
              {!decisionChat && (
                <Stat label={summaryCopy.code}>
                  {codeDelta && (
                    <>
                      <span className="sr-only">
                        {summaryCopy.codeDelta(codeDelta.added, codeDelta.removed)}
                      </span>
                      {/* A count carries its colour only when it is a change: "+0"
                        and "−0" are muted, so a chat that touched no code shows no
                        red. */}
                      <span aria-hidden="true">
                        <span
                          className={codeDelta.added > 0 ? 'text-text-success' : 'text-text-muted'}
                        >
                          +{codeDelta.added.toLocaleString()}
                        </span>{' '}
                        <span
                          className={codeDelta.removed > 0 ? 'text-text-danger' : 'text-text-muted'}
                        >
                          −{codeDelta.removed.toLocaleString()}
                        </span>
                      </span>
                    </>
                  )}
                </Stat>
              )}
            </dl>
          ) : null}
        </section>
      </div>
      <div className="br-summary__footer">
        {!decisionChat && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="br-summary__action"
            onClick={onWorkflow}
          >
            <Pipeline aria-hidden="true" />
            <span>{hasWorkflow ? summaryCopy.workflow : summaryCopy.makeWorkflow}</span>
          </Button>
        )}
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="br-summary__action"
          onClick={onDiagnostics}
        >
          <CodeAnalysis aria-hidden="true" />
          <span>{summaryCopy.diagnostics}</span>
        </Button>
      </div>
    </div>
  );
}
