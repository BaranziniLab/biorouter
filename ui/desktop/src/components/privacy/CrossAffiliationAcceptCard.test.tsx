import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { CrossAffiliationAcceptCard } from './CrossAffiliationAcceptCard';
// The real constant, through the partial mock's `importOriginal` below — so the
// scope test compares the card against the mirrored bytes rather than against a
// second hand-typed copy, which would drift in exactly the direction the mirror
// exists to catch.
import { GRANT_SCOPE_COPY } from '../../utils/crossAffiliation';

const mockReadMixingMode = vi.fn();
const mockAccept = vi.fn();

vi.mock('../../utils/crossAffiliation', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../utils/crossAffiliation')>()),
  readMixingMode: () => mockReadMixingMode(),
  acceptCrossAffiliationFlow: (...args: unknown[]) => mockAccept(...args),
}));

const ACCEPTED =
  'Cross-institutional data flow. The extension `ucsfomopagent` holds data belonging to UCSF, ' +
  'but this chat is bound to a model covered by Stanford’s agreements. Approving records your ' +
  'acceptance for this chat, this extension and this model’s institution only.';

beforeEach(() => {
  vi.clearAllMocks();
  mockReadMixingMode.mockResolvedValue('standard');
  mockAccept.mockResolvedValue(ACCEPTED);
});

const renderCard = (props: Partial<{ sessionId: string; extension: string }> = {}) =>
  render(
    <CrossAffiliationAcceptCard
      sessionId={props.sessionId ?? 'chat_7'}
      extension={props.extension ?? 'ucsfomopagent'}
    />
  );

