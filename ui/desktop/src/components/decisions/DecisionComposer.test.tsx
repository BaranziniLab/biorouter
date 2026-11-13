import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  DecisionComposer,
  DECISION_COMPOSER_COPY,
  DECISION_COMPOSER_SAVE_DELAY_MS,
  questionFromDraft,
  emptyDraft,
  settleDecisionQuestion,
} from './DecisionComposer';

const mocks = vi.hoisted(() => ({ get: vi.fn(), put: vi.fn(), headers: vi.fn() }));
vi.mock('../../api/client.gen', () => ({ client: { put: mocks.put, get: mocks.get } }));
vi.mock('../../utils/userAction', () => ({ userActionHeaders: mocks.headers }));

/** `GET /sessions/{id}/decision-chat`'s answer: `{ settings: { question } | null }`. */
const stored = (question: Record<string, unknown> | null = null) => ({
  data: { settings: question ? { question } : null },
  error: undefined,
  response: { status: 200 },
});

beforeEach(() => {
  mocks.get.mockReset().mockResolvedValue(stored());
  mocks.put
    .mockReset()
    .mockResolvedValue({ data: {}, error: undefined, response: { status: 200 } });
  mocks.headers.mockReset().mockResolvedValue({ 'X-User-Action': 'synthetic-proof' });
});

afterEach(() => {
  vi.useRealTimers();
});

async function ready() {
  await waitFor(() => expect(screen.getByTestId('decision-composer-question')).toBeEnabled());
}

/** The kind, the options and the threshold sit behind "Question options". */
function openOptions() {
  const trigger = screen.getByRole('button', { name: DECISION_COMPOSER_COPY.options });
  if (trigger.getAttribute('aria-expanded') !== 'true') fireEvent.click(trigger);
}

