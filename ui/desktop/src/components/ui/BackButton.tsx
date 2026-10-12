import React, { useEffect, useCallback } from 'react';
import { ArrowLeft } from '../icons/app-icons';
import { Button } from './button';
import type { VariantProps } from 'class-variance-authority';
import { buttonVariants } from './button';
import { cn } from '../../utils';

interface BackButtonProps extends VariantProps<typeof buttonVariants> {
  onClick?: () => void;
  className?: string;
  showText?: boolean;
  shape?: 'pill' | 'round';
}

/**
 * The mouse's "back" key (and the OS's browser-backward command) leaves the page it is on. The main
 * process forwards the command as `mouse-back-button-clicked`, and a raw `mouseup` of button 3 or 4
 * covers a mouse the OS does not report it for. Only inside Electron: a browser has its own Back.
 *
 * `BackButton` calls this itself; a page whose Back is another control (a `PageHeader`'s `onBack`,
 * which is only a button) calls it with the same handler, so the key does the same thing as the
 * click. Pass a stable function (`useCallback`) or the listeners are re-attached on every render;
 * `undefined` listens to nothing, so a page whose Back is optional can call it unconditionally.
 * One page, one caller: two hooks mounted together both leave.
 */
export function useMouseBackButton(onBack: (() => void) | undefined): void {
  useEffect(() => {
    if (!onBack || !window.electron) return undefined;

    // Wrapped, so the IPC event the main process passes along never reaches the caller's handler.
    const disposeCommand = window.electron.on('mouse-back-button-clicked', () => onBack());
    const onMouseUp = (event: MouseEvent) => {
      // Buttons 3 and 4 are the mouse's back and forward keys.
      if (event.button === 3 || event.button === 4) {
        onBack();
        event.preventDefault();
      }
    };
    document.addEventListener('mouseup', onMouseUp);

    return () => {
      disposeCommand?.();
      document.removeEventListener('mouseup', onMouseUp);
    };
  }, [onBack]);
}

const BackButton: React.FC<BackButtonProps> = ({
  onClick,
  className = '',
  variant = 'ghost',
  size = 'sm',
  shape = 'pill',
  showText = true,
  ...props
}) => {
  const handleExit = useCallback(() => {
    if (onClick) {
      onClick(); // Custom onClick handler passed via props
    } else if (window.history.length > 1) {
      window.history.back(); // Navigate to the previous page
    } else {
      console.warn('No history to go back to');
    }
  }, [onClick]);

  useMouseBackButton(handleExit);

  return (
    <Button
      onClick={handleExit}
      variant={variant}
      size={size}
      shape={shape}
      className={cn('flex items-center gap-1.5 text-text-muted hover:text-text-default', className)}
      {...props}
    >
      <ArrowLeft />
      {showText && 'Back'}
    </Button>
  );
};

export default BackButton;
