const fs = require('fs');
const path = require('path');
const { spawnSync } = require('child_process');
const { fetchLlamaServer } = require('./fetch-llama-server');
const { stageComputerUse } = require('./computer-use-resources');
const { npmCommand } = require('./npm-command');

// Paths
const appRoot = path.join(__dirname, '..');
const srcBinDir = path.join(__dirname, '..', 'src', 'bin');
const srcWebDir = path.join(__dirname, '..', 'src', 'web');
const platformWinDir = path.join(__dirname, '..', 'src', 'platform', 'windows', 'bin');

// Platform-specific file patterns
const windowsFiles = ['*.exe', '*.dll', '*.cmd', 'biorouter-npm/**/*', 'git/**/*'];

const macosFiles = ['biorouterd', 'biorouter', 'jbang', 'npx', 'uvx', '*.db', '*.log', '.gitkeep'];

// Helper function to check if file matches patterns
function matchesPattern(filename, patterns) {
  return patterns.some((pattern) => {
    if (pattern.includes('**')) {
      // Handle directory patterns
      const basePattern = pattern.split('/**')[0];
      return filename.startsWith(basePattern);
    } else if (pattern.includes('*')) {
      // Handle wildcard patterns - be more precise with file extensions
      if (pattern.startsWith('*.')) {
        // For file extension patterns like *.exe, *.dll
        const extension = pattern.substring(2); // Remove "*."
        return filename.endsWith('.' + extension);
      } else {
        // For other wildcard patterns
        const regex = new RegExp('^' + pattern.replace(/\*/g, '.*') + '$');
        return regex.test(filename);
      }
    } else {
      // Exact match
      return filename === pattern;
    }
  });
}

// Helper function to clean directory of cross-platform files
function cleanBinDirectory(targetPlatform) {
  console.log(`Cleaning bin directory for ${targetPlatform} build...`);

  if (!fs.existsSync(srcBinDir)) {
    console.log('src/bin directory does not exist, skipping cleanup');
    return;
  }

  const files = fs.readdirSync(srcBinDir, { withFileTypes: true });

  files.forEach((file) => {
    const filePath = path.join(srcBinDir, file.name);

    if (targetPlatform === 'darwin' || targetPlatform === 'linux') {
      // For macOS/Linux, remove Windows-specific files
      if (matchesPattern(file.name, windowsFiles)) {
        console.log(`Removing Windows file: ${file.name}`);
        if (file.isDirectory()) {
          fs.rmSync(filePath, { recursive: true, force: true });
        } else {
          fs.unlinkSync(filePath);
        }
      }
    } else if (targetPlatform === 'win32') {
      // For Windows, remove macOS-specific files (keep only Windows files and common files)
      if (
        !matchesPattern(file.name, windowsFiles) &&
        !matchesPattern(file.name, ['*.db', '*.log', '.gitkeep'])
      ) {
        // Check if it's a macOS binary (executable without extension)
        if (file.isFile() && !path.extname(file.name) && file.name !== '.gitkeep') {
          try {
            // Check if file is executable (likely a macOS binary)
            const stats = fs.statSync(filePath);
            if (stats.mode & parseInt('111', 8)) {
              // Check if any execute bit is set
              console.log(`Removing macOS binary: ${file.name}`);
              fs.unlinkSync(filePath);
            }
          } catch (err) {
            console.warn(`Could not check file ${file.name}:`, err.message);
          }
        }
      }
    }
  });
}