describe('the decision composer', () => {
  it('reads the chat’s stored question, with the user-action proof', async () => {
    mocks.get.mockResolvedValue(
      stored({
        instructions: 'Is an EDSS score stated?',
        kind: { choice: { options: { yes: null, no: null, unclear: null } } },
        threshold: { option: { name: 'yes', p: 0.7 } },
      })
    );
    render(<DecisionComposer sessionId="chat-1" />);
    await ready();
    expect(mocks.get).toHaveBeenCalledWith(
      expect.objectContaining({
        url: '/sessions/{session_id}/decision-chat',
        path: { session_id: 'chat-1' },
        headers: expect.objectContaining({ 'X-User-Action': 'synthetic-proof' }),
      })
    );
    expect(screen.getByTestId('decision-composer-question')).toHaveValue(
      'Is an EDSS score stated?'
    );
    // Closed, the summary says the kind and the rule in words.
    expect(
      screen.getByRole('button', { name: DECISION_COMPOSER_COPY.options })
    ).toHaveAccessibleDescription('Choice of 3 · when P(yes) is at or above 0.70');
    openOptions();
    expect(screen.getByRole('radio', { name: DECISION_COMPOSER_COPY.kinds.choice })).toBeChecked();
    expect(screen.getByLabelText('Option 3')).toHaveValue('unclear');
    expect(screen.getByTestId('decision-composer-threshold')).toHaveValue(0.7);
    // A question read back is not saved again.
    expect(mocks.put).not.toHaveBeenCalled();
    expect(screen.getByTestId('decision-composer-status')).toHaveTextContent(
      DECISION_COMPOSER_COPY.saved
    );
  });

  it('saves a typed question once typing pauses, with the proof', async () => {
    render(<DecisionComposer sessionId="chat-1" />);
    await ready();
    openOptions();
    vi.useFakeTimers();
    fireEvent.change(screen.getByTestId('decision-composer-question'), {
      target: { value: 'Does this note state MS?' },
    });
    fireEvent.change(screen.getByTestId('decision-composer-threshold'), {
      target: { value: '0.8' },
    });
    expect(mocks.put).not.toHaveBeenCalled();
    await act(() => vi.advanceTimersByTimeAsync(DECISION_COMPOSER_SAVE_DELAY_MS + 10));
    expect(mocks.put).toHaveBeenCalledTimes(1);
    expect(mocks.put).toHaveBeenCalledWith(
      expect.objectContaining({
        url: '/sessions/{session_id}/decision-chat',
        path: { session_id: 'chat-1' },
        body: {
          question: {
            instructions: 'Does this note state MS?',
            kind: { binary: {} },
            threshold: { yes: 0.8 },
          },
        },
        headers: expect.objectContaining({ 'X-User-Action': 'synthetic-proof' }),
      })
    );
  });

  it('saves at once when a field loses focus, so the question is stored before Send', async () => {
    render(<DecisionComposer sessionId="chat-1" />);
    await ready();
    const field = screen.getByTestId('decision-composer-question');
    fireEvent.change(field, { target: { value: 'Is the form complete?' } });
    fireEvent.blur(field);
    await waitFor(() => expect(mocks.put).toHaveBeenCalledTimes(1));
    await waitFor(() =>
      expect(screen.getByTestId('decision-composer-status')).toHaveTextContent(
        DECISION_COMPOSER_COPY.saved
      )
    );
  });

  it('says what an incomplete question needs, and saves nothing', async () => {
    render(<DecisionComposer sessionId="chat-1" />);
    await ready();
    openOptions();
    fireEvent.click(screen.getByRole('radio', { name: DECISION_COMPOSER_COPY.kinds.choice }));
    const field = screen.getByTestId('decision-composer-question');
    fireEvent.change(field, { target: { value: 'Which course?' } });
    fireEvent.change(screen.getByLabelText('Option 1'), { target: { value: 'rrms' } });
    fireEvent.blur(field);
    expect(screen.getByTestId('decision-composer-status')).toHaveTextContent(
      DECISION_COMPOSER_COPY.needsOptions
    );
    expect(mocks.put).not.toHaveBeenCalled();
  });

  it('shows the daemon’s refusal in place', async () => {
    mocks.put.mockResolvedValue({
      data: undefined,
      // The route answers a refusal in plain text.
      error: 'Only the person at the keyboard may set this.',
      response: { status: 403 },
    });
    render(<DecisionComposer sessionId="chat-1" />);
    await ready();
    const field = screen.getByTestId('decision-composer-question');
    fireEvent.change(field, { target: { value: 'Is it done?' } });
    fireEvent.blur(field);
    await waitFor(() =>
      expect(screen.getByTestId('decision-composer-status')).toHaveTextContent(
        'Only the person at the keyboard may set this.'
      )
    );
    // On its own line, in the primary ink with the failure glyph (red is rare,
    // design.md 9.13), and still the one status node.
    const status = screen.getByTestId('decision-composer-status');
    expect(status).toHaveAttribute('data-wide', 'true');
    expect(status).toHaveAttribute('data-tone', 'problem');
    expect(within(status).getByTestId('decision-composer-failed')).toBeInTheDocument();
    expect(screen.getAllByRole('status')).toHaveLength(1);
    expect(field).toHaveAccessibleDescription('Only the person at the keyboard may set this.');
  });

  it('names the prefix the daemon reads a question line by, with an example it accepts', () => {
    // `parse_question_line` reads only a first line that starts with `Q:`, `q:` or `?`;
    // a bare question there gets the how-to reply and decides nothing.
    const chat = readFileSync(
      join(
        process.cwd(),
        '..',
        '..',
        'crates',
        'biorouter',
        'src',
        'providers',
        'decision',
        'chat.rs'
      ),
      'utf8'
    );
    expect(chat).toContain('.strip_prefix("Q:")');
    const example = DECISION_COMPOSER_COPY.draftNote.match(/“(.+?)”/)?.[1];
    expect(example).toMatch(/^Q:\s*\S/);
    expect(DECISION_COMPOSER_COPY.refused(null)).toContain('Q:');
  });

  it('explains the question line where there is no chat yet', () => {
    render(<DecisionComposer sessionId={null} />);
    expect(screen.getByTestId('decision-composer')).toHaveTextContent(
      DECISION_COMPOSER_COPY.draftNote
    );
    expect(mocks.get).not.toHaveBeenCalled();
  });
});

