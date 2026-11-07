import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  DecisionComposer,
  DECISION_COMPOSER_COPY,
  DECISION_COMPOSER_SAVE_DELAY_MS,
  questionFromDraft,
  emptyDraft,
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
    expect(screen.getByTestId('decision-composer-kept-rule')).toHaveTextContent(
      DECISION_COMPOSER_COPY.keptRule('P(yes) < 0.30')
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
    expect(screen.getByTestId('decision-composer-kept-rule')).toHaveTextContent('P(no) ≥ 0.40');
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