// Helper function to copy platform-specific files
function copyPlatformFiles(targetPlatform) {
  if (targetPlatform === 'win32') {
    console.log('Copying Windows-specific files...');

    if (!fs.existsSync(platformWinDir)) {
      console.warn('Windows platform directory does not exist');
      return;
    }

    // Ensure src/bin exists
    if (!fs.existsSync(srcBinDir)) {
      fs.mkdirSync(srcBinDir, { recursive: true });
    }

    // Copy Windows-specific files
    const files = fs.readdirSync(platformWinDir, { withFileTypes: true });
    files.forEach((file) => {
      if (file.name === 'README.md' || file.name === '.gitignore') {
        return;
      }

      const srcPath = path.join(platformWinDir, file.name);
      const destPath = path.join(srcBinDir, file.name);

      if (file.isDirectory()) {
        fs.cpSync(srcPath, destPath, { recursive: true, force: true });
        console.log(`Copied directory: ${file.name}`);
      } else {
        fs.copyFileSync(srcPath, destPath);
        console.log(`Copied: ${file.name}`);
      }
    });
  }
}

// Build the browser interface bundle (src/web) that `biorouterd` serves when
// pointed at it with BIOROUTER_SERVE_UI. forge ships it as an extraResource
// beside src/bin, so the packaged daemon finds it at `<exe dir>/../web`.
//
// This runs here rather than in each package.json bundle:* script because this
// file is the one step every GUI packaging path already shares — bundle:default,
// bundle:intel, bundle:dmg, bundle:intel-dmg and build-windows.js all invoke it.
// A bundle that is merely stale is as broken as one that is missing (it would
// ship a renderer from whenever someone last ran the build by hand), so this
// always rebuilds rather than skipping when the directory exists.
function buildWebBundle() {
  console.log('Building browser interface bundle (npm run build:web)...');

  // Prefer the npm that invoked us so the build runs under the same Node
  // (packaging requires Node 24 — a newer Node makes electron-forge no-op).
  // Windows .cmd launchers cannot be spawned directly; resolve their JS CLI.
  const [command, args] = npmCommand(['run', 'build:web']);

  const result = spawnSync(command, args, { cwd: appRoot, stdio: 'inherit' });

  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    console.error('\n❌ PACKAGING ERROR: `npm run build:web` failed.');
    console.error('   The browser interface bundle could not be built, so the');
    console.error('   package would ship without it. Fix the build and retry.');
    console.error('');
    process.exit(result.status ?? 1);
  }
}

// Validate that required platform binaries are present before packaging
function listFilesRecursive(dir) {
  if (!fs.existsSync(dir)) return [];
  const out = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) out.push(...listFilesRecursive(full));
    else out.push(full);
  }
  return out;
}

/// Fail if a package carries another platform's executables.
///
/// `validateRequiredBinaries` asserts the RIGHT files are present. Nothing
/// asserted the WRONG ones are absent, and the two are not the same check: the
/// Linux .deb shipped 31 Windows files under `bin/llamacpp/` — `llama-server.exe`
/// among them — while every "is it there" assertion passed.
///
/// It has to recurse. The Linux packaging script removed `*.exe` with a
/// top-level glob, which cannot match `bin/llamacpp/llama-server.exe`, and
/// `cleanBinDirectory` walks one level for the same reason.
function assertNoForeignBinaries(targetPlatform) {
  const foreign = {
    darwin: [/\.exe$/i, /\.dll$/i, /\.cmd$/i],
    linux: [/\.exe$/i, /\.dll$/i, /\.cmd$/i],
    win32: [],
  }[targetPlatform];
  if (!foreign || foreign.length === 0) return;

  const offenders = listFilesRecursive(srcBinDir)
    .filter((f) => foreign.some((re) => re.test(f)))
    .map((f) => path.relative(srcBinDir, f));

  if (offenders.length > 0) {
    const label = targetPlatform === 'darwin' ? 'macOS' : 'Linux';
    console.error(
      `\n❌ PACKAGING ERROR: ${offenders.length} foreign executable(s) in the ${label} bundle:`
    );
    for (const o of offenders.slice(0, 40)) console.error(`   - ${o}`);
    if (offenders.length > 40) console.error(`   ... and ${offenders.length - 40} more`);
    console.error('\nThese belong to another platform and must not ship. If they are');
    console.error("under llamacpp/, the wrong platform's sidecar was fetched.");
    process.exit(1);
  }
}

