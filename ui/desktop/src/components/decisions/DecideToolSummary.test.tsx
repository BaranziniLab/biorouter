/**
 * The `decisions__decide` tool row, through the real ToolCallWithResponse: one
 * row per batch, its well holding the model, the questions in plain words and
 * their counts (at most three), and never the chunks, the per-chunk table, the
 * question names, the type, the threshold or the time (owner addendum A1, F-7).
 */
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import ToolCallWithResponse from '../ToolCallWithResponse';
import type { ToolRequestMessageContent, ToolResponseMessageContent } from '../../types/message';
import { DECIDE_TOOL_COPY, decideCallMeta, summarizeDecideCall } from './DecideToolSummary';
import { CALIBRATION_COPY } from './decisionWords';
import { PRIVATE_MARK_INK } from '../ui/PrivacyBadge';
import { wireDecideSummary } from './decisionFixtures';

const CHUNKS = Array.from({ length: 600 }, (_, i) => ({
  id: `note-${i}`,
  text: `Neuro f/u ${i}. Patient PHI-${i} reports...`,
}));

const ARGS = {
  questions: {
    has_ms: {
      instructions: 'Does this note state the patient has multiple sclerosis?',
      type: 'yes_no',
      threshold: 0.9,
    },
    edss_stated: { instructions: 'Is an EDSS score stated?', type: 'yes_no', threshold: 0.8 },
    course: {
      instructions: 'Which MS course is described?',
      type: 'choice',
      options: { rrms: 'Relapsing-remitting', none: 'Not stated' },
      threshold: 0.6,
      pass_option: 'rrms',
    },
  },
  chunks: CHUNKS,
};

const SUMMARY = wireDecideSummary();

/** What the model reads: a per-chunk table, which the row must never draw. */
const TABLE = CHUNKS.map((chunk) => `${chunk.id} | has_ms yes 0.97`).join('\n');

function show(
  answered: boolean,
  args: Record<string, unknown> = ARGS,
  summary: Record<string, unknown> = SUMMARY
) {
  const toolRequest = {
    type: 'toolRequest',
    id: 'call-1',
    toolCall: { status: 'success', value: { name: 'decisions__decide', arguments: args } },
  } as ToolRequestMessageContent;
  const toolResponse = answered
    ? ({
        type: 'toolResponse',
        id: 'call-1',
        toolResult: {
          status: 'success',
          value: { content: [{ type: 'text', text: TABLE }], structuredContent: summary },
        },
      } as ToolResponseMessageContent)
    : undefined;
  const onOpenArtifact = vi.fn();
  const view = render(
    <ToolCallWithResponse
      sessionId="chat-1"
      toolRequest={toolRequest}
      toolResponse={toolResponse}
      turnActive={!answered}
      isCancelledMessage={false}
      onOpenArtifact={onOpenArtifact}
    />
  );
  return { ...view, onOpenArtifact };
}

/** The batch summary for ONE yes or no question. */
function oneQuestion(overrides: Record<string, unknown> = {}) {
  return {
    ...SUMMARY,
    chunks: 30,
    decided: 30,
    errors: 0,
    errorGroups: [],
    questions: [{ ...SUMMARY.questions[0], decided: 30, yes: 10, passed: 10, review: 0 }],
    ...overrides,
  };
}

const ONE_ARGS = {
  questions: { has_ms: ARGS.questions.has_ms },
  chunks: CHUNKS.slice(0, 30),
};

function openWell(container: HTMLElement) {
  fireEvent.click(container.querySelector('button.br-tool-disclosure')!);
  return screen.getByTestId('decide-tool-summary');
}

