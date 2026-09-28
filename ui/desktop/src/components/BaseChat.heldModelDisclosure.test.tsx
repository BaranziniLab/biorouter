import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

/**
 * T3-SH-1, end to end in the renderer: the real gate, the real disclosure
 * store and the real send check, wired the way `BaseChat` wires them for a chat
 * that has not been sent yet (`BaseChat` itself cannot be mounted in jsdom; the
 * wiring is pinned at the source in `BaseChat.pendingModel.test.ts`).
 *
 * The copy below is a FIXTURE; the product's sentence lives in Rust.
 */
const mocks = vi.hoisted(() => ({
  getPrivacyDisclosure: vi.fn(),
  ackPrivacyDisclosure: vi.fn(),
  getProviders: vi.fn(),
  /** `/agent/start`: the chat is created here, before anything is sent. */
  startChat: vi.fn(),
}));

vi.mock('../api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../api')>()),
  getPrivacyDisclosure: mocks.getPrivacyDisclosure,
  ackPrivacyDisclosure: mocks.ackPrivacyDisclosure,
}));
vi.mock('./ConfigContext', () => ({
  useConfig: () => ({ getProviders: mocks.getProviders }),
}));
vi.mock('../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-key' }),
}));
vi.mock('../toasts', () => ({ toastError: vi.fn(), toastSuccess: vi.fn(), toastWarning: vi.fn() }));

import { heldModelMayBeSent } from './BaseChat';
import { NonPrivateModelDisclosureGate } from './privacy/NonPrivateModelDisclosureGate';
import { __resetDisclosureStoreForTests } from './privacy/disclosureCopy';
import { useConfig } from './ConfigContext';
import type Model from './settings/models/modelInterface';

const provider = (name: string, tier: 'private' | 'public') => ({
  name,
  is_configured: true,
  provider_type: 'Builtin',
  metadata: { name, display_name: name, tier, config_keys: [], known_models: [] },
});

/** An unsent chat holding `held`, as `BaseChat` mounts one. */
function UnsentChat({ held }: { held: Model }) {
  const { getProviders } = useConfig();
  const [handedBack, setHandedBack] = useState(0);
  const send = async () => {
    if (!(await heldModelMayBeSent(held, getProviders))) {
      setHandedBack((n) => n + 1);
      return;
    }
    mocks.startChat(held);
  };
  return (
    <>
      <NonPrivateModelDisclosureGate providerName={held.provider} />
      <button type="button" onClick={() => void send()}>
        Send
      </button>
      <output data-testid="handed-back">{handedBack}</output>
    </>
  );
}

afterEach(cleanup);

beforeEach(() => {
  vi.clearAllMocks();
  __resetDisclosureStoreForTests();
  mocks.getPrivacyDisclosure.mockResolvedValue({
    data: {
      title_template: '{provider} is not hosted by your institution.',
      long: 'SERVED-COPY-MARKER',
      short: 'SERVED-SHORT-MARKER',
      acknowledged: false,
    },
  });
  mocks.ackPrivacyDisclosure.mockResolvedValue({ data: undefined });
  mocks.getProviders.mockResolvedValue([
    provider('claude_code', 'public'),
    provider('versa_azure', 'private'),
  ]);
});

describe('a model held for an unsent chat', () => {
  it('is disclosed before its chat is created, and the first send waits for the acknowledgement', async () => {
    const user = userEvent.setup();
    render(<UnsentChat held={{ name: 'claude-opus-5-5', provider: 'claude_code' }} />);

    // Up as soon as the pick is held: nothing has been typed, let alone sent.
    expect(
      await screen.findByRole('dialog', { name: /claude_code is not hosted by your institution/i })
    ).toBeVisible();
    expect(mocks.startChat).not.toHaveBeenCalled();

    await user.click(screen.getByRole('button', { name: /I understand/i }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(mocks.startChat).not.toHaveBeenCalled();

    await user.click(screen.getByRole('button', { name: 'Send' }));
    await waitFor(() => expect(mocks.startChat).toHaveBeenCalledTimes(1));
  });

  it('hands a send that beats the dialog back, and creates no chat', async () => {
    // The gate needs the daemon's answer before it can show anything; a send
    // inside that interval must not go out behind it.
    render(<UnsentChat held={{ name: 'claude-opus-5-5', provider: 'claude_code' }} />);
    screen.getByRole('button', { name: 'Send' }).click();

    await waitFor(() => expect(screen.getByTestId('handed-back')).toHaveTextContent('1'));
    expect(mocks.startChat).not.toHaveBeenCalled();
    expect(await screen.findByRole('dialog')).toBeVisible();
  });

  it('on a private model, sends at once and shows nothing', async () => {
    const user = userEvent.setup();
    render(<UnsentChat held={{ name: 'gpt-5.5', provider: 'versa_azure' }} />);

    await user.click(screen.getByRole('button', { name: 'Send' }));
    await waitFor(() => expect(mocks.startChat).toHaveBeenCalledTimes(1));
    expect(screen.queryByRole('dialog')).toBeNull();
  });
});
