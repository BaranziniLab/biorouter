import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter, Route, Routes, useLocation } from 'react-router-dom';
import ToolCallWithResponse, {
  crewChangeTarget,
  crewTaskChat,
  summarizeToolCall,
} from './ToolCallWithResponse';
import type { ToolRequestMessageContent } from '../types/message';
import { CREW_SELECT_ROUTE_KEY, crewSelectIntent } from './crew/state/useCrewConnections';

/**
 * Chat parity WI-16: a chat's Crew person-tool rows say what the change did ("Posting in
 * #general"), and a finished change offers "Open in Crew", which opens Crew where the daemon's
 * answer says it went, with the IDs that answer carries and nothing the model typed.
 */

const CONNECTION = '0f3c1e2a-5b6d-4e7f-8a9b-0c1d2e3f4a5b';
const CHANNEL = '1a2b3c4d-5e6f-4a1b-8c2d-3e4f5a6b7c8d';
const MESSAGE = '2b3c4d5e-6f7a-4b2c-9d3e-4f5a6b7c8d9e';
const RUN = 'run-7f3a9c1e';
/** An ID the model typed into its params: it must never reach the link. */
const TYPED = '9e8d7c6b-5a4f-4e3d-8c2b-1a0f9e8d7c6b';

const answer = (value: unknown) => ({
  status: 'success',
  value: { content: [{ type: 'text', text: JSON.stringify(value) }] },
});

const change = (action: string, params: Record<string, unknown> = {}) => ({
  name: 'crew__change',
  arguments: { action, params },
});

describe('summarizeToolCall: a person-tool row names the action (WI-16)', () => {
  it.each([
    ['send', { channel: '#general', text: 'status update' }, 'Posting in #general'],
    ['send', { channel: 'general' }, 'Posting in #general'],
    ['send', { channel: TYPED }, 'Posting a message'],
    ['channels_create', { name: 'methods', team: 'lab' }, 'Creating #methods in lab'],
    ['channels_create', {}, 'Creating a channel'],
    [
      'channels_rename',
      { channel: '#methods', name: 'methods-2' },
      'Renaming #methods to #methods-2',
    ],
    ['channels_archive', { channel: '#methods' }, 'Marking #methods archived'],
    [
      'channels_join',
      { channel: 'analysis-lab/field-notes' },
      'Adding you to #field-notes in analysis-lab',
    ],
    ['channels_delete', { channel: '#methods' }, 'Deleting #methods'],
    ['channels_read', { channel: '#general' }, 'Marking #general read'],
    ['teams_create', { name: 'analysis-lab' }, 'Creating the team analysis-lab'],
    ['teams_rename', { team: 'lab', name: 'lab-2' }, 'Renaming the team lab to lab-2'],
    ['teams_join', { team: 'field-team' }, 'Adding you to the team field-team'],
    ['teams_delete', { team: 'lab' }, 'Deleting the team lab'],
    ['members_add', { member: '@bob', team: 'lab' }, 'Adding @bob to the team lab'],
    ['members_add', { member: '@bob', channels: ['#methods'] }, 'Adding @bob to #methods'],
    ['members_add', { member: '@bob', channels: ['#a', '#b'] }, 'Adding @bob to 2 channels'],
    [
      'invites_send',
      { member: '@bob', channel: '#methods' },
      'Sending @bob an invitation to #methods',
    ],
    ['invites_accept', { invitation: 'field-team' }, 'Using your invitation to field-team'],
    [
      'ownership_offer',
      { channel: '#methods', successor: '@bob' },
      'Sending @bob an offer to own #methods',
    ],
    ['ownership_accept', { channel: '#methods' }, 'Setting you as the owner of #methods'],
    ['remove_member', { channel: '#methods', member: '@bob' }, 'Removing @bob from #methods'],
    [
      'files_share',
      { channel: '#general', path: '~/results/plot.png' },
      'Sharing plot.png in #general',
    ],
    [
      'files_reference',
      { channel: '#general', path: '/data/lab/results.csv' },
      'Posting results.csv from the server in #general',
    ],
    ['tasks_ask', { channel: '#qc', prompt: 'Summarize this week.' }, 'Starting a task in #qc'],
    ['tasks_cancel', { task: RUN }, 'Stopping a task'],
    ['privacy_set_personal', { mode: 'private' }, 'Using Crew'],
  ])('%s %j reads “%s”', (action, params, label) => {
    expect(summarizeToolCall(change(action, params))).toBe(label);
  });

  it('prefers the name the daemon answered with to the one the call typed', () => {
    expect(
      summarizeToolCall(
        change('channels_create', { name: 'Methods!!', team: 'lab' }),
        answer({ created: true, name: 'methods' })
      )
    ).toBe('Creating #methods in lab');
  });

  it('conjugates a finished or failed change like every other row', async () => {
    const { toolRowLabel } = await import('./toolCallCopy');
    const label = summarizeToolCall(change('send', { channel: '#general' }));
    expect(toolRowLabel(label, 'done')).toBe('Posted in #general');
    expect(toolRowLabel(label, 'failed')).toBe('Failed to post in #general');
  });

  it('names a read, and never shows an ID or a hidden character', () => {
    expect(
      summarizeToolCall({
        name: 'crew__read',
        arguments: { action: 'history', params: { channel: '#gen‮eral' } },
      })
    ).toBe('Reading #general');
    expect(
      summarizeToolCall({
        name: 'crew__read',
        arguments: { action: 'search', params: { channel: '#methods', query: 'quota' } },
      })
    ).toBe('Searching #methods for quota');
    expect(summarizeToolCall({ name: 'crew__read', arguments: { action: 'status' } })).toBe(
      'Checking Crew'
    );
    expect(
      summarizeToolCall({ name: 'crew__read', arguments: { action: 'connections_preview' } })
    ).toBe('Reading Crew');
    const typedId = summarizeToolCall(change('channels_rename', { channel: TYPED, name: TYPED }));
    expect(typedId).toBe('Renaming a channel');
    expect(typedId).not.toContain(TYPED);
  });
});

