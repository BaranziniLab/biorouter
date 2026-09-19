const { FusesPlugin } = require('@electron-forge/plugin-fuses');
const { FuseV1Options, FuseVersion } = require('@electron/fuses');
const { AutoUnpackNativesPlugin } = require('@electron-forge/plugin-auto-unpack-natives');
const { resolve } = require('path');

// ⚠ Read from package.json, NOT `process.env.npm_package_version`. That variable
// only exists when forge is invoked through an `npm run` script, so a direct
// `npx electron-forge make` would name the installer `Biorouter-Setup-.exe` --
// and the updater matches that filename EXACTLY
// (`githubUpdater.ts`, `Biorouter-Setup-${v}.exe`), so a mis-named asset is an
// update that silently finds nothing. `WINDOWS_SETUP_EXE` is the one spelling
// both sides agree on; `forgeConfig.windowsSetupExe.test.ts` pins them together.
const { version: APP_VERSION } = require('./package.json');
const WINDOWS_SETUP_EXE = `Biorouter-Setup-${APP_VERSION}.exe`;

// `node-pty` is the only runtime dependency that cannot be bundled by Vite: it
// is a native module, so `vite.main.config.mts` externalises it and the built
// main.js issues a real `require('node-pty')`. That means the module has to
// physically exist inside the packaged app — but the Forge Vite plugin's
// default `packagerConfig.ignore` is `(file) => !file.startsWith('/.vite')`,
// which drops the entire `node_modules` tree. The shipped app.asar therefore
// contained exactly two entries (`/.vite` and `/package.json`), `require`
// threw MODULE_NOT_FOUND, and every terminal silently fell back to the
// TTY-less pipe backend.
//
// The plugin only installs its filter when `packagerConfig.ignore` is unset
// (see VitePlugin.resolveForgeConfig), so defining one here keeps its rule and
// adds the single exception node-pty needs.
//
// Only the target platform's prebuild is shipped. The tree also carries
// Windows prebuilds whose `.pdb` symbol files are ~40 MB, which have no
// business in a macOS bundle.
const nodePtyTargetPlatform = process.env.ELECTRON_PLATFORM || process.platform;
// The Windows and Linux targets are cross-built from an arm64 Mac and are
// x64-only, so `process.arch` is the wrong default for them — it would ship the
// win32-arm64 prebuild inside an x64 app, where node-pty fails to load it and
// silently falls back to pipes.
const nodePtyTargetArch =
  process.env.ELECTRON_ARCH || (nodePtyTargetPlatform === 'darwin' ? process.arch : 'x64');
const nodePtyPrebuildDir = `/node_modules/node-pty/prebuilds/${nodePtyTargetPlatform}-${nodePtyTargetArch}`;

const isUnder = (file, dir) => file === dir || file.startsWith(`${dir}/`);

/** Files electron-packager should copy into the app. Everything else is ignored. */
function keepInPackage(file) {
  if (!file) return true; // the package root itself
  if (isUnder(file, '/.vite')) return true;
  // The directory itself must be kept or packager never descends into it.
  if (file === '/node_modules') return true;
  if (!isUnder(file, '/node_modules/node-pty')) return false;
  if (file.endsWith('.pdb')) return false;
  if (isUnder(file, '/node_modules/node-pty/prebuilds')) {
    return (
      file === '/node_modules/node-pty/prebuilds' || isUnder(file, nodePtyPrebuildDir)
    );
  }
  return (
    file === '/node_modules/node-pty' ||
    file === '/node_modules/node-pty/package.json' ||
    isUnder(file, '/node_modules/node-pty/lib')
  );
}

