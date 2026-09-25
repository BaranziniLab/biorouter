import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { installResizeObserverStub } from '../test/crewTestUtils';
import {
  channelReady,
  ids,
  keys,
  installDaemon,
  MACHINE_ID_PATTERNS,
  ownedRun,
  renderCrew,
  richMessages,
} from './harness';

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

/** Attributes whose values reach a person: read aloud, shown on hover, or shown in a field. */
const PERCEIVABLE_ATTRIBUTES = [
  'aria-label',
  'aria-description',
  'aria-valuetext',
  'aria-placeholder',
  'title',
  'alt',
  'placeholder',
  'href',
] as const;

/**
 * Everything the page says, however it says it: its text, every attribute a person or a screen
 * reader can meet, and the values of its fields. `data-*` hooks and element ids are keys, never
 * shown, so they are not read.
 */
function perceivable(): string {
  const parts: string[] = [document.body.textContent ?? ''];
  document.body.querySelectorAll('*').forEach((element) => {
    for (const name of PERCEIVABLE_ATTRIBUTES) {
      const value = element.getAttribute(name);
      if (value) parts.push(value);
    }
    if (element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement)
      parts.push(element.value);
  });
  return parts.join('\n');
}

function leaks(): string[] {
  const text = perceivable();
  return MACHINE_ID_PATTERNS.flatMap((pattern) => {
    const match = pattern.exec(text);
    if (!match) return [];
    const at = match.index;
    return [`${pattern} in …${text.slice(Math.max(0, at - 80), at + 60)}…`];
  });
}

/**
 * Every machine-ID copy sits in a "Copy for support" submenu, last in its menu (Q3-26). With the
 * menu open, this goes into that submenu by keyboard, as a person would, and chooses `item`.
 */
async function chooseForSupport(user: ReturnType<typeof userEvent.setup>, item: string) {
  const support = await screen.findByRole('menuitem', { name: 'Copy for support' });
  act(() => support.focus());
  await user.keyboard('{ArrowRight}');
  await user.click(await screen.findByRole('menuitem', { name: item }));
}

/** The row of the timeline that holds `text`. */
function rowOf(text: string): HTMLElement {
  const row = screen.getByText(text).closest<HTMLElement>('[data-crew-row]');
  if (!row) throw new Error(`No timeline row holds “${text}”.`);
  return row;
}

describe('no machine IDs by default (naming rule 8)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    installDaemon({ runs: [ownedRun], messages: richMessages() });
  });

  async function openRichWorkspace() {
    renderCrew();
    await channelReady();
    // A former member's message, a file, the viewer's agent task, an invitation to the viewer, a
    // person waiting to join (the viewer hosts), and the About tab in the details pane.
    await screen.findByText('Counts are in.');
    await screen.findByText('counts.csv');
    expect(screen.getByText('Imaging Core')).toBeInTheDocument();
    expect(screen.getByText('@dave')).toBeInTheDocument();
    expect(screen.getAllByText(/Carol Diaz/).length).toBeGreaterThan(0);
    expect(screen.getByRole('button', { name: 'More task actions', hidden: true })).toBeVisible();
    // Row actions are revealed on hover (`pointer-events: none` at rest); a person hovers first.
    const user = userEvent.setup({ pointerEventsCheck: 0 });
    await user.click(screen.getByRole('button', { name: 'Channel details' }));
    // About keeps the channel's ID behind a quiet "IDs for support" disclosure (Q3-26).
    await screen.findByRole('button', { name: 'IDs for support' });
    return user;
  }

  it('renders no UUID, no 64-hex key and no server UID anywhere in the default view', async () => {
    await openRichWorkspace();
    expect(screen.getByRole('complementary')).toBeInTheDocument();
    expect(leaks()).toEqual([]);
  });

  it('sees an ID wherever a person could meet it, so the scan above cannot pass by not looking', async () => {
    await openRichWorkspace();
    const probe = document.createElement('span');
    document.body.append(probe);
    try {
      for (const [attribute, value] of [
        ['aria-label', ids.run],
        ['title', `uid ${70301}`],
        ['placeholder', 'aa11bb22cc33dd44'.repeat(4)],
      ] as const) {
        probe.setAttribute(attribute, value);
        expect(leaks()).not.toEqual([]);
        probe.removeAttribute(attribute);
      }
      probe.textContent = ids.general;
      expect(leaks()).not.toEqual([]);
    } finally {
      probe.remove();
    }
    expect(leaks()).toEqual([]);
  });

  it('hands each ID over only through its own Copy … ID, and never draws it', async () => {
    const user = await openRichWorkspace();
    const writeText = vi.spyOn(navigator.clipboard, 'writeText');

    // The channel: the About tab's "IDs for support".
    await user.click(screen.getByRole('button', { name: 'IDs for support' }));
    await user.click(await screen.findByRole('button', { name: 'Copy channel ID' }));
    expect(writeText).toHaveBeenLastCalledWith(ids.general);

    // The channel again: its menu, "Copy for support".
    await user.click(screen.getByRole('button', { name: /^#general, channel menu$/ }));
    await chooseForSupport(user, 'Copy channel ID');
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(ids.general));
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull(), { timeout: 2000 });

    // A message by a former member: its row's ⋯.
    await user.click(
      within(rowOf('Counts are in.')).getByRole('button', {
        name: /^More actions for /,
        hidden: true,
      })
    );
    await chooseForSupport(user, 'Copy message ID');
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(ids.messages[0]));
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull(), { timeout: 2000 });

    // A file: its card's ⋯, both the file's ID and its checksum.
    await user.click(
      screen.getAllByRole('button', { name: /^More actions for counts\.csv/, hidden: true })[0]
    );
    await chooseForSupport(user, 'Copy file ID');
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(ids.blob));
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull(), { timeout: 2000 });
    await user.click(
      screen.getAllByRole('button', { name: /^More actions for counts\.csv/, hidden: true })[0]
    );
    await chooseForSupport(user, 'Copy SHA-256');
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(keys.sha256));
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull(), { timeout: 2000 });

    // The viewer's task: the status row's ⋯.
    await user.click(screen.getByRole('button', { name: 'More task actions', hidden: true }));
    await chooseForSupport(user, 'Copy task ID');
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(ids.run));
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull(), { timeout: 2000 });

    // A person: the Members tab row's ⋯.
    await user.click(screen.getByRole('tab', { name: 'Members' }));
    const members = await screen.findByRole('list', { name: '#general members' });
    await user.click(within(members).getByRole('button', { name: /Bob Lee/ }));
    await chooseForSupport(user, 'Copy person ID');
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(ids.bob));
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull(), { timeout: 2000 });

    // The team: its section's options menu in the sidebar.
    await user.click(screen.getByRole('button', { name: 'Analysis Lab options' }));
    await chooseForSupport(user, 'Copy team ID');
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(ids.team));

    // Copying never renders what it copied.
    await user.keyboard('{Escape}');
    expect(leaks()).toEqual([]);
  });
});
