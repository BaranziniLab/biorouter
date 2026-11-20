import { EventEmitter } from 'node:events';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { FN_HOLD_MS } from './fnGesture';
import {
  FnKeyService,
  LineSplitter,
  fnKeyHelperPath,
  parseFnHelperLine,
  type FnKeyStatus,
  type HelperProcess,
} from './fnKeyHelper';
import type { FnIntent } from './fnGesture';

const last = <T>(items: T[]): T | undefined => items[items.length - 1];

describe('the helper protocol', () => {
  it('reads every line the helper writes (crates/biorouter-fnkey/src/protocol.rs)', () => {
    expect(
      parseFnHelperLine(
        '{"type":"ready","v":1,"permission":"granted","listening":true,"fnUsage":2,"t":900}'
      )
    ).toEqual({ type: 'ready', permission: 'granted', listening: true, fnUsage: 2, t: 900 });
    expect(
      parseFnHelperLine(
        '{"type":"ready","v":1,"permission":"denied","listening":false,"fnUsage":null}'
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
      '{"type":"other","t":null}',
      '{"type":"ready","v":2,"permission":"granted","listening":true,"fnUsage":null}',
      '{"type":"ready","v":1,"permission":"maybe","listening":true,"fnUsage":null}',
      '{"type":"ready","v":1,"permission":"granted","listening":true,"fnUsage":7}',
      '{"type":"ready","v":1,"permission":"granted","listening":true,"fnUsage":0,"t":-5}',
      '{"type":"ready","v":1,"permission":"granted","fnUsage":null}',
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
  let intents: FnIntent[];
  let statuses: FnKeyStatus[];
  beforeEach(() => {
    vi.useFakeTimers();
    now = 0;
    helpers = [];
    intents = [];
    statuses = [];
  });
  afterEach(() => vi.useRealTimers());

  const service = (platform: string = 'darwin', helperPath: string | null = '/h') =>
    new FnKeyService({
      platform,
      helperPath,
      spawn: () => {
        const helper = new FakeHelper();
        helpers.push(helper);
        return helper;
      },
      onIntent: (intent) => intents.push(intent),
      onStatus: (status) => statuses.push(status),
      now: () => now,
    });
  const advance = (ms: number) => {
    now += ms;
    vi.advanceTimersByTime(ms);
  };
  const READY = '{"type":"ready","v":1,"permission":"granted","listening":true,"fnUsage":0}';
  const DENIED = '{"type":"ready","v":1,"permission":"denied","listening":false,"fnUsage":2}';

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

  it('turns the helper’s fn reports into gesture intents', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(READY);
    expect(fnKey.status()).toMatchObject({ permission: 'granted', listening: true, fnUsage: 0 });
    helpers[0].say('{"type":"fn","down":true}');
    expect(intents).toEqual(['prime']);
    advance(FN_HOLD_MS);
    expect(intents).toEqual(['prime', 'start-hold']);
    advance(1500);
    helpers[0].say('{"type":"fn","down":false}');
    expect(intents).toEqual(['prime', 'start-hold', 'end-hold']);
  });

  it('keeps a hold a hold when its lines reach a busy main thread late, together', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(READY);
    now = 10_000;
    helpers[0].say('{"type":"other","t":5000}');
    // Main was busy for two seconds: the down and up of a 700 ms hold arrive at once.
    now = 12_000;
    helpers[0].say('{"type":"fn","down":true,"t":6000}');
    helpers[0].say('{"type":"fn","down":false,"t":6700}');
    expect(intents).toEqual(['prime', 'start-hold', 'end-hold']);
  });

  it('the ready line lines the clocks up, so even the first hold read late keeps its length', () => {
    const fnKey = service();
    fnKey.setActive(true);
    now = 1_000;
    helpers[0].say(
      '{"type":"ready","v":1,"permission":"granted","listening":true,"fnUsage":0,"t":51000}'
    );
    // Main was busy: the hold's down and up, 50 s on the helper's clock, arrive together.
    now = 5_000;
    helpers[0].say('{"type":"fn","down":true,"t":51200}');
    helpers[0].say('{"type":"fn","down":false,"t":52000}');
    expect(intents).toEqual(['prime', 'start-hold', 'end-hold']);
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
      helpers[0].say(`{"type":"fn","down":${down},"t":${t}}`);
    expect(intents).toEqual(['prime', 'cancel-hold', 'prime', 'toggle']);
  });

  it('a key the helper reports between two taps makes them two taps', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(READY);
    now = 10_000;
    helpers[0].say('{"type":"fn","down":true,"t":10000}');
    helpers[0].say('{"type":"fn","down":false,"t":10080}');
    helpers[0].say('{"type":"other","t":10150}');
    helpers[0].say('{"type":"fn","down":true,"t":10200}');
    helpers[0].say('{"type":"fn","down":false,"t":10260}');
    expect(intents).toEqual(['prime', 'cancel-hold', 'prime', 'cancel-hold']);
  });

  it('fn with another key reaches the gesture as a cancel', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(READY);
    helpers[0].say('{"type":"fn","down":true}');
    advance(50);
    helpers[0].say('{"type":"other"}');
    advance(FN_HOLD_MS * 3);
    helpers[0].say('{"type":"fn","down":false}');
    expect(intents).toEqual(['prime', 'cancel-hold']);
  });

  it('turning the shortcut off stops the helper and ends a hold in progress', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say(READY);
    helpers[0].say('{"type":"fn","down":true}');
    advance(FN_HOLD_MS);
    fnKey.setActive(false);
    expect(intents).toEqual(['prime', 'start-hold', 'end-hold']);
    expect(helpers[0].ended).toBe(true);
    expect(helpers[0].killed).toBe(true);
    expect(fnKey.status()).toMatchObject({ active: false, listening: false });
    // Its late lines are not listened to.
    helpers[0].say('{"type":"fn","down":true}');
    advance(FN_HOLD_MS);
    expect(intents).toEqual(['prime', 'start-hold', 'end-hold']);
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
    helpers[0].say('{"type":"fn","down":true}');
    advance(FN_HOLD_MS);
    helpers[0].emit('exit', null);
    expect(intents).toEqual(['prime', 'start-hold', 'end-hold']);
  });

  it('a helper whose listener macOS refused despite the grant counts as failed', () => {
    const fnKey = service();
    fnKey.setActive(true);
    last(helpers)!.say(
      '{"type":"ready","v":1,"permission":"granted","listening":false,"fnUsage":0}'
    );
    expect(fnKey.status()).toMatchObject({ failed: true, listening: false });
    fnKey.setActive(false);
    fnKey.setActive(true);
    expect(fnKey.status().failed).toBe(false);
  });

  it('ignores lines it cannot read', () => {
    const fnKey = service();
    fnKey.setActive(true);
    helpers[0].say('{"type":"ready","v":9}');
    helpers[0].say('hello');
    expect(fnKey.status().permission).toBe('unknown');
    expect(last(statuses)?.active).toBe(true);
  });
});
