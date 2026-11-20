// Stages and verifies the Biorouter Copilot engine payload (agent-desktop) that
// every desktop package carries in `resources/computer-use/`. The payload is built
// by scripts/computer-use-runtime.py, whose `verify` this mirrors so the packager
// refuses exactly what the build script refuses (AD-03 and AD-04 in
// docs/design/copilot-agent-desktop.md).
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const root = path.resolve(__dirname, '../../..');
const vendor = path.join(root, 'vendor/agent-desktop');
const pin = JSON.parse(fs.readFileSync(path.join(vendor, 'pin.json'), 'utf8'));
const hash = (file) => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');

const MANIFEST_SCHEMA = 2;
const PINNED_KEYS = [
  'engine',
  'upstream_tag',
  'upstream_commit',
  'upstream_version',
  'contract_version',
];
const LICENSE_FILE = 'LICENSE.agent-desktop';
const DEPENDENCIES_FILE = 'DEPENDENCIES.agent-desktop.txt';
// The macOS bundle folder under computer-use/. forge.config.ts keeps
// @electron/osx-sign away from it, because it is signed before its byte
// manifest is written.
const COPILOT_BUNDLE = pin.macos_bundle;

function workspaceVersion() {
  const cargo = fs.readFileSync(path.join(root, 'Cargo.toml'), 'utf8');
  const section = /^\[workspace\.package\]([\s\S]*?)(?=^\[|(?![\s\S]))/m.exec(cargo);
  const version = section && /^version\s*=\s*"([^"]+)"/m.exec(section[1]);
  if (!version) throw new Error('Cannot read [workspace.package] version from Cargo.toml');
  return version[1];
}

function executable(target) {
  if (target.startsWith('darwin-')) return `${COPILOT_BUNDLE}/Contents/MacOS/agent-desktop`;
  return target === 'win32-x64' ? 'agent-desktop.exe' : 'agent-desktop';
}

function minimumOs(target) {
  if (target.startsWith('darwin-')) return { macos: pin.minimum_macos };
  if (target.startsWith('linux-')) return { glibc: pin.glibc_floor };
  return { windows: '10' };
}

// The Python build sorts Path objects component by component, so the two orders
// can differ (`a-b` against `a/b`); both lists are sorted here before comparing.
const byPath = (a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0);

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
    .sort(byPath);
}

// The bundle's identity, read from the XML property list the build writes. A
// different identifier or executable would move macOS's Accessibility and Screen
// Recording grants to another program (AD-04).
function checkBundle(directory) {
  const plist = fs.readFileSync(
    path.join(directory, COPILOT_BUNDLE, 'Contents/Info.plist'),
    'utf8'
  );
  const value = (key) => {
    const match = new RegExp(`<key>${key}</key>\\s*<string>([^<]*)</string>`).exec(plist);
    return match && match[1];
  };
  if (value('CFBundleIdentifier') !== pin.bundle_identifier)
    throw new Error('Biorouter Copilot bundle identifier changed; rebuild helper');
  if (value('CFBundleExecutable') !== 'agent-desktop')
    throw new Error('Biorouter Copilot bundle executable changed; rebuild helper');
  if (value('LSMinimumSystemVersion') !== pin.minimum_macos)
    throw new Error('Biorouter Copilot bundle minimum macOS changed; rebuild helper');
}

function verifyComputerUse(directory, target) {
  const manifest = JSON.parse(fs.readFileSync(path.join(directory, 'manifest.json'), 'utf8'));
  if (manifest.schema_version !== MANIFEST_SCHEMA) {
    throw new Error(
      `Biorouter Copilot payload predates the agent-desktop engine; rebuild ${target}`
    );
  }
  for (const key of PINNED_KEYS) {
    if (manifest[key] !== pin[key])
      throw new Error(`Biorouter Copilot ${key} mismatch; rebuild ${target}`);
  }
  if (manifest.biorouter_version !== workspaceVersion()) {
    throw new Error(
      `Biorouter Copilot helper was built for ${manifest.biorouter_version}, not ${workspaceVersion()}; rebuild ${target}`
    );
  }
  if (
    !Object.prototype.hasOwnProperty.call(pin.targets, target) ||
    manifest.target !== target ||
    manifest.triple !== pin.targets[target] ||
    manifest.executable !== executable(target) ||
    JSON.stringify(manifest.minimum_os) !== JSON.stringify(minimumOs(target))
  ) {
    throw new Error(`Missing or foreign Biorouter Copilot helper: expected ${target}`);
  }
  const actualFiles = files(directory);
  const recordedFiles = [...(manifest.files || [])].sort(byPath);
  if (JSON.stringify(actualFiles) !== JSON.stringify(recordedFiles)) {
    throw new Error('Biorouter Copilot files changed or are missing; rebuild helper');
  }
  const names = new Set(actualFiles.map((entry) => entry.path));
  for (const required of [executable(target), LICENSE_FILE, DEPENDENCIES_FILE]) {
    if (!names.has(required)) throw new Error(`Biorouter Copilot payload has no ${required}`);
  }
  if (
    !fs
      .readFileSync(path.join(directory, LICENSE_FILE))
      .equals(fs.readFileSync(path.join(vendor, 'LICENSE')))
  ) {
    throw new Error("Biorouter Copilot payload licence is not upstream's Apache License text");
  }
  const data = fs.readFileSync(path.join(directory, executable(target)));
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
  if (target.startsWith('darwin-')) {
    checkBundle(directory);
  } else if (manifest.bundle_identifier !== undefined) {
    throw new Error('Only a macOS Biorouter Copilot payload carries a bundle identifier');
  }
  if (process.platform !== 'win32' && !target.startsWith('win32-')) {
    fs.accessSync(path.join(directory, executable(target)), fs.constants.X_OK);
  }
  return manifest;
}

function buildCommand(target) {
  return target.startsWith('darwin-')
    ? `python3 scripts/computer-use-runtime.py build ${target}   (macOS, Xcode command line tools, Rust 1.92)`
    : `bash scripts/build-computer-use-helper-cross.sh ${target}   (Docker; the pinned rust:1.92-bullseye image)`;
}

function stageComputerUse(platform, arch) {
  const target = `${platform}-${arch}`;
  const source = path.join(root, 'target/computer-use', target);
  const destination = path.join(root, 'ui/desktop/src/computer-use');
  // ⚠ Say what is missing and how to produce it. This is the FIRST statement of
  // `preparePlatformBinaries`, so it is the first thing every packaging command
  // reaches — `just make-ui`, `make-ui-windows`, `make-ui-intel`,
  // `make-ui-linux`, and `npm run bundle:*` directly. Nothing in the Justfile
  // builds this directory: only `scripts/release.sh`, the CI workflows and
  // `scripts/computer-use-package-acceptance.py` build it. Without this check the
  // developer gets a bare
  //     ENOENT: no such file or directory, open '.../manifest.json'
  // from inside `verifyComputerUse`, which names neither the payload nor the
  // command that creates it.
  if (!fs.existsSync(path.join(source, 'manifest.json'))) {
    throw new Error(
      `Biorouter Copilot helper payload missing for ${target}.\n` +
        `  expected: ${source}\n` +
        `  build it: ${buildCommand(target)}\n` +
        `  Packaging bundles this helper, so it cannot be skipped.`
    );
  }
  verifyComputerUse(source, target);
  fs.rmSync(destination, { recursive: true, force: true });
  fs.cpSync(source, destination, { recursive: true });
  verifyComputerUse(destination, target);
}

module.exports = { stageComputerUse, verifyComputerUse, COPILOT_BUNDLE };
