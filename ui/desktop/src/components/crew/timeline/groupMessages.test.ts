import { describe, expect, it } from 'vitest';
import type { CrewMessage } from '../crewApi';
import {
  GROUP_GAP_MS,
  HISTORY_PAGE_SIZE,
  canBePageBefore,
  groupMessages,
  isTraceMessage,
  newLineBeforeId,
  reachesChannelStart,
  taskTitle,
  type GroupMessagesOptions,
  type TimelineDay,
  type TimelineGroup,
  type TimelineItem,
} from './groupMessages';
import { ID, message, run } from './timelineTestUtils';
import { dayLabel, fullDateTime, messageTime, shortTime } from './timelineTime';

/**
 * The timeline's shape, decided in one pure function (ui-redesign-spec, "The
 * timeline"): grouping, day breaks, where the New line goes, the owner's task
 * rows, and an agent's folded tool updates.
 */

const NOW = new Date(2026, 8, 22, 18, 0);
const at = (hour: number, minute = 0, day = 22, month = 8, year = 2026) =>
  new Date(year, month, day, hour, minute);

function options(overrides: Partial<GroupMessagesOptions> = {}): GroupMessagesOptions {
  return {
    channelId: ID.general,
    channelRestricted: false,
    newLineBeforeId: null,
    runs: [],
    includeUnanchoredRuns: true,
    now: NOW,
    ...overrides,
  };
}

const items = (days: TimelineDay[]): TimelineItem[] => days.flatMap((day) => day.items);
const groups = (days: TimelineDay[]): TimelineGroup[] =>
  items(days).filter((item): item is TimelineGroup => item.kind === 'group');
/** Each group as the IDs of the messages it shows, traces flattened. */
const shape = (days: TimelineDay[]) =>
  groups(days).map((group) =>
    group.entries.flatMap((entry) =>
      entry.kind === 'trace' ? entry.messages.map((item) => item.id) : [entry.message.id]
    )
  );

describe('grouping', () => {
  it('keeps one author within five minutes in one group, with only the first row as the head', () => {
    const list = [
      message({ id: 'a', at: at(10, 0) }),
      message({ id: 'b', at: at(10, 3) }),
      message({ id: 'c', at: new Date(at(10, 3).getTime() + GROUP_GAP_MS) }),
    ];
    const days = groupMessages(list, options());
    expect(shape(days)).toEqual([['a', 'b', 'c']]);
    const [group] = groups(days);
    expect(group.entries.map((entry) => entry.head)).toEqual([true, false, false]);
  });

  it('breaks on a gap over five minutes', () => {
    const list = [
      message({ id: 'a', at: at(10, 0) }),
      message({ id: 'b', at: new Date(at(10, 0).getTime() + GROUP_GAP_MS + 1000) }),
    ];
    expect(shape(groupMessages(list, options()))).toEqual([['a'], ['b']]);
  });

  it('breaks on a new author, and again when the first author returns', () => {
    const list = [
      message({ id: 'a', actor_id: ID.bob, at: at(10, 0) }),
      message({ id: 'b', actor_id: ID.alice, at: at(10, 1) }),
      message({ id: 'c', actor_id: ID.bob, at: at(10, 2) }),
    ];
    expect(shape(groupMessages(list, options()))).toEqual([['a'], ['b'], ['c']]);
  });

  it('breaks between a person and their own agent, and between two of their tasks', () => {
    const list = [
      message({ id: 'human', actor_id: ID.alice, at: at(10, 0) }),
      message({ id: 'agent-1', actor_id: ID.alice, run_id: ID.run, at: at(10, 1) }),
      message({ id: 'agent-2', actor_id: ID.alice, run_id: ID.runB, at: at(10, 2) }),
      message({ id: 'human-2', actor_id: ID.alice, at: at(10, 3) }),
    ];
    const days = groupMessages(list, options());
    expect(shape(days)).toEqual([['human'], ['agent-1'], ['agent-2'], ['human-2']]);
    expect(groups(days).map((group) => group.agent)).toEqual([false, true, true, false]);
  });

  it('breaks at the New line', () => {
    const list = [message({ id: 'a', at: at(10, 0) }), message({ id: 'b', at: at(10, 1) })];
    const days = groupMessages(list, options({ newLineBeforeId: 'b' }));
    expect(items(days).map((item) => item.kind)).toEqual(['group', 'new', 'group']);
    expect(shape(days)).toEqual([['a'], ['b']]);
  });

  it('marks a restricted message only where the channel is not restricted', () => {
    const list = [message({ id: 'r', restricted: true }), message({ id: 'p', restricted: false })];
    const marks = (restricted: boolean) =>
      groups(groupMessages(list, options({ channelRestricted: restricted })))[0].entries.map(
        (entry) => entry.kind === 'message' && entry.restrictedMarker
      );
    expect(marks(false)).toEqual([true, false]);
    expect(marks(true)).toEqual([false, false]);
  });

  it('returns nothing for an empty channel with no runs', () => {
    expect(groupMessages([], options())).toEqual([]);
  });
});

