import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CrewStepRequest } from '../../../api';
import CrewSignInStep from './CrewSignInStep';
import { crewSignInStepCopy } from './CrewSignInStep.copy';

type SignIn = Extract<CrewStepRequest, { kind: 'signIn' }>;

interface DialogProps {
  target: unknown;
  open: boolean;
  host: string;
  onSignedIn: () => void;
  onClose: () => void;
}

const mocks = vi.hoisted(() => ({
  dismissCrewStep: vi.fn(),
  browser: false,
  dialogs: [] as DialogProps[],
  windowName: null as string | null,
  asked: [] as { target: unknown; enabled: boolean }[],
}));

vi.mock('../../../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  dismissCrewStep: mocks.dismissCrewStep,
}));
// The daemon's own read of the target (signInWindowName.test.ts covers the read itself).
vi.mock('./signInWindowName', () => ({
  useSignInWindowName: (target: unknown, enabled: boolean) => {
    mocks.asked.push({ target, enabled });
    return mocks.windowName;
  },
}));
vi.mock('../../../utils/surface', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  isBrowserSurface: () => mocks.browser,
}));
// The real window is an ssh terminal in a modal; what the card hands it is what matters here.
vi.mock('../auth/SignInDialog', () => ({
  SignInDialog: (props: DialogProps) => {
    mocks.dialogs.push(props);
    return props.open ? (
      <div data-testid="sign-in-window" data-host={props.host}>
        {JSON.stringify(props.target)}
      </div>
    ) : null;
  },
}));

const originalElectron = window.electron;

beforeEach(() => {
  mocks.dismissCrewStep.mockReset().mockResolvedValue({ data: { status: 'dismissed' } });
  mocks.browser = false;
  mocks.dialogs.length = 0;
  mocks.windowName = 'you@hpc.example.edu';
  mocks.asked.length = 0;
  window.electron = {
    createCrewAuthentication: vi.fn(),
    createCrewSetupAuthentication: vi.fn(),
  } as unknown as typeof window.electron;
});

afterEach(() => {
  window.electron = originalElectron;
});

const setupStep: SignIn = {
  kind: 'signIn',
  target: { kind: 'setup', jobId: 'job-1' },
  server: 'hpc.example.edu',
  login: 'you@hpc.example.edu',
};

const connectionStep: SignIn = {
  kind: 'signIn',
  target: { kind: 'connection', connectionId: 'conn-1' },
  server: 'lab.example.edu',
  login: 'me@lab.example.edu',
};

function card(step: SignIn = setupStep) {
  return render(<CrewSignInStep sessionId="chat-1" id="card-1" step={step} />);
}

function lastDialog(): DialogProps {
  const props = mocks.dialogs[mocks.dialogs.length - 1];
  if (!props) throw new Error('no Sign in window was mounted');
  return props;
}

/** No surface on the card can take a value. */
/**
 * The card's controls that act: every button but the title's info icon, which only explains
 * (design.md 9.16).
 */
function actionButtons() {
  return screen
    .queryAllByRole('button')
    .filter((button) => !button.getAttribute('aria-label')?.startsWith('About '));
}

function expectNoInput(container: HTMLElement) {
  expect(container.querySelector('input, textarea, select, [contenteditable]')).toBeNull();
}

