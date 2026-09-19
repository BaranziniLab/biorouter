const path = require('node:path');
const { execFileSync } = require('node:child_process');
const baseline = require('./linux-native-baseline.json');

function compareVersions(left, right) {
  const a = left.split('.').map(Number);
  const b = right.split('.').map(Number);
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    if ((a[i] || 0) !== (b[i] || 0)) return (a[i] || 0) - (b[i] || 0);
  }
  return 0;
}

function verifyVersionRequirements(output) {
  for (const [symbol, limit] of [['GLIBC', baseline.glibc], ['GLIBCXX', baseline.glibcxx], ['CXXABI', baseline.cxxabi]]) {
    for (const match of output.matchAll(new RegExp(`\\b${symbol}_(\\d+(?:\\.\\d+)+)\\b`, 'g'))) {
      if (compareVersions(match[1], limit) > 0) {
        throw new Error(`node-pty requires ${symbol}_${match[1]}, above baseline ${symbol}_${limit}`);
      }
    }
  }
  if (!/\bGLIBC_\d/.test(output)) throw new Error('No ELF GLIBC requirements found for node-pty');
}

function verifyLinuxNativePty(appRoot) {
  const native = path.join(appRoot, 'node_modules/node-pty/build/Release/pty.node');
  const requirements = execFileSync('readelf', ['--version-info', native], { encoding: 'utf8' });
  verifyVersionRequirements(requirements);
  console.log(requirements);
  const electron = path.join(appRoot, 'node_modules/electron/dist/electron');
  execFileSync(electron, [path.join(__dirname, 'verify-linux-native-pty.js'), '--electron-probe', appRoot], {
    env: { ...process.env, ELECTRON_RUN_AS_NODE: '1' }, stdio: 'inherit', timeout: 15000,
  });
}

function electronProbe(appRoot) {
  const expected = require(path.join(appRoot, 'node_modules/electron/package.json')).version;
  if (process.versions.electron !== expected) throw new Error('PTY probe did not run with the packaged Electron ABI');
  const pty = require(path.join(appRoot, 'node_modules/node-pty')).spawn('/bin/sh', ['-c', 'printf baseline-pty-ok'], { cols: 80, rows: 24 });
  let output = '';
  let exited = false;
  const timeout = setTimeout(() => { pty.kill(); throw new Error('Baseline Electron PTY did not finish'); }, 10000);
  function finish() {
    if (exited && output.includes('baseline-pty-ok')) {
      clearTimeout(timeout);
      console.log(JSON.stringify({ electron: process.versions.electron, modules: process.versions.modules,
        glibc: process.report.getReport().header.glibcVersionRuntime, pty: 'spawn-output-exit-verified' }));
    }
  }
  pty.onData((data) => { output += data; finish(); });
  pty.onExit(({ exitCode }) => {
    if (exitCode !== 0) throw new Error(`Baseline Electron PTY exited ${exitCode}`);
    exited = true;
    finish();
  });
}

if (require.main === module && process.argv[2] === '--electron-probe') electronProbe(process.argv[3]);
module.exports = { compareVersions, verifyVersionRequirements, verifyLinuxNativePty };
