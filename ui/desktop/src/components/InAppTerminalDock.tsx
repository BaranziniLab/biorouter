import { useFontSize } from '../hooks/useFontSize';
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Terminal as XTerm } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import { Plus, Terminal as TerminalIcon, X } from './icons/app-icons';
import { Button } from './ui/button';
import { useResolvedTheme, useThemeFamily } from '../contexts/ThemeContext';
import { registerCloseTerminalPane, registerNewTerminalPane } from '../utils/terminalFocus';
import { onTerminalRunRequest } from '../utils/terminalRunChannel';
import { terminalInputForCommand } from '../utils/shellCommandBlock';
import { cn } from '../utils';
import { GENERATED_THEMES } from '../styles/themes.generated';

interface InAppTerminalDockProps {
  open: boolean;
  workingDir?: string;
  /** Hide the dock — the header X or the parent toggle. Panes stay alive. */
  onClose: () => void;
  /**
   * The user closed the LAST pane, so this terminal is now empty. Distinct from
   * onClose (hide): the parent should DESTROY the terminal here, not just hide
   * it. Falls back to onClose when omitted, which is the local (non-/pair) dock's
   * behaviour — there hiding and emptying are the same "set open false".
   */
  onEmptied?: () => void;
  /**
   * This terminal's scope — the chat tab id `TerminalDockContext` is keyed by.
   *
   * Supplied ONLY so the dock can receive "run this code block" requests for its
   * own chat; everything else about a dock is already scoped by where it is
   * mounted. Omitted by the onboarding card's dock, which belongs to no chat and
   * must never be the landing zone for a transcript's Run button.
   */
  dockKey?: string;
}

type TerminalPane = {
  id: string;
  title: string;
};

type ProposedDimensions = {
  cols: number;
  rows: number;
};

/**
 * How many pane tabs ONE dock may hold.
 *
 * A comfort rail for the tab strip (which scrolls), not a resource limit — the
 * real rail is per WINDOW and lives in terminalSessionRegistry.ts. This was 8,
 * which collided with the old per-window cap of 8: three chat tabs with three
 * panes each asked for nine shells, the ninth was refused by the main process,
 * and the user got a dead pane instead of a disabled "+".
 */
export const MAX_TERMINAL_PANES = 32;

function getTerminalSize(fitAddon: FitAddon): ProposedDimensions {
  const proposed = (
    fitAddon as FitAddon & { proposeDimensions?: () => ProposedDimensions | undefined }
  ).proposeDimensions?.();
  return {
    cols: Math.max(24, proposed?.cols ?? 80),
    rows: Math.max(8, proposed?.rows ?? 18),
  };
}

function basename(path?: string) {
  if (!path) return 'terminal';
  return path.replace(/\/+$/, '').split('/').pop() || 'terminal';
}

function formatCwd(cwd?: string) {
  if (!cwd) return 'Chat terminal';
  const home = window.appConfig?.get('BIOROUTER_HOME_DIR') as string | undefined;
  if (home && cwd.startsWith(home)) return cwd.replace(home, '~');
  return cwd;
}

function makePaneTitle(workingDir: string | undefined, index: number) {
  const name = basename(workingDir);
  return index === 1 ? name : `${name} ${index}`;
}

// xterm measures glyph widths itself, so it cannot read `var(--font-mono)`.
// This stack must stay byte-identical to --font-mono in styles/main.css, so a
// command pasted from a chat code block renders in exactly the same face.
const TERMINAL_FONT =
  'ui-monospace, "SF Mono", SFMono-Regular, "Cascadia Mono", Menlo, Consolas, "Liberation Mono", monospace';
const TERMINAL_FONT_SIZE = 13;
const TERMINAL_LINE_HEIGHT = 20 / 13; // design.md §3.2 — code/terminal is 13/20

