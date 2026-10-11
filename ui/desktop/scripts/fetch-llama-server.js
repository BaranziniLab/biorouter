// Fetches the pinned llama.cpp `llama-server` binary for the target platform
// into src/bin/llamacpp/ so it ships inside the app bundle (next to
// biorouterd, which is how the Rust sidecar manager locates it).
//
// Pinned build: keep LLAMA_BUILD in sync with LLAMA_SERVER_BUILD in
// crates/biorouter/src/providers/llamacpp_sidecar.rs. b11429 (2026-10-05) is the
// first pin that carries every fix the two catalog models need: the Qwen3
// tool-call parser, `reasoning_effort` passed to the chat template, the
// Gated-DeltaNet normalization fix measured on Qwen3.8-27B, the Gemma 4
// required-tool grammar fix, the n-gram draft fix at temperature > 0, and the
// `-hf` file picker skipping `dflash-`/`eagle3-` draft files (b9611 resolved
// `ggml-org/Qwen3.8-27B-GGUF:Q4_0` to the dflash draft instead of the model).
//
// Per-platform variants (deliberate choices, see CLAUDE.md):
//   - macOS arm64:     Metal build (tiny, GPU out of the box on Apple Silicon)
//   - macOS x64:       CPU-only. Upstream builds the macos-x64 archive with
//                      -DGGML_METAL=OFF, so an Intel Mac runs every layer on the
//                      CPU and the sidecar budgets it by system RAM, with a
//                      slowness warning. (This comment used to call it a Metal
//                      build, which is what the Intel Mac suitability copy then
//                      wrongly assumed.)
//   - Windows x64:     Vulkan build (GPU on nearly all 2026 hardware; ggml
//                      falls back to CPU when no Vulkan loader is present).
//                      CUDA stays opt-in via BIOROUTER_LLAMACPP_BIN — its
//                      runtime alone is ~390 MB.
//   - Linux x64:       CPU build (GPU stacks vary too much to bundle one)
//
// Every archive is checked against a pinned SHA-256 before it is extracted, and
// again whenever the build cache supplies it: the cache under
// ~/.cache/biorouter-build is reused across builds without any other check, so a
// tampered or truncated archive there would otherwise ship inside a signed app.
// The digests are GitHub's own `digest` field for each release asset
// (`gh api repos/ggml-org/llama.cpp/releases/tags/<build>`). A pin bump must
// replace all of them; a missing digest is a refusal, not a skip.

const fs = require('fs');
const os = require('os');
const path = require('path');
const AdmZip = require('adm-zip');
const crypto = require('crypto');
const { execFileSync } = require('child_process');

const LLAMA_BUILD = 'b11429';
// SHA-256 of each release asset this script can fetch, for LLAMA_BUILD only.
const ASSET_SHA256 = {
  [`llama-${LLAMA_BUILD}-bin-macos-arm64.tar.gz`]:
    '740288ec6887be94280a5dfa25b5e23a78285cab104519e6c7e218904ee82459',
  [`llama-${LLAMA_BUILD}-bin-macos-x64.tar.gz`]:
    '29ac3ea02be6bd143e824973f2cc5fa74bc4094393a9eaab0ff6814f19dd8522',
  [`llama-${LLAMA_BUILD}-bin-win-vulkan-x64.zip`]:
    '1bfe78ad9168b79fa02bf67f6af9f5e17a966d824d77238517f7bef12ac73b36',
  [`llama-${LLAMA_BUILD}-bin-ubuntu-x64.tar.gz`]:
    'f6d25dde8f51133143d1453da4fd5f73b145127177612a283bf7995957af3392',
  [`llama-${LLAMA_BUILD}-bin-ubuntu-arm64.tar.gz`]:
    'ed44c0f79c3d02424f62bdc87e106318b0b89f49050e2e80c5f352723d14e00b',
};
// SHA-256 of LICENSE at the pinned tag (fetched only for the Windows zip).
const LICENSE_SHA256 = '94f29bbed6a22c35b992c5c6ebf0e7c92f13b836b90f36f461c9cf2f0f1d010d';
const BASE_URL = `https://github.com/ggml-org/llama.cpp/releases/download/${LLAMA_BUILD}`;
// llama.cpp is MIT, whose one obligation is that the copyright notice travels
// with every distributed copy. The macOS and Linux archives carry a LICENSE
// file; the Windows zip does not (verified against b9611: no licence-like entry
// at all), so for Windows we fetch it from the repository at the pinned tag.
// Without this the shipped Windows app redistributes llama.cpp with no notice.
const LICENSE_URL = `https://raw.githubusercontent.com/ggml-org/llama.cpp/${LLAMA_BUILD}/LICENSE`;

