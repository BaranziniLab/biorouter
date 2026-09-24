#!/usr/bin/env python3
"""Build and install genuine release-format candidates; never publish or notarize."""
import argparse
from collections import Counter
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]
DESKTOP = ROOT / 'ui/desktop'
OUTPUT = ROOT / 'target/package-acceptance'
PYTHON = sys.executable


def run(command, cwd=ROOT, env=None, timeout=5400):
    print('+', ' '.join(map(str, command)), flush=True)
    subprocess.run(list(map(str, command)), cwd=cwd, env=env, check=True, timeout=timeout)


def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            value.update(block)
    return value.hexdigest()


def npm(args, env):
    executable = Path(shutil.which('npm.cmd' if os.name == 'nt' else 'npm')).resolve()
    if os.name == 'nt':
        executable = executable.parent / 'node_modules/npm/bin/npm-cli.js'
    run(['node', executable, *args], cwd=DESKTOP, env=env)


# The verifier's own doctor budget, plus room for the MCP tool census and the
# receipt. IMPORTED, not restated: an outer cap smaller than what the inner step
# is allowed to spend kills the diagnostic before it can be written, which is
# exactly the opaque failure this harness exists to replace -- and two numbers
# maintained independently is how they drift apart.
_verifier = importlib.util.spec_from_file_location(
    'verify_installed', ROOT / 'scripts/verify-installed-computer-use.py')
_installed = importlib.util.module_from_spec(_verifier)
_verifier.loader.exec_module(_installed)
INSTALLED_CHECK_TIMEOUT = _installed.DOCTOR_TIMEOUT + 90


def installed(cli, helper, target, status, label):
    run([PYTHON, ROOT / 'scripts/verify-installed-computer-use.py', '--cli', cli,
         '--helper', helper, '--target', target, '--expect-status', status,
         '--backends', OUTPUT / 'backends.json', '--report', OUTPUT / f'{label}-installed.json'], timeout=INSTALLED_CHECK_TIMEOUT)


def dependencies(resources, target):
    for entry in resources.rglob('*'):
        if entry.is_symlink():
            entry.resolve(strict=True).relative_to(resources.resolve())
    platform, arch = target.split('-')
    run(['node', '-e', 'require("./ui/desktop/scripts/verify-packaged-dependencies.js").'
         'verifyPackagedDependencies(process.argv[1],process.argv[2],process.argv[3])', resources, platform, arch])


# Linux GUI packages are located by their helper, never by an install prefix.
# electron-installer-debian and electron-installer-redhat expose no prefix
# option at all -- `prefix: '/opt'` in forge.config.ts is inert -- and each
# derives its own base directory (electron-installer-common yields 'usr';
# electron-installer-redhat overrides it to 'BUILD/usr' for staging). Both land
# the tree at usr/lib/<name>/resources, and <name> is lowercased by the deb
# installer but case-preserved by the rpm one, so no single literal path serves
# both. Anchoring on the unique helper manifest avoids encoding either.
HELPER_MANIFEST = 'computer-use/manifest.json'


def packaged_desktop_resources(directory):
    """Return the one packaged desktop resources tree inside an extracted package."""
    manifests = list(Path(directory).rglob(HELPER_MANIFEST))
    if len(manifests) != 1:
        raise ValueError(f'Extracted package must contain exactly one helper, found {manifests}')
    resources = manifests[0].parent.parent
    if resources.name != 'resources':
        raise ValueError(f'Packaged helper is not inside a desktop resources directory: {resources}')
    return resources


def installed_linux_helper_roots(opt=Path('/opt'), lib=Path('/usr/lib'), libexec=Path('/usr/libexec')):
    """Every installed helper on a Linux host, GUI (usr/lib/<name>) or CLI (usr/libexec).

    Kept separate from linux_paths() so the directory layout stays unit-testable;
    linux_paths() only adds the container-absolute defaults. /opt is still probed
    because a relocated or hand-staged install may legitimately live there, but it
    is no longer the only place looked at, which is what broke the GUI packages.
    """
    roots = []
    for base in (lib, opt):
        roots += sorted(base.glob(f'*/resources/{HELPER_MANIFEST}'))
    fhs = libexec / 'biorouter' / HELPER_MANIFEST
    if fhs.exists():
        roots.append(fhs)
    return roots


