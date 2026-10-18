// The Linux x86_64 Crew broker every desktop package carries (OW-9).
//
// The daemon's Crew setup service installs `biorouter-crew` into a server account from the copy
// beside `biorouterd`, so it needs no network:
//
//   <bin>/crew-broker/linux-x86_64/biorouter-crew
//   <bin>/crew-broker/linux-x86_64/manifest.json
//
// `scripts/stage-crew-broker.sh` writes the pair from the bytes it stages. This module checks it
// wherever a package is assembled (prepare-platform-binaries.js before packaging, forge's
// postPackage after), because the service refuses a broker whose bytes differ from its manifest,
// and that refusal happens on somebody's server, long after the build that caused it.
//
// It is an ELF file for a different operating system on two of the three platforms, so the
// foreign-binary sweeps leave this directory alone and the macOS signer is told to skip it
// (forge.config.ts `osxSign.ignore`); the app's own seal still covers its bytes.

const crypto = require('crypto');
const fs = require('fs');
const path = require('path');

const CREW_BROKER_DIR = 'crew-broker';
const CREW_BROKER_PLATFORM_DIR = 'linux-x86_64';
const CREW_BROKER_BINARY = 'biorouter-crew';
const CREW_BROKER_TARGET = 'x86_64-unknown-linux-gnu';
const STAGED_FILES = new Set([CREW_BROKER_BINARY, 'manifest.json']);

/** True for a build that must carry the broker: `--release`, or BIOROUTER_REQUIRE_CREW_BROKER=1. */
function crewBrokerRequired(argv = process.argv, env = process.env) {
  return argv.includes('--release') || env.BIOROUTER_REQUIRE_CREW_BROKER === '1';
}

/** True for a path, relative to `bin`, that lies inside the staged broker directory. */
function isCrewBrokerPath(relative) {
  return relative.split(/[\\/]/)[0] === CREW_BROKER_DIR;
}

function isLinuxX86_64Elf(bytes) {
  // e_ident: 7f 'E' 'L' 'F', class 2 (64-bit), data 1 (little-endian); e_machine 0x3E at 18.
  return (
    bytes.length >= 20 &&
    bytes.subarray(0, 6).equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46, 2, 1])) &&
    bytes.readUInt16LE(18) === 0x3e
  );
}

/**
 * Inspect the staged broker under `binDir` (the directory that ships as `resources/bin`).
 *
 * Returns `{ present, problems, warnings, manifest }`. A pair that does not agree with itself is
 * always a problem: it can never work, in a release or a dev build. A missing pair, or a broker
 * whose version is not `appVersion`, is a problem when `required` and a warning otherwise, so a
 * dev package still builds and says what it lacks.
 */
function inspectCrewBroker(binDir, { required = false, appVersion } = {}) {
  const dir = path.join(binDir, CREW_BROKER_DIR, CREW_BROKER_PLATFORM_DIR);
  const binary = path.join(dir, CREW_BROKER_BINARY);
  const manifestPath = path.join(dir, 'manifest.json');
  const problems = [];
  const warnings = [];
  const result = (present, manifest = null) => ({ present, problems, warnings, manifest });

  const hasBinary = fs.existsSync(binary);
  const hasManifest = fs.existsSync(manifestPath);
  if (!hasBinary && !hasManifest) {
    (required ? problems : warnings).push(
      `no Crew broker at ${dir}: the Crew setup service in this package will have nothing to ` +
        'install on a server. Stage it with `just stage-crew-broker`.'
    );
    return result(false);
  }
  if (!hasBinary || !hasManifest) {
    problems.push(
      `${dir} holds ${hasBinary ? 'a broker with no manifest.json' : 'a manifest.json with no broker'}; ` +
        'stage both with scripts/stage-crew-broker.sh.'
    );
    return result(true);
  }
  for (const entry of fs.readdirSync(dir)) {
    if (!STAGED_FILES.has(entry)) problems.push(`${dir} holds ${entry}, which no staging writes`);
  }
  for (const file of [binary, manifestPath]) {
    if (!fs.lstatSync(file).isFile()) {
      problems.push(`${file} is not a regular file`);
      return result(true);
    }
  }

  let manifest;
  try {
    manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
  } catch (error) {
    problems.push(`${manifestPath} is not valid JSON: ${error.message}`);
    return result(true);
  }
  const bytes = fs.readFileSync(binary);
  const sha256 = crypto.createHash('sha256').update(bytes).digest('hex');
  const expect = (ok, sentence) => {
    if (!ok) problems.push(sentence);
  };
  expect(
    manifest.schema === 1,
    `${manifestPath} has schema ${JSON.stringify(manifest.schema)}, not 1`
  );
  expect(
    manifest.name === CREW_BROKER_BINARY,
    `${manifestPath} names ${JSON.stringify(manifest.name)}, not ${CREW_BROKER_BINARY}`
  );
  expect(
    manifest.target === CREW_BROKER_TARGET,
    `${manifestPath} is for ${JSON.stringify(manifest.target)}, not ${CREW_BROKER_TARGET}`
  );
  expect(isLinuxX86_64Elf(bytes), `${binary} is not a Linux x86-64 ELF executable`);
  expect(
    manifest.sha256 === sha256 && manifest.size === bytes.length,
    `${binary} (sha256 ${sha256}, ${bytes.length} bytes) is not the broker its manifest ` +
      `describes (sha256 ${manifest.sha256}, ${manifest.size} bytes); stage it again.`
  );
  if (typeof manifest.version !== 'string' || !manifest.version) {
    problems.push(`${manifestPath} records no version`);
  } else if (appVersion && manifest.version !== appVersion) {
    (required ? problems : warnings).push(
      `the Crew broker is ${manifest.version} but this package is ${appVersion}; ` +
        'the setup service installs the broker of its own version. Stage it again.'
    );
  }
  return result(true, manifest);
}

module.exports = {
  CREW_BROKER_DIR,
  CREW_BROKER_PLATFORM_DIR,
  CREW_BROKER_BINARY,
  crewBrokerRequired,
  inspectCrewBroker,
  isCrewBrokerPath,
};
