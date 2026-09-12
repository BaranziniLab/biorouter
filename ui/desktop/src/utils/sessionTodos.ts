import type { Message } from '../api';

export const TODO_STATUSES = ['pending', 'in_progress', 'blocked', 'completed'] as const;

export type TodoStatus = (typeof TODO_STATUSES)[number];

export type TodoItem = {
  id: string;
  text: string;
  status: TodoStatus;
  /** Set when the item was expanded out of another; nesting is one level deep. */
  parent?: string;
};

function isTodoStatus(value: unknown): value is TodoStatus {
  return TODO_STATUSES.includes(value as TodoStatus);
}

function record(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

export function sessionTodoItems(extensionData: unknown): TodoItem[] {
  const data = record(extensionData);
  const current = record(data?.['todo.v1']);
  if (current && current.items === undefined) return [];
  if (current && Array.isArray(current.items)) {
    const seen = new Set<string>();
    return current.items.flatMap((value): TodoItem[] => {
      const item = record(value);
      if (
        !item ||
        typeof item.id !== 'string' ||
        !item.id ||
        seen.has(item.id) ||
        typeof item.text !== 'string' ||
        !item.text.trim()
      )
        return [];
      // ⚠ Never drop the row for an unrecognised status. This used to `return
      // []`, so a status this build had not heard of — `blocked` was exactly
      // that, until it shipped — made the item VANISH from the panel while the
      // backend still tracked it: a task the user believes is on the list and
      // cannot see. An unknown status now degrades to `pending` instead.
      // `blocked` is persisted as an additive flag beside a `status` word a
      // reader predating the variant can parse (`in_progress`), because the
      // status word itself went into the unchanged `todo.v1` key and an older
      // Biorouter that cannot parse the blob overwrites it. See the
      // `BLOCKED_WIRE_STATUS` note in `session/extension_data.rs`. The flag
      // wins, so the panel still shows a blocked item as blocked.
      const raw = item.blocked === true ? 'blocked' : (item.status ?? 'pending');
      const status: TodoStatus = isTodoStatus(raw) ? raw : 'pending';
      seen.add(item.id);
      const parent = typeof item.parent === 'string' && item.parent ? item.parent : undefined;
      return [{ id: item.id, text: item.text.trim(), status, ...(parent ? { parent } : {}) }];
    });
  }
  const legacy = record(data?.['todo.v0'])?.content;
  if (typeof legacy !== 'string') return [];
  return legacy
    .split('\n')
    .flatMap((line): TodoItem[] => {
      const match = line.trimStart().match(/^[-*+] \[([ xX~!-])\] (.+)$/);
      if (!match || !match[2].trim()) return [];
      return [
        {
          id: '',
          text: match[2].trim(),
          status: /[xX]/.test(match[1])
            ? 'completed'
            : /[~-]/.test(match[1])
              ? 'in_progress'
              : match[1] === '!'
                ? 'blocked'
                : 'pending',
        },
      ];
    })
    .map((item, index) => ({ ...item, id: String(index + 1) }));
}

/**
 * The four tools that mutate the persisted checklist. `todo__plan_write` moves
 * the prose plan, not the checklist, so it is deliberately absent.
 *
 * ⚠ A checklist mutation missing from this list leaves the panel showing the
 * pre-call state until something else happens to move the revision.
 */
const TODO_MUTATION_TOOLS = [
  'todo__todo_write',
  'todo__todo_add',
  'todo__todo_expand',
  'todo__todo_update',
];

/**
 * Result-meta key under which one call records the sub-calls it actually ran.
 * The only producer in the tree is `code_execution_extension.rs:1841` (#28).
 * Must match `TOOL_CALLS_META_KEY` there.
 *
 * A coding-agent provider does NOT need this path: `reply_parts.rs:274-284`
 * hands bridge providers `plan.tools` instead of the code-execution-filtered
 * list, so their `todo__*` calls arrive as ordinary top-level requests.
 */
const EXECUTED_CALLS_META_KEY = 'biorouter/tool-calls';

/**
 * How many acknowledged checklist mutations a tool result ran as SUB-calls.
 *
 * Both channels are live, so both are read. Since 2026-09-11 the five
 * `todo__*` tools are exempt from `reply_parts::survives_code_execution_filter`
 * (`todo_extension::TODO_TOOL_NAMES`) and the planning gate points a multi-step
 * turn at `todo__todo_write` by name, so the common case is now an ordinary
 * top-level request, counted in `todoMutationRevision` below. But a script can
 * still import the same tools, and a checklist it writes leaves NO top-level
 * `todo__*` request in the transcript — only this meta.
 *
 * That second channel is why this function exists. Before the exemption it was
 * the ONLY one in a default chat (`code_execution` is `default_enabled: true`),
 * and a revision derived from top-level requests alone stayed constant for the
 * whole session: the effect never re-ran and the panel was frozen at open time.
 * Measured on the drive session `20260901_1`: top-level requests were
 * `code_execution` x39 and `workspace` x12 and nothing else, while 15 separate
 * `execute_code` runs carried `todo__*` sub-calls.
 *
 * ⚠ The counter therefore advances at `execute_code` run BOUNDARIES, not per
 * mutation — there is no incremental channel. Within one run that mutates the
 * list several times the panel still shows the pre-run state until the run
 * returns.
 *
 * The enclosing run's own status is deliberately NOT consulted. The backend
 * attaches this meta to a failed run too, precisely because a script can throw
 * long after its `todo_write` landed — the write is already persisted, so the
 * summary must still refresh. Each record carries the acknowledgement for its
 * own sub-call, in `status`.
 */
function executedTodoMutations(resultValue: unknown): number {
  const calls = record(record(resultValue)?._meta)?.[EXECUTED_CALLS_META_KEY];
  if (!Array.isArray(calls)) return 0;
  return calls.filter((candidate) => {
    const call = record(candidate);
    return !!call && call.status === 'ok' && TODO_MUTATION_TOOLS.includes(String(call.tool));
  }).length;
}

/** Only acknowledged mutations invalidate the persisted checklist, never proposed arguments. */
export function todoMutationRevision(messages: readonly Message[]): string {
  const requests = new Set<string>();
  const completed = new Set<string>();
  for (const message of messages) {
    for (const content of message.content) {
      if (content.type === 'toolRequest') {
        const call = record(content.toolCall.value);
        if (
          content.toolCall.status === 'success' &&
          TODO_MUTATION_TOOLS.includes(String(call?.name))
        ) {
          requests.add(content.id);
        }
      }
    }
  }
  for (const message of messages) {
    for (const content of message.content) {
      if (content.type !== 'toolResponse') continue;
      // Keyed by the call id AND the number of mutations it ran, so a second
      // script run (new id) and a longer run under the same id both move the
      // revision, while replaying the same transcript re-derives the identical
      // entry. Encoded as a JSON pair rather than `id#n`: nothing validates that
      // a provider-supplied call id excludes `#`, so an id literally `x#1` would
      // collide with the nested entry for id `x` after one mutation.
      //
      // ⚠ This reads the meta BEFORE any status check, unlike
      // `artifactRefresh.ts:54-56`, which gates nested-meta reading on the
      // enclosing result succeeding. The divergence is deliberate — see the
      // note on `executedTodoMutations` — so do not "align" one with the other
      // without reading both.
      const executed = executedTodoMutations(content.toolResult.value);
      if (executed > 0) completed.add(JSON.stringify([content.id, executed]));
      if (!requests.has(content.id)) continue;
      const result = record(content.toolResult.value);
      if (
        content.toolResult.status === 'success' &&
        result &&
        result.isError !== true &&
        result.is_error !== true
      ) {
        completed.add(content.id);
      }
    }
  }
  return JSON.stringify([...completed].sort());
}
