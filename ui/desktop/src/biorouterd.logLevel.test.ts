// @vitest-environment node

import type { App } from 'electron';
import type { PathLike, Stats } from 'node:fs';
import { afterAll, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  logInfo: vi.fn(),
  logError: vi.fn(),
  logWarn: vi.fn(),
  logDebug: vi.fn(),
  spawn: vi.fn(),
}));

// Same rationale as biorouterd.test.ts: the binary-path probe hits the real
// filesystem, so stub `node:fs` for the biorouterd candidates only.
const isBiorouterdPath = (p: PathLike): boolean => {
  const s = p.toString();
  return s.endsWith('biorouterd') || s.endsWith('biorouterd.exe');
};
vi.mock('node:fs', async (importOriginal) => {
  const actual = await importOriginal<typeof import('node:fs')>();
  const existsSync = (p: PathLike): boolean => (isBiorouterdPath(p) ? true : actual.existsSync(p));
  const statSync = (p: PathLike): Stats =>
    isBiorouterdPath(p) ? ({ isFile: () => true } as unknown as Stats) : actual.statSync(p);
  const mocked = { ...actual, existsSync, statSync };
  return { ...mocked, default: mocked };
});

vi.mock('./utils/logger', () => ({
  default: {
    info: mocks.logInfo,
    error: mocks.logError,
    warn: mocks.logWarn,
    debug: mocks.logDebug,
  },
}));

vi.mock('child_process', async (importOriginal) => {
  const actual = await importOriginal<typeof import('child_process')>();
  const mocked = { ...actual, spawn: mocks.spawn };
  return { ...mocked, default: mocked };
});

import {
  STDERR_MAX_LINE_CHARS,
  STDERR_TRUNCATION_SUFFIX,
  createStderrLineReader,
  daemonStderrLogLevel,
  startBiorouterd,
} from './biorouterd';

describe('daemonStderrLogLevel', () => {
  it('maps the daemon tracing level onto the matching electron-log level', () => {
    expect(
      daemonStderrLogLevel(
        '  2026-07-26T18:40:14.289898Z ERROR biorouter::agents::agent: provider call failed'
      )
    ).toBe('error');
    expect(
      daemonStderrLogLevel(
        '  2026-07-26T18:40:14.289898Z  WARN biorouter::slash_commands: nothing configured'
      )
    ).toBe('warn');
    expect(
      daemonStderrLogLevel(
        '  2026-07-26T18:40:14.289898Z  INFO biorouter_server::routes: listening on 127.0.0.1'
      )
    ).toBe('info');
    expect(
      daemonStderrLogLevel(
        '  2026-07-26T18:40:14.289898Z DEBUG biorouter::config::base: loaded config'
      )
    ).toBe('debug');
    expect(
      daemonStderrLogLevel('  2026-07-26T18:40:14.289898Z TRACE mcp_client::transport: frame')
    ).toBe('debug');
  });

  it('reads the level when the line carries no timestamp', () => {
    expect(daemonStderrLogLevel('WARN biorouter::slash_commands: missing key')).toBe('warn');
    expect(daemonStderrLogLevel(' INFO biorouter: started')).toBe('info');
  });

  it('defaults an unparseable line to info, not error', () => {
    // Pretty-format continuation lines, and anything a child process prints
    // without a level, carry no severity — that is not the same as a failure.
    expect(daemonStderrLogLevel('    at crates/biorouter/src/workflow/mod.rs:42 on main')).toBe(
      'info'
    );
    expect(daemonStderrLogLevel('some library banner text')).toBe('info');
  });

  it('does not take a level word from the middle of a message', () => {
    expect(
      daemonStderrLogLevel(
        '  2026-07-26T18:40:14.289898Z  INFO biorouter::agents: tool returned ERROR to the model'
      )
    ).toBe('info');
  });

  // The Rust panic hook and anyhow's `Termination` impl write straight to
  // stderr, bypassing tracing entirely, so these lines carry no level word.
  // They are exactly the lines that must not be swallowed at `info`.
  it('classifies a raw Rust panic as error', () => {
    expect(
      daemonStderrLogLevel("thread 'main' panicked at crates/biorouter-server/src/main.rs:44:9:")
    ).toBe('error');
    // A panic on a tokio worker is just as much a failure as one on main.
    expect(
      daemonStderrLogLevel(
        "thread 'tokio-runtime-worker' panicked at crates/biorouter/src/x.rs:1:1:"
      )
    ).toBe('error');
    expect(daemonStderrLogLevel("  thread 'main' panicked at src/main.rs:1:1:")).toBe('error');
  });

  it('classifies anyhow/fatal startup output as error', () => {
    // `async fn main() -> anyhow::Result<()>` prints `Error: <chain>` on Err.
    expect(daemonStderrLogLevel('Error: failed to bind 127.0.0.1:0')).toBe('error');
    expect(daemonStderrLogLevel('error: failed to spawn biorouterd: ENOENT')).toBe('error');
    expect(daemonStderrLogLevel('Fatal: unrecoverable')).toBe('error');
  });

  it('leaves panic follow-up and mid-message mentions at their own level', () => {
    expect(daemonStderrLogLevel('note: run with `RUST_BACKTRACE=1` to display a backtrace')).toBe(
      'info'
    );
    expect(
      daemonStderrLogLevel(
        "  2026-07-26T18:40:14.289898Z  INFO biorouter::agents: thread 'main' panicked at was in the tool output"
      )
    ).toBe('info');
  });
});

