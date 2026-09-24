import { act, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ChannelHeader } from './ChannelHeader';
import { channelCopy } from './copy';
import { channelHeaderCopy } from './headerCopy';
import {
  bob,
  currentCrew,
  general,
  installDaemon,
  installObserver,
  makeSnapshot,
  message,
  methods,
  renderCrew,
} from './crewTestHarness';

const mocks = vi.hoisted(() => ({
  crewHttp: vi.fn(),
  crewRequest: vi.fn(),
  observeCrew: vi.fn(),
}));

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return {
    ...actual,
    crewHttp: mocks.crewHttp,
    crewRequest: mocks.crewRequest,
    observeCrew: mocks.observeCrew,
  };
});

const { menu } = channelCopy;
const EVERYONE = [
  menu.details,
  menu.members,
  menu.files,
  menu.access,
  menu.markRead,
  menu.refresh,
  menu.copyName,
  menu.copyId,
];
const OWNER_ONLY = [menu.addPeople, menu.transfer, menu.archive];

function Layout({ canRename = false }: { canRename?: boolean }) {
  return <ChannelHeader canRename={canRename} />;
}

async function openMenu(user: ReturnType<typeof userEvent.setup>, slug = 'general') {
  await user.click(await screen.findByRole('button', { name: channelHeaderCopy.menuName(slug) }));
  return screen.findByRole('menu');
}

async function choose(user: ReturnType<typeof userEvent.setup>, item: string) {
  await openMenu(user);
  await user.click(await screen.findByRole('menuitem', { name: item }));
}

function itemNames() {
  return screen.getAllByRole('menuitem').map((item) => item.textContent);
}

beforeEach(() => {
  vi.clearAllMocks();
  installDaemon();
  installObserver({ messages: [message('41'), message('42')] });
});

describe('ChannelMenu items per role', () => {
  it('offers the owner every item, destructive last, and no Rename until names are unique', async () => {
    const user = userEvent.setup();
    renderCrew(() => <Layout />);
    await openMenu(user);
    const names = itemNames();
    expect(names).toEqual([
      menu.details,
      menu.members,
      menu.files,
      menu.access,
      menu.addPeople,
      menu.markRead,
      menu.refresh,
      menu.copyName,
      menu.copyId,
      menu.transfer,
      menu.archive,
    ]);
    expect(screen.getByRole('menuitem', { name: menu.archive })).toHaveAttribute(
      'data-variant',
      'destructive'
    );
  });

  it('adds Rename… for the owner when the broker advertises unique names', async () => {
    const user = userEvent.setup();
    renderCrew(() => <Layout canRename />);
    await openMenu(user);
    expect(itemNames()).toContain(menu.rename);
    expect(itemNames().indexOf(menu.rename)).toBeLessThan(itemNames().indexOf(menu.transfer));
  });

  it('shows a member who is not the owner only the shared items', async () => {
    const user = userEvent.setup();
    installObserver({ snapshot: makeSnapshot({ actor: bob }), messages: [message('41')] });
    renderCrew(() => <Layout canRename />);
    await openMenu(user);
    expect(itemNames()).toEqual(EVERYONE);
    for (const item of [...OWNER_ONLY, menu.rename]) {
      expect(screen.queryByRole('menuitem', { name: item })).toBeNull();
    }
  });

  it('hides the owner’s tools on an archived channel', async () => {
    const user = userEvent.setup();
    installObserver({
      snapshot: makeSnapshot({ channels: [{ ...general, archived: true }, methods] }),
    });
    renderCrew(() => <Layout canRename />);
    await screen.findByRole('button', { name: channelHeaderCopy.menuName('methods') });
    act(() => currentCrew().selectChannel(general.id));
    await openMenu(user);
    expect(itemNames()).toEqual(EVERYONE);
  });
});