def sign_macos_candidate(app):
    manifest = app / 'Contents/Resources/computer-use/manifest.json'
    manifest_before = digest(manifest)
    frameworks = app / 'Contents/Frameworks'
    # Electron x64 ships unsigned nested framework code; keep deep signing away
    # from the separately sealed Biorouter Copilot runtime under Resources.
    for bundle in sorted(frameworks.glob('*.framework')):
        run(['codesign', '--force', '--deep', '--sign', '-', bundle])
    for bundle in sorted(frameworks.glob('*.app')):
        run(['codesign', '--force', '--sign', '-', bundle])
    run(['codesign', '--force', '--sign', '-', app])
    run(['codesign', '--verify', '--deep', '--strict', app])
    assert digest(manifest) == manifest_before


def stage(target):
    platform, arch = target.split('-')
    env = dict(os.environ, ELECTRON_PLATFORM=platform, ELECTRON_ARCH=arch, BIOROUTER_BUILD_JOBS='2')
    # Disable the signing/notarization activation knobs read by Forge and the native builder.
    for key in ['APPLE_ID', 'APPLE_APP_SPECIFIC_PASSWORD', 'WINDOWS_CERTIFICATE_FILE',
                'WINDOW_SIGNING_ROLE', 'BIOROUTER_WINDOWS_SIGN_PASSWORD_FILE']:
        env.pop(key, None)
    run([PYTHON, 'scripts/computer-use-runtime.py', 'build', target], env=env)
    triples = {'linux-x64': 'x86_64-unknown-linux-gnu', 'win32-x64': 'x86_64-pc-windows-gnu',
               'darwin-arm64': 'aarch64-apple-darwin', 'darwin-x64': 'x86_64-apple-darwin'}
    triple = triples[target]
    if platform == 'darwin':
        run(['cargo', 'build', '--release', '--locked', '--target', triple, '-j', '2',
             '--bin', 'biorouter', '--bin', 'biorouterd', '--bin', 'biorouter-authprompt'])
        # The existing auth helper assembler expects the host ARM build in target/release.
        if arch == 'arm64':
            (ROOT / 'target/release').mkdir(exist_ok=True)
            shutil.copy2(ROOT / f'target/{triple}/release/biorouter-authprompt', ROOT / 'target/release/biorouter-authprompt')
    source = ROOT / f'target/{triple}/release'
    source_commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    if platform != 'darwin' and (source / 'package-source-commit.txt').read_text(encoding='utf-8').strip() != source_commit:
        raise ValueError('Cross-build artifact came from a different source revision')
    bin_dir = DESKTOP / 'src/bin'
    shutil.rmtree(bin_dir, ignore_errors=True)
    bin_dir.mkdir(parents=True)
    suffix = '.exe' if platform == 'win32' else ''
    names = ['biorouter' + suffix, 'biorouterd' + suffix]
    for name in names:
        shutil.copy2(source / name, bin_dir / name)
    for dll in source.glob('*.dll'):
        shutil.copy2(dll, bin_dir / dll.name)
    OUTPUT.mkdir(parents=True, exist_ok=True)
    (OUTPUT / 'backends.json').write_text(json.dumps({'source_commit': source_commit, 'triple': triple,
        'profile': 'release', 'backends': {name: digest(source / name) for name in names}}, indent=2),
        encoding='utf-8')
    npm(['ci'], env)
    if platform == 'win32':
        run(['node', 'scripts/download-mingit.js'], cwd=DESKTOP, env=env)
    if platform == 'darwin':
        run(['node', 'scripts/build-auth-helper.mjs'], cwd=DESKTOP, env=env)
    run(['node', 'scripts/prepare-platform-binaries.js'], cwd=DESKTOP, env=env)
    npm(['run', 'package', '--', '--platform=' + platform, '--arch=' + arch], env)
    packaged = DESKTOP / f'out/Biorouter-{target}'
    app = packaged / 'Biorouter.app' if platform == 'darwin' else packaged
    resources = app / ('Contents/Resources' if platform == 'darwin' else 'resources')
    dependencies(resources, target)
    if platform == 'darwin':
        sign_macos_candidate(app)
        makers = '@electron-forge/maker-dmg,@electron-forge/maker-zip'
    elif platform == 'win32':
        makers = '@electron-forge/maker-zip'
    else:
        makers = '@electron-forge/maker-deb,@electron-forge/maker-rpm'
    npm(['run', 'make', '--', '--skip-package', '--platform=' + platform, '--arch=' + arch,
         '--targets', makers], env)
    if platform == 'linux':
        version = json.loads((DESKTOP / 'package.json').read_text(encoding='utf-8'))['version']
        run(['bash', 'scripts/build-cli-linux-packages.sh', version], env=env)
    return target


