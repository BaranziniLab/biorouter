import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { ChatSummary } from './ChatSummary';
import type { Message, Session } from '../api';
import type { TodoItem } from '../utils/sessionTodos';
// One fixture, not two: the duplicate that used to live in this file took
// `string[]` while the canonical one takes `{tool, status?}[]`, so the copies
// had already drifted in shape before they could drift in behaviour.
import { scriptedTodoExchange } from '../utils/scriptedTodoExchange.fixture';
import { useSessionTodos } from '../hooks/useSessionTodos';
import { summaryCopy } from './summary/copy';

const mocks = vi.hoisted(() => ({ getSession: vi.fn(), headers: vi.fn() }));
vi.mock('../api', () => ({ getSession: mocks.getSession }));
vi.mock('../utils/userAction', () => ({ userActionHeaders: mocks.headers }));

const items: TodoItem[] = [
  { id: '1', text: 'Compare clinic options', status: 'completed' },
  { id: '2', text: 'Verify arithmetic', status: 'in_progress' },
  { id: '3', text: 'Present the comparison', status: 'pending' },
];
const props = {
  toolCalls: '6',
  billedTokens: '211k',
  artifacts: '1',
  codeDelta: { added: 0, removed: 0 },
  hasWorkflow: false,
  onWorkflow: vi.fn(),
  onDiagnostics: vi.fn(),
  todos: { items: [], loading: false, error: false, refresh: vi.fn() },
};