// Holding a line back until its newline arrives is what makes the classifier
// and the startup fatal probe see whole lines — but a daemon (or a dependency
// it links) that emits one very long unterminated record would otherwise grow
// the held-back buffer for as long as the record lasts, with nothing reaching
// the ring to show why memory is climbing. The buffer must be bounded.
describe('createStderrLineReader line cap', () => {
  const CHUNK = 'x'.repeat(64 * 1024);
  const KIB_64_PER_MIB = 16;

  const feed = (reader: ReturnType<typeof createStderrLineReader>, mib: number) => {
    const chunk = Buffer.from(CHUNK, 'utf8');
    for (let i = 0; i < mib * KIB_64_PER_MIB; i++) reader.push(chunk);
  };

  it('caps one unterminated record instead of buffering all of it', () => {
    const lines: string[] = [];
    const reader = createStderrLineReader((line) => lines.push(line));

    reader.push(Buffer.from('  2026-07-26T18:40:14.289898Z ERROR biorouter::agents: ', 'utf8'));
    feed(reader, 1);
    expect(lines).toHaveLength(0);

    reader.push(Buffer.from('\n', 'utf8'));
    expect(lines).toHaveLength(1);
    // Nothing like the 1 MiB that arrived: exactly the cap plus the marker.
    expect(lines[0]!.length).toBe(STDERR_MAX_LINE_CHARS + STDERR_TRUNCATION_SUFFIX.length);
    // The prefix is the part worth keeping: both the classifier and
    // checkServerStatus's fatal predicate key off the head of the line.
    expect(lines[0]!.startsWith('  2026-07-26T18:40:14.289898Z ERROR biorouter::agents: x')).toBe(
      true
    );
    expect(daemonStderrLogLevel(lines[0]!)).toBe('error');
    // A reader can tell the line was cut rather than silently believing it.
    expect(lines[0]!.endsWith(STDERR_TRUNCATION_SUFFIX)).toBe(true);
    expect(lines[0]!).toContain('truncated');
  });

  // The cap must be invisible to every line the daemon actually writes, so
  // check both sides of the boundary — an off-by-one here would quietly start
  // stamping `…[truncated]` onto complete lines.
  it('leaves a line at or under the cap untouched', () => {
    const emitFor = (bodyLength: number) => {
      const lines: string[] = [];
      const reader = createStderrLineReader((line) => lines.push(line));
      reader.push(Buffer.from(`${'y'.repeat(bodyLength)}\n`, 'utf8'));
      return lines[0]!;
    };

    const atCap = emitFor(STDERR_MAX_LINE_CHARS);
    expect(atCap).toBe('y'.repeat(STDERR_MAX_LINE_CHARS));

    const overByOne = emitFor(STDERR_MAX_LINE_CHARS + 1);
    expect(overByOne).toBe('y'.repeat(STDERR_MAX_LINE_CHARS) + STDERR_TRUNCATION_SUFFIX);
  });

  it('retains the same amount however much unterminated input arrives', () => {
    const retained = (mib: number) => {
      const lines: string[] = [];
      const reader = createStderrLineReader((line) => lines.push(line));
      reader.push(Buffer.from('ERROR biorouter::agents: ', 'utf8'));
      feed(reader, mib);
      reader.push(Buffer.from('\n', 'utf8'));
      return lines[0]!.length;
    };

    expect(retained(8)).toBe(retained(1));
  });

  it('resumes normally on the line after a truncated one', () => {
    const lines: string[] = [];
    const reader = createStderrLineReader((line) => lines.push(line));

    reader.push(Buffer.from('ERROR biorouter::agents: ', 'utf8'));
    feed(reader, 1);
    reader.push(
      Buffer.from('\n  2026-07-26T18:40:14.289898Z  INFO biorouter_server: listening\n', 'utf8')
    );

    expect(lines).toHaveLength(2);
    expect(lines[1]).toBe('  2026-07-26T18:40:14.289898Z  INFO biorouter_server: listening');
    expect(daemonStderrLogLevel(lines[1]!)).toBe('info');
  });

  it('flushes a capped line when the daemon dies mid-record', () => {
    const lines: string[] = [];
    const reader = createStderrLineReader((line) => lines.push(line));

    reader.push(Buffer.from("thread 'main' panicked at src/main.rs:44:9: ", 'utf8'));
    feed(reader, 1);
    reader.flush();

    expect(lines).toHaveLength(1);
    expect(lines[0]!.length).toBe(STDERR_MAX_LINE_CHARS + STDERR_TRUNCATION_SUFFIX.length);
    // Still trips checkServerStatus's fatal predicate, which reads the head.
    expect(lines[0]!.trim().toLowerCase().startsWith("thread 'main' panicked at")).toBe(true);
    // And flushing again must not re-emit it.
    reader.flush();
    expect(lines).toHaveLength(1);
  });
});

