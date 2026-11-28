import { EventEmitter } from 'node:events';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { FN_DOUBLE_TAP_GAP_MS, FN_HOLD_MS } from './fnGesture';
import {
  FN_LISTENER_RESTART_MIN_MS,
  FN_MAX_LISTENERS,
  FN_PROTOCOL_VERSION,
  FnKeyService,
  LineSplitter,
  MAX_LINE,
  fnKeyHelperPath,
  parseFnHelperLine,
  type FnKeyStatus,
  type FnLineTrace,
  type HelperProcess,
} from './fnKeyHelper';
import type { FnIntent, FnNote } from './fnGesture';
import { KNOWN_FN_APPS } from './fnCollision';

const last = <T>(items: T[]): T | undefined => items[items.length - 1];

describe('the helper protocol', () => {
  // Rewritten for protocol version 3 (DD-101): the ready line says v 3, an fn down line names
  // the modifier held with it (null for none), and the chord and listeners lines are new.
  it('reads every line the helper writes (crates/biorouter-fnkey/src/protocol.rs)', () => {
    expect(
      parseFnHelperLine(
        '{"type":"ready","v":3,"permission":"granted","listening":true,"fnUsage":2,"t":900}'
      )
    ).toEqual({ type: 'ready', permission: 'granted', listening: true, fnUsage: 2, t: 900 });
    expect(
      parseFnHelperLine(
        '{"type":"ready","v":3,"permission":"denied","listening":false,"fnUsage":null}'
      )
    ).toEqual({ type: 'ready', permission: 'denied', listening: false, fnUsage: null, t: null });
    expect(parseFnHelperLine('{"type":"fn","down":true,"mod":null,"t":1200}')).toEqual({
      type: 'fn',
      down: true,
      t: 1200,
    });
    for (const mod of ['shift', 'control', 'option'] as const) {
      expect(parseFnHelperLine(`{"type":"fn","down":true,"mod":"${mod}","t":1200}`)).toEqual({
        type: 'fn',
        down: true,
        mod,
        t: 1200,
      });
      expect(parseFnHelperLine(`{"type":"chord","mod":"${mod}","t":1240}`)).toEqual({
        type: 'chord',
        mod,
        t: 1240,
      });
    }
    expect(
      parseFnHelperLine(
        '{"type":"listeners","apps":["com.electron.wispr-flow","com.example.hot-keys2"],"t":1300}'
      )
    ).toEqual({
      type: 'listeners',
      apps: ['com.electron.wispr-flow', 'com.example.hot-keys2'],
      t: 1300,
    });
    expect(parseFnHelperLine('{"type":"listeners","apps":[]}')).toEqual({
      type: 'listeners',
      apps: [],
      t: null,
    });
    expect(parseFnHelperLine('{"type":"fn","down":false,"t":1500}')).toEqual({
      type: 'fn',
      down: false,
      t: 1500,
    });
    expect(parseFnHelperLine('{"type":"held","t":1500}')).toEqual({ type: 'held', t: 1500 });
    expect(parseFnHelperLine('{"type":"other","t":7}')).toEqual({ type: 'other', t: 7 });
    // A line without a time is timed on arrival.
    expect(parseFnHelperLine('{"type":"fn","down":true}')).toEqual({
      type: 'fn',
      down: true,
      t: null,
    });
    expect(parseFnHelperLine('{"type":"permission","permission":"granted"}')).toEqual({
      type: 'permission',
      permission: 'granted',
    });
    expect(parseFnHelperLine('{"type":"listener-lost"}')).toEqual({ type: 'listener-lost' });
  });

  it('ignores fields a known line does not use, so a later optional field needs no new version', () => {
    expect(
      parseFnHelperLine(
        '{"type":"ready","v":3,"permission":"granted","listening":true,"fnUsage":0,"t":5,"tapOwner":"x"}'
      )
    ).toEqual({ type: 'ready', permission: 'granted', listening: true, fnUsage: 0, t: 5 });
    // An fn up line names no modifier; a stray one is a field it does not use.
    expect(parseFnHelperLine('{"type":"fn","down":false,"mod":"command","t":6}')).toEqual({
      type: 'fn',
      down: false,
      t: 6,
    });
    expect(
      parseFnHelperLine('{"type":"listeners","apps":["com.example.a"],"t":7,"names":["A"]}')
    ).toEqual({ type: 'listeners', apps: ['com.example.a'], t: 7 });
    expect(parseFnHelperLine('{"type":"held","t":9,"extra":[1,2]}')).toEqual({
      type: 'held',
      t: 9,
    });
    expect(parseFnHelperLine('{"type":"fn","down":true,"t":3,"source":1}')).toEqual({
      type: 'fn',
      down: true,
      t: 3,
    });
    expect(parseFnHelperLine('{"type":"other","t":4,"why":"key"}')).toEqual({
      type: 'other',
      t: 4,
    });
  });

  // Rewritten for protocol version 3 (DD-101): a version 2 helper is now another build's, refused
  // like version 1, because `other` and `held` mean something else there.
  it('names a ready line of another protocol version, whatever else it says', () => {
    expect(FN_PROTOCOL_VERSION).toBe(3);
    expect(
      parseFnHelperLine(
        '{"type":"ready","v":1,"permission":"granted","listening":true,"fnUsage":0,"t":900}'
      )
    ).toEqual({ type: 'incompatible', v: 1 });
    expect(
      parseFnHelperLine(
        '{"type":"ready","v":2,"permission":"granted","listening":true,"fnUsage":0,"t":900}'
      )
    ).toEqual({ type: 'incompatible', v: 2 });
    expect(parseFnHelperLine('{"type":"ready","v":4}')).toEqual({ type: 'incompatible', v: 4 });
  });

  it('refuses anything else', () => {
    for (const line of [
      '',
      'fn down',
      'null',
      '[]',
      '{"type":"fn"}',
      '{"type":"fn","down":"yes"}',
      '{"type":"fn","down":true,"t":-1}',
      '{"type":"fn","down":true,"t":"soon"}',
      '{"type":"held","t":"later"}',
      '{"type":"held","t":-3}',
      '{"type":"other","t":null}',
      '{"type":"ready","permission":"granted","listening":true,"fnUsage":null}',
      '{"type":"ready","v":"2","permission":"granted","listening":true,"fnUsage":null}',
      '{"type":"ready","v":3.5,"permission":"granted","listening":true,"fnUsage":null}',
      '{"type":"ready","v":3,"permission":"maybe","listening":true,"fnUsage":null}',
      '{"type":"ready","v":3,"permission":"granted","listening":true,"fnUsage":7}',
      '{"type":"ready","v":3,"permission":"granted","listening":true,"fnUsage":0,"t":-5}',
      '{"type":"ready","v":3,"permission":"granted","fnUsage":null}',
      '{"type":"permission","permission":"yes"}',
      '{"type":"key","code":0}',
      // A line longer than the protocol allows (4096 bytes since version 3; it was 512).
      `{"type":"other","pad":"${'x'.repeat(MAX_LINE)}"}`,
      // An fn down whose modifier is none of the three, Command included: no press at all.
      '{"type":"fn","down":true,"mod":"command","t":1}',
      '{"type":"fn","down":true,"mod":"Control","t":1}',
      '{"type":"fn","down":true,"mod":1,"t":1}',
      '{"type":"fn","down":true,"mod":["shift"],"t":1}',
      '{"type":"chord","t":1}',
      '{"type":"chord","mod":null,"t":1}',
      '{"type":"chord","mod":"command","t":1}',
      '{"type":"chord","mod":"control","t":-1}',
      '{"type":"listeners","t":1}',
      '{"type":"listeners","apps":"com.electron.wispr-flow","t":1}',
      '{"type":"listeners","apps":[7],"t":1}',
      '{"type":"listeners","apps":["com.example.a"],"t":"now"}',
    ]) {
      expect(parseFnHelperLine(line), line).toBeNull();
    }
  });

  it('drops a listeners line with a bad id, more than 16 ids, or more than 4096 bytes', () => {
    expect(FN_MAX_LISTENERS).toBe(16);
    expect(MAX_LINE).toBe(4096);
    const listeners = (apps: unknown[], extra = '') =>
      `{"type":"listeners","apps":${JSON.stringify(apps)},"t":5${extra}}`;
    const ids = (n: number) => Array.from({ length: n }, (_, i) => `com.example.app${i}`);
    expect(parseFnHelperLine(listeners(ids(16)))).toEqual({
      type: 'listeners',
      apps: ids(16),
      t: 5,
    });
    expect(parseFnHelperLine(listeners(ids(17)))).toBeNull();
    // An id is letters, digits, dots and hyphens, from 1 to 255 of them.
    expect(parseFnHelperLine(listeners(['a'.repeat(255)]))).toMatchObject({ type: 'listeners' });
    for (const bad of [
      '',
      'a'.repeat(256),
      'com.example app',
      'com.example/app',
      'com.example_app',
      'com.exämple.app',
      'com.example.app\n',
      '../../etc',
      '<b>Wispr</b>',
    ]) {
      expect(parseFnHelperLine(listeners(['com.example.ok', bad])), JSON.stringify(bad)).toBeNull();
    }
    for (const bad of [null, 1, true, ['com.example.a'], { id: 'com.example.a' }]) {
      expect(parseFnHelperLine(listeners([bad])), JSON.stringify(bad)).toBeNull();
    }
    // A line whose ids are fine but that is longer than the protocol allows, in bytes.
    const padTo = (bytes: number, char: string) => {
      const base = listeners(['com.example.a'], ',"pad":""');
      const width = Buffer.byteLength(char);
      return base.replace(
        '"pad":""',
        `"pad":"${char.repeat(Math.ceil((bytes - base.length) / width))}"`
      );
    };
    expect(parseFnHelperLine(padTo(MAX_LINE, 'x'))).toMatchObject({ type: 'listeners' });
    expect(Buffer.byteLength(padTo(MAX_LINE + 1, 'x'))).toBeGreaterThan(MAX_LINE);
    expect(parseFnHelperLine(padTo(MAX_LINE + 1, 'x'))).toBeNull();
    // Counted in bytes, not characters: a multi-byte pad under 4096 characters is still too long.
    const wide = padTo(MAX_LINE + 1, '\u00e9');
    expect(wide.length).toBeLessThan(MAX_LINE);
    expect(parseFnHelperLine(wide)).toBeNull();
  });

  it('splits a stream into lines across chunks and drops an overlong one', () => {
    const lines: string[] = [];
    const splitter = new LineSplitter((line) => lines.push(line));
    splitter.push('{"type":"fn",');
    splitter.push('"down":true}\n{"type":"other"}\n\n{"type"');
    splitter.push(':"fn","down":false}\n');
    expect(lines).toEqual([
      '{"type":"fn","down":true}',
      '{"type":"other"}',
      '{"type":"fn","down":false}',
    ]);
    // Longer than the protocol allows (4096 since version 3; this was 2000 against 512).
    splitter.push('x'.repeat(MAX_LINE + 1));
    splitter.push('{"type":"other"}\n');
    expect(last(lines)).toBe('{"type":"other"}');
  });
});

