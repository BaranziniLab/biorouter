import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import LoadingBioRouter from './LoadingBioRouter';
import { WorkflowHeader } from './WorkflowHeader';
import { ChatState } from '../types/chatState';
import { transcriptCopy } from './transcript/copy';

describe('LoadingBioRouter (the quiet loading line)', () => {
  it('is one 12px muted line with the shared spinner, and no ring or glow', () => {
    const { container } = render(
      <LoadingBioRouter message={transcriptCopy.activity.loadingMessages} />
    );

    const line = screen.getByTestId('loading-biorouter');
    expect(line).toHaveTextContent(transcriptCopy.activity.loadingMessages);
    expect(line).toHaveClass('text-supporting', 'text-text-muted');
    expect(line).toHaveAttribute('aria-live', 'polite');
    expect(container.querySelector('[class*="working-ring"], [class*="working-glow"]')).toBeNull();
  });

  it('names the chat state when it is given no message', () => {
    render(<LoadingBioRouter chatState={ChatState.LoadingConversation} />);
    expect(screen.getByTestId('loading-biorouter')).toHaveTextContent(
      transcriptCopy.activity.loadingChat
    );
  });
});

describe('WorkflowHeader', () => {
  it('is a 32px line: the kind muted, the name in full ink', () => {
    const { container } = render(<WorkflowHeader title="Weekly review" />);

    expect(container.firstElementChild).toHaveClass('h-8', 'text-secondary');
    expect(screen.getByText(transcriptCopy.workflow.label)).toHaveClass('text-text-muted');
    expect(screen.getByText('Weekly review')).toHaveClass('text-text-default', 'truncate');
  });
});
