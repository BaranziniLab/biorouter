const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');
const {
  CREW_BROKER_BINARY,
  crewBrokerRequired,
  inspectCrewBroker,
  isCrewBrokerPath,
} = require('./crew-broker-resources');
const { cleanBinDirectory, foreignBinaries } = require('./prepare-platform-binaries');

/// The first 64 bytes of an x86-64 PIE: what the checks read, nothing more.
function elf({ machine = 0x3e } = {}) {
  const bytes = Buffer.alloc(64);
  Buffer.from([0x7f, 0x45, 0x4c, 0x46, 2, 1, 1]).copy(bytes);
  bytes.writeUInt16LE(3, 16);
  bytes.writeUInt16LE(machine, 18);
  return bytes;
}

function manifestFor(bytes, overrides = {}) {
  return {
    schema: 1,
    name: 'biorouter-crew',
    version: '1.93.0',
    target: 'x86_64-unknown-linux-gnu',
    glibc_max: '2.30',
    sha256: crypto.createHash('sha256').update(bytes).digest('hex'),
    size: bytes.length,
    protocol: 1,
    capabilities: [],
    source_sha: 'a'.repeat(40),
    source_dirty: false,
    ...overrides,
  };
}

const created = [];
test.after(() => created.forEach((dir) => fs.rmSync(dir, { recursive: true, force: true })));

/// A `bin` directory as packaging sees it, with a staged broker unless `broker` is null.
function binDir({ broker = elf(), manifest, extra = {} } = {}) {
  const bin = fs.mkdtempSync(path.join(os.tmpdir(), 'crew broker bin '));
  created.push(bin);
  const dir = path.join(bin, 'crew-broker', 'linux-x86_64');
  fs.mkdirSync(dir, { recursive: true });
  if (broker) fs.writeFileSync(path.join(dir, CREW_BROKER_BINARY), broker, { mode: 0o755 });
  const written = manifest === undefined && broker ? manifestFor(broker) : manifest;
  if (written) fs.writeFileSync(path.join(dir, 'manifest.json'), JSON.stringify(written));
  for (const [name, body] of Object.entries(extra)) {
    fs.mkdirSync(path.dirname(path.join(bin, name)), { recursive: true });
    fs.writeFileSync(path.join(bin, name), body, { mode: 0o755 });
  }
  return bin;
}

test('a release needs the broker; a dev build only says it is missing', () => {
  const bin = binDir({ broker: null, manifest: null });
  const dev = inspectCrewBroker(bin, { appVersion: '1.93.0' });
  assert.equal(dev.present, false);
  assert.deepEqual(dev.problems, []);
  assert.match(dev.warnings[0], /no Crew broker at .*just stage-crew-broker/);
  const release = inspectCrewBroker(bin, { required: true, appVersion: '1.93.0' });
  assert.match(release.problems[0], /no Crew broker at/);
});

test('a staged pair that agrees passes, and its manifest is returned', () => {
  const result = inspectCrewBroker(binDir(), { required: true, appVersion: '1.93.0' });
  assert.deepEqual(result.problems, []);
  assert.deepEqual(result.warnings, []);
  assert.equal(result.present, true);
  assert.equal(result.manifest.glibc_max, '2.30');
});

test('a broker that differs from its manifest fails even in a dev build', () => {
  const stale = manifestFor(elf(), { sha256: '0'.repeat(64) });
  const result = inspectCrewBroker(binDir({ manifest: stale }), { appVersion: '1.93.0' });
  assert.equal(result.problems.length, 1);
  assert.match(result.problems[0], /is not the broker its manifest describes/);
  const resized = manifestFor(elf(), { size: 63 });
  assert.match(inspectCrewBroker(binDir({ manifest: resized }), {}).problems[0], /63 bytes/);
});

test('only a Linux x86-64 ELF, for the named target, is a broker', () => {
  const arm = elf({ machine: 0xb7 });
  assert.match(
    inspectCrewBroker(binDir({ broker: arm }), {}).problems.join('\n'),
    /is not a Linux x86-64 ELF executable/
  );
  const macho = Buffer.alloc(64, 0xcf);
  assert.match(
    inspectCrewBroker(binDir({ broker: macho }), {}).problems.join('\n'),
    /is not a Linux x86-64 ELF executable/
  );
  const wrongTarget = manifestFor(elf(), { target: 'aarch64-unknown-linux-gnu' });
  assert.match(
    inspectCrewBroker(binDir({ manifest: wrongTarget }), {}).problems.join('\n'),
    /is for "aarch64-unknown-linux-gnu"/
  );
  const wrongSchema = manifestFor(elf(), { schema: 2 });
  assert.match(
    inspectCrewBroker(binDir({ manifest: wrongSchema }), {}).problems.join('\n'),
    /schema 2, not 1/
  );
});

