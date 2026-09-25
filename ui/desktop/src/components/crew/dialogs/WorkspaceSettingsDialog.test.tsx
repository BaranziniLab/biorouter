import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { sidebarCopy } from '../sidebar/copy';
import { CrewControllerProvider } from '../state/CrewControllerContext';
import { connectionUpdateBody } from '../state/useCrewConnections';
import { confirmCopy, workspaceSettingsCopy as copy } from './copy';
import {
  alice,
  bob,
  connection,
  installResizeObserverStub,
  makeSnapshot,
  renderWithCrew,
  requestsFor,
} from './dialogsTestHarness';
import { WorkspaceSettingsDialog } from './WorkspaceSettingsDialog';

installResizeObserverStub();

// Privacy reads the configured providers for the names they publish for an institution ID, as the
// sidebar chip does (Q2-38, Q3-40). Stable callbacks, as the real context's are.
const config = vi.hoisted(() => {
  const state = { providers: [] as unknown[] };
  return { state, getProviders: async () => state.providers, read: async () => null };
});
vi.mock('../../ConfigContext', async () => {
  const actual = await vi.importActual<typeof import('../../ConfigContext')>('../../ConfigContext');
  return {
    ...actual,
    useConfig: () => ({ getProviders: config.getProviders, read: config.read }),
  };
});

/** A configured provider whose affiliation publishes "UCSF" as the name of `ucsf`. */
const ucsfProvider = {
  name: 'versa_azure',
  is_configured: true,
  affiliation: { kind: 'institutions', institutions: [{ id: 'ucsf', display_name: 'UCSF' }] },
};

afterEach(() => {
  vi.clearAllMocks();
  config.state.providers = [];
});

/**
 * Open a member menu's "Copy for support" submenu the keyboard's way (→ opens it and moves into it)
 * and return it, its one item focused. jsdom has no layout, so Radix's pointer grace area cannot tell
 * a pointer on its way into the submenu from one leaving it; Enter chooses.
 */
async function openSupport(user: ReturnType<typeof userEvent.setup>, menu: HTMLElement) {
  const trigger = within(menu).getByRole('menuitem', { name: copy.copyForSupport });
  expect(trigger).toHaveAttribute('aria-haspopup', 'menu');
  act(() => trigger.focus());
  await user.keyboard('{ArrowRight}');
  await waitFor(() => expect(screen.getAllByRole('menu')).toHaveLength(2));
  const support = screen.getAllByRole('menu')[1];
  await waitFor(() => expect(within(support).getAllByRole('menuitem')[0]).toHaveFocus());
  return support;
}

const DIALOGS_CSS = readFileSync(join(__dirname, 'dialogs.css'), 'utf8').replace(
  /\/\*[\s\S]*?\*\//g,
  ' '
);

/** The declarations of the one rule in `dialogs.css` whose selector is exactly `selector`. */
function cssRule(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  return new RegExp(`(?:^|})\\s*${escaped}\\s*\\{([^}]*)\\}`).exec(DIALOGS_CSS)?.[1] ?? '';
}

function renderSettings(
  props: Partial<Parameters<typeof WorkspaceSettingsDialog>[0]> = {},
  options: Parameters<typeof renderWithCrew>[1] = {}
) {
  const onClose = vi.fn();
  const view = renderWithCrew(<WorkspaceSettingsDialog onClose={onClose} {...props} />, options);
  return { ...view, onClose };
}