describe('day breaks', () => {
  it('starts a day section at the first message and at each new calendar day', () => {
    const list = [
      message({ id: 'old', at: at(9, 0, 3, 5, 2025) }),
      message({ id: 'mon', at: at(9, 0, 14) }),
      message({ id: 'yesterday', at: at(23, 58, 21) }),
      message({ id: 'today', at: at(0, 1, 22) }),
    ];
    const days = groupMessages(list, options());
    expect(days.map((day) => day.label)).toEqual([
      'June 3, 2025',
      'Monday, September 14',
      'Yesterday',
      'Today',
    ]);
    // A day break always breaks the group, even two minutes apart.
    expect(shape(days)).toEqual([['old'], ['mon'], ['yesterday'], ['today']]);
  });

  it('labels days by the local calendar, and times in en-US', () => {
    expect(dayLabel(at(8, 0), NOW)).toBe('Today');
    expect(dayLabel(at(8, 0, 21), NOW)).toBe('Yesterday');
    expect(dayLabel(at(8, 0, 20), NOW)).toBe('Sunday, September 20');
    expect(dayLabel(at(8, 0, 22, 8, 2025), NOW)).toBe('September 22, 2025');
    expect(shortTime(at(10, 2))).toBe('10:02 AM');
    expect(fullDateTime(at(22, 5))).toBe('Tuesday, September 22, 2026 at 10:05 PM');
  });

  it('reads the broker’s Unix seconds, and milliseconds from a newer stamp', () => {
    const date = at(10, 2);
    expect(messageTime(date.getTime() / 1000).getTime()).toBe(date.getTime());
    expect(messageTime(date.getTime()).getTime()).toBe(date.getTime());
  });
});

describe('the New line', () => {
  const list = (): CrewMessage[] => [
    message({ id: 'a', sequence: 's1', actor_id: ID.bob }),
    message({ id: 'b', sequence: 's2', actor_id: ID.bob }),
    message({ id: 'c', sequence: 's3', actor_id: ID.carol }),
  ];

  it('goes before the first message after the read position', () => {
    expect(newLineBeforeId(list(), { readPosition: 's1', unread: 0, viewerId: ID.alice })).toBe(
      'b'
    );
  });

  it('is absent when the read position is the newest message', () => {
    expect(
      newLineBeforeId(list(), { readPosition: 's3', unread: 0, viewerId: ID.alice })
    ).toBeNull();
  });

  it('falls back to the unread count without a position, or with one that is not loaded', () => {
    expect(
      newLineBeforeId(list(), { readPosition: undefined, unread: 1, viewerId: ID.alice })
    ).toBe('c');
    expect(newLineBeforeId(list(), { readPosition: null, unread: 2, viewerId: ID.alice })).toBe(
      'b'
    );
    expect(newLineBeforeId(list(), { readPosition: 'gone', unread: 2, viewerId: ID.alice })).toBe(
      'b'
    );
    // More unread than loaded: everything loaded is new.
    expect(
      newLineBeforeId(list(), { readPosition: undefined, unread: 9, viewerId: ID.alice })
    ).toBe('a');
    expect(
      newLineBeforeId(list(), { readPosition: undefined, unread: 0, viewerId: ID.alice })
    ).toBeNull();
  });

  it('skips the viewer’s own posts at the start of the new region', () => {
    const mine = [
      message({ id: 'a', sequence: 's1', actor_id: ID.bob }),
      message({ id: 'mine', sequence: 's2', actor_id: ID.alice }),
      message({ id: 'theirs', sequence: 's3', actor_id: ID.bob }),
    ];
    expect(newLineBeforeId(mine, { readPosition: 's1', unread: 0, viewerId: ID.alice })).toBe(
      'theirs'
    );
    // …but the viewer's AGENT is not the viewer: its posts are news.
    const agent = [
      message({ id: 'a', sequence: 's1', actor_id: ID.bob }),
      message({ id: 'agent', sequence: 's2', actor_id: ID.alice, run_id: ID.run }),
    ];
    expect(newLineBeforeId(agent, { readPosition: 's1', unread: 0, viewerId: ID.alice })).toBe(
      'agent'
    );
    // Only the viewer's own posts are new: no line.
    expect(
      newLineBeforeId(mine.slice(0, 2), { readPosition: 's1', unread: 0, viewerId: ID.alice })
    ).toBeNull();
  });
});

