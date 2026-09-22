import { useEffect, useRef, useState } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import CrewHostTrust from './CrewHostTrust';

export default function CrewAuthentication({
  connectionId,
  onConnected,
  onClose,
}: {
  connectionId: string;
  onConnected: () => void;
  onClose: () => void;
}) {
  const container = useRef<HTMLDivElement>(null);
  const [error, setError] = useState('');
  const activeSession = useRef('');
  const closeRequested = useRef(false);
  const connectedCallback = useRef(onConnected);
  useEffect(() => {
    connectedCallback.current = onConnected;
  }, [onConnected]);
  useEffect(() => {
    if (!container.current) return;
    setError('');
    activeSession.current = '';
    closeRequested.current = false;
    const terminal = new Terminal({
      cols: 80,
      rows: 12,
      scrollback: 0,
      screenReaderMode: true,
      fontSize: 12,
      disableStdin: false,
      theme: { background: '#17191c' },
    });
    const fit = new FitAddon();
    terminal.loadAddon(fit);
    terminal.open(container.current);
    fit.fit();
    let sessionId = '';
    let cancelled = false;
    const pending: { sessionId: string; data: string }[] = [];
    const pendingExits: { sessionId: string; exitCode: number | null }[] = [];
    let pendingBytes = 0;
    const showExit = (exitCode: number | null) => {
      if (exitCode === 0) {
        connectedCallback.current();
        return;
      }
      setError(
        `Daemon SSH authentication ended (exit ${exitCode ?? 'unknown'}). Reconnect to check the connection.`
      );
    };
    const observer = new ResizeObserver(() => {
      fit.fit();
      if (sessionId)
        void window.electron
          .resizeTerminalSession(sessionId, terminal.cols, terminal.rows)
          .catch(() => {});
    });
    observer.observe(container.current);
    const removeData = window.electron.onTerminalData((event) => {
      if (cancelled) return;
      if (sessionId === event.sessionId) terminal.write(event.data);
      else if (!sessionId && pending.length < 30 && pendingBytes < 65536) {
        const data = event.data.slice(0, 65536 - pendingBytes);
        pending.push({ sessionId: event.sessionId, data });
        pendingBytes += data.length;
      }
    });
    const removeExit = window.electron.onTerminalExit((event) => {
      if (cancelled) return;
      if (sessionId === event.sessionId) showExit(event.exitCode);
      else if (!sessionId && pendingExits.length < 30) pendingExits.push(event);
    });
    const input = terminal.onData((data) => {
      if (sessionId)
        void window.electron.writeTerminalSession(sessionId, data).catch(() => {
          if (!cancelled)
            setError(
              'SSH authentication input could not be delivered. Close this connection and reopen authentication.'
            );
        });
    });
    void (async () => {
      if (!window.electron.createCrewAuthentication) {
        setError('SSH authentication requires the desktop application.');
        return;
      }
      const result = await window.electron.createCrewAuthentication(connectionId);
      if (!result.success) {
        if (!cancelled) setError(result.error);
        return;
      }
      sessionId = result.sessionId;
      if (cancelled) {
        if (closeRequested.current) await window.electron.disposeTerminalSession(sessionId);
        return;
      }
      activeSession.current = sessionId;
      pending
        .filter((event) => event.sessionId === sessionId)
        .forEach((event) => terminal.write(event.data));
      pending.length = 0;
      pendingBytes = 0;
      const earlyExit = pendingExits.find((event) => event.sessionId === sessionId);
      pendingExits.length = 0;
      if (earlyExit) {
        showExit(earlyExit.exitCode);
        return;
      }
      fit.fit();
      await window.electron.resizeTerminalSession(sessionId, terminal.cols, terminal.rows);
      terminal.focus();
    })().catch((err: Error) => {
      if (!cancelled) setError(err.message);
    });
    return () => {
      cancelled = true;
      pending.length = 0;
      pendingExits.length = 0;
      pendingBytes = 0;
      removeData();
      removeExit();
      input.dispose();
      observer.disconnect();
      terminal.dispose();
      // Hash-route navigation detaches the display; the owned SSH master also serves normal chat.
      if (closeRequested.current && sessionId)
        void window.electron.disposeTerminalSession(sessionId);
    };
  }, [connectionId]);
  return (
    <section className="crew-auth" aria-label="SSH authentication">
      <strong>Authenticate with your SSH host</strong>
      <p className="crew-small">
        Enter credentials only in this terminal. Prompts are not saved to Crew history. The SSH
        connection stays available when you switch to another page; close it explicitly when
        finished. Crew connects automatically after the daemon verifies authentication.
      </p>
      <div className="crew-auth-terminal" ref={container} />
      {error && <p role="alert">{error}</p>}
      <CrewHostTrust />
      <div className="crew-inline">
        <button
          className="crew-button"
          onClick={() => {
            closeRequested.current = true;
            if (activeSession.current)
              void window.electron.disposeTerminalSession(activeSession.current);
            onClose();
          }}
        >
          Close authentication connection
        </button>
      </div>
    </section>
  );
}