function validateRequiredBinaries(targetPlatform) {
  // Both the server (biorouterd) AND the CLI (biorouter) must ship, so the app
  // can offer "install the Biorouter CLI" from its bundled binary.
  const required = {
    win32: ['biorouterd.exe', 'biorouter.exe', 'llamacpp/llama-server.exe'],
    darwin: ['biorouterd', 'biorouter', 'llamacpp/llama-server'],
    linux: ['biorouterd', 'biorouter', 'llamacpp/llama-server'],
  };

  const requiredForPlatform = required[targetPlatform];
  if (!requiredForPlatform) return;

  const platformLabel =
    targetPlatform === 'win32' ? 'Windows' : targetPlatform === 'darwin' ? 'macOS' : 'Linux';

  const missing = requiredForPlatform.filter((name) => {
    const fullPath = path.join(srcBinDir, name);
    return !fs.existsSync(fullPath);
  });

  if (missing.length > 0) {
    console.error(`\n❌ PACKAGING ERROR: Missing required ${platformLabel} binary/binaries:`);
    missing.forEach((name) => console.error(`   - ${path.join(srcBinDir, name)}`));
    console.error('\nBuild the backend first:');
    if (targetPlatform === 'win32') {
      console.error('  just release-windows   (cross-compile from macOS/Linux via Docker)');
      console.error('  just win-bld-rls       (native Windows build)');
    } else {
      console.error('  cargo build --release  (then: just copy-binary)');
    }
    console.error('');
    process.exit(1);
  }

  // The browser interface bundle ships beside the binaries (extraResource
  // 'src/web' in forge.config.ts), and `biorouterd` serves it from
  // `<exe dir>/../web`. Missing, it fails far more quietly than a missing
  // binary — the app itself still runs, and only `biorouter serve` is dead — so
  // it gets the same fail-fast treatment. An empty or partial directory counts
  // as missing: an index.html with no assets/ beside it serves a blank page.
  const webIndex = path.join(srcWebDir, 'index.html');
  const webAssets = path.join(srcWebDir, 'assets');
  const webBundleIsUsable =
    fs.existsSync(webIndex) &&
    fs.statSync(webIndex).size > 0 &&
    fs.existsSync(webAssets) &&
    fs.readdirSync(webAssets).length > 0;

  if (!webBundleIsUsable) {
    console.error(
      `\n❌ PACKAGING ERROR: Missing or empty ${platformLabel} browser interface bundle:`
    );
    console.error(`   - ${srcWebDir}`);
    console.error('\nBuild the browser bundle first:');
    console.error('  just build-web         (or: cd ui/desktop && npm run build:web)');
    console.error('');
    process.exit(1);
  }
}

// Main function
function preparePlatformBinaries() {
  const targetPlatform = process.env.ELECTRON_PLATFORM || process.platform;
  const targetArch = process.env.ELECTRON_ARCH || process.arch;

  console.log(`Preparing binaries for platform: ${targetPlatform} (${targetArch})`);

  stageComputerUse(targetPlatform, targetArch);

  // First copy platform-specific files if needed
  copyPlatformFiles(targetPlatform);

  // Then clean up cross-platform files
  cleanBinDirectory(targetPlatform);

  // Bundle the pinned llama-server sidecar for the Llama Server provider
  fetchLlamaServer(targetPlatform, targetArch);

  // Build the browser interface bundle biorouterd serves (BIOROUTER_SERVE_UI)
  buildWebBundle();

  // Fail fast if the backend binary or the web bundle is absent — prevents
  // silent broken packages
  validateRequiredBinaries(targetPlatform);

  // ...and that no other platform's executables came along for the ride.
  assertNoForeignBinaries(targetPlatform);

  console.log('Platform binary preparation complete');
}

// Run if called directly
if (require.main === module) {
  preparePlatformBinaries();
}

module.exports = { preparePlatformBinaries };