describe('task rows', () => {
  it('anchors a run after its first message and continues its posts in a new group', () => {
    const list = [
      message({ id: 'ask', actor_id: ID.bob, at: at(10, 0) }),
      message({
        id: 'task',
        actor_id: ID.alice,
        run_id: ID.run,
        body: 'Task: Plot counts by sample\nand post the figure.',
        at: at(10, 1),
      }),
      message({ id: 'answer', actor_id: ID.alice, run_id: ID.run, body: 'Done.', at: at(10, 2) }),
    ];
    const days = groupMessages(list, options({ runs: [run()] }));
    const sequence = items(days).map((item) => item.kind);
    expect(sequence).toEqual(['group', 'group', 'task', 'group']);
    const task = items(days).find((item) => item.kind === 'task');
    expect(task).toMatchObject({ anchored: true, title: 'Plot counts by sample' });
    expect(shape(days)).toEqual([['ask'], ['task'], ['answer']]);
  });

  it('puts a run whose first message is not loaded at the end of the live log only', () => {
    const list = [message({ id: 'a' })];
    const live = groupMessages(list, options({ runs: [run()] }));
    expect(items(live).map((item) => item.kind)).toEqual(['group', 'task']);
    expect(items(live)[1]).toMatchObject({ anchored: false, title: null });

    const page = groupMessages(list, options({ runs: [run()], includeUnanchoredRuns: false }));
    expect(items(page).map((item) => item.kind)).toEqual(['group']);
  });

  it('shows a running task even before any message loads', () => {
    const days = groupMessages([], options({ runs: [run()] }));
    expect(days).toHaveLength(1);
    expect(days[0].label).toBeNull();
    expect(days[0].items.map((item) => item.kind)).toEqual(['task']);
  });

  it('ignores runs in other channels and runs that are not the viewer’s', () => {
    const list = [message({ id: 'task', actor_id: ID.bob, run_id: ID.runB, body: 'Task: theirs' })];
    const days = groupMessages(
      list,
      options({ runs: [run({ channel_id: ID.methods }), run({ run_id: 'not-listed-here' })] })
    );
    // Bob's run is not in `runs` (owner-scoped), and the other run lives in #methods.
    expect(items(days).filter((item) => item.kind === 'task')).toHaveLength(1);
    expect(items(days).find((item) => item.kind === 'task')).toMatchObject({
      run: { run_id: 'not-listed-here' },
      anchored: false,
    });
  });

  it('reads a title only from a "Task: …" post', () => {
    expect(taskTitle('Task: Plot counts\nsecond line')).toBe('Plot counts');
    expect(taskTitle('Using crew__request')).toBeNull();
    expect(taskTitle('Task:    ')).toBeNull();
  });
});

describe('an agent’s tool updates', () => {
  const agent = (body: string, extra: Partial<CrewMessage> = {}) =>
    message({ actor_id: ID.alice, run_id: ID.run, body, ...extra });

  it('recognizes the daemon’s projection lines', () => {
    for (const body of [
      'Using crew__request',
      'Tool response received: context.manifest.',
      'Tool failed: blob.read. Inspect the task conversation for details.',
      'Requested remote.read: /data/counts.csv',
      'remote.execute returned a job receipt. Check remote.job_status for its outcome.',
    ]) {
      expect(isTraceMessage(agent(body)), body).toBe(true);
    }
    expect(isTraceMessage(agent('Here are the totals: 1.80 / 18.0'))).toBe(false);
    expect(
      isTraceMessage(agent('Waiting for its owner’s approval in the task conversation.'))
    ).toBe(false);
  });

  it('never folds a person’s post, or an agent’s final answer', () => {
    expect(isTraceMessage(message({ body: 'Using crew__request' }))).toBe(false);
    expect(
      isTraceMessage(agent('Using crew__request', { status: 'completed' } as Partial<CrewMessage>))
    ).toBe(false);
  });

  it('folds consecutive updates into one entry, and never the task’s anchor', () => {
    const list = [
      agent('Task: Using crew__request', { id: 'task' }),
      agent('Using crew__request', { id: 't1' }),
      agent('Tool failed: blob.read. Inspect the task conversation for details.', { id: 't2' }),
      agent('The totals are 1.80 / 18.0.', { id: 'answer' }),
      agent('Tool response received: run.project.', { id: 't3' }),
    ];
    const days = groupMessages(list, options());
    const [group] = groups(days);
    expect(group.entries.map((entry) => entry.kind)).toEqual([
      'message',
      'trace',
      'message',
      'trace',
    ]);
    const firstTrace = group.entries[1];
    expect(firstTrace.kind === 'trace' && firstTrace.messages.map((item) => item.id)).toEqual([
      't1',
      't2',
    ]);
  });
});

describe('history pages', () => {
  it('knows the channel’s start is loaded only below a full page', () => {
    const page = (count: number) =>
      Array.from({ length: count }, (_, index) => message({ id: `${index}` }));
    expect(reachesChannelStart(page(0))).toBe(true);
    expect(reachesChannelStart(page(HISTORY_PAGE_SIZE - 1))).toBe(true);
    expect(reachesChannelStart(page(HISTORY_PAGE_SIZE))).toBe(false);
  });

  it('knows the list on screen is not yet the page before its first message', () => {
    const older = message({ id: 'older' });
    const live = [message({ id: 'a' }), message({ id: 'b' }), message({ id: 'c' })];
    const boundary = live[0].sequence;
    // loadOlder names live[0] as the boundary before the list is cleared.
    expect(canBePageBefore(live, boundary)).toBe(false);
    expect(canBePageBefore([...live], boundary)).toBe(false);
    // The page that lands holds only messages before it; the live tail always can.
    expect(canBePageBefore([older], boundary)).toBe(true);
    expect(canBePageBefore([], boundary)).toBe(true);
    expect(canBePageBefore(live, null)).toBe(true);
  });
});
