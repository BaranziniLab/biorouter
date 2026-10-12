import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { CommandChip, ResourceRefChip, ResourceRefText } from './ResourceRefChip';
import { labelledRefTag, refTag } from '../utils/resourceRefs';

const span = (kind: 'skill' | 'extension' | 'knowledge_base', value: string, label?: string) => ({
  kind,
  value,
  label,
  start: 0,
  end: 0,
  raw: '',
});

describe('ResourceRefChip', () => {
  it('shows the resource name, never the markup', () => {
    render(<ResourceRefChip refSpan={span('skill', 'single-cell "QC" & prep <v2>')} />);

    expect(screen.getByText('single-cell "QC" & prep <v2>')).toBeInTheDocument();
    expect(document.body.textContent).not.toContain('biorouter-ref');
    expect(document.body.textContent).not.toContain('&quot;');
  });

  it('prefers the display label when the tag carries one', () => {
    render(<ResourceRefChip refSpan={span('knowledge_base', 'soul', 'Soul & Body')} />);

    expect(screen.getByText('Soul & Body')).toBeInTheDocument();
    // The id is the machine identity; it stays available for disambiguation but
    // is not what the chip reads as.
    expect(screen.queryByText('soul')).not.toBeInTheDocument();
  });

  it('names the kind so the three are distinguishable', () => {
    const { unmount } = render(<ResourceRefChip refSpan={span('skill', 'rna-qc')} />);
    expect(screen.getByTestId('resource-ref-chip')).toHaveAttribute('data-ref-kind', 'skill');
    expect(screen.getByTestId('resource-ref-chip').title).toContain('Skill');
    unmount();

    render(<ResourceRefChip refSpan={span('extension', 'Chat Recall')} />);
    expect(screen.getByTestId('resource-ref-chip')).toHaveAttribute('data-ref-kind', 'extension');
    expect(screen.getByTestId('resource-ref-chip').title).toContain('Extension');
  });

  it('carries an icon for the kind', () => {
    render(<ResourceRefChip refSpan={span('knowledge_base', 'soul')} />);
    expect(screen.getByTestId('resource-ref-chip').querySelector('svg')).not.toBeNull();
  });

  // The glyph is the whole visual signal for the kind, so a screen reader that
  // never sees it would hear only "rna-qc" and not what was attached.
  it('says the kind in words for a screen reader', () => {
    render(<ResourceRefChip refSpan={span('skill', 'rna-qc')} />);

    const chip = screen.getByTestId('resource-ref-chip');
    expect(chip.querySelector('.sr-only')?.textContent).toContain('Skill');
  });

  // A flex/grid item's `min-width: auto` lets a long unbroken name keep the item
  // at full-token width and bleed past the bubble; the chip has to be able to
  // shrink and clip. jsdom computes no layout, so this asserts the class
  // contract that the browser sweep then confirms visually.
  it('can shrink and clip a name long enough to break the layout', () => {
    const long = 'single-cell-rna-sequencing-quality-control-and-doublet-removal-pipeline-v2';
    render(<ResourceRefChip refSpan={span('skill', long)} />);

    const chip = screen.getByTestId('resource-ref-chip');
    expect(chip.className).toContain('max-w-full');
    expect(chip.className).toContain('min-w-0');

    const label = screen.getByText(long);
    expect(label.className).toContain('truncate');
    expect(label.className).toContain('min-w-0');
    // Clipping hides characters, so the whole name stays reachable on hover.
    expect(chip.title).toContain(long);
  });

  // Found in a browser, not here: accent ink on the accent tone's 12% fill
  // measured 3.08:1 in `alma-mater:light` inside a user bubble — under AA for
  // 11px text — while every existing gate stayed green, because no token names
  // the composited colour and jsdom neither applies Tailwind nor blends alpha.
  // The accent now carries the fill and the glyph (affordances, 3:1); the label
  // uses the ink the audit guarantees on every ground. The ratios themselves
  // are gated by `scripts/check-contrast.mjs`; this stops the component from
  // drifting back while that gate stays green.
  it('reads the name in the default ink, not the accent tone', () => {
    render(<ResourceRefChip refSpan={span('skill', 'rna-qc')} onRemove={() => {}} />);

    expect(screen.getByTestId('resource-ref-chip-name').className).toContain('text-text-default');
    expect(screen.getByTestId('resource-ref-chip-name').className).not.toContain(
      'text-text-accent'
    );
    expect(screen.getByRole('button', { name: /remove/i }).className).not.toContain(
      'text-text-accent'
    );
  });

  it('offers no remove control unless the caller can act on it', () => {
    const { unmount } = render(<ResourceRefChip refSpan={span('skill', 'rna-qc')} />);
    expect(screen.queryByRole('button')).not.toBeInTheDocument();
    unmount();

    const onRemove = vi.fn();
    render(<ResourceRefChip refSpan={span('skill', 'rna-qc')} onRemove={onRemove} />);
    expect(screen.getByRole('button', { name: /remove/i })).toBeInTheDocument();
  });

  it('removes on click', async () => {
    const onRemove = vi.fn();
    render(<ResourceRefChip refSpan={span('skill', 'rna-qc')} onRemove={onRemove} />);

    await userEvent.click(screen.getByRole('button', { name: /remove/i }));
    expect(onRemove).toHaveBeenCalledTimes(1);
  });
});

