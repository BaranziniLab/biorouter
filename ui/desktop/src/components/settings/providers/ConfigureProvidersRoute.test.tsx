import { fireEvent, render, screen } from '@testing-library/react';
import { MemoryRouter, Route, Routes, useLocation } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';
import ConfigureProvidersRoute from './ConfigureProvidersRoute';

const seen = vi.hoisted(() => ({ props: null as Record<string, unknown> | null }));

vi.mock('./ProviderSettingsPage', () => ({
  default: (props: {
    onClose: () => void;
    onProviderLaunched?: () => void;
    chatSessionId?: string | null;
  }) => {
    seen.props = props as unknown as Record<string, unknown>;
    return (
      <div>
        <button onClick={props.onClose}>Back</button>
        <button onClick={() => props.onProviderLaunched?.()}>Model chosen</button>
      </div>
    );
  },
}));

function Where() {
  const location = useLocation();
  return <div data-testid="where">{`${location.pathname}${location.search}`}</div>;
}

function renderAt(state?: unknown) {
  return render(
    <MemoryRouter initialEntries={[{ pathname: '/configure-providers', state }]}>
      <Routes>
        <Route path="/configure-providers" element={<ConfigureProvidersRoute />} />
        <Route path="*" element={<Where />} />
      </Routes>
    </MemoryRouter>
  );
}

// W2-PRV-5: "Use other provider" navigated here with no return path, and Back
// went to Settings > Models.
describe('ConfigureProvidersRoute', () => {
  it('opened from a chat, goes back to that chat and scopes the model step to it', () => {
    renderAt({
      returnTo: '/pair?resumeSessionId=s-7',
      resumeSessionId: 's-7',
      privacyTier: 'public',
    });
    expect(seen.props?.chatSessionId).toBe('s-7');
    expect(seen.props?.chatPrivacyTier).toBe('public');

    fireEvent.click(screen.getByText('Model chosen'));
    expect(screen.getByTestId('where')).toHaveTextContent('/pair?resumeSessionId=s-7');
  });

  it('Back returns to where it was opened from', () => {
    renderAt({ returnTo: '/pair?resumeSessionId=s-7', resumeSessionId: 's-7' });
    fireEvent.click(screen.getByText('Back'));
    expect(screen.getByTestId('where')).toHaveTextContent('/pair?resumeSessionId=s-7');
  });

  // T3-SH-2: a chat not sent yet has no session; it is named by its tab.
  it('opened from a chat not sent yet, names it by its tab and goes back to it', () => {
    renderAt({ returnTo: '/pair', heldChatTabId: 'tab-3' });
    expect(seen.props?.chatSessionId).toBeNull();
    expect(seen.props?.heldChatTabId).toBe('tab-3');

    fireEvent.click(screen.getByText('Model chosen'));
    expect(screen.getByTestId('where')).toHaveTextContent('/pair');
  });

  it('a started chat outranks a tab: its session is what the model step switches', () => {
    renderAt({
      returnTo: '/pair?resumeSessionId=s-7',
      resumeSessionId: 's-7',
      heldChatTabId: 'tab-3',
    });
    expect(seen.props?.chatSessionId).toBe('s-7');
    expect(seen.props?.heldChatTabId).toBeNull();
  });

  it('opened from Settings, Back still goes to Settings and nothing is scoped to a chat', () => {
    renderAt();
    expect(seen.props?.chatSessionId).toBeNull();
    expect(seen.props?.heldChatTabId).toBeNull();
    expect(seen.props?.onProviderLaunched).toBeUndefined();
    fireEvent.click(screen.getByText('Back'));
    expect(screen.getByTestId('where')).toHaveTextContent('/settings');
  });
});
