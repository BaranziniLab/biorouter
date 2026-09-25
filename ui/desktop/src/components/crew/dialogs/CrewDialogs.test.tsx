import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../../ui/dropdown-menu';
import { Popover, PopoverContent, PopoverTrigger } from '../../ui/popover';
import { useCrew } from '../state/CrewControllerContext';
import type { DialogIntent } from '../state/types';
import { addPeopleCopy, sharePathCopy } from './copy';
import { CREW_DIALOG_DEFAULTS, CrewDialogs, dialogKey, HOSTED_DIALOG_KINDS } from './CrewDialogs';
import {
  alice,
  connection,
  installResizeObserverStub,
  makeSnapshot,
  renderWithCrew,
} from './dialogsTestHarness';

installResizeObserverStub();

beforeEach(() => {
  Object.assign(window, {
    electron: { ...window.electron, crewCredentials: vi.fn(async () => ({ cancelled: true })) },
  });
});
afterEach(() => vi.clearAllMocks());

/** A control that opens `intent` the way every Crew surface does: through the controller. */
function Opener({ intent, label = 'Open' }: { intent: DialogIntent; label?: string }) {
  const crew = useCrew();
  return (
    <button type="button" onClick={() => crew.openDialog(intent)}>
      {label}
    </button>
  );
}

/** One of every hosted dialog kind, opened as a person would reach it. */
const EVERY_KIND: DialogIntent[] = [
  { kind: 'connection-settings', connectionId: 'conn-1' },
  { kind: 'workspace-settings', tab: 'people' },
  { kind: 'invite-people' },
  { kind: 'let-in', username: 'eve' },
  { kind: 'create-team' },
  { kind: 'create-channel', teamId: 'team-1' },
  { kind: 'add-people', target: 'channel', targetId: 'channel-general' },
  { kind: 'transfer-ownership', channelId: 'channel-general' },
  { kind: 'rename', target: 'team', targetId: 'team-1' },
  { kind: 'edit-profile' },
  { kind: 'keys' },
  { kind: 'share-path' },
  { kind: 'confirm', confirm: { action: 'archive-channel', channelId: 'channel-general' } },
];

/** Each hosted dialog, and the control its first focus lands on. */
const FIRST_FIELDS: [DialogIntent, string][] = [
  [{ kind: 'connection-settings', connectionId: 'conn-1' }, 'Connection name'],
  [{ kind: 'invite-people' }, 'Username'],
  [{ kind: 'let-in', username: 'eve' }, 'Code from @eve'],
  [{ kind: 'create-team' }, 'Name'],
  [{ kind: 'create-channel', teamId: 'team-1' }, 'Name'],
  [{ kind: 'rename', target: 'team', targetId: 'team-1' }, 'Name'],
  [{ kind: 'edit-profile' }, 'Display name'],
  [{ kind: 'share-path' }, 'Path'],
];

