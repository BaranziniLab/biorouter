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

const mocks = vi.hoisted(() => ({ getSession: vi.fn(), headers: vi.fn() }));
vi.mock('../api', () => ({ getSession: mocks.getSession }));
vi.mock('../utils/userAction', () => ({ userActionHeaders: mocks.headers }));

const items: TodoItem[] = [
  { id: '1', text: 'Compare clinic options', status: 'completed' },
  { id: '2', text: 'Verify arithmetic', status: 'in_progress' },
  { id: '3', text: 'Present the comparison', status: 'pending' },
];
const props = {
  name: 'Clinic comparison',
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
  it('does not flash a To Do section while an empty summary refreshes', () => {
    render(<ChatSummary {...props} todos={{ ...props.todos, loading: true }} />);
    expect(screen.queryByText(/To Do/)).not.toBeInTheDocument();
  });
  it('uses compact label/value typography and hides absent progress', () => {
    render(<ChatSummary {...props} />);
    expect(screen.getByText('6')).toHaveClass('text-sm');
    expect(screen.getByText('Tool calls')).toHaveClass('text-xs');
    expect(screen.getAllByRole('definition')).toHaveLength(4);
    expect(screen.queryByRole('region', { name: 'To Do progress' })).not.toBeInTheDocument();
    expect(screen.queryByRole('progressbar')).not.toBeInTheDocument();
  });
  it('labels a blocked step and indents an expanded one', () => {
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
    const rows = within(screen.getByRole('list', { name: 'To Do tasks' })).getAllByRole('listitem');
    expect(rows.map((row) => row.textContent)).toEqual([
      'Do the actual workPending',
      'Write itBlocked',
    ]);
    expect(rows[0]).not.toHaveClass('pl-4');
    expect(rows[1]).toHaveClass('pl-4');
  });
  it('shows ordered, explicitly labelled steps and accessible progress', () => {
    render(<ChatSummary {...props} todos={{ ...props.todos, items }} />);
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '1');
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuemax', '3');
    const rows = within(screen.getByRole('list', { name: 'To Do tasks' })).getAllByRole('listitem');
    expect(rows.map((row) => row.textContent)).toEqual([
      'Compare clinic optionsComplete',
      'Verify arithmeticIn progress',
      'Present the comparisonPending',
    ]);
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
  it('keeps the To Do list a list with a tab stop, marked as a focus region not a control', () => {
    render(<ChatSummary {...props} todos={{ ...props.todos, items }} />);
    const list = screen.getByRole('list', { name: 'To Do tasks' });
    // The tab stop is what lets a keyboard user scroll it; the class is what
    // keeps D-15's focus fill off it (asserted at the source in
    // `styles/tabFocus.test.ts`, since jsdom never evaluates `:focus-visible`).
    expect(list).toHaveAttribute('tabindex', '0');
    expect(list).toHaveClass('biorouter-focus-region');
    // No `role`: `role="region"` would orphan the rows' list semantics.
    expect(list).not.toHaveAttribute('role');
  });
  it('contains long lists, wraps labels and never executes task markup', () => {
    const text = '検証 🧬 <img src=x onerror=alert(1)> '.repeat(20);
    render(
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
    expect(screen.getByRole('list')).toHaveClass('max-h-60', 'overflow-y-auto');
    expect(screen.queryByRole('img')).not.toBeInTheDocument();
  });
  it('retains actions, loading feedback and recoverable refresh errors', () => {
    const { rerender } = render(
      <ChatSummary {...props} todos={{ ...props.todos, items, loading: true }} />
    );
    expect(screen.getByRole('status')).toHaveTextContent('Refreshing To Do');
    fireEvent.click(screen.getByRole('button', { name: 'Make workflow' }));
    fireEvent.click(screen.getByRole('button', { name: 'Diagnostics' }));
    expect(props.onWorkflow).toHaveBeenCalled();
    expect(props.onDiagnostics).toHaveBeenCalled();
    rerender(<ChatSummary {...props} hasWorkflow todos={{ ...props.todos, items, error: true }} />);
    expect(screen.getByRole('alert')).toHaveTextContent('may be out of date');
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(props.todos.refresh).toHaveBeenCalled();
    expect(screen.getByRole('button', { name: 'Workflow' })).toBeInTheDocument();
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
      within(screen.getByRole('list', { name: 'To Do tasks' })).getAllByRole('listitem')
    ).toHaveLength(4);
  });

  it('shows a checklist a script created while the summary was already open', async () => {
    mocks.getSession.mockResolvedValue({ data: NO_TASK_SESSION });
    // Open on a chat with no checklist yet — so nothing can come from the
    // loaded session, only from a refresh.
    const { rerender } = render(<SummaryHarness session={undefined} messages={[]} />);
    await waitFor(() => expect(mocks.getSession).toHaveBeenCalledTimes(1));
    expect(screen.queryByRole('region', { name: 'To Do progress' })).not.toBeInTheDocument();

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
      within(screen.getByRole('list', { name: 'To Do tasks' })).getAllByRole('listitem')
    ).toHaveLength(4);
  });
});
