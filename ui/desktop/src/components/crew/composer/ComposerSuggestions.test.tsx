import { createEvent, fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Snapshot } from '../crewApi';
import { crewTestController, CrewTestProvider, testChannel } from '../files/crewTestController';
import type { CrewController, CrewDraft } from '../state/types';
import { Composer } from './Composer';
import { composerCopy } from './copy';

vi.mock('../crewTransfers', () => ({
  beginTransfer: vi.fn(),
  listTransfers: vi.fn(async () => []),
  pauseTransfer: vi.fn(),
  resumeTransfer: vi.fn(),
  forgetTransfer: vi.fn(),
  previewAttachment: vi.fn(),
}));

/**
 * The composer's suggestion list (design OW-10): a leading `@` or `/` offers the agent, and an `@`
 * word offers the channel's people. It only ever inserts text — Enter starts a task once the list
 * has closed — and it leaves Enter to an input method that is composing (C15).
 */

const alice = { id: 'person-1', uid: 1000, username: 'alice', nickname: 'Alice Chen' };
const bob = {
  id: 'person-2',
  uid: 1001,
  username: 'bob',
  nickname: 'Bob Lee',
  display_name: 'Bob Lee',
};
const carol = { id: 'person-3', uid: 1002, username: 'carol', nickname: 'Carol Diaz' };
const agentPerson = { id: 'person-4', uid: 1003, username: 'agent', nickname: 'Agent Smith' };
const channel = { ...testChannel, members: [alice.id, bob.id, carol.id] };

function snapshot(principals = [alice, bob, carol]): Snapshot {
  return {
    workspace: { id: 'workspace-1', host_uid: 1000, mode: 'private', policy_epoch: 1 },
    actor: alice as Snapshot['actor'],
    principals: principals as Snapshot['principals'],
    teams: [],
    channels: [channel],
    invitations: [],
    runs: [],
  };
}

function Stateful({ overrides = {} }: { overrides?: Partial<CrewController> }) {
  const [draft, setDraft] = useState<CrewDraft>({ body: '', attachments: [], references: [] });
  const controller = crewTestController({
    snapshot: snapshot(),
    channel,
    draft,
    setBody: (body) => setDraft((current) => ({ ...current, body })),
    ...overrides,
  });
  return (
    <CrewTestProvider controller={controller}>
      <Composer />
    </CrewTestProvider>
  );
}

const input = () => screen.getByLabelText('Message #general') as HTMLTextAreaElement;
const list = () => screen.queryByRole('listbox', { name: composerCopy.suggestions });
const options = () => within(list() as HTMLElement).getAllByRole('option');

