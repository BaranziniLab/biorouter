import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { ACCOUNT_IDENTITY_CAPABILITY } from '../crewApi';
import { AttentionSections } from './AttentionSections';
import { sidebarCopy } from './copy';
import { makeController, makeSnapshot, renderWithCrew } from './sidebarTestUtils';
import { WorkspaceSwitcher } from './WorkspaceSwitcher';

/**
 * The sidebar's account-ownership copy (docs/crew/design/account-ownership.md, "Desktop"): a
 * pending join reads "Waiting for @bob to open the workspace" on a workspace that belongs to
 * accounts, with Let in… kept for an older joiner's code, and the workspace menu's Add a workspace
 * offers Open a workspace on a server.
 */

describe('Waiting to join on a workspace that belongs to accounts', () => {
  const asHost = (capabilities: string[] | null) =>
    makeController({
      capabilities,
      snapshot: makeSnapshot({ pending_joins: [{ username: 'bob', full_name: 'Bob Lee' }] }),
    });

  it('waits for the joiner to open the workspace, and keeps Let in… for an older code', () => {
    renderWithCrew(<AttentionSections />, asHost([ACCOUNT_IDENTITY_CAPABILITY]));
    const row = within(screen.getByRole('list', { name: sidebarCopy.section.waiting })).getByRole(
      'listitem'
    );
    const next = row.querySelector('[data-crew-waiting-next]') as HTMLElement;
    expect(next.textContent).toBe('Waiting for @bob to open the workspace');
    expect(within(row).getByRole('button', { name: 'Let @bob in' })).toHaveAccessibleDescription(
      sidebarCopy.waiting.nextStepAccount('bob')
    );
  });

  it('keeps the code step on an older server', () => {
    renderWithCrew(<AttentionSections />, asHost(null));
    const row = within(screen.getByRole('list', { name: sidebarCopy.section.waiting })).getByRole(
      'listitem'
    );
    expect(row.querySelector('[data-crew-waiting-next]')?.textContent).toBe(
      'Let in… when Bob sends a code'
    );
  });
});

describe('Add a workspace', () => {
  it('offers Open a workspace on a server', async () => {
    const controller = makeController();
    renderWithCrew(<WorkspaceSwitcher />, controller);
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: /^Fixture/ }));
    const menu = await screen.findByRole('menu');
    const sub = within(menu).getByRole('menuitem', { name: sidebarCopy.workspaceMenu.add });
    sub.focus();
    await user.keyboard('{ArrowRight}');
    const open = await screen.findByRole('menuitem', { name: sidebarCopy.workspaceMenu.addOpen });
    await waitFor(() =>
      expect(
        screen.getByRole('menuitem', { name: sidebarCopy.workspaceMenu.addJoin })
      ).toHaveFocus()
    );
    await user.click(open);
    expect(controller.openDialog).toHaveBeenCalledWith({ kind: 'open-workspace' });
  });
});
