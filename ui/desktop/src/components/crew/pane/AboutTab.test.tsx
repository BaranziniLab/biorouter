import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  bob,
  currentCrew,
  general,
  installDaemon,
  installObserver,
  makeSnapshot,
  methods,
  renderCrew,
} from '../channel/crewTestHarness';
import { AboutTab } from './AboutTab';
import { aboutCopy } from './copy';

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

const UUID = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i;

function About({ canRename = false }: { canRename?: boolean }) {
  return (
    <div data-testid="about">
      <AboutTab canRename={canRename} />
    </div>
  );
}

async function shown() {
  await waitFor(() => expect(currentCrew().channel?.id).toBe(general.id));
  return screen.getByTestId('about');
}

beforeEach(() => {
  vi.clearAllMocks();
  installDaemon();
  installObserver();
});

describe('AboutTab', () => {
  it('names the channel, its content rule, owner, creator and team without an ID', async () => {
    renderCrew(() => <About />);
    const about = await shown();
    expect(within(about).getByText('#general')).toBeInTheDocument();
    expect(within(about).getByText(aboutCopy.restricted)).toBeInTheDocument();
    expect(within(about).getByText(aboutCopy.restrictedHint)).toBeInTheDocument();
    expect(within(about).getAllByText('Alice Chen')).toHaveLength(2);
    expect(within(about).getByText('Analysis Lab')).toBeInTheDocument();
    expect(about.textContent).not.toMatch(UUID);
  });

  it('gives the owner Transfer ownership…, the danger zone and Copy channel ID', async () => {
    const user = userEvent.setup();
    renderCrew(() => <About />);
    const about = await shown();
    expect(within(about).queryByRole('button', { name: aboutCopy.renameName })).toBeNull();

    await user.click(within(about).getByRole('button', { name: aboutCopy.transfer }));
    expect(currentCrew().ui.dialog).toEqual({ kind: 'transfer-ownership', channelId: general.id });

    expect(within(about).getByRole('heading', { name: aboutCopy.dangerZone })).toBeInTheDocument();
    await user.click(within(about).getByRole('button', { name: aboutCopy.archive }));
    expect(currentCrew().ui.dialog).toEqual({
      kind: 'confirm',
      confirm: { action: 'archive-channel', channelId: general.id },
    });

    await user.click(within(about).getByRole('button', { name: aboutCopy.copyId }));
    await waitFor(async () => expect(await navigator.clipboard.readText()).toBe(general.id));
    expect(about.textContent).not.toMatch(UUID);
  });

  it('offers Rename… to the owner once names are unique', async () => {
    const user = userEvent.setup();
    renderCrew(() => <About canRename />);
    const about = await shown();
    await user.click(within(about).getByRole('button', { name: aboutCopy.renameName }));
    expect(currentCrew().ui.dialog).toEqual({
      kind: 'rename',
      target: 'channel',
      targetId: general.id,
    });
  });

  it('shows a pending ownership offer at an authority point', async () => {
    installObserver({
      snapshot: makeSnapshot({ channels: [{ ...general, pending_owner: bob.id }, methods] }),
    });
    renderCrew(() => <About />);
    const about = await shown();
    const offer = within(about).getByText(aboutCopy.offeredTo, { exact: false });
    expect(offer).toHaveTextContent('Offered to Bob Lee (@bob) · waiting');
  });

  it('gives a member who is not the owner the facts and no owner tools', async () => {
    installObserver({ snapshot: makeSnapshot({ actor: bob }) });
    renderCrew(() => <About canRename />);
    const about = await shown();
    for (const name of [aboutCopy.transfer, aboutCopy.archive, aboutCopy.renameName]) {
      expect(within(about).queryByRole('button', { name })).toBeNull();
    }
    expect(within(about).queryByText(aboutCopy.dangerZone)).toBeNull();
    expect(within(about).getByRole('button', { name: aboutCopy.copyId })).toBeInTheDocument();
  });

  it('reads a public-safe channel’s rule', async () => {
    renderCrew(() => <About />);
    await shown();
    act(() => currentCrew().selectChannel(methods.id));
    await waitFor(() => expect(currentCrew().channel?.id).toBe(methods.id));
    expect(screen.getByText(aboutCopy.publicSafe)).toBeInTheDocument();
    expect(screen.getByText(aboutCopy.publicSafeHint)).toBeInTheDocument();
  });
});
