import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  getLoopExitCriteria,
  getBasetenDecisionModels,
  getProviderModels,
  providers,
  setLoopExitCriteria,
  type ExitCriteria,
} from '../../api';
import { getDraftExitCriteria } from '../../store/loopExitCriteria';
import { LoopExitCriteria } from './LoopExitCriteria';
vi.mock('../../api', () => ({
  getLoopExitCriteria: vi.fn(),
  getBasetenDecisionModels: vi.fn(),
  getProviderModels: vi.fn(),
  providers: vi.fn(),
  setLoopExitCriteria: vi.fn(),
}));
const criteria: ExitCriteria = {
  timing: 'post_response',
  prompt: 'Is progress useful?',
  provider: 'public-fixture',
  model: 'joey-1',
  decision: { type: 'boolean', interrupt_when: false },
};
const basetenModels = [
  { name: 'elephant-1', slug: 'decision-models/baseten-elephant-9b-latest' },
  { name: 'mammoth-1', slug: 'decision-models/baseten-mammoth-27b-latest' },
  { name: 'joey-1', slug: 'decision-models/baseten-joey-2b-latest' },
];
const catalog = [
  {
    name: 'baseten',
    is_configured: true,
    resolved_tier: 'public',
    metadata: { display_name: 'Baseten', known_models: [{ name: 'zai-org/GLM-5.3' }] },
  },
  {
    name: 'public-fixture',
    is_configured: true,
    resolved_tier: 'public',
    metadata: { display_name: 'Public fixture', known_models: [{ name: 'joey-1' }] },
  },
  {
    name: 'private-fixture',
    is_configured: true,
    resolved_tier: 'private',
    metadata: { display_name: 'Private fixture', known_models: [{ name: 'mammoth-1' }] },
  },
  {
    name: 'unresolved-fixture',
    is_configured: true,
    resolved_tier: null,
    metadata: { display_name: 'Unresolved fixture', known_models: [] },
  },
];
beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(getBasetenDecisionModels).mockResolvedValue({
    data: basetenModels,
    response: new Response(null, { status: 200 }),
  } as never);
  vi.mocked(getLoopExitCriteria).mockResolvedValue({ data: { criteria } } as never);
  vi.mocked(providers).mockResolvedValue({ data: catalog } as never);
  vi.mocked(getProviderModels).mockResolvedValue({ data: ['joey-1', 'elephant-1'] } as never);
  vi.mocked(setLoopExitCriteria).mockResolvedValue({ data: { criteria } } as never);
});
async function open(
  sessionId: string | null = 'chat-one',
  privacyTier: 'public' | 'private' = 'public'
) {
  render(
    <LoopExitCriteria
      sessionId={sessionId}
      draftScope="draft:ui-fixture"
      privacyTier={privacyTier}
    />
  );
  fireEvent.click(screen.getByRole('button', { name: 'Exit criteria' }));
  await waitFor(() =>
    expect(screen.queryByText('Loading criteria and configured models…')).not.toBeInTheDocument()
  );
}
function pick(label: string, option: string) {
  fireEvent.keyDown(screen.getByLabelText(label), { key: 'ArrowDown', code: 'ArrowDown' });
  fireEvent.click(screen.getByText(option));
}
describe('loop exit criteria editor', () => {
  it('loads and saves a chat criterion', async () => {
    await open();
    expect(screen.getByLabelText('Evaluation prompt')).toHaveValue(criteria.prompt);
    fireEvent.change(screen.getByLabelText('Evaluation prompt'), {
      target: { value: 'Revised criterion' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save criteria' }));
    await waitFor(() =>
      expect(setLoopExitCriteria).toHaveBeenCalledWith({
        path: { session_id: 'chat-one' },
        body: { criteria: { ...criteria, prompt: 'Revised criterion' } },
        throwOnError: true,
      })
    );
  });
  it('groups configured providers by resolved labels and omits unresolved ones', async () => {
    await open();
    fireEvent.keyDown(screen.getByLabelText('Decision provider · Public / Private'), {
      key: 'ArrowDown',
    });
    expect(screen.getByText('Public')).toBeInTheDocument();
    expect(screen.getByText('Private')).toBeInTheDocument();
    expect(screen.queryByText('Unresolved fixture')).not.toBeInTheDocument();
  });
  it('disables public evaluators in private chats', async () => {
    await open('private-chat', 'private');
    expect(
      screen.getByText('Public evaluators cannot receive this private chat.')
    ).toBeInTheDocument();
    fireEvent.keyDown(screen.getByLabelText('Decision provider · Public / Private'), {
      key: 'ArrowDown',
    });
    expect(
      screen
        .getAllByText('Public fixture')
        .some((element) => element.closest('[aria-disabled="true"]'))
    ).toBe(true);
  });
  it('validates ranking bounds and saves a numeric threshold', async () => {
    await open();
    pick('Decision type', 'Ranking · numeric score');
    fireEvent.change(screen.getByLabelText('Interrupt below'), { target: { value: '11' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save criteria' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('threshold within that range');
    expect(setLoopExitCriteria).not.toHaveBeenCalled();
    fireEvent.change(screen.getByLabelText('Interrupt below'), { target: { value: '4.5' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save criteria' }));
    await waitFor(() =>
      expect(setLoopExitCriteria).toHaveBeenCalledWith(
        expect.objectContaining({
          body: {
            criteria: {
              ...criteria,
              decision: { type: 'ranking', min: 0, max: 10, interrupt_below: 4.5 },
            },
          },
        })
      )
    );
  });
  it('rejects duplicate choices and supports disabling criteria', async () => {
    await open();
    pick('Decision type', 'Choice');
    fireEvent.change(screen.getByLabelText('Choices · one per line'), {
      target: { value: 'same\nsame' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save criteria' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('unique choices');
    fireEvent.click(screen.getByLabelText('Evaluate each agentic iteration'));
    fireEvent.click(screen.getByRole('button', { name: 'Save criteria' }));
    await waitFor(() =>
      expect(setLoopExitCriteria).toHaveBeenCalledWith(
        expect.objectContaining({ body: { criteria: null } })
      )
    );
  });
  it('keeps save failures visible', async () => {
    vi.mocked(setLoopExitCriteria).mockRejectedValueOnce(new Error('not permitted'));
    await open();
    fireEvent.click(screen.getByRole('button', { name: 'Save criteria' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('not permitted');
    expect(screen.getByRole('dialog')).toBeInTheDocument();
  });
  it('stores new-chat criteria in its draft without sending an API request', async () => {
    await open(null);
    fireEvent.click(screen.getByLabelText('Evaluate each agentic iteration'));
    pick('Decision provider · Public / Private', 'Public fixture');
    fireEvent.change(screen.getByLabelText('Decision model ID'), { target: { value: 'joey-1' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save criteria' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(getDraftExitCriteria('draft:ui-fixture')?.model).toBe('joey-1');
    expect(setLoopExitCriteria).not.toHaveBeenCalled();
  });
  it('does not allow overwriting criteria when their initial read failed', async () => {
    vi.mocked(getLoopExitCriteria).mockRejectedValueOnce(new Error('unreadable criteria'));
    await open();
    expect(await screen.findByRole('alert')).toHaveTextContent('unreadable criteria');
    expect(screen.getByRole('button', { name: 'Save criteria' })).toBeDisabled();
  });

  it('saves a valid choice contract with its selected interrupt value', async () => {
    await open();
    pick('Decision type', 'Choice');
    fireEvent.click(screen.getByRole('button', { name: 'Save criteria' }));
    await waitFor(() =>
      expect(setLoopExitCriteria).toHaveBeenCalledWith(
        expect.objectContaining({
          body: {
            criteria: {
              ...criteria,
              decision: {
                type: 'choice',
                choices: ['continue', 'needs_adjustment'],
                interrupt_when: 'needs_adjustment',
              },
            },
          },
        })
      )
    );
  });
  it('shows Baseten decision models rather than the provider’s chat catalog', async () => {
    await open();
    vi.mocked(getProviderModels).mockClear();
    pick('Decision provider · Public / Private', 'Baseten');
    await waitFor(() => expect(getBasetenDecisionModels).toHaveBeenCalled());
    fireEvent.keyDown(screen.getByLabelText('Available models'), { key: 'ArrowDown' });
    expect(await screen.findByText('elephant-1')).toBeInTheDocument();
    expect(screen.getByText('mammoth-1')).toBeInTheDocument();
    expect(screen.getByText('joey-1')).toBeInTheDocument();
    expect(screen.queryByText('zai-org/GLM-5.3')).not.toBeInTheDocument();
    expect(getProviderModels).not.toHaveBeenCalled();
    fireEvent.click(screen.getByText('elephant-1'));
    expect(screen.getByLabelText('P(yes) threshold')).toHaveValue(0.5);
    fireEvent.change(screen.getByLabelText('P(yes) threshold'), { target: { value: '0.9' } });
    pick('Interrupt when P(yes) is', 'At or above threshold');
    fireEvent.click(screen.getByRole('button', { name: 'Save criteria' }));
    await waitFor(() =>
      expect(setLoopExitCriteria).toHaveBeenCalledWith(
        expect.objectContaining({
          body: {
            criteria: {
              ...criteria,
              provider: 'baseten',
              model: basetenModels[0].slug,
              decision: { type: 'noul', threshold: 0.9, interrupt_when: true },
            },
          },
        })
      )
    );
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('uses an ordered rubric for Baseten ranking instead of an arbitrary numeric scale', async () => {
    vi.mocked(getLoopExitCriteria).mockResolvedValueOnce({
      data: {
        criteria: {
          ...criteria,
          provider: 'baseten',
          model: basetenModels[2].slug,
          decision: { type: 'noul', threshold: 0.9, interrupt_when: true },
        },
      },
    } as never);
    await open();
    pick('Decision type', 'Ranking · ordered rubric');
    expect(screen.queryByLabelText('Minimum')).not.toBeInTheDocument();
    fireEvent.change(screen.getByLabelText('Ordered rubric · one level per line'), {
      target: { value: 'Poor\nFair\nGood' },
    });
    fireEvent.change(screen.getByLabelText('Interrupt below'), { target: { value: '1.5' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save criteria' }));
    await waitFor(() =>
      expect(setLoopExitCriteria).toHaveBeenCalledWith(
        expect.objectContaining({
          body: {
            criteria: {
              ...criteria,
              provider: 'baseten',
              model: basetenModels[2].slug,
              decision: {
                type: 'ranking',
                min: 0,
                max: 2,
                interrupt_below: 1.5,
                levels: ['Poor', 'Fair', 'Good'],
              },
            },
          },
        })
      )
    );
  });
  it('saves pre-send timing and explains that it blocks the next main request', async () => {
    await open();
    expect(screen.getByText('Post-response')).toBeInTheDocument();
    pick('Evaluation timing', 'Pre-send');
    expect(
      screen.getByText(/A failed check prevents that request from starting/)
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Save criteria' }));
    await waitFor(() =>
      expect(setLoopExitCriteria).toHaveBeenCalledWith(
        expect.objectContaining({
          body: { criteria: { ...criteria, timing: 'pre_send' } },
        })
      )
    );
  });

  it('restores a saved pre-send selection', async () => {
    vi.mocked(getLoopExitCriteria).mockResolvedValueOnce({
      data: { criteria: { ...criteria, timing: 'pre_send' } },
    } as never);
    await open();
    expect(screen.getByText('Pre-send')).toBeInTheDocument();
  });

  it('keeps old criteria without a timing field on the post-response default', async () => {
    vi.mocked(getLoopExitCriteria).mockResolvedValueOnce({
      data: {
        criteria: {
          prompt: criteria.prompt,
          provider: criteria.provider,
          model: criteria.model,
          decision: criteria.decision,
        },
      },
    } as never);
    await open();
    expect(screen.getByText('Post-response')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Save criteria' }));
    await waitFor(() =>
      expect(setLoopExitCriteria).toHaveBeenCalledWith(
        expect.objectContaining({ body: { criteria } })
      )
    );
  });
  it('explains a stale shared daemon’s 404 and retries without losing the form', async () => {
    vi.mocked(getBasetenDecisionModels).mockResolvedValueOnce({
      error: '',
      response: new Response(null, { status: 404 }),
    } as never);
    await open();
    fireEvent.change(screen.getByLabelText('Evaluation prompt'), {
      target: { value: 'Keep this unsaved prompt' },
    });
    pick('Decision provider · Public / Private', 'Baseten');
    expect(await screen.findByText(/background service is older.*HTTP 404/)).toBeInTheDocument();
    expect(screen.getByText(/Closing Electron can leave that service running/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Retry model list' }));
    await waitFor(() => expect(getBasetenDecisionModels).toHaveBeenCalledTimes(2));
    await waitFor(() =>
      expect(screen.queryByRole('button', { name: 'Retry model list' })).not.toBeInTheDocument()
    );
    expect(screen.getByLabelText('Evaluation prompt')).toHaveValue('Keep this unsaved prompt');
    fireEvent.keyDown(screen.getByLabelText('Available models'), { key: 'ArrowDown' });
    expect(await screen.findByText('elephant-1')).toBeInTheDocument();
    expect(screen.getByText('mammoth-1')).toBeInTheDocument();
    expect(screen.getByText('joey-1')).toBeInTheDocument();
  });

  it('does not misdiagnose an authorization failure as an old backend', async () => {
    vi.mocked(getBasetenDecisionModels).mockResolvedValueOnce({
      error: 'Forbidden',
      response: new Response(null, { status: 403 }),
    } as never);
    await open();
    pick('Decision provider · Public / Private', 'Baseten');
    expect(await screen.findByText(/refused catalog access.*HTTP 403/)).toBeInTheDocument();
    expect(screen.queryByText(/background service is older/)).not.toBeInTheDocument();
  });
});