describe('the decide tool row', () => {
  it('counts what it asks of how many chunks', () => {
    expect(summarizeDecideCall(ARGS)).toBe('Checking 600 chunks against 3 questions');
    show(false);
    expect(screen.getByText('Checking 600 chunks against 3 questions')).toBeInTheDocument();
  });

  it('is one row once answered, with only the errors in its meta for several questions', () => {
    const { container } = show(true);
    expect(container.querySelectorAll('.br-transcript-row')).toHaveLength(1);
    expect(screen.getByText('Checked 600 chunks against 3 questions')).toBeInTheDocument();
    expect(container).toHaveTextContent('· 2 errors');
    expect(container).not.toHaveTextContent('41 yes');
  });

  it('puts one yes or no question’s answer counts in the meta, which stays short', () => {
    const wrap = (summary: Record<string, unknown>) => ({
      status: 'success',
      value: { content: [], structuredContent: summary },
    });
    expect(decideCallMeta(wrap(oneQuestion()))).toBe('10 yes');
    expect(
      decideCallMeta(
        wrap({
          ...SUMMARY,
          questions: [SUMMARY.questions[0]],
        })
      )
    ).toBe('41 yes, 6 to review, 2 errors');
    // A choice says only its errors.
    expect(decideCallMeta(wrap({ ...SUMMARY, questions: [SUMMARY.questions[2]] }))).toBe(
      '2 errors'
    );
    expect(
      decideCallMeta(wrap({ ...oneQuestion(), questions: [SUMMARY.questions[2]] }))
    ).toBeNull();
    const { container } = show(true, ONE_ARGS, oneQuestion());
    expect(container).toHaveTextContent('Checked 30 chunks against 1 question · 10 yes');
  });

  it('opens onto the model, the questions in plain words and their counts, never the chunks', () => {
    const { container } = show(true);
    const well = openWell(container);
    expect(screen.getByTestId('decide-tool-head')).toHaveTextContent(
      /^tev1:0\.8b · this computer · 2 errors$/
    );
    const lines = screen.getByTestId('decide-tool-questions');
    expect(lines).toHaveTextContent(
      'Does this note state the patient has multiple sclerosis?41 yes · 557 no · 6 of them to review'
    );
    // A review count of 0 says nothing.
    expect(lines).toHaveTextContent('Is an EDSS score stated?19 yes · 579 no');
    expect(lines).not.toHaveTextContent('579 no ·');
    // A choice: each answer's count in option order, then the part of them left to review.
    expect(lines).toHaveTextContent(
      'Which MS course is described?300 rrms · 200 spms · 98 none · 12 of them to review'
    );
    // Off the card: the names, the type word, the threshold, the passed count, the time.
    for (const gone of ['has_ms', 'edss_stated', 'threshold', 'yes or no', 'passed', '13 s']) {
      expect(well).not.toHaveTextContent(gone);
    }
    expect(well).not.toHaveTextContent(/\d ms\b/);
    expect(well).not.toHaveTextContent('Questions');
    expect(well).not.toHaveTextContent('Result');
    // The distinct errors, with how many chunks each hit, not which ones.
    const errors = screen.getByTestId('decide-tool-errors');
    expect(errors).toHaveTextContent('2 chunksThe decision model did not answer in 10 s');
    // Red is rare (design.md 9.14): each error is the failure glyph and words, in ink.
    expect(within(errors).getAllByTestId('decide-tool-error-glyph')).toHaveLength(1);
    expect(errors.outerHTML).not.toMatch(/danger|warning/);
    // No chunk text, no chunk id, no per-chunk line anywhere in the row.
    expect(container).not.toHaveTextContent('PHI-1');
    expect(container).not.toHaveTextContent('note-0');
    expect(container.textContent!.length).toBeLessThan(1200);
  });

  it('says when fewer chunks were checked than sent, beyond the errors', () => {
    const { container } = show(true, ONE_ARGS, oneQuestion({ chunks: 30, decided: 28 }));
    openWell(container);
    expect(screen.getByTestId('decide-tool-head')).toHaveTextContent(
      'tev1:0.8b · this computer · 28 of 30 chunks checked'
    );
  });

  it('names at most three questions, and points to the history for the rest', () => {
    const extra = (name: string, instructions: string) => ({
      [name]: { instructions, type: 'yes_no', threshold: 0.5 },
    });
    const args = {
      ...ARGS,
      questions: {
        ...ARGS.questions,
        ...extra('dmt', 'Is the patient on a disease-modifying therapy?'),
        ...extra('mri', 'Is a new MRI lesion reported?'),
      },
    };
    const summary = {
      ...SUMMARY,
      questions: [
        ...SUMMARY.questions,
        { ...SUMMARY.questions[1], name: 'dmt', yes: 212 },
        { ...SUMMARY.questions[1], name: 'mri', yes: 3 },
      ],
    };
    const { container } = show(true, args, summary);
    openWell(container);
    expect(screen.getByTestId('decide-tool-questions').querySelectorAll('li')).toHaveLength(3);
    expect(screen.getByTestId('decide-tool-more')).toHaveTextContent(
      '2 more questions in the decision history'
    );
    expect(screen.getByTestId('decide-tool-summary')).not.toHaveTextContent('disease-modifying');
  });

  it('says an uncalibrated batch is an estimate, with the reason in an InfoTip', () => {
    const { container } = show(true, ARGS, { ...SUMMARY, calibrated: false });
    openWell(container);
    const estimate = screen.getByTestId('decide-tool-estimate');
    expect(estimate).toHaveTextContent(CALIBRATION_COPY.estimate);
    expect(within(estimate).getByRole('button', { name: /^About / })).toBeInTheDocument();
  });

  it('while it runs, names only the questions it asked: no counts and no link', () => {
    const args = {
      questions: {
        course: { question: 'Which MS course is described?', options: { rrms: 'RR', spms: 'SP' } },
      },
      chunks: CHUNKS,
    };
    const { container } = show(false, args);
    const well = openWell(container);
    // `question` is the tool's other name for the instructions.
    expect(well).toHaveTextContent(/^Which MS course is described\?$/);
    expect(screen.queryByRole('button', { name: DECIDE_TOOL_COPY.history })).toBeNull();
  });

  it('calls a question the call gave no text "Question 1", never by its name', () => {
    const args = { questions: { course: { type: 'choice' } }, chunks: CHUNKS };
    const { container } = show(false, args);
    expect(openWell(container)).toHaveTextContent(/^Question 1$/);
  });

  it('names a Baseten decision model by its alias and place, not by its slug', () => {
    const summary = {
      ...SUMMARY,
      model: {
        provider: 'baseten_decision',
        model: 'decision-models/baseten-joey-2b-latest',
        adapter: 'system_one',
      },
      tier: 'public',
    };
    const { container } = show(true, ARGS, summary);
    openWell(container);
    const head = screen.getByTestId('decide-tool-head');
    expect(head).toHaveTextContent(/^joey-1 · Baseten · 2 errors$/);
    expect(head).not.toHaveTextContent('decision-models/');
    expect(head).not.toHaveTextContent('baseten_decision');
    expect(screen.queryByTestId('decide-tool-private')).toBeNull();
  });

  // R1-10 (live QA, 2026-11-10): the decide row named a private model with no mark,
  // where the decision card, the composer chip and the summary roster draw the padlock.
  it('marks a private model with the one padlock, in the shared ink, after its name', () => {
    const { container } = show(true);
    openWell(container);
    const head = screen.getByTestId('decide-tool-head');
    expect(head).toHaveTextContent(/^tev1:0\.8b · this computer · 2 errors$/);
    const lock = within(head).getByRole('img', { name: DECIDE_TOOL_COPY.privateModel });
    expect(lock.getAttribute('class')).toContain(PRIVATE_MARK_INK);
    expect(lock.previousSibling?.textContent).toBe('tev1:0.8b · this computer');
  });

  it('ends with the one decision history link, opening this call', () => {
    const { container, onOpenArtifact } = show(true);
    const well = openWell(container);
    const link = screen.getByRole('button', { name: DECIDE_TOOL_COPY.history });
    expect(link).toHaveTextContent(/^Decision history$/);
    expect(link).toHaveClass('br-decision-link');
    // The last line of its block.
    expect(well.lastElementChild).toBe(link);
    fireEvent.click(link);
    expect(onOpenArtifact).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: 'decisions',
        sessionId: 'chat-1',
        filter: { origin: 'tool', batchId: 'batch-42' },
      })
    );
  });
});