describe('where the helper is', () => {
  const resources = '/Applications/Biorouter.app/Contents/Resources';
  const cwd = '/repo/ui/desktop';

  it('a packaged app uses only its own copy beside biorouterd, whatever the environment says', () => {
    const packaged = path.join(resources, 'bin', 'biorouter-fnkey');
    expect(
      fnKeyHelperPath(
        { isPackaged: true },
        { BIOROUTER_FNKEY_HELPER: '/tmp/other' },
        (file) => file === packaged || file === '/tmp/other',
        resources,
        cwd
      )
    ).toBe(packaged);
    expect(fnKeyHelperPath({ isPackaged: true }, {}, () => false, resources, cwd)).toBeNull();
  });

  it('a development app takes the override, then a workspace build', () => {
    expect(
      fnKeyHelperPath(
        { isPackaged: false },
        { BIOROUTER_FNKEY_HELPER: '/tmp/fake-helper' },
        (file) => file === '/tmp/fake-helper',
        resources,
        cwd
      )
    ).toBe('/tmp/fake-helper');
    const debug = path.join(cwd, '..', '..', 'target', 'debug', 'biorouter-fnkey');
    expect(
      fnKeyHelperPath({ isPackaged: false }, {}, (file) => file === debug, resources, cwd)
    ).toBe(debug);
  });
});