describe('compact chat summary', () => {
  it('does not flash a To do section while an empty summary refreshes', () => {
    render(<ChatSummary {...props} todos={{ ...props.todos, loading: true }} />);
    expect(screen.queryByText(summaryCopy.todo)).not.toBeInTheDocument();
    expect(screen.queryByRole('progressbar')).not.toBeInTheDocument();
  });

  it('draws the statistics as label and value rows, and hides absent progress', () => {
    render(<ChatSummary {...props} />);
    expect(screen.getByText('6')).toHaveClass('br-summary__stat-value');
    expect(screen.getByText(summaryCopy.toolCalls).closest('dt')).toHaveClass(
      'br-summary__stat-label'
    );
    expect(screen.getAllByRole('definition')).toHaveLength(4);
    expect(screen.getByText('6').closest('dl')).toHaveAttribute(
      'aria-label',
      summaryCopy.statsLabel
    );
    expect(screen.queryByRole('region', { name: summaryCopy.todo })).not.toBeInTheDocument();
    expect(screen.queryByRole('progressbar')).not.toBeInTheDocument();
    // No "no tasks" sentence: the statistics simply come first.
    expect(screen.queryByText(/no tasks|nothing to do/i)).not.toBeInTheDocument();
  });

  it('explains Tokens in hover help that a screen reader also hears', () => {
    render(<ChatSummary {...props} />);
    expect(screen.getByText(summaryCopy.tokens)).toBeInTheDocument();
    const help = screen.getByRole('button', { name: `About ${summaryCopy.tokens}` });
    expect(help).toHaveAccessibleDescription(summaryCopy.tokensHelp);
    // The explanation is not a visible line of its own.
    expect(screen.queryByText('Billed tokens')).not.toBeInTheDocument();
  });

  it('reads the code delta as words and draws it with a real minus sign', () => {
    render(<ChatSummary {...props} codeDelta={{ added: 120, removed: 8 }} />);
    expect(screen.getByText(summaryCopy.codeDelta(120, 8))).toHaveClass('sr-only');
    expect(screen.getByText('−8')).toHaveClass('text-text-danger');
    expect(screen.getByText('+120')).toHaveClass('text-text-success');
  });

  it('labels a blocked step for assistive technology and indents an expanded one', () => {
    // A status the panel does not render is a task the user cannot see: the
    // list reader keeps unknown statuses, so every one the backend can persist
    // must have a label here.
    render(
      <ChatSummary
        {...props}
        todos={{
          ...props.todos,
          items: [
            { id: '1', text: 'Do the actual work', status: 'pending' },
            { id: '2', text: 'Write it', status: 'blocked', parent: '1' },
          ],
        }}
      />
    );
    const rows = within(screen.getByRole('list', { name: summaryCopy.todoListLabel })).getAllByRole(
      'listitem'
    );
    expect(rows.map((row) => row.textContent)).toEqual([
      'Pending, Do the actual work',
      'Blocked, Write it',
    ]);
    // The word is read, not shown: status is told apart by the glyph's shape.
    expect(within(rows[1]).getByText('Blocked,', { exact: false })).toHaveClass('sr-only');
    expect(rows[0]).not.toHaveAttribute('data-nested');
    expect(rows[1]).toHaveAttribute('data-nested');
    expect(rows.map((row) => row.getAttribute('data-status'))).toEqual(['pending', 'blocked']);
  });

  it('shows ordered, labelled steps and accessible progress', () => {
    render(<ChatSummary {...props} todos={{ ...props.todos, items }} />);
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '1');
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuemax', '3');
    expect(screen.getByRole('progressbar')).toHaveAccessibleName(summaryCopy.todoProgressLabel);
    const rows = within(screen.getByRole('list', { name: summaryCopy.todoListLabel })).getAllByRole(
      'listitem'
    );
    expect(rows.map((row) => row.textContent)).toEqual([
      'Complete, Compare clinic options',
      'In progress, Verify arithmetic',
      'Pending, Present the comparison',
    ]);
    // The visible count is "1 of 3"; the live region reads "1 of 3 complete".
    expect(screen.getByText('1 of 3')).toHaveAttribute('aria-hidden', 'true');
    const count = screen.getByText('1 of 3 complete');
    expect(count).toHaveAttribute('aria-live', 'polite');
    expect(count).toHaveClass('sr-only');
  });

  it('updates completion, reopening, renaming, replacement and clearing without stale rows', () => {
    const { rerender } = render(<ChatSummary {...props} todos={{ ...props.todos, items }} />);
    rerender(
      <ChatSummary
        {...props}
        todos={{ ...props.todos, items: items.map((item) => ({ ...item, status: 'completed' })) }}
      />
    );
    expect(screen.getByText('3 of 3 complete')).toBeInTheDocument();
    rerender(
      <ChatSummary
        {...props}
        todos={{
          ...props.todos,
          items: [{ id: '1', text: 'Recheck assumptions', status: 'in_progress' }],
        }}
      />
    );
    expect(screen.getByText('0 of 1 complete')).toBeInTheDocument();
    expect(screen.queryByText('Verify arithmetic')).not.toBeInTheDocument();
    expect(screen.getByText('Recheck assumptions')).toBeInTheDocument();
    rerender(<ChatSummary {...props} />);
    expect(screen.queryByRole('progressbar')).not.toBeInTheDocument();
  });

  it('crossfades a glyph only when a status changes after the row first rendered', () => {
    const { rerender, container } = render(
      <ChatSummary {...props} todos={{ ...props.todos, items }} />
    );
    expect(container.querySelectorAll('.br-summary__glyph[data-changed]')).toHaveLength(0);
    rerender(
      <ChatSummary
        {...props}
        todos={{
          ...props.todos,
          items: items.map((item) => ({ ...item, status: 'completed' as const })),
        }}
      />
    );
    // Two rows changed status (the first was already complete).
    expect(container.querySelectorAll('.br-summary__glyph[data-changed]')).toHaveLength(2);
  });

  it('enters rows added after the first snapshot, at most five at once, never the first ones', () => {
    const { rerender, container } = render(
      <ChatSummary {...props} todos={{ ...props.todos, items }} />
    );
    expect(container.querySelectorAll('.br-enter')).toHaveLength(0);
    const added = Array.from({ length: 7 }, (_, i) => ({
      id: `new-${i}`,
      text: `Added ${i}`,
      status: 'pending' as const,
    }));
    rerender(<ChatSummary {...props} todos={{ ...props.todos, items: [...items, ...added] }} />);
    const entering = Array.from(container.querySelectorAll<HTMLElement>('.br-enter'));
    expect(entering).toHaveLength(5);
    expect(entering.map((row) => row.style.animationDelay)).toEqual([
      '0ms',
      '30ms',
      '60ms',
      '90ms',
      '120ms',
    ]);
  });

  it('scrolls its body, not the list, and keeps the list a list', () => {
    const { container } = render(<ChatSummary {...props} todos={{ ...props.todos, items }} />);
    const body = container.querySelector('[data-summary-body]');
    // The tab stop is what lets a keyboard user scroll it; the class is what
    // keeps D-15's focus fill off it (asserted at the source in
    // `styles/tabFocus.test.ts`, since jsdom never evaluates `:focus-visible`).
    expect(body).toHaveAttribute('tabindex', '0');
    expect(body).toHaveClass('br-summary__body', 'biorouter-focus-region');
    expect(body).not.toHaveAttribute('role');
    const list = screen.getByRole('list', { name: summaryCopy.todoListLabel });
    expect(body).toContainElement(list);
    // No `role` on the list either: `role="region"` would orphan the rows.
    expect(list).not.toHaveAttribute('role');
    expect(list).not.toHaveAttribute('tabindex');
  });

  it('contains long lists, wraps labels and never executes task markup', () => {
    const text = '検証 🧬 <img src=x onerror=alert(1)> '.repeat(20);
    const { container } = render(
      <ChatSummary
        {...props}
        todos={{
          ...props.todos,
          items: Array.from({ length: 200 }, (_, i) => ({
            id: String(i),
            text: `${i} ${text}`,
            status: 'pending',
          })),
        }}
      />
    );
    expect(screen.getAllByRole('listitem')).toHaveLength(200);
    expect(container.querySelector('[data-summary-body]')).toContainElement(
      screen.getByRole('list')
    );
    expect(screen.queryByRole('img')).not.toBeInTheDocument();
  });

  it('retains actions, marks a refresh busy and offers a retry on a failed one', () => {
    const { rerender } = render(
      <ChatSummary {...props} todos={{ ...props.todos, items, loading: true }} />
    );
    expect(screen.getByRole('region', { name: summaryCopy.todo })).toHaveAttribute(
      'aria-busy',
      'true'
    );
    fireEvent.click(screen.getByRole('button', { name: summaryCopy.makeWorkflow }));
    fireEvent.click(screen.getByRole('button', { name: summaryCopy.diagnostics }));
    expect(props.onWorkflow).toHaveBeenCalled();
    expect(props.onDiagnostics).toHaveBeenCalled();
    rerender(<ChatSummary {...props} hasWorkflow todos={{ ...props.todos, items, error: true }} />);
    expect(screen.getByRole('alert')).toHaveTextContent(summaryCopy.todoRefreshFailed);
    fireEvent.click(screen.getByRole('button', { name: summaryCopy.retry }));
    expect(props.todos.refresh).toHaveBeenCalled();
    expect(screen.getByRole('button', { name: summaryCopy.workflow })).toBeInTheDocument();
  });
});

