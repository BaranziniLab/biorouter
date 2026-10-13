import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { _electron as electron } from '@playwright/test';
import { createServer } from 'vite';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const desktopDir = path.resolve(scriptDir, '..');
const repoDir = path.resolve(desktopDir, '../..');
const fixtureDir = await fs.mkdtemp(path.join(os.tmpdir(), 'biorouter-preview-fixtures-'));
const requestedOutputDir = process.env.BIOROUTER_PREVIEW_OUTPUT_DIR;
const outputDir = requestedOutputDir
  ? path.resolve(requestedOutputDir)
  : await fs.mkdtemp(path.join(os.tmpdir(), 'biorouter-preview-e2e-'));
await fs.mkdir(outputDir, { recursive: true });
const pptxSource = process.env.BIOROUTER_PREVIEW_PPTX;
const pptxTarget = path.join(fixtureDir, 'sample.pptx');
if (pptxSource) {
  await fs.copyFile(path.resolve(pptxSource), pptxTarget);
} else {
  const encoded = await fs.readFile(
    path.join(desktopDir, '.artifact-harness/sample-presentation.pptx.b64'),
    'utf8'
  );
  await fs.writeFile(pptxTarget, Buffer.from(encoded.replace(/\s/g, ''), 'base64'));
}

const fixtures = [
  [
    path.join(repoDir, 'crates/biorouter-mcp/src/webdocuments/tests/data/test_image.pdf'),
    'test.pdf',
  ],
  [
    path.join(repoDir, 'crates/biorouter-mcp/src/webdocuments/tests/data/sample.docx'),
    'sample.docx',
  ],
  [
    path.join(repoDir, 'crates/biorouter-mcp/src/webdocuments/tests/data/FinancialSample.xlsx'),
    'FinancialSample.xlsx',
  ],
  [path.join(repoDir, 'landing/icon.png'), 'volcano.png'],
];
for (const [source, name] of fixtures) {
  await fs.copyFile(source, path.join(fixtureDir, name));
}

const harnessPort = await new Promise((resolve, reject) => {
  const reservation = net.createServer();
  reservation.once('error', reject);
  reservation.listen(0, '127.0.0.1', () => {
    const address = reservation.address();
    assert(address && typeof address !== 'string', 'port reservation did not bind TCP');
    reservation.close((error) => (error ? reject(error) : resolve(address.port)));
  });
});

process.env.PREVIEW_FIXTURE_DIR = fixtureDir;
const vite = await createServer({
  configFile: path.join(desktopDir, '.artifact-harness/vite.config.mts'),
  server: { host: '127.0.0.1', port: harnessPort, strictPort: true },
  logLevel: 'warn',
});