describe('counts that add up (R1-5, R3-F1)', () => {
  const ONE_YES_NO = {
    questions: { has_ms: ARGS.questions.has_ms },
    chunks: CHUNKS.slice(0, 20),
  };

  it('shows the four answer groups as yes and no that add up, review said as part of them', () => {
    // N1 is a yes to review: the old card read "yes 1 · no 19 · 4 to review", 24 for 20.
    const summary = oneQuestion({
      chunks: 20,
      decided: 20,
      questions: [
        {
          ...SUMMARY.questions[0],
          decided: 20,
          yes: 1,
          passed: 1,
          review: 4,
          groups: {
            yesConfident: 0,
            yesReview: 1,
            noReview: 3,
            noConfident: 16,
            passFrom: 0.9,
            confidentFrom: 0.9,
            reviewFrom: 0.2,
          },
        },
      ],
    });
    const { container } = show(true, ONE_YES_NO, summary);
    expect(container).toHaveTextContent('1 yes, 4 to review');
    openWell(container);
    expect(screen.getByTestId('decide-tool-questions')).toHaveTextContent(
      /1 yes · 19 no · 4 of them to review$/
    );
  });

  it('reads a yes or no question sent as answer counts the same way', () => {
    const summary = oneQuestion({
      questions: [
        {
          ...SUMMARY.questions[0],
          type: 'binary',
          decided: 20,
          yes: null,
          review: 4,
          answerCounts: { yes: 1, no: 19 },
        },
      ],
    });
    const { container } = show(true, ONE_YES_NO, summary);
    openWell(container);
    expect(screen.getByTestId('decide-tool-questions')).toHaveTextContent(
      /1 yes · 19 no · 4 of them to review$/
    );
  });

  it('shows a vote of several models by its majority and how often the models agreed', () => {
    const vote = {
      kind: 'decide_vote',
      batchId: 'batch-vote',
      chunks: 20,
      elapsedMs: 9000,
      models: ['tev1:0.8b', 'clef-flash:latest', 'joey-1', 'elephant-1', 'mammoth-1'].map(
        (label) => ({
          model: { provider: 'baseten_decision', model: `decision-models/${label}` },
          label,
          tier: 'public',
          decided: 20,
          errors: 0,
          error: null,
        })
      ),
      questions: [
        {
          name: 'has_ms',
          type: 'yes_no',
          threshold: 0.9,
          passOn: 'yes',
          perModel: [{ yes: 6, no: 14 }],
          majority: { yes: 7, no: 13 },
          ties: 0,
          compared: 20,
          agreed: 19,
          disagreements: [],
          moreDisagreements: 0,
          majorityIds: {},
        },
      ],
    };
    const { container } = show(true, ONE_YES_NO, vote);
    expect(container).toHaveTextContent('7 yes by majority');
    openWell(container);
    expect(screen.getByTestId('decide-tool-head')).toHaveTextContent(
      'Vote of 5 models: tev1:0.8b, clef-flash:latest, joey-1, elephant-1, mammoth-1'
    );
    expect(screen.getByTestId('decide-tool-questions')).toHaveTextContent(
      /Majority 7 yes · 13 no · the models agreed on 19 of 20$/
    );
    // No slug, ever.
    expect(container).not.toHaveTextContent('decision-models/');
  });
});

