#!/usr/bin/env node
/**
 * Dev smoke check: can the node-pty in this tree start one shell under Electron's
 * own Node?
 *
 *   cd ui/desktop && ELECTRON_RUN_AS_NODE=1 npx electron scripts/smoke-node-pty.cjs
 *
 * It has to run under Electron, not plain Node. node-pty's `pty.node` is built for
 * Electron's ABI, so a plain `node` run fails with NODE_MODULE_VERSION for a reason
 * that says nothing about the app. Run as Node, Electron loads it the way the main
 * process does.
 *
 * It loads node-pty both ways `loadNodePty` (src/main.ts) does, the CommonJS loader
 * and the ESM import, reports whether each gives a real `spawn` function, and then
 * starts `echo` through the first one that does. The exit code is 0 only when a shell
 * really ran. An import that gives the right names and no values (V1-R3-1) shows here
 * as "no spawn function", and is survivable as long as the require line is ok.
 *
 * Added for V1-R3-1 ("pty.spawn is not a function" on every terminal dock tab).
 */
'use strict';

const { createRequire } = require('node:module');
const path = require('node:path');

const here = path.join(__dirname, '..', 'src', 'main.ts');
const spawnOf = (m) =>
  m && typeof m.spawn === 'function'
    ? m
    : m && m.default && typeof m.default.spawn === 'function'
      ? m.default
      : null;

async function main() {
  if (!process.versions.electron) {
    console.error(
      'Run this under Electron: ELECTRON_RUN_AS_NODE=1 npx electron scripts/smoke-node-pty.cjs\n' +
        "(node-pty is built for Electron's ABI, so plain node cannot load it.)"
    );
    return 2;
  }
  console.log(
    `electron ${process.versions.electron}, node ${process.versions.node}, ${process.platform}-${process.arch}`
  );

  const loaders = [
    ['require', async () => createRequire(here)('node-pty')],
    ['import', () => import('node-pty')],
  ];
  let usable = null;
  for (const [via, load] of loaders) {
    try {
      const m = await load();
      const ok = spawnOf(m);
      console.log(
        `${via.padEnd(8)} ${ok ? 'ok' : 'no spawn function (names: ' + Object.keys(m).join(', ') + ')'}`
      );
      if (ok && !usable) usable = ok;
    } catch (error) {
      console.log(`${via.padEnd(8)} threw: ${error && error.message ? error.message : error}`);
    }
  }
  if (!usable) {
    console.error('FAIL: no loader produced a usable node-pty');
    return 1;
  }

  const [file, args] =
    process.platform === 'win32'
      ? ['cmd.exe', ['/c', 'echo smoke']]
      : ['/bin/sh', ['-c', 'echo smoke']];
  let output = '';
  const term = usable.spawn(file, args, {
    name: 'xterm-256color',
    cols: 80,
    rows: 24,
    cwd: process.cwd(),
    env: process.env,
  });
  term.onData((d) => {
    output += d;
  });
  const exit = await new Promise((resolve) => {
    const timer = setTimeout(() => resolve({ exitCode: null, timedOut: true }), 10000);
    term.onExit((e) => {
      clearTimeout(timer);
      resolve(e);
    });
  });
  const passed = !exit.timedOut && exit.exitCode === 0 && output.includes('smoke');
  console.log(
    `spawn    ${passed ? 'ok' : 'FAILED'} (exit ${exit.exitCode}, output ${JSON.stringify(output.trim())})`
  );
  return passed ? 0 : 1;
}

main().then(
  (code) => process.exit(code),
  (error) => {
    console.error('FAIL:', error);
    process.exit(1);
  }
);
