import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { COPY_FIELD_FEEDBACK_MS, CopyField } from './copy-field';

const MAIN_CSS = readFileSync(join(__dirname, '../../styles/main.css'), 'utf8');

const liveRegion = (container: HTMLElement) =>
  container.querySelector('[aria-live="polite"]') as HTMLElement;

/** The label the Copy button shows now; the other one only reserves its width. */
const shown = (button: HTMLElement) =>
  button.querySelector('[data-active="true"]')?.textContent ?? '';

async function press(button: HTMLElement) {
  await act(async () => {
    fireEvent.click(button);
  });
}

describe('CopyField', () => {
  let writeText: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    writeText = vi.spyOn(navigator.clipboard, 'writeText').mockResolvedValue(undefined);
  });

  afterEach(() => {
    writeText.mockRestore();
    window.getSelection()?.removeAllRanges();
    vi.useRealTimers();
  });

  it('shows the display form but copies the value', async () => {
    render(
      <CopyField value="7QK2M9XA3JTPWZ4D" display="7QK2-M9XA-3JTP-WZ4D" label="device code" />
    );
    expect(screen.getByText('7QK2-M9XA-3JTP-WZ4D')).toBeInTheDocument();
    await press(screen.getByRole('button', { name: 'Copy device code' }));
    expect(writeText).toHaveBeenCalledTimes(1);
    expect(writeText).toHaveBeenCalledWith('7QK2M9XA3JTPWZ4D');
  });

  it('swaps to a settled Copied for two seconds and announces it once, politely', async () => {
    vi.useFakeTimers();
    const onCopied = vi.fn();
    const { container } = render(
      <CopyField value="biorouter crew join …" label="invitation message" onCopied={onCopied} />
    );
    const button = screen.getByRole('button', { name: 'Copy invitation message' });
    expect(liveRegion(container)).toBeEmptyDOMElement();

    await press(button);
    expect(onCopied).toHaveBeenCalledTimes(1);
    expect(shown(button)).toBe('Copied');
    expect(button.querySelector('.biorouter-check-settled')).not.toBeNull();
    // The name never changes identity; the live region carries the outcome.
    expect(button).toHaveAccessibleName('Copy invitation message');
    expect(liveRegion(container)).toHaveTextContent('Copied');
    expect(liveRegion(container)).toHaveAttribute('aria-atomic', 'true');
    // No second, louder announcement: nothing here is an alert or a toast.
    expect(container.querySelector('[role="alert"]')).toBeNull();

    act(() => {
      vi.advanceTimersByTime(COPY_FIELD_FEEDBACK_MS - 1);
    });
    expect(shown(button)).toBe('Copied');
    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(shown(button)).toBe('Copy');
    expect(button.querySelector('.biorouter-check-settled')).toBeNull();
    expect(liveRegion(container)).toBeEmptyDOMElement();
  });

  // QA Q2-24: the pill grew by "Copied"'s width, and an 18-line invitation beside it re-wrapped.
  it('keeps the button as wide as its widest label, whichever is showing', async () => {
    render(<CopyField value="abc" label="command" />);
    const button = screen.getByRole('button', { name: 'Copy command' });
    const cells = Array.from(
      button.querySelectorAll<HTMLElement>('[data-slot="copy-field-labels"] > span')
    );
    expect(cells.map((cell) => cell.textContent)).toEqual(['Copy', 'Copied']);
    const stack = button.querySelector<HTMLElement>('[data-slot="copy-field-labels"]')!;
    expect(stack.style.display).toBe('inline-grid');
    // Both in the one cell; the inactive one laid out (so it holds the width) but unseen.
    for (const cell of cells) expect(cell.style.gridArea).toContain('1 / 1');
    expect(cells[0].style.visibility).toBe('');
    expect(cells[1].style.visibility).toBe('hidden');
    expect(cells[1]).toHaveAttribute('aria-hidden', 'true');

    await press(button);
    expect(cells[0].style.visibility).toBe('hidden');
    expect(cells[1].style.visibility).toBe('');
    expect(shown(button)).toBe('Copied');
  });

  it('restarts the two seconds when copied again', async () => {
    vi.useFakeTimers();
    render(<CopyField value="abc" label="command" />);
    const button = screen.getByRole('button', { name: 'Copy command' });
    await press(button);
    act(() => {
      vi.advanceTimersByTime(1500);
    });
    await press(button);
    act(() => {
      vi.advanceTimersByTime(1500);
    });
    expect(shown(button)).toBe('Copied');
    act(() => {
      vi.advanceTimersByTime(500);
    });
    expect(shown(button)).toBe('Copy');
  });

  it('on a clipboard failure says so and selects the value so ⌘C works', async () => {
    writeText.mockRejectedValueOnce(new Error('denied'));
    const onCopied = vi.fn();
    const { container } = render(
      <CopyField
        value="/home/alice/lab/raw/counts.csv"
        label="server path"
        truncate="middle"
        onCopied={onCopied}
      />
    );
    const button = screen.getByRole('button', { name: 'Copy server path' });
    await press(button);

    expect(shown(button)).toBe('Copy failed');
    expect(onCopied).not.toHaveBeenCalled();
    expect(liveRegion(container)).toHaveTextContent('Copy failed');
    // The whole value — both halves of the middle truncation — is selected.
    expect(window.getSelection()?.toString()).toBe('/home/alice/lab/raw/counts.csv');
  });

  it('turns a ⌘C of the whole grouped display into the value, and leaves a partial one alone', async () => {
    writeText.mockRejectedValueOnce(new Error('denied'));
    const { container } = render(
      <CopyField value="7QK2M9XA3JTPWZ4D" display="7QK2-M9XA-3JTP-WZ4D" label="device code" />
    );
    // The fallback selects the grouped form, with the Copy button focused.
    const button = screen.getByRole('button', { name: 'Copy device code' });
    button.focus();
    await press(button);
    expect(window.getSelection()?.toString()).toBe('7QK2-M9XA-3JTP-WZ4D');

    const setData = vi.fn();
    const whole = fireEvent.copy(button, { clipboardData: { setData } });
    expect(whole).toBe(false); // default prevented: the browser's own copy does not run
    expect(setData).toHaveBeenCalledWith('text/plain', '7QK2M9XA3JTPWZ4D');

    // Someone selecting part of the code by hand gets exactly that part.
    const valueNode = container.querySelector('.biorouter-copy-field-value') as HTMLElement;
    const range = document.createRange();
    range.setStart(valueNode.firstChild as Text, 0);
    range.setEnd(valueNode.firstChild as Text, 4);
    window.getSelection()?.removeAllRanges();
    window.getSelection()?.addRange(range);
    const partialSetData = vi.fn();
    const partial = fireEvent.copy(valueNode, { clipboardData: { setData: partialSetData } });
    expect(partial).toBe(true);
    expect(partialSetData).not.toHaveBeenCalled();
  });

  it('treats an absent clipboard as a failure, not a silent no-op', async () => {
    const original = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
    Object.defineProperty(navigator, 'clipboard', { value: undefined, configurable: true });
    try {
      render(<CopyField value="ssh-ed25519 AAAA" label="public key" />);
      const button = screen.getByRole('button', { name: 'Copy public key' });
      await press(button);
      expect(shown(button)).toBe('Copy failed');
      expect(window.getSelection()?.toString()).toBe('ssh-ed25519 AAAA');
    } finally {
      if (original) Object.defineProperty(navigator, 'clipboard', original);
    }
  });

  it('masks a secret, copies it while masked, and reveals it on request', async () => {
    const { container } = render(<CopyField value="tok_s3cret" label="legacy token" secret />);
    expect(container).not.toHaveTextContent('tok_s3cret');
    const reveal = screen.getByRole('button', { name: 'Show legacy token' });
    expect(reveal).toHaveAttribute('aria-pressed', 'false');

    await press(screen.getByRole('button', { name: 'Copy legacy token' }));
    expect(writeText).toHaveBeenCalledWith('tok_s3cret');
    expect(container).not.toHaveTextContent('tok_s3cret');

    await press(reveal);
    expect(screen.getByText('tok_s3cret')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Hide legacy token' })).toHaveAttribute(
      'aria-pressed',
      'true'
    );
  });

  it('reveals a masked secret before selecting it when the clipboard fails', async () => {
    writeText.mockRejectedValueOnce(new Error('denied'));
    render(<CopyField value="tok_s3cret" label="legacy token" secret />);
    await press(screen.getByRole('button', { name: 'Copy legacy token' }));
    // Selecting the mask would put bullets on the clipboard.
    expect(window.getSelection()?.toString()).toBe('tok_s3cret');
  });

  it('keeps the full value in the DOM when truncating, and marks the form', () => {
    const { container, rerender } = render(
      <CopyField value="/home/alice/lab/raw/counts.csv" label="server path" truncate="middle" />
    );
    const field = container.querySelector('[data-slot="copy-field"]') as HTMLElement;
    const value = field.querySelector('.biorouter-copy-field-value') as HTMLElement;
    expect(value).toHaveAttribute('data-truncate', 'middle');
    expect(value).toHaveTextContent('/home/alice/lab/raw/counts.csv');
    expect(value.querySelector('.biorouter-copy-field-tail')).toHaveTextContent(/^w\/counts\.csv$/);

    rerender(<CopyField value={'line one\nline two'} label="invitation message" multiline />);
    expect(field).toHaveAttribute('data-multiline', 'true');
    expect(field.querySelector('.biorouter-copy-field-value')).not.toHaveAttribute('data-truncate');

    rerender(<CopyField value="7QK2M9XA" label="device code" size="code" />);
    expect(field).toHaveAttribute('data-size', 'code');
  });

  /**
   * jsdom does not load main.css, so the one ground rule is asserted at the
   * source: both forms sit on the well; `--background-code` equals the page in
   * dark mode and would make the box vanish inside a dialog.
   */
  it('sits on --background-well in both forms, never --background-code', () => {
    const start = MAIN_CSS.indexOf('/* CopyField');
    expect(start).toBeGreaterThanOrEqual(0);
    const block = MAIN_CSS.slice(start, MAIN_CSS.indexOf('/* StatusDot', start));
    expect(block).toMatch(
      /\.biorouter-copy-field \{[^}]*background-color: var\(--background-well\);/
    );
    expect(block).not.toMatch(/background(?:-color)?:[^;]*--background-code/);
    expect(block).toMatch(/\.biorouter-copy-field\[data-multiline='true'\]/);
  });
});