describe('a choice with a threshold', () => {
  const COURSE_ARGS = {
    questions: { course: ARGS.questions.course },
    chunks: CHUNKS.slice(0, 30),
  };
  // The daemon sends the four groups for a choice with a threshold too; the card
  // drew them as "10 yes · 20 no", answers a three-way choice never offered.
  const course = {
    ...SUMMARY.questions[2],
    decided: 30,
    passed: 10,
    review: 5,
    answerCounts: { rrms: 10, spms: 15, none: 5 },
    groups: {
      yesConfident: 8,
      yesReview: 2,
      noReview: 3,
      noConfident: 17,
      passFrom: 0.6,
      confidentFrom: 0.8,
      reviewFrom: 0.2,
    },
  };

  it('lists each of its answers in option order, never yes and no', () => {
    const summary = oneQuestion({ questions: [course] });
    const { container } = show(true, COURSE_ARGS, summary);
    // The closed row's meta gives a yes count for a yes or no question only.
    expect(
      decideCallMeta({ status: 'success', value: { content: [], structuredContent: summary } })
    ).toBeNull();
    expect(container).not.toHaveTextContent('10 yes');
    openWell(container);
    const lines = screen.getByTestId('decide-tool-questions');
    expect(lines).toHaveTextContent(
      'Which MS course is described?10 rrms · 15 spms · 5 none · 5 of them to review'
    );
    expect(lines).not.toHaveTextContent(/\d yes\b/);
    expect(lines).not.toHaveTextContent(/\d no\b/);
  });
});