let app;
try {
  await vite.listen();
  const address = vite.httpServer?.address();
  assert(address && typeof address !== 'string', 'preview harness did not bind a TCP port');
  const url = `http://127.0.0.1:${address.port}`;
  app = await electron.launch({
    args: [path.join(desktopDir, '.artifact-harness/electron-main.cjs')],
    cwd: desktopDir,
    env: {
      ...process.env,
      BIOROUTER_PREVIEW_HARNESS_URL: url,
      ELECTRON_RUN_AS_NODE: '',
    },
  });
  const page = await app.firstWindow();
  const nativeContentSize = await app.evaluate(({ BrowserWindow }) => {
    const window = BrowserWindow.getAllWindows()[0];
    if (!window) throw new Error('preview harness has no native window');
    window.setContentSize(1024, 645);
    return window.getContentSize();
  });
  assert.deepEqual(nativeContentSize, [1024, 645], 'native preview window dimensions drifted');
  const rendererErrors = [];
  page.on('pageerror', (error) => rendererErrors.push(error.message));
  page.on('console', (message) => {
    if (message.type() === 'error') rendererErrors.push(message.text());
  });
  await page.waitForLoadState('domcontentloaded');

  const open = async (testId) => {
    await page.getByTestId(testId).click();
    await page.getByTestId('artifact-viewer').waitFor({ state: 'visible' });
  };
  const screenshot = async (name) => {
    await page.screenshot({ path: path.join(outputDir, `${name}.png`) });
  };
  const contentBounds = {};
  const assertVisibleContent = async (name, content, scroller) => {
    const viewport = await page.evaluate(() => ({
      width: window.innerWidth,
      height: window.innerHeight,
      scrollX: window.scrollX,
      scrollY: window.scrollY,
    }));
    const viewer = await page.getByTestId('artifact-viewer').boundingBox();
    const bounds = await content.boundingBox();
    const clip = scroller ? await scroller.boundingBox() : viewer;
    assert(viewer && bounds, `${name} preview content must have bounds`);
    assert(clip, `${name} preview scroller must have bounds`);
    assert.equal(viewport.width, 1024, `${name} renderer viewport width drifted`);
    assert.equal(viewport.height, 645, `${name} renderer viewport height drifted`);
    assert.equal(viewport.scrollX, 0, `${name} must not scroll the harness document horizontally`);
    assert.equal(viewport.scrollY, 0, `${name} must not scroll the harness document vertically`);
    assert(
      viewer.x >= -1 &&
        viewer.y >= -1 &&
        viewer.x + viewer.width <= viewport.width + 1 &&
        viewer.y + viewer.height <= viewport.height + 1,
      `${name} viewer is outside the native viewport: ${JSON.stringify({ viewer, viewport })}`
    );
    const visible = {
      width:
        Math.min(
          bounds.x + bounds.width,
          clip.x + clip.width,
          viewer.x + viewer.width,
          viewport.width
        ) - Math.max(bounds.x, clip.x, viewer.x, 0),
      height:
        Math.min(
          bounds.y + bounds.height,
          clip.y + clip.height,
          viewer.y + viewer.height,
          viewport.height
        ) - Math.max(bounds.y, clip.y, viewer.y, 0),
    };
    assert(
      bounds.width > 0 &&
        bounds.height > 0 &&
        visible.width >= Math.min(bounds.width, 80) &&
        visible.height >= Math.min(bounds.height, 32),
      `${name} content is outside the visible viewer: ${JSON.stringify({ bounds, visible })}`
    );
    contentBounds[name] = { viewport, viewer, scroller: clip, content: bounds, visible };
  };

  await open('open-volcano.png');
  await page.waitForFunction(() => {
    const image = document.querySelector('[data-testid="artifact-viewer"] img');
    return image instanceof HTMLImageElement && image.complete && image.naturalWidth > 0;
  });
  await assertVisibleContent('image', page.getByTestId('artifact-viewer').locator('img').first());
  await screenshot('image');

  await open('open-test.pdf');
  try {
    await page.waitForFunction(() => {
      const canvas = document.querySelector(
        '[data-testid="artifact-viewer"] canvas[data-rendered="true"]'
      );
      return canvas instanceof HTMLCanvasElement && canvas.width > 0 && canvas.height > 0;
    });
  } catch (error) {
    await screenshot('pdf-failure');
    const panelText = await page
      .getByTestId('panel-host')
      .innerText()
      .catch(() => 'panel unavailable');
    throw new Error(
      `PDF preview did not finish rendering. Output: ${outputDir}\n${panelText}\n${rendererErrors.join('\n')}`,
      { cause: error }
    );
  }
  assert.doesNotMatch(
    await page.getByTestId('artifact-viewer').innerText(),
    /Could not render page/i
  );
  await assertVisibleContent(
    'pdf',
    page.getByTestId('artifact-viewer').locator('canvas[data-rendered="true"]').first(),
    page.locator('.artifact-document-scroll').first()
  );
  await screenshot('pdf');

  await open('open-sample.docx');
  await page.locator('.artifact-docx-preview .docx-wrapper > section.docx').first().waitFor();
  const paragraph = page
    .locator('.artifact-docx-preview .docx-wrapper > section.docx p')
    .filter({ hasText: /\S/ })
    .first();
  await paragraph.waitFor();
  await assertVisibleContent('docx', paragraph, page.locator('.artifact-document-scroll').first());
  await screenshot('docx');

  await open('open-FinancialSample.xlsx');
  const sheet = page.frameLocator('iframe[name="biorouter-spreadsheet-preview"]');
  await sheet.locator('table').first().waitFor();
  assert((await sheet.locator('body').innerText()).trim().length > 0, 'workbook rendered empty');
  await assertVisibleContent(
    'xlsx',
    sheet.locator('table').first(),
    page.locator('iframe[name="biorouter-spreadsheet-preview"]')
  );
  await screenshot('xlsx');

  await open('open-sample.pptx');
  await page.waitForFunction(
    () => {
      const preview = document.querySelector('.artifact-pptx-preview[data-rendered="true"]');
      return preview instanceof HTMLElement && preview.childElementCount > 0;
    },
    null,
    { timeout: 60_000 }
  );
  assert.doesNotMatch(await page.getByTestId('artifact-viewer').innerText(), /Could not render/i);
  const slide = page
    .locator(
      '.artifact-pptx-preview[data-rendered="true"] [data-slide-index] > :first-child > :first-child'
    )
    .first();
  await slide.waitFor();
  assert((await slide.innerText()).trim().length > 0, 'presentation rendered an empty slide');
  await assertVisibleContent('pptx', slide, page.locator('.artifact-pptx-scroll'));
  await screenshot('pptx');

  // "Send a region to the chat" lives in the header's More actions menu (spec 3.8).
  await page.getByLabel('More actions').click();
  await page.getByRole('menuitem', { name: 'Send a region to the chat' }).click();
  const overlay = page.getByTestId('annotation-overlay');
  const bounds = await overlay.boundingBox();
  assert(bounds, 'annotation overlay must have bounds');
  await page.mouse.move(bounds.x + 120, bounds.y + 160);
  await page.mouse.down();
  await page.mouse.move(bounds.x + 420, bounds.y + 390);
  await page.mouse.up();
  await page.waitForFunction(() => Boolean(window.__lastCapture));
  const capture = await page.evaluate(() => window.__lastCapture);
  assert(capture.width >= 299 && capture.height >= 229, 'annotation crop dimensions drifted');
  await assertVisibleContent('annotation-crop', slide, page.locator('.artifact-pptx-scroll'));
  await screenshot('annotation-crop');
  assert.deepEqual(rendererErrors, [], `renderer emitted errors:\n${rendererErrors.join('\n')}`);

  process.stdout.write(
    `${JSON.stringify({ outputDir, coverage: ['image', 'pdf', 'docx', 'xlsx', 'pptx', 'annotation-crop'], capture, nativeContentSize, contentBounds }, null, 2)}\n`
  );
} finally {
  await app?.close().catch(() => undefined);
  await vite.close().catch(() => undefined);
  await fs.rm(fixtureDir, { recursive: true, force: true });
}