describe('crewChangeTarget: built from the answer’s IDs only (WI-16)', () => {
  it('opens the channel and the message a post went to, never the channel the call typed', () => {
    const target = crewChangeTarget('send', {
      posted: true,
      connection_id: CONNECTION,
      channel_id: CHANNEL,
      message_id: MESSAGE,
    });
    expect(target).toEqual({ connectionId: CONNECTION, channelId: CHANNEL, messageId: MESSAGE });
    // What the call typed plays no part: the same answer gives the same target whatever it said.
    expect(JSON.stringify(target)).not.toContain(TYPED);
  });

  it('opens a task’s channel with its run, and offers its own chat', () => {
    const view = {
      run_id: RUN,
      connection_id: CONNECTION,
      channel_id: CHANNEL,
      session_id: '20261107_4',
      status: 'running',
    };
    expect(crewChangeTarget('tasks_ask', view)).toEqual({
      connectionId: CONNECTION,
      channelId: CHANNEL,
      runId: RUN,
    });
    expect(crewTaskChat('tasks_ask', view)).toBe('20261107_4');
    expect(crewTaskChat('send', view)).toBeNull();
  });

  it('opens a new team in its #general, and a deletion in the workspace only', () => {
    expect(
      crewChangeTarget('teams_create', {
        connection_id: CONNECTION,
        team_id: 't-new',
        general_channel_id: CHANNEL,
      })
    ).toEqual({ connectionId: CONNECTION, channelId: CHANNEL });
    expect(
      crewChangeTarget('channels_delete', {
        deleted: true,
        connection_id: CONNECTION,
        channel_id: CHANNEL,
      })
    ).toEqual({ connectionId: CONNECTION });
  });

  it('offers nothing without a connection, for a removed connection, or for IDs of the wrong shape', () => {
    expect(crewChangeTarget('send', { channel_id: CHANNEL, message_id: MESSAGE })).toBeNull();
    expect(crewChangeTarget('send', null)).toBeNull();
    expect(crewChangeTarget('connections_remove', { connection_id: CONNECTION })).toBeNull();
    expect(crewChangeTarget('workspace_delete', { connection_id: CONNECTION })).toBeNull();
    expect(
      crewChangeTarget('send', {
        connection_id: CONNECTION,
        channel_id: '#general <script>',
        message_id: MESSAGE,
      })
    ).toEqual({ connectionId: CONNECTION });
    expect(crewChangeTarget('send', { connection_id: 'x'.repeat(129) })).toBeNull();
  });

  it('reads a saved connection’s or a setup job’s connection when the answer has none at its top', () => {
    expect(crewChangeTarget('connect', { connection: { id: CONNECTION } })).toEqual({
      connectionId: CONNECTION,
    });
    expect(crewChangeTarget('setup', { outcome: { connection_id: CONNECTION } })).toEqual({
      connectionId: CONNECTION,
    });
  });
});

