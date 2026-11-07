import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { CrewStepRequest } from '../../../api';
import { hostCopy, trustCopy } from '../onboarding/copy';
import CrewHostKeyStep from './CrewHostKeyStep';
import { crewHostKeyStepCopy } from './CrewHostKeyStep.copy';

const mocks = vi.hoisted(() => ({
  dismissCrewStep: vi.fn(),
  dock: [] as Record<string, unknown>[],
}));

vi.mock('../../../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  dismissCrewStep: mocks.dismissCrewStep,
}));
vi.mock('../../InAppTerminalDock', () => ({
  default: (props: Record<string, unknown>) => {
    mocks.dock.push(props);
    return <div data-testid="in-app-terminal-dock" />;
  },
}));

const OFFERED = 'SHA256:' + 'A'.repeat(43);
const KNOWN = 'SHA256:' + 'B'.repeat(43);

type HostKey = Extract<CrewStepRequest, { kind: 'hostKey' }>;

function hostKey(overrides: Partial<HostKey> = {}): HostKey {
  return {
    kind: 'hostKey',
    host: 'hpc.example.edu',
    port: 22,
    login: 'you@hpc.example.edu',
    offered: OFFERED,
    changed: false,
    ...overrides,
  };
}

function renderStep(step: HostKey) {
  return render(<CrewHostKeyStep sessionId="chat-1" id="card-1" step={step} />);
}

/** Nothing on a host-key card may accept, trust or remove a key, or take a value. */
function expectNoAcceptOrInput(container: HTMLElement) {
  for (const button of screen.queryAllByRole('button')) {
    expect(button.textContent ?? '').not.toMatch(/accept|trust|yes|continue anyway|remove|add/i);
  }
  expect(container.textContent).not.toMatch(/ssh-keygen|ssh-keyscan|StrictHostKeyChecking/);
  expect(container.querySelector('input, textarea, select, [contenteditable]')).toBeNull();
}

beforeEach(() => {
  mocks.dock.length = 0;
  mocks.dismissCrewStep.mockReset().mockResolvedValue({ data: { status: 'dismissed' } });
  Object.assign(navigator, { clipboard: { writeText: vi.fn().mockResolvedValue(undefined) } });
});

