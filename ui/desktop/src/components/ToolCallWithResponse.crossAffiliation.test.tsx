import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import ToolCallWithResponse from './ToolCallWithResponse';
import type { ToolRequestMessageContent } from '../types/message';
import { CROSS_AFFILIATION_ACCEPT_MARKER } from '../utils/crossAffiliation';

// The transcript is no longer a display surface for artifacts — it can only hand
// one to the panel — so `onOpenArtifact` is required all the way down the chain.
const noopOpenArtifact = vi.fn();

/**
 * Issue #56, DR-26 / Task 57 — the gate that matters.
 *
 * The mechanism has existed since Task 49: the route, the store, the triple, the
 * proof-of-user, all tested. What was missing was that **nobody could reach it**
 * — `agentCrossAffiliationGrant` had zero callers outside `src/api/`. So a test
 * that mounts the card directly and presses its button would prove exactly what
 * was already true and nothing that was broken.
 *
 * This one therefore starts where a user starts: the transcript, a tool call
 * that failed, the daemon's refusal inside it. Nothing here reaches for the
 * accept component by name — it renders `ToolCallWithResponse`, the component
 * chat actually uses, and looks for a control by the role and name a person
 * would.
 */
const mockReadConfig = vi.fn();
const mockGrant = vi.fn();

vi.mock('../api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../api')>()),
  readConfig: (...args: unknown[]) => mockReadConfig(...args),
  agentCrossAffiliationGrant: (...args: unknown[]) => mockGrant(...args),
}));

/**
 * The daemon's refusal for a mismatch Gate C would clear on a grant, assembled
 * the way `privacy::refusal::cross_affiliation_refusal(warning, Some(ext))`
 * assembles it. The frame is the MIRRORED constant, never a string typed here.
 */
const grantableRefusal = (extension: string) =>
  'Cross-institutional data flow. The extension `' +
  extension +
  '` holds data belonging to UCSF, but this chat is bound to a model covered by Stanford’s ' +
  'agreements. Using it would send `' +
  extension +
  '`’s inputs and results across that boundary. This call was not made. Only the user can ' +
  'accept a cross-institutional risk, and only after it has been stated to them, so do not ' +
  'retry. Tell the user what you were trying to do and ask them to approve this specific flow ' +
  'or to switch this chat to a model covered by the same institution’s agreements. ' +
  CROSS_AFFILIATION_ACCEPT_MARKER +
  '`' +
  extension +
  "`, on this chat's current model.";

/** The same refusal from a site that never consults the grant: no accept frame. */
const bareRefusal =
  'Cross-institutional data flow. The extension `ucsfomopagent` holds data belonging to UCSF, ' +
  'but this chat is bound to a model covered by Stanford’s agreements. This call was not made.';

const toolRequest: ToolRequestMessageContent = {
  type: 'toolRequest',
  id: 'tool-xaff',
  toolCall: {
    status: 'success',
    value: { name: 'ucsfomopagent__cohort_lookup', arguments: { cohort_id: 42 } },
  },
};

const renderTranscript = (error: string) =>
  render(
    <ToolCallWithResponse
      sessionId="chat_7"
      isCancelledMessage={false}
      toolRequest={toolRequest}
      toolResponse={{
        type: 'toolResponse',
        id: 'tool-xaff',
        toolResult: { status: 'error', error },
      }}
      onOpenArtifact={noopOpenArtifact}
    />
  );

const acceptControl = () => screen.queryByRole('button', { name: /approve this flow/i });

beforeEach(() => {
  vi.clearAllMocks();
  // No mixing-policy key on this machine, which is every machine until the
  // setting ships: `standard`.
  mockReadConfig.mockResolvedValue({ data: undefined });
  mockGrant.mockResolvedValue({ data: { accepted: 'Recorded for this chat only.' } });
  // @ts-expect-error test shim
  window.electron = { getUserActionKey: vi.fn().mockResolvedValue('ua-key-1') };
});

