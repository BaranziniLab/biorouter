import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type Model from './modelInterface';
import {
  __resetHeldChatModelsForTests,
  heldChatModel,
  holdChatModel,
  useHeldChatModel,
} from './pendingChatModel';

const claude: Model = { name: 'claude-opus-5-5', provider: 'claude_code' };

function Chip({ tabId }: { tabId?: string }) {
  const [held] = useHeldChatModel(tabId);
  return <span data-testid="chip">{held?.name ?? 'none'}</span>;
}

beforeEach(__resetHeldChatModelsForTests);
afterEach(cleanup);

// T3-SH-2: the held pick lived in BaseChat's own state and died with it, so the
// provider catalog (another route) had nowhere to put a pick for the chat.
describe('held picks by chat tab', () => {
  it('outlive the component that showed them', () => {
    const first = render(<Chip tabId="tab-3" />);
    act(() => holdChatModel('tab-3', claude));
    expect(screen.getByTestId('chip')).toHaveTextContent('claude-opus-5-5');
    first.unmount();

    render(<Chip tabId="tab-3" />);
    expect(screen.getByTestId('chip')).toHaveTextContent('claude-opus-5-5');
  });

  it('belong to one tab', () => {
    holdChatModel('tab-3', claude);
    render(<Chip tabId="tab-4" />);
    expect(screen.getByTestId('chip')).toHaveTextContent('none');
    expect(heldChatModel('tab-4')).toBeNull();
  });

  it('are withdrawn with null', () => {
    holdChatModel('tab-3', claude);
    render(<Chip tabId="tab-3" />);
    act(() => holdChatModel('tab-3', null));
    expect(screen.getByTestId('chip')).toHaveTextContent('none');
    expect(heldChatModel('tab-3')).toBeNull();
  });

  it('with no tab, stay in the component', () => {
    holdChatModel('tab-3', claude);
    render(<Chip />);
    expect(screen.getByTestId('chip')).toHaveTextContent('none');
  });
});