const destDir = path.join(__dirname, '..', 'src', 'bin', 'llamacpp');
const markerFile = path.join(destDir, '.build');

function assetFor(platform, arch) {
  if (platform === 'darwin') {
    return arch === 'x64'
      ? `llama-${LLAMA_BUILD}-bin-macos-x64.tar.gz`
      : `llama-${LLAMA_BUILD}-bin-macos-arm64.tar.gz`;
  }
  if (platform === 'win32') {
    return `llama-${LLAMA_BUILD}-bin-win-vulkan-x64.zip`;
  }
  if (platform === 'linux') {
    return arch === 'arm64'
      ? `llama-${LLAMA_BUILD}-bin-ubuntu-arm64.tar.gz`
      : `llama-${LLAMA_BUILD}-bin-ubuntu-x64.tar.gz`;
  }
  throw new Error(`No llama-server asset mapping for platform ${platform}`);
}

// Only the server binary and its shared libraries ship; the other ~15 tools
// in the archive (llama-cli, llama-bench, ...) stay out of the bundle.
function wantedFile(name, platform) {
  if (platform === 'win32') {
    return name === 'llama-server.exe' || name.endsWith('.dll');
  }
  if (platform === 'darwin') {
    return name === 'llama-server' || name.endsWith('.dylib') || name === 'LICENSE';
  }
  return name === 'llama-server' || name.includes('.so') || name === 'LICENSE';
}

function findFilesRecursive(dir) {
  const out = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      out.push(...findFilesRecursive(full));
    } else {
      out.push(full);
    }
  }
  return out;
}

function download(url, dest) {
  // curl handles GitHub's redirect chain and resume; it is present on all
  // build hosts (macOS, the Linux/Windows docker images, CI runners).
  console.log(`Downloading ${url}`);
  execFileSync('curl', ['-fSL', '--retry', '3', '-o', dest, url], { stdio: 'inherit' });
}

function sha256File(file) {
  const hash = crypto.createHash('sha256');
  const fd = fs.openSync(file, 'r');
  try {
    const buf = Buffer.alloc(1 << 20);
    let n;
    while ((n = fs.readSync(fd, buf, 0, buf.length, null)) > 0) {
      hash.update(buf.subarray(0, n));
    }
  } finally {
    fs.closeSync(fd);
  }
  return hash.digest('hex');
}

function expectedSha256(asset) {
  const expected = ASSET_SHA256[asset];
  if (!expected) {
    throw new Error(
      `No pinned SHA-256 for ${asset}. Add the release asset's digest to ASSET_SHA256 ` +
        `(gh api repos/ggml-org/llama.cpp/releases/tags/${LLAMA_BUILD}); refusing to ship an ` +
        `unverified llama-server.`
    );
  }
  return expected;
}

// True when `file` exists and hashes to `expected`.
function matchesSha256(file, expected) {
  return fs.existsSync(file) && sha256File(file) === expected;
}

// Download `url` to `dest` and refuse it unless it hashes to `expected`. A
// mismatching file is deleted so the next run cannot pick it up from the cache.
function downloadVerified(url, dest, expected) {
  const partial = `${dest}.partial`;
  fs.rmSync(partial, { force: true });
  download(url, partial);
  const actual = sha256File(partial);
  if (actual !== expected) {
    fs.rmSync(partial, { force: true });
    throw new Error(
      `SHA-256 mismatch for ${url}: expected ${expected}, got ${actual}. ` +
        `Refusing to package it.`
    );
  }
  fs.renameSync(partial, dest);
}

