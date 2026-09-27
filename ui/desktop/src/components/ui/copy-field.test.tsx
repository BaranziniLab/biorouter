import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  COPY_FIELD_CLAMP_CHARS,
  COPY_FIELD_CLAMP_NOTE,
  COPY_FIELD_FEEDBACK_MS,
  COPY_FIELD_RETRY_DELAY_MS,
  CopyField,
} from './copy-field';
import { inviteCopy } from '../crew/dialogs/copy';
import { INSTALL_COMMANDS } from '../crew/onboarding/copy';
import { hostStartCommands, workspaceSlug } from '../crew/onboarding/joinText';

const MAIN_CSS = readFileSync(join(__dirname, '../../styles/main.css'), 'utf8');

const liveRegion = (container: HTMLElement) =>
  container.querySelector('[aria-live="polite"]') as HTMLElement;

/** The label the Copy button shows now; the others only reserve their width. */
const shown = (button: HTMLElement) =>
  button.querySelector('[data-active="true"]')?.textContent ?? '';

async function press(button: HTMLElement) {
  await act(async () => {
    fireEvent.click(button);
  });
}

/**
 * A click whose first write was refused: the retry waits `COPY_FIELD_RETRY_DELAY_MS` on a real
 * timer before the outcome lands, so wait it out (plus a margin) inside `act`.
 */
async function pressThroughRetry(button: HTMLElement) {
  await press(button);
  await act(() => new Promise((resolve) => setTimeout(resolve, COPY_FIELD_RETRY_DELAY_MS + 30)));
}