describe('biorouterd stderr routing', () => {
  // Capture the handlers `startBiorouterd` registers so a test can drive the
  // stderr stream chunk by chunk and close the child, exactly as Node would.
  const stderrHandlers: Record<string, (arg?: unknown) => void> = {};
  const childHandlers: Record<string, (arg?: unknown) => void> = {};
  const previousSharedDaemon = process.env.BIOROUTER_SHARED_DAEMON;

  const mockChild = () => {
    for (const k of Object.keys(stderrHandlers)) delete stderrHandlers[k];
    for (const k of Object.keys(childHandlers)) delete childHandlers[k];
    mocks.spawn.mockReturnValue({
      stdout: { on: vi.fn() },
      stderr: {
        on: vi.fn((event: string, handler: (arg?: unknown) => void) => {
          stderrHandlers[event] = handler;
        }),
      },
      on: vi.fn((event: string, handler: (arg?: unknown) => void) => {
        childHandlers[event] = handler;
      }),
      kill: vi.fn(),
      unref: vi.fn(),
    });
  };

  const write = (s: string) => stderrHandlers['data']?.(Buffer.from(s, 'utf8'));

  beforeEach(() => {
    // This suite drives the legacy stderr reader through a directly spawned
    // child. Default shared-daemon approval admission is covered separately.
    process.env.BIOROUTER_SHARED_DAEMON = '0';
    mocks.logInfo.mockClear();
    mocks.logError.mockClear();
    mocks.logWarn.mockClear();
    mocks.logDebug.mockClear();
    mocks.spawn.mockReset();
    mockChild();
  });

  afterAll(() => {
    if (previousSharedDaemon === undefined) delete process.env.BIOROUTER_SHARED_DAEMON;
    else process.env.BIOROUTER_SHARED_DAEMON = previousSharedDaemon;
  });

  const joined = (calls: unknown[][]) => calls.map((c) => String(c[0])).join('\n');

  it('logs each daemon stderr line at its own level', async () => {
    const app = { isPackaged: false, on: vi.fn() } as unknown as App;
    const result = await startBiorouterd({ app, serverSecret: 'secret', dir: process.cwd() });

    expect(stderrHandlers['data']).toBeDefined();
    stderrHandlers['data']!(
      Buffer.from(
        [
          '  2026-07-26T18:40:14.289898Z  INFO biorouter_server: listening',
          '  2026-07-26T18:40:14.289898Z  WARN biorouter::config: deprecated key',
          '  2026-07-26T18:40:14.289898Z ERROR biorouter::agents::agent: provider call failed',
          '  2026-07-26T18:40:14.289898Z DEBUG biorouter::session: opened db',
          '    at crates/biorouter/src/session/mod.rs:12',
          '',
        ].join('\n')
      )
    );

    expect(joined(mocks.logError.mock.calls)).toContain('provider call failed');
    expect(mocks.logError.mock.calls).toHaveLength(1);
    expect(joined(mocks.logWarn.mock.calls)).toContain('deprecated key');
    expect(mocks.logWarn.mock.calls).toHaveLength(1);
    expect(joined(mocks.logDebug.mock.calls)).toContain('opened db');
    expect(mocks.logDebug.mock.calls).toHaveLength(1);
    // startBiorouterd logs its own startup info lines too, so assert on content.
    expect(joined(mocks.logInfo.mock.calls)).toContain('listening');
    expect(joined(mocks.logInfo.mock.calls)).toContain('session/mod.rs:12');

    // Every stderr line still reaches the bounded ring the startup probe reads.
    expect(result.errorLog).toHaveLength(5);
  });

  // Node stream chunks are not line-framed: a single logical line can arrive
  // in two `data` events. Classifying per chunk turns `ERROR` into the
  // fragments `ER` and `ROR …`, both unparseable, both logged at info.
  it('reassembles a line split across two stream chunks', async () => {
    const app = { isPackaged: false, on: vi.fn() } as unknown as App;
    const result = await startBiorouterd({ app, serverSecret: 'secret', dir: process.cwd() });

    write('  2026-07-26T18:40:14.289898Z ER');
    write('ROR biorouter::agents::agent: provider call failed\n');

    expect(mocks.logError.mock.calls).toHaveLength(1);
    expect(joined(mocks.logError.mock.calls)).toContain('provider call failed');
    expect(result.errorLog).toEqual([
      '  2026-07-26T18:40:14.289898Z ERROR biorouter::agents::agent: provider call failed',
    ]);
  });

  // The startup probe scans the ring for `thread 'main' panicked at` /
  // `error:`. If the ring holds chunk fragments instead of whole lines, a
  // split panic is invisible and startup hangs for the full 10s poll timeout.
  it('keeps a split panic line intact for the startup fatal probe', async () => {
    const app = { isPackaged: false, on: vi.fn() } as unknown as App;
    const result = await startBiorouterd({ app, serverSecret: 'secret', dir: process.cwd() });

    write("thread 'main' pan");
    write('icked at crates/biorouter-server/src/main.rs:44:9:\n');
    write('called `Option::unwrap()` on a `None` value\n');

    expect(result.errorLog[0]).toBe(
      "thread 'main' panicked at crates/biorouter-server/src/main.rs:44:9:"
    );
    expect(
      result.errorLog.some((l) => l.trim().toLowerCase().startsWith("thread 'main' panicked at"))
    ).toBe(true);
    expect(joined(mocks.logError.mock.calls)).toContain('panicked at');
  });

  // A daemon that dies mid-line leaves the last, most interesting line
  // unterminated. It must still be logged and recorded, not dropped.
  it('flushes an unterminated final line when the child closes', async () => {
    const app = { isPackaged: false, on: vi.fn() } as unknown as App;
    const result = await startBiorouterd({ app, serverSecret: 'secret', dir: process.cwd() });

    write('Error: failed to bind 127.0.0.1:0');
    expect(result.errorLog).toHaveLength(0);

    childHandlers['close']?.(1);

    expect(result.errorLog).toEqual(['Error: failed to bind 127.0.0.1:0']);
    expect(joined(mocks.logError.mock.calls)).toContain('failed to bind');

    // Flushing twice (stream `end` then child `close`) must not duplicate it.
    stderrHandlers['end']?.();
    expect(result.errorLog).toHaveLength(1);
  });

  it('does not split a multi-byte character that straddles a chunk boundary', async () => {
    const app = { isPackaged: false, on: vi.fn() } as unknown as App;
    const result = await startBiorouterd({ app, serverSecret: 'secret', dir: process.cwd() });

    const full = Buffer.from('  2026-07-26T18:40:14.289898Z  WARN biorouter: café ☕\n', 'utf8');
    const cut = full.indexOf(Buffer.from('☕', 'utf8')) + 1;
    stderrHandlers['data']!(full.subarray(0, cut));
    stderrHandlers['data']!(full.subarray(cut));

    expect(result.errorLog).toEqual(['  2026-07-26T18:40:14.289898Z  WARN biorouter: café ☕']);
    expect(joined(mocks.logWarn.mock.calls)).toContain('café ☕');
  });

  it('strips a CRLF carriage return rather than trailing it onto the line', async () => {
    const app = { isPackaged: false, on: vi.fn() } as unknown as App;
    const result = await startBiorouterd({ app, serverSecret: 'secret', dir: process.cwd() });

    write('  2026-07-26T18:40:14.289898Z  INFO biorouter_server: listening\r\n');

    expect(result.errorLog).toEqual([
      '  2026-07-26T18:40:14.289898Z  INFO biorouter_server: listening',
    ]);
  });

  // A single unterminated record must not grow the main process's heap, and —
  // the perverse part — must not leave the ring empty while it does so: memory
  // climbing with no diagnostic is the worst of both.
  it('bounds a huge unterminated record and still records it in the ring', async () => {
    const app = { isPackaged: false, on: vi.fn() } as unknown as App;
    const result = await startBiorouterd({ app, serverSecret: 'secret', dir: process.cwd() });

    write('  2026-07-26T18:40:14.289898Z ERROR biorouter::agents::agent: ');
    const filler = 'x'.repeat(64 * 1024);
    for (let i = 0; i < 16; i++) write(filler); // 1 MiB, no newline
    expect(result.errorLog).toHaveLength(0);

    write('\n  2026-07-26T18:40:14.289898Z  INFO biorouter_server: listening\n');

    expect(result.errorLog).toHaveLength(2);
    expect(result.errorLog[0]!.length).toBe(
      STDERR_MAX_LINE_CHARS + STDERR_TRUNCATION_SUFFIX.length
    );
    expect(result.errorLog[0]!).toContain('ERROR biorouter::agents::agent');
    // Classified from the retained prefix, and logged exactly once.
    expect(mocks.logError.mock.calls).toHaveLength(1);
    // The next line is unaffected by the one before it.
    expect(result.errorLog[1]).toBe(
      '  2026-07-26T18:40:14.289898Z  INFO biorouter_server: listening'
    );
    expect(joined(mocks.logInfo.mock.calls)).toContain('listening');
  });
});
