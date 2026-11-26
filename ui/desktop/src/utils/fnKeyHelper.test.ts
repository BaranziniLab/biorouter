import { EventEmitter } from 'node:events';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { FN_HOLD_MS } from './fnGesture';
import {
  FN_PROTOCOL_VERSION,
  FnKeyService,
  LineSplitter,
  fnKeyHelperPath,
  parseFnHelperLine,
  type FnKeyStatus,
  type FnLineTrace,
  type HelperProcess,
} from './fnKeyHelper';
import type { FnIntent } from './fnGesture';

const last = <T>(items: T[]): T | undefined => items[items.length - 1];

describe('the helper protocol', () => {
  it('reads every line the helper writes (crates/biorouter-fnkey/src/protocol.rs)', () => {
    expect(
      parseFnHelperLine(
        '{"type":"ready","v":2,"permission":"granted","listening":true,"fnUsage":2,"t":900}'
      )
    ).toEqual({ type: 'ready', permission: 'granted', listening: true, fnUsage: 2, t: 900 });
    expect(
      parseFnHelperLine(
        '{"type":"ready","v":2,"permission":"denied","listening":false,"fnUsage":null}'
      )
    ).toEqual({ type: 'ready', permission: 'denied', listening: false, fnUsage: null, t: null });
    expect(parseFnHelperLine('{"type":"fn","down":true,"t":1200}')).toEqual({
      type: 'fn',
      down: true,
      t: 1200,
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
        '{"type":"ready","v":2,"permission":"granted","listening":true,"fnUsage":0,"t":5,"tapOwner":"x"}'
      )
    ).toEqual({ type: 'ready', permission: 'granted', listening: true, fnUsage: 0, t: 5 });
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

  it('names a ready line of another protocol version, whatever else it says', () => {
    expect(FN_PROTOCOL_VERSION).toBe(2);
    expect(
      parseFnHelperLine(
        '{"type":"ready","v":1,"permission":"granted","listening":true,"fnUsage":0,"t":900}'
      )
    ).toEqual({ type: 'incompatible', v: 1 });
    expect(parseFnHelperLine('{"type":"ready","v":3}')).toEqual({ type: 'incompatible', v: 3 });
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
      '{"type":"ready","v":2.5,"permission":"granted","listening":true,"fnUsage":null}',
      '{"type":"ready","v":2,"permission":"maybe","listening":true,"fnUsage":null}',
      '{"type":"ready","v":2,"permission":"granted","listening":true,"fnUsage":7}',
      '{"type":"ready","v":2,"permission":"granted","listening":true,"fnUsage":0,"t":-5}',
      '{"type":"ready","v":2,"permission":"granted","fnUsage":null}',
      '{"type":"permission","permission":"yes"}',
      '{"type":"key","code":0}',
      `{"type":"other","pad":"${'x'.repeat(600)}"}`,
    ]) {
      expect(parseFnHelperLine(line), line).toBeNull();
    }
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
    splitter.push('x'.repeat(2000));
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
  beforeEach(() => {
    vi.useFakeTimers();
    now = 0;
    helpers = [];
    seen = [];
    statuses = [];
    traces = [];
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
      now: () => now,
    });
  const advance = (ms: number) => {
    now += ms;
    vi.advanceTimersByTime(ms);
  };
  const READY = '{"type":"ready","v":2,"permission":"granted","listening":true,"fnUsage":0}';
  const DENIED = '{"type":"ready","v":2,"permission":"denied","listening":false,"fnUsage":2}';
  const readyAt = (t: number) =>
    `{"type":"ready","v":2,"permission":"granted","listening":true,"fnUsage":0,"t":${t}}`;
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
    helpers[0].say(
      '{"type":"ready","v":1,"permission":"granted","listening":true,"fnUsage":0,"t":900}'
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
      '{"type":"ready","v":2,"permission":"granted","listening":false,"fnUsage":0}'
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
      { line: 'fn-down', helperT: 40_150, mappedAt: 1_150, arrivalAt: 1_200, timedBy: 'helper' },
      { line: 'held', helperT: 40_450, mappedAt: 1_450, arrivalAt: 1_480, timedBy: 'helper' },
      { line: 'other', helperT: 40_460, mappedAt: 1_460, arrivalAt: 1_490, timedBy: 'helper' },
      { line: 'fn-up', helperT: 40_500, mappedAt: 1_500, arrivalAt: 1_520, timedBy: 'helper' },
      { line: 'permission', helperT: null, mappedAt: null, arrivalAt: 1_600, timedBy: null },
      { line: 'listener-lost', helperT: null, mappedAt: null, arrivalAt: 1_700, timedBy: null },
    ]);
    expect(intents()).toEqual(['prime', 'start-hold', 'cancel-hold']);
  });
});
