import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { forgetJoinContext, updateJoinContext } from '../onboarding/joinContext';
import { sidebarCopy } from './copy';
import { MENU_COPY_CLOSE_MS } from './menuCopy';
import { SidebarAnnouncer } from './SidebarAnnouncer';
import { COPY_FEEDBACK_MS } from './YouMenu';
import { YouRow } from './YouRow';
import { connection, makeController, makeSnapshot, renderWithCrew } from './sidebarTestUtils';

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
  vi.unstubAllEnvs();
  forgetJoinContext(connection.id);
});

const joiner = {
  snapshot: null,
  observedPrivacy: null,
  effectivePrivacy: null,
  status: 'not-joined',
  connection: { ...connection, ssh_target: 'crew_frank@52.33.141.141' },
} as const;

describe('YouRow', () => {
  it('says where I am, "on hpc.ucsf.edu", in the sans supporting style once a snapshot names me (Q4-50)', () => {
    renderYou();
    // Who I act as: my name, then @username as its own element…
    const name = document.querySelector('[data-person-context="header"]') as HTMLElement;
    expect(name).toHaveTextContent('Alice Chen @alice');
    expect(name.querySelector('[data-person-part="username"]')).toHaveTextContent('@alice');
    // …and where, under it: not the SSH login, which said the username a third time.
    const place = document.querySelector('[data-crew-you-place]') as HTMLElement;
    expect(place.textContent).toBe(`${copy.on} hpc.ucsf.edu`);
    expect(place).toHaveClass('text-supporting', 'text-text-muted');
    expect(place).not.toHaveClass('font-mono');
    expect(place.querySelector('.font-mono')).toBeNull();
    expect(screen.queryByText('alice@hpc.ucsf.edu')).toBeNull();
    expect(document.querySelector('[data-crew-you-login]')).toBeNull();
    // (jsdom lays out nothing, so it joins the row's two lines without the space a browser adds.)
    expect(screen.getByRole('button', { name: /^Alice Chen/ })).toHaveAccessibleName(
      /^Alice Chen @alice\s*on hpc\.ucsf\.edu$/
    );
  });

  it('keeps "on hpc.ucsf.edu" through a refresh that shows only the last verified copy', () => {
    const snapshot = makeSnapshot();
    renderYou(
      makeController({
        snapshot: null,
        observedPrivacy: null,
        effectivePrivacy: null,
        status: 'checking',
        lastVerified: {
          connectionId: connection.id,
          snapshot,
          observedPrivacy: {
            connectionId: connection.id,
            mode: 'private',
            institutionId: 'ucsf',
            policyEpoch: 1,
          },
          runs: [],
          labels: null,
          teamId: 'team-lab-0000',
          channelId: 'chan-methods',
          messages: [],
        },
      })
    );
    expect(document.querySelector('[data-crew-you-place]')?.textContent).toBe(
      `${copy.on} hpc.ucsf.edu`
    );
    expect(document.querySelector('[data-crew-you-login]')).toBeNull();
  });

  it('renders the SSH login alone, in monospace, with a placeholder avatar, before any snapshot', () => {
    renderYou(
      makeController({
        snapshot: null,
        observedPrivacy: null,
        effectivePrivacy: null,
        connection: { ...connection, ssh_target: 'fixture' },
      })
    );
    // The regression tests' anchor: exactly this text, in an element of its own.
    const login = screen.getByText('fixture');
    expect(login.childNodes).toHaveLength(1);
    expect(login.firstChild?.nodeType).toBe(Node.TEXT_NODE);
    expect(login).toHaveClass('font-mono');
    expect(document.querySelector('[data-crew-you-place]')).toBeNull();
    expect(document.querySelector('[data-person-context]')).toBeNull();
    const avatar = document.querySelector('[data-slot="avatar"]');
    expect(avatar).toHaveAttribute('aria-hidden', 'true');
    expect(avatar).toHaveTextContent('');
  });

  it('keeps a dev profile’s badge off the row and in the You menu’s header (T-71)', async () => {
    const user = userEvent.setup();
    stubAppConfig({ BIOROUTER_DEV_PROFILE_NAME: 'alice' });
    renderYou();
    const trigger = screen.getByRole('button', { name: /^Alice Chen/ });
    // The row keeps its width for the name, and its name carries no development detail.
    expect(screen.queryByText(copy.devProfile('alice'))).toBeNull();
    expect(trigger).not.toHaveAccessibleName(/Profile:/);

    await user.click(trigger);
    const menu = await screen.findByRole('menu');
    const header = menu.querySelector('[data-crew-menu-header]') as HTMLElement;
    const badge = within(header).getByText(copy.devProfile('alice')).parentElement as HTMLElement;
    expect(badge.className).toContain('bg-background-medium');
  });

  it('shows no profile badge in a built app, even one launched with a dev profile (Q2-43)', async () => {
    const user = userEvent.setup();
    vi.stubEnv('DEV', false);
    stubAppConfig({ BIOROUTER_DEV_PROFILE_NAME: 'frank' });
    renderYou();
    await user.click(screen.getByRole('button', { name: /^Alice Chen/ }));
    await screen.findByRole('menu');
    expect(screen.queryByText(/^Profile:/)).toBeNull();
  });

  it('shows no profile badge outside a dev profile', async () => {
    const user = userEvent.setup();
    stubAppConfig({});
    renderYou();
    await user.click(screen.getByRole('button', { name: /^Alice Chen/ }));
    await screen.findByRole('menu');
    expect(screen.queryByText(/^Profile:/)).toBeNull();
  });

  it('opens the You menu upward, with the profile, keys and username items', async () => {
    const user = userEvent.setup();
    const controller = makeController();
    renderYou(controller);
    const trigger = screen.getByRole('button', { name: /^Alice Chen/ });
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
    // The login names the username, so it can be copied before any snapshot (Q2-43).
    expect(within(menu).getByRole('menuitem', { name: copy.copyUsername })).not.toHaveAttribute(
      'aria-disabled'
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

  it('lets a joiner copy the username the login already names, and shows its initial (Q2-43)', async () => {
    const user = userEvent.setup();
    const writeText = vi.spyOn(navigator.clipboard, 'writeText').mockResolvedValue(undefined);
    renderYou(makeController(joiner));
    // Not a blank circle: the username is known, so the avatar has its initial.
    const avatar = document.querySelector('[data-slot="avatar"]') as HTMLElement;
    expect(avatar.textContent).toMatch(/^f$/i);
    await user.click(screen.getByRole('button', { name: /crew_frank@52\.33\.141\.141/ }));
    const menu = await screen.findByRole('menu');
    const item = within(menu).getByRole('menuitem', { name: copy.copyUsername });
    expect(item).not.toHaveAttribute('aria-disabled');
    await user.click(item);
    expect(writeText).toHaveBeenCalledWith('crew_frank');
    // Edit profile still waits for the join, and says so.
    expect(within(menu).getByRole('menuitem', { name: copy.editProfile })).toHaveAttribute(
      'aria-disabled',
      'true'
    );
  });

  it('takes a joiner’s username from the join when the login is an alias', async () => {
    const user = userEvent.setup();
    updateJoinContext(connection.id, { username: 'crew_frank' });
    renderYou(
      makeController({ ...joiner, connection: { ...connection, ssh_target: 'lab-server' } })
    );
    await user.click(screen.getByRole('button', { name: /lab-server/ }));
    const menu = await screen.findByRole('menu');
    expect(within(menu).getByRole('menuitem', { name: copy.copyUsername })).not.toHaveAttribute(
      'aria-disabled'
    );
  });

  it('disables Copy my username only while nothing names the username', async () => {
    const user = userEvent.setup();
    renderYou(
      makeController({ ...joiner, connection: { ...connection, ssh_target: 'lab-server' } })
    );
    await user.click(screen.getByRole('button', { name: /lab-server/ }));
    const menu = await screen.findByRole('menu');
    expect(within(menu).getByRole('menuitem', { name: copy.copyUsername })).toHaveAttribute(
      'aria-disabled',
      'true'
    );
  });

  it('names the server by the person’s own alias for it (D-ALIAS), on the row and in the menu', async () => {
    const user = userEvent.setup();
    const labelled = {
      ...connection,
      ssh_target: 'crew_alice@52.33.141.141',
      server_label: 'lab-server',
    };
    renderYou(makeController({ connection: labelled, connections: [labelled] }));
    const place = document.querySelector('[data-crew-you-place]') as HTMLElement;
    expect(place.textContent).toBe(`${copy.on} lab-server`);
    expect(screen.queryByText(/52\.33\.141\.141/)).toBeNull();
    expect(screen.queryByText('crew_alice@lab-server')).toBeNull();
    await user.click(screen.getByRole('button', { name: /^Alice Chen/ }));
    const menu = await screen.findByRole('menu');
    const header = menu.querySelector('[data-crew-menu-header]') as HTMLElement;
    expect(header.querySelector('[data-crew-you-place]')?.textContent).toBe(
      `${copy.on} lab-server`
    );
    expect(header).not.toHaveTextContent('crew_alice@lab-server');
  });

  it('shows the aliased login as the placeholder before any snapshot names me', () => {
    const labelled = {
      ...connection,
      ssh_target: 'crew_alice@52.33.141.141',
      server_label: 'lab-server',
    };
    renderYou(
      makeController({
        snapshot: null,
        observedPrivacy: null,
        effectivePrivacy: null,
        connection: labelled,
        connections: [labelled],
      })
    );
    const login = screen.getByText('crew_alice@lab-server');
    expect(login.childNodes).toHaveLength(1);
    expect(login).toHaveClass('font-mono');
    expect(screen.queryByText(/52\.33\.141\.141/)).toBeNull();
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
    await user.click(screen.getByRole('button', { name: /^Alice Chen/ }));
    const menu = await screen.findByRole('menu');
    expect(menu.querySelector('[data-crew-menu-note]')).toBeNull();
    expect(menu).not.toHaveAttribute('aria-describedby');
  });

  it('Copy my username copies the bare username, says "Copied", then closes like every menu (Q3-57)', async () => {
    const user = userEvent.setup();
    const writeText = vi.spyOn(navigator.clipboard, 'writeText').mockResolvedValue(undefined);
    const controller = makeController();
    renderYou(controller);
    const trigger = screen.getByRole('button', { name: /^Alice Chen/ });
    await user.click(trigger);
    const menu = await screen.findByRole('menu');
    await user.click(within(menu).getByRole('menuitem', { name: copy.copyUsername }));
    expect(writeText).toHaveBeenCalledWith('alice');
    // The item itself says so, in the menu that is still open…
    const item = await within(menu).findByRole('menuitem', { name: copy.copiedUsername });
    expect(item).toBeVisible();
    expect(screen.getByRole('menu')).toBe(menu);
    // …and the same result is spoken.
    await waitFor(() =>
      expect(document.querySelector('[data-crew-sidebar-announcer]')).toHaveTextContent(
        copy.announceCopiedUsername('alice')
      )
    );
    expect(controller.reportError).not.toHaveBeenCalled();
    // Then the menu closes by itself — the team and message menus' timing — still saying
    // "Copied" to the end, and focus goes back to the row.
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull(), {
      timeout: MENU_COPY_CLOSE_MS + 1000,
    });
    expect(item).toHaveTextContent(copy.copiedUsername);
    await waitFor(() => expect(trigger).toHaveFocus());
    // Opening it again gives the item its own words back.
    await user.click(trigger);
    const again = await screen.findByRole('menu');
    expect(within(again).getByRole('menuitem', { name: copy.copyUsername })).toBeVisible();
  });

  it('shows a refused copy on the item, never in the channel’s connection bar', async () => {
    const user = userEvent.setup();
    vi.spyOn(navigator.clipboard, 'writeText').mockRejectedValue(new Error('denied'));
    const controller = makeController();
    renderYou(controller);
    await user.click(screen.getByRole('button', { name: /^Alice Chen/ }));
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
    // A refused copy keeps the menu open, so the person can try again, and the label comes back.
    await new Promise((resolve) => setTimeout(resolve, MENU_COPY_CLOSE_MS + 100));
    expect(screen.getByRole('menu')).toBe(menu);
    await waitFor(
      () => expect(within(menu).getByRole('menuitem', { name: copy.copyUsername })).toBeVisible(),
      { timeout: COPY_FEEDBACK_MS + 1000 }
    );
  });

  it('renders nothing until a connection is selected', () => {
    const { container } = renderYou(makeController({ connection: null }));
    expect(container.querySelector('[data-crew-you]')).toBeNull();
  });
});