def linux_install(archive, index):
    rpm = archive.suffix == '.rpm'
    image = 'rockylinux:9' if rpm else 'debian:bookworm-slim'
    install = 'dnf install -y /pkg/' + archive.name if rpm else 'apt-get update -qq && apt-get install -y /pkg/' + archive.name
    extras = ('dnf install -y epel-release && dnf install -y openbox xorg-x11-server-Xvfb xorg-x11-xauth dbus-x11' if rpm else
              'apt-get install -y xvfb xauth dbus-x11 openbox')
    command = install + '''
python3 -c 'import gi; gi.require_version("Atspi", "2.0"); gi.require_version("Gdk", "3.0"); from gi.repository import Atspi, Gdk'
''' + runtime_only_path_guard() + '''
python3 /repo/scripts/computer-use-package-acceptance.py installed-linux ''' + str(index) + '\n' + extras + '''
Xvfb :99 -screen 0 1280x900x24 >/tmp/xvfb.log 2>&1 &
xvfb=$!
trap 'kill "$xvfb" 2>/dev/null || true' EXIT
export DISPLAY=:99
display_ready=
for attempt in {1..50}; do
  kill -0 "$xvfb"
  if python3 -c 'import gi,sys; gi.require_version("Gdk", "3.0"); from gi.repository import Gdk; sys.exit(Gdk.Display.open(":99") is None)'; then
    display_ready=yes
    break
  fi
  sleep 0.1
done
test "$display_ready" = yes
dbus-run-session -- python3 /repo/scripts/computer-use-package-acceptance.py fixture-linux ''' + str(index)
    owned_container(['--cpus=2', '--memory=4g', '-v', f'{ROOT}:/repo:ro',
         '-v', f'{OUTPUT}:/evidence', '-v', f'{archive.parent}:/pkg:ro', image,
         'bash', '-euc', command])


def runtime_only_path_guard():
    return '''for tool in node npm; do
  if command -v "$tool"; then
    echo "Unexpected runtime toolchain executable: $tool" >&2
    exit 1
  fi
done
'''


def owned_container(arguments):
    # Create first so even a timed-out attached client leaves an exact owned ID to remove.
    container = subprocess.check_output(['docker', 'create', *map(str, arguments)],
                                        text=True, timeout=300).strip()
    if not re.fullmatch('[0-9a-f]{64}', container):
        raise ValueError('Docker did not return an exact container ID')
    try:
        run(['docker', 'start', '--attach', container], timeout=600)
    finally:
        run(['docker', 'rm', '--force', container], timeout=30)


def linux_paths(**roots):
    """The one installed helper and the CLI beside it. Roots are injectable so the
    GUI-vs-CLI branch and the 'exactly one' refusal are reachable from a test --
    otherwise they run only inside a container after a ~35 minute build, which is
    how the /opt assumption survived unnoticed from the day it was written."""
    manifests = installed_linux_helper_roots(**roots)
    if len(manifests) != 1:
        raise ValueError(f'Expected one installed helper: {manifests}')
    helper = manifests[0].parent
    # The CLI package puts the helper under <prefix>/libexec/biorouter and its
    # binary on PATH; the GUI package keeps both inside its resources tree.
    fhs = helper.parent.name == 'biorouter' and helper.parent.parent.name == 'libexec'
    cli = Path('/usr/bin/biorouter') if fhs else helper.parent / 'bin/biorouter'
    return cli, helper


def container_check(fixture, label):
    cli, helper = linux_paths()
    if fixture:
        run(['/usr/bin/python3', ROOT / 'scripts/test-computer-use-linux-fixture.py', helper,
             '--report', f'/evidence/linux-{label}-fixture.json'], timeout=120)
    else:
        run(['/usr/bin/python3', ROOT / 'scripts/computer-use-runtime.py', 'verify', 'linux-x64', '--directory', helper])
        run(['/usr/bin/python3', ROOT / 'scripts/verify-installed-computer-use.py', '--cli', cli,
             '--helper', helper, '--target', 'linux-x64', '--expect-status', 'desktop_unavailable',
             '--backends', '/evidence/backends.json', '--report', f'/evidence/linux-{label}-installed.json'], timeout=INSTALLED_CHECK_TIMEOUT)


