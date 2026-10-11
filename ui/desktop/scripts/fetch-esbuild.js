// Stages the pinned esbuild binary for the target platform as
// src/bin/esbuild (src/bin/esbuild.exe on Windows), so it ships inside the app
// bundle beside biorouterd. That is where the daemon looks first
// (`find_esbuild` in crates/biorouter-mcp/src/agent_drafter/bundle.rs).
//
// Why ship it at all: a daemon the desktop app starts has launchd's PATH on
// macOS (`/usr/bin:/bin:/usr/sbin:/sbin` plus the app's own bin/) and $HOME as
// its working directory, so it found no esbuild and no npx. Every app it built
// went through the vendored type-stripper, and three of one user's seven apps
// shipped bundles that did not even parse.
//
// The binary comes from the official per-platform npm package
// (`@esbuild/<platform>-<arch>`), the same artifact `npm install esbuild`
// picks, and is checked against the sha512 the registry publishes for that
// exact version (pinned below, copied from ui/desktop/package-lock.json, which
// records the same digests). A tarball that does not match is refused, never
// staged.
//
// ⚠ ESBUILD_VERSION is pinned in two places that never import each other:
// here, and `ESBUILD_VERSION` in bundle.rs, which pins the `npx` last resort to
// the same release. `the_npx_pin_matches_the_shipped_binary` (bundle.rs) holds
// them together. Keep it equal to the esbuild in ui/desktop/node_modules too,
// so a dev checkout (which builds with that one) and the packaged app build
// apps with the same compiler.

const crypto = require('crypto');
const fs = require('fs');
const os = require('os');
const path = require('path');
const zlib = require('zlib');
const { execFileSync } = require('child_process');

const ESBUILD_VERSION = '0.27.3';

// The registry's `dist.integrity` for each package we can ship, at
// ESBUILD_VERSION. To bump: change the version, then copy the new digests from
// package-lock.json after `npm install esbuild@<version>` (or `npm view
// @esbuild/<pkg>@<version> dist.integrity`).
const INTEGRITY = {
  'darwin-arm64':
    'sha512-Re491k7ByTVRy0t3EKWajdLIr0gz2kKKfzafkth4Q8A5n1xTHrkqZgLLjFEHVD+AXdUGgQMq+Godfq45mGpCKg==',
  'darwin-x64':
    'sha512-vHk/hA7/1AckjGzRqi6wbo+jaShzRowYip6rt6q7VYEDX4LEy1pZfDpdxCBnGtl+A5zq8iXDcyuxwtv3hNtHFg==',
  'linux-x64':
    'sha512-Czi8yzXUWIQYAtL/2y6vogER8pvcsOsk5cpwL4Gk5nJqH5UZiVByIY8Eorm5R13gq+DQKYg0+JyQoytLQas4dA==',
  'linux-arm64':
    'sha512-sZOuFz/xWnZ4KH3YfFrKCf1WyPZHakVzTiqji3WDc0BCl2kBwiJLCXpzLzUBLgmp4veFZdvN5ChW4Eq/8Fc2Fg==',
  'win32-x64':
    'sha512-4uJGhsxuptu3OcpVAzli+/gWusVGwZZHTlS63hh++ehExkVT8SgiEf7/uC/PclrPPkLhZqGgCTjd0VWLo6xMqA==',
  'win32-arm64':
    'sha512-B2t59lWWYrbRDw/tjiWOuzSsFh1Y/E95ofKz7rIVYSQkUYBjfSgf6oeYPNWHToFRr2zx52JKApIcAS/D5TUBnA==',
};

const defaultBinDir = path.join(__dirname, '..', 'src', 'bin');

/// The npm package, the file inside its tarball, and the name it is staged
/// under, for a packaging target. Mirrors fetch-llama-server.js's choices:
/// macOS and Linux by arch, Windows x64 unless arm64 is asked for.
function targetFor(platform, arch) {
  const key =
    platform === 'darwin'
      ? `darwin-${arch === 'x64' ? 'x64' : 'arm64'}`
      : platform === 'linux'
        ? `linux-${arch === 'arm64' ? 'arm64' : 'x64'}`
        : platform === 'win32'
          ? `win32-${arch === 'arm64' ? 'arm64' : 'x64'}`
          : null;
  if (!key || !INTEGRITY[key]) {
    throw new Error(`No esbuild package mapping for platform ${platform} (${arch})`);
  }
  const windows = platform === 'win32';
  return {
    key,
    pkg: `@esbuild/${key}`,
    tarball: `https://registry.npmjs.org/@esbuild/${key}/-/${key}-${ESBUILD_VERSION}.tgz`,
    // The Windows package keeps the binary at its root; the others under bin/.
    member: windows ? 'package/esbuild.exe' : 'package/bin/esbuild',
    staged: windows ? 'esbuild.exe' : 'esbuild',
    integrity: INTEGRITY[key],
  };
}