let cfg = {
  // A native module cannot be `dlopen`'d from inside an asar archive, and
  // node-pty's macOS `spawn-helper` cannot be `posix_spawn`'d from one either.
  // node-pty handles this itself — `unixTerminal.js` rewrites `app.asar` to
  // `app.asar.unpacked` in the helper path — but only if the files are
  // actually unpacked. AutoUnpackNativesPlugin is not enough on its own: it
  // unpacks `**/*.node`, and `spawn-helper` has no extension, so it would stay
  // sealed in the archive at the exact path node-pty rewrites away from. The
  // plugin composes with this value rather than replacing it.
  asar: { unpack: '**/node_modules/node-pty/**' },
  ignore: (file) => !keepInPackage(file),
  // `src/web` is the ROOT-BASE build of the same renderer (`npm run build:web`),
  // which `biorouterd` serves to a browser when pointed at it with
  // BIOROUTER_SERVE_UI. It has to be an extraResource rather than a bundled
  // asset for one reason: the daemon is a separate process that reads it off
  // disk, and it cannot read out of app.asar. Every extraResource lands at
  // `Contents/Resources/<basename>` on macOS and `resources/<basename>` in the
  // Windows zip and the Linux staging tree — the same directory `bin/` lands
  // in — so the shipped `biorouterd` finds the bundle at `<exe dir>/../web`
  // with no path configuration anywhere.
  //
  // It must also stay a SIBLING of `src/bin`, never a child: `stage_bin` in
  // scripts/release.sh does `rm -rf ui/desktop/src/bin`, which would take the
  // bundle with it.
  extraResource: ['src/bin', 'src/images', 'src/web'],
  icon: 'src/images/icon',
  // macOS code signing and notarization
  // Activate by setting APPLE_ID and APPLE_APP_SPECIFIC_PASSWORD in the build environment.
  // Generate an app-specific password at https://appleid.apple.com/account/manage
  ...(process.env.APPLE_ID
    ? {
        osxSign: {
          identity: 'Developer ID Application: University of California at San Francisco (F3YYBXAFJ8)',
          hardenedRuntime: true,
          entitlements: 'entitlements.plist',
          'entitlements-inherit': 'entitlements.plist',
          'signature-flags': 'library',
        },
        // Notarization can be skipped (BIOROUTER_SKIP_NOTARIZE=1) for fast,
        // signed-but-not-notarized local/test builds — the signature alone is
        // enough for an in-place Squirrel.Mac update (identity match) and for
        // running locally without Gatekeeper quarantine. Release builds leave
        // it unset so the app is fully notarized + stapled.
        ...(process.env.BIOROUTER_SKIP_NOTARIZE === '1'
          ? {}
          : {
              osxNotarize: {
                tool: 'notarytool',
                appleId: process.env.APPLE_ID,
                appleIdPassword: process.env.APPLE_APP_SPECIFIC_PASSWORD,
                teamId: 'F3YYBXAFJ8',
              },
            }),
      }
    : {}),
  // Windows specific configuration
  win32: {
    icon: 'src/images/icon.ico',
    certificateFile: process.env.WINDOWS_CERTIFICATE_FILE,
    signingRole: process.env.WINDOW_SIGNING_ROLE,
    rfc3161TimeStampServer: 'http://timestamp.digicert.com',
    signWithParams: '/fd sha256 /tr http://timestamp.digicert.com /td sha256',
  },
  // Protocol registration
  protocols: [
    {
      name: 'BiorouterProtocol',
      schemes: ['biorouter'],
    },
  ],
  // macOS Info.plist extensions for drag-and-drop support
  extendInfo: {
    // Document types for drag-and-drop support onto dock icon
    CFBundleDocumentTypes: [
      {
        CFBundleTypeName: 'Folders',
        CFBundleTypeRole: 'Viewer',
        LSHandlerRank: 'Alternate',
        LSItemContentTypes: ['public.directory', 'public.folder'],
      },
      {
        CFBundleTypeName: 'Biorouter Extension Bundle',
        CFBundleTypeRole: 'Viewer',
        CFBundleTypeExtensions: ['brxt'],
        LSHandlerRank: 'Owner',
      },
    ],
  },
  // Windows file associations
  fileAssociations: [
    {
      ext: 'brxt',
      name: 'Biorouter Extension Bundle',
      description: 'Biorouter Extension Bundle',
      role: 'Viewer',
    },
  ],
};

