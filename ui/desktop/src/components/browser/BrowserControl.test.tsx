import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { BrowserControl } from './BrowserControl';
import type { BrowserStatus } from './browserApi';

const mocks = vi.hoisted(() => ({
  status: vi.fn(),
  allow: vi.fn(),
  stop: vi.fn(),
  NotApplicable: class BrowserNotApplicable extends Error {},
}));
vi.mock('./browserApi', () => ({
  browserStatus: mocks.status,
  allowBrowser: mocks.allow,
  stopBrowser: mocks.stop,
  BrowserNotApplicable: mocks.NotApplicable,
}));

const base: BrowserStatus = {
  enabled: true,
  available: true,
  session_id: 'chat-a',
  provider: 'public-provider',
  model: 'model-a',
  destination: 'https://provider.example/api',
  public_model: true,
  state: 'approval_required',
  requested: 'observe',
  observe: false,
  act: null,
  challenge_id: 'challenge-a',
  disclosure:
    'Let Biorouter browse with public-provider/model-a? Pages it opens in the Biorouter Browser, including sites where you are logged in, and screenshots of them go to https://provider.example/api.',
  activity_id: 'activity-a',
};

beforeEach(() => {
  vi.clearAllMocks();
  mocks.status.mockResolvedValue(base);
  mocks.allow.mockResolvedValue({ ...base, state: 'active', requested: null, observe: true });
  mocks.stop.mockResolvedValue({ ...base, state: 'stopped', requested: null });
});
afterEach(() => vi.useRealTimers());

describe('BrowserControl (BB-17, BB-18)', () => {
  it('asks to browse with the full consent, the model and the destination', async () => {
    render(<BrowserControl sessionId="chat-a" />);
    expect(
      await screen.findByText('Biorouter Browser wants to read web pages')
    ).toBeInTheDocument();
    expect(
      screen.getByText(/Let Biorouter browse with public-provider\/model-a/)
    ).toBeInTheDocument();
    expect(screen.getByText('Data destination: https://provider.example/api')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Allow reading' }));
    await waitFor(() => expect(mocks.allow).toHaveBeenCalledWith(base, 'observe', 'request'));
    expect(await screen.findByText(/Biorouter Browser active/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Stop' })).toBeInTheDocument();
  });

  it('asks to act for the request, or for the chat from the secondary button', async () => {
    const acting = {
      ...base,
      requested: 'act' as const,
      observe: true,
      disclosure:
        'Let Biorouter click, type and submit forms… Stop cannot undo changes already made.',
    };
    mocks.status.mockResolvedValue(acting);
    render(<BrowserControl sessionId="chat-a" />);
    expect(
      await screen.findByText('Biorouter Browser wants to open or act on websites')
    ).toBeInTheDocument();
    // The consequence is part of the consent text, shown once and in full.
    expect(screen.getAllByText(/Stop cannot undo changes already made\./)).toHaveLength(1);
    fireEvent.click(screen.getByRole('button', { name: 'Allow for this chat' }));
    await waitFor(() => expect(mocks.allow).toHaveBeenCalledWith(acting, 'act', 'chat'));
  });

  it('cancels and stops through the one revoke', async () => {
    render(<BrowserControl sessionId="chat-a" />);
    fireEvent.click(await screen.findByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(mocks.stop).toHaveBeenCalledWith('chat-a'));
  });

  it('keeps Stop in view while a grant is held', async () => {
    mocks.status.mockResolvedValue({
      ...base,
      state: 'active',
      requested: null,
      observe: true,
      act: 'request',
    });
    render(<BrowserControl sessionId="chat-a" />);
    expect(await screen.findByText(/reading, and acting for this request/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
    await waitFor(() => expect(mocks.stop).toHaveBeenCalledWith('chat-a'));
  });

  it('shows nothing when idle, where no desktop hosts the browser, or in Chat mode', async () => {
    mocks.status.mockResolvedValue({ ...base, state: 'idle', requested: null, activity_id: null });
    const idle = render(<BrowserControl sessionId="chat-a" />);
    await waitFor(() => expect(mocks.status).toHaveBeenCalled());
    expect(idle.container).toBeEmptyDOMElement();
    idle.unmount();

    mocks.status.mockResolvedValue({ ...base, available: false });
    const serve = render(<BrowserControl sessionId="chat-a" />);
    await waitFor(() => expect(mocks.status).toHaveBeenCalled());
    expect(serve.container).toBeEmptyDOMElement();
    serve.unmount();

    mocks.status.mockRejectedValue(
      new mocks.NotApplicable('Chat mode does not run Biorouter Browser tools')
    );
    const chat = render(<BrowserControl sessionId="chat-a" />);
    await waitFor(() => expect(mocks.status).toHaveBeenCalled());
    expect(chat.container).toBeEmptyDOMElement();
  });
});