describe('CrewDialogs', () => {
  it.each(FIRST_FIELDS)('opens %j on its first field', async (intent, label) => {
    renderWithCrew(<CrewDialogs />, {
      dialog: intent,
      snapshot: makeSnapshot({ pending_joins: [{ username: 'eve' }] }),
    });
    const field = await screen.findByLabelText(label);
    await waitFor(() => expect(field).toHaveFocus());
  });

  it('titles Share a path with the server’s alias, not its address (QA Q3-39)', async () => {
    const labelled = {
      ...connection,
      ssh_target: 'crew_dave@52.33.141.141',
      server_label: 'lab-server',
    };
    renderWithCrew(<CrewDialogs />, { dialog: { kind: 'share-path' }, connections: [labelled] });
    expect(
      await screen.findByRole('dialog', { name: sharePathCopy.title('lab-server') })
    ).toBeInTheDocument();
    expect(document.body.textContent).not.toContain('52.33.141.141');
    // The placeholder still starts in the person's own home, from the saved login.
    expect(screen.getByLabelText('Path')).toHaveAttribute('placeholder', '/home/crew_dave/…');
  });

  it('opens Add people on its checklist’s search', async () => {
    renderWithCrew(<CrewDialogs />, {
      dialog: { kind: 'add-people', target: 'channel', targetId: 'channel-general' },
    });
    const search = await screen.findByRole('searchbox', { name: addPeopleCopy.search });
    await waitFor(() => expect(search).toHaveFocus());
  });

  it('opens a destructive confirmation on Cancel', async () => {
    renderWithCrew(<CrewDialogs />, {
      dialog: {
        kind: 'confirm',
        confirm: { action: 'archive-channel', channelId: 'channel-general' },
      },
    });
    const cancel = await screen.findByRole('button', { name: 'Cancel' });
    await waitFor(() => expect(cancel).toHaveFocus());
  });

  it('closes through the controller, and renders nothing for dialogs other areas own', async () => {
    const { crew } = renderWithCrew(<CrewDialogs />, { dialog: { kind: 'keys' } });
    expect(await screen.findByRole('dialog', { name: 'Keys and security' })).toBeInTheDocument();
    act(() => crew.current().closeDialog());
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());

    for (const intent of [{ kind: 'join' }, { kind: 'host' }] as DialogIntent[]) {
      act(() => crew.current().openDialog(intent));
      expect(screen.queryByRole('dialog')).toBeNull();
    }
  });

  it('mounts a fresh dialog for a different intent of the same kind', async () => {
    const snapshot = makeSnapshot({
      pending_joins: [{ username: 'eve' }, { username: 'frank' }],
    });
    const { crew } = renderWithCrew(<CrewDialogs />, {
      dialog: { kind: 'let-in', username: 'eve' },
      snapshot,
    });
    expect(await screen.findByRole('dialog', { name: 'Let @eve into lab' })).toBeInTheDocument();
    act(() => crew.current().openDialog({ kind: 'let-in', username: 'frank' }));
    expect(await screen.findByRole('dialog', { name: 'Let @frank into lab' })).toBeInTheDocument();
    expect(dialogKey({ kind: 'let-in', username: 'eve' })).not.toBe(
      dialogKey({ kind: 'let-in', username: 'frank' })
    );
  });

  it('covers every hosted kind in the focus-return check below', () => {
    expect([...new Set(EVERY_KIND.map((intent) => intent.kind))].sort()).toEqual(
      [...HOSTED_DIALOG_KINDS].sort()
    );
  });

  // QA T-15: these dialogs have no Radix trigger, so focus fell to <body> on every close.
  it.each(EVERY_KIND.map((intent) => [intent.kind, intent] as const))(
    'returns focus to the button that opened %s when Escape closes it',
    async (_kind, intent) => {
      const user = userEvent.setup();
      renderWithCrew(
        <>
          <Opener intent={intent} />
          <CrewDialogs />
        </>,
        { snapshot: makeSnapshot({ pending_joins: [{ username: 'eve' }] }) }
      );
      const open = screen.getByRole('button', { name: 'Open' });
      await user.click(open);
      const role = intent.kind === 'confirm' ? 'alertdialog' : 'dialog';
      const dialog = await screen.findByRole(role);
      await waitFor(() => expect(dialog).toContainElement(document.activeElement as HTMLElement));
      await user.keyboard('{Escape}');
      await waitFor(() => expect(screen.queryByRole(role)).toBeNull());
      await waitFor(() => expect(open).toHaveFocus());
    }
  );

  it('returns focus to a menu’s trigger when a menu item opened the dialog', async () => {
    const user = userEvent.setup();
    function Menu() {
      const crew = useCrew();
      return (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button type="button">Channel menu</button>
          </DropdownMenuTrigger>
          <DropdownMenuContent>
            <DropdownMenuItem onSelect={() => crew.openDialog({ kind: 'edit-profile' })}>
              Edit profile…
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      );
    }
    renderWithCrew(
      <>
        <Menu />
        <CrewDialogs />
      </>
    );
    const trigger = screen.getByRole('button', { name: 'Channel menu' });
    await user.click(trigger);
    await user.keyboard('{ArrowDown}');
    await user.keyboard('{Enter}');
    await screen.findByRole('dialog', { name: 'Edit profile' });
    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    await waitFor(() => expect(trigger).toHaveFocus());
  });

  // QA Q2-27: a pointer, not only the keyboard — Workspace settings opened from the workspace menu
  // by pointer was reported to return focus to <body>. This is a guard, not the fix: the menu rule
  // already covered this path. Checked in the running round-2 app with real CDP pointer events
  // (move, press, release): People…, Privacy… and Connection settings… each returned focus to the
  // switcher, closed by Done, Cancel, × or Escape. What could still end on <body> was an opener
  // gone with no channel showing, which the switcher fallback in `focusReturn.ts` now covers.
  it('returns focus to the switcher when a pointer chose the workspace-menu item and closed the dialog', async () => {
    const user = userEvent.setup();
    function WorkspaceMenu() {
      const crew = useCrew();
      return (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button type="button" className="crew-sidebar-switcher">
              lab
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent>
            <DropdownMenuItem
              onSelect={() => crew.openDialog({ kind: 'workspace-settings', tab: 'people' })}
            >
              People…
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      );
    }
    renderWithCrew(
      <>
        <WorkspaceMenu />
        <CrewDialogs />
      </>
    );
    const switcher = screen.getByRole('button', { name: 'lab' });
    await user.click(switcher);
    await user.click(await screen.findByRole('menuitem', { name: 'People…' }));
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    await user.click(within(dialog).getByRole('button', { name: 'Done' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    await waitFor(() => expect(switcher).toHaveFocus());
    expect(document.activeElement).not.toBe(document.body);
  });

  // QA Q2-27: Privacy… in the privacy popover handed focus back to the channel heading.
  it('returns focus to a popover’s trigger when a control in the popover opened the dialog', async () => {
    const user = userEvent.setup();
    function PrivacyPopover() {
      const crew = useCrew();
      return (
        <Popover>
          <PopoverTrigger asChild>
            <button type="button">Private · ucsf</button>
          </PopoverTrigger>
          <PopoverContent>
            <button
              type="button"
              onClick={() => crew.openDialog({ kind: 'workspace-settings', tab: 'privacy' })}
            >
              Privacy…
            </button>
          </PopoverContent>
        </Popover>
      );
    }
    renderWithCrew(
      <>
        <PrivacyPopover />
        <CrewDialogs />
      </>
    );
    const chip = screen.getByRole('button', { name: 'Private · ucsf' });
    await user.click(chip);
    await user.click(await screen.findByRole('button', { name: 'Privacy…' }));
    await screen.findByRole('dialog', { name: 'lab settings' });
    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'lab settings' })).toBeNull());
    await waitFor(() => expect(chip).toHaveFocus());
  });

  it('returns to the first opener when one dialog hands over to another', async () => {
    // Add people (nobody else has joined) → "Invite people to lab…" → Escape: the button inside
    // Add people is gone, so focus goes back to what opened Add people.
    const user = userEvent.setup();
    renderWithCrew(
      <>
        <Opener intent={{ kind: 'add-people', target: 'team', targetId: 'team-1' }} />
        <CrewDialogs />
      </>,
      { snapshot: makeSnapshot({ principals: [alice] }) }
    );
    const open = screen.getByRole('button', { name: 'Open' });
    await user.click(open);
    await user.click(
      await screen.findByRole('button', { name: addPeopleCopy.inviteToWorkspace('lab') })
    );
    await screen.findByRole('dialog', { name: 'Invite people to lab' });
    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    await waitFor(() => expect(open).toHaveFocus());
  });

  it('pins its dialogs by their top edge, so a change of height never re-centres them', async () => {
    renderWithCrew(<CrewDialogs />, { dialog: { kind: 'workspace-settings' } });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    expect(dialog).toHaveAttribute('data-anchor', 'top');
    expect(within(dialog).getByRole('tablist')).toBeInTheDocument();
  });

  it('hosts every dialog kind but join, host and sign-in', () => {
    expect([...HOSTED_DIALOG_KINDS].sort()).toEqual(
      [
        'add-people',
        'confirm',
        'connection-settings',
        'create-channel',
        'create-team',
        'edit-profile',
        'invite-people',
        'keys',
        'let-in',
        'rename',
        'share-path',
        'transfer-ownership',
        'workspace-settings',
      ].sort()
    );
  });
});