/**
 * Terminal palettes, keyed by family then resolved mode — GENERATED.
 *
 * xterm paints to a canvas and cannot read a CSS custom property, so these
 * values must exist as JS. They used to be hand-typed here: 126 hexes across
 * three families, with per-stop ratios documented in comments and NOT ONE TEST
 * covering them. The generator now derives them from the same theme definition
 * that produces the CSS, and checks every stop against its own ground before it
 * will emit — which is how five Parchment light stops were found sitting below
 * the AA floor their comment claimed they cleared.
 *
 * `background` and `cursorAccent` are derived from each family's own
 * `terminalGround` token. That token is per-family on purpose: Parchment dark
 * paints --background-code, while Alma Mater and Roche Limit paint
 * --background-muted. Assuming they agreed would silently re-ground two
 * terminals under palettes tuned for a different surface.
 */
const TERMINAL_THEMES_BY_FAMILY = GENERATED_THEMES;

/**
 * How many Run commands one pane may hold while its shell starts.
 *
 * Same bound and same reason as `terminalRunChannel`'s own queue: a click
 * nobody could serve is not a debt that accumulates.
 */
const MAX_PENDING_RUNS = 4;

/**
 * How long a queued Run waits for the shell to say anything before it is
 * written anyway.
 *
 * Not a guess at how long a shell takes to start — the ordinary path is the
 * first output chunk, which arrives in milliseconds. This is the answer for a
 * shell that emits nothing at all, where the alternative is a command the user
 * was told had been sent and that never runs.
 */
const SHELL_READY_FALLBACK_MS = 2_000;

