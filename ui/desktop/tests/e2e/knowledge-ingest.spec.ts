import { test, expect, Page } from '@playwright/test';
import AdmZip from 'adm-zip';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { closeApp, launchApp, type LaunchedApp } from './helpers/app';
import { openSidebarEntry } from './helpers/sidebar';

let launched: LaunchedApp;
let page: Page;
let tmpDir: string;
let folderFixturePath: string;

type FixtureCase = {
  filePath: string;
  label: string;
  expectedPages: number;
};

test.describe('Knowledge ingest workflow', () => {
  test.skip(
    process.env.BIOROUTER_E2E_LIVE !== '1',
    'Set BIOROUTER_E2E_LIVE=1 to run the Electron end-to-end suite.'
  );

  test.beforeAll(async () => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'biorouter-knowledge-ingest-'));
    folderFixturePath = writeFixtures(tmpDir);

    // BIOROUTER_KNOWLEDGE_TEST_MODE keeps the digest off a real provider, so
    // this suite ingests without spending a model turn.
    launched = await launchApp({
      env: {
        BIOROUTER_KNOWLEDGE_TEST_MODE: '1',
        PLAYWRIGHT_SELECT_PATH: folderFixturePath,
      },
    });
    page = launched.page;
    await page.waitForTimeout(1500);
  });

  test.afterAll(async () => {
    await closeApp(launched);
    if (tmpDir && fs.existsSync(tmpDir)) {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  });

  test('digests supported files and updates the graph without errors', async () => {
    await openSidebarEntry(page, 'Knowledge');
    await expect(page.getByRole('heading', { name: 'Knowledge' })).toBeVisible();

    const kbName = `Playwright KB ${Date.now()}`;
    await createKnowledgeBase(page, kbName);

    const digestButton = page.getByTestId('knowledge-digest-button');
    const graphSummary = page.getByTestId('knowledge-graph-summary');

    for (const fixture of fixtureCases(tmpDir)) {
      await page
        .locator('[data-testid="knowledge-ingest-file-input"]')
        .setInputFiles(fixture.filePath);

      const stagedItem = page.locator(
        `[data-testid="knowledge-staged-item"][data-label="${fixture.label}"]`
      );
      await expect(stagedItem).toBeVisible();
      await expect(digestButton).toBeEnabled();

      await digestButton.click();
      // While busy the button is the ABORT control: `Stop`, or `Stopping…`
      // once abort is pending (IngestPanel.tsx:636). `digestLabel`'s busy
      // branches — 'Checking model…' / 'Digesting…' — are unreachable there,
      // because `busy` is exactly `digestState !== 'idle'`.
      await expect(digestButton).toHaveText(/^Stop(ping…)?$/);
      await expect(digestButton).toHaveText('Digest staged sources', {
        timeout: 90000,
      });
      await expect(stagedItem).toHaveCount(0, { timeout: 90000 });

      await expect(graphSummary).toContainText(
        `${fixture.expectedPages} ${fixture.expectedPages === 1 ? 'page' : 'pages'}`,
        { timeout: 90000 }
      );
    }

    await expect(page.getByTestId('knowledge-graph-canvas')).toBeVisible();
    await expect(
      page.getByText('No pages yet. Ingest a source to populate the graph.')
    ).toHaveCount(0);
  });

  test('stages folders and archives through the desktop flow and still forms the graph', async () => {
    await openSidebarEntry(page, 'Knowledge');
    await expect(page.getByRole('heading', { name: 'Knowledge' })).toBeVisible();

    const kbName = `Playwright Path KB ${Date.now()}`;
    await createKnowledgeBase(page, kbName);

    const digestButton = page.getByTestId('knowledge-digest-button');
    const graphSummary = page.getByTestId('knowledge-graph-summary');

    await page.getByText('Drag and drop to stage').click();
    await page.getByTestId('knowledge-ingest-browse-path').click();
    await expect(
      page.locator('[data-testid="knowledge-staged-item"][data-label="folder-input/alpha.md"]')
    ).toBeVisible();
    await expect(
      page.locator('[data-testid="knowledge-staged-item"][data-label="folder-input/beta.txt"]')
    ).toBeVisible();

    await digestButton.click();
    await expect(digestButton).toHaveText('Digest staged sources', {
      timeout: 90000,
    });
    await expect(graphSummary).toContainText('2 pages', { timeout: 90000 });

    await page
      .locator('[data-testid="knowledge-ingest-file-input"]')
      .setInputFiles(path.join(tmpDir, 'bundle.zip'));

    await expect(
      page.locator('[data-testid="knowledge-staged-item"][data-label="bundle/gamma.md"]')
    ).toBeVisible();
    await expect(
      page.locator('[data-testid="knowledge-staged-item"][data-label="bundle/delta.csv"]')
    ).toBeVisible();

    await digestButton.click();
    await expect(digestButton).toHaveText('Digest staged sources', {
      timeout: 90000,
    });
    await expect(graphSummary).toContainText('4 pages', { timeout: 90000 });
    await expect(page.getByTestId('knowledge-graph-canvas')).toBeVisible();
  });
});