module.exports = {
  packagerConfig: cfg,
  rebuildConfig: {},
  publishers: [
    {
      name: '@electron-forge/publisher-github',
      config: {
        repository: {
          owner: 'BaranziniLab',
          name: 'biorouter',
        },
        prerelease: false,
        draft: true,
      },
    },
  ],
  makers: [
    {
      name: '@electron-forge/maker-zip',
      platforms: ['darwin', 'win32', 'linux'],
      config: {
        arch: process.env.ELECTRON_ARCH === 'x64' ? ['x64'] : ['arm64'],
        options: {
          icon: 'src/images/icon.ico',
        },
      },
    },
    // Windows in-place updates.
    //
    // ⚠ Without this the Windows release is a plain zip, which nothing can
    // install *over* an existing copy — so Windows had no in-place updater at
    // all and fell back to "download it yourself and replace the folder".
    //
    // Squirrel.Windows rather than NSIS because this project packages with
    // electron-**forge**: `maker-squirrel` is first-party here (and was already
    // a declared devDependency), while NSIS + `latest.yml` belong to the
    // electron-builder world and would mean adopting a second packaging stack.
    //
    // The install-time half was already in place and is what makes this safe to
    // add: Squirrel re-launches the app with `--squirrel-install` /
    // `--squirrel-updated` / `--squirrel-uninstall` to create and remove
    // shortcuts, and `main.ts` already quits immediately on those
    // (`import started from 'electron-squirrel-startup'; if (started) app.quit()`).
    // Without that guard, installing would flash several real app windows.
    {
      name: '@electron-forge/maker-squirrel',
      platforms: ['win32'],
      config: {
        // ⚠ Squirrel keys its installed package on this name, and changing it
        // later orphans every existing install (the updater looks for a package
        // that is no longer published). It is `package.json`'s `name`, which is
        // what Squirrel defaults to, spelled out so a rename of that field
        // cannot silently break updates for shipped clients.
        name: 'biorouter_app',
        // The Start-menu and Add/Remove Programs entry the user actually reads.
        setupExe: WINDOWS_SETUP_EXE,
        setupIcon: 'src/images/icon.ico',
        iconUrl:
          'https://raw.githubusercontent.com/BaranziniLab/biorouter/main/ui/desktop/src/images/icon.ico',
        authors: 'Baranzini Lab, UCSF',
        description: 'Biorouter',
      },
    },
    {
      name: '@electron-forge/maker-dmg',
      platforms: ['darwin'],
      config: {
        icon: './src/images/icon.icns',
        format: 'ULFO',
        overwrite: true,
      },
    },
    {
      name: '@electron-forge/maker-deb',
      config: {
        name: 'Biorouter',
        bin: 'Biorouter',
        maintainer: 'BaranziniLab',
        homepage: 'https://github.com/BaranziniLab/biorouter',
        categories: ['Development'],
        mimeType: ['application/x-biorouter-brxt'],
        desktopTemplate: './forge.deb.desktop',
        options: {
          icon: 'src/images/icon.png',
          prefix: '/opt',
          // Runtime deps of the bundled llama-server (Llama Server provider):
          // OpenSSL 3 and OpenMP. Implies Debian 12+ / Ubuntu 22.04+.
          //
          // libxcb1 is a dep of the bundled `biorouter`/`biorouterd` themselves,
          // not of llama-server: both carry libxcb.so.1 as a DT_NEEDED entry
          // (arboard's clipboard and xcap's screen capture), so the loader
          // refuses to start them without it. It has always been installed here
          // anyway, but only INCIDENTALLY — electron-installer-debian's own
          // defaults ask for libgtk-3-0, which drags libxcb1 in transitively.
          // Naming it makes the requirement ours instead of a side effect of a
          // dependency we do not control, and the array is merged with those
          // Electron defaults rather than replacing them, so this is additive.
          // zlib1g provides libz.so.1, linked through git2/libgit2.
          // scripts/check-linux-runtime-deps.sh asserts it stays in step with
          // what the binaries actually link.
          depends: ['libssl3', 'libgomp1', 'libxcb1', 'zlib1g'],
        },
      },
    },
    {
      name: '@electron-forge/maker-rpm',
      config: {
        name: 'Biorouter',
        bin: 'Biorouter',
        maintainer: 'BaranziniLab',
        homepage: 'https://github.com/BaranziniLab/biorouter',
        categories: ['Development'],
        mimeType: ['application/x-biorouter-brxt'],
        desktopTemplate: './forge.rpm.desktop',
        options: {
          icon: 'src/images/icon.png',
          prefix: '/opt',
          // openssl-libs ships libssl.so.3 on EL9+/Fedora; libgomp for llama-server.
          // libxcb is the RPM spelling of the deb's libxcb1 — see the maker-deb
          // comment above for why the bundled binaries need it.
          // zlib provides libz.so.1 on RPM-based distributions.
          requires: ['openssl-libs', 'libgomp', 'libxcb', 'zlib'],
          fpm: ['--rpm-rpmbuild-define', '_build_id_links none'],
        },
      },
    },
    {
      name: '@electron-forge/maker-flatpak',
      config: {
        options: {
          categories: ['Development'],
          icon: 'src/images/icon.png',
          homepage: 'https://github.com/BaranziniLab/biorouter',
          runtimeVersion: '25.08',
          baseVersion: '25.08',
          bin: 'Biorouter',
          modules: [
            {
              name: 'libbz2-shim',
              buildsystem: 'simple',
              'build-commands': [
                // Create the lib directory in the app bundle
                'mkdir -p /app/lib',
                // Point to the actual library in the 25.08 runtime
                // We use a wildcard to handle multi-arch paths (x86_64-linux-gnu, etc)
                'ln -s $(find /usr/lib -name "libbz2.so.1" | head -n 1) /app/lib/libbz2.so.1.0'
              ]
            }
          ],
          finishArgs: [
            '--share=ipc',
            '--socket=x11',
            '--socket=wayland',
            '--device=dri',
            '--share=network',
            '--filesystem=home',
            '--talk-name=org.freedesktop.Notifications',
            '--socket=session-bus',
            '--socket=system-bus',
            // This ensures the app looks in our shim folder first
            '--env=LD_LIBRARY_PATH=/app/lib'
          ],
        },
      },
    },
  ],
  plugins: [
    {
      name: '@electron-forge/plugin-vite',
      config: {
        build: [
          {
            entry: 'src/main.ts',
            config: 'vite.main.config.mts',
          },
          {
            entry: 'src/preload.ts',
            config: 'vite.preload.config.mts',
          },
        ],
        renderer: [
          {
            name: 'main_window',
            config: 'vite.renderer.config.mts',
          },
        ],
      },
    },
    new AutoUnpackNativesPlugin({}),
    // Fuses are used to enable/disable various Electron functionality
    // at package time, before code signing the application
    new FusesPlugin({
      version: FuseVersion.V1,
      [FuseV1Options.RunAsNode]: false,
      [FuseV1Options.EnableCookieEncryption]: true,
      [FuseV1Options.EnableNodeOptionsEnvironmentVariable]: false,
      [FuseV1Options.EnableNodeCliInspectArguments]: false,
      [FuseV1Options.EnableEmbeddedAsarIntegrityValidation]: true,
      [FuseV1Options.OnlyLoadAppFromAsar]: true,
    }),
  ],
};
