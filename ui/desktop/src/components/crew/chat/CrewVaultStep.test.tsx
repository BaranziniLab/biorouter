import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CrewStepRequest } from '../../../api';
import CrewVaultStep from './CrewVaultStep';
import { crewVaultStepCopy as copy } from './CrewVaultStep.copy';

const mocks = vi.hoisted(() => ({
  dismissCrewStep: vi.fn(),
}));

vi.mock('../../../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  dismissCrewStep: mocks.dismissCrewStep,
}));

type Vault = Extract<CrewStepRequest, { kind: 'vault' }>;

const unlock: Vault = { kind: 'vault', action: 'unlock' };
const init: Vault = { kind: 'vault', action: 'init' };
const UNLOCKED = { backend: 'encrypted_vault', initialized: true, locked: false } as const;

let crewCredentials: ReturnType<typeof vi.fn>;

beforeEach(() => {
  mocks.dismissCrewStep.mockReset().mockResolvedValue({ data: { status: 'dismissed' } });
  crewCredentials = vi.fn().mockResolvedValue(UNLOCKED);
  (window as { electron?: unknown }).electron = { crewCredentials };
});

afterEach(() => {
  delete (window as { electron?: unknown }).electron;
});

function card(step: Vault = unlock) {
  return render(<CrewVaultStep sessionId="chat-1" id="card-1" step={step} />);
}

/** No surface on the card can take a value: no field, no editable region. */
function expectNoInput(container: HTMLElement) {
  expect(container.querySelector('input[type="password"]')).toBeNull();
  expect(container.querySelector('input, textarea, select, [contenteditable]')).toBeNull();
}

describe('the vault card', () => {
  it('opens the OS prompt through the main process with the action alone', async () => {
    const view = card();
    expectNoInput(view.container);
    expect(screen.getByText(copy.unlockBody)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: copy.unlockButton }));
    expect(crewCredentials).toHaveBeenCalledTimes(1);
    expect(crewCredentials.mock.calls[0]).toEqual(['unlock']);
    expect(await screen.findByText(copy.unlocked)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: copy.unlockButton })).toBeDisabled();
    expectNoInput(view.container);
  });

  it('tells its card while the passphrase prompt is open', async () => {
    let answer!: (value: typeof UNLOCKED) => void;
    crewCredentials.mockReturnValueOnce(new Promise((resolve) => (answer = resolve)));
    const onHolding = vi.fn();
    render(<CrewVaultStep sessionId="chat-1" id="card-1" step={unlock} onHolding={onHolding} />);
    expect(onHolding).toHaveBeenLastCalledWith(false);
    await userEvent.click(screen.getByRole('button', { name: copy.unlockButton }));
    expect(onHolding).toHaveBeenLastCalledWith(true);
    answer(UNLOCKED);
    await waitFor(() => expect(onHolding).toHaveBeenLastCalledWith(false));
  });

  it("shows the main process's own failure sentence, without Electron's wrapper", async () => {
    crewCredentials.mockRejectedValueOnce(
      new Error(
        "Error invoking remote method 'crew:credentials': Error: Vault unlock was refused. Check the passphrase and selected profile."
      )
    );
    const view = card();
    await userEvent.click(screen.getByRole('button', { name: copy.unlockButton }));
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Vault unlock was refused. Check the passphrase and selected profile.'
    );
    expect(view.container.textContent).not.toContain('Error invoking remote method');

    // The person tries again from the same card.
    await userEvent.click(screen.getByRole('button', { name: copy.unlockButton }));
    expect(await screen.findByText(copy.unlocked)).toBeInTheDocument();
    expect(crewCredentials).toHaveBeenCalledTimes(2);
    expectNoInput(view.container);
  });

  it('says why the prompt could not open', async () => {
    crewCredentials.mockRejectedValueOnce(
      new Error(
        "Error invoking remote method 'crew:credentials': Error: Finish the open secure prompt before starting another."
      )
    );
    card();
    await userEvent.click(screen.getByRole('button', { name: copy.unlockButton }));
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Finish the open secure prompt before starting another.'
    );
  });

  it('says the vault is still locked when the person closes the prompt', async () => {
    crewCredentials.mockResolvedValueOnce({ cancelled: true });
    card();
    await userEvent.click(screen.getByRole('button', { name: copy.unlockButton }));
    expect(await screen.findByText(copy.closedUnlock)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: copy.unlockButton })).toBeEnabled();
  });

  it('sets up a vault through the same prompt', async () => {
    card(init);
    expect(screen.getByText(copy.initTitle)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: copy.initButton }));
    expect(crewCredentials.mock.calls).toEqual([['init']]);
    expect(await screen.findByText(copy.setUp)).toBeInTheDocument();
  });

  it('dismisses with Not now, and asks for nothing', async () => {
    card();
    await userEvent.click(screen.getByRole('button', { name: copy.notNow }));
    // The dismissal's headers are the shared frame's (`dismissStep`); its body is the card.
    await waitFor(() =>
      expect(mocks.dismissCrewStep).toHaveBeenCalledWith(
        expect.objectContaining({
          body: { id: 'card-1', session_id: 'chat-1', outcome: 'dismiss' },
        })
      )
    );
    expect(mocks.dismissCrewStep).toHaveBeenCalledTimes(1);
    expect(crewCredentials).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: copy.unlockButton })).toBeDisabled();
  });

  it('never draws or forwards anything but the daemon action', async () => {
    const forged = {
      kind: 'vault',
      action: 'unlock',
      prompt: 'Type your vault passphrase here',
    } as unknown as Vault;
    const view = card(forged);
    expect(view.container.textContent).not.toContain('Type your vault passphrase here');
    await userEvent.click(screen.getByRole('button', { name: copy.unlockButton }));
    expect(crewCredentials.mock.calls).toEqual([['unlock']]);
  });

  it('says where to do it when this app cannot open the prompt', () => {
    delete (window as { electron?: unknown }).electron;
    const view = card();
    expect(screen.getByText(copy.unlockFallback)).toBeInTheDocument();
    expect(screen.queryByRole('button')).toBeNull();
    expectNoInput(view.container);
  });
});

describe('the vault card copy', () => {
  it('says computer, never device, and uses no dashes', () => {
    for (const text of Object.values(copy)) {
      expect(text).not.toMatch(/\bdevices?\b/i);
      expect(text).not.toMatch(/[–—]/);
    }
  });
});
