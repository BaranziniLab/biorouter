import { fireEvent, render, screen, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { DecisionCard, DECISION_CARD_COPY } from './DecisionCard';
import { resetDecisionRecordCacheForTests } from './decisionChatApi';
import { wireBinaryDetail, wireChoiceDetail, wireErrorDetail } from './decisionFixtures';

const mocks = vi.hoisted(() => ({ get: vi.fn(), headers: vi.fn() }));
vi.mock('../../api/client.gen', () => ({ client: { get: mocks.get, put: vi.fn() } }));
vi.mock('../../utils/userAction', () => ({ userActionHeaders: mocks.headers }));

const ok = (body: unknown) => ({ data: body, error: undefined, response: { status: 200 } });

beforeEach(() => {
  resetDecisionRecordCacheForTests();
  mocks.get.mockReset().mockResolvedValue(ok(wireBinaryDetail()));
  mocks.headers.mockReset().mockResolvedValue({ 'X-User-Action': 'synthetic-proof' });
});

function show(decisionId = 'dec-1') {
  const onOpenArtifact = vi.fn();
  const view = render(
    <DecisionCard
      sessionId="chat-1"
      decisionId={decisionId}
      fallback={<p>Yes · P(yes)=0.93 · tev1:0.8b</p>}
      onOpenArtifact={onOpenArtifact}
    />
  );
  return { ...view, onOpenArtifact };
}

describe('the decision card', () => {
  it('reads its one row with the user-action proof', async () => {
    show();
    await screen.findByTestId('decision-card');
    expect(mocks.get).toHaveBeenCalledTimes(1);
    expect(mocks.get).toHaveBeenCalledWith(
      expect.objectContaining({
        url: '/sessions/{session_id}/decisions/{decision_id}',
        path: { session_id: 'chat-1', decision_id: 'dec-1' },
        headers: expect.objectContaining({ 'X-User-Action': 'synthetic-proof' }),
      })
    );
  });

  it('draws the reply’s own text while the row loads', () => {
    mocks.get.mockReturnValue(new Promise(() => {}));
    show();
    expect(screen.getByTestId('decision-card-fallback')).toHaveTextContent('P(yes)=0.93');
    expect(screen.queryByTestId('decision-card')).toBeNull();
  });

  it('shows the question, the answer, one bar per outcome, the threshold and the verdict', async () => {
    show();
    const card = await screen.findByTestId('decision-card');
    expect(card).toHaveAccessibleName(
      'Decision: Does this note state the patient has multiple sclerosis?'
    );
    expect(within(card).getByTestId('decision-answer')).toHaveTextContent('Yes');
    expect(card).toHaveTextContent('P 0.93');
    const bars = within(card).getAllByTestId('decision-bar');
    expect(bars.map((bar) => bar.dataset.key)).toEqual(['yes', 'no']);
    expect(bars[0]).toHaveAttribute('data-answer');
    expect(bars[1]).not.toHaveAttribute('data-answer');
    expect(within(bars[0]).getByRole('progressbar', { name: 'Yes 0.93' })).toHaveAttribute(
      'aria-valuenow',
      '93'
    );
    // The threshold is drawn on the bar it is compared against, at its probability.
    const mark = within(bars[0]).getByTestId('decision-threshold-mark');
    expect(mark.style.left).toBe('80%');
    expect(within(bars[1]).queryByTestId('decision-threshold-mark')).toBeNull();
    expect(within(card).getByTestId('decision-threshold')).toHaveTextContent('P(yes) ≥ 0.80');
    expect(within(card).getByTestId('decision-verdict')).toHaveTextContent(
      DECISION_CARD_COPY.passed
    );
    // A native distribution carries no uncalibrated note.
    expect(within(card).queryByTestId('decision-calibration')).toBeNull();
  });

  it('names the model, its tier and the latency', async () => {
    show();
    const meta = await screen.findByTestId('decision-meta');
    expect(meta).toHaveTextContent('tev1:0.8b');
    expect(meta).toHaveTextContent(DECISION_CARD_COPY.privateModel);
    expect(meta).toHaveTextContent('22 ms');
  });

  /** One glyph per concept: the model line carries the decision mark every decision surface draws. */
  it('marks the model with the decision glyph', async () => {
    show();
    const meta = await screen.findByTestId('decision-meta');
    expect(within(meta).getByTestId('decision-card-mark')).toHaveClass('lucide-gauge');
  });

  it('says in words when a source is uncalibrated, and marks a failed threshold', async () => {
    mocks.get.mockResolvedValue(ok(wireChoiceDetail()));
    show('dec-2');
    const card = await screen.findByTestId('decision-card');
    expect(within(card).getByTestId('decision-answer')).toHaveTextContent('spms');
    expect(
      within(card)
        .getAllByTestId('decision-bar')
        .map((bar) => bar.dataset.key)
    ).toEqual(['rrms', 'spms', 'ppms']);
    expect(within(card).getByTestId('decision-calibration')).toHaveTextContent(/Uncalibrated/);
    expect(within(card).getByTestId('decision-verdict')).toHaveTextContent(
      DECISION_CARD_COPY.failed
    );
    expect(within(card).getByTestId('decision-verdict')).toHaveAttribute('data-passed', 'false');
    expect(within(card).getByTestId('decision-meta')).toHaveTextContent(
      DECISION_CARD_COPY.publicModel
    );
    expect(within(card).getByTestId('decision-meta')).toHaveTextContent('1.4 s');
  });

  it('names a Baseten model by its alias, not its slug or the id Baseten served it under', async () => {
    const detail = wireChoiceDetail();
    mocks.get.mockResolvedValue(
      ok({ ...detail, record: { ...detail.record, servedModel: 'joey-2b-0.3' } })
    );
    show('dec-2');
    const meta = await screen.findByTestId('decision-meta');
    expect(meta).toHaveTextContent('joey-1');
    expect(meta).not.toHaveTextContent('baseten-joey-2b-latest');
    expect(meta).not.toHaveTextContent('joey-2b-0.3');
  });

  it('says why a failed call has no answer', async () => {
    mocks.get.mockResolvedValue(ok(wireErrorDetail()));
    show('dec-3');
    expect(await screen.findByTestId('decision-error')).toHaveTextContent(
      'Not decided: The decision model did not answer in 10 s'
    );
  });

  it('keeps the reply’s text when the row is gone or cannot be read', async () => {
    mocks.get.mockResolvedValue({ data: undefined, error: 'gone', response: { status: 404 } });
    show();
    await vi.waitFor(() =>
      expect(screen.getByTestId('decision-card-fallback')).toHaveAttribute('data-state', 'missing')
    );
    expect(screen.getByText('Yes · P(yes)=0.93 · tev1:0.8b')).toBeInTheDocument();
  });

  it('reads a row once per window', async () => {
    const first = show();
    await screen.findByTestId('decision-card');
    first.unmount();
    show();
    // Drawn at once from the cache, with no second read.
    expect(screen.getByTestId('decision-card')).toBeInTheDocument();
    expect(mocks.get).toHaveBeenCalledTimes(1);
  });

  it('opens the decision history on this one call, in its turn of the decision chat', async () => {
    const { onOpenArtifact } = show();
    await screen.findByTestId('decision-card');
    fireEvent.click(screen.getByRole('button', { name: DECISION_CARD_COPY.historyLabel }));
    expect(onOpenArtifact).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: 'decisions',
        sessionId: 'chat-1',
        filter: { decisionId: 'dec-1', origin: 'chat', turnId: 'turn-7' },
      })
    );
  });
});
