import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
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
import { prefersReducedMotion } from '../styles/motion';
import type { TodoItem, TodoStatus } from '../utils/sessionTodos';
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
 * One to-do row. The glyph crossfades when the status changes after the row
 * first rendered; a row that arrives with the list stays at rest.
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
  const [initialStatus] = useState(item.status);
  const Icon = TODO_STATUS_ICONS[item.status];
  const style: CSSProperties | undefined = entering
    ? { animationDelay: `${enterIndex * ROW_STAGGER_MS}ms` }
    : undefined;
  return (
    <li
      className={entering ? 'br-summary__todo br-enter' : 'br-summary__todo'}
      data-status={item.status}
      data-nested={item.parent !== undefined ? '' : undefined}
      data-todo-id={item.id}
      style={style}
    >
      <span
        key={item.status}
        className="br-summary__glyph"
        data-changed={item.status !== initialStatus ? '' : undefined}
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

export interface ChatSummaryProps {
  toolCalls: string;
  billedTokens: string;
  artifacts: string;
  codeDelta: { added: number; removed: number };
  todos: { items: TodoItem[]; loading: boolean; error: boolean; refresh: () => void };
  hasWorkflow: boolean;
  onWorkflow: () => void;
  onDiagnostics: () => void;
}

/**
 * The Chat summary's content: the live to-do list, the chat's statistics and
 * two actions. The docked rail (`summary/ChatSummaryRail.tsx`) and the header
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
  hasWorkflow,
  onWorkflow,
  onDiagnostics,
}: ChatSummaryProps) {
  const total = todos.items.length;
  const completed = todos.items.filter((item) => item.status === 'completed').length;

  // Rows that arrive with the first snapshot (an open, a reload, a tab switch)
  // are at rest; only rows added later enter. The baseline is taken once the
  // list has answered: either it has rows, or its first refresh has finished.
  const baselineRef = useRef<Set<string> | null>(null);
  if (baselineRef.current === null && (total > 0 || !todos.loading)) {
    baselineRef.current = new Set(todos.items.map((item) => item.id));
  }
  const baseline = baselineRef.current;
  const entering = new Set<string>();
  if (baseline) {
    for (const item of todos.items) {
      if (!baseline.has(item.id) && entering.size < MAX_ANIMATED_ROWS) entering.add(item.id);
    }
  }
  useEffect(() => {
    if (!baselineRef.current) return;
    for (const item of todos.items) baselineRef.current.add(item.id);
  }, [todos.items]);

  // Keep the running step in view, smoothly unless motion is reduced or the
  // pointer is over the summary (a list must not scroll under a reading cursor).
  const bodyRef = useRef<HTMLDivElement>(null);
  const pointerInsideRef = useRef(false);
  const runningId = todos.items.find((item) => item.status === 'in_progress')?.id;
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
            <div className="br-summary__heading">
              <h3 className="br-summary__label">{summaryCopy.todo}</h3>
              {/* Visible "2 of 5"; announced "2 of 5 complete". */}
              <span className="br-summary__count" aria-hidden="true">
                {summaryCopy.todoCount(completed, total)}
              </span>
              <span className="sr-only" aria-live="polite">
                {summaryCopy.todoCount(completed, total)}
                {summaryCopy.todoCountSuffix}
              </span>
            </div>
            <Progress
              className="br-summary__progress"
              value={completed}
              max={total}
              label={summaryCopy.todoProgressLabel}
            />
            <ol aria-label={summaryCopy.todoListLabel} className="br-summary__todos">
              {todos.items.map((item) => {
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
          </section>
        )}
        {todos.error && (
          <div role="alert" className="br-summary__error">
            <span>{summaryCopy.todoRefreshFailed}</span>
            <Button type="button" variant="ghost" size="xs" onClick={todos.refresh}>
              {summaryCopy.retry}
            </Button>
          </div>
        )}
        <dl
          aria-label={summaryCopy.statsLabel}
          className="br-summary__stats"
          data-after-todos={total > 0 ? '' : undefined}
        >
          <Stat label={summaryCopy.toolCalls}>{toolCalls}</Stat>
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
          <Stat label={summaryCopy.artifacts}>{artifacts}</Stat>
          <Stat label={summaryCopy.code}>
            <span className="sr-only">
              {summaryCopy.codeDelta(codeDelta.added, codeDelta.removed)}
            </span>
            <span aria-hidden="true">
              <span className="text-text-success">+{codeDelta.added.toLocaleString()}</span>{' '}
              <span className="text-text-danger">−{codeDelta.removed.toLocaleString()}</span>
            </span>
          </Stat>
        </dl>
      </div>
      <div className="br-summary__footer">
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
