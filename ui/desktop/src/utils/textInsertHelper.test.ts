import { EventEmitter } from 'node:events';
import fs from 'node:fs';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  ACCESS_TIMEOUT_MS,
  AnswerSplitter,
  FRONT_TIMEOUT_MS,
  INSERT_RESTORED,
  INSERT_TIMEOUT_MS,
  MAX_REQUEST_LINE_BYTES,
  STOP_GRACE_MS,
  TextInsertService,
  parseTextInsertLine,
  textInsertHelperArgs,
  textInsertHelperPath,
  type TextInsertProcess,
  type TextInsertStatus,
} from './textInsertHelper';

const last = <T>(items: T[]): T | undefined => items[items.length - 1];

describe('the helper protocol', () => {
  it('reads every answer the helper writes (crates/biorouter-textinsert/src/protocol.rs)', () => {
    expect(parseTextInsertLine('{"type":"ready","v":1,"postAllowed":true}')).toEqual({
      type: 'ready',
      v: 1,
      postAllowed: true,
    });
    expect(
      parseTextInsertLine(
        '{"id":9,"type":"front","pid":501,"bundle":"com.apple.TextEdit","secureInput":false,"postAllowed":true}'
      )
    ).toEqual({
      type: 'front',
      id: 9,
      pid: 501,
      bundle: 'com.apple.TextEdit',
      secureInput: false,
      postAllowed: true,
    });
    expect(
      parseTextInsertLine(
        '{"id":10,"type":"front","pid":null,"bundle":null,"secureInput":true,"postAllowed":false}'
      )
    ).toEqual({
      type: 'front',
      id: 10,
      pid: null,
      bundle: null,
      secureInput: true,
      postAllowed: false,
    });
    expect(parseTextInsertLine('{"id":11,"type":"inserted"}')).toEqual({
      type: 'inserted',
      id: 11,
    });
    for (const reason of ['no-permission', 'secure-input', 'target-changed', 'failed']) {
      expect(parseTextInsertLine(`{"id":12,"type":"refused","reason":"${reason}"}`)).toEqual({
        type: 'refused',
        id: 12,
        reason,
      });
    }
    expect(parseTextInsertLine('{"id":13,"type":"access","postAllowed":false}')).toEqual({
      type: 'access',
      id: 13,
      postAllowed: false,
    });
    // Fields it does not know are ignored, as the helper ignores them in requests.
    expect(parseTextInsertLine('{"id":14,"type":"inserted","later":1}')).toEqual({
      type: 'inserted',
      id: 14,
    });
  });

  it('refuses anything that is not exactly an answer', () => {
    for (const line of [
      'not json',
      '[]',
      'null',
      '{"type":"ready","postAllowed":true}',
      '{"type":"ready","v":1,"postAllowed":"yes"}',
      '{"type":"inserted"}',
      '{"id":0,"type":"inserted"}',
      '{"id":1.5,"type":"inserted"}',
      '{"id":"1","type":"inserted"}',
      '{"id":1,"type":"refused","reason":"nope"}',
      '{"id":1,"type":"refused"}',
      '{"id":1,"type":"front","pid":0,"bundle":null,"secureInput":false,"postAllowed":true}',
      '{"id":1,"type":"front","pid":-1,"bundle":null,"secureInput":false,"postAllowed":true}',
      '{"id":1,"type":"front","pid":5,"bundle":7,"secureInput":false,"postAllowed":true}',
      '{"id":1,"type":"front","pid":5,"bundle":null,"postAllowed":true}',
      '{"id":1,"type":"access"}',
      '{"id":1,"type":"pasted"}',
      `{"id":1,"type":"inserted","pad":"${'x'.repeat(5000)}"}`,
    ]) {
      expect(parseTextInsertLine(line), line.slice(0, 80)).toBeNull();
    }
  });

  it('reads how the clipboard was left after a paste, and ignores a word it does not know (DD-102)', () => {
    for (const restored of INSERT_RESTORED) {
      expect(parseTextInsertLine(`{"id":15,"type":"inserted","restored":"${restored}"}`)).toEqual({
        type: 'inserted',
        id: 15,
        restored,
      });
    }
    // The paste happened either way: an answer from a newer helper is still a paste.
    for (const odd of ['"later"', '1', 'null', '["all"]']) {
      expect(parseTextInsertLine(`{"id":16,"type":"inserted","restored":${odd}}`)).toEqual({
        type: 'inserted',
        id: 16,
      });
    }
  });

  it('splits the output into lines and drops one too long to be an answer', () => {
    const lines: string[] = [];
    const splitter = new AnswerSplitter((line) => lines.push(line));
    splitter.push('{"id":1,"type":"ins');
    splitter.push('erted"}\n\n{"id":2,');
    splitter.push('"type":"inserted"}\n');
    expect(lines).toEqual(['{"id":1,"type":"inserted"}', '{"id":2,"type":"inserted"}']);
    splitter.push('x'.repeat(5000));
    splitter.push('{"id":3,"type":"inserted"}\n');
    expect(last(lines)).toBe('{"id":3,"type":"inserted"}');
  });
});