describe('WorkspaceSettingsDialog', () => {
  it('opens on the requested tab, titled by the workspace, with Done', async () => {
    renderSettings({ tab: 'privacy' });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    expect(within(dialog).getByRole('tab', { name: 'Privacy' })).toHaveAttribute(
      'aria-selected',
      'true'
    );
    await waitFor(() => expect(within(dialog).getByRole('tab', { name: 'Privacy' })).toHaveFocus());
    expect(within(dialog).getByRole('button', { name: 'Done' })).toBeInTheDocument();
    // Agent access is a slot: without content from the access area it is not offered.
    expect(within(dialog).queryByRole('tab', { name: 'Agent access' })).toBeNull();
  });

  it('renders the Agent access slot it is given', async () => {
    renderSettings({ tab: 'agent-access', agentAccess: <p>agent access fixture</p> });
    expect(await screen.findByText('agent access fixture')).toBeInTheDocument();
  });

  it('names the host and the server on General', async () => {
    renderSettings();
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    expect(dialog).toHaveTextContent('Hosted by');
    expect(dialog).toHaveTextContent('Alice Chen (@alice)');
    expect(dialog).toHaveTextContent('hpc.example.edu');
    // Rename appears only where the broker speaks the S2 naming rules.
    expect(within(dialog).queryByRole('button', { name: 'Rename…' })).toBeNull();
  });

  it('names the server as the person does, with its address to copy (QA Q3-39)', async () => {
    const labelled = {
      ...connection,
      ssh_target: 'crew_alice@52.33.141.141',
      server_label: 'lab-server',
    };
    renderSettings({}, { connections: [labelled] });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    const general = within(dialog).getByRole('tabpanel', { name: 'General' });
    expect(general).toHaveTextContent('lab-server·52.33.141.141');
    const copyAddress = within(general).getByRole('button', {
      name: `Copy ${copy.serverAddress}`,
    });
    expect(copyAddress).toBeInTheDocument();
  });

  it('shows only the address, to copy, when no alias names the server', async () => {
    renderSettings();
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    const general = within(dialog).getByRole('tabpanel', { name: 'General' });
    expect(general).toHaveTextContent('hpc.example.edu');
    expect(general.textContent).not.toContain('·');
    expect(
      within(general).getByRole('button', { name: `Copy ${copy.serverAddress}` })
    ).toBeInTheDocument();
  });

  it('offers Rename to the host when the broker projects name handles', async () => {
    const snapshot = makeSnapshot();
    snapshot.teams[0].handle = 'analysis-lab';
    const { crew } = renderSettings({}, { snapshot });
    fireEvent.click(await screen.findByRole('button', { name: 'Rename…' }));
    expect(crew.current().ui.dialog).toEqual({
      kind: 'rename',
      target: 'workspace',
      targetId: 'workspace-1',
    });
  });

  it('offers Rename in a new workspace with no teams when the broker says it speaks the rules', async () => {
    const snapshot = makeSnapshot({ teams: [], channels: [] });
    const onClose = vi.fn();
    const { crew } = renderWithCrew(
      (fake) => (
        <CrewControllerProvider controller={{ ...fake, capabilities: ['unique_names_v1'] }}>
          <WorkspaceSettingsDialog onClose={onClose} />
        </CrewControllerProvider>
      ),
      { snapshot }
    );
    fireEvent.click(await screen.findByRole('button', { name: 'Rename…' }));
    expect(crew.current().ui.dialog).toEqual({
      kind: 'rename',
      target: 'workspace',
      targetId: 'workspace-1',
    });
  });

  it('does not offer Rename when the broker says it lacks the rules, whatever it projects', async () => {
    const snapshot = makeSnapshot();
    snapshot.teams[0].handle = 'analysis-lab';
    renderWithCrew(
      (fake) => (
        <CrewControllerProvider controller={{ ...fake, capabilities: ['join_v1'] }}>
          <WorkspaceSettingsDialog onClose={vi.fn()} />
        </CrewControllerProvider>
      ),
      { snapshot }
    );
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    expect(within(dialog).queryByRole('button', { name: 'Rename…' })).toBeNull();
  });

  it('lists members by name with a row menu to copy and, for the host, remove', async () => {
    const user = userEvent.setup();
    const writeText = vi.fn(async () => {});
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    const { crew } = renderSettings({ tab: 'people' });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    expect(dialog).toHaveTextContent('Bob Lee');
    expect(dialog).not.toHaveTextContent(bob.id);

    await user.click(within(dialog).getByRole('button', { name: 'Bob Lee (@bob) options' }));
    // A machine ID is behind "Copy for support", never among the everyday items (QA Q3-26).
    expect(screen.queryByRole('menuitem', { name: 'Copy person ID' })).toBeNull();
    const support = await openSupport(user, await screen.findByRole('menu'));
    expect(within(support).getByRole('menuitem', { name: 'Copy person ID' })).toHaveFocus();
    await user.keyboard('{Enter}');
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(bob.id));
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull(), { timeout: 2000 });

    await user.click(within(dialog).getByRole('button', { name: 'Bob Lee (@bob) options' }));
    await user.click(await screen.findByRole('menuitem', { name: 'Remove from lab…' }));
    const confirm = await screen.findByRole('alertdialog', {
      name: confirmCopy.removePerson.title('Bob Lee (@bob)', 'lab'),
    });
    expect(within(confirm).getByLabelText('Type bob to confirm')).toBeInTheDocument();
    expect(requestsFor(crew, 'enrollment.revoke')).toEqual([]);
  });

  it('does not offer the host a way to remove themselves or the host', async () => {
    const user = userEvent.setup();
    renderSettings({ tab: 'people' });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    await user.click(within(dialog).getByRole('button', { name: 'Alice Chen (@alice) options' }));
    expect(await screen.findByRole('menuitem', { name: 'Copy username' })).toBeInTheDocument();
    expect(screen.queryByRole('menuitem', { name: 'Remove from lab…' })).toBeNull();
  });

  it('shows waiting joiners to the host, with Let in and an inline cancel', async () => {
    const snapshot = makeSnapshot({
      pending_joins: [{ username: 'eve', full_name: 'Eve Park', mismatched_attempts: 1 }],
    });
    const { crew } = renderSettings({ tab: 'people' }, { snapshot });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    expect(dialog).toHaveTextContent('@eve · Eve Park (name on the server account)');
    expect(within(dialog).getByText(copy.otherDevice('eve'))).toBeInTheDocument();

    fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel @eve’s invitation' }));
    expect(within(dialog).getByText(copy.cancelInvitationConfirm('eve'))).toBeInTheDocument();
    expect(within(dialog).getByRole('button', { name: copy.keepInvitation })).toHaveFocus();
    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel invitation' }));
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'enrollment.cancel')).toEqual([{ username: 'eve' }])
    );

    fireEvent.click(within(dialog).getByRole('button', { name: 'Let @eve in' }));
    expect(crew.current().ui.dialog).toEqual({ kind: 'let-in', username: 'eve' });
  });

  it('shows an expired join as expired, to invite again, never to let in', async () => {
    const snapshot = makeSnapshot({
      pending_joins: [{ username: 'eve', full_name: 'Eve Park', approved: true, expired: true }],
    });
    const { crew } = renderSettings({ tab: 'people' }, { snapshot });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    expect(dialog).toHaveTextContent('@eve · Eve Park (name on the server account)');
    expect(dialog).toHaveTextContent(
      `${sidebarCopy.waiting.expired} ${sidebarCopy.waiting.separator} ${sidebarCopy.waiting.inviteAgain}`
    );
    expect(within(dialog).queryByRole('button', { name: 'Let @eve in' })).toBeNull();
    expect(within(dialog).queryByRole('button', { name: 'Cancel @eve’s invitation' })).toBeNull();

    fireEvent.click(within(dialog).getByRole('button', { name: 'Invite @eve again' }));
    expect(crew.current().ui.dialog).toEqual({ kind: 'invite-people' });
  });

  it('shows a member neither waiting joiners nor host controls', async () => {
    const snapshot = makeSnapshot({
      actor: bob,
      pending_joins: [{ username: 'eve' }],
    });
    renderSettings({ tab: 'privacy' }, { snapshot });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    expect(within(dialog).queryByRole('button', { name: copy.allowPublic })).toBeNull();
    expect(within(dialog).getAllByText(copy.hostOnly).length).toBeGreaterThan(0);
    fireEvent.mouseDown(within(dialog).getByRole('tab', { name: 'People' }));
    fireEvent.click(within(dialog).getByRole('tab', { name: 'People' }));
    await waitFor(() => expect(dialog).toHaveTextContent('Members'));
    expect(dialog).not.toHaveTextContent('Waiting to join');
    expect(within(dialog).queryByRole('button', { name: copy.invite })).toBeNull();
  });

  it('asks for the workspace name before allowing Public, from the Privacy tab', async () => {
    const { crew } = renderSettings({ tab: 'privacy' });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    expect(dialog).toHaveTextContent(copy.privateForEveryone);
    fireEvent.click(within(dialog).getByRole('button', { name: copy.allowPublic }));
    const confirm = await screen.findByRole('alertdialog', {
      name: confirmCopy.allowWorkspacePublic.title('lab'),
    });
    fireEvent.click(within(confirm).getByRole('button', { name: 'Cancel' }));
    await waitFor(() =>
      expect(
        screen.queryByRole('alertdialog', { name: confirmCopy.allowWorkspacePublic.title('lab') })
      ).toBeNull()
    );
    // Cancel returns to the settings, having sent nothing.
    expect(screen.getByRole('dialog', { name: 'lab settings' })).toBeInTheDocument();
    expect(requestsFor(crew, 'policy.set')).toEqual([]);
  });

  it('makes the connection public only through the typed confirmation', async () => {
    const { crew } = renderSettings({ tab: 'privacy' });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    fireEvent.click(within(dialog).getByRole('button', { name: copy.makePublic }));
    const confirm = await screen.findByRole('alertdialog', {
      name: confirmCopy.makeConnectionPublic.title('lab'),
    });
    expect(crew.updateConnection).not.toHaveBeenCalled();
    fireEvent.change(within(confirm).getByLabelText('Type lab to confirm'), {
      target: { value: 'lab' },
    });
    await act(async () => {
      fireEvent.click(within(confirm).getByRole('button', { name: 'Make public' }));
    });
    await waitFor(() =>
      expect(crew.updateConnection).toHaveBeenCalledWith(connection.id, {
        ...connectionUpdateBody(connection),
        mode: 'public',
      })
    );
  });

  it('makes a public connection private in one click, or asks for the institution first', async () => {
    const publicConnection = { ...connection, mode: 'public' as const };
    const one = renderSettings({ tab: 'privacy' }, { connections: [publicConnection] });
    fireEvent.click(await screen.findByRole('button', { name: copy.makePrivate }));
    await waitFor(() =>
      expect(one.crew.updateConnection).toHaveBeenCalledWith(connection.id, {
        ...connectionUpdateBody(publicConnection),
        mode: 'private',
      })
    );
    one.unmount();

    const bare = { ...publicConnection, institution_id: null };
    const two = renderSettings({ tab: 'privacy' }, { connections: [bare] });
    fireEvent.click(await screen.findByRole('button', { name: copy.makePrivate }));
    const ask = await screen.findByRole('dialog', { name: 'Make your lab connection private' });
    expect(two.crew.updateConnection).not.toHaveBeenCalled();
    const institution = within(ask).getByPlaceholderText('For example, ucsf or sdsc');
    fireEvent.change(institution, { target: { value: 'sdsc' } });
    await act(async () => {
      fireEvent.click(within(ask).getByRole('button', { name: 'Make private' }));
    });
    await waitFor(() =>
      expect(two.crew.updateConnection).toHaveBeenCalledWith(connection.id, {
        ...connectionUpdateBody(bare),
        mode: 'private',
        institution_id: 'sdsc',
      })
    );
  });

  it('offers to set the workspace institution from the connection, with its permanent confirmation', async () => {
    const snapshot = makeSnapshot();
    snapshot.workspace.institution_id = null;
    const { crew } = renderSettings({ tab: 'privacy' }, { snapshot });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    expect(dialog).toHaveTextContent(copy.notSet);
    fireEvent.click(within(dialog).getByRole('button', { name: 'Set institution to ucsf…' }));
    const confirm = await screen.findByRole('alertdialog', {
      name: confirmCopy.setInstitution.title('lab', 'ucsf'),
    });
    await act(async () => {
      fireEvent.click(within(confirm).getByRole('button', { name: 'Set ucsf permanently' }));
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'policy.set')).toEqual([{ mode: 'private', institution_id: 'ucsf' }])
    );
  });

  it('names its tab list and lets Shift+Tab leave it instead of looping on the tab', async () => {
    const user = userEvent.setup();
    renderSettings({ tab: 'people' });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    expect(within(dialog).getByRole('tablist', { name: copy.tabsLabel })).toBeInTheDocument();
    const people = within(dialog).getByRole('tab', { name: 'People' });
    await waitFor(() => expect(people).toHaveFocus());

    // Nothing in the dialog comes before the tab list, so backward wraps to its last control.
    await user.keyboard('{Shift>}{Tab}{/Shift}');
    expect(within(dialog).getByRole('button', { name: 'Close' })).toHaveFocus();
    // …and on backward through the dialog's controls, not back onto the same tab.
    await user.keyboard('{Shift>}{Tab}{/Shift}');
    expect(within(dialog).getByRole('button', { name: 'Done' })).toHaveFocus();
  });

  it('keeps every tab of its own mounted, so its height is the tallest one’s', async () => {
    renderSettings({ tab: 'general' });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    const panels = dialog.querySelectorAll('.crew-settings-panel');
    expect(panels).toHaveLength(3);
    const inactive = Array.from(panels).filter(
      (panel) => panel.getAttribute('data-state') === 'inactive'
    );
    expect(inactive).toHaveLength(2);
    // Out of the tab order and the accessibility tree, not merely unseen.
    for (const panel of inactive) {
      expect(panel).toHaveAttribute('aria-hidden', 'true');
      expect(panel).toHaveAttribute('inert');
    }
    expect(within(dialog).queryByRole('button', { name: copy.invite })).toBeNull();
    expect(within(dialog).getAllByRole('tabpanel')).toHaveLength(1);
  });

  it('lists the host first, then you, then everyone else alphabetically', async () => {
    const zed = { id: 'person-zed', uid: 1009, username: 'zed', nickname: 'Aaron Zed' };
    const snapshot = makeSnapshot({
      actor: bob,
      principals: [
        { id: 'person-dan', uid: 1003, username: 'dan', nickname: 'Dan Wu' },
        bob,
        zed,
        { id: 'person-carol', uid: 1002, username: 'carol', nickname: 'Carol Diaz' },
        alice,
      ],
    });
    renderSettings({ tab: 'people' }, { snapshot });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    const rows = within(dialog)
      .getAllByRole('button', { name: / options$/ })
      .map((button) => button.getAttribute('aria-label'));
    expect(rows).toEqual([
      'Alice Chen (@alice) options',
      'Bob Lee (@bob) options',
      'Aaron Zed (@zed) options',
      'Carol Diaz (@carol) options',
      'Dan Wu (@dan) options',
    ]);
  });

  it('returns focus to the row menu’s trigger when a confirmation opened from it is cancelled', async () => {
    const user = userEvent.setup();
    renderSettings({ tab: 'people' });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    const trigger = within(dialog).getByRole('button', { name: 'Bob Lee (@bob) options' });
    await user.click(trigger);
    await user.click(await screen.findByRole('menuitem', { name: 'Remove from lab…' }));
    const confirm = await screen.findByRole('alertdialog', {
      name: confirmCopy.removePerson.title('Bob Lee (@bob)', 'lab'),
    });
    await user.click(within(confirm).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(trigger).toHaveFocus());
  });

  it('tells the host to add an institution to the connection first', async () => {
    const snapshot = makeSnapshot();
    snapshot.workspace.institution_id = null;
    renderSettings(
      { tab: 'privacy' },
      { snapshot, connections: [{ ...connection, institution_id: null }] }
    );
    expect(await screen.findByText(copy.institutionNeedsConnection)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /^Set institution to/ })).toBeNull();
    expect(alice.id).toBe('person-alice');
  });
});

