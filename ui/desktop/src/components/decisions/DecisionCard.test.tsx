import { fireEvent, render, screen, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { DecisionCard, DECISION_CARD_COPY, calibrationHelp } from './DecisionCard';
import { resetDecisionRecordCacheForTests } from './decisionChatApi';
import { wireBinaryDetail, wireChoiceDetail, wireErrorDetail } from './decisionFixtures';
import { renderToStaticMarkup } from 'react-dom/server';
import { AlertCircle } from '../icons/app-icons';

/** The failure glyph's drawing, to tell it from any other icon by shape. */
function failureGlyph(): string {
  const host = document.createElement('div');
  host.innerHTML = renderToStaticMarkup(<AlertCircle />);
  return host.querySelector('svg')?.innerHTML ?? '';
}

const mocks = vi.hoisted(() => ({ get: vi.fn(), headers: vi.fn() }));
vi.mock('../../api/client.gen', () => ({ client: { get: mocks.get, put: vi.fn() } }));
vi.mock('../../utils/userAction', () => ({ userActionHeaders: mocks.headers }));

const ok = (body: unknown) => ({ data: body, error: undefined, response: { status: 200 } });

beforeEach(() => {
  resetDecisionRecordCacheForTests();
  mocks.get.mockReset().mockResolvedValue(ok(wireBinaryDetail()));
  mocks.headers.mockReset().mockResolvedValue({ 'X-User-Action': 'synthetic-proof' });
});

/** The bars, threshold, model and history link sit behind Details. */
function openDetails(card: HTMLElement) {
  fireEvent.click(within(card).getByRole('button', { name: DECISION_CARD_COPY.details }));
}

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

  it('shows the question and the answer once, with that answer’s P and where it stands', async () => {
    show();
    const card = await screen.findByTestId('decision-card');
    expect(card).toHaveAccessibleName(
      'Decision: Does this note state the patient has multiple sclerosis?'
    );
    expect(within(card).getAllByTestId('decision-answer')).toHaveLength(1);
    expect(within(card).getByTestId('decision-answer')).toHaveTextContent('Yes');
    expect(card).toHaveTextContent('P 0.93');
    const verdict = within(card).getByTestId('decision-verdict');
    expect(verdict).toHaveTextContent(/^Meets the threshold$/);
    expect(verdict.querySelector('svg')).toHaveClass('lucide-check');
    // A native distribution carries no Uncalibrated badge.
    expect(within(card).queryByTestId('decision-calibration')).toBeNull();
    // Everything else is behind Details, closed.
    expect(within(card).queryByTestId('decision-bar')).toBeNull();
    expect(within(card).queryByTestId('decision-threshold')).toBeNull();
    expect(within(card).queryByTestId('decision-meta')).toBeNull();
    expect(within(card).getByRole('button', { name: 'Details' })).toHaveAttribute(
      'aria-expanded',
      'false'
    );
  });

  it('puts one bar per outcome and the threshold, in its record form, inside Details', async () => {
    show();
    const card = await screen.findByTestId('decision-card');
    openDetails(card);
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
    expect(within(card).getByTestId('decision-threshold')).toHaveTextContent(
      'Threshold: P(yes) ≥ 0.80'
    );
  });

  it('names the model with where it runs, privacy as a lock, and the latency, in Details', async () => {
    show();
    const card = await screen.findByTestId('decision-card');
    openDetails(card);
    const meta = within(card).getByTestId('decision-meta');
    expect(meta).toHaveTextContent('tev1:0.8b · this computer');
    expect(meta).toHaveTextContent('22 ms');
    // The words are for a screen reader only: the lock says it.
    const hidden = within(meta).getByText(DECISION_CARD_COPY.privateModel);
    expect(hidden).toHaveClass('sr-only');
    expect(hidden.previousElementSibling).toHaveClass('lucide-lock');
    expect(meta).not.toHaveTextContent('Public model');
  });

  /** One glyph per concept: the model line carries the decision mark every decision surface draws. */
  it('marks the model with the decision glyph', async () => {
    show();
    const card = await screen.findByTestId('decision-card');
    openDetails(card);
    expect(within(card).getByTestId('decision-card-mark')).toHaveClass('lucide-gauge');
  });

  it('says an uncalibrated source with a badge whose InfoTip says what it means, once', async () => {
    mocks.get.mockResolvedValue(ok(wireChoiceDetail()));
    show('dec-2');
    const card = await screen.findByTestId('decision-card');
    expect(within(card).getByTestId('decision-answer')).toHaveTextContent('spms');
    const badge = within(card).getByTestId('decision-calibration');
    expect(badge).toHaveTextContent(/^Uncalibrated$/);
    const help = 'The model wrote this number itself, so read it as a rough guide.';
    expect(badge).toHaveAccessibleDescription(help);
    expect(within(card).getAllByText(help)).toHaveLength(1);
    // A verbalized source with no reason has no "Why" line.
    openDetails(card);
    expect(within(card).queryByTestId('decision-why')).toBeNull();
    expect(
      within(card)
        .getAllByTestId('decision-bar')
        .map((bar) => bar.dataset.key)
    ).toEqual(['rrms', 'spms', 'ppms']);
    const meta = within(card).getByTestId('decision-meta');
    expect(meta).toHaveTextContent('joey-1 · Baseten');
    expect(meta).toHaveTextContent('1.4 s');
    expect(meta).not.toHaveTextContent(/Private model|Public model/);
  });

  it('says an answer that does not meet its threshold without a cross or a verdict colour', async () => {
    mocks.get.mockResolvedValue(ok(wireChoiceDetail()));
    show('dec-2');
    const card = await screen.findByTestId('decision-card');
    const verdict = within(card).getByTestId('decision-verdict');
    expect(verdict).toHaveTextContent(/^Doesn't meet the threshold$/);
    expect(verdict).toHaveAttribute('data-meets', 'false');
    expect(verdict.querySelector('svg')).toHaveClass('lucide-circle');
    expect(verdict.querySelector('.lucide-x')).toBeNull();
    expect(verdict.className).not.toMatch(/danger|success/);
    expect(card).not.toHaveTextContent(/Failed|Passed/);
  });

  it('shows a verbalized reason as “Why” in Details only', async () => {
    const detail = wireChoiceDetail();
    mocks.get.mockResolvedValue(
      ok({
        ...detail,
        record: {
          ...detail.record,
          decisions: [
            {
              ...detail.record.decisions[0],
              source: { kind: 'verbalized', reason: 'the model could not return token scores' },
            },
          ],
        },
      })
    );
    show('dec-2');
    const card = await screen.findByTestId('decision-card');
    expect(card).not.toHaveTextContent('Why:');
    openDetails(card);
    expect(within(card).getByTestId('decision-why')).toHaveTextContent(
      'Why: the model could not return token scores'
    );
  });

  it('words the other calibration sources', () => {
    expect(calibrationHelp({ kind: 'logprob', mass: 0.62 })).toBe(
      'Only 0.62 of the probability fell on the options, so read it as a rough guide.'
    );
    expect(calibrationHelp({ kind: 'native' })).toBe(
      'This number has not been calibrated on your data, so read it as a rough guide.'
    );
  });

  /**
   * A `below` rule is met when P is under its number, so the standing names no
   * direction: "Below threshold" would contradict the threshold line.
   */
  it('words an unmet below rule without contradicting its threshold line', async () => {
    const detail = wireBinaryDetail();
    mocks.get.mockResolvedValue(
      ok({
        ...detail,
        record: {
          ...detail.record,
          questions: {
            has_ms: {
              ...detail.record.questions.has_ms,
              threshold: { below: { yes: 0.3 } },
            },
          },
          decisions: [
            {
              ...detail.record.decisions[0],
              probabilities: { yes: 0.9, no: 0.1 },
              passed: false,
              passProbability: 0.9,
            },
          ],
          outcome: 'failed',
        },
      })
    );
    show();
    const card = await screen.findByTestId('decision-card');
    const verdict = within(card).getByTestId('decision-verdict');
    expect(verdict).toHaveAttribute('data-meets', 'false');
    expect(verdict).toHaveTextContent(DECISION_CARD_COPY.doesNotMeet);
    openDetails(card);
    expect(within(card).getByTestId('decision-threshold')).toHaveTextContent(
      'Threshold: P(yes) < 0.30'
    );
    expect(card).not.toHaveTextContent(/below threshold/i);
  });

  it('names a Baseten model by its alias, not its slug or the id Baseten served it under', async () => {
    const detail = wireChoiceDetail();
    mocks.get.mockResolvedValue(
      ok({ ...detail, record: { ...detail.record, servedModel: 'joey-2b-0.3' } })
    );
    show('dec-2');
    const card = await screen.findByTestId('decision-card');
    openDetails(card);
    const meta = within(card).getByTestId('decision-meta');
    expect(meta).toHaveTextContent('joey-1');
    expect(meta).not.toHaveTextContent('baseten-joey-2b-latest');
    expect(meta).not.toHaveTextContent('joey-2b-0.3');
  });

  it('gives a record with two decisions one Details with a labelled group per question, in order', async () => {
    const detail = wireBinaryDetail();
    const fever = detail.record.decisions[0];
    mocks.get.mockResolvedValue(
      ok({
        ...detail,
        record: {
          ...detail.record,
          questions: {
            fever: {
              instructions: 'Does this note mention a fever?',
              kind: { binary: {} },
              threshold: { yes: 0.85 },
            },
            rash: { instructions: 'Does this note mention a rash?', kind: { binary: {} } },
          },
          decisions: [
            { ...fever, question: 'fever', probabilities: { yes: 0.998, no: 0.002 } },
            {
              ...fever,
              question: 'rash',
              answer: { type: 'binary', value: false },
              probabilities: { yes: 0.12, no: 0.88 },
              passed: null,
              passProbability: null,
            },
          ],
        },
      })
    );
    show();
    const card = await screen.findByTestId('decision-card');
    expect(
      within(card)
        .getAllByTestId('decision-answer')
        .map((node) => node.textContent)
    ).toEqual(['Yes', 'No']);
    expect(within(card).getAllByRole('button', { name: 'Details' })).toHaveLength(1);
    openDetails(card);
    const groups = within(card).getAllByTestId('decision-group');
    expect(groups.map((group) => group.getAttribute('aria-label'))).toEqual([
      'Does this note mention a fever?',
      'Does this note mention a rash?',
    ]);
    expect(groups[0]).toHaveTextContent('Yes');
    expect(groups[0]).toHaveTextContent('P >0.99');
    expect(within(groups[0]).getAllByTestId('decision-bar')).toHaveLength(2);
    expect(within(groups[0]).getByTestId('decision-threshold')).toHaveTextContent('P(yes) ≥ 0.85');
    expect(groups[1]).toHaveTextContent('P 0.88');
    expect(within(groups[1]).queryByTestId('decision-threshold')).toBeNull();
    // One meta row, shared by both: one model answered them all.
    expect(within(card).getAllByTestId('decision-meta')).toHaveLength(1);
    expect(within(groups[1]).queryByTestId('decision-meta')).toBeNull();
  });

  it('says why a failed call has no answer', async () => {
    mocks.get.mockResolvedValue(ok(wireErrorDetail()));
    show('dec-3');
    const error = await screen.findByTestId('decision-error');
    expect(error).toHaveTextContent('Not decided: The decision model did not answer in 10 s');
    // Red is rare (design.md 9.14): the failure glyph and the words, nothing in a status hue.
    expect(within(error).getByTestId('decision-error-glyph').innerHTML).toBe(failureGlyph());
    expect(error.outerHTML).not.toMatch(/danger|warning/);
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
    openDetails(await screen.findByTestId('decision-card'));
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
