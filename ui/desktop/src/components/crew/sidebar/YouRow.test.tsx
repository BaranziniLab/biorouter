import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { sidebarCopy } from './copy';
import { SidebarAnnouncer } from './SidebarAnnouncer';
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

  it('adds a neutral "Profile: {name}" badge in a dev profile', () => {
    stubAppConfig({ BIOROUTER_DEV_PROFILE_NAME: 'alice' });
    renderYou();
    const badge = screen.getByText(copy.devProfile('alice')).parentElement as HTMLElement;
    expect(badge.className).toContain('bg-background-medium');
  });

  it('shows no profile badge outside a dev profile', () => {
    stubAppConfig({});
    renderYou();
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
    await user.click(within(menu).getByRole('menuitem', { name: copy.keys }));
    expect(controller.openDialog).toHaveBeenCalledWith({ kind: 'keys' });
  });

  it('Copy my username copies the bare username and confirms without a toast', async () => {
    const user = userEvent.setup();
    const writeText = vi.spyOn(navigator.clipboard, 'writeText').mockResolvedValue(undefined);
    renderYou();
    await user.click(screen.getByRole('button', { name: /alice@hpc\.ucsf\.edu/ }));
    const menu = await screen.findByRole('menu');
    await user.click(within(menu).getByRole('menuitem', { name: copy.copyUsername }));
    expect(writeText).toHaveBeenCalledWith('alice');
    await waitFor(() =>
      expect(document.querySelector('[data-crew-sidebar-announcer]')).toHaveTextContent(
        sidebarCopy.clipboard.copied
      )
    );
  });

  it('renders nothing until a connection is selected', () => {
    const { container } = renderYou(makeController({ connection: null }));
    expect(container.querySelector('[data-crew-you]')).toBeNull();
  });
});