describe('where the helper is, and how it is started', () => {
  const resources = '/Applications/Biorouter.app/Contents/Resources';
  const cwd = '/repo/ui/desktop';

  it('a packaged app uses only its own copy beside biorouterd, whatever the environment says', () => {
    const packaged = path.join(resources, 'bin', 'biorouter-textinsert');
    expect(
      textInsertHelperPath(
        { isPackaged: true },
        { BIOROUTER_TEXTINSERT_HELPER: '/tmp/stand-in' },
        (file) => file === packaged || file === '/tmp/stand-in',
        resources,
        cwd
      )
    ).toBe(packaged);
    expect(
      textInsertHelperPath(
        { isPackaged: true },
        { BIOROUTER_TEXTINSERT_HELPER: '/tmp/stand-in' },
        (file) => file === '/tmp/stand-in',
        resources,
        cwd
      )
    ).toBeNull();
  });

  it('a development app takes the override, then a workspace build', () => {
    expect(
      textInsertHelperPath(
        { isPackaged: false },
        { BIOROUTER_TEXTINSERT_HELPER: ' /tmp/stand-in ' },
        (file) => file === '/tmp/stand-in',
        resources,
        cwd
      )
    ).toBe('/tmp/stand-in');
    // An override that names nothing starts nothing, rather than falling back to the real helper.
    expect(
      textInsertHelperPath(
        { isPackaged: false },
        { BIOROUTER_TEXTINSERT_HELPER: '/tmp/missing' },
        (file) => file !== '/tmp/missing',
        resources,
        cwd
      )
    ).toBeNull();
    const debug = path.join(cwd, '..', '..', 'target', 'debug', 'biorouter-textinsert');
    expect(
      textInsertHelperPath({ isPackaged: false }, {}, (file) => file === debug, resources, cwd)
    ).toBe(debug);
  });

  it('names a test pasteboard only in an unpackaged app, and never falls back to the real one', () => {
    const env = { BIOROUTER_TEXTINSERT_TEST_PASTEBOARD: 'biorouter.insert-probe_1' };
    expect(textInsertHelperArgs({ isPackaged: true }, env)).toEqual([]);
    expect(textInsertHelperArgs({ isPackaged: false }, env)).toEqual([
      '--test-pasteboard',
      'biorouter.insert-probe_1',
    ]);
    expect(textInsertHelperArgs({ isPackaged: false }, {})).toEqual([]);
    expect(
      textInsertHelperArgs({ isPackaged: false }, { BIOROUTER_TEXTINSERT_TEST_PASTEBOARD: '  ' })
    ).toEqual([]);
    for (const bad of ['Apple CFPasteboard general', 'a/b', 'x'.repeat(129), '--flag;rm']) {
      expect(
        textInsertHelperArgs({ isPackaged: false }, { BIOROUTER_TEXTINSERT_TEST_PASTEBOARD: bad })
      ).toBeNull();
    }
  });
});