describe('the composer’s suggestion list', () => {
  let send: ReturnType<typeof vi.fn<() => Promise<void>>>;
  beforeEach(() => {
    send = vi.fn(async () => undefined);
  });

  it('offers the agent first on a leading @, then the channel’s people in the authority form', async () => {
    const user = userEvent.setup();
    render(<Stateful overrides={{ send }} />);
    await user.type(input(), '@');

    const rows = options();
    expect(rows[0]).toHaveTextContent('@agent');
    expect(rows[0]).toHaveTextContent('Your agent · posts in #general');
    // Never the composer button's name (C6): that query must find exactly one control.
    expect(screen.getAllByRole('button', { name: 'Ask my agent' })).toHaveLength(1);
    expect(rows.some((row) => row.textContent === 'Bob Lee (@bob)')).toBe(true);
    expect(rows.some((row) => row.textContent === 'Carol Diaz (@carol)')).toBe(true);
    expect(input()).toHaveAttribute('aria-controls', list()?.id);
    expect(input()).toHaveAttribute('aria-activedescendant', rows[0].id);
    expect(rows[0]).toHaveAttribute('aria-selected', 'true');
    // The textarea keeps its pinned name (C6).
    expect(input()).toHaveAccessibleName('Message #general');
  });

  it('offers /agent and /task on a leading /, narrowing as the word is typed', async () => {
    const user = userEvent.setup();
    render(<Stateful overrides={{ send }} />);
    await user.type(input(), '/');
    expect(
      options().map((row) => row.querySelector('.crew-compose-suggestion-label')?.textContent)
    ).toEqual(['/agent', '/task']);
    expect(options()[1]).toHaveTextContent('Start your agent on a task in #general');
    await user.type(input(), 't');
    expect(options()).toHaveLength(1);
    expect(options()[0]).toHaveTextContent('/task');
  });

  it('accepts a row with Enter or Tab by inserting it, and sends nothing', async () => {
    const user = userEvent.setup();
    render(<Stateful overrides={{ send }} />);
    await user.type(input(), '@ag');
    await user.keyboard('{Enter}');
    expect(input()).toHaveValue('@agent ');
    expect(input().selectionStart).toBe(7);
    expect(list()).toBeNull();
    expect(send).not.toHaveBeenCalled();

    await user.clear(input());
    await user.type(input(), 'thanks @b');
    await user.keyboard('{Tab}');
    expect(input()).toHaveValue('thanks @bob ');
    expect(send).not.toHaveBeenCalled();
  });

  it('moves with the arrow keys and accepts the row it is on', async () => {
    const user = userEvent.setup();
    render(<Stateful overrides={{ send }} />);
    await user.type(input(), '/');
    await user.keyboard('{ArrowDown}');
    expect(input()).toHaveAttribute('aria-activedescendant', options()[1].id);
    await user.keyboard('{ArrowDown}');
    expect(input()).toHaveAttribute('aria-activedescendant', options()[0].id);
    await user.keyboard('{ArrowUp}{Enter}');
    expect(input()).toHaveValue('/task ');
  });

  it('accepts a row by pointer without taking focus from the text', async () => {
    const user = userEvent.setup();
    render(<Stateful overrides={{ send }} />);
    await user.type(input(), 'cc @ca');
    await user.click(screen.getByRole('option', { name: 'Carol Diaz (@carol)' }));
    expect(input()).toHaveValue('cc @carol ');
    expect(input()).toHaveFocus();
  });

  it('closes on Escape until the word changes, and Enter then submits as usual', async () => {
    const user = userEvent.setup();
    render(<Stateful overrides={{ send }} />);
    await user.type(input(), 'hi @bo');
    expect(list()).not.toBeNull();
    await user.keyboard('{Escape}');
    expect(list()).toBeNull();
    expect(input()).not.toHaveAttribute('aria-controls');
    await user.keyboard('{Enter}');
    expect(send).toHaveBeenCalledTimes(1);
    await user.type(input(), 'b');
    expect(list()).not.toBeNull();
  });

  it('never opens for a path, so Enter on one sends', async () => {
    const user = userEvent.setup();
    render(<Stateful overrides={{ send }} />);
    await user.type(input(), '/pool1/data');
    expect(list()).toBeNull();
    await user.keyboard('{Enter}');
    expect(send).toHaveBeenCalledTimes(1);
  });

  it('ignores Enter and the arrows while an input method is composing (C15)', async () => {
    const user = userEvent.setup();
    render(<Stateful overrides={{ send }} />);
    await user.type(input(), '@');
    const composing = createEvent.keyDown(input(), {
      key: 'Enter',
      code: 'Enter',
      keyCode: 13,
      isComposing: true,
    });
    fireEvent(input(), composing);
    const process = createEvent.keyDown(input(), { key: 'ArrowDown', keyCode: 229 });
    fireEvent(input(), process);
    expect(composing.defaultPrevented).toBe(false);
    expect(process.defaultPrevented).toBe(false);
    expect(input()).toHaveValue('@');
    expect(input()).toHaveAttribute('aria-activedescendant', options()[0].id);
    expect(send).not.toHaveBeenCalled();
  });

  it('says @agent is a person called agent, and offers /agent for the agent', async () => {
    const user = userEvent.setup();
    render(
      <Stateful
        overrides={{
          send,
          snapshot: snapshot([alice, bob, agentPerson]),
          channel: { ...channel, members: [alice.id, bob.id, agentPerson.id] },
        }}
      />
    );
    await user.type(input(), '@');
    const first = options()[0];
    expect(first).toHaveTextContent('/agent');
    expect(first).toHaveTextContent(
      '@agent is Agent Smith (@agent). Type /agent to ask your agent.'
    );
    expect(screen.getByRole('option', { name: 'Agent Smith (@agent)' })).toBeInTheDocument();
    await user.keyboard('{Enter}');
    expect(input()).toHaveValue('/agent ');
  });

  it('closes when the text loses focus', async () => {
    const user = userEvent.setup();
    render(<Stateful overrides={{ send }} />);
    await user.type(input(), '@');
    expect(list()).not.toBeNull();
    await user.tab();
    expect(list()).toBeNull();
  });
});
