/**
 * Issue #71 — the UI half.
 *
 * The panel is rendered against the *real* `useIngestStream` and a stubbed
 * `fetch`, because the reported bug lived precisely in the seam between them: a
 * digest that failed still ended with "Digest complete", the source was marked
 * ingested, and it was cleared off the staged list — so the user was told their
 * PDF was in the knowledge base and had nothing left on screen to say otherwise.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ModelRef } from '../../../api/types.gen';
import { IngestPanel } from './IngestPanel';
import { knowledgeCopy } from '../copy';

const mocks = vi.hoisted(() => ({
  knowledge: {
    primaryKbId: 'kb-1' as string | null,
    primaryKb: {
      id: 'kb-1',
      name: 'Notes',
      default_model: { provider: 'google', model: 'gemini-3.5-flash-lite' } as ModelRef | null,
    },
    loading: false,
    basesError: null as string | null,
    refresh: vi.fn(),
    triggerGraphRefresh: vi.fn(),
  },
  modelAndProvider: {
    currentProvider: 'google' as string | null,
    currentModel: 'gemini-3.5-flash-lite' as string | null,
    modelConfigStatus: 'ready' as 'loading' | 'ready',
  },
  checkModel: vi.fn(),
  knowledgeFetch: vi.fn(),
  expandKnowledgePath: vi.fn(),
  config: { getProviders: vi.fn(), getProviderModels: vi.fn() },
}));

vi.mock('../KnowledgeContext', () => ({ useKnowledge: () => mocks.knowledge }));
vi.mock('../../ModelAndProviderContext', () => ({
  useModelAndProvider: () => mocks.modelAndProvider,
}));
vi.mock('../../ConfigContext', () => ({ useConfig: () => mocks.config }));
vi.mock('../../../api/sdk.gen', () => ({ checkModel: mocks.checkModel }));
vi.mock('../../../toasts', () => ({ toastError: vi.fn(), toastSuccess: vi.fn() }));

// Only the request plumbing is stubbed — `useIngestStream` itself is the code
// under test, so it is deliberately NOT mocked here.
vi.mock('../hooks/knowledgeRequest', () => ({
  knowledgeFetch: mocks.knowledgeFetch,
  expandKnowledgePath: mocks.expandKnowledgePath,
  buildKnowledgeUrl: async (path: string) => `http://backend.test${path}`,
  getSecretKey: async () => 'secret-123',
}));

/** A 200 response whose SSE body is exactly `text`, delivered in one chunk. */
function sseResponse(text: string): Response {
  const encoder = new TextEncoder();
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(encoder.encode(text));
      controller.close();
    },
  });
  return { ok: true, status: 200, body: stream } as unknown as Response;
}

function stageSomeText() {
  fireEvent.click(screen.getByTestId('knowledge-ingest-paste-text'));
  fireEvent.change(screen.getByPlaceholderText(knowledgeCopy.ingest.pastePlaceholder), {
    target: { value: 'some knowledge' },
  });
  fireEvent.click(screen.getByRole('button', { name: 'Stage' }));
}

async function digest() {
  fireEvent.click(screen.getByTestId('knowledge-digest-button'));
  await waitFor(() => expect(mocks.checkModel).toHaveBeenCalled());
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.checkModel.mockResolvedValue({ data: { ok: true } });
  mocks.config.getProviders.mockResolvedValue([]);
  mocks.config.getProviderModels.mockResolvedValue([]);
});

describe('IngestPanel surfaces a failed digest', () => {
  it('shows the backend error and keeps the source staged', async () => {
    vi.stubGlobal(
      'fetch',
      vi
        .fn()
        .mockResolvedValue(
          sseResponse(
            'data: {"kind":"step","index":0,"assistant_text":""}\n\n' +
              'event: error\ndata: {"message":"ingest wrote no knowledge pages for source hrv-note"}\n\n'
          )
        )
    );

    render(<IngestPanel />);
    stageSomeText();
    await digest();

    await waitFor(() =>
      expect(screen.getByText(/Digest error/i)).toHaveTextContent(/no knowledge pages/i)
    );
    expect(screen.queryByText(/Digest complete/i)).not.toBeInTheDocument();

    // The source must still be on the staged list, carrying its error — a
    // failed digest that quietly cleared the list would leave the user with
    // nothing to retry and nothing to read.
    const staged = screen.getAllByTestId('knowledge-staged-item');
    expect(staged).toHaveLength(1);
    expect(staged[0]).toHaveTextContent(/error/i);
  });

  it('does not claim completion when the stream ends without a terminal frame', async () => {
    vi.stubGlobal(
      'fetch',
      vi
        .fn()
        .mockResolvedValue(
          sseResponse('data: {"kind":"step","index":0,"assistant_text":"reading source"}\n\n')
        )
    );

    render(<IngestPanel />);
    stageSomeText();
    await digest();

    await waitFor(() => expect(screen.getByText(/Digest error/i)).toBeInTheDocument());
    expect(screen.queryByText(/Digest complete/i)).not.toBeInTheDocument();

    const staged = screen.getAllByTestId('knowledge-staged-item');
    expect(staged).toHaveLength(1);
    expect(staged[0]).toHaveTextContent(/error/i);
  });

  it('still reports a real completion as complete', async () => {
    vi.stubGlobal(
      'fetch',
      vi
        .fn()
        .mockResolvedValue(
          sseResponse('event: done\ndata: {"source_id":"hrv","commit_sha":"abc","steps":3}\n\n')
        )
    );

    render(<IngestPanel />);
    stageSomeText();
    await digest();

    await waitFor(() => expect(screen.getByText(/Digest complete/i)).toBeInTheDocument());
    expect(screen.queryByText(/Digest error/i)).not.toBeInTheDocument();
    // A digest that succeeded clears the source off the staged list.
    expect(screen.queryAllByTestId('knowledge-staged-item')).toHaveLength(0);
  });
});
