const assert = require('node:assert/strict');
const path = require('node:path');
const fs = require('node:fs');
const os = require('node:os');
const test = require('node:test');
const { prepareNativeDependencies } = require('./prepare-native-dependencies');
const { verifyVersionRequirements } = require('./verify-linux-native-pty');
const appRoot = path.resolve(__dirname, '..');

test('Linux packaging explicitly builds node-pty even when npm skipped install scripts', async () => {
  let options;
  await prepareNativeDependencies(appRoot, 'linux', 'x64', {
    hostPlatform: 'linux', hostArch: 'x64', glibcVersion: '2.31',
    rebuild: async (value) => { options = value; }, run: () => {}, verify: () => {},
  });
  assert.equal(options.buildPath, appRoot);
  assert.equal(options.electronVersion, require('electron/package.json').version);
  assert.deepEqual(options.onlyModules, ['node-pty']);
  assert.equal(options.force, true);
  assert.equal(options.buildFromSource, true);
  assert.equal(options.platform, 'linux');
  assert.equal(options.arch, 'x64');
});

test('native dependency preparation preserves build failure and refuses cross-host output', async () => {
  await assert.rejects(prepareNativeDependencies(appRoot, 'linux', 'x64', {
    hostPlatform: 'linux', hostArch: 'x64', glibcVersion: '2.31',
    rebuild: async () => { throw new Error('compiler failed'); },
  }), /compiler failed/);
  await assert.rejects(prepareNativeDependencies(appRoot, 'linux', 'x64', {
    hostPlatform: 'darwin', hostArch: 'arm64', rebuild: async () => { assert.fail('wrong-host build'); },
  }), /native linux-x64 runner/);
});

test('baseline rejects newer glibc, libstdc++, and C++ ABI requirements', () => {
  verifyVersionRequirements('GLIBC_2.2.5 GLIBC_2.31 GLIBCXX_3.4.28 CXXABI_1.3.12');
  for (const requirement of ['GLIBC_2.34', 'GLIBCXX_3.4.29', 'CXXABI_1.3.13']) {
    assert.throws(() => verifyVersionRequirements('GLIBC_2.2.5 ' + requirement), /above baseline/);
  }
  assert.throws(() => verifyVersionRequirements('not an ELF inspection'), /No ELF GLIBC/);
});

test('failed baseline container removes only its owned ID', async () => {
  const id = 'a'.repeat(64);
  const calls = [];
  await assert.rejects(prepareNativeDependencies(appRoot, 'linux', 'x64', {
    hostPlatform: 'linux', hostArch: 'x64', glibcVersion: '2.39',
    run: (command, args) => {
      calls.push([command, ...args]);
      if (args[0] === 'create') return id + '\n';
      if (args[0] === 'start') throw new Error('baseline failed');
    },
  }), /baseline failed/);
  assert.deepEqual(calls.at(-1), ['docker', 'rm', '--force', id]);
  assert.ok(calls[0].includes('--cpus=1'));
  assert.ok(calls[0].includes('--memory=2g'));
  assert.ok(calls[0].includes(`${appRoot}:/source:ro`));
  assert.equal(calls.some((args) => args[1] === 'cp'), false);
});

test('platforms with shipped prebuilds are not rebuilt', async () => {
  for (const platform of ['darwin', 'win32']) {
    await prepareNativeDependencies(appRoot, platform, 'x64', {
      rebuild: async () => { assert.fail('unnecessary rebuild'); },
    });
  }
});


test('successful baseline copies back only the verified PTY file', async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'native-copy-test-'));
  const electron = path.join(root, 'node_modules/electron/dist/electron');
  fs.mkdirSync(path.dirname(electron), { recursive: true });
  fs.writeFileSync(electron, 'host-electron-preserved');
  const calls = [];
  try {
    await prepareNativeDependencies(root, 'linux', 'x64', {
      hostPlatform: 'linux', hostArch: 'x64', glibcVersion: '2.39',
      run: (command, args) => {
        calls.push([command, ...args]);
        if (args[0] === 'create') return 'a'.repeat(64) + '\n';
        if (args[0] === 'cp') fs.writeFileSync(args[2], 'verified-native');
      },
    });
    assert.equal(fs.readFileSync(electron, 'utf8'), 'host-electron-preserved');
    assert.equal(fs.readFileSync(path.join(root, 'node_modules/node-pty/build/Release/pty.node'), 'utf8'), 'verified-native');
    assert.equal(calls.filter((args) => args[1] === 'cp').length, 1);
    assert.equal(fs.existsSync(path.dirname(calls.find((args) => args[1] === 'cp')[3])), false);
  } finally { fs.rmSync(root, { recursive: true, force: true }); }
});
