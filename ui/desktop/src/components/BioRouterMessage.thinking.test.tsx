import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import BioRouterMessage from './BioRouterMessage';
import type { Message } from '../api';

// The transcript is no longer a display surface for artifacts — it can only hand
// one to the panel — so `onOpenArtifact` is required all the way down the chain.
const noopOpenArtifact = vi.fn();

vi.mock('./MarkdownContent', () => ({
  default: ({ content }: { content: string }) => <div data-testid="markdown">{content}</div>,
}));

function thinkingMessage(): Message {
  return {
    id: 'm1',
    role: 'assistant',
    created: 1700000000000,
    metadata: { userVisible: true, agentVisible: true },
    content: [{ type: 'text', text: '<think>weighing two options</think>Here is the answer.' }],
  };
}

describe('BioRouterMessage chain-of-thought disclosure', () => {
  it("is the app's own control, not the browser's <details>", () => {
    const { container } = render(
      <BioRouterMessage
        sessionId="s1"
        message={thinkingMessage()}
        messages={[thinkingMessage()]}
        toolCallNotifications={new Map()}
        onRunInTerminal={null}
        onOpenArtifact={noopOpenArtifact}
      />
    );

    // A native <details> could not take the app's chevron, its 32px row, its
    // type role or its motion — the marker was drawn by the user agent. It is a
    // button now, so all four are ours.
    expect(container.querySelector('details')).toBeNull();
    expect(screen.getByRole('button', { name: /thinking/i })).toHaveAttribute(
      'aria-expanded',
      'false'
    );
  });

  it('keeps the chevron visible at rest and rotates it on open', async () => {
    render(
      <BioRouterMessage
        sessionId="s1"
        message={thinkingMessage()}
        messages={[thinkingMessage()]}
        toolCallNotifications={new Map()}
        onRunInTerminal={null}
        onOpenArtifact={noopOpenArtifact}
      />
    );

    const toggle = screen.getByRole('button', { name: 'Show thinking' });
    const chevron = toggle.querySelector('svg')!;
    // Visible at rest — the whole point. A marker that only appears on hover
    // teaches nobody that the row opens.
    expect(chevron.getAttribute('class')).not.toMatch(/opacity-0/);
    expect(chevron.getAttribute('class')).not.toMatch(/rotate-90/);
    // 16px, the row's icon size — the tool-call chevron beside it is the same,
    // so the transcript never shows two chevron scales (§3.8b).
    expect(chevron.getAttribute('class')).toMatch(/size-4/);

    // The thought itself is not in the DOM until it is asked for.
    expect(screen.queryByText('weighing two options')).not.toBeInTheDocument();

    await userEvent.click(toggle);

    expect(screen.getByText('weighing two options')).toBeInTheDocument();
    expect(
      screen
        .getByRole('button', { name: 'Hide thinking' })
        .querySelector('svg')!
        .getAttribute('class')
    ).toMatch(/rotate-90/);
  });
});
