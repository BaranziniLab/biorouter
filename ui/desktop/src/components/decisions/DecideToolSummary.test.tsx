/**
 * The `decisions__decide` tool row, through the real ToolCallWithResponse: one
 * row per batch, its well holding the questions and the per-question counts, and
 * never the chunks or the per-chunk table (owner addendum A1).
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import ToolCallWithResponse from '../ToolCallWithResponse';
import type { ToolRequestMessageContent, ToolResponseMessageContent } from '../../types/message';
import { DECIDE_TOOL_COPY, summarizeDecideCall } from './DecideToolSummary';
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

function show(answered: boolean) {
  const toolRequest = {
    type: 'toolRequest',
    id: 'call-1',
    toolCall: { status: 'success', value: { name: 'decisions__decide', arguments: ARGS } },
  } as ToolRequestMessageContent;
  const toolResponse = answered
    ? ({
        type: 'toolResponse',
        id: 'call-1',
        toolResult: {
          status: 'success',
          value: { content: [{ type: 'text', text: TABLE }], structuredContent: SUMMARY },
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

describe('the decide tool row', () => {
  it('counts what it asks of how many chunks', () => {
    expect(summarizeDecideCall(ARGS)).toBe('Checking 600 chunks against 3 questions');
    show(false);
    expect(screen.getByText('Checking 600 chunks against 3 questions')).toBeInTheDocument();
  });

  it('is one row once answered, with the errors in its meta', () => {
    const { container } = show(true);
    expect(container.querySelectorAll('.br-transcript-row')).toHaveLength(1);
    expect(screen.getByText('Checked 600 chunks against 3 questions')).toBeInTheDocument();
    expect(container).toHaveTextContent('· 2 errors');
  });

  it('opens onto the questions and per-question counts, never the chunks or the table', () => {
    const { container } = show(true);
    fireEvent.click(container.querySelector('button.br-tool-disclosure')!);
    const well = screen.getByTestId('decide-tool-summary');
    expect(well).toHaveTextContent('Does this note state the patient has multiple sclerosis?');
    expect(well).toHaveTextContent('threshold 0.90');
    expect(screen.getByTestId('decide-tool-head')).toHaveTextContent(
      '600 chunks · tev1:0.8b · 2 errors · 13 s'
    );
    const lines = screen.getByTestId('decide-tool-questions');
    expect(lines).toHaveTextContent('has_msyes 41, 6 to review, max p 0.98');
    // A review count of 0 says nothing.
    expect(lines).toHaveTextContent('edss_statedyes 19, 17 passed, max p 0.91');
    // A choice with a threshold has a review list too, not only a yes or no question.
    expect(lines).toHaveTextContent(
      'courserrms 300, spms 200, none 98, 280 passed, 12 to review, max p 0.71'
    );
    expect(lines).not.toHaveTextContent('uncertain');
    // The distinct errors, with how many chunks each hit, not which ones.
    expect(screen.getByTestId('decide-tool-errors')).toHaveTextContent(
      '2 chunksThe decision model did not answer in 10 s'
    );
    // No chunk text, no chunk id, no per-chunk line anywhere in the row.
    expect(container).not.toHaveTextContent('PHI-1');
    expect(container).not.toHaveTextContent('note-0');
    expect(container.textContent!.length).toBeLessThan(1200);
  });

  it('links to the batch in the decision history', () => {
    const { container, onOpenArtifact } = show(true);
    fireEvent.click(container.querySelector('button.br-tool-disclosure')!);
    fireEvent.click(screen.getByRole('button', { name: DECIDE_TOOL_COPY.history }));
    expect(onOpenArtifact).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: 'decisions',
        sessionId: 'chat-1',
        filter: { origin: 'tool', batchId: 'batch-42' },
      })
    );
  });
});