/** A stylesheet's rules as `selector → declarations`, comments removed; enough for these files. */
function rulesOf(path: string): Map<string, string> {
  const text = readFileSync(path, 'utf8').replace(/\/\*[\s\S]*?\*\//g, ' ');
  const rules = new Map<string, string>();
  for (const match of text.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    rules.set(match[1].trim().replace(/\s+/g, ' '), match[2].replace(/\s+/g, ' ').trim());
  }
  return rules;
}

describe('Crew dialog chrome (QA Q2-25, Q2-26)', () => {
  const FOCUS_EDGE = ":read-write:focus-visible:not([aria-invalid='true'])";

  /**
   * jsdom loads no stylesheet and evaluates no `:focus-visible`, so a component test that focuses
   * a field and reads its border sees the resting edge whether the rule exists or not. The rule is
   * asserted at the source, next to the Crew area's own, which it must match.
   */
  it('gives a dialog’s text fields the Crew area’s focus edge, through the dialog’s own class', () => {
    const inArea = rulesOf(join(__dirname, '../crew-app.css')).get(`.crew-app ${FOCUS_EDGE}`);
    const inDialog = rulesOf(join(__dirname, 'dialogs.css')).get(`.crew-dialog ${FOCUS_EDGE}`);
    expect(inArea).toBe('border-color: var(--border-accent);');
    expect(inDialog).toBe(inArea);
    expect(CREW_DIALOG_DEFAULTS.className).toBe('crew-dialog');
    // The class is the one the stylesheet is loaded with.
    expect(readFileSync(join(__dirname, 'CrewDialogs.tsx'), 'utf8')).toContain(
      "import './dialogs.css';"
    );
  });

  /**
   * A confirmation renders the shared `DangerousConfirmDialog`, whose content takes no class, so
   * `crew-dialog` never reaches its typed-name field and the review found it still showed no focus
   * (QA Q2-25). Its rule finds the confirmation by the mark `confirmations.tsx` renders inside it.
   */
  it('gives a confirmation’s typed-name field the same edge, through the confirmation’s mark', async () => {
    const inArea = rulesOf(join(__dirname, '../crew-app.css')).get(`.crew-app ${FOCUS_EDGE}`);
    const inConfirm = rulesOf(join(__dirname, 'dialogs.css')).get(
      `[data-slot='dialog-content']:has(.crew-confirmation) ${FOCUS_EDGE}`
    );
    expect(inConfirm).toBe(inArea);
    // Wherever a confirmation mounts, it brings the stylesheet with it.
    expect(readFileSync(join(__dirname, 'confirmations.tsx'), 'utf8')).toContain(
      "import './dialogs.css';"
    );

    renderWithCrew(<CrewDialogs />, {
      dialog: {
        kind: 'confirm',
        confirm: { action: 'make-connection-public', connectionId: 'conn-1' },
      },
    });
    const dialog = await screen.findByRole('alertdialog');
    const field = within(dialog).getByRole('textbox');
    // The relation the selector needs: the field and the mark share one dialog content node.
    const content = field.closest('[data-slot="dialog-content"]');
    expect(content).toBe(dialog);
    expect(content?.querySelector('.crew-confirmation')).not.toBeNull();
    // And the mark is not the ModalShell class, which this primitive never gets.
    expect(dialog).not.toHaveClass('crew-dialog');
  });

  it('puts the class and the header hairline on every dialog it hosts', async () => {
    renderWithCrew(<CrewDialogs />, { dialog: { kind: 'edit-profile' } });
    const dialog = await screen.findByRole('dialog', { name: 'Edit profile' });
    expect(dialog).toHaveClass('crew-dialog');
    expect(dialog.firstElementChild).toHaveClass('border-b', 'border-border-subtle');
  });

  it.each<[DialogIntent, string]>([
    [{ kind: 'connection-settings', connectionId: 'conn-1' }, 'Cancel'],
    [{ kind: 'invite-people' }, 'Cancel'],
    [{ kind: 'let-in', username: 'eve' }, 'Cancel'],
    [{ kind: 'create-channel', teamId: 'team-1' }, 'Cancel'],
    [{ kind: 'add-people', target: 'channel', targetId: 'channel-general' }, 'Cancel'],
    [{ kind: 'edit-profile' }, 'Cancel'],
    [{ kind: 'share-path' }, 'Cancel'],
    [{ kind: 'create-team' }, 'Cancel'],
    [{ kind: 'rename', target: 'team', targetId: 'team-1' }, 'Cancel'],
    [{ kind: 'transfer-ownership', channelId: 'channel-general' }, 'Cancel'],
  ])('draws %j’s Cancel as a secondary button', async (intent, name) => {
    renderWithCrew(<CrewDialogs />, {
      dialog: intent,
      snapshot: makeSnapshot({ pending_joins: [{ username: 'eve' }] }),
    });
    const cancel = await screen.findByRole('button', { name });
    expect(cancel).toHaveClass('bg-background-medium');
    expect(cancel).not.toHaveClass('border-border-emphasized');
  });

  /**
   * The review found four dialogs the list above did not reach still drawing an outlined Cancel
   * (Create team and its "Skip for now", Rename, Transfer ownership, Make private), so one team
   * menu opened two Cancel styles (QA Q2-26). Every button a dialog in this folder writes is
   * checked at the source, so a dialog added later cannot bring the outline back. The
   * confirmations are not in this claim: they render the app's shared `ConfirmationModal` and
   * `DangerousConfirmDialog`, whose Cancel is the same on every privacy surface in the app.
   */
  it('draws no outlined button in any dialog in this folder', () => {
    const offenders = readdirSync(__dirname)
      .filter((file) => file.endsWith('.tsx') && !file.includes('.test.'))
      .filter((file) => /variant=["{']+outline/.test(readFileSync(join(__dirname, file), 'utf8')));
    expect(offenders).toEqual([]);
  });
});