describe('the sign-in card', () => {
  it('says in fixed words where the typing goes, with the daemon’s values sanitized', () => {
    const view = card({
      ...setupStep,
      login: 'you\u202E@hpc.example.edu\u200B',
      server: 'hpc\u200B.example.edu',
    });
    expect(screen.getByText(crewSignInStepCopy.title('you@hpc.example.edu'))).toBeInTheDocument();
    // Where the typing goes is a privacy disclosure, so it is the line in view (owner,
    // 2026-11-13; design.md 9.16).
    const lead = screen.getByText(crewSignInStepCopy.lead);
    expect(lead).not.toHaveClass('sr-only');
    expect(crewSignInStepCopy.lead).toBe(
      'What you type goes to ssh on that server, never to this chat or the model.'
    );
    // Which server, and where to type, are the title's info tip, word for word, and only there.
    const about = crewSignInStepCopy.about('you@hpc.example.edu', 'hpc.example.edu');
    expect(about).toBe(
      'Sign in to you@hpc.example.edu on hpc.example.edu. Type only into the Sign in window.'
    );
    expect(
      screen.getByRole('button', { name: `About ${crewSignInStepCopy.aboutLabel}` })
    ).toHaveAccessibleDescription(about);
    expect(screen.getByText(about)).toHaveClass('sr-only');
    expectNoInput(view.container);
  });

  it('opens the Sign in window for the setup job the card names', () => {
    const view = card();
    expect(screen.queryByTestId('sign-in-window')).toBeNull();
    fireEvent.click(
      screen.getByRole('button', { name: 'Sign in to you@hpc.example.edu on hpc.example.edu' })
    );
    expect(screen.getByTestId('sign-in-window')).toHaveAttribute(
      'data-host',
      'you@hpc.example.edu'
    );
    expect(lastDialog()).toMatchObject({
      target: { kind: 'setup', jobId: 'job-1' },
      open: true,
      host: 'you@hpc.example.edu',
    });
    expectNoInput(view.container);
  });

  it('opens the Sign in window for the saved connection the card names', () => {
    mocks.windowName = 'lab server';
    card(connectionStep);
    fireEvent.click(screen.getByRole('button', { name: /^Sign in to me@lab\.example\.edu/ }));
    expect(lastDialog()).toMatchObject({
      target: { kind: 'connection', id: 'conn-1' },
      open: true,
      host: 'lab server',
    });
    expect(mocks.asked[mocks.asked.length - 1]).toEqual({
      target: { kind: 'connection', id: 'conn-1' },
      enabled: true,
    });
  });

  it('names the Sign in window by the daemon’s read of the target, never by the card’s words', () => {
    // The card says one server; the window connects to the job's ID, so it names what that is.
    mocks.windowName = null;
    const view = card({ ...setupStep, server: 'not-the-server.example', login: 'someone@else' });
    fireEvent.click(screen.getByRole('button', { name: /^Sign in to/ }));
    expect(lastDialog().host).toBe(crewSignInStepCopy.windowFallback);
    expect(mocks.asked[mocks.asked.length - 1]).toEqual({
      target: { kind: 'setup', jobId: 'job-1' },
      enabled: true,
    });

    mocks.windowName = 'you@hpc.example.edu';
    view.rerender(
      <CrewSignInStep
        sessionId="chat-1"
        id="card-1"
        step={{ ...setupStep, server: 'not-the-server.example', login: 'someone@else' }}
      />
    );
    expect(lastDialog().host).toBe('you@hpc.example.edu');
    for (const dialog of mocks.dialogs) {
      expect(dialog.host).not.toContain('not-the-server');
      expect(dialog.host).not.toContain('someone@else');
    }
  });

  it('tells its card while the Sign in window is open', () => {
    const onHolding = vi.fn();
    render(
      <CrewSignInStep sessionId="chat-1" id="card-1" step={setupStep} onHolding={onHolding} />
    );
    expect(onHolding).toHaveBeenLastCalledWith(false);
    fireEvent.click(screen.getByRole('button', { name: /^Sign in to/ }));
    expect(onHolding).toHaveBeenLastCalledWith(true);
    act(() => lastDialog().onClose());
    expect(onHolding).toHaveBeenLastCalledWith(false);
  });

  it('says it is signed in once the window finishes, and offers nothing more', () => {
    card();
    fireEvent.click(screen.getByRole('button', { name: /^Sign in to/ }));
    act(() => lastDialog().onSignedIn());
    expect(screen.getByRole('status')).toHaveTextContent(crewSignInStepCopy.signedIn);
    expect(actionButtons()).toEqual([]);
    expect(lastDialog().open).toBe(false);
  });

  it('dismisses with the card and its chat, and nothing else', async () => {
    card();
    fireEvent.click(screen.getByRole('button', { name: crewSignInStepCopy.dismiss }));
    await waitFor(() =>
      expect(screen.getByRole('status')).toHaveTextContent(crewSignInStepCopy.dismissed)
    );
    expect(mocks.dismissCrewStep).toHaveBeenCalledTimes(1);
    // The headers are the frame's (the person's proof); the body names this card alone.
    expect(mocks.dismissCrewStep).toHaveBeenCalledWith(
      expect.objectContaining({
        body: { id: 'card-1', session_id: 'chat-1', outcome: 'dismiss' },
      })
    );
    expect(actionButtons()).toEqual([]);
  });

  it('says the sign-in no longer waits when the daemon had no such card', async () => {
    mocks.dismissCrewStep.mockResolvedValueOnce({ error: { status: 'unknown' } });
    card();
    fireEvent.click(screen.getByRole('button', { name: crewSignInStepCopy.dismiss }));
    await waitFor(() =>
      expect(screen.getByRole('status')).toHaveTextContent(crewSignInStepCopy.gone)
    );
  });

  it('in a browser says Sign in is only in the desktop app, and opens nothing', () => {
    mocks.browser = true;
    const view = card();
    expect(screen.getByText(crewSignInStepCopy.fallback)).toBeInTheDocument();
    expect(crewSignInStepCopy.fallback).toBe(
      'Sign in is only in the desktop app. Never type your password into the chat.'
    );
    // Where to sign in instead is folded under its disclosure, and still all there once opened.
    expect(screen.queryByText(crewSignInStepCopy.fallbackHow)).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: crewSignInStepCopy.fallbackHowLabel }));
    expect(screen.getByText(crewSignInStepCopy.fallbackHow)).toBeInTheDocument();
    expect(crewSignInStepCopy.fallbackHow).toBe(
      'Open Crew there and choose Sign in, or run biorouter crew auth in a terminal.'
    );
    expect(screen.queryByRole('button', { name: /^Sign in to/ })).toBeNull();
    // No info tip names a server the card cannot sign in to here.
    expect(
      screen.queryByRole('button', { name: `About ${crewSignInStepCopy.aboutLabel}` })
    ).toBeNull();
    expect(mocks.dialogs).toHaveLength(0);
    // Nothing is read for a window that cannot open.
    expect(mocks.asked.every(({ enabled }) => !enabled)).toBe(true);
    expect(screen.getByRole('button', { name: crewSignInStepCopy.dismiss })).toBeInTheDocument();
    expectNoInput(view.container);
  });

  it('in an app without the Sign in window says so too', () => {
    window.electron = {
      createCrewAuthentication: vi.fn(),
    } as unknown as typeof window.electron;
    card();
    expect(screen.getByText(crewSignInStepCopy.fallback)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /^Sign in to/ })).toBeNull();
    expect(mocks.dialogs).toHaveLength(0);
  });
});

describe('the sign-in copy', () => {
  it('says computer, never device, uses no dashes, and asks for nothing in the chat', () => {
    const words = [
      crewSignInStepCopy.title('x'),
      crewSignInStepCopy.lead,
      crewSignInStepCopy.aboutLabel,
      crewSignInStepCopy.about('x', 'y'),
      crewSignInStepCopy.signIn,
      crewSignInStepCopy.signInLabel('x', 'y'),
      crewSignInStepCopy.dismiss,
      crewSignInStepCopy.signedIn,
      crewSignInStepCopy.dismissed,
      crewSignInStepCopy.gone,
      crewSignInStepCopy.windowFallback,
      crewSignInStepCopy.fallback,
      crewSignInStepCopy.fallbackHowLabel,
      crewSignInStepCopy.fallbackHow,
    ];
    for (const text of words) {
      expect(text).not.toMatch(/\bdevices?\b/i);
      expect(text).not.toMatch(/[\u2013\u2014]/);
    }
  });
});