describe('WorkspaceSettingsDialog, one vocabulary (QA Q2-29, Q2-66, Q2-69)', () => {
  /** The access area's content as it really comes: a section headed by the tab's own name. */
  function AgentAccessFixture() {
    return (
      <section aria-labelledby="access-heading">
        <h3 id="access-heading">Agent access</h3>
        <p>agent access rows</p>
      </section>
    );
  }

  /** The caps labels a panel draws, in order. */
  const capsLabels = (panel: HTMLElement) =>
    Array.from(panel.querySelectorAll('.text-caps')).map((node) => node.textContent);

  it('opens no tab with a label repeating its name; MEMBERS is a section label (QA Q3-40)', async () => {
    const user = userEvent.setup();
    renderSettings({ tab: 'general', agentAccess: <AgentAccessFixture /> });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    const expected: Record<string, string[]> = {
      General: [],
      People: [copy.members],
      Privacy: [],
      'Agent access': [],
    };
    for (const [tab, labels] of Object.entries(expected)) {
      await user.click(within(dialog).getByRole('tab', { name: tab }));
      const panel = within(dialog).getByRole('tabpanel');
      expect(capsLabels(panel)).toEqual(labels);
      for (const name of Object.values(copy.tabs)) {
        expect(within(panel).queryByRole('heading', { name })).toBeNull();
      }
    }
    // The access area's own heading repeats the tab's name: its panel is marked for the rule that
    // hides it there, and that rule is read at the source (jsdom applies no stylesheet).
    const access = within(dialog).getByRole('tabpanel');
    expect(access).toHaveAttribute('data-crew-tab', 'agent-access');
    const css = DIALOGS_CSS;
    const hide =
      /\.crew-settings-panel\[data-crew-tab='agent-access'\]\s*>\s*\[aria-labelledby\]\s*>\s*:first-child\s*\{([^}]*)\}/.exec(
        css
      );
    expect(hide?.[1]).toMatch(/display:\s*none;/);
    // That selector reaches exactly the access area's heading, and nothing of ours.
    expect(access.querySelectorAll(':scope > [aria-labelledby] > :first-child')).toHaveLength(1);
    expect(access.querySelector(':scope > [aria-labelledby] > :first-child')).toHaveTextContent(
      'Agent access'
    );
    // It still names the section.
    expect(within(access).getByRole('region', { name: 'Agent access' })).toBeInTheDocument();
  });

  it('makes the People rows a list, one item per person', async () => {
    renderSettings(
      { tab: 'people' },
      { snapshot: makeSnapshot({ pending_joins: [{ username: 'eve' }] }) }
    );
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    const members = within(dialog).getByRole('region', { name: copy.members });
    const list = within(members).getByRole('list');
    expect(within(list).getAllByRole('listitem')).toHaveLength(makeSnapshot().principals.length);
    const waiting = within(dialog).getByRole('region', { name: copy.waiting });
    expect(within(within(waiting).getByRole('list')).getAllByRole('listitem')).toHaveLength(1);
  });

  it('names the connection-only action as the popover does, with the popover’s one line', async () => {
    renderSettings({ tab: 'privacy' });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    const button = within(dialog).getByRole('button', { name: 'Make my connection public…' });
    expect(button).toHaveAccessibleDescription(
      sidebarCopy.privacy.makePublicEffect('lab', 'private')
    );
  });

  it('says what the Host badge means, on hover and to the keyboard', async () => {
    const user = userEvent.setup();
    renderSettings({ tab: 'people' });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    const badge = within(dialog).getByText(copy.host);
    const trigger = badge.parentElement as HTMLElement;
    expect(trigger).toHaveAttribute('tabindex', '0');
    await user.hover(trigger);
    expect(await screen.findByRole('tooltip')).toHaveTextContent(
      'Workspace host: runs lab on the server'
    );
    expect(copy.hostTooltip('lab')).toBe('Workspace host: runs lab on the server');
  });
});

