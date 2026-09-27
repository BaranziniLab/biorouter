import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { CrewMessage } from '../crewApi';
import { installResizeObserverStub } from '../test/crewTestUtils';
import { channelReady, general, ids, installDaemon, renderCrew, richSnapshot } from './harness';

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: vi.fn(), crewRequest: vi.fn(), observeCrew: vi.fn() };
});
// Stable across renders, as the real context's callbacks are.
const config = vi.hoisted(() => ({
  getProviders: async () => [],
  read: async () => '',
  getProviderModels: async () => [],
}));
vi.mock('../../ConfigContext', async () => {
  const actual = await vi.importActual<typeof import('../../ConfigContext')>('../../ConfigContext');
  return { ...actual, useConfig: () => config };
});
vi.mock('../CrewAuthentication', () => ({ default: () => <div /> }));

installResizeObserverStub();

/**
 * Q3-05: in a channel with shared files, the reply path was 8–10 Tab stops, and a different
 * number each pass. Every file card's Save and ⋯ were always Tab stops, and tabbing into one made
 * its message the active row, which pulled that row's Copy text and ⋯ in too. So one Tab from the
 * log landed on "Save attachment" — a person expecting the composer typed a space into a button.
 *
 * A card's controls now follow the row's own rule: Tab stops only while their message is the
 * active row (arrowed or clicked to). This mounts the real layout over two file messages.
 */

const now = Math.floor(Date.now() / 1000);

function fileMessages(): CrewMessage[] {
  const base = {
    channel_id: ids.general,
    restricted: false,
    source_channels: [ids.general],
  };
  return [
    {
      ...base,
      id: ids.messages[0],
      sequence: ids.messages[0],
      actor_id: ids.bob,
      body: 'Here is the plate reader file.',
      created_at: now - 600,
      attachments: [ids.blob],
    },
    {
      ...base,
      id: ids.messages[1],
      sequence: ids.messages[1],
      actor_id: ids.alice,
      body: 'And the same run again.',
      created_at: now - 300,
      attachments: [ids.device],
    },
  ];
}

const composer = () => screen.getByRole('textbox', { name: 'Message #general' });

/** Tab until the composer has focus, and say how many presses it took. */
async function tabsToComposer(user: ReturnType<typeof userEvent.setup>): Promise<number> {
  let stops = 0;
  while (document.activeElement !== composer() && stops < 20) {
    await user.tab();
    stops += 1;
  }
  expect(document.activeElement).toBe(composer());
  return stops;
}

describe('file cards and the Tab order through a channel (Q3-05)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // Bob owns #general, so the channel intro offers the viewer no "Add people" of its own.
    installDaemon({
      snapshot: richSnapshot({ channels: [{ ...general, owner_id: ids.bob }] }),
      messages: fileMessages(),
    });
  });

  async function openChannel() {
    renderCrew();
    await channelReady();
    // Both cards have loaded their files.
    await waitFor(() => expect(screen.getAllByText('counts.csv')).toHaveLength(2));
    return userEvent.setup({ pointerEventsCheck: 0 });
  }

  it('goes from the log to the composer in one stop, past both cards', async () => {
    const user = await openChannel();
    const log = screen.getByRole('log');
    act(() => log.focus());
    expect(await tabsToComposer(user)).toBe(1);
  });

  it('goes from the channel header to the composer in at most three stops', async () => {
    const user = await openChannel();
    act(() => screen.getByRole('button', { name: 'Channel details' }).focus());
    expect(await tabsToComposer(user)).toBeLessThanOrEqual(3);
  });

  it('comes back from the composer to the log in one Shift+Tab, not onto the last card', async () => {
    const user = await openChannel();
    act(() => composer().focus());
    await user.tab({ shift: true });
    expect(screen.getByRole('log')).toHaveFocus();
  });

  it('puts a file message’s Save and ⋯ in reach once it is arrowed to, and only its own', async () => {
    const user = await openChannel();
    const log = screen.getByRole('log');
    act(() => log.focus());
    // Up from the log lands on the newest message: the second file.
    fireEvent.keyDown(log, { key: 'ArrowUp' });
    const row = document.activeElement as HTMLElement;
    expect(row).toHaveAttribute('data-crew-row');
    expect(row).toHaveTextContent('And the same run again.');

    const reached: string[] = [];
    while (document.activeElement !== composer() && reached.length < 10) {
      await user.tab();
      reached.push(document.activeElement?.getAttribute('aria-label') ?? '');
    }
    // The row's toolbar, then the card's own controls, then the composer — never the other
    // card's. The toolbar sits above the card on screen, so Tab reaches it first: it used to go
    // Save, ⋯, then back up to Copy text and More actions (Q4-22).
    const saves = reached.filter((name) => name.startsWith('Save counts.csv'));
    const cardMenus = reached.filter((name) => name.startsWith('More actions for counts.csv'));
    expect(saves).toHaveLength(1);
    expect(cardMenus).toHaveLength(1);
    expect(within(row).getByRole('button', { name: saves[0] })).toBeInTheDocument();
    expect(within(row).getByRole('button', { name: cardMenus[0] })).toBeInTheDocument();
    const copyText = reached.findIndex((name) => name.startsWith('Copy text of '));
    const rowMenu = reached.findIndex(
      (name) => name.startsWith('More actions for ') && !name.startsWith('More actions for counts')
    );
    expect(copyText).toBeGreaterThanOrEqual(0);
    expect(rowMenu).toBeGreaterThan(copyText);
    expect(reached.indexOf(saves[0])).toBeGreaterThan(rowMenu);
    expect(reached.indexOf(cardMenus[0])).toBeGreaterThan(reached.indexOf(saves[0]));
    expect(reached[reached.length - 1]).toBe('Message #general');

    // Same-named files are told apart by their post time in those names (Q3-13).
    const other = screen
      .getAllByRole('button', { name: /^Save counts\.csv/, hidden: true })
      .find((button) => !row.contains(button)) as HTMLElement;
    expect(saves[0]).not.toBe(other.getAttribute('aria-label'));
    expect(other.tabIndex).toBe(-1);
  });
});
