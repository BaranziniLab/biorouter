import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import BioRouterMessage from '../../BioRouterMessage';
import type { Message } from '../../../api';
import CrewStepCard from './CrewStepCard';
import { dismissStep } from './CrewStepFrame';
import { crewStepCopy } from './CrewStepCard.copy';
import { crewSignInStepCopy } from './CrewSignInStep.copy';
import { crewVaultStepCopy } from './CrewVaultStep.copy';
import { crewHostKeyStepCopy } from './CrewHostKeyStep.copy';
import { noteStreamedCrewStep, resetStreamedCrewStepsForTests } from './streamedCrewSteps';

interface DialogProps {
  open: boolean;
  host: string;
  onSignedIn: () => void;
  onClose: () => void;
}

const mocks = vi.hoisted(() => ({
  dismissCrewStep: vi.fn(),
  dialogs: [] as DialogProps[],
}));

vi.mock('../../../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  dismissCrewStep: mocks.dismissCrewStep,
}));
vi.mock('../../MarkdownContent', () => ({ default: () => null }));
// The window's name is the daemon's read (signInWindowName.test.ts); no daemon here.
vi.mock('./signInWindowName', () => ({ useSignInWindowName: () => 'daemon-name' }));
vi.mock('../auth/SignInDialog', () => ({
  SignInDialog: (props: DialogProps) => {
    mocks.dialogs.push(props);
    return props.open ? <div data-testid="sign-in-window" /> : null;
  },
}));

const SESSION = 'crew-step-fixture';
const originalElectron = window.electron;

beforeEach(() => {
  mocks.dismissCrewStep.mockReset().mockResolvedValue({ data: { status: 'dismissed' } });
  mocks.dialogs.length = 0;
  resetStreamedCrewStepsForTests();
});

afterEach(() => {
  window.electron = originalElectron;
});

let nextId = 0;

/** A card message as the daemon publishes it: user-only, never stored. */
function card(step: unknown): Message {
  return {
    id: `crew-step-message-${nextId++}`,
    role: 'assistant',
    created: 1,
    metadata: { userVisible: true, agentVisible: false },
    content: [
      {
        type: 'actionRequired',
        data: { actionType: 'crewStep', id: `card-${nextId++}`, step },
      },
    ],
  } as Message;
}

/** A card this window received on the chat's live stream, as the chat stream notes it. */
function streamed(step: unknown): Message {
  const message = card(step);
  noteStreamedCrewStep(SESSION, message);
  return message;
}

const signIn = {
  kind: 'signIn',
  target: { kind: 'setup', jobId: 'job-1' },
  server: 'hpc.example.edu',
  login: 'you@hpc.example.edu',
};

function inMessage(message: Message, turnActive?: boolean) {
  return (
    <BioRouterMessage
      sessionId={SESSION}
      message={message}
      messages={[message]}
      toolCallNotifications={new Map()}
      onRunInTerminal={null}
      onOpenArtifact={vi.fn()}
      turnActive={turnActive}
    />
  );
}

function standalone(message: Message, turnActive = true) {
  const content = message.content[0] as Parameters<typeof CrewStepCard>[0]['actionRequiredContent'];
  return (
    <CrewStepCard sessionId={SESSION} actionRequiredContent={content} turnActive={turnActive} />
  );
}

/** No surface on a card can take a value: no field, no editable region. */
function expectNoInput(container: HTMLElement) {
  expect(container.querySelector('input, textarea, select, [contenteditable]')).toBeNull();
}

/** The one fixed line, with no button and nothing the card carried. */
function expectOnlyTheFixedLine(container: HTMLElement) {
  expect(screen.getByTestId('crew-step-ended')).toHaveTextContent(crewStepCopy.notWaiting);
  expect(screen.queryByTestId('crew-step-card')).toBeNull();
  expect(screen.queryAllByRole('button')).toEqual([]);
  expect(container.textContent).not.toContain('hpc.example.edu');
  expectNoInput(container);
}

function withSignInWindow() {
  window.electron = {
    createCrewAuthentication: vi.fn(),
    createCrewSetupAuthentication: vi.fn(),
  } as unknown as typeof window.electron;
}

