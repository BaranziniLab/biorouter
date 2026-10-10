import { act, render, screen } from '@testing-library/react';
import { beforeAll, afterAll, describe, expect, it, vi } from 'vitest';
import { ComposerFooter, type ComposerFooterProps } from './ComposerFooter';
import { FOOTER_COPY } from './copy';

vi.mock('../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({ currentModel: 'gpt-5.6-sol', currentProvider: 'versa_azure' }),
}));
vi.mock('../../utils/pricing', () => ({
  fetchModelPricing: vi.fn(async () => ({
    input_token_cost: 0.00001,
    output_token_cost: 0.00003,
    currency: '$',
  })),
}));

beforeAll(() => {
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
  );
});
afterAll(() => vi.unstubAllGlobals());

const BASE: ComposerFooterProps = {
  sessionId: 'chat-1',
  workingDir: '/Users/wgu/Desktop',
  workingDirLocked: true,
  onWorkingDirChange: vi.fn(),
  totalTokens: 92_000,
  tokenLimit: 128_000,
  isTokenLimitLoaded: true,
  onCompact: vi.fn(),
  modelCostRows: [
    {
      provider: 'versa_azure',
      model: 'gpt-5.6-sol',
      inputTokens: 1000,
      outputTokens: 200,
      cacheReadTokens: 0,
      cacheCreationTokens: 0,
      totalTokens: 1200,
      turns: 2,
      totalCost: 0.42,
      costIsPartial: false,
    },
  ],
};

/** Let the cost tracker's pricing fetch land, so its state update is inside act. */
async function settle() {
  await act(async () => {});
}

describe('ComposerFooter (spec 3.7)', () => {
  it('names the folder on the left and reads context and cost on the right in a chat', async () => {
    render(<ComposerFooter {...BASE} />);
    await settle();
    const footer = screen.getByTestId('composer-footer');
    expect(footer).toHaveClass('br-footline');
    expect(screen.getByText('Desktop')).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: FOOTER_COPY.contextTooltip(28, '92k', '128k') })
    ).toBeInTheDocument();
    expect(screen.getByText(FOOTER_COPY.contextLeft(28))).toBeInTheDocument();
    expect(screen.getByTestId('chat-cost')).toHaveTextContent('$0.42');
  });

  it('shows only the folder on Home: no ring, no "100%", no "$0.00"', async () => {
    render(<ComposerFooter {...BASE} sessionId={null} workingDirLocked={false} />);
    await settle();
    expect(screen.getByText('Desktop')).toBeInTheDocument();
    expect(screen.queryByTestId('context-window-indicator')).toBeNull();
    expect(screen.queryByTestId('chat-cost')).toBeNull();
    expect(screen.queryByText('100%')).toBeNull();
    expect(screen.queryByText('$0.00')).toBeNull();
  });

  it('hides a $0.00 cost inside a chat', async () => {
    render(
      <ComposerFooter {...BASE} modelCostRows={[{ ...BASE.modelCostRows![0], totalCost: 0 }]} />
    );
    await settle();
    expect(screen.queryByTestId('chat-cost')).toBeNull();
    expect(screen.queryByText('$0.00')).toBeNull();
  });

  it('has no monospace text anywhere on the line', async () => {
    const { container } = render(<ComposerFooter {...BASE} />);
    await settle();
    expect(container.querySelector('.font-mono')).toBeNull();
  });
});
