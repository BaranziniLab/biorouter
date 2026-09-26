import { useEffect, useRef, useState } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import { useResolvedTheme, useThemeFamily } from '../../contexts/ThemeContext';
import { GENERATED_THEMES } from '../../styles/themes.generated';
import { Button } from '../ui/button';
import { Disclosure } from '../ui/disclosure';
import { Note } from '../ui/note';
import CrewHostTrust from './CrewHostTrust';
import { signInCopy } from './auth/copy';
import './auth/auth.css';

/** The terminal's type, the app's code role: 13px on a 20px line (design.md §3.2). */
const TERMINAL_FONT_SIZE = 13;
const TERMINAL_LINE_HEIGHT = 20 / 13;

/**
 * xterm measures glyphs itself and cannot read `var(--font-mono)`, so the stack is resolved from
 * the stylesheet — the same face the in-app terminal and every code block use, with no third copy
 * of the stack to drift.
 */
function terminalFontFamily(): string {
  try {
    const stack = getComputedStyle(document.documentElement).getPropertyValue('--font-mono').trim();
    if (stack) return stack;
  } catch {
    // No stylesheet to read (a test, a detached document): the platform's monospace.
  }
  return 'monospace';
}

/**
 * The SSH sign-in terminal: the body of the Sign in dialog (`crew/auth/SignInDialog.tsx`).
 *
 * The contract the regression tests pin (C16) is unchanged: data and an exit that arrive before
 * the IPC answers are replayed for the right session only; the session is resized only after it
 * exists and never after an early exit; exit 0 is the only completion (no manual bypass); there is
 * exactly one `role="alert"`; and the SSH session is disposed only by an explicit Close — an
 * unmount (navigation, a re-render) leaves the owned SSH master running for normal chat.
 *
 * The terminal wears the theme family's generated terminal palette and re-themes on a family or
 * mode change without recreating the session.
 */
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
  const terminalRef = useRef<Terminal | null>(null);
  const family = useThemeFamily();
  const mode = useResolvedTheme();
  const palette = GENERATED_THEMES[family][mode];
  const paletteRef = useRef(palette.terminal);
  useEffect(() => {
    connectedCallback.current = onConnected;
  }, [onConnected]);
  useEffect(() => {
    paletteRef.current = palette.terminal;
    if (terminalRef.current) terminalRef.current.options.theme = palette.terminal;
  }, [palette.terminal]);
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
      fontFamily: terminalFontFamily(),
      fontSize: TERMINAL_FONT_SIZE,
      lineHeight: TERMINAL_LINE_HEIGHT,
      disableStdin: false,
      theme: paletteRef.current,
    });
    terminalRef.current = terminal;
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
      setError(signInCopy.ended(exitCode ?? 'unknown'));
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
          if (!cancelled) setError(signInCopy.inputLost);
        });
    });
    void (async () => {
      if (!window.electron.createCrewAuthentication) {
        setError(signInCopy.needsDesktop);
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
      terminalRef.current = null;
      // Hash-route navigation detaches the display; the owned SSH master also serves normal chat.
      if (closeRequested.current && sessionId)
        void window.electron.disposeTerminalSession(sessionId);
    };
  }, [connectionId]);
  return (
    <section className="crew-signin" aria-label={signInCopy.terminalName}>
      {/* The family's own terminal ground token, which the generated palette's background is
          derived from, so the inset around the xterm canvas and the canvas agree. */}
      <div
        className="crew-signin-terminal"
        ref={container}
        data-terminal-ground={palette.terminalGround}
        style={{ background: `var(${palette.terminalGround})` }}
      />
      {error && (
        <Note tone="danger" role="alert">
          {error}
        </Note>
      )}
      <Disclosure label={signInCopy.help}>
        <CrewHostTrust />
      </Disclosure>
      <div className="crew-signin-actions">
        <Button
          type="button"
          variant="outline"
          aria-label={signInCopy.closeName}
          onClick={() => {
            closeRequested.current = true;
            if (activeSession.current)
              void window.electron.disposeTerminalSession(activeSession.current);
            onClose();
          }}
        >
          {signInCopy.close}
        </Button>
      </div>
    </section>
  );
}
