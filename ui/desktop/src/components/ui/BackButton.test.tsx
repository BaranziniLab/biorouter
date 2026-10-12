import { act, fireEvent, render, renderHook, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import BackButton, { useMouseBackButton } from './BackButton';

/**
 * The mouse's back key leaves the page. The listener lived in `BackButton` only, so a page whose
 * Back is another control (a `PageHeader`'s `onBack`) lost the key (V4-14); the hook is the one
 * place it lives now, and the button calls it itself.
 */
type Listener = () => void;

describe('useMouseBackButton', () => {
  let dispose: ReturnType<typeof vi.fn>;
  let commandListener: Listener | undefined;
  let on: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    dispose = vi.fn();
    commandListener = undefined;
    on = vi.fn((_channel: string, listener: Listener) => {
      commandListener = listener;
      return dispose;
    });
    Object.defineProperty(window, 'electron', {
      configurable: true,
      writable: true,
      value: { on },
    });
  });

  afterEach(() => {
    Object.defineProperty(window, 'electron', {
      configurable: true,
      writable: true,
      value: undefined,
    });
  });

  it('leaves on the main process’s command and hands the caller no IPC arguments', () => {
    const onBack = vi.fn();
    renderHook(() => useMouseBackButton(onBack));
    expect(on).toHaveBeenCalledWith('mouse-back-button-clicked', expect.any(Function));
    act(() => (commandListener as unknown as (...args: unknown[]) => void)({ sender: {} }, 'x'));
    expect(onBack).toHaveBeenCalledTimes(1);
    expect(onBack).toHaveBeenCalledWith();
  });

  it('leaves on the mouse’s back and forward keys and on no other button', () => {
    const onBack = vi.fn();
    renderHook(() => useMouseBackButton(onBack));
    for (const button of [0, 1, 2]) {
      fireEvent.mouseUp(document, { button });
    }
    expect(onBack).not.toHaveBeenCalled();
    fireEvent.mouseUp(document, { button: 3 });
    fireEvent.mouseUp(document, { button: 4 });
    expect(onBack).toHaveBeenCalledTimes(2);
  });

  it('takes the click away from the browser (preventDefault) when it acts', () => {
    renderHook(() => useMouseBackButton(vi.fn()));
    const event = new MouseEvent('mouseup', { button: 3, cancelable: true, bubbles: true });
    document.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
    const plain = new MouseEvent('mouseup', { button: 0, cancelable: true, bubbles: true });
    document.dispatchEvent(plain);
    expect(plain.defaultPrevented).toBe(false);
  });

  it('removes both listeners on unmount', () => {
    const onBack = vi.fn();
    const { unmount } = renderHook(() => useMouseBackButton(onBack));
    unmount();
    expect(dispose).toHaveBeenCalledTimes(1);
    fireEvent.mouseUp(document, { button: 3 });
    expect(onBack).not.toHaveBeenCalled();
  });

  it('does nothing outside Electron: a browser has its own Back', () => {
    Object.defineProperty(window, 'electron', {
      configurable: true,
      writable: true,
      value: undefined,
    });
    const onBack = vi.fn();
    renderHook(() => useMouseBackButton(onBack));
    fireEvent.mouseUp(document, { button: 3 });
    expect(onBack).not.toHaveBeenCalled();
  });

  it('listens to nothing without a handler, so a page with an optional Back can call it always', () => {
    renderHook(() => useMouseBackButton(undefined));
    expect(on).not.toHaveBeenCalled();
    fireEvent.mouseUp(document, { button: 3 });
  });

  it('is what BackButton uses: the key and the click do the same thing', () => {
    const onClick = vi.fn();
    render(<BackButton onClick={onClick} />);
    fireEvent.click(screen.getByRole('button', { name: 'Back' }));
    fireEvent.mouseUp(document, { button: 4 });
    expect(onClick).toHaveBeenCalledTimes(2);
  });
});