/// Whether `bytes` are exactly what an npm `integrity` string ("sha512-<b64>")
/// names.
function matchesIntegrity(bytes, integrity) {
  const [algorithm, expected] = integrity.split(/-(.*)/s);
  if (algorithm !== 'sha512' || !expected) {
    throw new Error(`unsupported integrity string ${integrity}`);
  }
  const actual = crypto.createHash('sha512').update(bytes).digest('base64');
  return actual === expected;
}

/// One regular file out of a gzipped tar (an npm tarball), or null when it is
/// not there. Handles the ustar `prefix` field and pax `path` records, which is
/// all npm writes; nothing is extracted to disk except what the caller asks for.
function readTarMember(tgz, wanted) {
  const tar = zlib.gunzipSync(tgz);
  let offset = 0;
  let paxPath = null;
  const field = (block, start, length) => {
    const raw = block.subarray(start, start + length);
    const end = raw.indexOf(0);
    return raw.subarray(0, end === -1 ? raw.length : end).toString('utf8');
  };
  while (offset + 512 <= tar.length) {
    const header = tar.subarray(offset, offset + 512);
    if (header.every((b) => b === 0)) break;
    const name = field(header, 0, 100);
    const size = parseInt(field(header, 124, 12).trim() || '0', 8);
    const type = String.fromCharCode(header[156] || 0x30);
    const prefix = field(header, 345, 155);
    const body = tar.subarray(offset + 512, offset + 512 + size);
    offset += 512 + Math.ceil(size / 512) * 512;
    if (type === 'x') {
      // A pax extended header: `<len> path=<name>\n` overrides the next entry.
      const match = /(?:^|\n)\d+ path=([^\n]*)\n/.exec(body.toString('utf8'));
      paxPath = match ? match[1] : null;
      continue;
    }
    const full = paxPath ?? (prefix ? `${prefix}/${name}` : name);
    paxPath = null;
    if ((type === '0' || type === '\0') && full === wanted) {
      return Buffer.from(body);
    }
  }
  return null;
}

function download(url, dest) {
  // curl, as fetch-llama-server.js uses: present on every build host (macOS,
  // the Linux/Windows docker images, CI runners) and it honours proxies.
  console.log(`Downloading ${url}`);
  execFileSync('curl', ['-fSL', '--retry', '3', '-o', dest, url], { stdio: 'inherit' });
}

/// The verified tarball for `target`, from the build cache or the registry.
/// A cached file that no longer matches its pin is fetched again, once.
function verifiedTarball(target, cacheDir, fetch = download) {
  fs.mkdirSync(cacheDir, { recursive: true });
  const cached = path.join(cacheDir, `${target.key}-${ESBUILD_VERSION}.tgz`);
  for (let attempt = 0; attempt < 2; attempt++) {
    if (!fs.existsSync(cached)) {
      const partial = `${cached}.${process.pid}.part`;
      fetch(target.tarball, partial);
      fs.renameSync(partial, cached);
    }
    const bytes = fs.readFileSync(cached);
    if (matchesIntegrity(bytes, target.integrity)) {
      return bytes;
    }
    console.warn(`${cached} does not match its pinned sha512; fetching it again`);
    fs.rmSync(cached, { force: true });
  }
  throw new Error(
    `${target.pkg}@${ESBUILD_VERSION} from ${target.tarball} does not match the pinned ` +
      `integrity ${target.integrity}. Refusing to ship it.`
  );
}

/// Stage esbuild for `platform`/`arch` into `binDir` (src/bin by default).
function fetchEsbuild(platform, arch, options = {}) {
  const binDir = options.binDir ?? defaultBinDir;
  const cacheDir =
    options.cacheDir ?? path.join(os.homedir(), '.cache', 'biorouter-build', 'esbuild');
  const target = targetFor(platform, arch);
  const tgz = verifiedTarball(target, cacheDir, options.download);
  const binary = readTarMember(tgz, target.member);
  if (!binary || binary.length === 0) {
    throw new Error(`${target.member} not found in ${target.pkg}@${ESBUILD_VERSION}`);
  }
  fs.mkdirSync(binDir, { recursive: true });
  // The OTHER platform's name is removed, so a Windows package never carries
  // a macOS `esbuild` left over from an earlier bundle, or the reverse.
  for (const stale of ['esbuild', 'esbuild.exe']) {
    if (stale !== target.staged) fs.rmSync(path.join(binDir, stale), { force: true });
  }
  const dest = path.join(binDir, target.staged);
  const partial = `${dest}.${process.pid}.part`;
  fs.writeFileSync(partial, binary);
  fs.chmodSync(partial, 0o755);
  fs.renameSync(partial, dest);
  console.log(`esbuild ${ESBUILD_VERSION} (${target.key}) -> ${dest}`);
  return dest;
}

if (require.main === module) {
  const platform = process.env.ELECTRON_PLATFORM || process.platform;
  const arch = process.env.ELECTRON_ARCH || process.arch;
  fetchEsbuild(platform, arch);
}

module.exports = {
  ESBUILD_VERSION,
  INTEGRITY,
  fetchEsbuild,
  matchesIntegrity,
  readTarMember,
  targetFor,
};