describe('a send waits for the question (R1-6)', () => {
  it('saves a question typed moments before Send, and resolves only once it is stored', async () => {
    let answer!: (value: unknown) => void;
    mocks.put.mockImplementation(() => new Promise((done) => (answer = done)));
    render(<DecisionComposer sessionId="chat-1" />);
    await ready();
    // Typed, and sent before the half-second save wait ran out: nothing stored yet.
    fireEvent.change(screen.getByTestId('decision-composer-question'), {
      target: { value: 'Does this note mention a fever?' },
    });
    expect(mocks.put).not.toHaveBeenCalled();
    let settled = false;
    let send!: Promise<unknown>;
    act(() => {
      send = settleDecisionQuestion('chat-1').then(() => (settled = true));
    });
    await waitFor(() => expect(mocks.put).toHaveBeenCalledTimes(1));
    expect(mocks.put.mock.calls[0][0].body.question.instructions).toBe(
      'Does this note mention a fever?'
    );
    // The PUT is still in flight: the send still waits.
    await act(() => Promise.resolve());
    expect(settled).toBe(false);
    // A second settle (a blur, then Send) joins the same save, never a second PUT.
    let again!: Promise<void>;
    act(() => {
      again = settleDecisionQuestion('chat-1');
    });
    answer({ data: {}, error: undefined, response: { status: 200 } });
    await act(() => send);
    await act(() => again);
    expect(settled).toBe(true);
    expect(mocks.put).toHaveBeenCalledTimes(1);
  });

  it('resolves at once for a chat with no composer, or nothing new to save', async () => {
    await expect(settleDecisionQuestion('no-such-chat')).resolves.toBeUndefined();
    await expect(settleDecisionQuestion(null)).resolves.toBeUndefined();
    render(<DecisionComposer sessionId="chat-1" />);
    await ready();
    await act(() => settleDecisionQuestion('chat-1'));
    expect(mocks.put).not.toHaveBeenCalled();
  });
});

