// Capture each reel.html slide as a 1920x1080 PNG using system Chrome.
// Run from this directory; outputs to ./frames. Requires Playwright
// (e.g. `npm i playwright`, or run with NODE_PATH pointing at an install).
//
//   NODE_PATH=/path/to/node_modules node capture-reel.cjs
//
// Then encode the frames with `python3 encode.py`.
const { chromium } = require('playwright');
const path = require('path');
const fs = require('fs');

const REEL = 'file://' + path.join(__dirname, 'reel.html');
const OUT = path.join(__dirname, 'frames');

(async () => {
  fs.mkdirSync(OUT, { recursive: true });
  const browser = await chromium.launch({ channel: 'chrome' });
  const page = await browser.newPage({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
  const errors = [];
  page.on('console', m => { if (m.type() === 'error') errors.push(m.text()); });
  page.on('pageerror', e => errors.push(String(e)));
  await page.goto(REEL, { waitUntil: 'networkidle' });
  await page.waitForTimeout(1200);
  const n = await page.evaluate(() => window.__slideCount);
  console.log('slideCount', n);
  for (let i = 0; i < n; i++) {
    await page.evaluate(i => window.__goto(i), i);
    await page.waitForTimeout(700); // let entrance settle
    const f = path.join(OUT, `slide-${String(i).padStart(2, '0')}.png`);
    await page.screenshot({ path: f, clip: { x: 0, y: 0, width: 1920, height: 1080 } });
    console.log('captured', f);
  }
  await browser.close();
  if (errors.length) { console.log('PAGE ERRORS:', errors.slice(0, 5)); }
  console.log('done');
})().catch(e => { console.error(e); process.exit(1); });