describe('ResourceRefText', () => {
  it('draws a chip where a tag was and keeps the prose around it', () => {
    render(<ResourceRefText text={`please run ${refTag('skill', 'my skill')} on this`} />);

    expect(screen.getByTestId('resource-ref-chip')).toHaveTextContent('my skill');
    expect(document.body.textContent).toContain('please run');
    expect(document.body.textContent).toContain('on this');
    expect(document.body.textContent).not.toContain('biorouter-ref');
  });

  it('reads a knowledge base by its label', () => {
    render(<ResourceRefText text={labelledRefTag('knowledge_base', 'soul', 'Soul & Body')} />);
    expect(screen.getByTestId('resource-ref-chip')).toHaveTextContent('Soul & Body');
  });

  it('draws one chip per tag, in order', () => {
    render(
      <ResourceRefText text={`${refTag('skill', 'first')} and ${refTag('extension', 'second')}`} />
    );

    const names = screen.getAllByTestId('resource-ref-chip-name');
    expect(names.map((name) => name.textContent)).toEqual(['first', 'second']);
  });

  // Degrading to readable text is the requirement: a tag this build cannot parse
  // must never render as a blank or take the message down with it.
  it('leaves a tag it cannot parse as visible text', () => {
    const broken = `<biorouter-ref type="skill" name="never closed`;
    render(<ResourceRefText text={`before ${broken}`} />);

    expect(screen.queryByTestId('resource-ref-chip')).not.toBeInTheDocument();
    expect(document.body.textContent).toContain(broken);
  });

  it('passes plain text through unchanged', () => {
    render(<ResourceRefText text={'line one\nline two'} />);
    expect(document.body.textContent).toBe('line one\nline two');
  });
});

describe('ResourceRefText with a leading chip command', () => {
  it('draws /bug as the command chip and keeps the prose and references', () => {
    render(<ResourceRefText text={`/bug the panel is blank ${refTag('skill', 'rna-qc')}`} />);

    expect(screen.getByTestId('command-chip')).toHaveAttribute('data-command', 'bug');
    expect(screen.getByTestId('resource-ref-chip-name')).toHaveTextContent('rna-qc');
    expect(document.body.textContent).toContain('the panel is blank');
    // The typed command is gone from the prose; only the chip (and its words for
    // a screen reader) says it.
    const outsideChip = document.body.textContent!.replace(
      screen.getByTestId('command-chip').textContent!,
      ''
    );
    expect(outsideChip).not.toContain('/bug');
    expect(document.body.textContent).not.toContain('biorouter-ref');
  });

  it('draws the command chip before anything else', () => {
    const { container } = render(<ResourceRefText text="/bug x" />);
    expect(container.firstElementChild).toBe(screen.getByTestId('command-chip'));
  });

  it('says what the chip is to a screen reader', () => {
    render(<ResourceRefText text="/bug" />);
    expect(screen.getByTestId('command-chip').querySelector('.sr-only')?.textContent).toContain(
      '/bug'
    );
  });

  it('leaves text that only looks like the command alone', () => {
    for (const text of ['/bugs', '/Bug x', 'see /bug', ' /bug x']) {
      const { unmount } = render(<ResourceRefText text={text} />);
      expect(screen.queryByTestId('command-chip'), text).not.toBeInTheDocument();
      unmount();
    }
  });
});

describe('CommandChip', () => {
  it('offers a remove control only when asked to', async () => {
    const onRemove = vi.fn();
    const { rerender } = render(<CommandChip command="bug" />);
    expect(screen.queryByRole('button')).not.toBeInTheDocument();

    rerender(<CommandChip command="bug" onRemove={onRemove} />);
    await userEvent.click(screen.getByRole('button', { name: 'Remove command /bug' }));
    expect(onRemove).toHaveBeenCalledTimes(1);
  });

  it('wears the reference chip look', () => {
    render(<CommandChip command="bug" />);
    const chip = screen.getByTestId('command-chip');
    expect(chip.querySelector('svg')).not.toBeNull();
    expect(chip.className).toContain('max-w-full');
    expect(screen.getByTestId('command-chip-name').className).toContain('text-text-default');
  });
});