class FakeHelper extends EventEmitter implements TextInsertProcess {
  written: string[] = [];
  ended = false;
  killed = false;
  stdout = new EventEmitter();
  stdin = {
    write: (text: string) => this.written.push(text),
    end: () => {
      this.ended = true;
    },
    on: () => undefined,
  };
  constructor(readonly args: readonly string[]) {
    super();
  }
  kill() {
    this.killed = true;
  }
  say(line: string) {
    this.stdout.emit('data', Buffer.from(`${line}\n`));
  }
  exit(code: number | null = 0) {
    this.emit('exit', code);
  }
  /** The requests written so far, parsed. */
  requests(): Array<Record<string, unknown>> {
    return this.written.map((line) => JSON.parse(line) as Record<string, unknown>);
  }
  lastId(): number {
    return last(this.requests())!.id as number;
  }
}

describe('the text insertion service', () => {
  let now = 0;
  let helpers: FakeHelper[];
  let statuses: TextInsertStatus[];
  beforeEach(() => {
    vi.useFakeTimers();
    now = 0;
    helpers = [];
    statuses = [];
  });
  afterEach(() => vi.useRealTimers());

  const service = (
    platform: string = 'darwin',
    helperPath: string | null = '/h',
    args: readonly string[] = []
  ) =>
    new TextInsertService({
      platform,
      helperPath,
      args,
      spawn: (_file, spawnArgs) => {
        const helper = new FakeHelper(spawnArgs);
        helpers.push(helper);
        return helper;
      },
      onStatus: (status) => statuses.push(status),
      now: () => now,
    });
  const advance = async (ms: number) => {
    now += ms;
    await vi.advanceTimersByTimeAsync(ms);
  };
  const READY = '{"type":"ready","v":1,"postAllowed":true}';

  it('starts nothing off macOS or without the program, and every request then fails at once', async () => {
    for (const idle of [service('win32'), service('linux'), service('darwin', null)]) {
      idle.start();
      expect(idle.supported).toBe(false);
      await expect(idle.front()).resolves.toBeNull();
      await expect(idle.insert(501, 'words')).resolves.toEqual({ ok: false, reason: 'failed' });
      await expect(idle.access()).resolves.toBe(false);
      await expect(idle.requestAccess()).resolves.toBe(false);
    }
    expect(helpers).toHaveLength(0);
  });

  it('passes its arguments, and asks nothing before it is started', async () => {
    const insert = service('darwin', '/h', ['--test-pasteboard', 'probe']);
    await expect(insert.front()).resolves.toBeNull();
    expect(helpers).toHaveLength(0);
    insert.start();
    expect(helpers[0].args).toEqual(['--test-pasteboard', 'probe']);
    helpers[0].say(READY);
    expect(insert.status()).toEqual({
      supported: true,
      active: true,
      postAllowed: true,
      failed: false,
    });
  });

  it('gives each request its own id and settles it by that id, in any order', async () => {
    const insert = service();
    insert.start();
    const first = insert.front();
    const second = insert.insert(777, 'two words');
    const [a, b] = helpers[0].requests();
    expect(a).toEqual({ id: 1, op: 'front' });
    expect(b).toEqual({ id: 2, op: 'insert', pid: 777, text: 'two words' });
    // `id` comes first on the line, so a line the helper refuses whole can still be answered.
    expect(helpers[0].written[1].startsWith('{"id":2,')).toBe(true);
    expect(helpers[0].written.every((line) => line.endsWith('\n'))).toBe(true);
    helpers[0].say('{"id":2,"type":"inserted"}');
    helpers[0].say(
      '{"id":1,"type":"front","pid":777,"bundle":"com.apple.Notes","secureInput":false,"postAllowed":true}'
    );
    await expect(second).resolves.toEqual({ ok: true });
    await expect(first).resolves.toEqual({
      pid: 777,
      bundle: 'com.apple.Notes',
      secureInput: false,
      postAllowed: true,
    });
    // Ids keep rising.
    void insert.access();
    expect(helpers[0].lastId()).toBe(3);
  });

  it('turns each refusal into its reason', async () => {
    const insert = service();
    insert.start();
    for (const reason of ['no-permission', 'secure-input', 'target-changed', 'failed'] as const) {
      const result = insert.insert(501, 'words');
      helpers[0].say(`{"id":${helpers[0].lastId()},"type":"refused","reason":"${reason}"}`);
      await expect(result).resolves.toEqual({ ok: false, reason });
    }
    expect(insert.status().postAllowed).toBe(false);
  });

  it('a paste answers how the helper left the clipboard, when it says', async () => {
    const insert = service();
    insert.start();
    for (const restored of INSERT_RESTORED) {
      const result = insert.insert(501, 'words');
      helpers[0].say(`{"id":${helpers[0].lastId()},"type":"inserted","restored":"${restored}"}`);
      await expect(result).resolves.toEqual({ ok: true, restored });
    }
    const older = insert.insert(501, 'words');
    helpers[0].say(`{"id":${helpers[0].lastId()},"type":"inserted"}`);
    await expect(older).resolves.toEqual({ ok: true });
  });

  it('waits for the helper longer than its post deadline plus its longest restore delay', () => {
    // A client that gave up first would copy the text while the helper still pastes it, so the
    // text would land in the app and on the clipboard (F9). Read from the helper's own source.
    const rust = fs.readFileSync(
      path.join(__dirname, '../../../../crates/biorouter-textinsert/src/protocol.rs'),
      'utf8'
    );
    const millis = (name: string): number => {
      const match = rust.match(
        new RegExp(`pub const ${name}: Duration = Duration::from_millis\\(([0-9_]+)\\);`)
      );
      expect(match, name).not.toBeNull();
      return Number(match![1].replace(/_/g, ''));
    };
    expect(INSERT_TIMEOUT_MS).toBe(5_000);
    expect(INSERT_TIMEOUT_MS).toBeGreaterThan(
      millis('POST_DEADLINE') + millis('MAX_RESTORE_DELAY') + 500
    );
    // F13: an insert queued behind a slow one gets its own post deadline from when it begins,
    // and the helper still never posts so late that its answer would come after this client gave
    // up. It counts that from the request's arrival against its copy of this timeout.
    expect(millis('APP_INSERT_TIMEOUT')).toBe(INSERT_TIMEOUT_MS);
  });

  it('a front with no app in front, or an answer of the wrong kind, is no target', async () => {
    const insert = service();
    insert.start();
    const none = insert.front();
    helpers[0].say(
      `{"id":${helpers[0].lastId()},"type":"front","pid":null,"bundle":null,"secureInput":false,"postAllowed":true}`
    );
    await expect(none).resolves.toBeNull();
    const refused = insert.front();
    helpers[0].say(`{"id":${helpers[0].lastId()},"type":"refused","reason":"failed"}`);
    await expect(refused).resolves.toBeNull();
    const crossed = insert.insert(501, 'words');
    helpers[0].say(`{"id":${helpers[0].lastId()},"type":"access","postAllowed":true}`);
    await expect(crossed).resolves.toEqual({ ok: false, reason: 'failed' });
  });

  it('a late or silent helper gives failed, and its late answer changes nothing', async () => {
    const insert = service();
    insert.start();

    const front = insert.front();
    await advance(FRONT_TIMEOUT_MS - 1);
    let settled = false;
    void front.then(() => (settled = true));
    await Promise.resolve();
    expect(settled).toBe(false);
    await advance(1);
    await expect(front).resolves.toBeNull();

    const words = insert.insert(501, 'words');
    const id = helpers[0].lastId();
    await advance(INSERT_TIMEOUT_MS);
    await expect(words).resolves.toEqual({ ok: false, reason: 'failed' });
    // The answer that comes after the timeout is ignored.
    helpers[0].say(`{"id":${id},"type":"inserted"}`);

    const access = insert.access();
    await advance(ACCESS_TIMEOUT_MS);
    await expect(access).resolves.toBe(false);
  });

  it('settles every waiting request as failed when the helper dies, then restarts it with a back off', async () => {
    const insert = service();
    insert.start();
    const front = insert.front();
    const words = insert.insert(501, 'words');
    helpers[0].exit(1);
    await expect(front).resolves.toBeNull();
    await expect(words).resolves.toEqual({ ok: false, reason: 'failed' });
    // While it is down, a request fails at once rather than waiting.
    await expect(insert.front()).resolves.toBeNull();

    // 1 s, then 2 s, 4 s, 8 s and 16 s between restarts.
    for (const [index, delay] of [1_000, 2_000, 4_000, 8_000, 16_000].entries()) {
      await advance(delay - 1);
      expect(helpers).toHaveLength(index + 1);
      await advance(1);
      expect(helpers).toHaveLength(index + 2);
      helpers[index + 1].exit(1);
    }
    // Five restarts inside a minute: it stops trying, and says so.
    await advance(60_000);
    expect(helpers).toHaveLength(6);
    expect(insert.status()).toMatchObject({ active: true, failed: true });
    await expect(insert.insert(501, 'words')).resolves.toEqual({ ok: false, reason: 'failed' });

    // Turning it on again tries afresh.
    insert.start();
    expect(helpers).toHaveLength(7);
    expect(insert.status().failed).toBe(false);
  });

  it('a spawn that throws is retried like a helper that died', async () => {
    let attempts = 0;
    const insert = new TextInsertService({
      platform: 'darwin',
      helperPath: '/h',
      spawn: (_file, args) => {
        attempts += 1;
        if (attempts === 1) throw new Error('EACCES');
        const helper = new FakeHelper(args);
        helpers.push(helper);
        return helper;
      },
      now: () => now,
    });
    insert.start();
    expect(helpers).toHaveLength(0);
    await advance(1_000);
    expect(helpers).toHaveLength(1);
  });

  it('a helper speaking another protocol is stopped, not trusted', async () => {
    const insert = service();
    insert.start();
    const front = insert.front();
    helpers[0].say('{"type":"ready","v":2,"postAllowed":true}');
    await expect(front).resolves.toBeNull();
    expect(helpers[0].ended).toBe(true);
    expect(insert.status()).toMatchObject({ failed: true, postAllowed: null });
    await advance(10_000);
    expect(helpers).toHaveLength(1);
  });

  it('stop closes the helper’s input, settles what waits, and terminates it only after a grace', async () => {
    const insert = service();
    insert.start();
    const words = insert.insert(501, 'words');
    insert.stop();
    await expect(words).resolves.toEqual({ ok: false, reason: 'failed' });
    expect(helpers[0].ended).toBe(true);
    expect(helpers[0].killed).toBe(false);
    await advance(STOP_GRACE_MS);
    expect(helpers[0].killed).toBe(true);
    expect(insert.status().active).toBe(false);
    // Stopped, it is not restarted.
    await advance(60_000);
    expect(helpers).toHaveLength(1);

    // A helper that leaves on its own within the grace is not terminated.
    insert.start();
    insert.stop();
    helpers[1].exit(0);
    await advance(STOP_GRACE_MS);
    expect(helpers[1].killed).toBe(false);
  });

  it('only requestAccess sends the request that can make macOS ask', async () => {
    const insert = service();
    insert.start();
    const read = insert.access();
    helpers[0].say(`{"id":${helpers[0].lastId()},"type":"access","postAllowed":true}`);
    await expect(read).resolves.toBe(true);
    void insert.front();
    void insert.insert(501, 'words');
    expect(helpers[0].requests().map((request) => request.op)).toEqual([
      'access',
      'front',
      'insert',
    ]);

    const ask = insert.requestAccess();
    expect(last(helpers[0].requests())).toEqual({ id: 4, op: 'request-access' });
    helpers[0].say('{"id":4,"type":"access","postAllowed":false}');
    await expect(ask).resolves.toBe(false);
  });

  it('a "not allowed" from an idle helper is checked again by a fresh one, at most every two seconds', async () => {
    const insert = service();
    insert.start();
    // Too young to replace: a fresh helper would read what this one just read.
    let read = insert.access();
    helpers[0].say(`{"id":${helpers[0].lastId()},"type":"access","postAllowed":false}`);
    await expect(read).resolves.toBe(false);
    expect(helpers).toHaveLength(1);

    await advance(2_000);
    read = insert.access();
    helpers[0].say(`{"id":${helpers[0].lastId()},"type":"access","postAllowed":false}`);
    await vi.advanceTimersByTimeAsync(0);
    // The old helper's input is closed and a new one is asked.
    expect(helpers[0].ended).toBe(true);
    expect(helpers).toHaveLength(2);
    helpers[1].say(`{"id":${helpers[1].lastId()},"type":"access","postAllowed":true}`);
    await expect(read).resolves.toBe(true);
    expect(insert.status().postAllowed).toBe(true);

    // Not again within two seconds.
    read = insert.access();
    helpers[1].say(`{"id":${helpers[1].lastId()},"type":"access","postAllowed":false}`);
    await expect(read).resolves.toBe(false);
    expect(helpers).toHaveLength(2);
  });

  it('reads Accessibility while stopped with a helper of its own, and lets it go after', async () => {
    const insert = service();
    const read = insert.access();
    expect(helpers).toHaveLength(1);
    expect(helpers[0].requests()).toEqual([{ id: 1, op: 'access' }]);
    helpers[0].say('{"id":1,"type":"access","postAllowed":true}');
    await expect(read).resolves.toBe(true);
    expect(helpers[0].ended).toBe(true);
    expect(insert.status()).toMatchObject({ active: false, postAllowed: true });
    // It is not restarted when it leaves.
    helpers[0].exit(0);
    await advance(60_000);
    expect(helpers).toHaveLength(1);

    // The Settings button works while stopped too, and a start meanwhile keeps the helper.
    const ask = insert.requestAccess();
    expect(helpers).toHaveLength(2);
    insert.start();
    expect(helpers).toHaveLength(2);
    helpers[1].say(`{"id":${helpers[1].lastId()},"type":"access","postAllowed":false}`);
    await expect(ask).resolves.toBe(false);
    expect(helpers[1].ended).toBe(false);
    expect(insert.status()).toMatchObject({ active: true, postAllowed: false });
  });

  it('an insert it cannot send fails without reaching the helper', async () => {
    const insert = service();
    insert.start();
    for (const [pid, text] of [
      [0, 'words'],
      [-3, 'words'],
      [1.5, 'words'],
      [501, ''],
    ] as const) {
      await expect(insert.insert(pid, text)).resolves.toEqual({ ok: false, reason: 'failed' });
    }
    // A line over the helper's limit is refused here rather than sent to be refused there.
    const long = 'é'.repeat(MAX_REQUEST_LINE_BYTES / 2);
    await expect(insert.insert(501, long)).resolves.toEqual({ ok: false, reason: 'failed' });
    expect(helpers[0].written).toEqual([]);
    // Just under the limit is sent.
    void insert.insert(501, 'a'.repeat(MAX_REQUEST_LINE_BYTES - 100));
    expect(helpers[0].written).toHaveLength(1);
  });

  it('never logs the text', async () => {
    const logged = vi.spyOn(console, 'log').mockImplementation(() => undefined);
    const warned = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    const erred = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    const insert = service();
    insert.start();
    const words = insert.insert(501, 'a private sentence');
    helpers[0].say('garbage');
    helpers[0].say(`{"id":${helpers[0].lastId()},"type":"inserted"}`);
    await words;
    for (const spy of [logged, warned, erred]) expect(spy).not.toHaveBeenCalled();
    const source = fs.readFileSync(path.join(__dirname, 'textInsertHelper.ts'), 'utf8');
    expect(source).not.toMatch(/console\./);
    vi.restoreAllMocks();
  });
});
