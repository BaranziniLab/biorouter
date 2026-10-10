import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import { ProvenanceChip } from './ProvenanceChip';

describe('ProvenanceChip', () => {
  it('labels agent injections with the source session name', () => {
    render(
      <ProvenanceChip
        provenance={{
          kind: 'agent_injection',
          fromSessionId: 's1',
          fromSessionName: 'Planning chat',
        }}
      />
    );
    expect(screen.getByText(/injected by Planning chat/i)).toBeTruthy();
  });

  it('falls back to the source session id when it has no name', () => {
    render(<ProvenanceChip provenance={{ kind: 'agent_injection', fromSessionId: 's1' }} />);
    expect(screen.getByText(/injected by s1/i)).toBeTruthy();
  });

  it('still names an origin when neither id nor name survived', () => {
    render(<ProvenanceChip provenance={{ kind: 'agent_injection' }} />);
    expect(screen.getByText(/injected by another agent/i)).toBeTruthy();
  });

  it('labels the context a spawning session handed down', () => {
    render(<ProvenanceChip provenance={{ kind: 'spawn_context', fromSessionId: 's-parent' }} />);
    expect(screen.getByText(/spawn context/i)).toBeTruthy();
  });

  it('labels direct human input into subagent tabs', () => {
    render(<ProvenanceChip provenance={{ kind: 'user_direct' }} />);
    expect(screen.getByText(/direct user message/i)).toBeTruthy();
  });

  it('clips a long injected session name and keeps it reachable on hover and Tab', async () => {
    // The name is chosen by *another* session, so its length is not ours to
    // assume. jsdom applies no CSS, so this guard is necessarily structural:
    // the chip must cap its own width and clip the label instead of widening
    // the transcript row (the repo's recurring min-w-0 / overflow bug class).
    const longName = `Planning ${'chat '.repeat(40)}`.trim();
    render(
      <ProvenanceChip
        provenance={{ kind: 'agent_injection', fromSessionId: 's1', fromSessionName: longName }}
      />
    );

    const chip = document.querySelector('[data-provenance-kind]') as HTMLElement;
    expect(chip.className).toMatch(/\bmax-w-/);
    expect(chip.className).toMatch(/\bmin-w-0\b/);

    const label = screen.getByText(new RegExp(`injected by ${longName}`, 'i'));
    expect(label.className).toMatch(/\btruncate\b/);

    // Clipped text must still be readable somewhere, and the id must survive:
    // in the badge's tooltip, which Tab reaches as well as hover, never a `title`.
    expect(chip).not.toHaveAttribute('title');
    expect(chip).toHaveAttribute('tabindex', '0');
    await userEvent.setup().tab();
    expect(chip).toHaveFocus();
    expect(await screen.findByRole('tooltip')).toHaveTextContent(`(s1)`);
  });

  it('renders nothing without provenance', () => {
    const { container } = render(<ProvenanceChip provenance={undefined} />);
    expect(container.firstChild).toBeNull();
  });
});
