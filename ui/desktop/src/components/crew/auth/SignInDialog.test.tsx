import { fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { fakeConnection, makeCrew, renderWithCrew } from '../onboarding/testCrew';
import { useCrew } from '../state/CrewControllerContext';
import type { CrewController } from '../state/types';
import { signInCopy } from './copy';
import { SignInDialog } from './SignInDialog';

const auth = vi.hoisted(() => ({ props: [] as { connectionId: string }[] }));

// The terminal body has its own regression suite; here it stands in with its contract.
vi.mock('../CrewAuthentication', () => ({
  default: (props: { connectionId: string; onConnected: () => void; onClose: () => void }) => {
    auth.props.push(props);
    return (
      <div data-testid="crew-authentication">
        <button onClick={props.onConnected}>Simulate authenticated completion</button>
        <button onClick={props.onClose} aria-label={signInCopy.closeName}>
          {signInCopy.close}
        </button>
      </div>
    );
  },
}));

function renderSignIn(overrides: Partial<CrewController> = {}) {
  const crew = makeCrew({
    connectionId: 'conn-1',
    connection: fakeConnection(),
    signIn: { open: true, reason: 'auto' },
    ...overrides,
  });
  return { crew, view: renderWithCrew(<SignInDialog />, crew) };
}

describe('SignInDialog', () => {
  it('names the server, says one line about what to type, and holds the terminal', () => {
    renderSignIn();
    const dialog = screen.getByRole('dialog', { name: signInCopy.title('hpc.ucsf.edu') });
    expect(dialog).toHaveAccessibleDescription(signInCopy.lead);
    expect(screen.getByTestId('crew-authentication')).toBeInTheDocument();
    expect(auth.props[auth.props.length - 1].connectionId).toBe('conn-1');
  });

  it('has no × and cannot be dismissed with Escape or a click outside', () => {
    const { crew } = renderSignIn();
    const dialog = screen.getByRole('dialog');
    expect(screen.queryByRole('button', { name: 'Close' })).toBeNull();

    // The dialog refuses Escape outright (the event's default is prevented), not merely ignores it.
    expect(fireEvent.keyDown(dialog, { key: 'Escape', code: 'Escape' })).toBe(false);
    const overlay = document.querySelector('[data-slot="dialog-overlay"]');
    expect(overlay).not.toBeNull();
    fireEvent.pointerDown(overlay as Element);
    fireEvent.mouseDown(overlay as Element);
    fireEvent.click(overlay as Element);

    expect(crew.closeSignIn).not.toHaveBeenCalled();
    expect(screen.getByRole('dialog')).toBeInTheDocument();
  });

  it('closes only through the terminal’s own Close, and completes without a second connect', () => {
    const { crew } = renderSignIn();
    fireEvent.click(screen.getByRole('button', { name: signInCopy.closeName }));
    expect(crew.closeSignIn).toHaveBeenCalledOnce();

    fireEvent.click(screen.getByRole('button', { name: 'Simulate authenticated completion' }));
    expect(crew.onSignedIn).toHaveBeenCalledOnce();
    expect(crew.connect).not.toHaveBeenCalled();
  });

  it('renders nothing while sign-in is closed', () => {
    renderSignIn({ signIn: { open: false, reason: null } });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(screen.queryByTestId('crew-authentication')).toBeNull();
  });
});

describe('SignInDialog focus return (QA T-15)', () => {
  it('gives focus back to whatever opened it once it has closed', async () => {
    const crew = makeCrew({
      connectionId: 'conn-1',
      connection: fakeConnection(),
      signIn: { open: false, reason: null },
    });
    const view = renderWithCrew(
      <>
        <button type="button">Sign in…</button>
        <SignInDialog />
      </>,
      crew
    );
    const opener = screen.getByRole('button', { name: 'Sign in…' });
    opener.focus();
    view.update({ signIn: { open: true, reason: 'user' } });
    const dialog = await screen.findByRole('dialog');
    await waitFor(() => expect(dialog).toContainElement(document.activeElement as HTMLElement));

    view.update({ signIn: { open: false, reason: null } });
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    await waitFor(() => expect(opener).toHaveFocus());
  });

  it('falls back to the workspace switcher when its opener is gone', async () => {
    const crew = makeCrew({
      connectionId: 'conn-1',
      connection: fakeConnection(),
      signIn: { open: false, reason: null },
    });
    // The connect screen that held the button is replaced while the terminal is open.
    const state = { withOpener: true };
    function Screen() {
      // Reads the controller, so it re-renders with it as a real screen does.
      useCrew();
      return (
        <div className="crew-app">
          <button type="button" className="crew-sidebar-switcher">
            lab
          </button>
          {state.withOpener ? <button type="button">Connect</button> : null}
          <SignInDialog />
        </div>
      );
    }
    const view = renderWithCrew(<Screen />, crew);
    screen.getByRole('button', { name: 'Connect' }).focus();
    view.update({ signIn: { open: true, reason: 'auto' } });
    await screen.findByRole('dialog');
    state.withOpener = false;
    view.update({ signIn: { open: false, reason: null } });
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    await waitFor(() => expect(screen.getByRole('button', { name: 'lab' })).toHaveFocus());
  });
});