test('a broker of another version is a warning in dev and a refusal in a release', () => {
  const older = manifestFor(elf(), { version: '1.92.1' });
  const bin = binDir({ manifest: older });
  const dev = inspectCrewBroker(bin, { appVersion: '1.93.0' });
  assert.deepEqual(dev.problems, []);
  assert.match(dev.warnings[0], /the Crew broker is 1\.92\.1 but this package is 1\.93\.0/);
  const release = inspectCrewBroker(bin, { required: true, appVersion: '1.93.0' });
  assert.match(release.problems[0], /1\.92\.1 but this package is 1\.93\.0/);
});

test('half a pair, a stray file, a symlink or an unreadable manifest is refused', () => {
  assert.match(
    inspectCrewBroker(binDir({ manifest: null }), {}).problems[0],
    /a broker with no manifest\.json/
  );
  const strayed = binDir({ extra: { 'crew-broker/linux-x86_64/.staging-left-over': 'x' } });
  assert.match(inspectCrewBroker(strayed, {}).problems[0], /holds \.staging-left-over/);
  const linked = binDir();
  const dir = path.join(linked, 'crew-broker', 'linux-x86_64');
  const target = path.join(linked, 'elsewhere');
  fs.renameSync(path.join(dir, CREW_BROKER_BINARY), target);
  fs.symlinkSync(target, path.join(dir, CREW_BROKER_BINARY));
  assert.match(inspectCrewBroker(linked, {}).problems[0], /is not a regular file/);
  const garbled = binDir();
  fs.writeFileSync(path.join(garbled, 'crew-broker', 'linux-x86_64', 'manifest.json'), '{');
  assert.match(inspectCrewBroker(garbled, {}).problems[0], /is not valid JSON/);
});

test('--release or BIOROUTER_REQUIRE_CREW_BROKER=1 makes the broker required', () => {
  assert.equal(crewBrokerRequired(['node', 'prepare'], {}), false);
  assert.equal(crewBrokerRequired(['node', 'prepare', '--release'], {}), true);
  assert.equal(
    crewBrokerRequired(['node', 'prepare'], { BIOROUTER_REQUIRE_CREW_BROKER: '1' }),
    true
  );
  assert.equal(
    crewBrokerRequired(['node', 'prepare'], { BIOROUTER_REQUIRE_CREW_BROKER: '0' }),
    false
  );
});

test('the foreign-binary sweep never reads the broker as another platform', () => {
  assert.equal(isCrewBrokerPath(path.join('crew-broker', 'linux-x86_64', 'biorouter-crew')), true);
  assert.equal(isCrewBrokerPath('crew-broker\\linux-x86_64\\manifest.json'), true);
  assert.equal(isCrewBrokerPath(path.join('llamacpp', 'llama-server')), false);
  const bin = binDir({
    extra: {
      'llamacpp/llama-server.exe': 'MZ',
      'crew-broker/linux-x86_64/biorouter-crew.exe': 'MZ',
    },
  });
  assert.deepEqual(foreignBinaries('darwin', bin), [path.join('llamacpp', 'llama-server.exe')]);
  assert.deepEqual(foreignBinaries('linux', bin), [path.join('llamacpp', 'llama-server.exe')]);
  assert.deepEqual(foreignBinaries('win32', bin), []);
});

test('cleaning src/bin for any platform keeps the staged broker', () => {
  const quietly = (fn) => {
    const log = console.log;
    console.log = () => {};
    try {
      fn();
    } finally {
      console.log = log;
    }
  };
  for (const platform of ['darwin', 'linux', 'win32']) {
    const bin = binDir({ extra: { biorouterd: 'mac binary', 'helper.exe': 'MZ' } });
    quietly(() => cleanBinDirectory(platform, bin));
    const staged = path.join(bin, 'crew-broker', 'linux-x86_64');
    assert.deepEqual(fs.readdirSync(staged).sort(), ['biorouter-crew', 'manifest.json'], platform);
    assert.deepEqual(inspectCrewBroker(bin, { required: true }).problems, [], platform);
    // The sweep still does its own job around the broker.
    const left = fs.readdirSync(bin).sort();
    if (platform === 'win32') assert.deepEqual(left, ['crew-broker', 'helper.exe']);
    else assert.deepEqual(left, ['biorouterd', 'crew-broker']);
  }
});