def verify(target):
    platform = target.split('-')[0]
    archives = list((DESKTOP / 'out/make').rglob('*'))
    if platform == 'linux':
        archives += list((ROOT / 'dist/cli').glob('*'))
    extensions = {'darwin': {'.dmg', '.zip'}, 'win32': {'.zip'}, 'linux': {'.deb', '.rpm'}}[platform]
    archives = sorted(path for path in archives if path.is_file() and path.suffix in extensions)
    expected = {'darwin': 2, 'win32': 1, 'linux': 4}[platform]
    expected_formats = {'darwin': {'.dmg': 1, '.zip': 1}, 'win32': {'.zip': 1},
                        'linux': {'.deb': 2, '.rpm': 2}}[platform]
    if len(archives) != expected or Counter(path.suffix for path in archives) != expected_formats:
        raise ValueError(f'Expected {expected} actual release-format archives, found {archives}')
    receipts = []
    for index, archive in enumerate(archives):
        run([PYTHON, 'scripts/verify-computer-use-artifact.py', archive, target])
        if platform == 'linux':
            with tempfile.TemporaryDirectory(prefix='biorouter-package-content-') as temp:
                directory = Path(temp)
                if archive.suffix == '.deb':
                    run(['dpkg-deb', '-x', archive, directory])
                else:
                    run(['bsdtar', '-xf', archive, '-C', directory])
                if 'cli' not in archive.name:
                    dependencies(packaged_desktop_resources(directory), target)
            linux_install(archive, index)
        else:
            with tempfile.TemporaryDirectory(prefix='BioRouter installed ü ') as temp:
                directory = Path(temp)
                if archive.suffix == '.dmg':
                    mount = directory / 'mount'
                    mount.mkdir()
                    run(['hdiutil', 'attach', '-nobrowse', '-readonly', '-mountpoint', mount, archive])
                    try:
                        run(['ditto', mount / 'Biorouter.app', directory / 'Biorouter.app'])
                    finally:
                        run(['hdiutil', 'detach', mount])
                elif platform == 'darwin':
                    run(['ditto', '-x', '-k', archive, directory])
                else:
                    with zipfile.ZipFile(archive) as zipped:
                        zipped.extractall(directory)
                manifests = list(directory.rglob('computer-use/manifest.json'))
                if len(manifests) != 1:
                    raise ValueError('Extracted archive must contain exactly one helper')
                helper = manifests[0].parent
                resources = helper.parent
                dependencies(resources, target)
                cli = resources / ('bin/biorouter.exe' if platform == 'win32' else 'bin/biorouter')
                if platform == 'darwin':
                    run(['codesign', '--verify', '--deep', '--strict', resources.parent.parent])
                    run(['codesign', '--verify', '--deep', '--strict', helper / 'BioRouter Computer Use.app'])
                installed(cli, helper, target, 'ready,os_permission_required,desktop_unavailable' if platform == 'darwin' else 'ready', f'{target}-{index}')
                if platform == 'win32':
                    run([PYTHON, 'scripts/test-computer-use-windows-fixture.py', helper,
                         '--report', OUTPUT / 'windows-installed-fixture.json'], timeout=180)
                    run([PYTHON, 'scripts/test-computer-use-packaged-windows-console.py',
                         '--cli', cli, '--report', OUTPUT / 'windows-packaged-console.json'], timeout=180)
                    local = directory / 'isolated-user'
                    env = dict(os.environ, LOCALAPPDATA=str(local), BIOROUTER_DISABLE_KEYRING='true',
                               BIOROUTER_PATH_ROOT=str(directory / 'isolated-config'))
                    run([cli, 'setup-path'], env=env, timeout=30)
                    copied = local / 'Biorouter/bin/biorouter.exe'
                    if not copied.exists() or not (copied.parent / '.biorouter-origin').exists():
                        raise ValueError('Windows setup-path did not install the copied CLI and origin marker')
                    installed(copied, helper, target, 'ready', 'windows-copied-cli')
        receipts.append({'file': archive.name, 'sha256': digest(archive), 'bytes': archive.stat().st_size})
        shutil.copy2(archive, OUTPUT / archive.name)
    (OUTPUT / 'archives.json').write_text(json.dumps({'target': target, 'archives': receipts,
        'notarized': False, 'published': False, 'signing': 'ad-hoc' if platform == 'darwin' else 'unsigned'}, indent=2),
        encoding='utf-8')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=['build', 'verify', 'installed-linux', 'fixture-linux'])
    parser.add_argument('target')
    args = parser.parse_args()
    if args.mode == 'build':
        stage(args.target)
    elif args.mode == 'verify':
        verify(args.target)
    else:
        container_check(args.mode == 'fixture-linux', args.target)