describe('ChannelMenu actions', () => {
  it('marks the channel read at the latest message with channel.read and never refreshes', async () => {
    const user = userEvent.setup();
    renderCrew(() => <Layout />);
    await waitFor(() => expect(currentCrew().messages).toHaveLength(2));
    const observations = mocks.observeCrew.mock.calls.length;
    const connectionLoads = mocks.crewHttp.mock.calls.filter(
      ([path]) => path === '/connections'
    ).length;

    await choose(user, menu.markRead);

    await waitFor(() =>
      expect(mocks.crewRequest).toHaveBeenCalledWith(
        'conn-1',
        'channel.read',
        { channel_id: general.id, sequence: '42' },
        true
      )
    );
    expect(mocks.observeCrew.mock.calls.length).toBe(observations);
    expect(mocks.crewHttp.mock.calls.filter(([path]) => path === '/connections').length).toBe(
      connectionLoads
    );
    expect(currentCrew().error).toBeNull();
  });

  it('cannot mark read before any message has loaded', async () => {
    const user = userEvent.setup();
    installObserver({ messages: [] });
    renderCrew(() => <Layout />);
    await openMenu(user);
    expect(screen.getByRole('menuitem', { name: menu.markRead })).toHaveAttribute(
      'aria-disabled',
      'true'
    );
  });

  it('routes a failed mark-read to the connection bar source', async () => {
    const user = userEvent.setup();
    mocks.crewRequest.mockImplementation(async (_id: string, method: string) => {
      if (method === 'channel.read') throw new Error('read refused');
      return {};
    });
    renderCrew(() => <Layout />);
    await waitFor(() => expect(currentCrew().messages).toHaveLength(2));
    await choose(user, menu.markRead);
    await waitFor(() =>
      expect(currentCrew().error).toEqual({ message: 'read refused', source: 'global' })
    );
  });

  it('refreshes the channel and keeps an error that still needs attention', async () => {
    const user = userEvent.setup();
    renderCrew(() => <Layout />);
    await waitFor(() => expect(currentCrew().messagesLoaded).toBe(true));
    act(() => currentCrew().reportError('start failed', 'pane:agent'));
    const observations = mocks.observeCrew.mock.calls.length;
    await choose(user, menu.refresh);
    await waitFor(() => expect(mocks.observeCrew.mock.calls.length).toBeGreaterThan(observations));
    expect(currentCrew().error?.message).toBe('start failed');
  });

  it.each([
    [menu.details, 'about'],
    [menu.members, 'members'],
    [menu.files, 'files'],
    [menu.access, 'access'],
  ] as const)('%s opens the details pane on its tab', async (item, tab) => {
    const user = userEvent.setup();
    renderCrew(() => <Layout />);
    await choose(user, item);
    expect(currentCrew().ui.pane).toEqual({ mode: 'details', tab });
  });

  it('opens the owner dialogs by intent, focusing the trigger first so it is their opener', async () => {
    const user = userEvent.setup();
    renderCrew(() => <Layout canRename />);
    const trigger = await screen.findByRole('button', {
      name: channelHeaderCopy.menuName('general'),
    });

    await choose(user, menu.addPeople);
    expect(currentCrew().ui.dialog).toEqual({
      kind: 'add-people',
      target: 'channel',
      targetId: general.id,
    });
    expect(trigger).toHaveFocus();

    await choose(user, menu.rename);
    expect(currentCrew().ui.dialog).toEqual({
      kind: 'rename',
      target: 'channel',
      targetId: general.id,
    });

    await choose(user, menu.transfer);
    expect(currentCrew().ui.dialog).toEqual({ kind: 'transfer-ownership', channelId: general.id });

    await choose(user, menu.archive);
    expect(currentCrew().ui.dialog).toEqual({
      kind: 'confirm',
      confirm: { action: 'archive-channel', channelId: general.id },
    });
  });

  it('copies the channel name and ID without a toast', async () => {
    const user = userEvent.setup();
    renderCrew(() => <Layout />);
    await choose(user, menu.copyName);
    await waitFor(async () => expect(await navigator.clipboard.readText()).toBe('#general'));
    await choose(user, menu.copyId);
    await waitFor(async () => expect(await navigator.clipboard.readText()).toBe(general.id));
    // The header's one status region (Refresh channel's answer) stays empty.
    for (const status of screen.queryAllByRole('status')) expect(status).toBeEmptyDOMElement();
  });
});
