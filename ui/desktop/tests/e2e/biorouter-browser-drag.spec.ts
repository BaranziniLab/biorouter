import { _electron as electron, expect, test, type ElectronApplication } from '@playwright/test';
import { build } from 'esbuild';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
// The fixture site is plain CommonJS so it also runs with `node` by hand.
// eslint-disable-next-line @typescript-eslint/no-require-imports
const { startFixtureSite } = require('./fixtures/biorouter-browser/site.cjs');

/**
 * `browser_drag` (AD-17) in a real Electron: the REAL driver drags on the fixture site's
 * /agent/drag/ pages, each built the way a real drag library behaves (native HTML5 drop zones
 * that reflow on dragstart, a SortableJS style native list, a jQuery UI style mouse sortable),
 * and the spec reads the page's own DOM order afterwards. The snapshot's drag references are
 * checked on the same pages.
 *
 * Ungated: no daemon, no model, no network. Run alone:
 *   npx playwright test tests/e2e/biorouter-browser-drag.spec.ts --reporter=list
 *
 * With BIOROUTER_BROWSER_LIVE_DRAG=1 it also drags on the public SortableJS and jQuery UI demo
 * pages, which needs the network.
 */

type Site = {
  url: (role: 'publicA' | 'publicB' | 'intranet', pathname?: string) => string;
  reservedPort: number;
  close: () => Promise<void>;
};

type Result = {
  ok: boolean;
  detail: string;
  data?: Record<string, unknown> & {
    snapshot?: string;
    title?: string;
    needs_act_grant?: boolean;
    error_kind?: string;
  };
};

let app: ElectronApplication;
let site: Site;
let work: string;

// eslint-disable-next-line @typescript-eslint/no-explicit-any
async function call<T = any>(name: string, ...args: unknown[]): Promise<T> {
  return app.evaluate(
    async (_electron, request) =>
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      (globalThis as any).__biorouterBrowserTest[request.name](...request.args),
    { name, args }
  );
}

const CHAT = 'chat-drag-1';
function agent(
  op: string,
  args: Record<string, unknown> = {},
  options: Record<string, unknown> = {}
): Promise<Result> {
  return call<Result>('agent', CHAT, op, args, options);
}

/** The reference a snapshot gave a line matching `pattern`. */
function refFor(snapshot: string | undefined, pattern: RegExp): string {
  const line = (snapshot ?? '').split('\n').find((candidate) => pattern.test(candidate));
  const ref = line?.match(/@e\d+/)?.[0];
  if (!ref) throw new Error(`no line matches ${pattern} in:\n${snapshot}`);
  return ref;
}

/** Run script in the agent tab, as the page's own code would. */
async function page<T>(script: string): Promise<T> {
  const views = await call<string[]>('agentViews');
  return call<T>('run', views[0], script);
}

const order = (selector: string) =>
  page<string>(
    `[...document.querySelectorAll(${JSON.stringify(selector)})].map((e) => e.textContent.trim()).join(',')`
  );

async function open(pathname: string): Promise<void> {
  const opened = await agent('open', { url: site.url('publicA', pathname) });
  expect(opened.ok, opened.detail).toBe(true);
}

test.describe.configure({ mode: 'serial' });

test.beforeAll(async () => {
  site = (await startFixtureSite()) as Site;
  work = fs.mkdtempSync(path.join(os.tmpdir(), 'biorouter-browser-drag-e2e-'));
  const bundle = path.join(work, 'main.cjs');
  await build({
    entryPoints: [path.join(__dirname, 'biorouter-browser.main.ts')],
    outfile: bundle,
    bundle: true,
    platform: 'node',
    format: 'cjs',
    external: ['electron'],
    logLevel: 'warning',
  });
  app = await electron.launch({
    args: [bundle],
    env: {
      ...process.env,
      ELECTRON_RUN_AS_NODE: '',
      BIOROUTER_E2E: '1',
      BIOROUTER_BROWSER_TEST_HOSTS:
        'site-a.test=127.0.0.1:public,site-b.test=127.0.0.1:public,intranet.test=127.0.0.1:private',
      BIOROUTER_BROWSER_TEST_DOWNLOAD_DIR: path.join(work, 'person-downloads'),
      BIOROUTER_BROWSER_TEST_USER_DATA: path.join(work, 'profile'),
    },
  });
  await call('ready');
  await call('reservePort', site.reservedPort);
});

test.afterAll(async () => {
  await app?.close().catch(() => {});
  await site?.close();
  if (work) fs.rmSync(work, { recursive: true, force: true });
});

