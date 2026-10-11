const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');
const zlib = require('node:zlib');
const {
  ESBUILD_VERSION,
  INTEGRITY,
  fetchEsbuild,
  matchesIntegrity,
  readTarMember,
  targetFor,
} = require('./fetch-esbuild');

/// A minimal ustar archive, gzipped, holding `files` ({ name: Buffer }). A name
/// written `x:<name>` is a pax extended header (type flag `x`).
function tgz(files) {
  const blocks = [];
  for (const [entry, body] of Object.entries(files)) {
    const [type, name] = entry.startsWith('x:') ? ['x', entry.slice(2)] : ['0', entry];
    const header = Buffer.alloc(512);
    header.write(name, 0, 'utf8');
    header.write('0000755\0', 100);
    header.write(body.length.toString(8).padStart(11, '0') + '\0', 124);
    header.write(type, 156);
    header.write('ustar\0', 257);
    header.write('        ', 148);
    let sum = 0;
    for (const b of header) sum += b;
    header.write(sum.toString(8).padStart(6, '0') + '\0 ', 148);
    blocks.push(header, body, Buffer.alloc((512 - (body.length % 512)) % 512));
  }
  blocks.push(Buffer.alloc(1024));
  return zlib.gzipSync(Buffer.concat(blocks));
}

const integrityOf = (bytes) =>
  `sha512-${crypto.createHash('sha512').update(bytes).digest('base64')}`;

test('every packaging target maps to a pinned package and the binary inside it', () => {
  assert.equal(targetFor('darwin', 'arm64').pkg, '@esbuild/darwin-arm64');
  assert.equal(targetFor('darwin', 'x64').pkg, '@esbuild/darwin-x64');
  assert.equal(targetFor('linux', 'x64').pkg, '@esbuild/linux-x64');
  assert.equal(targetFor('linux', 'arm64').pkg, '@esbuild/linux-arm64');
  const win = targetFor('win32', 'x64');
  assert.equal(win.pkg, '@esbuild/win32-x64');
  assert.equal(win.member, 'package/esbuild.exe');
  assert.equal(win.staged, 'esbuild.exe');
  assert.equal(targetFor('darwin', 'arm64').member, 'package/bin/esbuild');
  assert.equal(targetFor('darwin', 'arm64').staged, 'esbuild');
  assert.match(
    targetFor('linux', 'x64').tarball,
    new RegExp(
      `^https://registry\\.npmjs\\.org/@esbuild/linux-x64/-/linux-x64-${ESBUILD_VERSION}\\.tgz$`
    )
  );
  assert.throws(() => targetFor('freebsd', 'x64'), /No esbuild package mapping/);
  for (const value of Object.values(INTEGRITY)) assert.match(value, /^sha512-[A-Za-z0-9+/]+=*$/);
});

test('the pin is the esbuild ui/desktop already builds with, at the same digests', () => {
  const lock = JSON.parse(fs.readFileSync(path.join(__dirname, '..', 'package-lock.json'), 'utf8'));
  for (const [key, integrity] of Object.entries(INTEGRITY)) {
    const entry = lock.packages[`node_modules/@esbuild/${key}`];
    assert.ok(entry, `package-lock.json has no @esbuild/${key}`);
    assert.equal(entry.version, ESBUILD_VERSION, `@esbuild/${key}`);
    assert.equal(entry.integrity, integrity, `@esbuild/${key}`);
  }
});

test('the binary is read out of the npm tarball, pax path records included', () => {
  const archive = tgz({
    'package/package.json': Buffer.from('{}'),
    'package/bin/esbuild': Buffer.from('#!binary'),
  });
  assert.deepEqual(readTarMember(archive, 'package/bin/esbuild'), Buffer.from('#!binary'));
  assert.equal(readTarMember(archive, 'package/esbuild.exe'), null);

  // A name longer than the 100-byte header field arrives in a pax record.
  const long = `package/${'x'.repeat(120)}`;
  const pax = tgz({
    'x:PaxHeader/long': Buffer.from(`137 path=${long}\n`),
    'package/truncated-name': Buffer.from('long body'),
  });
  assert.deepEqual(readTarMember(pax, long), Buffer.from('long body'));
  assert.equal(readTarMember(pax, 'package/truncated-name'), null);
});

test('a tarball that does not match its pin is refused and nothing is staged', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'fetch-esbuild-'));
  try {
    const binDir = path.join(root, 'bin');
    const cacheDir = path.join(root, 'cache');
    let downloads = 0;
    const forged = tgz({ 'package/bin/esbuild': Buffer.from('not esbuild') });
    assert.equal(matchesIntegrity(forged, INTEGRITY['darwin-arm64']), false);
    assert.throws(
      () =>
        fetchEsbuild('darwin', 'arm64', {
          binDir,
          cacheDir,
          download: (_url, dest) => {
            downloads++;
            fs.writeFileSync(dest, forged);
          },
        }),
      /does not match the pinned integrity/
    );
    assert.equal(downloads, 2, 'a mismatching file is fetched again once, then refused');
    assert.equal(fs.existsSync(path.join(binDir, 'esbuild')), false);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test('a verified tarball is staged executable, replacing the other platform name', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'fetch-esbuild-'));
  try {
    const binDir = path.join(root, 'bin');
    fs.mkdirSync(binDir);
    fs.writeFileSync(path.join(binDir, 'esbuild'), 'left by a mac bundle');
    const archive = tgz({ 'package/esbuild.exe': Buffer.from('MZ windows esbuild') });
    // Stand a digest of the synthetic archive in for the real pin.
    const saved = INTEGRITY['win32-x64'];
    INTEGRITY['win32-x64'] = integrityOf(archive);
    try {
      const dest = fetchEsbuild('win32', 'x64', {
        binDir,
        cacheDir: path.join(root, 'cache'),
        download: (_url, file) => fs.writeFileSync(file, archive),
      });
      assert.equal(dest, path.join(binDir, 'esbuild.exe'));
      assert.equal(fs.readFileSync(dest, 'utf8'), 'MZ windows esbuild');
      assert.equal(fs.existsSync(path.join(binDir, 'esbuild')), false);
      if (process.platform !== 'win32') {
        assert.equal(fs.statSync(dest).mode & 0o111, 0o111);
      }
    } finally {
      INTEGRITY['win32-x64'] = saved;
    }
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});
