import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import * as React from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../../ui/dropdown-menu';
import { keysCopy } from './copy';
import {
  alice,
  connection,
  installResizeObserverStub,
  makeSnapshot,
  renderWithCrew,
} from './dialogsTestHarness';
import { groupedFingerprint, workspaceKeyFingerprint } from './fingerprint';
import { CREDENTIAL_BACKENDS, KeysDialog, keysErrorText, STATUS_PATIENCE_MS } from './KeysDialog';

/**
 * Keys and security reads where this profile keeps its keys from the main process
 * (`crew:credentials`). QA Q2-02: the daemon started reporting `backend: "file"`, the main process
 * refused it, and the dialog showed Electron's raw "Error invoking remote method…" in red forever.
 */

installResizeObserverStub();

const SRC = join(__dirname, '../../..');
const IPC_REFUSAL =
  "Error invoking remote method 'crew:credentials': Error: Invalid Crew credential status.";

const credentials = vi.fn();

beforeEach(() => {
  credentials.mockReset();
  (window as unknown as { electron: Record<string, unknown> }).electron = {
    ...(window as unknown as { electron?: Record<string, unknown> }).electron,
    crewCredentials: credentials,
  };
});

afterEach(() => {
  vi.useRealTimers();
});

const noRemoteMethod = () => expect(document.body.textContent ?? '').not.toMatch(/remote method/i);

describe('KeysDialog, reading the status', () => {
  it('says a file store is a file on this computer', async () => {
    credentials.mockResolvedValue({ backend: 'file', initialized: true, locked: false });
    renderWithCrew(<KeysDialog onClose={vi.fn()} />);
    expect(await screen.findByText(keysCopy.file)).toBeInTheDocument();
    expect(screen.queryByText(keysCopy.statusFailed)).toBeNull();
    expect(screen.queryByRole('button', { name: keysCopy.retry })).toBeNull();
    noRemoteMethod();
  });

  it('says it couldn’t check, in plain words with a Retry, when the main process refuses', async () => {
    credentials.mockRejectedValueOnce(new Error(IPC_REFUSAL));
    renderWithCrew(<KeysDialog onClose={vi.fn()} />);
    expect(await screen.findByText(keysCopy.statusFailed)).toBeInTheDocument();
    // Not an error note: reading the status is not something the person did.
    expect(screen.queryByRole('alert')).toBeNull();
    noRemoteMethod();

    credentials.mockResolvedValueOnce({ backend: 'keyring', initialized: true, locked: false });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: keysCopy.retry }));
    });
    expect(await screen.findByText(keysCopy.keychain)).toBeInTheDocument();
    expect(credentials).toHaveBeenCalledTimes(2);
    expect(credentials).toHaveBeenLastCalledWith('status');
    expect(screen.queryByRole('button', { name: keysCopy.retry })).toBeNull();
  });

  it('treats an answer it cannot read as a failed check', async () => {
    credentials.mockResolvedValue({ backend: 'tpm', initialized: true, locked: false });
    renderWithCrew(<KeysDialog onClose={vi.fn()} />);
    expect(await screen.findByText(keysCopy.statusFailed)).toBeInTheDocument();
    expect(screen.queryByText(keysCopy.keychain)).toBeNull();
  });

  it('stops saying "Checking…" after a few seconds with no answer, and takes a late one', async () => {
    vi.useFakeTimers();
    let answer: (value: unknown) => void = () => {};
    credentials.mockReturnValue(new Promise((resolve) => (answer = resolve)));
    renderWithCrew(<KeysDialog onClose={vi.fn()} />);
    expect(screen.getByText(keysCopy.checking)).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(STATUS_PATIENCE_MS - 1);
    });
    expect(screen.getByText(keysCopy.checking)).toBeInTheDocument();
    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(screen.getByText(keysCopy.statusFailed)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: keysCopy.retry })).toBeInTheDocument();
    expect(screen.queryByText(keysCopy.checking)).toBeNull();

    await act(async () => {
      answer({ backend: 'keyring', initialized: true, locked: false });
    });
    expect(screen.getByText(keysCopy.keychain)).toBeInTheDocument();
    expect(screen.queryByText(keysCopy.statusFailed)).toBeNull();
  });
});