describe('WorkspaceSettingsDialog, the seams (QA Q3-40, Q3-26, Q3-33)', () => {
  it('shows the institution as the sidebar does, and the connection as one badge', async () => {
    config.state.providers = [ucsfProvider];
    renderSettings({ tab: 'privacy' });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    const panel = within(dialog).getByRole('tabpanel', { name: 'Privacy' });
    // Read-only, in its display form: never "ucsf" beside the sidebar's "UCSF".
    const badge = await waitFor(() => {
      const found = panel.querySelector('[data-crew-privacy-badge]');
      expect(found).toHaveTextContent('Private · UCSF');
      return found!;
    });
    // One piece: the institution inside the badge, not a second chip beside it.
    expect(badge).toHaveClass('crew-settings-privacy-badge');
    expect(within(badge as HTMLElement).getByTestId('privacy-badge')).toBeInTheDocument();
    const institutionRow = within(panel).getByText(copy.institution).parentElement!;
    expect(institutionRow).toHaveTextContent('UCSF');
    expect(institutionRow.textContent).not.toContain('ucsf');
    expect(cssRule('.crew-settings-privacy-badge')).toMatch(
      /background-color:\s*var\(--background-muted\)/
    );
  });

  it('keeps the raw ID on the control that writes it', async () => {
    config.state.providers = [ucsfProvider];
    const snapshot = makeSnapshot();
    snapshot.workspace.institution_id = null;
    renderSettings({ tab: 'privacy' }, { snapshot });
    expect(
      await screen.findByRole('button', { name: copy.setInstitution('ucsf') })
    ).toBeInTheDocument();
  });

  it('puts machine IDs, and only they, in a "Copy for support" submenu, last', async () => {
    const user = userEvent.setup();
    const writeText = vi.fn(async () => {});
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    renderSettings({ tab: 'people' });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    await user.click(within(dialog).getByRole('button', { name: 'Bob Lee (@bob) options' }));
    const menu = await screen.findByRole('menu');
    const items = Array.from(menu.querySelectorAll('[role="menuitem"], [role="separator"]')).map(
      (node) => (node.getAttribute('role') === 'separator' ? '—' : node.textContent)
    );
    expect(items).toEqual(['Copy username', '—', 'Remove from lab…', '—', copy.copyForSupport]);

    const support = await openSupport(user, menu);
    expect(
      within(support)
        .getAllByRole('menuitem')
        .map((item) => item.textContent)
    ).toEqual(['Copy person ID']);
    await user.keyboard('{Enter}');
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(bob.id));
    // "Copied" holds in the item until the menu goes, as in the message menu.
    expect(await within(support).findByRole('menuitem', { name: 'Copied' })).toHaveAttribute(
      'data-crew-copy-state',
      'copied'
    );
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull(), { timeout: 2000 });
  });

  it('opens a member menu beside its ⋯, inside the dialog’s body, never over Done', async () => {
    const user = userEvent.setup();
    renderSettings({ tab: 'people' });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    await user.click(within(dialog).getByRole('button', { name: 'Dan Wu (@dan) options' }));
    const menu = await screen.findByRole('menu');
    expect(menu).toHaveAttribute('data-side', 'left');
    // The collision boundary is the dialog's scrolling body, which the footer is outside.
    const body = dialog.querySelector('.crew-settings-body');
    expect(body).not.toBeNull();
    expect(body!.contains(within(dialog).getByRole('button', { name: 'Done' }))).toBe(false);
  });

  it('hides a member’s ⋯ at rest and shows it on hover, focus and while open', async () => {
    renderSettings({ tab: 'people' });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    const trigger = within(dialog).getByRole('button', { name: 'Bob Lee (@bob) options' });
    const hook = trigger.closest('[data-row-action]');
    expect(hook).not.toBeNull();
    expect(hook!.closest('li')).toHaveClass('crew-settings-member');
    // jsdom applies no stylesheet: the rules are read at the source.
    expect(cssRule('.crew-settings-member [data-row-action]')).toMatch(/opacity:\s*0;/);
    expect(
      cssRule(
        ".crew-settings-member:is(:hover, :focus-within) [data-row-action],\n.crew-settings-member [data-row-action]:has([data-state='open'])"
      )
    ).toMatch(/opacity:\s*1;/);
  });

  it('swaps tab panels without painting two at once, and rings a focused panel inside its padding', () => {
    // The outgoing panel hides at once; the incoming one fades.
    const inactive = cssRule(".crew-settings-panel[data-state='inactive']");
    expect(inactive).toMatch(/visibility:\s*hidden;/);
    expect(inactive).toMatch(/transition:\s*visibility 0s,\s*opacity 0s;/);
    const panel = cssRule('.crew-settings-panel');
    expect(panel).toMatch(/transition:\s*opacity var\(--dur-fast\) var\(--ease-out\);/);
    // The shared TabsContent's enter animation, whose duration also held `visibility`, is off.
    expect(panel).toMatch(/animation:\s*none;/);
    expect(panel).toMatch(/padding:\s*6px;/);
    expect(cssRule('.crew-settings-panel:focus-visible')).toMatch(
      /outline:\s*2px solid var\(--ring\);\s*outline-offset:\s*-2px;/
    );
    expect(DIALOGS_CSS).toMatch(
      /@media \(prefers-reduced-motion: reduce\)\s*\{\s*\.crew-settings-panel\s*\{\s*transition:\s*none;/
    );
  });
});