describe('CrossAffiliationAcceptCard', () => {
  it('records the acceptance for the refused triple and shows the daemon’s own statement', async () => {
    const user = userEvent.setup();
    renderCard();

    const button = await screen.findByRole('button', { name: /approve this flow/i });
    await user.click(button);

    // Session and extension exactly as refused — the daemon reads the
    // institution off the model it samples, and the request carries none.
    expect(mockAccept).toHaveBeenCalledWith('chat_7', 'ucsfomopagent');

    // The confirmation is the daemon's composition, not a paraphrase: the
    // sentence recorded and the sentence read must not differ by a word.
    expect(await screen.findByText(ACCEPTED)).toBeInTheDocument();
    // …and the control is gone, so a second press cannot re-post it.
    expect(screen.queryByRole('button', { name: /approve this flow/i })).not.toBeInTheDocument();
    // The refusal told the model not to retry, and it will not. A recorded
    // acceptance beside a conversation that has stopped is the same dead end
    // one press further along, so the card says who moves next.
    expect(screen.getByTestId('cross-affiliation-accepted')).toHaveTextContent(
      /try that step again/i
    );
  });

  it('says what failed and leaves the control pressable when the daemon refuses', async () => {
    const user = userEvent.setup();
    // The realistic failure: a daemon the app did not start, holding no
    // user-action key (open question 23).
    mockAccept.mockRejectedValue('This backend was started without a user-action key.');
    renderCard();

    await user.click(await screen.findByRole('button', { name: /approve this flow/i }));

    expect(await screen.findByRole('alert')).toHaveTextContent(/user-action key/i);
    // Nothing was recorded, so the way forward must still be on screen.
    expect(screen.getByRole('button', { name: /approve this flow/i })).toBeInTheDocument();
  });

  it('renders nothing under the open mixing policy', async () => {
    mockReadMixingMode.mockResolvedValue('open');
    const { container } = renderCard();

    // Settled — the mode really was read and answered, so this is not just the
    // pre-read blank.
    await waitFor(() => expect(mockReadMixingMode).toHaveBeenCalled());
    await waitFor(() => expect(container).toBeEmptyDOMElement());
    expect(mockAccept).not.toHaveBeenCalled();
  });

  it('offers the same control under strict, and warns the system may ask', async () => {
    // Issue #56, DR-27. Its three modes differ in what the press COSTS, never
    // in whether there is one — the extra proof is the daemon's to demand
    // (`strict_mode_authorization` reads `mixing::policy()` and raises DR-20's
    // dialog between the resolution and the write). A renderer that withheld the
    // button under `strict` restores the hard block DR-26 exists to prevent, for
    // exactly the deployments careful enough to choose it.
    const user = userEvent.setup();
    mockReadMixingMode.mockResolvedValue('strict');
    renderCard();

    // Told BEFORE the press, not after: a system dialog nobody was warned about
    // is one people dismiss as spurious, and dismissing it leaves the flow
    // refused.
    //
    // ⚠ "may ask", never "will ask". F-13 measured macOS approving
    // `evaluatePolicy` instantly, with no dialog at all, off a recent
    // authentication; whether an explicit reuse duration of 0 defeats that is
    // still untested. Promising a dialog that then does not appear is worse
    // than not promising one: it teaches the user that a silent approval on
    // this prompt is normal, and this prompt is the one gating enforcement.
    const notice = await screen.findByTestId('cross-affiliation-strict-notice');
    expect(notice).toHaveTextContent(/operating system may ask you/i);
    expect(notice).not.toHaveTextContent(/will ask you/i);

    // …and the control itself is the same one, posting the same triple.
    await user.click(await screen.findByRole('button', { name: /approve this flow/i }));
    expect(mockAccept).toHaveBeenCalledWith('chat_7', 'ucsfomopagent');
    expect(await screen.findByText(ACCEPTED)).toBeInTheDocument();
  });

  // V4-R7-6, class only (privacy UI, ruling 15): the emphasised word is 600, not the preflight's
  // `bolder` (700). `privacyEmphasisWeight.browser.test.tsx` measures the computed weight.
  it('sets the emphasised policy word at the strong weight, 600', async () => {
    mockReadMixingMode.mockResolvedValue('strict');
    renderCard();
    const notice = await screen.findByTestId('cross-affiliation-strict-notice');
    const emphasised = notice.querySelectorAll('strong');
    expect([...emphasised].map((el) => el.textContent)).toEqual(['strict']);
    for (const el of emphasised) expect(el).toHaveClass('font-semibold');
    // The space round the word is the sentence's own: prettier re-wrapped the line.
    expect(notice).toHaveTextContent(/policy is set to strict, so your operating system/);
  });

  it('does not warn about a system password under standard', async () => {
    // The other half of the same claim: `standard` must be exactly today's
    // behaviour — one in-app confirmation — so a card that showed the strict
    // sentence unconditionally would be telling most users something false about
    // the control they are about to use.
    renderCard();
    await screen.findByRole('button', { name: /approve this flow/i });
    expect(screen.queryByTestId('cross-affiliation-strict-notice')).not.toBeInTheDocument();
    expect(screen.getByTestId('cross-affiliation-accept')).not.toHaveTextContent(
      /operating system/i
    );
  });

  it('says the operating system refused and leaves the control pressable under strict', async () => {
    // The daemon's 403 when the password prompt is denied or unavailable. It
    // must read as the user's own answer (or their machine's incapacity) rather
    // than as a broken button — which is why the daemon's sentence is printed
    // verbatim and the way forward stays on screen.
    const user = userEvent.setup();
    mockReadMixingMode.mockResolvedValue('strict');
    mockAccept.mockRejectedValue(
      "This machine's cross-institution mixing policy is set to 'strict', so accepting a " +
        'cross-institutional data flow needs your operating system to confirm it is you as well ' +
        'as the in-app approval. That did not happen. Nothing was recorded.'
    );
    renderCard();

    await user.click(await screen.findByRole('button', { name: /approve this flow/i }));

    expect(await screen.findByRole('alert')).toHaveTextContent(/Nothing was recorded/);
    expect(screen.getByRole('button', { name: /approve this flow/i })).toBeInTheDocument();
    // No acceptance was drawn on a grant that was never written.
    expect(screen.queryByTestId('cross-affiliation-accepted')).not.toBeInTheDocument();
  });

  it('offers nothing when there is no chat to key the acceptance on', async () => {
    const { container } = render(
      <CrossAffiliationAcceptCard sessionId={undefined} extension="ucsfomopagent" />
    );
    await waitFor(() => expect(container).toBeEmptyDOMElement());
    expect(mockAccept).not.toHaveBeenCalled();
  });

  it('states the scope before the user presses, not only after', async () => {
    renderCard();
    await screen.findByRole('button', { name: /approve this flow/i });
    // "How far does my yes reach" is part of what is being decided.
    //
    // ⚠ The WHOLE sentence, byte for byte, and not three regexes for its three
    // narrowings. Review found the seam: `privacy::grant::tests::
    // the_scope_copy_the_user_reads_is_the_one_the_daemon_records` asserts the
    // constant exists in the module FILE, and a loose match here asserts the
    // card says something like it — so a card that kept the export and rendered
    // a paraphrase satisfied both, and the byte-level mirror the Rust test
    // exists to enforce stopped reaching the screen. This closes it: the two
    // tests now pin the same bytes at both ends of the wire.
    expect(screen.getByTestId('cross-affiliation-accept')).toHaveTextContent(GRANT_SCOPE_COPY);
  });
});
