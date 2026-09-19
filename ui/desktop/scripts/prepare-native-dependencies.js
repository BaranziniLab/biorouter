const path = require('node:path');
const fs = require('node:fs');
const os = require('node:os');
const { execFileSync } = require('node:child_process');
const baseline = require('./linux-native-baseline.json');
const { compareVersions, verifyLinuxNativePty } = require('./verify-linux-native-pty');

async function prepareNativeDependencies(appRoot, platform, arch, {
  hostPlatform = process.platform,
  hostArch = process.arch,
  glibcVersion = process.report.getReport().header.glibcVersionRuntime,
  rebuild = require('@electron/rebuild').rebuild,
  run = execFileSync,
  verify = verifyLinuxNativePty,
} = {}) {
  if (platform !== 'linux') return;
  if (hostPlatform !== platform || hostArch !== arch) {
    throw new Error(`Build node-pty on a native ${platform}-${arch} runner before packaging.`);
  }
  if (!glibcVersion || compareVersions(glibcVersion, baseline.glibc) > 0) {
    const container = run('docker', ['create', '--cpus=1', '--memory=2g', '--platform', `linux/${arch === 'x64' ? 'amd64' : arch}`,
      '-v', `${appRoot}:/source:ro`, '-w', '/build', baseline.image, 'bash', '-euc',
      'bash /source/scripts/setup-linux-native-baseline.sh\n' +
      'mkdir -p node_modules/electron\n' +
      'cp /source/package.json package.json\n' +
      'cp -a /source/node_modules/node-pty node_modules/node-pty\n' +
      'cp /source/node_modules/electron/{package.json,install.js,checksums.json} node_modules/electron/\n' +
      'ln -s /source/node_modules/node-addon-api node_modules/node-addon-api\n' +
      'export NODE_PATH=/source/node_modules\n' +
      'node /source/scripts/prepare-native-dependencies.js /build ' + arch],
      { encoding: 'utf8', timeout: 300000 }).trim();
    if (!/^[a-f0-9]{64}$/.test(container)) throw new Error('Docker did not return an exact container ID');
    const staging = fs.mkdtempSync(path.join(os.tmpdir(), 'biorouter-native-'));
    try {
      run('docker', ['start', '--attach', container], { stdio: 'inherit', timeout: 1200000 });
      const candidate = path.join(staging, 'pty.node');
      run('docker', ['cp', `${container}:/build/node_modules/node-pty/build/Release/pty.node`, candidate], { stdio: 'inherit', timeout: 30000 });
      const destination = path.join(appRoot, 'node_modules/node-pty/build/Release/pty.node');
      fs.mkdirSync(path.dirname(destination), { recursive: true });
      fs.copyFileSync(candidate, destination);
    } finally {
      try {
        run('docker', ['rm', '--force', container], { stdio: 'inherit', timeout: 30000 });
      } finally {
        fs.rmSync(staging, { recursive: true, force: true });
      }
    }
    return;
  }
  await rebuild({
    buildPath: appRoot,
    electronVersion: require(path.join(appRoot, 'node_modules/electron/package.json')).version,
    platform,
    arch,
    onlyModules: ['node-pty'],
    force: true,
    buildFromSource: true,
  });
  run(process.execPath, [path.join(appRoot, 'node_modules/electron/install.js')], { stdio: 'inherit', timeout: 300000 });
  verify(appRoot);
}

if (require.main === module) {
  prepareNativeDependencies(path.resolve(process.argv[2]), 'linux', process.argv[3]).catch((error) => {
    console.error(error);
    process.exitCode = 1;
  });
}
module.exports = { prepareNativeDependencies };
