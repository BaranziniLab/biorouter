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
import { COPY_FEEDBACK_MS } from './presentation';

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

/** Open "IDs for support" and return Copy channel ID, which waits behind it (Q3-26). */
async function openIds(user: ReturnType<typeof userEvent.setup>, about: HTMLElement) {
  await user.click(within(about).getByRole('button', { name: aboutCopy.idsForSupport }));
  return within(about).findByRole('button', { name: aboutCopy.copyId });
}

beforeEach(() => {
  vi.clearAllMocks();
  installDaemon();
  installObserver();
});

describe('AboutTab', () => {
  it('names the channel, who can read it, owner, creator and team without an ID', async () => {
    renderCrew(() => <About />);
    const about = await shown();
    expect(within(about).getByText('#general')).toBeInTheDocument();
    expect(within(about).getByText(aboutCopy.restricted)).toBeInTheDocument();
    expect(within(about).getByText(aboutCopy.restrictedHint)).toBeInTheDocument();
    expect(within(about).getAllByText('Alice Chen')).toHaveLength(2);
    expect(within(about).getByText('Analysis Lab')).toBeInTheDocument();
    expect(about.textContent).not.toMatch(UUID);
  });

  it('names the owner and the creator in one format, the authority form (Q4-21)', async () => {
    renderCrew(() => <About />);
    const about = await shown();
    const value = (label: string) =>
      within(about).getByText(label, { selector: 'dt' }).nextElementSibling as HTMLElement;
    const owner = value(aboutCopy.owner);
    const createdBy = value(aboutCopy.createdBy);
    // It read "Alice Chen @alice" as Owner and "Alice Chen (@alice)" as Created by, one row
    // apart. Both are now `personLabel(…, 'authority')`, drawn the way a Members row is.
    for (const row of [owner, createdBy]) {
      expect(row.querySelector('[data-person-context]')).toHaveAttribute(
        'data-person-context',
        'authority'
      );
      expect(row).toHaveTextContent(/^Alice Chen \(@alice\)$/);
    }
    expect(createdBy.innerHTML).toBe(owner.innerHTML);
  });

  it('says who can read a Restricted channel in words, not "Content: Restricted" (T-67)', async () => {
    renderCrew(() => <About />);
    const about = await shown();
    const label = within(about).getByText(aboutCopy.whoCanRead);
    expect(label.tagName).toBe('DT');
    expect(label.nextElementSibling).toHaveTextContent(
      `${aboutCopy.restricted}${aboutCopy.restrictedHint}`
    );
    expect(aboutCopy.restricted).toBe('Private models only');
    expect(within(about).queryByText('Content')).toBeNull();
    expect(within(about).queryByText('Restricted')).toBeNull();
  });

  it('keeps Copy channel ID behind a quiet "IDs for support" disclosure (Q3-26)', async () => {
    const user = userEvent.setup();
    renderCrew(() => <About />);
    const about = await shown();
    // At rest the tab holds no machine-ID control at all, only the disclosure that leads to one.
    expect(within(about).queryByRole('button', { name: aboutCopy.copyId })).toBeNull();
    const ids = within(about).getByRole('button', { name: aboutCopy.idsForSupport });
    expect(aboutCopy.idsForSupport).toBe('IDs for support');
    expect(ids).toHaveAttribute('aria-expanded', 'false');
    // Quiet: the shared Disclosure's muted ghost trigger, never a filled button.
    expect(ids).toHaveClass('biorouter-disclosure-trigger', 'text-text-muted');
    const copy = await openIds(user, about);
    expect(ids).toHaveAttribute('aria-expanded', 'true');
    expect(ids.getAttribute('aria-controls')).toBeTruthy();
    expect(document.getElementById(ids.getAttribute('aria-controls') ?? '')).toContainElement(copy);
    expect(about.textContent).not.toMatch(UUID);
  });

  it('keeps Copy channel ID out of the danger zone, above it (T-67)', async () => {
    const user = userEvent.setup();
    renderCrew(() => <About />);
    const about = await shown();
    const zone = within(about)
      .getByRole('heading', { name: aboutCopy.dangerZone })
      .closest('section') as HTMLElement;
    const ids = within(about).getByRole('button', { name: aboutCopy.idsForSupport });
    expect(zone).not.toContainElement(ids);
    expect(ids.compareDocumentPosition(zone) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    const copy = await openIds(user, about);
    expect(zone).not.toContainElement(copy);
    expect(copy.compareDocumentPosition(zone) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    // Its words sit on the pane's one inset, not 12px inside it (T-62).
    expect(copy).toHaveClass('crew-pane-flush');
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

    await user.click(await openIds(user, about));
    await waitFor(async () => expect(await navigator.clipboard.readText()).toBe(general.id));
    expect(about.textContent).not.toMatch(UUID);
  });

  it('says "Copied" on Copy channel ID for a moment, and says it aloud (Q2-34)', async () => {
    const user = userEvent.setup();
    renderCrew(() => <About />);
    const about = await shown();
    const copy = await openIds(user, about);
    await user.click(copy);
    await waitFor(async () => expect(await navigator.clipboard.readText()).toBe(general.id));
    // The same node flips its words, so focus stays where it was.
    expect(copy).toHaveTextContent(aboutCopy.copied);
    expect(copy).toHaveAttribute('data-crew-copy-state', 'copied');
    expect(copy).toHaveFocus();
    const region = about.querySelector('[aria-live="polite"]') as HTMLElement;
    expect(region).toHaveTextContent(aboutCopy.copied);
    expect(about.textContent).not.toMatch(UUID);
    await waitFor(() => expect(copy).toHaveTextContent(aboutCopy.copyId), {
      timeout: COPY_FEEDBACK_MS + 1000,
    });
    expect(region).toHaveTextContent('');
    expect(COPY_FEEDBACK_MS).toBe(1500);
  });

  it('says so when the copy is refused', async () => {
    const user = userEvent.setup();
    const refuse = vi
      .spyOn(navigator.clipboard, 'writeText')
      .mockRejectedValue(new Error('denied'));
    renderCrew(() => <About />);
    const about = await shown();
    const copy = await openIds(user, about);
    await user.click(copy);
    await waitFor(() => expect(copy).toHaveTextContent(aboutCopy.copyFailed));
    expect(copy).toHaveAttribute('data-crew-copy-state', 'failed');
    refuse.mockRestore();
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
    const user = userEvent.setup();
    installObserver({ snapshot: makeSnapshot({ actor: bob }) });
    renderCrew(() => <About canRename />);
    const about = await shown();
    for (const name of [aboutCopy.transfer, aboutCopy.archive, aboutCopy.renameName]) {
      expect(within(about).queryByRole('button', { name })).toBeNull();
    }
    expect(within(about).queryByText(aboutCopy.dangerZone)).toBeNull();
    expect(await openIds(user, about)).toBeInTheDocument();
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