describe('a call that escalated', () => {
  const answer = (overrides: Record<string, unknown> = {}) => ({
    question: 'has_ms',
    label: 'no',
    p: 0.9,
    by: 'clef-flash:latest',
    firstLabel: 'yes',
    firstP: 0.6,
    ...overrides,
  });
  // Every other chunk of the call, by the id the agent gave it.
  const items = Array.from({ length: 12 }, (_, i) => ({
    chunkId: `note-${i * 2}`,
    answers: [
      i === 1
        ? answer({ label: 'yes', p: 0.6, by: 'tev1:0.8b', error: 'the decision server failed' })
        : answer(),
    ],
  }));
  const escalation = {
    model: { provider: 'ollama_decision', model: 'clef-flash:latest' },
    label: 'clef-flash:latest',
    tier: 'private',
    firstLabel: 'tev1:0.8b',
    below: 0.8,
    firstDecided: 30,
    escalated: 12,
    answers: 12,
    decided: 11,
    changed: 11,
    errors: 1,
    items,
    errorGroups: [],
  };
  const summary = oneQuestion({
    escalation,
    lowConfidenceBelow: 0.8,
    lowConfidenceIds: ['n01'],
  });

  it('says it in one line, with the chunks asked again behind the row and capped', () => {
    const { container } = show(true, ONE_ARGS, summary);
    // Closed: the meta adds only the count, and no chunk shows.
    expect(container).toHaveTextContent(
      'Checked 30 chunks against 1 question · 10 yes, 12 escalated'
    );
    expect(container).not.toHaveTextContent('Chunk 1');
    const well = openWell(container);
    const line = screen.getByTestId('decide-tool-escalation');
    expect(screen.getByTestId('decide-tool-escalation-text')).toHaveTextContent(
      /^12 of 30 escalated to clef-flash:latest · 1 failed$/
    );
    expect(within(line).getByRole('img', { name: DECIDE_TOOL_COPY.privateModel })).toBeTruthy();
    // The bound and what became of the failures sit in the line's InfoTip.
    expect(
      within(line).getByRole('button', { name: `About ${DECIDE_TOOL_COPY.escalation}` })
    ).toBeInTheDocument();
    expect(line).toHaveTextContent(
      'Answers with a top probability under 0.80 were asked again of clef-flash:latest, and ' +
        "its answers are final. Where it gave no answer, tev1:0.8b's answer stands."
    );
    const listed = screen.getByTestId('decide-tool-escalated');
    expect(listed.querySelectorAll('li')).toHaveLength(10);
    // Each chunk by its number in the call, as the decision history names it, never
    // by the id the agent gave it.
    expect(listed.querySelector('li')).toHaveTextContent(
      'Chunk 1no 0.90, was yes 0.60 by tev1:0.8b'
    );
    expect(listed.querySelectorAll('li')[1]).toHaveTextContent(
      'Chunk 3yes by tev1:0.8b: the second model gave no answer'
    );
    expect(listed).not.toHaveTextContent('note-');
    expect(screen.getByTestId('decide-tool-escalated-more')).toHaveTextContent(
      '2 more chunks in the decision history'
    );
    // The single question needs no name beside each answer; the text never shows.
    expect(listed).not.toHaveTextContent('multiple sclerosis');
    expect(well).not.toHaveTextContent('PHI-');
    expect(well).not.toHaveTextContent('Chunk 21');
  });

  it('says what the second model changed when nothing failed', () => {
    const clean = {
      ...summary,
      escalation: {
        ...escalation,
        errors: 0,
        decided: 12,
        changed: 3,
        items: items.map((item) => ({ ...item, answers: [answer()] })),
      },
    };
    const { container } = show(true, ONE_ARGS, clean);
    openWell(container);
    expect(screen.getByTestId('decide-tool-escalation-text')).toHaveTextContent(
      /^12 of 30 escalated to clef-flash:latest$/
    );
    expect(screen.getByTestId('decide-tool-escalation')).toHaveTextContent(
      'and its answers are final. It changed 3 of 12.'
    );
  });

  it('names the question beside each answer when several were asked again', () => {
    const several = {
      ...summary,
      escalation: {
        ...escalation,
        items: [
          {
            chunkId: 'note-4',
            answers: [
              answer(),
              answer({ question: 'edss_stated', label: 'yes', firstLabel: 'no' }),
            ],
          },
        ],
      },
    };
    const { container } = show(true, ARGS, several);
    openWell(container);
    expect(screen.getByTestId('decide-tool-escalated')).toHaveTextContent(
      'Chunk 5Does this note state the patient has multiple sclerosis?: no 0.90, was yes 0.60 ' +
        'by tev1:0.8b; Is an EDSS score stated?: yes 0.90, was no 0.60 by tev1:0.8b'
    );
  });

  it('draws no escalation for a call that did not escalate', () => {
    const { container } = show(true, ONE_ARGS, oneQuestion());
    openWell(container);
    expect(screen.queryByTestId('decide-tool-escalation')).toBeNull();
    expect(screen.queryByTestId('decide-tool-escalated')).toBeNull();
  });
});

describe('opening the decide row at the end of the conversation (G-7)', () => {
  it('scrolls the open card into view once the panel has opened', async () => {
    const scrollIntoView = vi.fn();
    const original = Element.prototype.scrollIntoView;
    Element.prototype.scrollIntoView = scrollIntoView;
    try {
      const { container } = show(true);
      expect(scrollIntoView).not.toHaveBeenCalled();
      fireEvent.click(container.querySelector('button.br-tool-disclosure')!);
      const card = screen.getByTestId('decide-tool-summary');
      await waitFor(() => expect(scrollIntoView).toHaveBeenCalledTimes(1), { timeout: 2000 });
      expect(scrollIntoView.mock.contexts[0]).toBe(card);
      expect(scrollIntoView.mock.calls[0][0]).toMatchObject({ block: 'nearest' });
    } finally {
      Element.prototype.scrollIntoView = original;
    }
  });
});
