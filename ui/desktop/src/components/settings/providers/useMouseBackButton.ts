import { useEffect } from 'react';

/**
 * The mouse's "back" button (and the OS's browser-backward command) leaves this page, as it did
 * when the page's Back control was `ui/BackButton`. The page's Back is the shared band action now
 * (`PageHeader`'s `onBack`), which is only a button, so the listener `BackButton` carried moves
 * here unchanged: the main process forwards the command as `mouse-back-button-clicked`, and a raw
 * `mouseup` of button 3 or 4 covers a mouse the OS does not report it for.
 *
 * Only inside Electron, as `BackButton` was; a browser has its own Back.
 */
export function useMouseBackButton(onBack: () => void): void {
  useEffect(() => {
    if (!window.electron) return undefined;

    const disposeCommand = window.electron.on('mouse-back-button-clicked', onBack);
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
