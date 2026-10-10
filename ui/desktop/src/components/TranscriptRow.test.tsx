import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { TranscriptRow, TranscriptRowSection } from './TranscriptRow';
import { Brain, Check } from './icons/app-icons';

describe('TranscriptRow', () => {
  it('draws a static line when it has no body: no button, no chevron', () => {
    const { container } = render(<TranscriptRow icon={Check} label="Information sent" />);
    expect(screen.queryByRole('button')).toBeNull();
    expect(container.querySelector('.br-transcript-row-chevron')).toBeNull();
    expect(screen.getByText('Information sent')).toHaveClass('br-transcript-row-label');
  });

  it('opens and closes its body from the trigger, unmounting it while closed', async () => {
    const user = userEvent.setup();
    render(
      <TranscriptRow icon={Brain} label="Thought" meta="· 8s">
        <p>Body text</p>
      </TranscriptRow>
    );
    const trigger = screen.getByRole('button', { name: /Thought/ });
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByText('Body text')).toBeNull();

    await user.click(trigger);
    expect(trigger).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByText('Body text')).toBeInTheDocument();
    expect(screen.getByText('Body text').closest('.br-transcript-row-well')).not.toBeNull();
  });

  it('starts open on defaultOpen and reports changes', async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    render(
      <TranscriptRow icon={Brain} label="Ran ls" defaultOpen onOpenChange={onOpenChange}>
        <p>Output</p>
      </TranscriptRow>
    );
    expect(screen.getByText('Output')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: /Ran ls/ }));
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it('indents a plain body without a well', async () => {
    const user = userEvent.setup();
    render(
      <TranscriptRow icon={Brain} label="Thought" body="plain">
        <p>Prose</p>
      </TranscriptRow>
    );
    await user.click(screen.getByRole('button', { name: /Thought/ }));
    const prose = screen.getByText('Prose');
    expect(prose.closest('.br-transcript-row-body')).not.toBeNull();
    expect(prose.closest('.br-transcript-row-well')).toBeNull();
  });

  it('stamps the glyph kind, the running state, the tone and a screen-reader status', () => {
    const { container } = render(
      <TranscriptRow
        icon={Check}
        glyph="shell"
        label="Running ls"
        running
        tone="danger"
        statusLabel="Tool status: loading"
      />
    );
    const root = container.querySelector('.br-transcript-row')!;
    expect(root).toHaveAttribute('data-running');
    expect(root).toHaveAttribute('data-tone', 'danger');
    expect(container.querySelector('[data-tool-glyph="shell"]')).not.toBeNull();
    expect(screen.getByRole('img', { name: 'Tool status: loading' })).toHaveClass('sr-only');
  });

  it('keeps trailing content outside the trigger so it may be a control', () => {
    render(
      <TranscriptRow icon={Check} label="Ran ls" trailing={<button type="button">About</button>}>
        <p>Body</p>
      </TranscriptRow>
    );
    const about = screen.getByRole('button', { name: 'About' });
    expect(about.closest('.br-transcript-row-head')).toBeNull();
  });

  it('labels a well section in supporting type', () => {
    render(
      <TranscriptRowSection label="Output">
        <pre>ok</pre>
      </TranscriptRowSection>
    );
    expect(screen.getByText('Output')).toHaveClass('br-transcript-row-section-label');
  });
});
