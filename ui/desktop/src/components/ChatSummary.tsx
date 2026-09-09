import type { ReactNode } from 'react';
import {
  AlertCircle,
  Check,
  CircleIcon,
  CircleDotDashed,
  CodeAnalysis,
  Pipeline,
} from './icons/app-icons';
import { Button } from './ui/button';
import { cn } from '../utils';
import type { TodoItem, TodoStatus } from '../utils/sessionTodos';

/**
 * Keyed by status so a status added to the backend fails the build here rather
 * than falling through a ternary chain and rendering as "Pending".
 */
const TODO_STATUS_LABELS: Record<TodoStatus, string> = {
  pending: 'Pending',
  in_progress: 'In progress',
  blocked: 'Blocked',
  completed: 'Complete',
};

const TODO_STATUS_ICONS: Record<TodoStatus, typeof CircleIcon> = {
  pending: CircleIcon,
  in_progress: CircleDotDashed,
  blocked: AlertCircle,
  completed: Check,
};

function Metric({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex min-w-0 flex-wrap items-baseline justify-between gap-x-2 gap-y-0.5">
      <dt className="text-xs text-text-muted">{label}</dt>
      <dd className="text-sm font-medium tabular-nums text-text-default">{children}</dd>
    </div>
  );
}

export function ChatSummary({
  name,
  toolCalls,
  billedTokens,
  artifacts,
  codeDelta,
  todos,
  hasWorkflow,
  onWorkflow,
  onDiagnostics,
}: {
  name: string;
  toolCalls: string;
  billedTokens: string;
  artifacts: string;
  codeDelta: { added: number; removed: number };
  todos: { items: TodoItem[]; loading: boolean; error: boolean; refresh: () => void };
  hasWorkflow: boolean;
  onWorkflow: () => void;
  onDiagnostics: () => void;
}) {
  const completed = todos.items.filter((item) => item.status === 'completed').length;
  return (
    <div className="space-y-3">
      <header className="min-w-0">
        <h2 className="text-sm font-medium text-text-default">Chat summary</h2>
        <p className="truncate text-xs text-text-muted" title={name}>
          {name}
        </p>
      </header>
      <dl className="grid grid-cols-2 gap-x-4 gap-y-2" aria-label="Chat statistics">
        <Metric label="Tool calls">{toolCalls}</Metric>
        <Metric label="Billed tokens">{billedTokens}</Metric>
        <Metric label="Artifacts">{artifacts}</Metric>
        <Metric label="Code">
          <span className="text-text-success">+{codeDelta.added.toLocaleString()}</span>{' '}
          <span className="text-text-danger">−{codeDelta.removed.toLocaleString()}</span>
        </Metric>
      </dl>
      {todos.items.length > 0 && (
        <section aria-label="To Do progress" className="border-t border-border-subtle pt-3">
          <div className="mb-2 flex items-baseline justify-between gap-2 text-xs">
            <h3 className="font-medium text-text-default">To Do</h3>
            <span className="text-text-muted" aria-live="polite">
              {completed} of {todos.items.length} complete
            </span>
          </div>
          <div
            role="progressbar"
            aria-label="Completed tasks"
            aria-valuemin={0}
            aria-valuemax={todos.items.length}
            aria-valuenow={completed}
            className="mb-3 h-1 overflow-hidden rounded-full bg-background-medium"
          >
            <div
              className="h-full bg-text-success"
              style={{ width: `${(completed / todos.items.length) * 100}%` }}
            />
          </div>
          {/* A scroll container with a tab stop so a keyboard user can scroll
              it — a region, not a control, so it opts out of D-15's focus fill
              with `.biorouter-focus-region` (authored in main.css). It keeps
              its implicit `list` role: `role="region"` would orphan the rows. */}
          <ol
            aria-label="To Do tasks"
            tabIndex={0}
            className="biorouter-focus-region max-h-60 overflow-y-auto overscroll-contain pr-1"
          >
            {todos.items.map((item, index) => {
              const label = TODO_STATUS_LABELS[item.status];
              const Icon = TODO_STATUS_ICONS[item.status];
              // Expanded steps are indented under the item they came from, the
              // one level of nesting the backend allows.
              const nested = item.parent !== undefined;
              return (
                <li
                  key={item.id}
                  className={cn('relative flex gap-2 pb-3 last:pb-0', nested && 'pl-4')}
                >
                  {index < todos.items.length - 1 && (
                    <span
                      aria-hidden="true"
                      className={cn(
                        'absolute bottom-0 top-4 border-l border-border-subtle',
                        nested ? 'left-[23px]' : 'left-[7px]'
                      )}
                    />
                  )}
                  <Icon
                    aria-hidden="true"
                    className={cn(
                      'relative mt-0.5 h-4 w-4 shrink-0',
                      item.status === 'in_progress'
                        ? 'text-text-accent'
                        : item.status === 'blocked'
                          ? 'text-text-warning'
                          : 'text-text-muted'
                    )}
                  />
                  <div className="min-w-0 flex-1 text-xs leading-relaxed">
                    <p
                      className={cn(
                        'whitespace-pre-wrap break-words [overflow-wrap:anywhere]',
                        item.status === 'completed' ? 'text-text-muted' : 'text-text-default'
                      )}
                    >
                      {item.text}
                    </p>
                    <span className="text-text-muted">{label}</span>
                  </div>
                </li>
              );
            })}
          </ol>
        </section>
      )}
      {todos.error ? (
        <div
          role="alert"
          className="flex items-center justify-between gap-2 text-xs text-text-warning"
        >
          <span>
            {todos.items.length
              ? 'To Do could not refresh. Displayed progress may be out of date.'
              : 'Summary could not refresh.'}
          </span>
          <Button type="button" variant="ghost" size="xs" onClick={todos.refresh}>
            Retry
          </Button>
        </div>
      ) : (
        todos.loading &&
        todos.items.length > 0 && (
          <p role="status" className="text-xs text-text-muted">
            Refreshing To Do…
          </p>
        )
      )}
      <div className="flex flex-wrap gap-2 border-t border-border-subtle pt-3">
        <Button
          type="button"
          variant="secondary"
          size="sm"
          className="min-w-0 flex-1 basis-36 gap-1.5"
          onClick={onWorkflow}
        >
          <Pipeline />
          <span>{hasWorkflow ? 'Workflow' : 'Make workflow'}</span>
        </Button>
        <Button
          type="button"
          variant="secondary"
          size="sm"
          className="min-w-0 flex-1 basis-36 gap-1.5"
          onClick={onDiagnostics}
        >
          <CodeAnalysis />
          <span>Diagnostics</span>
        </Button>
      </div>
    </div>
  );
}