/**
 * Creates a knowledge base and leaves it as the session's primary.
 *
 * Three steps, not one, and the shape has changed since this spec was written:
 * the selector menu only OFFERS creation (`knowledge-kb-open-create`), the
 * manager dialog hosts it, and the name is typed into `KbFormatChooser` — which
 * exists because a base now declares a `format` (defaulting to OKF, so no radio
 * needs touching). `knowledge-kb-create` / `knowledge-kb-name-input` /
 * `knowledge-kb-submit`, which this spec used to drive, are the MANAGER's own
 * controls: the first opens the chooser and the other two belong to the RENAME
 * draft, which only renders once a rename is in progress.
 */
async function createKnowledgeBase(page: Page, kbName: string): Promise<void> {
  await page.getByTestId('knowledge-kb-selector-trigger').click();
  await page.getByTestId('knowledge-kb-open-create').click();
  await page.getByTestId('knowledge-format-name').fill(kbName);
  await page.getByTestId('knowledge-format-submit').click();
  // The manager dialog stays open behind the chooser; its overlay would
  // intercept every click on the ingest panel underneath.
  await expect(page.getByTestId('knowledge-format-submit')).toHaveCount(0, { timeout: 30000 });
  await page.keyboard.press('Escape');
  await expect(page.locator('[data-slot="dialog-overlay"][data-state="open"]')).toHaveCount(0, {
    timeout: 10000,
  });
  await expect(page.getByTestId('knowledge-kb-selector-trigger')).toContainText(kbName);
}

function fixtureCases(baseDir: string): FixtureCase[] {
  return [
    { filePath: path.join(baseDir, 'note.md'), label: 'note.md', expectedPages: 1 },
    { filePath: path.join(baseDir, 'note.txt'), label: 'note.txt', expectedPages: 2 },
    { filePath: path.join(baseDir, 'table.csv'), label: 'table.csv', expectedPages: 3 },
    { filePath: path.join(baseDir, 'article.html'), label: 'article.html', expectedPages: 4 },
    { filePath: path.join(baseDir, 'sample.pdf'), label: 'sample.pdf', expectedPages: 5 },
    { filePath: path.join(baseDir, 'sample.docx'), label: 'sample.docx', expectedPages: 6 },
  ];
}

function writeFixtures(baseDir: string): string {
  fs.writeFileSync(
    path.join(baseDir, 'note.md'),
    '# Markdown Fixture\n\nDigest this markdown note.'
  );
  fs.writeFileSync(path.join(baseDir, 'note.txt'), 'Plain text fixture for ingestion.');
  fs.writeFileSync(path.join(baseDir, 'table.csv'), 'name,score\nAlice,9\nBob,7\n');
  fs.writeFileSync(
    path.join(baseDir, 'article.html'),
    fs.readFileSync(
      path.join(
        __dirname,
        '../../../../crates/biorouter-mcp/src/knowledge/convert/fixtures/article.html'
      )
    )
  );
  fs.writeFileSync(
    path.join(baseDir, 'sample.pdf'),
    fs.readFileSync(
      path.join(
        __dirname,
        '../../../../crates/biorouter-mcp/src/computercontroller/tests/data/test.pdf'
      )
    )
  );
  fs.writeFileSync(
    path.join(baseDir, 'sample.docx'),
    fs.readFileSync(
      path.join(
        __dirname,
        '../../../../crates/biorouter-mcp/src/computercontroller/tests/data/sample.docx'
      )
    )
  );

  const folderInput = path.join(baseDir, 'folder-input');
  fs.mkdirSync(folderInput, { recursive: true });
  fs.writeFileSync(path.join(folderInput, 'alpha.md'), '# Alpha\n\nFolder fixture markdown.');
  fs.writeFileSync(path.join(folderInput, 'beta.txt'), 'Folder fixture plain text.');
  fs.writeFileSync(path.join(folderInput, 'ignore.exe'), Buffer.from([0, 1, 2, 3]));

  const zip = new AdmZip();
  zip.addFile('docs/gamma.md', Buffer.from('# Gamma\n\nArchive fixture markdown.'));
  zip.addFile('docs/delta.csv', Buffer.from('name,score\nGamma,8\nDelta,7\n'));
  zip.addFile('__MACOSX/docs/._gamma.md', Buffer.from('metadata'));
  zip.writeZip(path.join(baseDir, 'bundle.zip'));

  return folderInput;
}
