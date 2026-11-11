import { describe, expect, it } from 'vitest';
import type { Message } from '../api';
import { sessionTodoItems, todoMutationRevision, withDerivedActiveItem } from './sessionTodos';
import { scriptedTodoExchange } from './scriptedTodoExchange.fixture';

export function todoExchange(
  id: string,
  options: { name?: string; error?: boolean; pending?: boolean; refused?: boolean } = {}
): Message[] {
  return [
    {
      role: 'assistant',
      created: 0,
      metadata: { agentVisible: true, userVisible: true },
      content: [
        {
          type: 'toolRequest',
          id,
          toolCall: {
            status: 'success',
            value: {
              name: options.name ?? 'todo__todo_update',
              arguments: { id: '1', status: 'completed' },
            },
          },
        },
        ...(options.pending
          ? []
          : [
              {
                type: 'toolResponse',
                id,
                toolResult: {
                  status: options.refused ? 'error' : 'success',
                  value: { content: [], isError: options.error ?? false },
                },
              },
            ]),
      ],
    },
  ] as Message[];
}

describe('persisted session checklist', () => {
  it.each([undefined, {}, { 'todo.v1': { plan: 'Only a plan', items: [] } }])(
    'hides an absent checklist',
    (data) => {
      expect(sessionTodoItems(data)).toEqual([]);
    }
  );
  it('reads canonical order, multiple active steps, and reopened work', () => {
    const items = [
      { id: '1', text: 'Compare', status: 'completed' },
      { id: '2', text: 'Verify', status: 'in_progress' },
      { id: '3', text: 'Present', status: 'in_progress' },
    ];
    expect(sessionTodoItems({ 'todo.v1': { items } })).toEqual(items);
    items[0].status = 'pending';
    expect(sessionTodoItems({ 'todo.v1': { items } })[0].status).toBe('pending');
  });
  it('never resurrects a cleared list from legacy state', () => {
    expect(
      sessionTodoItems({ 'todo.v1': { items: [] }, 'todo.v0': { content: '- [x] Old' } })
    ).toEqual([]);
  });
  it('treats a v1 plan-only state as empty even when a legacy list remains', () => {
    expect(
      sessionTodoItems({
        'todo.v1': { plan: 'Only a plan' },
        'todo.v0': { content: '- [x] Obsolete' },
      })
    ).toEqual([]);
  });
  it('supports legacy checklists but not freeform plans', () => {
    expect(
      sessionTodoItems({ 'todo.v0': { content: '- [ ] A\n* [X] B\n  + [-] C\nnot a task' } }).map(
        (item) => item.status
      )
    ).toEqual(['pending', 'completed', 'in_progress']);
    expect(sessionTodoItems({ 'todo.v0': { content: 'Plan prose' } })).toEqual([]);
  });
  it('reads blocked items and the parent of an expanded step', () => {
    expect(
      sessionTodoItems({
        'todo.v1': {
          items: [
            { id: '1', text: 'Coarse', status: 'pending' },
            { id: '2', text: 'Step', status: 'blocked', parent: '1' },
          ],
        },
      })
    ).toEqual([
      { id: '1', text: 'Coarse', status: 'pending' },
      { id: '2', text: 'Step', status: 'blocked', parent: '1' },
    ]);
    expect(sessionTodoItems({ 'todo.v0': { content: '- [!] Waiting' } })).toEqual([
      { id: '1', text: 'Waiting', status: 'blocked' },
    ]);
  });
  it('reads the additive blocked flag the backend now persists', () => {
    // The backend writes `status: 'in_progress'` plus `blocked: true` so a
    // Biorouter build predating the status word can still parse the whole
    // `todo.v1` blob instead of failing it and overwriting the checklist. The
    // flag is the truth; without honouring it the panel would show a blocked
    // item as merely in progress.
    expect(
      sessionTodoItems({
        'todo.v1': {
          items: [
            { id: '1', text: 'Waiting', status: 'in_progress', blocked: true },
            { id: '2', text: 'Working', status: 'in_progress' },
          ],
        },
      })
    ).toEqual([
      { id: '1', text: 'Waiting', status: 'blocked' },
      { id: '2', text: 'Working', status: 'in_progress' },
    ]);
  });
  it('keeps an item whose status this build does not know, as pending', () => {
    // ⚠ Deliberately NOT dropped. An unrecognised status used to remove the row
    // entirely, so a desktop build older than the backend hid every item in a
    // status it had not shipped yet — `blocked` was exactly that case. A task
    // the backend is tracking must stay visible; only its styling degrades.
    expect(
      sessionTodoItems({ 'todo.v1': { items: [{ id: '2', text: 'Bad', status: 'invented' }] } })
    ).toEqual([{ id: '2', text: 'Bad', status: 'pending' }]);
  });
  it('discards malformed entries and duplicate ids without losing valid tasks', () => {
    expect(
      sessionTodoItems({
        'todo.v1': {
          items: [
            null,
            { id: '1', text: ' Keep ' },
            { id: '1', text: 'duplicate' },
            { id: '3', text: '' },
          ],
        },
      })
    ).toEqual([{ id: '1', text: 'Keep', status: 'pending' }]);
  });
});

describe('acknowledged To Do changes', () => {
  it('invalidates for successful write/add/update only, once per id', () => {
    for (const name of ['todo_write', 'todo_add', 'todo_expand', 'todo_update']) {
      const messages = todoExchange('a', { name: `todo__${name}` });
      expect(todoMutationRevision([...messages, ...messages])).toBe('["a"]');
    }
  });
  it.each([
    { error: true },
    { pending: true },
    { refused: true },
    { name: 'todo__plan_write' },
    { name: 'developer__shell' },
  ])('does not treat an unconfirmed or unrelated call as an update', (options) => {
    expect(todoMutationRevision(todoExchange('a', options))).toBe('[]');
  });
});