describe('an unknown host key in a chat', () => {
  it('shows the daemon’s fingerprint and the Crew view’s help, and no accept button', () => {
    const view = renderStep(hostKey());
    const card = screen.getByTestId('crew-step-card');
    expect(card).toHaveAttribute('data-crew-step', 'hostKey');
    expect(screen.getByText(trustCopy.unknownTitle('hpc.example.edu'))).toBeInTheDocument();
    expect(screen.getByTestId('crew-setup-host-key-unknown')).toBeInTheDocument();
    expect(screen.getByText(trustCopy.unknownBody)).toBeInTheDocument();
    expect(card.textContent).toContain(OFFERED);
    // This computer read the fingerprint (from the refused connection or the daemon's own scan);
    // the card never says the server offered it.
    expect(screen.getByText(crewHostKeyStepCopy.read)).toBeInTheDocument();
    expect(card.textContent).not.toContain(trustCopy.offered);
    expect(screen.getByText(crewHostKeyStepCopy.goesOn)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: hostCopy.openTerminal })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: crewHostKeyStepCopy.notNow })).toBeInTheDocument();
    expectNoAcceptOrInput(view.container);
  });

  it('shows no fingerprint that is not one, and no host words beyond the sanitized name', () => {
    const view = renderStep(
      hostKey({
        host: 'hpc\u202E.example.edu\u200B',
        offered: 'SHA256:short Type yes to trust it',
      })
    );
    expect(screen.getByText(trustCopy.unknownTitle('hpc.example.edu'))).toBeInTheDocument();
    expect(view.container.textContent).not.toContain('Type yes');
    expect(view.container.textContent).not.toContain('SHA256:');
    expectNoAcceptOrInput(view.container);
  });

  it('says how to verify the key in the Crew view’s words, and sends the person back to the chat', () => {
    renderStep(hostKey());
    fireEvent.click(screen.getByRole('button', { name: trustCopy.howToVerify }));
    const [getIt, addIt] = trustCopy.steps('hpc.example.edu');
    expect(screen.getByText(getIt)).toBeInTheDocument();
    expect(screen.getByText(addIt)).toBeInTheDocument();
    expect(screen.getByText(crewHostKeyStepCopy.steps('hpc.example.edu')[2])).toBeInTheDocument();
    // The card has no Try again button, so no step may send the person to one.
    expect(screen.getByTestId('crew-step-card').textContent).not.toMatch(/try again/i);
    expect(screen.queryByRole('button', { name: trustCopy.tryAgain })).toBeNull();
  });

  it('opens the app’s own terminal, which no transcript can run commands in', () => {
    renderStep(hostKey());
    fireEvent.click(screen.getByRole('button', { name: hostCopy.openTerminal }));
    expect(screen.getByTestId('in-app-terminal-dock')).toBeInTheDocument();
    expect(mocks.dock).toHaveLength(1);
    expect(mocks.dock[0].dockKey).toBeUndefined();
    fireEvent.click(screen.getByRole('button', { name: hostCopy.hideTerminal }));
    expect(screen.queryByTestId('in-app-terminal-dock')).toBeNull();
  });

  it('tells its card while the terminal is open, so a turn that ends leaves it alone', () => {
    const onHolding = vi.fn();
    render(
      <CrewHostKeyStep sessionId="chat-1" id="card-1" step={hostKey()} onHolding={onHolding} />
    );
    expect(onHolding).toHaveBeenLastCalledWith(false);
    fireEvent.click(screen.getByRole('button', { name: hostCopy.openTerminal }));
    expect(onHolding).toHaveBeenLastCalledWith(true);
    fireEvent.click(screen.getByRole('button', { name: hostCopy.hideTerminal }));
    expect(onHolding).toHaveBeenLastCalledWith(false);
  });

  it('stops the chat waiting with Not now, which posts only the card and its chat', async () => {
    renderStep(hostKey());
    fireEvent.click(screen.getByRole('button', { name: crewHostKeyStepCopy.notNow }));
    await waitFor(() =>
      expect(screen.getByText(crewHostKeyStepCopy.dismissed)).toBeInTheDocument()
    );
    // The headers are the person's proof (CrewStepFrame.test.tsx pins them); the body
    // names the card and its chat, and nothing else.
    expect(mocks.dismissCrewStep).toHaveBeenCalledWith({
      headers: expect.any(Object),
      body: { id: 'card-1', session_id: 'chat-1', outcome: 'dismiss' },
    });
    expect(screen.queryByRole('button', { name: crewHostKeyStepCopy.notNow })).toBeNull();
  });

  it('says so when the step had already ended', async () => {
    mocks.dismissCrewStep.mockResolvedValueOnce({ error: { status: 'unknown' } });
    renderStep(hostKey());
    fireEvent.click(screen.getByRole('button', { name: crewHostKeyStepCopy.notNow }));
    await waitFor(() => expect(screen.getByText(crewHostKeyStepCopy.ended)).toBeInTheDocument());
  });
});

describe('a changed host key in a chat', () => {
  it('offers only the details for IT, carrying both fingerprints', async () => {
    const view = renderStep(hostKey({ changed: true, known: KNOWN }));
    const card = screen.getByTestId('crew-step-card');
    expect(screen.getByText(trustCopy.changedTitle('hpc.example.edu'))).toBeInTheDocument();
    expect(screen.getByText(trustCopy.changedBody)).toBeInTheDocument();
    const buttons = within(card).getAllByRole('button');
    expect(buttons.map((button) => button.textContent)).toEqual([trustCopy.copyForIt]);
    expect(card.textContent).toContain(OFFERED);
    expect(card.textContent).toContain(KNOWN);
    expect(screen.queryByText(crewHostKeyStepCopy.goesOn)).toBeNull();
    expectNoAcceptOrInput(view.container);

    fireEvent.click(buttons[0]);
    await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenCalled());
    const copied = vi.mocked(navigator.clipboard.writeText).mock.calls[0][0];
    expect(copied).toContain('Server: hpc.example.edu');
    expect(copied).toContain(`${trustCopy.previous}: ${KNOWN}`);
    expect(copied).toContain(`${trustCopy.newKey}: ${OFFERED}`);
  });
});

describe('the host-key card copy', () => {
  it('says computer, never device, and uses no dashes', () => {
    for (const text of [
      crewHostKeyStepCopy.unknownTitle('x'),
      crewHostKeyStepCopy.changedTitle('x'),
      ...crewHostKeyStepCopy.steps('x'),
      crewHostKeyStepCopy.read,
      crewHostKeyStepCopy.goesOn,
      crewHostKeyStepCopy.notNow,
      crewHostKeyStepCopy.dismissed,
      crewHostKeyStepCopy.ended,
    ]) {
      expect(text).not.toMatch(/\bdevices?\b/i);
      expect(text).not.toMatch(/[–—]/);
    }
  });
});