describe('KeysDialog, changing the store', () => {
  it('shows an action’s failure in the main process’s own words, without Electron’s wrapper', async () => {
    credentials.mockResolvedValueOnce({
      backend: 'encrypted_vault',
      initialized: true,
      locked: true,
    });
    renderWithCrew(<KeysDialog onClose={vi.fn()} />);
    expect(await screen.findByText(keysCopy.vault)).toBeInTheDocument();
    credentials.mockRejectedValueOnce(
      new Error(
        "Error invoking remote method 'crew:credentials': Error: Vault unlock was refused. Check the passphrase and selected profile."
      )
    );
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: keysCopy.unlock }));
    });
    expect(await screen.findByRole('alert')).toHaveTextContent(
      /^Vault unlock was refused\. Check the passphrase and selected profile\.$/
    );
    noRemoteMethod();
  });

  it('words every failure for a person', () => {
    expect(keysErrorText(IPC_REFUSAL)).toBe(keysCopy.statusFailed);
    expect(keysErrorText("Error invoking remote method 'crew:credentials': Error: ")).toBe(
      keysCopy.failed
    );
    expect(keysErrorText('The Crew window closed.')).toBe('The Crew window closed.');
  });
});

describe('the backends the main process accepts', () => {
  /**
   * The list lives twice — the main process validates the daemon's answer before the renderer
   * sees it — and the two drifted once. Read the main process's list from its source.
   */
  it('are exactly the ones the dialog can show, and the preload bridge types', () => {
    const main = readFileSync(join(SRC, 'main.ts'), 'utf8');
    const handler = main.slice(main.indexOf("ipcMain.handle('crew:credentials'"));
    expect(handler.length).toBeLessThan(main.length);
    const list = /\[((?:\s*'[a-z_]+',?)+)\s*\]\.includes\(status\.backend\)/.exec(handler);
    expect(list, 'the backend allowlist in main.ts').not.toBeNull();
    const accepted = [...list![1].matchAll(/'([a-z_]+)'/g)].map((match) => match[1]);
    expect([...accepted].sort()).toEqual([...CREDENTIAL_BACKENDS].sort());

    const preload = readFileSync(join(SRC, 'preload.ts'), 'utf8');
    const typed = /crewCredentials:[\s\S]*?backend:\s*([^;]+);/.exec(preload)?.[1] ?? '';
    for (const backend of CREDENTIAL_BACKENDS) expect(typed).toContain(`'${backend}'`);
  });
});