const TerminalPaneView: React.FC<{
  active: boolean;
  open: boolean;
  paneId: string;
  workingDir?: string;
  dockKey?: string;
}> = ({ active, open, paneId, workingDir, dockKey }) => {
  const { fontScale } = useFontSize();
  const fontScaleRef = useRef(fontScale);
  fontScaleRef.current = fontScale;
  const terminalHostRef = useRef<HTMLDivElement | null>(null);
  const terminalRef = useRef<XTerm | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const backendSessionIdRef = useRef<string | null>(null);
  const pendingInputRef = useRef<string[]>([]);
  // Run requests waiting for this pane's shell to be ready to RECEIVE them,
  // which is later than "the pty exists" — see `deliverRun`.
  const pendingRunsRef = useRef<string[]>([]);
  const shellReadyRef = useRef(false);
  const shellReadyFallbackRef = useRef<number | null>(null);
  const terminalExitedRef = useRef(false);
  const fitEnabledRef = useRef(open && active);
  const focusFrameRef = useRef<number | null>(null);
  const focusTimerRef = useRef<number | null>(null);
  fitEnabledRef.current = open && active;

  // The xterm instance is created once, in an effect that must not re-run when
  // the theme flips (that would tear down the pty session). The ref lets the
  // constructor read the current theme; the effect below repaints on change.
  const resolvedTheme = useResolvedTheme();
  const resolvedThemeRef = useRef(resolvedTheme);
  resolvedThemeRef.current = resolvedTheme;
  const themeFamily = useThemeFamily();
  const themeFamilyRef = useRef(themeFamily);
  themeFamilyRef.current = themeFamily;

  useEffect(() => {
    const term = terminalRef.current;
    if (term) term.options.theme = TERMINAL_THEMES_BY_FAMILY[themeFamily][resolvedTheme].terminal;
  }, [resolvedTheme, themeFamily]);

  const focusTerminal = useCallback(() => {
    terminalRef.current?.focus();
  }, []);

  const writeToBackend = useCallback((data: string) => {
    const sessionId = backendSessionIdRef.current;
    if (!sessionId) {
      if (!terminalExitedRef.current) pendingInputRef.current.push(data);
      return;
    }
    window.electron.writeTerminalSession(sessionId, data).catch(() => {});
  }, []);

  /**
   * Write one queued or live Run command into this pane's shell.
   *
   * ⚠ A Run command CANNOT reuse `pendingInputRef`, and that is the whole point
   * of this second buffer. That one holds BYTES, which means the bracketing
   * decision has already been made when the command goes into it — and the
   * moment a Run is buffered is exactly the moment that decision is
   * unknowable. `modes.bracketedPasteMode` reports what the SHELL asked for
   * (DECSET 2004), the shell asks by emitting an escape sequence as part of
   * printing its first prompt, and until that output has been parsed the flag
   * reads `false` whether or not the shell wants bracketing. Buffering bytes
   * therefore commits a multi-line block to the un-bracketed form and the
   * heredoc or `for` loop lands in the line editor a line at a time, each
   * newline its own Enter.
   *
   * So a Run that arrives early is held as a COMMAND and bracketed at the
   * moment it is written, which `flushPendingRuns` does once the shell has
   * spoken. Keystrokes have no such problem: they are single characters the
   * user typed, and go on being buffered by `writeToBackend`.
   *
   * Returns whether the pane accepted it. `false` only for a shell that has
   * exited — a command written there vanishes, and the transcript's button must
   * not paint that as "Sent".
   */
  const deliverRun = useCallback(
    (command: string): boolean => {
      if (terminalExitedRef.current) return false;
      const term = terminalRef.current;
      if (!term || !backendSessionIdRef.current || !shellReadyRef.current) {
        // Bounded like the channel's own queue: a click nobody could serve is
        // not a debt that accumulates.
        if (pendingRunsRef.current.length >= MAX_PENDING_RUNS) pendingRunsRef.current.shift();
        pendingRunsRef.current.push(command);
        return true;
      }
      // The `?.` is load-bearing: `modes` is non-optional in the typings but
      // absent from the test double.
      writeToBackend(terminalInputForCommand(command, term.modes?.bracketedPasteMode ?? false));
      // So the user can Ctrl-C it, or answer whatever it asks. A hidden dock's
      // focus() is a browser no-op; `fitTerminal` focuses again when it shows.
      term.focus();
      return true;
    },
    [writeToBackend]
  );

  /** Write everything `deliverRun` parked, now that the shell can be read. */
  const flushPendingRuns = useCallback(() => {
    if (pendingRunsRef.current.length === 0) return;
    const term = terminalRef.current;
    if (!term || !backendSessionIdRef.current || terminalExitedRef.current) return;
    const bracketedPaste = term.modes?.bracketedPasteMode ?? false;
    for (const command of pendingRunsRef.current.splice(0)) {
      writeToBackend(terminalInputForCommand(command, bracketedPaste));
    }
    term.focus();
  }, [writeToBackend]);

  // Stable handle for the mount effect below, which must not re-run (and so
  // tear down the pty) because a callback identity changed.
  const flushPendingRunsRef = useRef(flushPendingRuns);
  flushPendingRunsRef.current = flushPendingRuns;

  // "Run" on a chat code block lands here (utils/terminalRunChannel.ts).
  //
  // Claimed by the ACTIVE pane only — the one the user can see is the one a
  // command belongs in — and only when this dock has a key, which the
  // onboarding card's chat-less dock does not.
  //
  // It goes through this pane's OWN writeToBackend rather than around it, which
  // is the whole reason the channel carries a command instead of a session id.
  useEffect(() => {
    if (!active) return;
    return onTerminalRunRequest(dockKey, deliverRun);
  }, [active, dockKey, deliverRun]);

  const fitTerminal = useCallback(
    (focus: boolean) => {
      if (focusFrameRef.current !== null) {
        window.cancelAnimationFrame(focusFrameRef.current);
        focusFrameRef.current = null;
      }
      if (focusTimerRef.current !== null) {
        window.clearTimeout(focusTimerRef.current);
        focusTimerRef.current = null;
      }
      const term = terminalRef.current;
      const fitAddon = fitAddonRef.current;
      if (!term || !fitAddon || !open || !active) return;
      focusFrameRef.current = window.requestAnimationFrame(() => {
        focusFrameRef.current = null;
        try {
          fitAddon.fit();
          const { cols, rows } = getTerminalSize(fitAddon);
          const sessionId = backendSessionIdRef.current;
          if (sessionId) {
            window.electron.resizeTerminalSession(sessionId, cols, rows).catch(() => {});
          }
          if (focus) {
            term.focus();
            focusTimerRef.current = window.setTimeout(() => {
              focusTimerRef.current = null;
              if (fitEnabledRef.current) term.focus();
            }, 30);
          }
        } catch {
          /* xterm can throw while the hidden dock has no measurable dimensions */
        }
      });
    },
    [active, open]
  );

  useEffect(() => {
    const host = terminalHostRef.current;
    if (!host) return;

    let disposed = false;
    let animationFrame = 0;
    const fitAddon = new FitAddon();
    const term = new XTerm({
      allowProposedApi: true,
      allowTransparency: true,
      convertEol: true,
      cursorBlink: true,
      cursorStyle: 'block',
      fontFamily: TERMINAL_FONT,
      fontSize: TERMINAL_FONT_SIZE * fontScaleRef.current,
      lineHeight: TERMINAL_LINE_HEIGHT,
      scrollback: 8000,
      theme: TERMINAL_THEMES_BY_FAMILY[themeFamilyRef.current][resolvedThemeRef.current].terminal,
    });

    terminalRef.current = term;
    fitAddonRef.current = fitAddon;
    term.loadAddon(fitAddon);
    term.open(host);

    const flushResize = () => {
      if (disposed) return;
      try {
        fitAddon.fit();
        const { cols, rows } = getTerminalSize(fitAddon);
        const sessionId = backendSessionIdRef.current;
        if (sessionId) {
          window.electron.resizeTerminalSession(sessionId, cols, rows).catch(() => {});
        }
      } catch {
        /* ignored while hidden */
      }
    };

    const dataDisposer = window.electron.onTerminalData((event) => {
      if (event.sessionId !== backendSessionIdRef.current) return;
      // The callback fires once xterm has PARSED the chunk, which is the
      // earliest moment `modes.bracketedPasteMode` is an observation rather
      // than a default: the shell announces DECSET 2004 as part of printing its
      // first prompt, so the flag is settled exactly here and not before. See
      // `deliverRun`.
      term.write(event.data, () => {
        shellReadyRef.current = true;
        flushPendingRunsRef.current();
      });
    });
    const exitDisposer = window.electron.onTerminalExit((event) => {
      if (event.sessionId !== backendSessionIdRef.current) return;
      backendSessionIdRef.current = null;
      terminalExitedRef.current = true;
      shellReadyRef.current = false;
      pendingInputRef.current = [];
      // A queued Run has nowhere to go now, and holding it would fire it into
      // whatever shell this pane starts next.
      pendingRunsRef.current = [];
      const suffix = event.signal ? ` (${event.signal})` : '';
      term.writeln(
        `\r\n\x1b[90m[terminal exited with code ${event.exitCode ?? 0}${suffix}]\x1b[0m`
      );
    });
    const inputDisposer = term.onData((data) => {
      writeToBackend(data);
    });

    const resizeObserver = new ResizeObserver(() => {
      if (!fitEnabledRef.current) return;
      window.cancelAnimationFrame(animationFrame);
      animationFrame = window.requestAnimationFrame(flushResize);
    });
    resizeObserver.observe(host);

    const start = async () => {
      const { cols, rows } = getTerminalSize(fitAddon);
      const result = await window.electron.createTerminalSession({ workingDir, cols, rows });
      if (disposed) {
        if (result.success) {
          await window.electron.disposeTerminalSession(result.sessionId).catch(() => {});
        }
        return;
      }
      if (!result.success) {
        terminalExitedRef.current = true;
        pendingInputRef.current = [];
        pendingRunsRef.current = [];
        term.writeln(`\x1b[31mCould not start terminal: ${result.error}\x1b[0m`);
        return;
      }
      terminalExitedRef.current = false;
      backendSessionIdRef.current = result.sessionId;
      pendingInputRef.current.splice(0).forEach((data) => {
        window.electron.writeTerminalSession(result.sessionId, data).catch(() => {});
      });
      // A shell that says nothing at all would otherwise hold a queued Run for
      // ever, having already told the user it was sent. Every real login shell
      // prints a prompt long before this fires; this is the answer for the one
      // that does not, and it delivers with whatever the mode reads as then.
      shellReadyFallbackRef.current = window.setTimeout(() => {
        shellReadyFallbackRef.current = null;
        shellReadyRef.current = true;
        flushPendingRunsRef.current();
      }, SHELL_READY_FALLBACK_MS);
      flushResize();
    };

    start().catch((error) => {
      if (!disposed) {
        term.writeln(`\x1b[31mCould not start terminal: ${String(error)}\x1b[0m`);
      }
    });

    return () => {
      disposed = true;
      if (shellReadyFallbackRef.current !== null) {
        window.clearTimeout(shellReadyFallbackRef.current);
        shellReadyFallbackRef.current = null;
      }
      shellReadyRef.current = false;
      pendingRunsRef.current = [];
      window.cancelAnimationFrame(animationFrame);
      resizeObserver.disconnect();
      dataDisposer();
      exitDisposer();
      inputDisposer.dispose();
      if (backendSessionIdRef.current) {
        window.electron.disposeTerminalSession(backendSessionIdRef.current).catch(() => {});
      }
      term.dispose();
      terminalRef.current = null;
      fitAddonRef.current = null;
      backendSessionIdRef.current = null;
    };
  }, [paneId, workingDir, writeToBackend]);

  useEffect(() => {
    fitTerminal(true);
  }, [fitTerminal]);

  useEffect(() => {
    const term = terminalRef.current;
    const nextFontSize = TERMINAL_FONT_SIZE * fontScale;
    if (!term || term.options.fontSize === nextFontSize) return;
    term.options.fontSize = nextFontSize;
    fitTerminal(false);
  }, [fontScale, fitTerminal]);

  useEffect(() => {
    return () => {
      if (focusFrameRef.current !== null) window.cancelAnimationFrame(focusFrameRef.current);
      if (focusTimerRef.current !== null) window.clearTimeout(focusTimerRef.current);
    };
  }, []);

  return (
    <div
      className={cn(
        'col-start-1 row-start-1 flex min-h-0 overflow-hidden',
        !active && 'pointer-events-none invisible'
      )}
      data-terminal-pane={paneId}
    >
      <div
        ref={terminalHostRef}
        onMouseDown={(event) => {
          event.preventDefault();
          focusTerminal();
        }}
        // The ground is painted ONCE, by the dock's terminal region (bg-background-muted).
        // This host is transparent and contributes only the inset xterm needs to
        // breathe, so the generated palette — not a hand-typed hex — is what reaches the screen;
        // the theme `background` above is the forced mirror xterm needs internally
        // (it cannot read a CSS var). The bg-transparent! overrides are what keep
        // the token authoritative: without them xterm's own layers would repaint
        // the ground and any drift between hex and token would show as a seam.
        // No border and no radius here — the dock's border-t is the only hairline
        // the terminal needs (D-11).
        className="h-full min-h-0 w-full flex-1 overflow-hidden px-2 py-1.5 [&_.xterm]:h-full [&_.xterm-viewport]:bg-transparent! [&_.xterm-screen]:bg-transparent!"
      />
    </div>
  );
};