const refused = () => new Error('denied');

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
  // The review then found "Copy failed", the widest label, still widening it.
  it('keeps the button as wide as its widest label, whichever is showing', async () => {
    writeText
      .mockResolvedValueOnce(undefined)
      .mockRejectedValueOnce(refused())
      .mockRejectedValueOnce(refused());
    render(<CopyField value="abc" label="command" />);
    const button = screen.getByRole('button', { name: 'Copy command' });
    const cells = Array.from(
      button.querySelectorAll<HTMLElement>('[data-slot="copy-field-labels"] > span')
    );
    expect(cells.map((cell) => cell.textContent)).toEqual(['Copy', 'Copied', 'Copy failed']);
    const stack = button.querySelector<HTMLElement>('[data-slot="copy-field-labels"]')!;
    expect(stack.style.display).toBe('inline-grid');
    // All in the one cell; the inactive ones laid out (so they hold the width) but unseen.
    for (const cell of cells) expect(cell.style.gridArea).toContain('1 / 1');
    const visible = () => cells.map((cell) => cell.style.visibility !== 'hidden');
    const hiddenFromReaders = () =>
      cells.map((cell) => cell.getAttribute('aria-hidden') === 'true');
    expect(visible()).toEqual([true, false, false]);
    expect(hiddenFromReaders()).toEqual([false, true, true]);

    await press(button);
    expect(visible()).toEqual([false, true, false]);
    expect(shown(button)).toBe('Copied');

    await pressThroughRetry(button);
    expect(visible()).toEqual([false, false, true]);
    expect(hiddenFromReaders()).toEqual([true, true, false]);
    expect(shown(button)).toBe('Copy failed');
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
    // Refused twice, and jsdom has no `document.execCommand`: every path has failed.
    writeText.mockRejectedValue(refused());
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
    await pressThroughRetry(button);

    expect(writeText).toHaveBeenCalledTimes(2);
    expect(shown(button)).toBe('Copy failed');
    expect(onCopied).not.toHaveBeenCalled();
    expect(liveRegion(container)).toHaveTextContent('Copy failed');
    // The whole value — both halves of the middle truncation — is selected.
    expect(window.getSelection()?.toString()).toBe('/home/alice/lab/raw/counts.csv');
  });

  it('turns a ⌘C of the whole grouped display into the value, and leaves a partial one alone', async () => {
    writeText.mockRejectedValue(refused());
    const { container } = render(
      <CopyField value="7QK2M9XA3JTPWZ4D" display="7QK2-M9XA-3JTP-WZ4D" label="device code" />
    );
    // The fallback selects the grouped form, with the Copy button focused.
    const button = screen.getByRole('button', { name: 'Copy device code' });
    button.focus();
    await pressThroughRetry(button);
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
      await pressThroughRetry(button);
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
    writeText.mockRejectedValue(refused());
    render(<CopyField value="tok_s3cret" label="legacy token" secret />);
    await pressThroughRetry(screen.getByRole('button', { name: 'Copy legacy token' }));
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
   * QA Q3-41. Keys and security's first Copy said "Copy failed" once and then worked three
   * times in a row: `navigator.clipboard.writeText` refuses a document without focus, which is
   * rarely lasting. A refusal now focuses the window, waits a beat and retries once, then tries
   * the document's own copy on a hidden selection, and only then says it failed.
   */
  describe('when the clipboard refuses (Q3-41)', () => {
    let focus: ReturnType<typeof vi.spyOn>;
    const originalExecCommand = Object.getOwnPropertyDescriptor(document, 'execCommand');

    beforeEach(() => {
      focus = vi.spyOn(window, 'focus').mockImplementation(() => {});
    });

    afterEach(() => {
      focus.mockRestore();
      if (originalExecCommand) Object.defineProperty(document, 'execCommand', originalExecCommand);
      else delete (document as { execCommand?: unknown }).execCommand;
    });

    const installExecCommand = (impl: (command: string) => boolean) => {
      const execCommand = vi.fn(impl);
      Object.defineProperty(document, 'execCommand', { value: execCommand, configurable: true });
      return execCommand;
    };

    it('focuses the window and retries once before anything else', async () => {
      writeText.mockRejectedValueOnce(refused());
      const execCommand = installExecCommand(() => true);
      const onCopied = vi.fn();
      const { container } = render(
        <CopyField
          value="6BC5D3F4014ED272"
          display="6BC5 D3F4 014E D272"
          label="device key"
          onCopied={onCopied}
        />
      );
      const button = screen.getByRole('button', { name: 'Copy device key' });

      await press(button);
      // Not "Copy failed" while the retry is still pending.
      expect(shown(button)).toBe('Copy');
      await pressThroughRetry(button);

      expect(focus).toHaveBeenCalled();
      expect(writeText).toHaveBeenNthCalledWith(1, '6BC5D3F4014ED272');
      expect(writeText).toHaveBeenNthCalledWith(2, '6BC5D3F4014ED272');
      expect(execCommand).not.toHaveBeenCalled();
      expect(shown(button)).toBe('Copied');
      expect(liveRegion(container)).toHaveTextContent('Copied');
      expect(onCopied).toHaveBeenCalled();
    });

    it('waits the retry delay, not less, before the second write', async () => {
      vi.useFakeTimers();
      writeText.mockRejectedValueOnce(refused());
      render(<CopyField value="abc" label="command" />);
      await press(screen.getByRole('button', { name: 'Copy command' }));
      expect(writeText).toHaveBeenCalledTimes(1);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(COPY_FIELD_RETRY_DELAY_MS - 1);
      });
      expect(writeText).toHaveBeenCalledTimes(1);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(1);
      });
      expect(writeText).toHaveBeenCalledTimes(2);
    });

    it('falls back to the document’s copy of a hidden selection, inside the field', async () => {
      writeText.mockRejectedValue(refused());
      const { container } = render(<CopyField value="tok_s3cret" label="legacy token" secret />);
      const field = container.querySelector('[data-slot="copy-field"]') as HTMLElement;
      const button = screen.getByRole('button', { name: 'Copy legacy token' });
      act(() => button.focus());
      let copiedFrom: {
        text: string;
        selected: string;
        inField: boolean;
        focused: boolean;
      } | null = null;
      const execCommand = installExecCommand((command) => {
        const area = field.querySelector<HTMLTextAreaElement>('[data-slot="copy-field-fallback"]');
        copiedFrom = area
          ? {
              text: area.value,
              selected: area.value.slice(area.selectionStart, area.selectionEnd),
              inField: field.contains(area),
              focused: document.activeElement === area,
            }
          : null;
        return command === 'copy';
      });

      await pressThroughRetry(button);

      expect(writeText).toHaveBeenCalledTimes(2);
      expect(execCommand).toHaveBeenCalledWith('copy');
      // The VALUE, never the mask, selected whole in a focused textarea inside the field (a
      // dialog's focus trap would pull focus back out of <body>).
      expect(copiedFrom).toEqual({
        text: 'tok_s3cret',
        selected: 'tok_s3cret',
        inField: true,
        focused: true,
      });
      expect(shown(button)).toBe('Copied');
      // It leaves nothing behind, gives focus back, and did not reveal the secret.
      expect(field.querySelector('[data-slot="copy-field-fallback"]')).toBeNull();
      expect(button).toHaveFocus();
      expect(container).not.toHaveTextContent('tok_s3cret');
    });

    it('says "Copy failed" only when the retry and the fallback have both refused', async () => {
      writeText.mockRejectedValue(refused());
      const execCommand = installExecCommand(() => false);
      const { container } = render(<CopyField value="abc" label="command" />);
      const button = screen.getByRole('button', { name: 'Copy command' });

      await pressThroughRetry(button);

      expect(writeText).toHaveBeenCalledTimes(2);
      expect(execCommand).toHaveBeenCalledWith('copy');
      expect(shown(button)).toBe('Copy failed');
      expect(liveRegion(container)).toHaveTextContent('Copy failed');
      expect(window.getSelection()?.toString()).toBe('abc');
      expect(container.querySelector('[data-slot="copy-field-fallback"]')).toBeNull();
    });

    it('settles nothing on a field that closed while the retry was waiting', async () => {
      writeText.mockRejectedValueOnce(refused());
      const onCopied = vi.fn();
      const { unmount } = render(<CopyField value="abc" label="command" onCopied={onCopied} />);
      await press(screen.getByRole('button', { name: 'Copy command' }));
      unmount();
      await act(
        () => new Promise((resolve) => setTimeout(resolve, COPY_FIELD_RETRY_DELAY_MS + 30))
      );
      // The write still happened (the person asked for it); the gone field just says nothing.
      expect(writeText).toHaveBeenCalledTimes(2);
      expect(onCopied).not.toHaveBeenCalled();
    });
  });

  /**
   * QA Q3-18. The host's start commands scroll sideways (one per line, never wrapped mid-flag),
   * and macOS hides an idle scrollbar, so they looked cut off mid-path. The field measures the
   * value and says so: `data-overflow="true"` while more lies to the right, `"end"` once it is
   * scrolled there. jsdom has no layout, so the widths are stood in for and the ResizeObserver is
   * driven by hand; the fade and the scrollbar are asserted at the source below.
   */
  describe('a multi-line value wider than its box (Q3-18)', () => {
    type ObserverCallback = ConstructorParameters<typeof ResizeObserver>[0];
    let observers: Array<{ callback: ObserverCallback; targets: Element[] }>;
    const original = globalThis.ResizeObserver;

    beforeEach(() => {
      observers = [];
      globalThis.ResizeObserver = class {
        private readonly entry: { callback: ObserverCallback; targets: Element[] };
        constructor(callback: ObserverCallback) {
          this.entry = { callback, targets: [] };
          observers.push(this.entry);
        }
        observe(target: Element) {
          this.entry.targets.push(target);
        }
        unobserve() {}
        disconnect() {
          this.entry.targets = [];
        }
      } as unknown as typeof ResizeObserver;
    });

    afterEach(() => {
      globalThis.ResizeObserver = original;
    });

    const size = (node: HTMLElement, box: { scrollWidth: number; clientWidth: number }) => {
      Object.defineProperty(node, 'scrollWidth', { value: box.scrollWidth, configurable: true });
      Object.defineProperty(node, 'clientWidth', { value: box.clientWidth, configurable: true });
    };
    const resize = () =>
      act(() => {
        for (const { callback, targets } of observers) {
          if (targets.length) callback([], {} as ResizeObserver);
        }
      });

    it('marks the overflow, and clears the fade once scrolled to the end', () => {
      const commands = hostStartCommands('lab', 'k'.repeat(64));
      const { container } = render(<CopyField multiline value={commands} label="commands" />);
      const field = container.querySelector('[data-slot="copy-field"]') as HTMLElement;
      const value = field.querySelector('.biorouter-copy-field-value') as HTMLElement;
      expect(field).not.toHaveAttribute('data-overflow');
      expect(observers.some((o) => o.targets.includes(value))).toBe(true);

      size(value, { scrollWidth: 900, clientWidth: 400 });
      resize();
      expect(field).toHaveAttribute('data-overflow', 'true');

      value.scrollLeft = 250;
      fireEvent.scroll(value);
      expect(field).toHaveAttribute('data-overflow', 'true');

      value.scrollLeft = 500;
      fireEvent.scroll(value);
      expect(field).toHaveAttribute('data-overflow', 'end');

      // The dialog widened until everything fits: no cue at all.
      size(value, { scrollWidth: 400, clientWidth: 400 });
      resize();
      expect(field).not.toHaveAttribute('data-overflow');
    });

    it('never marks a single-line value, which wraps or truncates instead', () => {
      const { container } = render(<CopyField value={'x'.repeat(200)} label="token" />);
      const field = container.querySelector('[data-slot="copy-field"]') as HTMLElement;
      const value = field.querySelector('.biorouter-copy-field-value') as HTMLElement;
      size(value, { scrollWidth: 900, clientWidth: 400 });
      resize();
      fireEvent.scroll(value);
      expect(field).not.toHaveAttribute('data-overflow');
      expect(observers.every((o) => !o.targets.includes(value))).toBe(true);
    });

    it('stops observing when it unmounts', () => {
      const { container, unmount } = render(<CopyField multiline value="a\nb" label="commands" />);
      const value = container.querySelector('.biorouter-copy-field-value') as HTMLElement;
      expect(observers.some((o) => o.targets.includes(value))).toBe(true);
      unmount();
      expect(observers.every((o) => o.targets.length === 0)).toBe(true);
    });
  });

  /**
   * QA Q3-37. The Invite dialog showed a 724-character invitation — 4 lines of instructions and
   * 16 of base64 — as the largest thing in the dialog. A multi-line value over
   * `COPY_FIELD_CLAMP_CHARS` now shows four lines behind a fade, says the whole message is copied,
   * and offers Show all. No prop: callers do not change.
   */
  describe('a long multi-line value folds (Q3-37)', () => {
    const invitation = [
      'Join chen-lab on Biorouter Crew.',
      'In Biorouter, open Crew, choose Join a workspace, and paste this whole message.',
      'It expires in 7 days and works once.',
      '',
      `brcrew1:${'eyJ2IjoxLCJ3Ijoi'.repeat(40)}`,
    ].join('\n');

    it('shows four lines, says Copy takes all of it, and offers the rest', async () => {
      expect(invitation.length).toBeGreaterThan(COPY_FIELD_CLAMP_CHARS);
      const { container } = render(
        <CopyField value={invitation} label="invitation message" multiline />
      );
      const field = container.querySelector('[data-slot="copy-field"]') as HTMLElement;
      const value = field.querySelector('.biorouter-copy-field-value') as HTMLElement;
      expect(field).toHaveAttribute('data-clamp', 'collapsed');
      expect(screen.getByText(COPY_FIELD_CLAMP_NOTE)).toBeInTheDocument();
      // The whole value is still there: for Copy, for a click-select and ⌘C, for a screen reader.
      expect(value.textContent).toBe(invitation);

      const copyButton = screen.getByRole('button', { name: 'Copy invitation message' });
      expect(copyButton).toHaveAccessibleDescription(COPY_FIELD_CLAMP_NOTE);
      await press(copyButton);
      expect(writeText).toHaveBeenCalledWith(invitation);

      const toggle = screen.getByRole('button', { name: 'Show all of the invitation message' });
      expect(toggle).toHaveTextContent('Show all');
      expect(toggle).toHaveAttribute('aria-expanded', 'false');
      expect(toggle).toHaveAttribute('aria-controls', value.id);

      fireEvent.click(toggle);
      expect(field).toHaveAttribute('data-clamp', 'expanded');
      expect(screen.queryByText(COPY_FIELD_CLAMP_NOTE)).toBeNull();
      const less = screen.getByRole('button', { name: 'Show less of the invitation message' });
      expect(less).toHaveAttribute('aria-expanded', 'true');
      expect(copyButton).not.toHaveAttribute('aria-describedby');

      fireEvent.click(less);
      expect(field).toHaveAttribute('data-clamp', 'collapsed');
    });

    it('leaves a short multi-line value, a single-line value and a masked secret alone', () => {
      const { container, rerender } = render(
        <CopyField value={'a'.repeat(COPY_FIELD_CLAMP_CHARS)} label="note" multiline />
      );
      const field = () => container.querySelector('[data-slot="copy-field"]') as HTMLElement;
      expect(field()).not.toHaveAttribute('data-clamp');

      rerender(<CopyField value={invitation} label="invitation message" />);
      expect(field()).not.toHaveAttribute('data-clamp');

      rerender(<CopyField value={invitation} label="invitation message" multiline secret />);
      expect(field()).not.toHaveAttribute('data-clamp');
      expect(screen.queryByRole('button', { name: /^Show all/ })).toBeNull();
    });

    // The callers the triage named: the invitation folds, the commands someone reviews do not.
    it('folds the invitation but never the host’s start commands or the install commands', () => {
      const longestSlug = workspaceSlug('w'.repeat(80));
      expect(longestSlug).toHaveLength(40);
      const start = hostStartCommands(longestSlug, 'f'.repeat(64));
      for (const commands of [start, INSTALL_COMMANDS, inviteCopy.installCommands]) {
        expect(Array.from(commands).length).toBeLessThanOrEqual(COPY_FIELD_CLAMP_CHARS);
        const { container, unmount } = render(
          <CopyField value={commands} label="commands" multiline />
        );
        expect(container.querySelector('[data-slot="copy-field"]')).not.toHaveAttribute(
          'data-clamp'
        );
        unmount();
      }
    });
  });

  /**
   * jsdom has no layout, no masks and no scrollbars, so the cues the three behaviours above hang
   * on are asserted where they live.
   */
  it('draws the overflow fade, a visible scrollbar and the fold in main.css', () => {
    const start = MAIN_CSS.indexOf('/* CopyField');
    const block = MAIN_CSS.slice(start, MAIN_CSS.indexOf('/* StatusDot', start));
    const rule = (selector: string) => {
      const at = block.indexOf(`${selector} {`);
      expect(at, selector).toBeGreaterThanOrEqual(0);
      return block.slice(at, block.indexOf('}', at)).replace(/\s+/g, ' ');
    };
    const valueSel = '.biorouter-copy-field-value';
    // The fade is a mask (correct on every ground), only while more lies to the right.
    const fade = rule(
      `.biorouter-copy-field[data-multiline='true'][data-overflow='true'] ${valueSel}`
    );
    expect(fade).toMatch(
      /(^| )mask-image: linear-gradient\(to right, #000 calc\(100% - 32px\), transparent\)/
    );
    expect(fade).toContain('-webkit-mask-image:');
    // The standard scrollbar properties go back to `auto`, or Chromium ignores the pseudo-elements
    // and draws macOS's overlay bar, invisible at rest.
    const bar = rule(`.biorouter-copy-field[data-multiline='true'][data-overflow] ${valueSel}`);
    expect(bar).toContain('scrollbar-width: auto');
    expect(bar).toContain('scrollbar-color: auto');
    expect(block).toMatch(
      /\[data-overflow\]\s+\.biorouter-copy-field-value::-webkit-scrollbar \{[^}]*height: 6px/
    );
    expect(block).toMatch(
      /\[data-overflow\]\s+\.biorouter-copy-field-value::-webkit-scrollbar-thumb \{[^}]*background-color: var\(--border-strong\)/
    );
    // Four code lines plus the value's block padding; vertical overflow only, so a command box's
    // own `overflow-x: auto` survives.
    const fold = rule(`.biorouter-copy-field[data-clamp='collapsed'] ${valueSel}`);
    expect(fold).toContain('max-height: calc(4 * var(--text-code--line-height) + 8px)');
    expect(fold).toContain('overflow-y: hidden');
    expect(fold).not.toMatch(/(^| )overflow: /);
    expect(fold).toMatch(/(^| )mask-image: linear-gradient\( to bottom/);
    expect(rule('.biorouter-copy-field[data-clamp]')).toContain(
      'grid-template-columns: minmax(0, 1fr) auto'
    );
    expect(rule('.biorouter-copy-field-footer')).toContain('grid-column: 1 / -1');
    expect(rule('.biorouter-copy-field-note')).toContain('color: var(--text-muted)');
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