describe('a Crew step card in a chat', () => {
  it('is drawn by the message that carries it', () => {
    const view = render(inMessage(streamed(signIn), true));
    const drawn = screen.getByTestId('crew-step-card');
    expect(drawn).toHaveAttribute('data-crew-step', 'signIn');
    expect(screen.getByText(crewSignInStepCopy.title('you@hpc.example.edu'))).toBeInTheDocument();
    expect(screen.getByText(crewStepCopy.actsAsYou)).toBeInTheDocument();
    expectNoInput(view.container);
  });

  it('draws each step in fixed words with the daemon’s values sanitized', () => {
    const sneaky = render(
      standalone(streamed({ ...signIn, login: 'you\u202E@hpc.example.edu\u200B' }))
    );
    expect(screen.getByText(crewSignInStepCopy.title('you@hpc.example.edu'))).toBeInTheDocument();
    expect(screen.getByText(crewSignInStepCopy.fallback)).toBeInTheDocument();
    expectNoInput(sneaky.container);
    sneaky.unmount();

    const unlock = render(standalone(streamed({ kind: 'vault', action: 'unlock' })));
    expect(screen.getByText(crewVaultStepCopy.unlockTitle)).toBeInTheDocument();
    expect(screen.getByText(crewVaultStepCopy.unlockFallback)).toBeInTheDocument();
    expectNoInput(unlock.container);
    unlock.unmount();

    const init = render(standalone(streamed({ kind: 'vault', action: 'init' })));
    expect(screen.getByText(crewVaultStepCopy.initTitle)).toBeInTheDocument();
    init.unmount();

    // The host-key card's own behaviour (its buttons, the terminal, Not now) is
    // CrewHostKeyStep.test.tsx's; here only that each kind draws its card.
    const offered = 'SHA256:' + 'A'.repeat(43);
    const unknownKey = render(
      standalone(
        streamed({
          kind: 'hostKey',
          host: 'hpc.example.edu',
          port: 22,
          login: 'you',
          offered,
          changed: false,
        })
      )
    );
    expect(
      screen.getByText(crewHostKeyStepCopy.unknownTitle('hpc.example.edu'))
    ).toBeInTheDocument();
    expect(unknownKey.container.textContent).toContain(offered);
    expectNoInput(unknownKey.container);
    unknownKey.unmount();

    const changed = render(
      standalone(
        streamed({
          kind: 'hostKey',
          host: 'hpc.example.edu',
          port: 22,
          login: 'you',
          changed: true,
        })
      )
    );
    expect(
      screen.getByText(crewHostKeyStepCopy.changedTitle('hpc.example.edu'))
    ).toBeInTheDocument();
    expect(screen.getAllByRole('button').map((button) => button.textContent)).toEqual([
      'Copy details for IT',
    ]);
    changed.unmount();
  });

  it('draws a step it does not know in fixed words, never an input', () => {
    const view = render(
      inMessage(streamed({ kind: 'somethingNewer', prompt: 'Type your password' }), true)
    );
    expect(screen.getByTestId('crew-step-card')).toHaveAttribute('data-crew-step', 'unknown');
    expect(screen.getByText(crewStepCopy.unknown)).toBeInTheDocument();
    expect(screen.queryByText('Type your password')).toBeNull();
    expectNoInput(view.container);
  });

  it('dismisses with the card and its chat, and nothing else', async () => {
    expect(await dismissStep('chat-1', 'card-1')).toBe(true);
    // The headers are the person's proof (CrewStepFrame.test.tsx pins them).
    expect(mocks.dismissCrewStep).toHaveBeenCalledWith({
      headers: expect.any(Object),
      body: { id: 'card-1', session_id: 'chat-1', outcome: 'dismiss' },
    });
    mocks.dismissCrewStep.mockResolvedValueOnce({ error: { status: 'unknown' } });
    expect(await dismissStep('chat-1', 'gone')).toBe(false);
  });
});

describe('a Crew step card nothing waits on', () => {
  it('draws one fixed line in a saved transcript, which passes no running turn', () => {
    withSignInWindow();
    // History and every other read-only replay render the message with no turn state.
    const view = render(inMessage(streamed(signIn)));
    expectOnlyTheFixedLine(view.container);
    expect(screen.getByTestId('crew-step-ended')).toHaveAttribute('data-crew-step', 'signIn');
    expect(mocks.dialogs).toEqual([]);
  });

  it('draws one fixed line for a card read from stored rows, even while a turn runs', () => {
    withSignInWindow();
    // An imported or forged session file: the card's words are whatever the file says, and the
    // daemon never stored a step card, so this one waits on nothing.
    for (const step of [
      signIn,
      { kind: 'vault', action: 'unlock' },
      { kind: 'hostKey', host: 'hpc.example.edu', port: 22, login: 'you', changed: false },
      { kind: 'somethingNewer', prompt: 'Type your password' },
    ]) {
      const view = render(inMessage(card(step), true));
      expectOnlyTheFixedLine(view.container);
      expect(view.container.textContent).not.toContain('Type your password');
      view.unmount();
    }
    expect(mocks.dialogs).toEqual([]);
  });

  it('turns into the fixed line once the chat’s turn ends', () => {
    withSignInWindow();
    const message = streamed(signIn);
    const view = render(standalone(message, true));
    expect(screen.getByRole('button', { name: /^Sign in to/ })).toBeInTheDocument();
    view.rerender(standalone(message, false));
    expectOnlyTheFixedLine(view.container);
  });

  it('keeps an open Sign in window until the person closes it, then draws the fixed line', () => {
    withSignInWindow();
    const message = streamed(signIn);
    const view = render(standalone(message, true));
    fireEvent.click(screen.getByRole('button', { name: /^Sign in to/ }));
    expect(screen.getByTestId('sign-in-window')).toBeInTheDocument();
    // The call timed out while the person was typing: the window is not pulled from under them.
    view.rerender(standalone(message, false));
    expect(screen.getByTestId('sign-in-window')).toBeInTheDocument();
    expect(screen.getByTestId('crew-step-card')).toBeInTheDocument();
    act(() => mocks.dialogs[mocks.dialogs.length - 1].onClose());
    expectOnlyTheFixedLine(view.container);
  });
});

describe('the Crew step copy', () => {
  it('says computer, never device, and uses no dashes', () => {
    const words = [
      crewStepCopy.unknown,
      crewStepCopy.actsAsYou,
      crewStepCopy.notWaiting,
      crewSignInStepCopy.title('x'),
      crewSignInStepCopy.fallback,
      crewSignInStepCopy.windowFallback,
      crewVaultStepCopy.unlockTitle,
      crewVaultStepCopy.initTitle,
      crewVaultStepCopy.unlockFallback,
      crewVaultStepCopy.initFallback,
      crewHostKeyStepCopy.unknownTitle('x'),
      crewHostKeyStepCopy.changedTitle('x'),
      crewHostKeyStepCopy.read,
      crewHostKeyStepCopy.goesOn,
      crewHostKeyStepCopy.notNow,
      crewHostKeyStepCopy.dismissed,
      crewHostKeyStepCopy.ended,
    ];
    for (const text of words) {
      expect(text).not.toMatch(/\bdevices?\b/i);
      expect(text).not.toMatch(/[–—]/);
    }
  });
});