describe('what the composer shows up front (F-8, B-5)', () => {
  it('shows the question line and Question options, closed, and nothing else', async () => {
    render(<DecisionComposer sessionId="chat-1" />);
    await ready();
    const trigger = screen.getByRole('button', { name: DECISION_COMPOSER_COPY.options });
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
    expect(trigger).toHaveAccessibleDescription('Yes or no');
    expect(screen.queryByTestId('decision-composer-kind')).toBeNull();
    expect(screen.queryByTestId('decision-composer-threshold')).toBeNull();
    // An empty question says nothing: its placeholder says what to write.
    expect(screen.getByTestId('decision-composer-status')).toHaveTextContent(/^$/);
  });

  it('gives the question a real example by kind, never one that reads as a kept value', async () => {
    render(<DecisionComposer sessionId="chat-1" />);
    await ready();
    const field = screen.getByTestId('decision-composer-question');
    expect(field).toHaveAttribute('placeholder', 'For example: Does this note mention a fever?');
    openOptions();
    fireEvent.click(screen.getByRole('radio', { name: DECISION_COMPOSER_COPY.kinds.choice }));
    expect(field).toHaveAttribute(
      'placeholder',
      'For example: Which MS course does this note describe?'
    );
    expect(screen.getByLabelText('Option 1')).toHaveAttribute('placeholder', 'Option 1');
    expect(screen.getByLabelText('Option 2')).toHaveAttribute('placeholder', 'Option 2');
    fireEvent.click(screen.getByRole('radio', { name: DECISION_COMPOSER_COPY.kinds.score }));
    expect(field).toHaveAttribute(
      'placeholder',
      'For example: How severe is the pain this note describes?'
    );
    expect(screen.getByLabelText('Level 3')).toHaveAttribute('placeholder', 'Level 3');
  });

  it('says Saved on the question line, and the question is described by it', async () => {
    mocks.get.mockResolvedValue(
      stored({ instructions: 'Does this note mention a fever?', kind: { binary: {} } })
    );
    render(<DecisionComposer sessionId="chat-1" />);
    await ready();
    const status = screen.getByTestId('decision-composer-status');
    expect(status).toHaveTextContent(/^Saved$/);
    expect(status).not.toHaveAttribute('data-wide');
    expect(status.querySelector('svg')).not.toBeNull();
    expect(screen.getByTestId('decision-composer-question')).toHaveAccessibleDescription('Saved');
    expect(status.parentElement).toBe(
      screen.getByTestId('decision-composer-question').parentElement
    );
  });

  it('says the rule in words under a threshold, in the card’s words', async () => {
    render(<DecisionComposer sessionId="chat-1" />);
    await ready();
    fireEvent.change(screen.getByTestId('decision-composer-question'), {
      target: { value: 'Does this note mention a fever?' },
    });
    openOptions();
    fireEvent.change(screen.getByTestId('decision-composer-threshold'), {
      target: { value: '0.85' },
    });
    expect(screen.getByTestId('decision-composer-meets')).toHaveTextContent(
      'Meets the threshold when P(yes) is at or above 0.85.'
    );
    expect(screen.getByRole('button', { name: 'About Threshold' })).toHaveAccessibleDescription(
      DECISION_COMPOSER_COPY.thresholdInfo
    );
    expect(screen.getByTestId('decision-composer').textContent).not.toMatch(/[≥<]/);
  });

  it('marks a threshold out of range in the primary ink, on its own line', async () => {
    render(<DecisionComposer sessionId="chat-1" />);
    await ready();
    fireEvent.change(screen.getByTestId('decision-composer-question'), {
      target: { value: 'Is it signed?' },
    });
    openOptions();
    const threshold = screen.getByTestId('decision-composer-threshold');
    fireEvent.change(threshold, { target: { value: '1.5' } });
    const status = screen.getByTestId('decision-composer-status');
    expect(status).toHaveTextContent('The threshold is a number from 0 to 1.');
    expect(status).toHaveAttribute('data-tone', 'problem');
    expect(status).toHaveAttribute('data-wide', 'true');
    // An invalid field, not a failure: the field keeps its one red edge, so no glyph here.
    expect(within(status).queryByTestId('decision-composer-failed')).toBeNull();
    expect(threshold).toHaveAttribute('aria-invalid', 'true');
  });

  it('keeps the kept rule inside Question options', async () => {
    mocks.get.mockResolvedValue(
      stored({
        instructions: 'Has the form been signed?',
        kind: { binary: {} },
        threshold: { below: { yes: 0.3 } },
      })
    );
    render(<DecisionComposer sessionId="chat-1" />);
    await ready();
    expect(screen.queryByTestId('decision-composer-kept-rule')).toBeNull();
    openOptions();
    expect(screen.getByTestId('decision-composer-kept-rule')).toBeInTheDocument();
  });

  it('authors its geometry: an unfilled question, the card’s focus edge, hugging tracks', () => {
    const css = readFileSync(join(__dirname, 'decisionsChat.css'), 'utf8');
    expect(css).toMatch(
      /\.biorouter-composer-card:has\(\.br-decision-composer input:focus\) \{\s*border-color: var\(--border-accent\);/
    );
    expect(css).toMatch(/\.br-decision-composer \.br-segmented \{\s*align-self: flex-start;/);
    const question = css.match(/\n\.br-decision-composer__question \{([^}]*)\}/)?.[1];
    expect(question).toMatch(/background: none;/);
    expect(question).toMatch(/border: 1px solid transparent;/);
  });
});

/**
 * A threshold set over HTTP or the CLI can pass below its value, or sit on P(no):
 * neither has a field here. Saving the question after an edit must not turn
 * P(yes) < 0.30 into P(yes) ≥ 0.30, or P(no) into P(yes).
 */
describe('a stored rule the fields do not edit', () => {
  const putQuestion = () =>
    (mocks.put.mock.calls[0][0] as { body: { question: Record<string, unknown> } }).body.question;

  async function editAndSave() {
    await ready();
    const field = screen.getByTestId('decision-composer-question');
    fireEvent.change(field, { target: { value: 'Has the form been signed today?' } });
    fireEvent.blur(field);
    await waitFor(() => expect(mocks.put).toHaveBeenCalledTimes(1));
  }

  it('keeps a below threshold through an edit, and says it', async () => {
    mocks.get.mockResolvedValue(
      stored({
        instructions: 'Has the form been signed?',
        kind: { binary: {} },
        threshold: { below: { yes: 0.3 } },
      })
    );
    render(<DecisionComposer sessionId="chat-1" />);
    await ready();
    // Read back as it is stored: nothing to save, and the rule is said.
    expect(screen.getByTestId('decision-composer-status')).toHaveTextContent(
      DECISION_COMPOSER_COPY.saved
    );
    openOptions();
    expect(screen.getByTestId('decision-composer-kept-rule')).toHaveTextContent(
      DECISION_COMPOSER_COPY.keptRule('when P(yes) is below 0.30')
    );
    expect(screen.getByTestId('decision-composer-kept-rule')).toHaveTextContent(
      'Meets the threshold when P(yes) is below 0.30, as set outside this chat.'
    );
    await editAndSave();
    expect(putQuestion().threshold).toEqual({ below: { yes: 0.3 } });
  });

  it('keeps a yes/no threshold on P(no) through an edit', async () => {
    mocks.get.mockResolvedValue(
      stored({
        instructions: 'Has the form been signed?',
        kind: { binary: {} },
        threshold: { no: 0.4 },
      })
    );
    render(<DecisionComposer sessionId="chat-1" />);
    await ready();
    openOptions();
    expect(screen.getByTestId('decision-composer-kept-rule')).toHaveTextContent(
      'when P(no) is at or above 0.40'
    );
    await editAndSave();
    expect(putQuestion().threshold).toEqual({ no: 0.4 });
  });

  it('keeps a below threshold on a choice option', async () => {
    mocks.get.mockResolvedValue(
      stored({
        instructions: 'Which course?',
        kind: { choice: { options: { rrms: null, spms: null } } },
        threshold: { below: { option: { name: 'spms', p: 0.2 } } },
      })
    );
    render(<DecisionComposer sessionId="chat-1" />);
    await editAndSave();
    expect(putQuestion().threshold).toEqual({ below: { option: { name: 'spms', p: 0.2 } } });
  });

  it('drops the stored rule when the threshold is cleared, so a new one is P(yes) at or above', async () => {
    mocks.get.mockResolvedValue(
      stored({
        instructions: 'Has the form been signed?',
        kind: { binary: {} },
        threshold: { below: { no: 0.3 } },
      })
    );
    render(<DecisionComposer sessionId="chat-1" />);
    await ready();
    openOptions();
    const threshold = screen.getByTestId('decision-composer-threshold');
    fireEvent.change(threshold, { target: { value: '' } });
    fireEvent.change(threshold, { target: { value: '0.8' } });
    expect(screen.queryByTestId('decision-composer-kept-rule')).toBeNull();
    fireEvent.blur(threshold);
    await waitFor(() => expect(mocks.put).toHaveBeenCalledTimes(1));
    expect(putQuestion().threshold).toEqual({ yes: 0.8 });
  });
});

describe('a draft as the question the route stores', () => {
  it('builds a score with an at-least-level threshold, lowest level first', () => {
    const draft = emptyDraft();
    draft.instructions = 'How severe?';
    draft.kind = 'score';
    draft.levels = [
      { id: 1, text: 'Mild' },
      { id: 2, text: 'Moderate' },
      { id: 3, text: 'Severe' },
    ];
    draft.threshold = '0.6';
    draft.target = '1';
    expect(questionFromDraft(draft)).toEqual({
      ok: true,
      question: {
        instructions: 'How severe?',
        kind: { type: 'score', levels: ['Mild', 'Moderate', 'Severe'] },
        threshold: { type: 'at_least_level', level: 1, p: 0.6 },
      },
    });
  });

  it('refuses a threshold outside 0 to 1, and duplicate options', () => {
    const draft = emptyDraft();
    draft.instructions = 'q';
    draft.threshold = '1.5';
    expect(questionFromDraft(draft)).toEqual({
      ok: false,
      problem: DECISION_COMPOSER_COPY.badThreshold,
    });
    draft.threshold = '';
    draft.kind = 'choice';
    draft.options = [
      { id: 1, text: 'A' },
      { id: 2, text: 'a' },
    ];
    expect(questionFromDraft(draft)).toEqual({
      ok: false,
      problem: DECISION_COMPOSER_COPY.needsOptions,
    });
  });
});
