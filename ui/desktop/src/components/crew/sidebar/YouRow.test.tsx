import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { sidebarCopy } from './copy';
import { SidebarAnnouncer } from './SidebarAnnouncer';
import { COPY_FEEDBACK_MS } from './YouMenu';
import { YouRow } from './YouRow';
import { connection, makeController, renderWithCrew } from './sidebarTestUtils';

const copy = sidebarCopy.you;

function renderYou(controller = makeController()) {
  return renderWithCrew(
    <SidebarAnnouncer>
      <YouRow />
    </SidebarAnnouncer>,
    controller
  );
}

function stubAppConfig(values: Record<string, unknown>) {
  Object.defineProperty(window, 'appConfig', {
    configurable: true,
    writable: true,
    value: { get: (key: string) => values[key], getAll: () => values },
  });
}

afterEach(() => {
  Reflect.deleteProperty(window, 'appConfig');
  vi.restoreAllMocks();
});

describe('YouRow', () => {
  it('renders the SSH login as its own standalone text node, with a snapshot', () => {
    renderYou();
    // The regression tests' anchor: exactly this text, in an element of its own.
    const login = screen.getByText('alice@hpc.ucsf.edu');
    expect(login.childNodes).toHaveLength(1);
    expect(login.firstChild?.nodeType).toBe(Node.TEXT_NODE);
    expect(login).toHaveClass('font-mono');
    // …beside who I act as: my name, then @username as its own element.
    const name = document.querySelector('[data-person-context="header"]') as HTMLElement;
    expect(name).toHaveTextContent('Alice Chen @alice');
    expect(name.querySelector('[data-person-part="username"]')).toHaveTextContent('@alice');
  });

  it('renders the SSH login alone, with a placeholder avatar, before any snapshot', () => {
    renderYou(
      makeController({
        snapshot: null,
        observedPrivacy: null,
        effectivePrivacy: null,
        connection: { ...connection, ssh_target: 'fixture' },
      })
    );
    const login = screen.getByText('fixture');
    expect(login.childNodes).toHaveLength(1);
    expect(document.querySelector('[data-person-context]')).toBeNull();
    const avatar = document.querySelector('[data-slot="avatar"]');
    expect(avatar).toHaveAttribute('aria-hidden', 'true');
    expect(avatar).toHaveTextContent('');
  });

  it('keeps a dev profile’s badge off the row and in the You menu’s header (T-71)', async () => {
    const user = userEvent.setup();
    stubAppConfig({ BIOROUTER_DEV_PROFILE_NAME: 'alice' });
    renderYou();
    const trigger = screen.getByRole('button', { name: /alice@hpc\.ucsf\.edu/ });
    // The row keeps its width for the name, and its name carries no development detail.
    expect(screen.queryByText(copy.devProfile('alice'))).toBeNull();
    expect(trigger).not.toHaveAccessibleName(/Profile:/);

    await user.click(trigger);
    const menu = await screen.findByRole('menu');
    const header = menu.querySelector('[data-crew-menu-header]') as HTMLElement;
    const badge = within(header).getByText(copy.devProfile('alice')).parentElement as HTMLElement;
    expect(badge.className).toContain('bg-background-medium');
  });

  it('shows no profile badge outside a dev profile', async () => {
    const user = userEvent.setup();
    stubAppConfig({});
    renderYou();
    await user.click(screen.getByRole('button', { name: /alice@hpc\.ucsf\.edu/ }));
    await screen.findByRole('menu');
    expect(screen.queryByText(/^Profile:/)).toBeNull();
  });

  it('opens the You menu upward, with the profile, keys and username items', async () => {
    const user = userEvent.setup();
    const controller = makeController();
    renderYou(controller);
    const trigger = screen.getByRole('button', { name: /alice@hpc\.ucsf\.edu/ });
    expect(trigger).toHaveAttribute('aria-haspopup', 'menu');
    expect(trigger).toHaveClass('no-drag');
    // A real menu looks like one: a trailing chevron, hidden from assistive technology, that the
    // shared rule turns 180° by the trigger's own open state.
    const chevron = trigger.querySelector(':scope > .crew-sidebar-chevron');
    expect(chevron).not.toBeNull();
    expect(chevron).toHaveAttribute('data-turn', 'half');
    expect(chevron).toHaveAttribute('aria-hidden', 'true');
    expect(trigger.lastElementChild).toBe(chevron);
    expect(trigger).toHaveAttribute('data-state', 'closed');
    await user.click(trigger);
    const menu = await screen.findByRole('menu');
    expect(trigger).toHaveAttribute('data-state', 'open');
    expect(menu).toHaveAttribute('data-side', 'top');
    expect(
      within(menu)
        .getAllByRole('menuitem')
        .map((item) => item.textContent)
    ).toEqual([copy.editProfile, copy.keys, copy.copyUsername]);

    await user.click(within(menu).getByRole('menuitem', { name: copy.editProfile }));
    expect(controller.openDialog).toHaveBeenCalledWith({ kind: 'edit-profile' });
  });

  it('Keys and security… opens its dialog, even before a snapshot', async () => {
    const user = userEvent.setup();
    const controller = makeController({ snapshot: null, observedPrivacy: null });
    renderYou(controller);
    await user.click(screen.getByRole('button', { name: /alice@hpc\.ucsf\.edu/ }));
    const menu = await screen.findByRole('menu');
    expect(within(menu).getByRole('menuitem', { name: copy.editProfile })).toHaveAttribute(
      'aria-disabled',
      'true'
    );
    expect(within(menu).getByRole('menuitem', { name: copy.copyUsername })).toHaveAttribute(
      'aria-disabled',
      'true'
    );
    // A disabled item says why, above the items and in the menu's description (T-71).
    const note = menu.querySelector('[data-crew-menu-note]') as HTMLElement;
    expect(note).toHaveTextContent(sidebarCopy.unavailable.notVerified);
    expect(menu).toHaveAttribute('aria-describedby', note.id);
    expect(
      within(menu).getByRole('menuitem', { name: copy.editProfile })
    ).toHaveAccessibleDescription(sidebarCopy.unavailable.notVerified);
    await user.click(within(menu).getByRole('menuitem', { name: copy.keys }));
    expect(controller.openDialog).toHaveBeenCalledWith({ kind: 'keys' });
  });

  it('tells a joiner the profile items open once they join', async () => {
    const user = userEvent.setup();
    renderYou(makeController({ snapshot: null, observedPrivacy: null, status: 'not-joined' }));
    await user.click(screen.getByRole('button', { name: /alice@hpc\.ucsf\.edu/ }));
    const menu = await screen.findByRole('menu');
    expect(menu.querySelector('[data-crew-menu-note]')).toHaveTextContent(
      'Available after you join'
    );
  });

  it('gives no reason while every item is available', async () => {
    const user = userEvent.setup();
    renderYou();
    await user.click(screen.getByRole('button', { name: /alice@hpc\.ucsf\.edu/ }));
    const menu = await screen.findByRole('menu');
    expect(menu.querySelector('[data-crew-menu-note]')).toBeNull();
    expect(menu).not.toHaveAttribute('aria-describedby');
  });

  it('Copy my username copies the bare username and confirms on the item, without a toast', async () => {
    const user = userEvent.setup();
    const writeText = vi.spyOn(navigator.clipboard, 'writeText').mockResolvedValue(undefined);
    const controller = makeController();
    renderYou(controller);
    await user.click(screen.getByRole('button', { name: /alice@hpc\.ucsf\.edu/ }));
    const menu = await screen.findByRole('menu');
    await user.click(within(menu).getByRole('menuitem', { name: copy.copyUsername }));
    expect(writeText).toHaveBeenCalledWith('alice');
    // The menu stays open and the item itself says so…
    expect(await within(menu).findByRole('menuitem', { name: copy.copiedUsername })).toBeVisible();
    expect(screen.getByRole('menu')).toBe(menu);
    // …and the same result is spoken.
    await waitFor(() =>
      expect(document.querySelector('[data-crew-sidebar-announcer]')).toHaveTextContent(
        copy.announceCopiedUsername('alice')
      )
    );
    expect(controller.reportError).not.toHaveBeenCalled();
    // The label comes back.
    await waitFor(
      () => expect(within(menu).getByRole('menuitem', { name: copy.copyUsername })).toBeVisible(),
      { timeout: COPY_FEEDBACK_MS + 1000 }
    );
  });

  it('shows a refused copy on the item, never in the channel’s connection bar', async () => {
    const user = userEvent.setup();
    vi.spyOn(navigator.clipboard, 'writeText').mockRejectedValue(new Error('denied'));
    const controller = makeController();
    renderYou(controller);
    await user.click(screen.getByRole('button', { name: /alice@hpc\.ucsf\.edu/ }));
    const menu = await screen.findByRole('menu');
    await user.click(within(menu).getByRole('menuitem', { name: copy.copyUsername }));
    expect(
      await within(menu).findByRole('menuitem', { name: copy.copyUsernameFailed })
    ).toBeVisible();
    await waitFor(() =>
      expect(document.querySelector('[data-crew-sidebar-announcer]')).toHaveTextContent(
        copy.announceCopyUsernameFailed('alice')
      )
    );
    expect(controller.reportError).not.toHaveBeenCalled();
  });

  it('renders nothing until a connection is selected', () => {
    const { container } = renderYou(makeController({ connection: null }));
    expect(container.querySelector('[data-crew-you]')).toBeNull();
  });
});
