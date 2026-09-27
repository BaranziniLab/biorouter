import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, cleanup, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  alice,
  bob,
  carol,
  currentCrew,
  general,
  installDaemon,
  installObserver,
  makeSnapshot,
  methods,
  renderCrew,
} from '../channel/crewTestHarness';
import { membersCopy } from './copy';
import { MembersTab } from './MembersTab';
import { COPY_FEEDBACK_MS, MENU_COPY_CLOSE_MS } from './presentation';

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
const ghost = '6f1c2a3b-0000-4000-8000-000000000000';

function Members() {
  return (
    <div data-testid="members">
      <MembersTab />
    </div>
  );
}

async function shown() {
  await waitFor(() => expect(currentCrew().channel?.id).toBe(general.id));
  return screen.getByTestId('members');
}

/** `pane.css` without its comments, for the rules jsdom never applies. */
const paneCss = readFileSync(join(__dirname, 'pane.css'), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
function cssRule(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  return new RegExp(`(^|\\n)${escaped}\\s*\\{([^}]*)\\}`).exec(paneCss)?.[2] ?? '';
}

/**
 * Open a member menu's "Copy for support" submenu the keyboard's way (→ opens it and moves into
 * it) and return it, its one item focused. jsdom has no layout, so Radix's pointer grace area
 * cannot tell a pointer on its way into the submenu from one leaving it; Enter chooses.
 */
async function openSupport(user: ReturnType<typeof userEvent.setup>, menu: HTMLElement) {
  const trigger = within(menu).getByRole('menuitem', { name: membersCopy.copyForSupport });
  expect(trigger).toHaveAttribute('aria-haspopup', 'menu');
  act(() => trigger.focus());
  await user.keyboard('{ArrowRight}');
  await waitFor(() => expect(screen.getAllByRole('menu')).toHaveLength(2));
  const support = screen.getAllByRole('menu')[1];
  await waitFor(() => expect(within(support).getAllByRole('menuitem')[0]).toHaveFocus());
  return support;
}

/** Each row's text as a reader gets it: decorative avatars (aria-hidden) left out. */
function rows(tab: HTMLElement) {
  return within(within(tab).getByRole('list', { name: membersCopy.listLabel('#general') }))
    .getAllByRole('listitem')
    .map((item) => {
      const copy = item.cloneNode(true) as HTMLElement;
      copy.querySelectorAll('[aria-hidden="true"]').forEach((node) => node.remove());
      return copy.textContent;
    });
}

beforeEach(() => {
  vi.clearAllMocks();
  installDaemon();
  installObserver();
});

describe('MembersTab', () => {
  it('lists the channel’s members — not the team’s people — owner first, with markers', async () => {
    renderCrew(Members);
    const tab = await shown();
    expect(within(tab).getByText(membersCopy.count(2))).toBeInTheDocument();
    expect(rows(tab)).toEqual([`Alice Chen (@alice) · you${membersCopy.owner}`, 'Bob Lee (@bob)']);
    // The channel's owner, never a bare "Owner" that reads as the workspace's Host (Q2-69).
    expect(membersCopy.owner).toBe('Channel owner');
    expect(tab.textContent).not.toContain('Carol');
    expect(tab.textContent).not.toMatch(UUID);
  });

  it('lets the owner add people, make someone owner, or remove them — through dialogs', async () => {
    const user = userEvent.setup();
    renderCrew(Members);
    const tab = await shown();

    await user.click(within(tab).getByRole('button', { name: membersCopy.addPeople }));
    expect(currentCrew().ui.dialog).toEqual({
      kind: 'add-people',
      target: 'channel',
      targetId: general.id,
    });

    await user.click(within(tab).getByRole('button', { name: membersCopy.more('Bob Lee (@bob)') }));
    await user.click(await screen.findByRole('menuitem', { name: membersCopy.makeOwner }));
    expect(currentCrew().ui.dialog).toEqual({
      kind: 'transfer-ownership',
      channelId: general.id,
      successorId: bob.id,
    });

    await user.click(within(tab).getByRole('button', { name: membersCopy.more('Bob Lee (@bob)') }));
    await user.click(await screen.findByRole('menuitem', { name: membersCopy.remove('#general') }));
    expect(currentCrew().ui.dialog).toEqual({
      kind: 'confirm',
      confirm: { action: 'remove-channel-member', channelId: general.id, principalId: bob.id },
    });
  });

  it('offers Copy username at the top and Copy person ID under Copy for support (T-33, Q3-26)', async () => {
    const user = userEvent.setup();
    renderCrew(Members);
    const tab = await shown();
    const more = within(tab).getByRole('button', { name: membersCopy.more('Alice Chen (@alice)') });
    await user.click(more);
    const menu = await screen.findByRole('menu');
    expect(
      within(menu)
        .getAllByRole('menuitem')
        .map((item) => item.textContent)
    ).toEqual([membersCopy.copyUsername, membersCopy.copyForSupport]);
    // The username goes without its `@`, as Workspace settings copies it.
    await user.click(within(menu).getByRole('menuitem', { name: membersCopy.copyUsername }));
    await waitFor(async () => expect(await navigator.clipboard.readText()).toBe('alice'));
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull(), {
      timeout: MENU_COPY_CLOSE_MS + 1000,
    });

    await user.click(more);
    const support = await openSupport(user, await screen.findByRole('menu'));
    expect(
      within(support)
        .getAllByRole('menuitem')
        .map((item) => item.textContent)
    ).toEqual([membersCopy.copyPersonId]);
    await user.keyboard('{Enter}');
    await waitFor(async () => expect(await navigator.clipboard.readText()).toBe(alice.id));
    // The item answers "Copied", then the whole menu closes and hands focus back to the ⋯.
    expect(
      await within(support).findByRole('menuitem', { name: membersCopy.copied })
    ).toHaveAttribute('data-crew-copy-state', 'copied');
    await waitFor(() => expect(screen.queryAllByRole('menu')).toHaveLength(0), {
      timeout: MENU_COPY_CLOSE_MS + 1000,
    });
    await waitFor(() => expect(more).toHaveFocus());
  });

  it('answers a copy on the item: "Copied" for a moment, then the menu closes (Q2-34)', async () => {
    const user = userEvent.setup();
    renderCrew(Members);
    const tab = await shown();
    const more = within(tab).getByRole('button', { name: membersCopy.more('Bob Lee (@bob)') });
    await user.click(more);
    const menu = await screen.findByRole('menu');
    await user.click(within(menu).getByRole('menuitem', { name: membersCopy.copyUsername }));

    // The menu stays open and the item itself says so…
    const copied = await within(menu).findByRole('menuitem', { name: membersCopy.copied });
    expect(copied).toHaveAttribute('data-crew-copy-state', 'copied');
    expect(screen.getByRole('menu')).toBe(menu);
    expect(await navigator.clipboard.readText()).toBe('bob');
    // …the same result is spoken, from a region the open menu does not hide…
    const region = tab.querySelector('[aria-live="polite"]') as HTMLElement;
    expect(region).toHaveTextContent(membersCopy.copied);
    expect(region.closest('[aria-hidden="true"]')).toBeNull();
    // …and then the menu closes by itself, handing focus back to the ⋯.
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull(), {
      timeout: MENU_COPY_CLOSE_MS + 1000,
    });
    await waitFor(() => expect(more).toHaveFocus());

    // Opened again, the item reads as before.
    await user.click(more);
    expect(
      await screen.findByRole('menuitem', { name: membersCopy.copyUsername })
    ).toBeInTheDocument();
  });

  it('says a refused copy on the item and keeps the menu open', async () => {
    const user = userEvent.setup();
    const refuse = vi
      .spyOn(navigator.clipboard, 'writeText')
      .mockRejectedValue(new Error('denied'));
    renderCrew(Members);
    const tab = await shown();
    await user.click(within(tab).getByRole('button', { name: membersCopy.more('Bob Lee (@bob)') }));
    const menu = await screen.findByRole('menu');
    const support = await openSupport(user, menu);
    await user.keyboard('{Enter}');
    expect(
      await within(support).findByRole('menuitem', { name: membersCopy.copyFailed })
    ).toHaveAttribute('data-crew-copy-state', 'failed');
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, MENU_COPY_CLOSE_MS + 100));
    });
    expect(screen.getAllByRole('menu')).toEqual([menu, support]);
    // The label comes back; the menu is the person's to close.
    await waitFor(
      () =>
        expect(
          within(support).getByRole('menuitem', { name: membersCopy.copyPersonId })
        ).toBeInTheDocument(),
      { timeout: COPY_FEEDBACK_MS + 1000 }
    );
    expect(screen.getAllByRole('menu')[0]).toBe(menu);
    refuse.mockRestore();
  });

  it('puts the owner’s actions after Copy username, and Copy for support last (Q3-26)', async () => {
    const user = userEvent.setup();
    renderCrew(Members);
    const tab = await shown();
    await user.click(within(tab).getByRole('button', { name: membersCopy.more('Bob Lee (@bob)') }));
    const menu = await screen.findByRole('menu');
    expect(
      within(menu)
        .getAllByRole('menuitem')
        .map((item) => item.textContent)
    ).toEqual([
      membersCopy.copyUsername,
      membersCopy.makeOwner,
      membersCopy.remove('#general'),
      membersCopy.copyForSupport,
    ]);
    // The support submenu is last, after a separator, and holds the machine ID and nothing else.
    const support = within(menu).getByRole('menuitem', { name: membersCopy.copyForSupport });
    expect(support).toHaveAttribute('aria-haspopup', 'menu');
    expect(menu.lastElementChild).toBe(support);
    expect(support.previousElementSibling).toHaveAttribute('role', 'separator');
    expect(within(menu).queryByRole('menuitem', { name: membersCopy.copyPersonId })).toBeNull();
    const submenu = await openSupport(user, menu);
    expect(
      within(submenu)
        .getAllByRole('menuitem')
        .map((item) => item.textContent)
    ).toEqual([membersCopy.copyPersonId]);
  });

  it('opens the owner’s menu for an unknown member on Make owner…, never on a separator', async () => {
    const user = userEvent.setup();
    installObserver({
      snapshot: makeSnapshot({
        principals: [alice, bob],
        channels: [{ ...general, members: [alice.id, bob.id, ghost] }, methods],
      }),
    });
    renderCrew(Members);
    const tab = await shown();
    // An unknown member has no username, but the owner can still manage them, so the row keeps
    // its ⋯ — and the menu starts with the owner's actions, not a rule with nothing above it.
    await user.click(within(tab).getByRole('button', { name: membersCopy.more('Unknown member') }));
    const menu = await screen.findByRole('menu');
    const lines = Array.from(menu.children).map((child) =>
      child.getAttribute('role') === 'separator' ? '—' : child.textContent
    );
    expect(lines).toEqual([
      membersCopy.makeOwner,
      membersCopy.remove('#general'),
      '—',
      membersCopy.copyForSupport,
    ]);
    expect(menu.firstElementChild).not.toHaveAttribute('role', 'separator');
    expect(within(menu).queryByRole('menuitem', { name: membersCopy.copyUsername })).toBeNull();
    const submenu = await openSupport(user, menu);
    expect(
      within(submenu)
        .getAllByRole('menuitem')
        .map((item) => item.textContent)
    ).toEqual([membersCopy.copyPersonId]);
  });

  it('never draws a ⋯ that holds only Copy person ID (T-33)', async () => {
    const user = userEvent.setup();
    // A member who is not the owner, looking at a named member, a former one and an unknown one.
    installObserver({
      snapshot: makeSnapshot({
        actor: bob,
        principals: [alice, bob],
        former_principals: [{ id: carol.id, username: 'carol', display_name: 'Carol Diaz' }],
        channels: [{ ...general, members: [alice.id, bob.id, carol.id, ghost] }, methods],
      }),
    });
    renderCrew(Members);
    const tab = await shown();
    const list = within(tab).getByRole('list', { name: membersCopy.listLabel('#general') });
    const items = within(list).getAllByRole('listitem');
    expect(items).toHaveLength(4);

    // The unknown member has nothing to copy but an ID, so the row has no menu at all.
    const unknown = items.find((item) => item.textContent?.includes('Unknown member'));
    expect(unknown).toBeDefined();
    expect(within(unknown as HTMLElement).queryByRole('button')).toBeNull();

    // Every menu that is drawn holds something besides the ID.
    const triggers = within(list).getAllByRole('button', { name: /^More actions for/ });
    expect(triggers).toHaveLength(3);
    for (const trigger of triggers) {
      await user.click(trigger);
      const menu = await screen.findByRole('menu');
      const entries = within(menu)
        .getAllByRole('menuitem')
        .map((item) => item.textContent);
      expect(entries).not.toEqual([membersCopy.copyPersonId]);
      expect(entries[0]).toBe(membersCopy.copyUsername);
      await user.keyboard('{Escape}');
      await waitFor(() => expect(screen.queryByRole('menu')).toBeNull());
    }
  });

  it('tells a member who cannot add people who can, instead of showing nothing', async () => {
    const user = userEvent.setup();
    installObserver({ snapshot: makeSnapshot({ actor: bob }) });
    renderCrew(Members);
    const tab = await shown();
    expect(within(tab).queryByRole('button', { name: membersCopy.addPeople })).toBeNull();
    expect(within(tab).getByText(/^Ask/)).toHaveTextContent(
      'Ask Alice Chen (@alice) to add people.'
    );
    await user.click(
      within(tab).getByRole('button', { name: membersCopy.more('Alice Chen (@alice)') })
    );
    const menu = await screen.findByRole('menu');
    expect(within(menu).queryByRole('menuitem', { name: membersCopy.makeOwner })).toBeNull();
    expect(within(menu).queryByRole('menuitem', { name: /Remove from/ })).toBeNull();
  });

  it('names former and unknown members without ever printing an ID', async () => {
    const user = userEvent.setup();
    installObserver({
      snapshot: makeSnapshot({
        principals: [alice, bob],
        former_principals: [{ id: carol.id, username: 'carol', display_name: 'Carol Diaz' }],
        channels: [{ ...general, members: [alice.id, carol.id, ghost] }, methods],
      }),
    });
    renderCrew(Members);
    const tab = await shown();
    expect(within(tab).getByText(membersCopy.count(3))).toBeInTheDocument();
    const list = rows(tab);
    expect(list).toContain('Carol Diaz (@carol) · former member');
    expect(list).toContain('Unknown member');
    expect(tab.textContent).not.toMatch(UUID);
    // A former member is not offered Make owner or Remove; their ID stays behind Copy for
    // support, after the username, so the menu never holds the ID alone.
    await user.click(
      within(tab).getByRole('button', {
        name: membersCopy.more('Carol Diaz (@carol) · former member'),
      })
    );
    const menu = await screen.findByRole('menu');
    expect(
      within(menu)
        .getAllByRole('menuitem')
        .map((item) => item.textContent)
    ).toEqual([membersCopy.copyUsername, membersCopy.copyForSupport]);
  });

  it('shows channel invitations the viewer sent as muted invited rows', async () => {
    installObserver({
      snapshot: makeSnapshot({
        invitations: [
          {
            id: 'invitation-1',
            kind: 'channel',
            target_id: general.id,
            principal_id: carol.id,
            inviter_id: alice.id,
            expires_at: Date.now() / 1000 + 3600,
          },
          {
            id: 'invitation-2',
            kind: 'channel',
            target_id: general.id,
            principal_id: carol.id,
            inviter_id: alice.id,
            expires_at: 1,
            expired: true,
          },
        ],
      }),
    });
    renderCrew(Members);
    const tab = await shown();
    expect(rows(tab)).toEqual([
      `Alice Chen (@alice) · you${membersCopy.owner}`,
      'Bob Lee (@bob)',
      `Carol Diaz (@carol)${membersCopy.invited}`,
    ]);
    // The count is the channel's members; an invitation is not a member yet.
    expect(within(tab).getByText(membersCopy.count(2))).toBeInTheDocument();
  });

  describe('order (Q3-32)', () => {
    const erin = {
      id: '6f1c2a3b-0000-4000-8000-00000000e41e',
      uid: 1004,
      username: 'erin',
      nickname: 'Zoe Wu',
    };
    const aaron = {
      id: '6f1c2a3b-0000-4000-8000-00000000aa01',
      uid: 1005,
      username: 'aaron',
      nickname: 'aaron Park',
    };
    const samB = {
      id: '6f1c2a3b-0000-4000-8000-0000000005b0',
      uid: 1006,
      username: 'sam_b',
      nickname: 'Sam Park',
    };
    const samA = {
      id: '6f1c2a3b-0000-4000-8000-0000000005a0',
      uid: 1007,
      username: 'sam_a',
      nickname: 'Sam Park',
    };

    function install(you: Record<string, unknown> & { id: string }) {
      installObserver({
        snapshot: makeSnapshot({
          actor: you,
          principals: [alice, bob, you, aaron, samB, samA],
          former_principals: [{ id: carol.id, username: 'carol', display_name: 'Carol Diaz' }],
          channels: [
            {
              ...general,
              // Listed out of order on purpose, the former member and an unknown one first.
              members: [carol.id, ghost, samB.id, bob.id, you.id, aaron.id, alice.id, samA.id],
            },
            methods,
          ],
        }),
      });
    }

    const names = (tab: HTMLElement) =>
      rows(tab).map((row) => (row ?? '').replace(membersCopy.owner, ''));

    it('lists the owner, then you, then everyone by name, then unknown and former members', async () => {
      install(erin);
      renderCrew(Members);
      const tab = await shown();
      expect(names(tab)).toEqual([
        'Alice Chen (@alice)',
        'Zoe Wu (@erin) · you',
        // By the name shown, whatever its case…
        'aaron Park (@aaron)',
        'Bob Lee (@bob)',
        // …and, for one name, by username.
        'Sam Park (@sam_a)',
        'Sam Park (@sam_b)',
        'Unknown member',
        'Carol Diaz (@carol) · former member',
      ]);
    });

    it('sorts a person without a chosen name by the handle shown, its "@" ignored (Q4-32)', async () => {
      // Carol's #general: three people who never chose a name read "@crew_bob" and so on. The
      // contract is owner, you, then the visible name with a leading "@" ignored, case aside, then
      // the username — so a handle sorts among the names by its letters, never ahead of them all
      // because "@" sorts before a letter.
      const person = (suffix: string, username: string, nickname?: string) => ({
        id: `6f1c2a3b-0000-4000-8000-0000000d${suffix}`,
        uid: 2000 + Number.parseInt(suffix, 16),
        username,
        ...(nickname ? { nickname } : {}),
      });
      const you = person('0001', 'crew_carol', 'Carol Nguyen');
      const crewBob = person('0002', 'crew_bob');
      const crewDave = person('0003', 'crew_dave');
      const crewFrank = person('0004', 'crew_frank');
      const erinWu = person('0005', 'crew_erin', 'Erin Wu');
      const benOrtiz = person('0006', 'crew_ben', 'Ben Ortiz');
      installObserver({
        snapshot: makeSnapshot({
          actor: you,
          principals: [alice, you, crewBob, crewDave, crewFrank, erinWu, benOrtiz],
          channels: [
            {
              ...general,
              members: [
                erinWu.id,
                crewFrank.id,
                crewDave.id,
                you.id,
                crewBob.id,
                benOrtiz.id,
                alice.id,
              ],
            },
            methods,
          ],
        }),
      });
      renderCrew(Members);
      const tab = await shown();
      expect(names(tab)).toEqual([
        'Alice Chen (@alice)',
        'Carol Nguyen (@crew_carol) · you',
        // "Ben Ortiz" before "@crew_bob": with the "@" counted, every handle would lead.
        'Ben Ortiz (@crew_ben)',
        '@crew_bob',
        '@crew_dave',
        '@crew_frank',
        'Erin Wu (@crew_erin)',
      ]);
    });

    it('never moves you when you set your display name', async () => {
      // Before: `@erin`, which sorts among the others' names. After: "Zoe Wu", which would sort
      // last. Both times the row comes straight after the owner (Erin's moved from 5th to last).
      for (const you of [{ ...erin, nickname: undefined }, erin]) {
        install(you);
        renderCrew(Members);
        const tab = await shown();
        expect(names(tab)[1]).toMatch(/· you$/);
        cleanup();
      }
    });
  });

  describe('row layout (Q3-33)', () => {
    it('puts the Channel owner badge on its own line under the name, never beside a cut handle', async () => {
      renderCrew(Members);
      const tab = await shown();
      const badge = within(tab).getByText(membersCopy.owner);
      const row = badge.closest('li') as HTMLElement;
      const name = row.querySelector('.crew-member-name') as HTMLElement;
      // The badge and the name share one column, the badge after the name…
      expect(badge.parentElement).toBe(name.parentElement);
      expect(name.parentElement).toHaveClass('flex-col');
      expect(name.nextElementSibling).toBe(badge);
      // …and no name in the list is truncated: it wraps (`pane.css`).
      for (const item of within(tab).getAllByRole('listitem')) {
        expect(item.querySelector('.truncate')).toBeNull();
      }
      expect(cssRule('.crew-member-name')).toMatch(/overflow-wrap:\s*anywhere;/);
      expect(cssRule('.crew-member-name')).not.toMatch(/text-overflow|white-space:\s*nowrap/);
    });

    it('hides each row’s ⋯ at rest and shows it on hover, focus and while its menu is open', async () => {
      const user = userEvent.setup();
      renderCrew(Members);
      const tab = await shown();
      const more = within(tab).getByRole('button', { name: membersCopy.more('Bob Lee (@bob)') });
      expect(more).toHaveClass('crew-member-actions');
      expect(more.closest('li')).toHaveClass('crew-member-row');
      // Opacity only (design.md §4.14): still a tab stop, still in the accessibility tree.
      expect(cssRule('.crew-member-actions')).toMatch(/opacity:\s*0;/);
      expect(cssRule('.crew-member-actions')).not.toMatch(/display|visibility|pointer-events/);
      expect(paneCss).toMatch(
        /\.crew-member-row:hover \.crew-member-actions,\s*\.crew-member-row:focus-within \.crew-member-actions,\s*\.crew-member-actions\[data-state='open'\]\s*\{\s*opacity:\s*1;/
      );
      expect(paneCss).toMatch(
        /@media \(hover: none\)\s*\{\s*\.crew-member-actions\s*\{\s*opacity:\s*1;/
      );
      expect(paneCss).toMatch(
        /prefers-reduced-motion:\s*reduce[\s\S]*\.crew-member-actions\s*\{\s*transition:\s*none;/
      );
      // The open state the rule keys on is the trigger's own.
      await user.click(more);
      await screen.findByRole('menu');
      expect(more).toHaveAttribute('data-state', 'open');
    });
  });
});