/**
 * Issue #144. The two halves above are unit-level: `ChatSummary` is handed an
 * items array. These mount the REAL `useSessionTodos` behind it.
 *
 * ⚠ This comment previously said the failure was "the panel never renders" and
 * located it "in the join between the persisted `todo.v1` state and the
 * section's mount condition". **Both halves were wrong**, and the issue has been
 * re-titled. The panel renders; it sits behind the Chat-summary popover. The
 * defect is in REVISION DERIVATION, not the mount condition: `todo__*` is not
 * exempt from `reply_parts::survives_code_execution_filter` (applied at
 * reply_parts.rs:286) and `code_execution` is `default_enabled: true`, so a
 * default chat carries ZERO top-level `todo__*` requests — leaving the old
 * revision key a constant, so the refetch effect never re-ran and the open
 * popover stayed frozen at its value at open time.
 *
 * The live-refresh path itself already existed and is NOT what these tests add:
 * `useSessionTodos.ts:14` computes `revision` and lists it in the effect's deps
 * at `:63`.
 *
 * ⚠ The exemption described as missing above now exists (2026-09-11): the five
 * `todo__*` tools stay directly callable in Code Execution mode, and the
 * planning gate sends a multi-step turn to `todo__todo_write` first. So the
 * COMMON path is now a top-level request, pinned by the last test below; the
 * scripted path above still happens whenever a model imports the tools.
 */
const FOUR_TASK_SESSION = {
  id: 'chat',
  extension_data: {
    'todo.v1': {
      items: [
        { id: '1', text: 'alpha', status: 'completed' },
        { id: '2', text: 'beta', status: 'pending' },
        { id: '3', text: 'gamma', status: 'pending' },
        { id: '4', text: 'delta', status: 'pending' },
      ],
    },
  },
} as unknown as Session;
const NO_TASK_SESSION = { id: 'chat', extension_data: {} } as unknown as Session;

function SummaryHarness({
  session,
  messages,
}: {
  session: Session | undefined;
  messages: Message[];
}) {
  return <ChatSummary {...props} todos={useSessionTodos('chat', session, messages, true)} />;
}