/** Where the router went, and with what state. */
let arrived: { path: string; state: unknown } | null = null;
function Arrived() {
  const location = useLocation();
  arrived = { path: `${location.pathname}${location.search}`, state: location.state };
  return <div data-testid="arrived">{location.pathname}</div>;
}

const request = (action: string, params: Record<string, unknown>): ToolRequestMessageContent => ({
  type: 'toolRequest',
  id: `tool-${action}`,
  toolCall: { status: 'success', value: change(action, params) },
});

/** The row in a chat (`chat_7`), or, `saved`, in a saved transcript, which names no chat. */
function renderRow(
  action: string,
  params: Record<string, unknown>,
  result: Record<string, unknown>,
  { saved = false }: { saved?: boolean } = {}
) {
  arrived = null;
  return render(
    <MemoryRouter initialEntries={['/transcript']}>
      <Routes>
        <Route
          path="/transcript"
          element={
            <ToolCallWithResponse
              sessionId={saved ? undefined : 'chat_7'}
              isCancelledMessage={false}
              toolRequest={request(action, params)}
              toolResponse={{ type: 'toolResponse', id: `tool-${action}`, toolResult: result }}
              onOpenArtifact={vi.fn()}
            />
          }
        />
        <Route path="*" element={<Arrived />} />
      </Routes>
    </MemoryRouter>
  );
}

describe('Open in Crew on a finished change (WI-16)', () => {
  it('opens Crew on the channel and message the answer named, with a fresh request', async () => {
    renderRow(
      'send',
      // The model typed another channel, by an ID: the link must not use it.
      { channel: TYPED, text: 'status update', connection: TYPED },
      answer({ posted: true, connection_id: CONNECTION, channel_id: CHANNEL, message_id: MESSAGE })
    );
    await userEvent.setup().click(screen.getByRole('button', { name: 'Open in Crew' }));
    expect(screen.getByTestId('arrived')).toHaveTextContent('/crew');
    expect(arrived?.path).toBe('/crew');
    const intent = crewSelectIntent(arrived?.state);
    expect(intent).toMatchObject({
      connectionId: CONNECTION,
      channelId: CHANNEL,
      messageId: MESSAGE,
    });
    expect(intent?.intentId).toBeTruthy();
    expect(JSON.stringify(arrived?.state)).not.toContain(TYPED);
    expect(Object.keys(arrived?.state as object)).toEqual([CREW_SELECT_ROUTE_KEY]);
  });

  it('opens a started task’s own chat with Open task chat', async () => {
    renderRow(
      'tasks_ask',
      { channel: '#qc', prompt: 'Summarize this week.' },
      answer({
        run_id: RUN,
        connection_id: CONNECTION,
        channel_id: CHANNEL,
        session_id: '20261107_4',
        status: 'starting',
      })
    );
    expect(screen.getByRole('button', { name: 'Open in Crew' })).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole('button', { name: 'Open task chat' }));
    expect(arrived?.path).toBe('/pair?resumeSessionId=20261107_4');
  });

  it('offers nothing for a refusal, a read, a saved transcript, or an answer without a connection', () => {
    const { unmount } = renderRow(
      'send',
      { channel: '#general' },
      { status: 'error', error: 'The person declined. Nothing was changed.' }
    );
    expect(screen.queryByTestId('crew-change-links')).toBeNull();
    unmount();

    const saved = renderRow(
      'send',
      { channel: '#general' },
      answer({ posted: true, connection_id: CONNECTION, channel_id: CHANNEL }),
      { saved: true }
    );
    expect(screen.queryByTestId('crew-change-links')).toBeNull();
    saved.unmount();

    const bare = renderRow('send', { channel: '#general' }, answer({ posted: true }));
    expect(screen.queryByTestId('crew-change-links')).toBeNull();
    bare.unmount();

    arrived = null;
    render(
      <MemoryRouter>
        <ToolCallWithResponse
          sessionId="chat_7"
          isCancelledMessage={false}
          toolRequest={{
            type: 'toolRequest',
            id: 'tool-read',
            toolCall: {
              status: 'success',
              value: { name: 'crew__read', arguments: { action: 'tasks_watch', params: {} } },
            },
          }}
          toolResponse={{
            type: 'toolResponse',
            id: 'tool-read',
            toolResult: answer({ task: { connection_id: CONNECTION }, connection_id: CONNECTION }),
          }}
          onOpenArtifact={vi.fn()}
        />
      </MemoryRouter>
    );
    expect(screen.queryByTestId('crew-change-links')).toBeNull();
  });
});