test('the snapshot gives references to what a person would drag, and caps a flood', async () => {
  await open('/agent/drag/affordances');
  const snap = (await agent('snapshot', {}, { grant: 'observe' })).data?.snapshot ?? '';
  // An element that sets a grab cursor is an item; a card with a heading and paragraphs stays one.
  expect(snap).toMatch(/@e\d+ \[draggable\] "Card Alpha First line Second line"/);
  expect(snap).toMatch(/@e\d+ \[draggable\] "Card Beta First line Second line"/);
  expect(snap).not.toMatch(/"First line"/);
  // A list whose cursor is move: its items, not the list and not each descendant.
  for (const item of ['Move one', 'Move two', 'Move three'])
    expect(snap).toMatch(new RegExp(`@e\\d+ \\[listitem\\] "${item}" \\[draggable\\]`));
  // Chromium calls a draggable div a group.
  expect(snap).toMatch(/@e\d+ \[group\] "Native box" \[draggable\]/);
  expect(snap).not.toContain('Hidden box');
  // A grip with no words is named by its row.
  expect(snap).toMatch(/@e\d+ \[draggable\] in:"⋮⋮ Alpha task"/);
  expect(snap).toMatch(/@e\d+ \[button\] "Move button" \[draggable\]/);
  // Three hundred draggable tiles: capped, so the snapshot is not flooded.
  const tiles = snap.split('\n').filter((line) => /\[draggable\] "T\d+"/.test(line)).length;
  expect(tiles).toBeGreaterThan(10);
  expect(snap.split('\n').filter((line) => line.includes('draggable')).length).toBeLessThanOrEqual(
    100
  );
});

test('drops a card on a zone even though the page reflows when it picks the card up', async () => {
  await open('/agent/drag/zone');
  const snap = (await agent('snapshot')).data?.snapshot;
  const dragged = await agent('drag', {
    from: { ref: refFor(snap, /"Write the report" \[draggable\]/) },
    to: { ref: refFor(snap, /\[region\] "Done"/) },
  });
  expect(dragged.ok, dragged.detail).toBe(true);
  expect(dragged.detail).toContain('with an HTML drag');
  expect(dragged.detail).toContain('the page moved it into [region] "Done"');
  expect(await page<string>('document.title')).toBe('dropped card-a in done');
  expect(await order('#done .card')).toBe('Write the report');

  // By text, both ends: the heading inside the zone takes the drop for it.
  const byText = await agent('drag', {
    from: { text: 'Review the data', scope: null },
    to: { text: 'Later', scope: null },
  });
  expect(byText.ok, byText.detail).toBe(true);
  expect(await order('#later .card')).toBe('Review the data');
  expect(await order('#todo .card')).toBe('Send the samples');
});

test('reorders a SortableJS style native list by selector and by scoped text', async () => {
  await open('/agent/drag/native');
  const dragged = await agent('drag', {
    from: { selector: '#native-list > .item:nth-child(1)' },
    to: { text: 'Item 3', scope: '#native-list' },
  });
  expect(dragged.ok, dragged.detail).toBe(true);
  expect(dragged.detail).toContain('with an HTML drag');
  expect(dragged.detail).toContain('the page moved it after [div] "Item 3"');
  expect(await order('#native-list .item')).toBe('Item 2,Item 3,Item 1,Item 4');
  expect(await page<string>("document.getElementById('native-list').dataset.done")).toBe('1');

  // Upward, in the other list: the dragged item takes the place of the one it is dropped on.
  const up = await agent('drag', {
    from: { text: 'Item 4', scope: '#other-list' },
    to: { text: 'Item 1', scope: '#other-list' },
  });
  expect(up.ok, up.detail).toBe(true);
  expect(await order('#other-list .item')).toBe('Item 4,Item 1,Item 2,Item 3');
  // The first list was left alone.
  expect(await order('#native-list .item')).toBe('Item 2,Item 3,Item 1,Item 4');
});

test('reorders a jQuery UI style mouse sortable, by text and by its snapshot references', async () => {
  await open('/agent/drag/mouse');
  const dragged = await agent('drag', {
    from: { text: 'Item 1', scope: null },
    to: { text: 'Item 3', scope: null },
  });
  expect(dragged.ok, dragged.detail).toBe(true);
  expect(dragged.detail).toContain('with pointer events');
  expect(await order('#mouse-list li')).toBe('Item 2,Item 3,Item 1,Item 4,Item 5');

  const snap = (await agent('snapshot')).data?.snapshot;
  const up = await agent('drag', {
    from: { ref: refFor(snap, /\[listitem\] "Item 5" \[draggable\]/) },
    to: { ref: refFor(snap, /\[listitem\] "Item 2" \[draggable\]/) },
  });
  expect(up.ok, up.detail).toBe(true);
  expect(await order('#mouse-list li')).toBe('Item 5,Item 2,Item 3,Item 1,Item 4');
  expect(await page<string>("document.getElementById('mouse-list').dataset.done")).toBe('2');
});