describe('the summary panel over persisted To Do state', () => {
  beforeEach(() => {
    mocks.getSession.mockReset().mockResolvedValue({ data: FOUR_TASK_SESSION });
    mocks.headers.mockReset().mockResolvedValue({ 'X-User-Action': 'synthetic-proof' });
  });

  // CONTROL, not evidence about #144. This passes with the entire revision-
  // derivation change reverted, because it pins the pre-existing `initialItems`
  // fallback (`useSessionTodos.ts:66-68`), already covered by
  // `useSessionTodos.test.tsx:37-52` and `:130-139`. Kept because first-paint
  // rendering from a reloaded session is worth holding; it is labelled so the
  // next reader does not mistake a green run here for the bug being fixed.
  it('renders the four tasks a reloaded session already carries, before any refresh lands', () => {
    render(<SummaryHarness session={FOUR_TASK_SESSION} messages={[]} />);
    // Asserted on the FIRST paint, while the refresh is still in flight.
    expect(screen.getByText('1 of 4 complete')).toBeInTheDocument();
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '1');
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuemax', '4');
    expect(
      within(screen.getByRole('list', { name: summaryCopy.todoListLabel })).getAllByRole('listitem')
    ).toHaveLength(4);
  });

  it('shows a checklist a script created while the summary was already open', async () => {
    mocks.getSession.mockResolvedValue({ data: NO_TASK_SESSION });
    // Open on a chat with no checklist yet — so nothing can come from the
    // loaded session, only from a refresh.
    const { rerender } = render(<SummaryHarness session={undefined} messages={[]} />);
    await waitFor(() => expect(mocks.getSession).toHaveBeenCalledTimes(1));
    expect(screen.queryByRole('region', { name: summaryCopy.todo })).not.toBeInTheDocument();

    // The agent now creates the list from a script. The only trace in the
    // transcript is the enclosing call's executed-sub-call meta.
    mocks.getSession.mockResolvedValue({ data: FOUR_TASK_SESSION });
    rerender(
      <SummaryHarness
        session={undefined}
        messages={scriptedTodoExchange(
          'run-1',
          ['todo__todo_write', 'todo__todo_add', 'todo__todo_add', 'todo__todo_update'].map(
            (tool) => ({ tool })
          )
        )}
      />
    );

    await waitFor(() => expect(screen.getByText('1 of 4 complete')).toBeInTheDocument());
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuemax', '4');
    expect(
      within(screen.getByRole('list', { name: summaryCopy.todoListLabel })).getAllByRole('listitem')
    ).toHaveLength(4);
  });

  // The path the planning gate makes the usual one: the model's FIRST action on
  // a multi-step request is a direct `todo__todo_write`, and the list has to
  // appear the moment that call is acknowledged — then tick as updates land.
  it('shows a checklist the moment a direct todo_write lands, then ticks it off', async () => {
    mocks.getSession.mockResolvedValue({ data: NO_TASK_SESSION });
    const { rerender } = render(<SummaryHarness session={undefined} messages={[]} />);
    await waitFor(() => expect(mocks.getSession).toHaveBeenCalledTimes(1));
    expect(screen.queryByRole('region', { name: summaryCopy.todo })).not.toBeInTheDocument();

    const direct = (id: string, name: string): Message[] =>
      [
        {
          role: 'assistant',
          created: 0,
          metadata: { agentVisible: true, userVisible: true },
          content: [
            { type: 'toolRequest', id, toolCall: { status: 'success', value: { name } } },
            {
              type: 'toolResponse',
              id,
              toolResult: { status: 'success', value: { content: [], isError: false } },
            },
          ],
        },
      ] as Message[];

    const planned = {
      id: 'chat',
      extension_data: {
        'todo.v1': {
          items: [
            { id: '1', text: 'create a temp dir', status: 'pending' },
            { id: '2', text: 'write hello.txt', status: 'pending' },
          ],
        },
      },
    } as unknown as Session;
    mocks.getSession.mockResolvedValue({ data: planned });
    const afterPlan = direct('plan', 'todo__todo_write');
    rerender(<SummaryHarness session={undefined} messages={afterPlan} />);
    await waitFor(() => expect(screen.getByText('0 of 2 complete')).toBeInTheDocument());

    const ticked = {
      id: 'chat',
      extension_data: {
        'todo.v1': {
          items: [
            { id: '1', text: 'create a temp dir', status: 'completed' },
            { id: '2', text: 'write hello.txt', status: 'pending' },
          ],
        },
      },
    } as unknown as Session;
    mocks.getSession.mockResolvedValue({ data: ticked });
    rerender(
      <SummaryHarness
        session={undefined}
        messages={[...afterPlan, ...direct('tick-1', 'todo__todo_update')]}
      />
    );
    await waitFor(() => expect(screen.getByText('1 of 2 complete')).toBeInTheDocument());
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '1');
  });
});
