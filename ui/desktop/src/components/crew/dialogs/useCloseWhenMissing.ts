import { useEffect, useRef } from 'react';

/**
 * Close a dialog whose subject is gone — a connection removed elsewhere, a channel or person no
 * longer in the verified view. A dialog never lingers over, or acts on, something that is not there.
 */
export function useCloseWhenMissing(missing: boolean, onClose: () => void): void {
  const latest = useRef(onClose);
  useEffect(() => {
    latest.current = onClose;
  }, [onClose]);
  useEffect(() => {
    if (missing) latest.current();
  }, [missing]);
}
