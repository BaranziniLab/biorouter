const assert = require('node:assert/strict');
const fs = require('fs-extra');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');
const glob = require('fast-glob');
const asar = require('@electron/asar');
const { verifyPackagedDependencies, validateArchiveLinks } = require('./verify-packaged-dependencies');
const { packagerConfig } = require('../forge.config.ts');
const { spawnSync } = require('node:child_process');

async function fixture(run) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'biorouter-package-deps-'));
  try { await run(root); } finally { await fs.remove(root); }
}

test('Forge dependency copy cannot mutate the source bin links', async () => fixture(async (root) => {
  const source = path.join(root, 'source');
  const modules = path.join(root, 'dependencies');
  const build = path.join(root, 'build');
  await fs.outputFile(path.join(modules, '.bin/tool'), 'source tool');
  await fs.ensureDir(source);
  await fs.symlink(modules, path.join(source, 'node_modules'), 'dir');
  assert.equal(packagerConfig.derefSymlinks, true);
  await fs.copy(source, build, { dereference: packagerConfig.derefSymlinks });
  // Reproduce Forge's afterCopy removal, which follows copied directory links.
  for (const bin of await glob(path.join(build, '**/.bin/**/*'))) await fs.remove(bin);
  assert.equal(await fs.readFile(path.join(modules, '.bin/tool'), 'utf8'), 'source tool');
  assert.equal((await fs.lstat(path.join(build, 'node_modules'))).isSymbolicLink(), false);
}));

test('archive validation rejects a dependency link to an external install', async () => fixture(async (root) => {
  const source = path.join(root, 'source');
  const resources = path.join(root, 'resources');
  await fs.ensureDir(source);
  await fs.ensureDir(resources);
  await fs.ensureDir(path.join(root, 'external'));
  await fs.symlink(path.join(root, 'external'), path.join(source, 'node_modules'), 'dir');
  await asar.createPackage(source, path.join(resources, 'app.asar'));
  assert.throws(() => verifyPackagedDependencies(resources, 'darwin', 'arm64'), /link/);
}));

test('archive validation requires unpacked native modules for the selected architecture', async () => fixture(async (root) => {
  const source = path.join(root, 'source');
  const resources = path.join(root, 'resources');
  await fs.outputFile(path.join(source, 'node_modules/node-pty/lib/index.js'), 'module.exports = {};');
  await fs.symlink('index.js', path.join(source, 'node_modules/node-pty/lib/internal-link.js'));
  await fs.outputFile(path.join(source, 'node_modules/node-pty/prebuilds/darwin-arm64/pty.node'), 'placement fixture');
  const helper = path.join(source, 'node_modules/node-pty/prebuilds/darwin-arm64/spawn-helper');
  await fs.outputFile(helper, '#!/bin/sh\nexit 0\n');
  await fs.chmod(helper, 0o755);
  await fs.ensureDir(resources);
  await asar.createPackageWithOptions(source, path.join(resources, 'app.asar'), { unpack: '**/node_modules/node-pty/**' });
  verifyPackagedDependencies(resources, 'darwin', 'arm64');
  assert.throws(() => verifyPackagedDependencies(resources, 'darwin', 'x64'), /native module missing/);
}));

test('Linux Forge filter retains its built native module in the unpacked archive', async () => fixture(async (root) => {
  const candidates = [
    '/node_modules/node-pty/build',
    '/node_modules/node-pty/build/Release',
    '/node_modules/node-pty/build/Release/pty.node',
    '/node_modules/node-pty/build/Release/obj.target/source.o',
    '/node_modules/node-pty/prebuilds/darwin-arm64/pty.node',
  ];
  const inspect = spawnSync(process.execPath, ['-e',
    `const cfg=require('./forge.config.ts').packagerConfig; process.stdout.write(JSON.stringify(${JSON.stringify(candidates)}.map(p=>!cfg.ignore(p))));`], {
    cwd: path.resolve(__dirname, '..'), encoding: 'utf8',
    env: { ...process.env, ELECTRON_PLATFORM: 'linux', ELECTRON_ARCH: 'x64' },
  });
  assert.equal(inspect.status, 0, inspect.stderr);
  assert.deepEqual(JSON.parse(inspect.stdout), [true, true, true, false, false]);
  const source = path.join(root, 'source');
  const resources = path.join(root, 'resources');
  await fs.outputFile(path.join(source, 'node_modules/node-pty/lib/index.js'), 'module.exports = {};');
  const native = 'node_modules/node-pty/build/Release/pty.node';
  await fs.outputFile(path.join(source, native), 'native placement fixture');
  await fs.ensureDir(resources);
  await asar.createPackageWithOptions(source, path.join(resources, 'app.asar'), packagerConfig.asar);
  verifyPackagedDependencies(resources, 'linux', 'x64');
  await fs.remove(path.join(resources, 'app.asar.unpacked', native));
  assert.throws(() => verifyPackagedDependencies(resources, 'linux', 'x64'), /native module missing/);
}));


test('archive links normalize Windows separators and reject escapes or cycles', () => {
  validateArchiveLinks([['\\node_modules', { link: 'bundled\\modules' }], ['\\bundled\\modules', { files: {} }]]);
  for (const link of ['C:\\external', 'C:relative', '\\\\server\\share', '/outside', '..\\outside']) {
    assert.throws(() => validateArchiveLinks([['\\node_modules', { link }]]), /escapes/);
  }
  assert.throws(() => validateArchiveLinks([['/a', { link: 'b' }], ['/b', { link: 'a' }]]), /cycle/);
});