test('a locator names exactly one visible element, or the drag says why not', async () => {
  await open('/agent/drag/native');
  const several = await agent('drag', {
    from: { text: 'Item 1', scope: null },
    to: { text: 'Item 2', scope: null },
  });
  expect(several.ok).toBe(false);
  expect(several.detail).toContain('2 visible elements match the text "Item 1"');
  expect(several.detail).toContain('(in #native-list, #other-list)');
  expect(several.detail).toContain('"#native-list >> text=Item 1"');

  const none = await agent('drag', {
    from: { text: 'Item 9', scope: null },
    to: { selector: '.item' },
  });
  expect(none.ok).toBe(false);
  expect(none.detail).toContain('No visible element matches the text "Item 9"');

  const invalid = await agent('drag', { from: { selector: 'Item 1' }, to: { selector: '#x' } });
  expect(invalid.ok).toBe(false);
  expect(invalid.detail).toContain('is not a valid CSS selector');
  expect(invalid.detail).toContain('text=');

  const same = await agent('drag', {
    from: { selector: '#native-list > .item:nth-child(2)' },
    to: { text: 'Item 2', scope: '#native-list' },
  });
  expect(same.ok).toBe(false);
  expect(same.detail).toContain('both the thing to drag and the place to drop it');

  const observe = await agent(
    'drag',
    { from: { selector: '#native-list > .item:nth-child(1)' }, to: { selector: '#other-list' } },
    { grant: 'observe' }
  );
  expect(observe.ok).toBe(false);
  expect(observe.data?.needs_act_grant).toBe(true);
  // Nothing moved through any of the refusals.
  expect(await order('#native-list .item')).toBe('Item 1,Item 2,Item 3,Item 4');
});

test('never drags into a password field, by locator or at its point', async () => {
  await open('/agent/drag/zone');
  const byLocator = await agent('drag', {
    from: { text: 'Write the report', scope: null },
    to: { selector: '#pw' },
  });
  expect(byLocator.ok).toBe(false);
  expect(byLocator.data?.error_kind).toBe('protected_field');
  const point = await page<{ x: number; y: number }>(
    `(() => { const r = document.getElementById('pw').getBoundingClientRect(); return { x: Math.round(r.left + r.width / 2), y: Math.round(r.top + r.height / 2) }; })()`
  );
  const atPoint = await agent('drag', {
    from: { text: 'Write the report', scope: null },
    point,
  });
  expect(atPoint.ok).toBe(false);
  expect(atPoint.data?.error_kind).toBe('protected_field');
  expect(await page<string>("document.getElementById('pw').value")).toBe('');
});

/** Run script in the W3Schools-style result frame of /agent/drag/frame. */
const inResult = <T>(script: string) =>
  page<T>(
    `(() => { const document = window.document.getElementById('iframeResult').contentDocument; return ${script}; })()`
  );

test('drops the W3Schools image into the empty rectangle in its frame', async () => {
  await open('/agent/drag/frame');
  await expect.poll(() => inResult<boolean>("!!document.getElementById('div1')")).toBe(true);
  const snap = (await agent('snapshot')).data?.snapshot ?? '';
  // The empty zone has no role and no words: its drop handlers are what make it a place.
  const zone = refFor(snap, /\[dropzone\] id:"div1"/);
  const image = refFor(snap, /\[image\].*\[draggable\]/);
  const dragged = await agent('drag', { from: { ref: image }, to: { ref: zone } });
  expect(dragged.ok, dragged.detail).toBe(true);
  expect(dragged.detail).toContain('with an HTML drag');
  expect(dragged.detail).toContain('the page moved it into');
  expect(await inResult<boolean>("!!document.querySelector('#div1 > #drag1')")).toBe(true);
});

test('says how to reach an element in a frame when a selector finds nothing', async () => {
  await open('/agent/drag/frame');
  await expect.poll(() => inResult<boolean>("!!document.getElementById('div1')")).toBe(true);
  for (const to of [{ selector: '#div1' }, { selector: 'iframe#iframeResult div#div1' }]) {
    const missed = await agent('drag', { from: { selector: '#textareaCode' }, to });
    expect(missed.ok).toBe(false);
    expect(missed.detail).toContain('No visible element matches');
    expect(missed.detail).toContain('not inside its frame');
    expect(missed.detail).toContain('@e reference from browser_snapshot');
  }
});

