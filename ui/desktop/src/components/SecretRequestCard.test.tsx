import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ActionRequired } from '../api';

/**
 * Issue #117. **The point of this suite is what does NOT happen.**
 *
 * A passing render test would prove nothing here: the defect the feature exists
 * to prevent is a secret reaching the transcript, and a card that both renders
 * correctly and appends the value would pass every ordinary assertion. So these
 * tests assert the negative — no message is created, the value never appears in
 * anything the conversation can carry, and the source tree contains no
 * response-message constructor that could carry one.
 */

const submitSecrets = vi.hoisted(() => vi.fn());
vi.mock('../api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../api')>()),
  submitSecrets,
}));

const userActionHeaders = vi.hoisted(() => vi.fn(async () => ({ 'X-User-Action': 'proof' })));
vi.mock('../utils/userAction', () => ({ userActionHeaders }));

import SecretRequestCard from './SecretRequestCard';

const SECRET = 'sk-live-do-not-leak-me';

function card(
  keys: Array<{ key: string; label: string; required?: boolean; description?: string }>
): ActionRequired & { type: 'actionRequired' } {
  return {
    type: 'actionRequired',
    data: {
      actionType: 'secretRequest',
      id: 'card-1',
      prompt: 'SPOKE Agent needs 1 value before it can run.',
      keys,
      destination: { kind: 'extensionEnv', extensionName: 'spokeagent' },
    },
  };
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('SecretRequestCard — issue #117', () => {
  it('sends the value to the daemon with the proof-of-user header, and creates no message', async () => {
    const user = userEvent.setup();
    submitSecrets.mockResolvedValue({
      data: { status: 'configured', configuredKeys: ['PASSCODE'] },
    });

    render(
      <SecretRequestCard
        isCancelledMessage={false}
        actionRequiredContent={card([{ key: 'PASSCODE', label: 'PASSCODE', required: true }])}
      />
    );

    await user.type(screen.getByLabelText(/PASSCODE/), SECRET);
    await user.click(screen.getByRole('button', { name: 'Save and continue' }));

    await waitFor(() => expect(submitSecrets).toHaveBeenCalled());
    const call = submitSecrets.mock.calls[0][0];
    expect(call.body).toEqual({ id: 'card-1', values: { PASSCODE: SECRET } });
    // DR-16: without this the model could satisfy its own credential card.
    expect(call.headers).toEqual({ 'X-User-Action': 'proof' });
    expect(userActionHeaders).toHaveBeenCalled();
  });

  /**
   * ⚠ The component takes no `append`, no `onSubmit` and no session id — it
   * cannot put anything into the conversation even if it tried. This asserts
   * the consequence: after a successful save, the only thing on screen is the
   * key's NAME.
   */
  it('reports the key name and never the value once configured', async () => {
    const user = userEvent.setup();
    submitSecrets.mockResolvedValue({
      data: { status: 'configured', configuredKeys: ['PASSCODE'] },
    });

    const { container } = render(
      <SecretRequestCard
        isCancelledMessage={false}
        actionRequiredContent={card([{ key: 'PASSCODE', label: 'PASSCODE', required: true }])}
      />
    );

    await user.type(screen.getByLabelText(/PASSCODE/), SECRET);
    await user.click(screen.getByRole('button', { name: 'Save and continue' }));

    expect(
      await screen.findByText(/Credentials configured for spokeagent: PASSCODE/)
    ).toBeInTheDocument();
    expect(container.innerHTML).not.toContain(SECRET);
  });

  it('masks by default and reveals only on an explicit click', async () => {
    const user = userEvent.setup();
    render(
      <SecretRequestCard
        isCancelledMessage={false}
        actionRequiredContent={card([{ key: 'PASSCODE', label: 'PASSCODE', required: true }])}
      />
    );

    const input = screen.getByLabelText(/PASSCODE/);
    expect(input).toHaveAttribute('type', 'password');
    // Never pre-filled: a default would have to be read back out of the
    // credential store.
    expect(input).toHaveValue('');

    await user.click(screen.getByRole('button', { name: 'Show' }));
    expect(screen.getByLabelText(/PASSCODE/)).toHaveAttribute('type', 'text');
  });

  it('gates Save on the required fields and separates the optional ones', async () => {
    const user = userEvent.setup();
    render(
      <SecretRequestCard
        isCancelledMessage={false}
        actionRequiredContent={card([
          { key: 'PASSCODE', label: 'PASSCODE', required: true },
          { key: 'SPOKE_HOST', label: 'SPOKE_HOST' },
        ])}
      />
    );

    expect(screen.getByText('Required')).toBeInTheDocument();
    expect(screen.getByText('Optional')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Save and continue' })).toBeDisabled();

    await user.type(screen.getByLabelText(/PASSCODE/), 'x');
    expect(screen.getByRole('button', { name: 'Save and continue' })).toBeEnabled();
  });

  it('cancel posts a dismissal rather than an empty set of values', async () => {
    const user = userEvent.setup();
    submitSecrets.mockResolvedValue({ data: { status: 'cancelled' } });

    render(
      <SecretRequestCard
        isCancelledMessage={false}
        actionRequiredContent={card([{ key: 'PASSCODE', label: 'PASSCODE', required: true }])}
      />
    );

    await user.click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(submitSecrets).toHaveBeenCalled());
    expect(submitSecrets.mock.calls[0][0].body).toEqual({ id: 'card-1', cancelled: true });
    // The two spellings on these two lines are both correct and are not the same
    // kind of thing: `cancelled` above is the daemon's wire value, `canceled` below
    // is copy, which is American English (design.md §3.10).
    expect(await screen.findByText(/canceled/i)).toBeInTheDocument();
  });

  /**
   * The daemon is the authority on what is required — the card only knows what
   * the manifest declared when it was published. When the two disagree the
   * dialog stays open with the field flagged, because releasing the install on a
   * disagreement would turn it into a rollback the user has to start over from.
   */
  it('a daemon "incomplete" keeps the dialog open instead of losing the install', async () => {
    const user = userEvent.setup();
    submitSecrets.mockResolvedValue({ data: { status: 'incomplete', missing: ['PASSCODE'] } });

    render(
      <SecretRequestCard
        isCancelledMessage={false}
        actionRequiredContent={card([
          { key: 'PASSCODE', label: 'PASSCODE', required: true },
          { key: 'SPOKE_HOST', label: 'SPOKE_HOST' },
        ])}
      />
    );

    await user.type(screen.getByLabelText(/PASSCODE/), 'typed-but-rejected');
    await user.click(screen.getByRole('button', { name: 'Save and continue' }));

    expect(await screen.findByText(/Still needed: PASSCODE/)).toBeInTheDocument();
    // The form is still there to correct.
    expect(screen.getByLabelText(/PASSCODE/)).toBeInTheDocument();
  });

  /** The client's own gate, before the daemon's. */
  it('will not submit at all while a required field is blank', async () => {
    const user = userEvent.setup();
    render(
      <SecretRequestCard
        isCancelledMessage={false}
        actionRequiredContent={card([{ key: 'PASSCODE', label: 'PASSCODE', required: true }])}
      />
    );

    await user.type(screen.getByLabelText(/PASSCODE/), '   ');
    expect(screen.getByRole('button', { name: 'Save and continue' })).toBeDisabled();
    expect(submitSecrets).not.toHaveBeenCalled();
  });

  it('says so when the install stopped waiting, instead of spinning forever', async () => {
    const user = userEvent.setup();
    submitSecrets.mockResolvedValue({ data: { status: 'unknown' } });

    render(
      <SecretRequestCard
        isCancelledMessage={false}
        actionRequiredContent={card([{ key: 'PASSCODE', label: 'PASSCODE', required: true }])}
      />
    );

    await user.type(screen.getByLabelText(/PASSCODE/), 'x');
    await user.click(screen.getByRole('button', { name: 'Save and continue' }));

    expect(await screen.findByText(/no longer waiting for an answer/i)).toBeInTheDocument();
  });
});

/**
 * A structural assertion, because the defect it guards against is one line of
 * plausible code away.
 *
 * `createElicitationResponseMessage` is the pattern every other card follows,
 * and a `createSecretResponseMessage` beside it would look like the obvious
 * next step — while putting the credential into an `agentVisible` message that
 * the agent forwards verbatim to the parked request. There is no daemon-side
 * `SecretResponse` variant for it to serialise into, and there must be no
 * renderer-side constructor for one either.
 */
describe('the conversation transport has no way to carry a credential', () => {
  const src = (rel: string) => readFileSync(path.join(__dirname, rel), 'utf8');

  it('defines no secret-response message constructor', () => {
    // Declarations only — the file deliberately *names* the constructor that
    // must not exist, in the comment explaining why, so a bare substring match
    // would fail on the very documentation that guards it.
    const messageTs = src('../types/message.ts');
    expect(messageTs).not.toMatch(/export function createSecret\w*Response/);
    expect(messageTs).not.toMatch(/actionType:\s*['"]secretResponse['"]/);
  });

  it('the card takes no append/onSubmit seam a value could travel out through', () => {
    const card = src('./SecretRequestCard.tsx');
    const props = card.slice(card.indexOf('interface Props'), card.indexOf('type Status'));
    expect(props).not.toMatch(/append|onSubmit|sessionId/);
  });
});