describe('KeysDialog, the layout (QA Q3-41)', () => {
  const ADDED = 1_758_700_000;
  const addedText = () =>
    `${keysCopy.deviceAdded(
      new Date(ADDED * 1000).toLocaleDateString(undefined, {
        year: 'numeric',
        month: 'long',
        day: 'numeric',
      })
    )} · ${keysCopy.addedVia.invitation_code}`;

  it('opens on Done: the dialog is read, and the first key must not copy anything', async () => {
    credentials.mockResolvedValue({ backend: 'keyring', initialized: true, locked: false });
    // A saved device ID that IS the digest, as a real profile has: the fingerprint and its Copy are
    // there from the first frame, and the dialog's own first-control focus landed on Copy.
    renderWithCrew(<KeysDialog onClose={vi.fn()} />, {
      connections: [{ ...connection, device_id: 'ef'.repeat(32) }],
    });
    const dialog = await screen.findByRole('dialog', { name: keysCopy.title });
    expect(
      within(dialog).getByRole('button', { name: `Copy ${keysCopy.deviceKeyLabel}` })
    ).toBeInTheDocument();
    const done = within(dialog).getByRole('button', { name: keysCopy.done });
    await waitFor(() => expect(done).toHaveFocus());
  });

  it('shows this computer once when it is the account’s only device, with when it was added', async () => {
    credentials.mockResolvedValue({ backend: 'keyring', initialized: true, locked: false });
    const mine = groupedFingerprint((await workspaceKeyFingerprint(connection.public_key))!);
    renderWithCrew(<KeysDialog onClose={vi.fn()} />, {
      snapshot: makeSnapshot({
        actor: {
          ...alice,
          devices: [{ fingerprint: mine, added_at: ADDED, added_via: 'invitation_code' }],
        },
      }),
    });
    const dialog = await screen.findByRole('dialog', { name: keysCopy.title });
    await waitFor(() =>
      expect(within(dialog).getByRole('button', { name: `Copy ${keysCopy.deviceKeyLabel}` }))
    );
    await waitFor(() =>
      expect(within(dialog).queryByRole('region', { name: keysCopy.devices })).toBeNull()
    );
    // The fingerprint appears once — in the box to copy — and the line under it says when.
    expect(within(dialog).getAllByText(mine)).toHaveLength(1);
    expect(within(dialog).getByText(addedText())).toHaveAttribute('data-crew-device-meta');
  });

  it('puts each device’s when-and-how on its own line under its fingerprint', async () => {
    credentials.mockResolvedValue({ backend: 'keyring', initialized: true, locked: false });
    const mine = groupedFingerprint((await workspaceKeyFingerprint(connection.public_key))!);
    renderWithCrew(<KeysDialog onClose={vi.fn()} />, {
      snapshot: makeSnapshot({
        actor: {
          ...alice,
          devices: [
            { fingerprint: '3F2A 9C1E 77B0 D4E1', added_at: ADDED, added_via: 'invitation_code' },
            { fingerprint: mine, added_at: ADDED, added_via: 'invitation_code' },
          ],
        },
      }),
    });
    const devices = await screen.findByRole('region', { name: keysCopy.devices });
    await waitFor(() => expect(within(devices).getAllByText(keysCopy.thisDevice)).toHaveLength(1));
    for (const row of within(devices).getAllByRole('listitem')) {
      // A column: the fingerprint line, then the meta line, both from the start edge.
      expect(row).toHaveClass('flex-col', 'items-start');
      const meta = within(row).getByText(addedText());
      expect(meta).toHaveAttribute('data-crew-device-meta');
      expect(meta).not.toHaveClass('text-right');
    }
  });
});

/** A menu whose one item opens the dialog, as the You row's menu opens Keys and security. */
function OpenFromMenu({ children }: { children: (close: () => void) => React.ReactNode }) {
  const [open, setOpen] = React.useState(false);
  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger>You</DropdownMenuTrigger>
        <DropdownMenuContent>
          <DropdownMenuItem onSelect={() => setOpen(true)}>{keysCopy.title}…</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      {open ? children(() => setOpen(false)) : null}
    </>
  );
}

describe('KeysDialog, opened from a menu with the pointer (QA Q4-33)', () => {
  it('keeps focus on Done while the menu that opened it closes', async () => {
    credentials.mockResolvedValue({ backend: 'keyring', initialized: true, locked: false });
    const user = userEvent.setup();
    renderWithCrew(<OpenFromMenu>{(close) => <KeysDialog onClose={close} />}</OpenFromMenu>);
    await user.click(screen.getByRole('button', { name: 'You' }));
    await user.click(await screen.findByRole('menuitem', { name: `${keysCopy.title}…` }));
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull());
    const done = await screen.findByRole('button', { name: keysCopy.done });
    await waitFor(() => expect(done).toHaveFocus());
    // The closing menu drops focus to <body> a moment later in the app (218–271 ms, Carol R4-3):
    // the dialog takes it back rather than leaving the person nowhere.
    act(() => done.blur());
    expect(document.activeElement).toBe(document.body);
    await waitFor(() => expect(done).toHaveFocus());
  });
});