export const InAppTerminalDock: React.FC<InAppTerminalDockProps> = ({
  open,
  workingDir,
  onClose,
  onEmptied,
  dockKey,
}) => {
  const [panes, setPanes] = useState<TerminalPane[]>([]);
  const [activePaneId, setActivePaneId] = useState<string | null>(null);
  const suppressAutoOpenRef = useRef(false);
  const pendingDockCloseRef = useRef(false);
  // This dock's own DOM root, handed to the pane registries so a Cmd+T/Cmd+W
  // routes to THIS dock only when it is the one holding (or last holding)
  // focus — several docks can be visible at once in a split (one per pane).
  const rootRef = useRef<HTMLElement | null>(null);
  const getRoot = useCallback(() => rootRef.current, []);

  // The single painted terminal ground, read from the family's own declaration
  // rather than hardcoded. `terminalGround` is a TOKEN NAME (e.g.
  // `--background-muted`), and it is per-family on purpose — see the comment on
  // TERMINAL_THEMES_BY_FAMILY above. xterm's canvas is already coloured from
  // this exact field (`terminal.background`, derived from it by the generator),
  // so painting the region behind it from anything else is two answers to one
  // question. All six family/mode combinations resolve to `--background-muted`
  // today, which is precisely why the hardcode survived: it was latent, not
  // wrong, and the first family to move its ground would have left a rim of the
  // old surface around a re-grounded canvas.
  const dockThemeFamily = useThemeFamily();
  const dockResolvedTheme = useResolvedTheme();
  const terminalGroundToken =
    TERMINAL_THEMES_BY_FAMILY[dockThemeFamily][dockResolvedTheme].terminalGround;

  const addPane = useCallback(() => {
    setPanes((current) => {
      if (current.length >= MAX_TERMINAL_PANES) return current;
      const index = current.length + 1;
      const pane = {
        id: window.crypto.randomUUID(),
        title: makePaneTitle(workingDir, index),
      };
      setActivePaneId(pane.id);
      return [...current, pane];
    });
  }, [workingDir]);

  useEffect(() => {
    if (!open) {
      suppressAutoOpenRef.current = false;
      pendingDockCloseRef.current = false;
      return;
    }
    if (panes.length === 0 && !suppressAutoOpenRef.current) {
      addPane();
    }
  }, [addPane, open, panes.length]);

  const closePane = useCallback((paneId: string) => {
    setPanes((current) => {
      const closedIndex = current.findIndex((pane) => pane.id === paneId);
      if (closedIndex === -1) return current;

      const next = current.filter((pane) => pane.id !== paneId);
      if (next.length === 0) {
        suppressAutoOpenRef.current = true;
        pendingDockCloseRef.current = true;
        setActivePaneId(null);
        return [];
      }

      setActivePaneId((activeId) => {
        if (activeId && activeId !== paneId && next.some((pane) => pane.id === activeId)) {
          return activeId;
        }
        return next[Math.min(closedIndex, next.length - 1)].id;
      });
      return next;
    });
  }, []);

  useEffect(() => {
    if (!open || panes.length !== 0 || !pendingDockCloseRef.current) return;
    pendingDockCloseRef.current = false;
    // The last pane closed: DESTROY, don't merely hide. onEmptied lets the
    // per-tab shell drop this terminal entirely (disposing it); the local dock
    // has no onEmptied and falls back to onClose, its "set open false".
    (onEmptied ?? onClose)();
  }, [onClose, onEmptied, open, panes.length]);

  // While this dock is VISIBLE, own the "new terminal pane" gesture: focus-aware
  // Cmd+T (routed in App.tsx) adds a pane here instead of a chat tab. Only open
  // docks register — but a split can show SEVERAL open docks at once (one per
  // pane), so the registration carries this dock's root and the registry routes
  // the keystroke to the dock that holds focus (Codex review B6 finding 3).
  useEffect(() => {
    if (!open) return;
    return registerNewTerminalPane(addPane, getRoot);
  }, [open, addPane, getRoot]);

  // …and the "close terminal pane" gesture (issue #21): focus-aware Cmd+W
  // closes the focused PANE here instead of the chat tab. The ref keeps the
  // registration stable across pane switches; closing the last pane flows
  // through pendingDockCloseRef -> onEmptied above, so no extra teardown is
  // needed — the dock disposes, the registry disposer runs, and the NEXT Cmd+W
  // falls through to the chat tab.
  const activePaneIdRef = useRef<string | null>(null);
  activePaneIdRef.current = activePaneId;

  useEffect(() => {
    if (!open) return;
    return registerCloseTerminalPane(() => {
      const paneId = activePaneIdRef.current;
      if (!paneId) return false;
      closePane(paneId);
      return true;
    }, getRoot);
  }, [open, closePane, getRoot]);

  useEffect(() => {
    if (!open || panes.length === 0) return;
    if (activePaneId && panes.some((pane) => pane.id === activePaneId)) return;
    setActivePaneId(panes[0].id);
  }, [activePaneId, open, panes]);

  if (!open && panes.length === 0) return null;

  return (
    <section
      ref={rootRef}
      data-testid="in-app-terminal-dock"
      className={cn(
        'no-drag flex min-h-[100px] flex-shrink-0 flex-col overflow-hidden border-t border-border-subtle bg-background-default text-text-default ',
        open
          ? 'animate-in slide-in-from-bottom-2 fade-in duration-[var(--motion-base)] ease-out'
          : 'hidden'
      )}
      style={{ height: 'min(42vh, 380px, max(100px, calc(100% - 200px)))' }}
    >
      {/* Safari-style tab strip (Ω/Ψ). .br-tabstrip owns the flex layout, the gap,
          the padding and the bottom hairline; `--sm` re-grounds it on the
          SURFACE (the chat and artifact strips keep `--sidebar` to continue the
          window's top edge — at the bottom edge that rationale inverts, and the
          strip read as a floating slice of sidebar); .br-tab owns the tabs, the
          active pill and the divider between them.
          Nothing here may restyle any of that — the only additions are h-10 (the
          class leaves height to the host; the dock's strip is 40px) and
          flex-shrink-0, so the strip holds its height in the dock's flex column. */}
      <div className="br-tabstrip br-tabstrip--sm h-10 flex-shrink-0">
        <div
          className="flex min-w-0 flex-1 items-center overflow-x-auto"
          role="tablist"
          aria-label="Terminals"
        >
          {panes.map((pane) => {
            const active = pane.id === activePaneId;
            return (
              <div
                key={pane.id}
                // .br-tab already caps at 190px and lets the label ellipsis;
                // flex-shrink-0 is the only addition, so a full strip scrolls
                // rather than crushing every tab.
                className="br-tab flex-shrink-0"
                data-active={active}
              >
                <button
                  type="button"
                  role="tab"
                  aria-selected={active}
                  onClick={() => setActivePaneId(pane.id)}
                  className="flex h-full min-w-0 flex-1 items-center gap-1.5 text-left"
                >
                  <TerminalIcon className="h-4 w-4 flex-shrink-0" />
                  <span className="br-tab__label">{pane.title}</span>
                </button>
                <button
                  type="button"
                  onMouseDown={(event) => event.stopPropagation()}
                  onClick={(event) => {
                    event.stopPropagation();
                    closePane(pane.id);
                  }}
                  className="flex h-5 w-5 flex-shrink-0 items-center justify-center rounded-sm text-text-muted transition-colors hover:bg-background-medium hover:text-text-default"
                  aria-label={`Close terminal tab ${pane.title}`}
                  title={`Close ${pane.title}`}
                >
                  <X className="h-4 w-4" />
                </button>
              </div>
            );
          })}
          {/* Left-aligned "+", immediately to the right of the last tab and
              scrolling WITH the tabs — the same placement (and the chat strip's
              .br-tab-new styling) as the chat tabs' new-tab button. It used to be
              pushed to the far right of the strip. */}
          <button
            type="button"
            onClick={addPane}
            disabled={panes.length >= MAX_TERMINAL_PANES}
            className="br-tab-new ml-0.5 flex h-6 w-6 flex-none items-center justify-center rounded-md text-text-muted transition-colors hover:bg-background-medium hover:text-text-default disabled:pointer-events-none disabled:opacity-40"
            aria-label="New terminal"
            title={
              panes.length >= MAX_TERMINAL_PANES
                ? `Maximum of ${MAX_TERMINAL_PANES} terminals reached`
                : 'New terminal'
            }
          >
            <Plus className="h-4 w-4" />
          </button>
        </div>
        <span className="hidden min-w-0 max-w-[38%] truncate text-xs text-text-muted sm:block">
          {formatCwd(workingDir)}
        </span>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          shape="round"
          onClick={onClose}
          className="h-7 w-7 flex-shrink-0 p-0 text-text-muted hover:bg-background-default/70 hover:text-text-default"
          aria-label="Hide terminal"
          title="Hide terminal"
        >
          <X />
        </Button>
      </div>
      {/* The single painted terminal ground — the family's own `terminalGround`
          token (see above), never a hardcoded one. No gutter padding: the
          terminal bleeds to the dock edges, and the panes' own inset does the
          breathing. */}
      <div
        className="grid min-h-0 flex-1 grid-cols-1 grid-rows-1"
        data-terminal-ground={terminalGroundToken}
        style={{ background: `var(${terminalGroundToken})` }}
      >
        {panes.map((pane) => (
          <TerminalPaneView
            key={pane.id}
            active={pane.id === activePaneId}
            open={open}
            paneId={pane.id}
            workingDir={workingDir}
            dockKey={dockKey}
          />
        ))}
      </div>
    </section>
  );
};

export default InAppTerminalDock;