function fetchLlamaServer(platform, arch) {
  const marker = `${LLAMA_BUILD}-${platform}-${arch}`;
  if (fs.existsSync(markerFile) && fs.readFileSync(markerFile, 'utf8').trim() === marker) {
    console.log(`llama-server ${marker} already present, skipping fetch`);
    return;
  }

  const asset = assetFor(platform, arch);
  const expected = expectedSha256(asset);
  const cacheDir = path.join(os.homedir(), '.cache', 'biorouter-build', 'llamacpp');
  fs.mkdirSync(cacheDir, { recursive: true });
  const archivePath = path.join(cacheDir, asset);
  if (matchesSha256(archivePath, expected)) {
    console.log(`Using cached ${archivePath} (sha256 verified)`);
  } else {
    if (fs.existsSync(archivePath)) {
      console.log(`Cached ${archivePath} does not match its pinned SHA-256; downloading again`);
      fs.rmSync(archivePath, { force: true });
    }
    downloadVerified(`${BASE_URL}/${asset}`, archivePath, expected);
  }
  // Checked once more right before extraction, so nothing between the check
  // above and `tar`/AdmZip can hand over a different file.
  const actual = sha256File(archivePath);
  if (actual !== expected) {
    throw new Error(`SHA-256 mismatch for ${archivePath}: expected ${expected}, got ${actual}`);
  }

  const extractDir = fs.mkdtempSync(path.join(os.tmpdir(), 'llamacpp-extract-'));
  try {
    if (asset.endsWith('.zip')) {
      new AdmZip(archivePath).extractAllTo(extractDir, true);
    } else {
      execFileSync('tar', ['-xzf', archivePath, '-C', extractDir], { stdio: 'inherit' });
    }

    fs.rmSync(destDir, { recursive: true, force: true });
    fs.mkdirSync(destDir, { recursive: true });

    let copied = 0;
    for (const file of findFilesRecursive(extractDir)) {
      const name = path.basename(file);
      if (!wantedFile(name, platform)) {
        continue;
      }
      const dest = path.join(destDir, name);
      const stat = fs.lstatSync(file);
      if (stat.isSymbolicLink()) {
        // The unix archives ship versioned dylib/so names as symlinks
        // (libllama.dylib -> libllama.0.dylib -> ...); keep them as
        // links instead of materializing duplicate copies.
        fs.symlinkSync(path.basename(fs.readlinkSync(file)), dest);
      } else {
        fs.copyFileSync(file, dest);
        // Preserve executability (tar keeps it; zip does not).
        if (!name.includes('.') || name.endsWith('.exe')) {
          fs.chmodSync(dest, 0o755);
        }
      }
      copied++;
    }

    const serverName = platform === 'win32' ? 'llama-server.exe' : 'llama-server';
    if (!fs.existsSync(path.join(destDir, serverName))) {
      throw new Error(`${serverName} not found in ${asset} — release layout changed?`);
    }

    // The licence must ship beside the binaries on every platform. This is
    // checked rather than assumed because the gap it closes was invisible:
    // Windows shipped without a notice for as long as the bundle existed, and
    // nothing failed. If an upstream archive stops carrying LICENSE the same
    // way, this fetches it rather than going quiet.
    const licensePath = path.join(destDir, 'LICENSE');
    if (!fs.existsSync(licensePath)) {
      console.log(`${asset} carries no LICENSE; fetching it from ${LICENSE_URL}`);
      downloadVerified(LICENSE_URL, licensePath, LICENSE_SHA256);
      copied++;
    }
    if (!fs.existsSync(licensePath) || fs.statSync(licensePath).size === 0) {
      throw new Error(
        `llama.cpp LICENSE is missing from ${destDir}. It is MIT-licensed and the notice ` +
          `must ship with the binaries; refusing to package without it.`
      );
    }

    fs.writeFileSync(markerFile, marker);
    console.log(`llama-server ${marker}: ${copied} files -> ${destDir}`);
  } finally {
    fs.rmSync(extractDir, { recursive: true, force: true });
  }
}

if (require.main === module) {
  const platform = process.env.ELECTRON_PLATFORM || process.platform;
  const arch = process.env.ELECTRON_ARCH || process.arch;
  fetchLlamaServer(platform, arch);
}

module.exports = { fetchLlamaServer, LLAMA_BUILD, ASSET_SHA256, sha256File };