test('drops between references inside a frame from another site', async () => {
  await open('/agent/drag/frame-cross');
  let snap = '';
  await expect
    .poll(async () => {
      snap = (await agent('snapshot')).data?.snapshot ?? '';
      return /\[dropzone\] id:"div1"/.test(snap) && /\[image\].*\[draggable\]/.test(snap);
    })
    .toBe(true);
  // The frame runs in a process of its own, so its nodes are reached through its own session.
  const processes = await app.evaluate(({ webContents }) =>
    webContents
      .getAllWebContents()
      .flatMap((contents) => contents.mainFrame.framesInSubtree)
      .filter((frame) => frame.url.includes('/agent/drag/frame'))
      .map((frame) => frame.processId)
  );
  expect(new Set(processes).size).toBe(2);
  const dragged = await agent('drag', {
    from: { ref: refFor(snap, /\[image\].*\[draggable\]/) },
    to: { ref: refFor(snap, /\[dropzone\] id:"div1"/) },
  });
  expect(dragged.ok, dragged.detail).toBe(true);
  expect(dragged.detail).toContain('with an HTML drag');
  await expect.poll(() => page<string>('document.title')).toBe('image in div1');
});

test('never drops through a frame, nor on a password field inside what it names', async () => {
  await open('/agent/drag/guard');
  await expect
    .poll(() =>
      page<boolean>(
        "!!document.getElementById('inner-frame').contentDocument?.getElementById('inner-pw')"
      )
    )
    .toBe(true);
  const from = { selector: '#card' };
  // A frame named as the destination: what a drop lands on inside it is not checked.
  for (const selector of ['iframe#pay', '#inner-frame']) {
    const framed = await agent('drag', { from, to: { selector } });
    expect(framed.ok, selector).toBe(false);
    expect(framed.detail, selector).toContain('is a frame');
  }
  // A form whose drop point is its password field.
  const form = await agent('drag', { from, to: { selector: '#login-form' } });
  expect(form.ok).toBe(false);
  expect(form.data?.error_kind).toBe('protected_field');
  // A box whose drop point is over a frame inside it.
  const holder = await agent('drag', { from, to: { selector: '#holder' } });
  expect(holder.ok).toBe(false);
  expect(holder.detail).toContain('over a frame');
  // A source whose press point is a password field.
  const source = await agent('drag', { from: { selector: '#pw-row' }, to: { selector: '#card' } });
  expect(source.ok).toBe(false);
  expect(source.data?.error_kind).toBe('protected_field');
  // A frame named as the thing to drag.
  const frameSource = await agent('drag', { from: { selector: '#inner-frame' }, to: from });
  expect(frameSource.ok).toBe(false);
  expect(frameSource.detail).toContain('is a frame');
  expect(await page<string>('document.title')).toBe('Drag guard');
  expect(await page<string>("document.getElementById('pw').value")).toBe('');
  expect(
    await page<string>(
      "document.getElementById('inner-frame').contentDocument.getElementById('inner-pw').value"
    )
  ).toBe('');
  // Nothing reached the field in the other site's frame either.
  expect(await page<string>("document.body.dataset.typed || ''")).toBe('');
});

test.describe('live demo pages (network)', () => {
  test.skip(!process.env.BIOROUTER_BROWSER_LIVE_DRAG, 'set BIOROUTER_BROWSER_LIVE_DRAG=1');

  test('SortableJS: Item 1 goes below Item 3 in the simple list', async () => {
    const opened = await agent('open', { url: 'https://sortablejs.github.io/Sortable/' });
    expect(opened.ok, opened.detail).toBe(true);
    const before = await order('#example1 .list-group-item');
    expect(before).toBe('Item 1,Item 2,Item 3,Item 4,Item 5,Item 6');
    const dragged = await agent('drag', {
      from: { text: 'Item 1', scope: '#example1' },
      to: { text: 'Item 3', scope: '#example1' },
    });
    console.log('SortableJS:', dragged.detail);
    expect(dragged.ok, dragged.detail).toBe(true);
    await expect
      .poll(() => order('#example1 .list-group-item'))
      .toBe('Item 2,Item 3,Item 1,Item 4,Item 5,Item 6');
  });

  test('jQuery UI: Item 1 goes below Item 3, by text and by reference', async () => {
    const opened = await agent('open', {
      url: 'https://jqueryui.com/resources/demos/sortable/default.html',
    });
    expect(opened.ok, opened.detail).toBe(true);
    await expect.poll(() => page<string>('document.readyState')).toBe('complete');
    const dragged = await agent('drag', {
      from: { text: 'Item 1', scope: null },
      to: { text: 'Item 3', scope: null },
    });
    console.log('jQuery UI:', dragged.detail);
    expect(dragged.ok, dragged.detail).toBe(true);
    expect(await order('#sortable li')).toBe('Item 2,Item 3,Item 1,Item 4,Item 5,Item 6,Item 7');
    const snap = (await agent('snapshot')).data?.snapshot ?? '';
    console.log(snap);
    const back = await agent('drag', {
      from: { ref: refFor(snap, /"Item 1" \[draggable\]/) },
      to: { ref: refFor(snap, /"Item 2" \[draggable\]/) },
    });
    expect(back.ok, back.detail).toBe(true);
    expect(await order('#sortable li')).toBe('Item 1,Item 2,Item 3,Item 4,Item 5,Item 6,Item 7');
  });
});