describe('the cross-institutional refusal in the transcript', () => {
  it('carries an accept control the user can reach without expanding anything', async () => {
    const user = userEvent.setup();
    renderTranscript(grantableRefusal('ucsfomopagent'));

    // ⚠ NO click first. A failed tool call is a collapsed line in the
    // transcript, so a control only reachable behind that disclosure is one most
    // people never find — and "there is a handler" was already true before this
    // task. The refusal and its way out arrive together.
    const button = await screen.findByRole('button', { name: /approve this flow/i });
    expect(button).toBeVisible();
    // The daemon's own words are on screen beside it, so the person is deciding
    // on the stated risk rather than on a bare button.
    expect(screen.getByText(/Cross-institutional data flow/)).toBeVisible();

    await user.click(button);

    await waitFor(() => expect(mockGrant).toHaveBeenCalledTimes(1));
    const [options] = mockGrant.mock.calls[0] as [
      { body: { session_id: string; extension: string }; headers: Record<string, string> },
    ];
    // The triple as refused: this chat, this connector. No affiliation — the
    // daemon reads that off the model it samples, and a client-supplied one
    // would record an acceptance of a flow the user was never shown.
    expect(options.body).toEqual({ session_id: 'chat_7', extension: 'ucsfomopagent' });
    // DR-16's proof, per request. Without it the daemon refuses with a 403.
    expect(options.headers['X-User-Action']).toBe('ua-key-1');

    expect(await screen.findByText('Recorded for this chat only.')).toBeInTheDocument();
  });

  it('opens onto the accept control when the refusal lands after the row was drawn', async () => {
    // A call is drawn the moment it starts, running, with nothing to open. The
    // refusal arrives later on the same row; the row must still open onto the
    // way out rather than wait for a click nobody knows to make.
    const props = {
      sessionId: 'chat_7',
      isCancelledMessage: false,
      toolRequest,
      onOpenArtifact: noopOpenArtifact,
    };
    const view = render(<ToolCallWithResponse {...props} turnActive />);
    expect(acceptControl()).toBeNull();
    view.rerender(
      <ToolCallWithResponse
        {...props}
        toolResponse={{
          type: 'toolResponse',
          id: 'tool-xaff',
          toolResult: { status: 'error', error: grantableRefusal('ucsfomopagent') },
        }}
      />
    );
    expect(await screen.findByRole('button', { name: /approve this flow/i })).toBeVisible();
  });

  it('reads the connector out of the refusal rather than off the tool name', async () => {
    // `get_client_for_tool` resolves an extension by longest key, so an
    // extension named `ucsf` and one named `ucsf__omop` both prefix
    // `ucsf__omop__lookup`. Splitting the tool name here would key the grant on
    // the wrong connector and record a row no lookup ever matches — a control
    // that silently does nothing. The refusal names the key Gate C itself used.
    const user = userEvent.setup();
    render(
      <ToolCallWithResponse
        sessionId="chat_7"
        isCancelledMessage={false}
        toolRequest={{
          type: 'toolRequest',
          id: 'tool-xaff-2',
          toolCall: { status: 'success', value: { name: 'ucsf__omop__lookup', arguments: {} } },
        }}
        toolResponse={{
          type: 'toolResponse',
          id: 'tool-xaff-2',
          toolResult: { status: 'error', error: grantableRefusal('ucsf') },
        }}
        onOpenArtifact={noopOpenArtifact}
      />
    );

    await user.click(await screen.findByRole('button', { name: /approve this flow/i }));
    await waitFor(() => expect(mockGrant).toHaveBeenCalledTimes(1));
    const [options] = mockGrant.mock.calls[0] as [{ body: { extension: string } }];
    expect(options.body.extension).toBe('ucsf');
  });

  it('offers nothing when there is no mismatch to accept', async () => {
    // The `open` mixing policy raises no mismatch at all, so what reaches the
    // transcript is an ordinary tool failure — and an accept control on one
    // would be an offer to approve a boundary nobody crossed.
    renderTranscript('The command could not be started');
    await waitFor(() => expect(screen.getByText(/^Failed/)).toBeInTheDocument());
    expect(acceptControl()).not.toBeInTheDocument();
    expect(mockGrant).not.toHaveBeenCalled();
  });

  it('offers nothing on a refusal the grant is never consulted for', async () => {
    // `assert_extension_reachable` and the agent's own enable path compose the
    // same refusal without the accept frame, because neither reads a grant.
    // Pressing a control there would record a real acceptance and leave the
    // retry refused.
    renderTranscript(bareRefusal);
    await waitFor(() => expect(screen.getByText(/^Failed/)).toBeInTheDocument());
    expect(acceptControl()).not.toBeInTheDocument();
    expect(mockGrant).not.toHaveBeenCalled();
  });

  it('does not open a saved transcript onto a control it cannot offer', async () => {
    // `sessions/SessionViewComponents.tsx` renders a finished conversation and
    // passes NO `sessionId` — deliberately, since there is no live chat to key
    // an acceptance on — so the card returns null there. The expansion must
    // therefore follow the CARD and not the refusal: forcing the disclosure open
    // for a way out that is not on the surface is the disclosure opening onto
    // nothing, and it silently drops every other saved failure's quiet default
    // for one class of refusal that gains nothing by it.
    render(
      <ToolCallWithResponse
        isCancelledMessage={false}
        toolRequest={toolRequest}
        toolResponse={{
          type: 'toolResponse',
          id: 'tool-xaff',
          toolResult: { status: 'error', error: grantableRefusal('ucsfomopagent') },
        }}
        onOpenArtifact={noopOpenArtifact}
      />
    );

    // Rendered, and collapsed. `ToolCallExpandable` renders
    // `{isExpanded && <div>{children}</div>}`, so a collapsed call has its body
    // genuinely ABSENT from the DOM rather than merely hidden — which is what
    // lets this assert the default rather than a style.
    await waitFor(() => expect(screen.getByText(/^Failed/)).toBeInTheDocument());
    expect(screen.queryByText(/Cross-institutional data flow/)).not.toBeInTheDocument();
    expect(acceptControl()).not.toBeInTheDocument();
    expect(mockGrant).not.toHaveBeenCalled();
  });

  /**
   * Issue #56, DR-27 — **the accept control across the three mixing
   * modes, reached the way a person reaches it.**
   *
   * The blocker this closes was not a missing handler: `strict`'s daemon half
   * landed with Task 52, tested down to the ordering of the password prompt
   * against the write. What was missing was that the renderer's strict branch
   * still rendered the dead end it was given before that landed — "this build
   * cannot ask for a system password, so the flow cannot be approved" — so on a
   * machine set to `strict` there was **no accept control at all**, and DR-26's
   * hard block came back for exactly the deployments careful enough to choose it.
   *
   * ⚠ So these drive the mode through the REAL read path: `mockReadConfig`
   * answers what `/config/read` actually answers for the mixing key (a bare JSON
   * string), and the assertion is on a control found by role and name in a
   * rendered transcript. Mounting the card and mocking `readMixingMode` would
   * skip both the key and the parser, which is two of the four places this can
   * silently answer `standard`.
   */
  describe('under each mixing policy', () => {
    it('offers the same control under strict, warning that the system will ask', async () => {
      const user = userEvent.setup();
      mockReadConfig.mockResolvedValue({ data: 'strict' });
      renderTranscript(grantableRefusal('ucsfomopagent'));

      // Reachable from the refusal itself, with no expansion — as in `standard`.
      const button = await screen.findByRole('button', { name: /approve this flow/i });
      expect(button).toBeVisible();
      // …and the extra cost is stated BEFORE the press. The daemon raises the
      // dialog (`strict_mode_authorization`); an unannounced one reads as
      // spurious, and dismissing it leaves the flow refused.
      expect(await screen.findByTestId('cross-affiliation-strict-notice')).toBeVisible();

      await user.click(button);

      // The same route, the same triple, the same proof-of-user. `strict` adds a
      // demand the DAEMON makes; it does not change what the renderer posts.
      await waitFor(() => expect(mockGrant).toHaveBeenCalledTimes(1));
      const [options] = mockGrant.mock.calls[0] as [
        { body: { session_id: string; extension: string }; headers: Record<string, string> },
      ];
      expect(options.body).toEqual({ session_id: 'chat_7', extension: 'ucsfomopagent' });
      expect(options.headers['X-User-Action']).toBe('ua-key-1');
    });

    it('asks for no system confirmation under standard', async () => {
      mockReadConfig.mockResolvedValue({ data: 'standard' });
      renderTranscript(grantableRefusal('ucsfomopagent'));

      await screen.findByRole('button', { name: /approve this flow/i });
      // `standard` must be exactly today's behaviour: one in-app confirmation.
      // A card that showed the strict sentence unconditionally would tell most
      // users something false about the control they are about to use.
      expect(screen.queryByTestId('cross-affiliation-strict-notice')).not.toBeInTheDocument();
    });

    it('offers nothing under open, where no mismatch is raised', async () => {
      mockReadConfig.mockResolvedValue({ data: 'open' });
      // ⚠ The refusal text is the grantable one, which on an `open` machine the
      // daemon never composes — `refusing_mismatch` goes quiet. Feeding it
      // anyway is the stronger test: the control must not appear even when the
      // text says it may, because tool-result prose is written by the extension.
      renderTranscript(grantableRefusal('ucsfomopagent'));

      await waitFor(() => expect(mockReadConfig).toHaveBeenCalled());
      // ⚠ `findBy…().rejects`, not `queryBy…()`. A control that has not been
      // drawn YET and one that never will be are the same empty DOM, so a
      // synchronous "not in the document" here passes before the policy read
      // even resolves — it would pass the pre-fix build too, and pass a build
      // that draws the button for `open`. This waits the full poll instead.
      // The 2 s bound is generous rather than tight: with the policy read
      // already settled above, a build that draws the button for `open` draws it
      // within one commit — measured at 24 ms when this was broken on purpose.
      await expect(
        screen.findByRole('button', { name: /approve this flow/i }, { timeout: 2000 })
      ).rejects.toBeTruthy();
      expect(screen.queryByTestId('cross-affiliation-strict-notice')).not.toBeInTheDocument();
      expect(mockGrant).not.toHaveBeenCalled();
    });
  });

  it('records nothing until a person presses it', async () => {
    // DR-19's asymmetry, at the surface: producing the refusal is something the
    // MODEL did — it made the tool call. Rendering one must therefore never be
    // the acceptance. Only the click is.
    renderTranscript(grantableRefusal('ucsfomopagent'));
    await screen.findByRole('button', { name: /approve this flow/i });
    // Settled: the policy was read and the control drawn, and still nothing was
    // posted.
    await waitFor(() => expect(mockReadConfig).toHaveBeenCalled());
    expect(mockGrant).not.toHaveBeenCalled();
  });
});
