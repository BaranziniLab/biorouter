/**
 * Runs INSIDE Electron's main process and reports, for each stdio shape, whether
 * a console child got a visible window.
 *
 * It has to be Electron and not Node. Electron sets
 * `EnvironmentFlags::kHideConsoleWindows` on every Node environment it creates
 * (shell/common/node_bindings.cc, since Electron 16), which makes libuv take its
 * console-hiding branch for EVERY spawn regardless of the caller's
 * `windowsHide`. So the app's real behaviour is not a property of the app's
 * source, and measuring it under plain Node answers a different question.
 *
 * Invoked as: electron windows-console-electron-probe.cjs <results.json>
 */
const { app } = require('electron');
const { spawn } = require('node:child_process');
const { writeFileSync } = require('node:fs');
const { join } = require('node:path');

const resultsPath = process.argv[process.argv.length - 1];

async function main() {
  const probe = await import('./windows-console-probe.mjs');

  const CASES = [
    { name: 'pipe-without-windowsHide', stdio: 'pipe' },
    { name: 'pipe-with-windowsHide', stdio: 'pipe', windowsHide: true },
    { name: 'ignore-without-windowsHide', stdio: 'ignore' },
    { name: 'inherit-with-windowsHide', stdio: 'inherit', windowsHide: true },
  ];

  const results = {};
  for (const testCase of CASES) {
    const dir = probe.scratchDir();
    const outputPath = join(dir, 'verdict.txt');
    const options = { stdio: testCase.stdio };
    if ('windowsHide' in testCase) options.windowsHide = testCase.windowsHide;
    try {
      await new Promise((resolve, reject) => {
        const child = spawn('powershell.exe', probe.probeArgs(outputPath), options);
        child.on('error', reject);
        child.on('close', resolve);
      });
      results[testCase.name] = probe.readVerdict(outputPath, testCase.name);
    } catch (error) {
      results[testCase.name] = `ERROR: ${error.message}`;
    }
  }

  writeFileSync(resultsPath, JSON.stringify(results, null, 2));
}

app.whenReady().then(async () => {
  try {
    await main();
  } catch (error) {
    writeFileSync(resultsPath, JSON.stringify({ fatal: String(error && error.stack) }, null, 2));
  }
  app.exit(0);
});
