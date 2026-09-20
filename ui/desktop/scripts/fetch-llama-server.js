// Fetches the pinned llama.cpp `llama-server` binary for the target platform
// into src/bin/llamacpp/ so it ships inside the app bundle (next to
// biorouterd, which is how the Rust sidecar manager locates it).
//
// Pinned build: keep LLAMA_BUILD in sync with LLAMA_SERVER_BUILD in
// crates/biorouter/src/providers/llamacpp_sidecar.rs.
//
// Per-platform variants (deliberate choices, see CLAUDE.md):
//   - macOS arm64/x64: Metal build (tiny, GPU out of the box on Apple Silicon)
//   - Windows x64:     Vulkan build (GPU on nearly all 2026 hardware; ggml
//                      falls back to CPU when no Vulkan loader is present).
//                      CUDA stays opt-in via BIOROUTER_LLAMACPP_BIN — its
//                      runtime alone is ~390 MB.
//   - Linux x64:       CPU build (GPU stacks vary too much to bundle one)

const fs = require('fs');
const os = require('os');
const path = require('path');
const AdmZip = require('adm-zip');
const { execFileSync } = require('child_process');

const LLAMA_BUILD = 'b9611';
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

function fetchLlamaServer(platform, arch) {
  const marker = `${LLAMA_BUILD}-${platform}-${arch}`;
  if (fs.existsSync(markerFile) && fs.readFileSync(markerFile, 'utf8').trim() === marker) {
    console.log(`llama-server ${marker} already present, skipping fetch`);
    return;
  }

  const asset = assetFor(platform, arch);
  const cacheDir = path.join(os.homedir(), '.cache', 'biorouter-build', 'llamacpp');
  fs.mkdirSync(cacheDir, { recursive: true });
  const archivePath = path.join(cacheDir, asset);
  if (!fs.existsSync(archivePath)) {
    download(`${BASE_URL}/${asset}`, archivePath);
  } else {
    console.log(`Using cached ${archivePath}`);
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
      download(LICENSE_URL, licensePath);
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

module.exports = { fetchLlamaServer, LLAMA_BUILD };