describe('acknowledged To Do changes a script ran as sub-calls', () => {
  const write = { tool: 'todo__todo_write' };
  const add = { tool: 'todo__todo_add' };
  const update = { tool: 'todo__todo_update' };

  it('invalidates for a scripted checklist, and again for every later run', () => {
    const first = scriptedTodoExchange('run-1', [write, add, update]);
    const one = todoMutationRevision(first);
    // The bug: top-level requests alone see nothing here, so the summary never
    // refetches and a list created while it is open never appears.
    expect(one).not.toBe('[]');
    // Replay is not new work.
    expect(todoMutationRevision([...first, ...first])).toBe(one);
    // A second run is, even though nothing top-level changed. Catches a marker
    // that latches on the first scripted mutation and never moves again.
    expect(todoMutationRevision([...first, ...scriptedTodoExchange('run-2', [add])])).not.toBe(one);
    // So is a longer run under the same id. Catches keying on the id alone.
    expect(todoMutationRevision(scriptedTodoExchange('run-1', [write, add, update, add]))).not.toBe(
      one
    );
  });

  it('invalidates even when the enclosing script run itself failed', () => {
    // The sub-call's write is already persisted; a later throw does not undo it.
    expect(
      todoMutationRevision(scriptedTodoExchange('run-1', [write], { failedRun: true }))
    ).not.toBe('[]');
  });

  it.each([
    { tool: 'todo__todo_update', status: 'error' },
    { tool: 'todo__plan_write' },
    { tool: 'developer__shell' },
  ])('ignores a failed or non-checklist sub-call', (call) => {
    expect(todoMutationRevision(scriptedTodoExchange('run-1', [call]))).toBe('[]');
  });

  it('ignores a run that recorded no checklist sub-calls at all', () => {
    expect(todoMutationRevision(scriptedTodoExchange('run-1', []))).toBe('[]');
  });
});

function sidecarExchange(
  id: string,
  options: { name?: string; todo?: unknown; refused?: boolean; pending?: boolean } = {}
): Message[] {
  return [
    {
      role: 'assistant',
      created: 0,
      metadata: { agentVisible: true, userVisible: true },
      content: [
        {
          type: 'toolRequest',
          id,
          toolCall: {
            status: 'success',
            value: {
              name: options.name ?? 'code_execution__execute_code',
              arguments: { code: 'record_result(1)', todo: options.todo },
            },
          },
        },
        ...(options.pending
          ? []
          : [
              {
                type: 'toolResponse',
                id,
                toolResult: {
                  status: options.refused ? 'error' : 'success',
                  value: { content: [], isError: options.refused ?? false },
                },
              },
            ]),
      ],
    },
  ] as Message[];
}

describe('checklist progress that rode on a real call', () => {
  const empty = todoMutationRevision([]);

  it('moves the revision once the call carrying a todo field has answered', () => {
    const done = todoMutationRevision(sidecarExchange('a', { todo: { done: [2, 3] } }));
    expect(done).not.toBe(empty);
    // A second call with its own sidecar moves it again.
    expect(
      todoMutationRevision([
        ...sidecarExchange('a', { todo: { done: [2, 3] } }),
        ...sidecarExchange('b', { name: 'developer__shell', todo: '{"done":[4]}' }),
      ])
    ).not.toBe(done);
  });

  it('counts a failed call too, because the field is applied whatever the tool answered', () => {
    expect(
      todoMutationRevision(sidecarExchange('a', { todo: { done: [1] }, refused: true }))
    ).not.toBe(empty);
  });

  it('waits for the answer and ignores a call without the field', () => {
    expect(todoMutationRevision(sidecarExchange('a', { todo: { done: [1] }, pending: true }))).toBe(
      empty
    );
    expect(todoMutationRevision(sidecarExchange('a'))).toBe(empty);
    expect(todoMutationRevision(sidecarExchange('a', { todo: null }))).toBe(empty);
  });
});

describe('the derived current item', () => {
  it('marks the first open, unblocked item in progress when none is', () => {
    const items = [
      { id: '1', text: 'a', status: 'completed' as const },
      { id: '2', text: 'b', status: 'blocked' as const },
      { id: '3', text: 'c', status: 'pending' as const },
      { id: '4', text: 'd', status: 'pending' as const },
    ];
    const read = withDerivedActiveItem(items);
    expect(read[2]).toEqual({ id: '3', text: 'c', status: 'in_progress', derived: true });
    expect(read[3].status).toBe('pending');
    // The stored list is not touched.
    expect(items[2].status).toBe('pending');
  });

  it('leaves an explicit in-progress item alone and derives nothing', () => {
    const items = [
      { id: '1', text: 'a', status: 'pending' as const },
      { id: '2', text: 'b', status: 'in_progress' as const },
    ];
    expect(withDerivedActiveItem(items)).toEqual(items);
  });

  it('skips a grouping row while its steps are open, then returns to it', () => {
    const items = [
      { id: '1', text: 'coarse', status: 'pending' as const },
      { id: '3', text: 'step', status: 'pending' as const, parent: '1' },
      { id: '2', text: 'later', status: 'pending' as const },
    ];
    expect(withDerivedActiveItem(items).find((item) => item.derived)?.id).toBe('3');
    const stepsDone = items.map((item) =>
      item.id === '3' ? { ...item, status: 'completed' as const } : item
    );
    expect(withDerivedActiveItem(stepsDone).find((item) => item.derived)?.id).toBe('1');
    expect(
      withDerivedActiveItem([{ id: '1', text: 'x', status: 'completed' as const }]).some(
        (item) => item.derived
      )
    ).toBe(false);
  });
});
