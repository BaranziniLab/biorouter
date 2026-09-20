const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const root = path.resolve(__dirname, '../../..');
const vendor = path.join(root, 'vendor/computer-use');
const pin = JSON.parse(fs.readFileSync(path.join(vendor, 'pin.json'), 'utf8'));
const hash = (file) => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');

function files(directory, base = directory) {
  return fs
    .readdirSync(directory, { withFileTypes: true })
    .flatMap((entry) => {
      const full = path.join(directory, entry.name);
      if (entry.isSymbolicLink()) throw new Error(`Unexpected helper symlink: ${full}`);
      if (entry.isDirectory()) return files(full, base);
      return full === path.join(base, 'manifest.json')
        ? []
        : [{ path: path.relative(base, full).split(path.sep).join('/'), sha256: hash(full) }];
    })
    .sort((a, b) => a.path.localeCompare(b.path, 'en'));
}

function verifyComputerUse(directory, target) {
  const manifest = JSON.parse(fs.readFileSync(path.join(directory, 'manifest.json'), 'utf8'));
  for (const key of ['schema_version', 'upstream_commit', 'upstream_version', 'patch_revision']) {
    if (manifest[key] !== pin[key])
      throw new Error(`Biorouter Copilot ${key} mismatch; rebuild ${target}`);
  }
  const executable = target.startsWith('darwin-')
    ? 'BioRouter Computer Use.app/Contents/MacOS/ocu'
    : target === 'win32-x64'
      ? 'ocu.exe'
      : 'ocu';
  if (
    !pin.targets.includes(target) ||
    manifest.target !== target ||
    manifest.executable !== executable
  ) {
    throw new Error(`Missing or foreign Biorouter Copilot helper: expected ${target}`);
  }
  const expectedPatches = fs
    .readdirSync(path.join(vendor, 'patches'))
    .filter((p) => p.endsWith('.patch'))
    .sort()
    .map((p) => ({ path: p, sha256: hash(path.join(vendor, 'patches', p)) }));
  if (JSON.stringify(manifest.patches) !== JSON.stringify(expectedPatches)) {
    throw new Error('Biorouter Copilot source patches changed; rebuild helper');
  }
  const actualFiles = files(directory);
  const recordedFiles = [...manifest.files].sort((a, b) => a.path.localeCompare(b.path, 'en'));
  if (JSON.stringify(actualFiles) !== JSON.stringify(recordedFiles)) {
    throw new Error('Biorouter Copilot files changed or are missing; rebuild helper');
  }
  const data = fs.readFileSync(path.join(directory, executable));
  let valid = false;
  if (target.startsWith('darwin-') && data.readUInt32LE(0) === 0xfeedfacf) {
    valid = data.readUInt32LE(4) === (target.endsWith('arm64') ? 0x0100000c : 0x01000007);
  } else if (target.startsWith('linux-')) {
    valid =
      data.subarray(0, 6).equals(Buffer.from([127, 69, 76, 70, 2, 1])) &&
      data.readUInt16LE(18) === (target.endsWith('arm64') ? 183 : 62);
  } else if (target === 'win32-x64' && data.subarray(0, 2).toString() === 'MZ') {
    valid = data
      .subarray(data.readUInt32LE(60), data.readUInt32LE(60) + 6)
      .equals(Buffer.from([80, 69, 0, 0, 100, 134]));
  }
  if (!valid) throw new Error(`Biorouter Copilot binary architecture mismatch: ${target}`);
  if (process.platform !== 'win32' && !target.startsWith('win32-')) {
    fs.accessSync(path.join(directory, executable), fs.constants.X_OK);
  }
  return manifest;
}

function stageComputerUse(platform, arch) {
  const target = `${platform}-${arch}`;
  const source = path.join(root, 'target/computer-use', target);
  const destination = path.join(root, 'ui/desktop/src/computer-use');
  // ⚠ Say what is missing and how to produce it. This is the FIRST statement of
  // `preparePlatformBinaries`, so it is the first thing every packaging command
  // reaches — `just make-ui`, `make-ui-windows`, `make-ui-intel`,
  // `make-ui-linux`, and `npm run bundle:*` directly. Nothing in the Justfile
  // builds this directory: in the whole tree only `scripts/release.sh` and the
  // CI workflow invoke `computer-use-runtime.py build`. Without this check the
  // developer gets a bare
  //     ENOENT: no such file or directory, open '.../manifest.json'
  // from inside `verifyComputerUse`, which names neither the payload nor the
  // command that creates it.
  if (!fs.existsSync(path.join(source, 'manifest.json'))) {
    throw new Error(
      `Biorouter Copilot helper payload missing for ${target}.\n` +
        `  expected: ${source}\n` +
        `  build it: python3 scripts/computer-use-runtime.py build ${target}\n` +
        `            (needs Go; see .github/workflows/computer-use-native.yml)\n` +
        `  Packaging bundles this helper, so it cannot be skipped.`
    );
  }
  verifyComputerUse(source, target);
  fs.rmSync(destination, { recursive: true, force: true });
  fs.cpSync(source, destination, { recursive: true });
  verifyComputerUse(destination, target);
}

module.exports = { stageComputerUse, verifyComputerUse };