class FakeHelper extends EventEmitter implements HelperProcess {
  written: string[] = [];
  ended = false;
  killed = false;
  stdout = new EventEmitter();
  stdin = {
    write: (text: string) => this.written.push(text),
    end: () => {
      this.ended = true;
    },
  };
  kill() {
    this.killed = true;
  }
  say(line: string) {
    this.stdout.emit('data', Buffer.from(`${line}\n`));
  }
}

describe('the fn key service', () => {
  let now = 0;
  let helpers: FakeHelper[];
  let seen: { intent: FnIntent; gesture: number }[];
  let statuses: FnKeyStatus[];
  let traces: FnLineTrace[];
  let notes: { note: FnNote; gesture: number }[];
  beforeEach(() => {
    vi.useFakeTimers();
    now = 0;
    helpers = [];
    seen = [];
    statuses = [];
    traces = [];
    notes = [];
  });
  afterEach(() => vi.useRealTimers());

  const intents = () => seen.map((s) => s.intent);
  const service = (platform: string = 'darwin', helperPath: string | null = '/h') =>
    new FnKeyService({
      platform,
      helperPath,
      spawn: () => {
        const helper = new FakeHelper();
        helpers.push(helper);
        return helper;
      },
      onIntent: (intent, gesture) => seen.push({ intent, gesture }),
      onStatus: (status) => statuses.push(status),
      onTrace: (trace) => traces.push(trace),
      onNote: (note, gesture) => notes.push({ note, gesture }),
      now: () => now,
    });
  const advance = (ms: number) => {
    now += ms;
    vi.advanceTimersByTime(ms);
  };
  // Protocol version 3 (DD-101); these were version 2.
  const READY = '{"type":"ready","v":3,"permission":"granted","listening":true,"fnUsage":0}';
  const DENIED = '{"type":"ready","v":3,"permission":"denied","listening":false,"fnUsage":2}';
  const readyAt = (t: number) =>
    `{"type":"ready","v":3,"permission":"granted","listening":true,"fnUsage":0,"t":${t}}`;
  const fnLine = (down: boolean, t?: number) =>
    t === undefined ? `{"type":"fn","down":${down}}` : `{"type":"fn","down":${down},"t":${t}}`;

  it('starts no listener until the shortcut is on, and none off macOS', () => {
    const off = service();
    expect(helpers).toHaveLength(0);
    expect(off.status()).toMatchObject({ supported: true, active: false, listening: false });
    const windows = service('win32');
    windows.setActive(true);
    expect(helpers).toHaveLength(0);
    expect(windows.status().supported).toBe(false);
    const missing = service('darwin', null);
    missing.setActive(true);
    expect(helpers).toHaveLength(0);
  });

  it('turns the helper’s fn reports into gesture intents, a hold confirmed by its held line', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(READY);
    expect(fnKey.status()).toMatchObject({ permission: 'granted', listening: true, fnUsage: 0 });
    helpers[0].say(fnLine(true));
    expect(intents()).toEqual(['prime']);
    // Main's clock passing the threshold confirms nothing.
    advance(FN_HOLD_MS * 2);
    expect(intents()).toEqual(['prime']);
    helpers[0].say('{"type":"held"}');
    expect(intents()).toEqual(['prime', 'start-hold']);
    advance(1500);
    helpers[0].say(fnLine(false));
    expect(seen).toEqual([
      { intent: 'prime', gesture: 1 },
      { intent: 'start-hold', gesture: 1 },
      { intent: 'end-hold', gesture: 1 },
    ]);
  });

  it('arms no timer for a press', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(READY);
    const timers = vi.spyOn(globalThis, 'setTimeout');
    // A double-tap, then a hold.
    for (const [line, wait] of [
      [fnLine(true), 90],
      [fnLine(false), 130],
      [fnLine(true), 90],
      [fnLine(false), 1000],
      [fnLine(true), FN_HOLD_MS],
      ['{"type":"held"}', 700],
      [fnLine(false), 0],
    ] as const) {
      helpers[0].say(line);
      expect(vi.getTimerCount()).toBe(0);
      advance(wait);
    }
    expect(timers).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
    expect(intents()).toEqual([
      'prime',
      'cancel-hold',
      'prime',
      'toggle',
      'prime',
      'start-hold',
      'end-hold',
    ]);
    timers.mockRestore();
  });

  it('keeps a tap a tap when main reads its release late (row D)', () => {
    const fnKey = service();
    fnKey.setActive(true);
    now = 1_000;
    helpers[0].say(readyAt(80_000));
    helpers[0].say(fnLine(true, 80_000));
    // Main is busy for 400 ms: the rest of the double-tap is read together, after it.
    now = 1_400;
    for (const [down, t] of [
      [false, 80_090],
      [true, 80_220],
      [false, 80_310],
    ] as const)
      helpers[0].say(fnLine(down, t));
    expect(intents()).toEqual(['prime', 'cancel-hold', 'prime', 'toggle']);
  });

  it('keeps a hold a hold when its lines reach a busy main thread late, together', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(READY);
    now = 10_000;
    helpers[0].say('{"type":"other","t":5000}');
    // Main was busy for two seconds: the down and up of a 700 ms hold arrive at once, with no
    // held line between them, so the release's time decides.
    now = 12_000;
    helpers[0].say(fnLine(true, 6000));
    helpers[0].say(fnLine(false, 6700));
    expect(intents()).toEqual(['prime', 'start-hold', 'end-hold']);
  });

  it('the ready line lines the clocks up, so even the first hold read late keeps its length', () => {
    const fnKey = service();
    fnKey.setActive(true);
    now = 1_000;
    helpers[0].say(readyAt(51_000));
    // Main was busy: the hold's down and up, 50 s on the helper's clock, arrive together.
    now = 5_000;
    helpers[0].say(fnLine(true, 51_200));
    helpers[0].say(fnLine(false, 52_000));
    expect(intents()).toEqual(['prime', 'start-hold', 'end-hold']);
  });

  it('times lines stamped 0 by their arrival, so a quick tap is not a hold (row K)', () => {
    const fnKey = service();
    fnKey.setActive(true);
    now = 5_000;
    helpers[0].say(readyAt(1_000_000));
    for (const [down, at] of [
      [true, 6_000],
      [false, 6_090],
      [true, 6_220],
      [false, 6_310],
    ] as const) {
      now = at;
      helpers[0].say(fnLine(down, 0));
    }
    expect(intents()).toEqual(['prime', 'cancel-hold', 'prime', 'toggle']);
    expect(traces.filter((t) => t.line.startsWith('fn-')).map((t) => t.timedBy)).toEqual([
      'arrival',
      'arrival',
      'arrival',
      'arrival',
    ]);
  });

  it('times lines on another clock by their arrival (row B, and a clock that runs ahead)', () => {
    // Row B: the ready line in uptime ms, the events in mach ticks / 1e6 (125/3 timebase).
    const ticks = (uptimeMs: number) => Math.round((uptimeMs * 3) / 125);
    const uptime = 1_166_666_667;
    const fnKey = service();
    fnKey.setActive(true);
    now = 5_000;
    helpers[0].say(readyAt(uptime));
    for (const [down, real] of [
      [true, 1_000],
      [false, 1_090],
      [true, 1_220],
      [false, 1_310],
    ] as const) {
      now = 5_000 + real;
      helpers[0].say(fnLine(down, ticks(uptime + real)));
    }
    expect(intents()).toEqual(['prime', 'cancel-hold', 'prime', 'toggle']);
    // A clock three hours ahead maps into the future: timed by arrival, and the offset holds.
    seen = [];
    now = 20_000;
    helpers[0].say(fnLine(true, uptime + 3 * 3600_000));
    now = 20_090;
    helpers[0].say(fnLine(false, uptime + 3 * 3600_000 + 90));
    expect(intents()).toEqual(['prime', 'cancel-hold']);
    expect(last(traces)).toMatchObject({ line: 'fn-up', mappedAt: 20_090, timedBy: 'arrival' });
    // The real clock still maps as before.
    now = 30_000;
    helpers[0].say(fnLine(true, uptime + 24_990));
    expect(last(traces)).toMatchObject({ line: 'fn-down', mappedAt: 29_990, timedBy: 'helper' });
  });

  it('a time more than ten seconds before its arrival is timed by the arrival', () => {
    const fnKey = service();
    fnKey.setActive(true);
    now = 1_000;
    helpers[0].say(readyAt(100_000));
    // Written at helper 108 000 (main 9 000), read at 20 000: eleven seconds is not believed.
    now = 20_000;
    helpers[0].say(fnLine(true, 108_000));
    expect(last(traces)).toMatchObject({ mappedAt: 20_000, timedBy: 'arrival' });
    // A plausible time still maps by the helper's clock.
    now = 20_050;
    helpers[0].say(fnLine(false, 119_040));
    expect(last(traces)).toMatchObject({ mappedAt: 20_040, timedBy: 'helper' });
    expect(intents()).toEqual(['prime', 'cancel-hold']);
  });

  it('a ready line read late is corrected by a sooner line, so later late lines keep their time', () => {
    const fnKey = service();
    fnKey.setActive(true);
    // The ready line was written at helper 10 000 and read two seconds late.
    now = 2_000;
    helpers[0].say(readyAt(10_000));
    // A down written at helper 12 500 is read 5 ms after it was written: it maps 1 995 ms
    // ahead of its arrival, so the offset moves and the down is placed at its arrival.
    now = 2_505;
    helpers[0].say(fnLine(true, 12_500));
    expect(last(traces)).toMatchObject({ mappedAt: 2_505, timedBy: 'helper' });
    // Main is busy: the release written 90 ms later is read 400 ms later, and keeps its 90 ms.
    now = 2_905;
    helpers[0].say(fnLine(false, 12_590));
    expect(last(traces)).toMatchObject({ mappedAt: 2_595, timedBy: 'helper' });
    expect(intents()).toEqual(['prime', 'cancel-hold']);
  });

  it('refuses a helper of another protocol version: failed, stopped, and not started again', () => {
    const fnKey = service();
    fnKey.setActive(true);
    // Since version 3 (DD-101) a version 2 helper is another build's, refused as version 1 was.
    helpers[0].say(
      '{"type":"ready","v":2,"permission":"granted","listening":true,"fnUsage":0,"t":900}'
    );
    expect(fnKey.status()).toMatchObject({ failed: true, listening: false });
    expect(last(statuses)).toMatchObject({ failed: true, listening: false });
    expect(helpers[0].killed).toBe(true);
    // Its lines are not read, it is not restarted when it exits, and a refresh leaves it.
    helpers[0].say(fnLine(true));
    expect(intents()).toEqual([]);
    helpers[0].emit('exit', 0);
    advance(60_000);
    fnKey.refresh();
    expect(helpers).toHaveLength(1);
    // Turning the switch off and on tries again (MSG-FN-FAILED).
    fnKey.setActive(false);
    fnKey.setActive(true);
    expect(helpers).toHaveLength(2);
    expect(fnKey.status().failed).toBe(false);
    helpers[1].say(READY);
    expect(fnKey.status()).toMatchObject({ failed: false, listening: true });
  });

  it('a double-tap that arrives late is still a double-tap', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(READY);
    now = 10_000;
    for (const [down, t] of [
      [true, 10_000],
      [false, 10_080],
      [true, 10_200],
      [false, 10_260],
    ] as const)
      helpers[0].say(fnLine(down, t));
    expect(seen).toEqual([
      { intent: 'prime', gesture: 1 },
      { intent: 'cancel-hold', gesture: 1 },
      { intent: 'prime', gesture: 1 },
      { intent: 'toggle', gesture: 1 },
    ]);
  });

  it('a key the helper reports between two taps makes them two taps', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(READY);
    now = 10_000;
    helpers[0].say(fnLine(true, 10_000));
    helpers[0].say(fnLine(false, 10_080));
    helpers[0].say('{"type":"other","t":10150}');
    helpers[0].say(fnLine(true, 10_200));
    helpers[0].say(fnLine(false, 10_260));
    expect(seen.map((s) => `${s.intent}:${s.gesture}`)).toEqual([
      'prime:1',
      'cancel-hold:1',
      'prime:2',
      'cancel-hold:2',
    ]);
  });

  it('fn with another key reaches the gesture as a cancel', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(READY);
    helpers[0].say(fnLine(true));
    advance(50);
    helpers[0].say('{"type":"other"}');
    advance(FN_HOLD_MS * 3);
    helpers[0].say(fnLine(false));
    expect(intents()).toEqual(['prime', 'cancel-hold']);
  });

  it('turning the shortcut off stops the helper and ends a hold in progress', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(READY);
    helpers[0].say(fnLine(true));
    advance(FN_HOLD_MS);
    helpers[0].say('{"type":"held"}');
    fnKey.setActive(false);
    expect(intents()).toEqual(['prime', 'start-hold', 'end-hold']);
    expect(helpers[0].ended).toBe(true);
    expect(helpers[0].killed).toBe(true);
    expect(fnKey.status()).toMatchObject({ active: false, listening: false });
    // Its late lines are not listened to.
    helpers[0].say(fnLine(true));
    helpers[0].say('{"type":"held"}');
    expect(intents()).toEqual(['prime', 'start-hold', 'end-hold']);
  });

  it('reports a missing permission, asks only when told to, and restarts on a grant', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(DENIED);
    expect(fnKey.status()).toMatchObject({ permission: 'denied', listening: false, fnUsage: 2 });
    expect(helpers[0].written).toEqual([]);
    expect(fnKey.requestPermission()).toBe(true);
    expect(helpers[0].written).toEqual(['request\n']);
    helpers[0].say('{"type":"permission","permission":"granted"}');
    // A grant reaches a new process, so the helper is started again.
    expect(helpers).toHaveLength(2);
    expect(helpers[0].killed).toBe(true);
    helpers[1].say(READY);
    expect(fnKey.status()).toMatchObject({ permission: 'granted', listening: true });
  });

  it('a refresh starts a fresh helper only while nothing listens, at most every two seconds', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(DENIED);
    fnKey.refresh();
    expect(helpers).toHaveLength(2);
    fnKey.refresh();
    expect(helpers).toHaveLength(2);
    advance(2000);
    helpers[1].say(READY);
    fnKey.refresh();
    expect(helpers).toHaveLength(2);
  });

  it('restarts a helper that dies, backing off, and gives up after five in a minute', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(READY);
    for (let i = 0; i < 5; i += 1) {
      last(helpers)!.emit('exit', 1);
      expect(fnKey.status().listening).toBe(false);
      advance(1000 * 2 ** i);
      expect(helpers).toHaveLength(i + 2);
    }
    last(helpers)!.emit('exit', 1);
    advance(60_000);
    expect(helpers).toHaveLength(6);
    expect(fnKey.status().failed).toBe(true);
    // A refresh (Settings, or a window gaining focus) tries again.
    fnKey.refresh();
    expect(helpers).toHaveLength(7);
    expect(fnKey.status().failed).toBe(false);
  });

  it('a helper that dies during a hold ends the hold', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(READY);
    helpers[0].say(fnLine(true));
    helpers[0].say('{"type":"held"}');
    helpers[0].emit('exit', null);
    expect(intents()).toEqual(['prime', 'start-hold', 'end-hold']);
  });

  it('a helper whose listener macOS refused despite the grant counts as failed', () => {
    const fnKey = service();
    fnKey.setActive(true);
    last(helpers)!.say(
      '{"type":"ready","v":3,"permission":"granted","listening":false,"fnUsage":0}'
    );
    expect(fnKey.status()).toMatchObject({ failed: true, listening: false });
    fnKey.setActive(false);
    fnKey.setActive(true);
    expect(fnKey.status().failed).toBe(false);
  });

  it('ignores lines it cannot read, and traces them without their content', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say('{"type":"ready","permission":"granted"}');
    helpers[0].say('hello secret words');
    expect(fnKey.status().permission).toBe('unknown');
    expect(last(statuses)?.active).toBe(true);
    expect(traces.map((t) => t.line)).toEqual(['unreadable', 'unreadable']);
    expect(JSON.stringify(traces)).not.toContain('secret');
  });

  it('traces every line it reads with the helper, mapped and arrival times', () => {
    const fnKey = service();
    fnKey.setActive(true);
    now = 1_000;
    helpers[0].say(readyAt(40_000));
    for (const [line, at] of [
      [fnLine(true, 40_150), 1_200],
      ['{"type":"held","t":40450}', 1_480],
      ['{"type":"other","t":40460}', 1_490],
      [fnLine(false, 40_500), 1_520],
      ['{"type":"permission","permission":"granted"}', 1_600],
      ['{"type":"listener-lost"}', 1_700],
    ] as const) {
      now = at;
      helpers[0].say(line);
    }
    expect(traces).toEqual([
      { line: 'ready', helperT: 40_000, mappedAt: null, arrivalAt: 1_000, timedBy: null },
      // Since version 3 an fn down trace names the modifier held with it (none here).
      {
        line: 'fn-down',
        helperT: 40_150,
        mappedAt: 1_150,
        arrivalAt: 1_200,
        timedBy: 'helper',
        mod: null,
      },
      { line: 'held', helperT: 40_450, mappedAt: 1_450, arrivalAt: 1_480, timedBy: 'helper' },
      { line: 'other', helperT: 40_460, mappedAt: 1_460, arrivalAt: 1_490, timedBy: 'helper' },
      { line: 'fn-up', helperT: 40_500, mappedAt: 1_500, arrivalAt: 1_520, timedBy: 'helper' },
      { line: 'permission', helperT: null, mappedAt: null, arrivalAt: 1_600, timedBy: null },
      { line: 'listener-lost', helperT: null, mappedAt: null, arrivalAt: 1_700, timedBy: null },
    ]);
    expect(intents()).toEqual(['prime', 'start-hold', 'cancel-hold']);
  });

  // ── Protocol version 3: the keys, the colliding app and the restart (DD-101) ──
  // A service whose keys were never set uses the default modifier, Option since the owner's
  // ruling O-49 (2026-11-27); the tests below that press it without setting keys pressed Control
  // under O-46 and were rewritten to Option. A test that sets Control itself keeps it.

  const WISPR = '{"type":"listeners","apps":["com.example.hotkeys","com.electron.wispr-flow"]}';
  const NO_LISTED = '{"type":"listeners","apps":["com.example.hotkeys"]}';
  const NONE = '{"type":"listeners","apps":[]}';
  const modDown = (mod: string | null, t?: number) =>
    `{"type":"fn","down":true,"mod":${mod === null ? 'null' : `"${mod}"`}${t === undefined ? '' : `,"t":${t}`}}`;
  const HELD = '{"type":"held"}';

  it('refuses a version 1 helper too', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say('{"type":"ready","v":1,"permission":"granted","listening":true,"fnUsage":0}');
    expect(fnKey.status()).toMatchObject({ failed: true, listening: false });
    expect(helpers[0].killed).toBe(true);
  });

  it('passes the modifier on fn down and a chord on to the gesture', () => {
    const fnKey = service();
    fnKey.setKeys({ combination: 'modifier', modifier: 'option' });
    fnKey.setActive(true);
    helpers[0].say(READY);
    // A bare press goes unclaimed and an Option chord within the grace claims it.
    helpers[0].say(modDown(null));
    expect(intents()).toEqual([]);
    helpers[0].say('{"type":"chord","mod":"option"}');
    expect(intents()).toEqual(['prime']);
    helpers[0].say(HELD);
    helpers[0].say(fnLine(false));
    expect(intents()).toEqual(['prime', 'start-hold', 'end-hold']);
    // Option held from before fn went down.
    helpers[0].say(modDown('option'));
    helpers[0].say(HELD);
    helpers[0].say(fnLine(false));
    expect(intents()).toEqual([
      'prime',
      'start-hold',
      'end-hold',
      'prime',
      'start-hold',
      'end-hold',
    ]);
    expect(traces.filter((t) => t.line === 'fn-down').map((t) => t.mod)).toEqual([null, 'option']);
    expect(last(traces.filter((t) => t.line === 'chord'))).toMatchObject({ mod: 'option' });
  });

  it('keys set before the switch reach the first press', () => {
    const fnKey = service();
    fnKey.setKeys({ combination: 'modifier', modifier: 'control' });
    expect(fnKey.accept).toEqual({ bare: false, modifier: 'control' });
    fnKey.setActive(true);
    helpers[0].say(READY);
    // The very first press is bare: left to other apps, its hold noted.
    helpers[0].say(modDown(null));
    helpers[0].say(HELD);
    helpers[0].say(fnLine(false));
    expect(intents()).toEqual([]);
    expect(notes).toEqual([{ note: 'ignored-bare-hold', gesture: 1 }]);
    helpers[0].say(modDown('control'));
    expect(intents()).toEqual(['prime']);
  });

  it('a change of keys applies to the next press without restarting the helper', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(READY);
    expect(fnKey.accept).toEqual({ bare: true, modifier: 'option' });
    fnKey.setKeys({ combination: 'fn', modifier: 'control' });
    expect(fnKey.accept).toEqual({ bare: true, modifier: null });
    helpers[0].say(modDown('control'));
    helpers[0].say(HELD);
    helpers[0].say(fnLine(false));
    expect(intents()).toEqual([]);
    expect(helpers).toHaveLength(1);
    // Anything but the three words reads as the defaults: Automatic with Option (O-49).
    fnKey.setKeys({ combination: 'chord' as never, modifier: 'command' as never });
    expect(fnKey.accept).toEqual({ bare: true, modifier: 'option' });
    fnKey.setKeys({});
    expect(fnKey.accept).toEqual({ bare: true, modifier: 'option' });
  });

  it('the colliding app follows the listeners lines, and each change is published', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(READY);
    expect(fnKey.status().collidingApp).toBeNull();
    const before = statuses.length;
    helpers[0].say(WISPR);
    expect(fnKey.status().collidingApp).toEqual({
      bundle: 'com.electron.wispr-flow',
      name: 'Wispr Flow',
    });
    expect(fnKey.status().collidingApp).toBe(KNOWN_FN_APPS[0]);
    expect(statuses).toHaveLength(before + 1);
    expect(last(statuses)?.collidingApp).toBe(KNOWN_FN_APPS[0]);
    // The same set again publishes nothing.
    helpers[0].say(WISPR);
    expect(statuses).toHaveLength(before + 1);
    // Automatic gives up bare fn while it runs, and keeps Option+fn.
    expect(fnKey.accept).toEqual({ bare: false, modifier: 'option' });
    helpers[0].say(modDown(null));
    helpers[0].say(HELD);
    helpers[0].say(fnLine(false));
    expect(intents()).toEqual([]);
    expect(notes.map((n) => n.note)).toEqual(['ignored-bare-hold']);
    helpers[0].say(modDown('option'));
    helpers[0].say(HELD);
    helpers[0].say(fnLine(false));
    expect(intents()).toEqual(['prime', 'start-hold', 'end-hold']);
    // An unlisted app that watches the keyboard is no collision.
    helpers[0].say(NO_LISTED);
    expect(fnKey.status().collidingApp).toBeNull();
    expect(last(statuses)?.collidingApp).toBeNull();
    expect(fnKey.accept).toEqual({ bare: true, modifier: 'option' });
  });

  it('a dropped listeners line leaves the colliding app as it was', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(READY);
    helpers[0].say(WISPR);
    const ids = Array.from({ length: 17 }, (_, i) => `"com.example.app${i}"`).join(',');
    for (const bad of [
      '{"type":"listeners","apps":["com.example bad"]}',
      `{"type":"listeners","apps":[${ids}]}`,
      `{"type":"listeners","apps":[],"pad":"${'x'.repeat(MAX_LINE)}"}`,
    ]) {
      helpers[0].say(bad);
      expect(fnKey.status().collidingApp).toBe(KNOWN_FN_APPS[0]);
    }
    expect(traces.slice(-3).map((t) => t.line)).toEqual(['unreadable', 'unreadable', 'unreadable']);
  });

  it('names the colliding app while Input Monitoring is not granted', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(DENIED);
    helpers[0].say(WISPR);
    expect(fnKey.status()).toMatchObject({ listening: false, collidingApp: KNOWN_FN_APPS[0] });
  });

  it('turning the shortcut off forgets the colliding app', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(READY);
    helpers[0].say(WISPR);
    fnKey.setActive(false);
    expect(fnKey.status().collidingApp).toBeNull();
    expect(last(statuses)?.collidingApp).toBeNull();
  });

  it('never puts the listener list in its status, its published statuses or its traces', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(READY);
    helpers[0].say(WISPR);
    helpers[0].say(NO_LISTED);
    helpers[0].say(WISPR);
    expect(JSON.stringify(fnKey.status())).not.toContain('com.example.hotkeys');
    expect(JSON.stringify(statuses)).not.toContain('com.example.hotkeys');
    expect(JSON.stringify(traces)).not.toContain('com.example.hotkeys');
    expect(Object.keys(fnKey.status()).sort()).toEqual(
      ['active', 'collidingApp', 'failed', 'fnUsage', 'listening', 'permission', 'supported'].sort()
    );
    // The trace counts the ids and names only a listed app.
    expect(traces.filter((t) => t.line === 'listeners').map((t) => t.listeners)).toEqual([
      { count: 2, known: 'com.electron.wispr-flow' },
      { count: 1, known: null },
      { count: 2, known: 'com.electron.wispr-flow' },
    ]);
  });

  it('asks the helper for the listeners only on request', () => {
    const fnKey = service();
    expect(fnKey.requestListeners()).toBe(false);
    fnKey.setActive(true);
    helpers[0].say(READY);
    expect(helpers[0].written).toEqual([]);
    expect(fnKey.requestListeners()).toBe(true);
    expect(helpers[0].written).toEqual(['listeners\n']);
  });

  it('a listed app that appears after the spawn restarts the helper once, while idle', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(READY);
    helpers[0].say(NONE);
    expect(helpers).toHaveLength(1);
    now = 40_000;
    helpers[0].say(WISPR);
    // A fresh helper's tap goes ahead of the other app's: the ordinary start path.
    expect(helpers).toHaveLength(2);
    expect(helpers[0].killed).toBe(true);
    expect(last(traces)?.line).toBe('restart');
    expect(fnKey.status()).toMatchObject({ failed: false, collidingApp: KNOWN_FN_APPS[0] });
    // The new helper's first listeners line names the app too, and is never a reason.
    helpers[1].say(READY);
    helpers[1].say(WISPR);
    expect(helpers).toHaveLength(2);
    // It still listens for both, and none of it counted as a crash.
    helpers[1].say(modDown('option'));
    expect(intents()).toEqual(['prime']);
  });

  it('a listed app already there at the spawn never restarts the helper', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(READY);
    helpers[0].say(WISPR);
    helpers[0].say(WISPR);
    expect(helpers).toHaveLength(1);
    // The same after a crash restart: the new helper's first line is no reason either.
    helpers[0].emit('exit', 1);
    advance(1000);
    expect(helpers).toHaveLength(2);
    helpers[1].say(READY);
    helpers[1].say(WISPR);
    expect(helpers).toHaveLength(2);
  });

  it('never restarts mid-press: it waits for the release', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(READY);
    helpers[0].say(NONE);
    now = 40_000;
    helpers[0].say(modDown('option', 40_000));
    helpers[0].say(WISPR);
    expect(helpers).toHaveLength(1);
    now = 40_300;
    helpers[0].say('{"type":"held","t":40300}');
    expect(helpers).toHaveLength(1);
    now = 41_000;
    helpers[0].say(fnLine(false, 41_000));
    // The hold was delivered whole, and then the helper was restarted.
    expect(intents()).toEqual(['prime', 'start-hold', 'end-hold']);
    expect(helpers).toHaveLength(2);
  });

  it('a pending restart waits out the double-tap gap, so the second tap still counts', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(READY);
    helpers[0].say(NONE);
    now = 40_000;
    helpers[0].say(modDown('option', 40_000));
    now = 40_090;
    helpers[0].say(fnLine(false, 40_090));
    helpers[0].say(WISPR);
    expect(helpers).toHaveLength(1);
    // The second tap arrives within the gap: the double-tap toggles, then the helper restarts.
    now = 40_220;
    helpers[0].say(modDown('option', 40_220));
    now = 40_310;
    helpers[0].say(fnLine(false, 40_310));
    expect(intents()).toEqual(['prime', 'cancel-hold', 'prime', 'toggle']);
    expect(helpers).toHaveLength(2);
  });

  it('a pending restart after a single tap happens once the gap has passed', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(READY);
    helpers[0].say(NONE);
    now = 40_000;
    helpers[0].say(modDown(null, 40_000));
    now = 40_090;
    helpers[0].say(fnLine(false, 40_090));
    helpers[0].say(WISPR);
    expect(helpers).toHaveLength(1);
    advance(FN_DOUBLE_TAP_GAP_MS - 10);
    expect(helpers).toHaveLength(1);
    advance(20);
    expect(helpers).toHaveLength(2);
    expect(intents()).toEqual(['prime', 'cancel-hold']);
  });

  it('restarts for a listed app at most once every 30 seconds', () => {
    expect(FN_LISTENER_RESTART_MIN_MS).toBe(30_000);
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(READY);
    helpers[0].say(NONE);
    now = 40_000;
    helpers[0].say(WISPR);
    expect(helpers).toHaveLength(2);
    helpers[1].say(READY);
    helpers[1].say(WISPR);
    // The app quits and starts again five seconds later: the restart waits for the spacing.
    advance(4_000);
    helpers[1].say(NONE);
    advance(1_000);
    helpers[1].say(WISPR);
    expect(helpers).toHaveLength(2);
    advance(24_990);
    expect(helpers).toHaveLength(2);
    advance(20);
    expect(helpers).toHaveLength(3);
    expect(traces.filter((t) => t.line === 'restart')).toHaveLength(2);
  });

  it('does not restart a helper that does not listen, nor one the shortcut turned off', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(DENIED);
    helpers[0].say(NONE);
    now = 40_000;
    helpers[0].say(WISPR);
    expect(helpers).toHaveLength(1);
    // Off with a restart pending: nothing is started later.
    const on = service();
    on.setActive(true);
    helpers[1].say(READY);
    helpers[1].say(NONE);
    helpers[1].say(modDown(null, 40_000));
    helpers[1].say(fnLine(false, 40_090));
    helpers[1].say(WISPR);
    on.setActive(false);
    advance(60_000);
    expect(helpers).toHaveLength(2);
  });
});
