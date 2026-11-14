//! Builds the local dictation engine (DD-10 of
//! `docs/history/local-dictation-2026-11/implementation-plan.md`).
//!
//! Without the `engine` feature this only reads the pin, so `build_info` can name it.
//! With the feature, whisper.cpp is configured by CMake from a copy of the vendored tree
//! (rule 1), always as a Release build whatever the Cargo profile (rule 7), with an
//! explicit instruction set and never the build machine's own (rule 2), as static
//! libraries without OpenMP (rule 3), with static libstdc++ on Linux and mingw (rule 4),
//! verbatim archive names and `advapi32` on mingw (rule 5), Metal embedded on Apple
//! Silicon and no BLAS anywhere (rule 6). The shim is compiled with `cc` and declared by
//! hand in `src/engine/ffi.rs`, with no bindgen (rule 8). The vendored tree must be
//! unchanged after the build (rule 9).

use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(br_no_native)");
    println!("cargo:rerun-if-changed=build.rs");

    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let vendored = manifest
        .join("..")
        .join("..")
        .join("vendor")
        .join("whisper.cpp");
    let pin_path = vendored.join("pin.json");
    println!("cargo:rerun-if-changed={}", pin_path.display());
    let pin = std::fs::read_to_string(&pin_path)
        .ok()
        .and_then(|text| Pin::parse(&text));

    let (commit, ggml) = match &pin {
        Some(pin) => (pin.short_commit(), pin.ggml.clone()),
        None => ("unknown".to_string(), "unknown".to_string()),
    };
    println!("cargo:rustc-env=BR_WHISPER_COMMIT={commit}");
    println!("cargo:rustc-env=BR_GGML_VERSION={ggml}");

    #[cfg(feature = "engine")]
    let isa = engine::build(&manifest, &vendored, pin.is_some());
    #[cfg(not(feature = "engine"))]
    let isa = String::new();
    println!("cargo:rustc-env=BR_DICTATION_ISA={isa}");
}

/// The two strings of `vendor/whisper.cpp/pin.json` the binaries report. A small string
/// scan, so the build script needs no JSON parser.
struct Pin {
    commit: String,
    ggml: String,
}

impl Pin {
    fn parse(text: &str) -> Option<Pin> {
        let commit = json_string(text, "commit")?;
        let ggml = json_string(text, "ggml")?;
        let is_hex = commit.len() == 40 && commit.chars().all(|c| c.is_ascii_hexdigit());
        (is_hex && !ggml.is_empty()).then_some(Pin { commit, ggml })
    }

    fn short_commit(&self) -> String {
        self.commit.chars().take(8).collect()
    }
}

fn json_string(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let (_, rest) = text.split_once(needle.as_str())?;
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let (value, _) = rest.split_once('"')?;
    Some(value.to_string())
}

#[cfg(feature = "engine")]
mod engine {
    use std::collections::BTreeMap;
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::SystemTime;

    /// M-3. `false` turns the MSVC test leg into the `no_engine` stub. Flip only after a
    /// dispatched rust.yml run shows the MSVC build cannot be fixed inside PR-09.
    const MSVC_NATIVE: bool = true;

    /// The x86 instruction set every shipped x86 build carries (build rule 2): AVX2, FMA,
    /// F16C and BMI2. The CPU gate refuses processors without them.
    const X86_FLAGS: [&str; 6] = [
        "GGML_SSE42",
        "GGML_AVX",
        "GGML_AVX2",
        "GGML_FMA",
        "GGML_F16C",
        "GGML_BMI2",
    ];

    struct Target {
        triple: String,
        arch: String,
        os: String,
        env: String,
    }

    impl Target {
        fn from_env() -> Target {
            let var = |name: &str| env::var(name).unwrap_or_default();
            Target {
                triple: var("TARGET"),
                arch: var("CARGO_CFG_TARGET_ARCH"),
                os: var("CARGO_CFG_TARGET_OS"),
                env: var("CARGO_CFG_TARGET_ENV"),
            }
        }

        fn apple(&self) -> bool {
            self.os == "macos"
        }

        fn msvc(&self) -> bool {
            self.env == "msvc"
        }

        fn windows(&self) -> bool {
            self.os == "windows"
        }

        fn mingw(&self) -> bool {
            self.windows() && self.env == "gnu"
        }
    }

    /// The instruction set families the engine builds for; any other target builds no
    /// engine and answers `no_engine` (`br_no_native`).
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Flavor {
        X86,
        MacArm,
        LinuxArm,
    }

    impl Flavor {
        fn of(t: &Target) -> Option<Flavor> {
            match (t.arch.as_str(), t.os.as_str()) {
                ("x86_64", "macos") | ("x86_64", "linux") => Some(Flavor::X86),
                ("x86_64", "windows") if t.env == "gnu" => Some(Flavor::X86),
                ("x86_64", "windows") if t.env == "msvc" && MSVC_NATIVE => Some(Flavor::X86),
                ("aarch64", "macos") => Some(Flavor::MacArm),
                ("aarch64", "linux") => Some(Flavor::LinuxArm),
                _ => None,
            }
        }

        fn isa(self) -> &'static str {
            match self {
                Flavor::X86 => "x86-64-v3",
                Flavor::MacArm => "armv8.2-a+dotprod+fp16",
                Flavor::LinuxArm => "armv8.2-a+dotprod",
            }
        }
    }

    /// Builds whisper.cpp and the shim and prints the link lines. Returns the ISA string
    /// the binary reports, or an empty string for a target the engine does not build on.
    pub fn build(manifest: &Path, vendored: &Path, pinned: bool) -> String {
        let target = Target::from_env();

        println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
        if target.apple() && env::var_os("MACOSX_DEPLOYMENT_TARGET").is_none() {
            panic!(
                "MACOSX_DEPLOYMENT_TARGET is not set. The repository's .cargo/config.toml sets it \
                 to 12.0 (DD-11). Build from the repository root, or export \
                 MACOSX_DEPLOYMENT_TARGET=12.0."
            );
        }

        let Some(flavor) = Flavor::of(&target) else {
            println!("cargo:rustc-cfg=br_no_native");
            println!(
                "cargo:warning=the local dictation engine is not built for {}; this build answers no_engine",
                target.triple
            );
            return String::new();
        };
        assert!(
            pinned,
            "vendor/whisper.cpp/pin.json is missing or unreadable, so the engine cannot be built"
        );

        let source = vendored.join("source");
        let shim_dir = manifest.join("shim");
        let wrapper = manifest.join("cmake");
        println!("cargo:rerun-if-changed={}", source.display());
        println!(
            "cargo:rerun-if-changed={}",
            shim_dir.join("br_whisper.h").display()
        );
        println!(
            "cargo:rerun-if-changed={}",
            shim_dir.join("br_whisper.cpp").display()
        );
        println!(
            "cargo:rerun-if-changed={}",
            wrapper.join("CMakeLists.txt").display()
        );

        let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
        let copy = out.join("whisper-src");

        // Rules 1 and 9: build from a copy, and prove the vendored tree was not touched.
        let before = snapshot(&source);
        assert!(
            !before.is_empty(),
            "vendor/whisper.cpp/source is empty or missing"
        );
        sync(&source, &before, &copy);

        let dst = configure_and_build(&target, flavor, &wrapper, &copy);

        let after = snapshot(&source);
        if before != after {
            let mut changed: Vec<String> = before
                .iter()
                .filter(|(path, meta)| after.get(*path) != Some(*meta))
                .map(|(path, _)| path.display().to_string())
                .chain(
                    after
                        .keys()
                        .filter(|path| !before.contains_key(*path))
                        .map(|path| path.display().to_string()),
                )
                .collect();
            changed.truncate(10);
            panic!(
                "the vendored whisper.cpp tree changed during the build: {}",
                changed.join(", ")
            );
        }

        compile_shim(&target, &shim_dir, &copy);
        link(&target, flavor, &dst);
        flavor.isa().to_string()
    }

    type Tree = BTreeMap<PathBuf, (u64, Option<SystemTime>)>;

    /// Every file under `root` with its length and modified time, keyed by relative path.
    /// A metadata snapshot catches any write without hashing the tree on every build;
    /// `scripts/check-vendored-whisper-cpp.sh` keeps the content hash check.
    fn snapshot(root: &Path) -> Tree {
        let mut tree = Tree::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(dir) = pending.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(meta) = fs::metadata(&path) else {
                    continue;
                };
                if meta.is_dir() {
                    pending.push(path);
                } else if let Ok(rel) = path.strip_prefix(root) {
                    tree.insert(rel.to_path_buf(), (meta.len(), meta.modified().ok()));
                }
            }
        }
        tree
    }

    /// Copies `files` from `source` into `dest`, but only a file whose copy is missing or
    /// differs in length or modified time, and gives each copy the source's modified time,
    /// so CMake sees an unchanged file as unchanged and an edit to the shim recompiles only
    /// the shim. Files that no longer exist in the source are removed from the copy.
    fn sync(source: &Path, files: &Tree, dest: &Path) {
        for (rel, (len, modified)) in files {
            let from = source.join(rel);
            let to = dest.join(rel);
            let fresh = fs::metadata(&to)
                .ok()
                .is_some_and(|m| m.len() == *len && m.modified().ok() == *modified);
            if fresh {
                continue;
            }
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent)
                    .unwrap_or_else(|e| panic!("could not create {}: {e}", parent.display()));
            }
            let _ = fs::remove_file(&to);
            fs::copy(&from, &to)
                .unwrap_or_else(|e| panic!("could not copy {}: {e}", from.display()));
            if let Some(time) = modified {
                // Best effort: without it CMake only recompiles the file once more.
                let _ = fs::OpenOptions::new()
                    .write(true)
                    .open(&to)
                    .and_then(|f| f.set_modified(*time));
            }
        }
        for rel in snapshot(dest).keys() {
            if !files.contains_key(rel) {
                let _ = fs::remove_file(dest.join(rel));
            }
        }
    }

    /// CMake on the wrapper project. Every definition is explicit, so nothing depends on
    /// the build machine or on the Cargo profile.
    fn configure_and_build(
        target: &Target,
        flavor: Flavor,
        wrapper: &Path,
        copy: &Path,
    ) -> PathBuf {
        let mut cfg = cmake::Config::new(wrapper);
        // Rule 7: the cmake crate maps opt-level 0 to Debug and "s"/"z" to MinSizeRel; the
        // engine is always optimized.
        cfg.profile("Release");
        // CMake reads backslashes in an unquoted argument as escapes.
        cfg.define("BR_WHISPER_SRC", copy.to_string_lossy().replace('\\', "/"));
        for (key, value) in [
            ("BUILD_SHARED_LIBS", "OFF"),
            ("CMAKE_INSTALL_LIBDIR", "lib"),
            ("WHISPER_BUILD_TESTS", "OFF"),
            ("WHISPER_BUILD_EXAMPLES", "OFF"),
            ("WHISPER_BUILD_SERVER", "OFF"),
            ("WHISPER_SDL2", "OFF"),
            ("WHISPER_CURL", "OFF"),
            ("WHISPER_ALL_WARNINGS", "OFF"),
            ("GGML_NATIVE", "OFF"),
            ("GGML_OPENMP", "OFF"),
            ("GGML_CCACHE", "OFF"),
            ("GGML_BUILD_TESTS", "OFF"),
            ("GGML_BUILD_EXAMPLES", "OFF"),
            ("GGML_ALL_WARNINGS", "OFF"),
            ("GGML_BACKEND_DL", "OFF"),
            ("GGML_BLAS", "OFF"),
        ] {
            cfg.define(key, value);
        }

        match flavor {
            Flavor::X86 => {
                for flag in X86_FLAGS {
                    cfg.define(flag, "ON");
                }
                if target.apple() {
                    cfg.define("GGML_METAL", "OFF");
                }
            }
            Flavor::MacArm => {
                cfg.define("GGML_METAL", "ON")
                    .define("GGML_METAL_EMBED_LIBRARY", "ON")
                    .define("GGML_CPU_ARM_ARCH", "armv8.2-a+dotprod+fp16");
            }
            Flavor::LinuxArm => {
                cfg.define("GGML_CPU_ARM_ARCH", "armv8.2-a+dotprod");
            }
        }

        if target.msvc() {
            // The Visual Studio generator takes its optimization from the per configuration
            // variables, which the cmake crate fills with every /O flag removed. /MP lets
            // each project compile its files in parallel; without it MSBuild compiles every
            // file of ggml-cpu one after another.
            cfg.cflag("/utf-8").cxxflag("/utf-8");
            cfg.cflag("/MP").cxxflag("/MP");
            cfg.define("CMAKE_C_FLAGS_RELEASE", "/MD /O2 /Ob2 /DNDEBUG /utf-8");
            cfg.define(
                "CMAKE_CXX_FLAGS_RELEASE",
                "/MD /O2 /Ob2 /DNDEBUG /utf-8 /EHsc",
            );
        } else {
            // Explicit, so a -O0 that `cc` puts into CMAKE_C_FLAGS for a debug Cargo profile
            // is always overridden: the release flags come later on the command line.
            cfg.define("CMAKE_C_FLAGS_RELEASE", "-O3 -DNDEBUG");
            cfg.define("CMAKE_CXX_FLAGS_RELEASE", "-O3 -DNDEBUG");
        }

        cfg.build()
    }

    fn compile_shim(target: &Target, shim_dir: &Path, copy: &Path) {
        let mut build = cc::Build::new();
        build
            .cpp(true)
            .std("c++17")
            .file(shim_dir.join("br_whisper.cpp"))
            .include(shim_dir)
            .include(copy.join("include"))
            .include(copy.join("ggml").join("include"))
            .warnings(false);
        if !target.apple() && !target.msvc() {
            // cc would otherwise add a DYNAMIC -lstdc++ (libstdc++.so.6, libstdc++-6.dll),
            // which rule 4 forbids; the static one is linked below.
            build.cpp_link_stdlib(None);
        }
        if target.msvc() {
            build.flag("/EHsc").flag("/utf-8");
        }
        build.compile("br_whisper_shim");
    }

    /// The link lines, after the shim's own (GNU ld resolves archives left to right and
    /// the shim references whisper, which references ggml).
    fn link(target: &Target, flavor: Flavor, dst: &Path) {
        println!(
            "cargo:rustc-link-search=native={}",
            dst.join("lib").display()
        );
        println!("cargo:rustc-link-lib=static=whisper");

        let metal = flavor == Flavor::MacArm;
        if target.mingw() {
            // Rule 5: ggml drops the "lib" prefix on WIN32, so -lggml finds nothing.
            println!("cargo:rustc-link-lib=static:+verbatim=ggml.a");
            println!("cargo:rustc-link-lib=static:+verbatim=ggml-cpu.a");
            println!("cargo:rustc-link-lib=static:+verbatim=ggml-base.a");
        } else {
            println!("cargo:rustc-link-lib=static=ggml");
            println!("cargo:rustc-link-lib=static=ggml-cpu");
            if metal {
                println!("cargo:rustc-link-lib=static=ggml-metal");
            }
            println!("cargo:rustc-link-lib=static=ggml-base");
        }

        if target.apple() {
            if metal {
                for framework in ["Metal", "MetalKit", "Foundation"] {
                    println!("cargo:rustc-link-lib=framework={framework}");
                }
            }
            println!("cargo:rustc-link-lib=framework=Accelerate");
            println!("cargo:rustc-link-lib=dylib=c++");
        } else if !target.msvc() {
            link_static_libstdcxx(target);
        }

        if target.windows() {
            // ggml-cpu reads the processor's name from the registry.
            println!("cargo:rustc-link-lib=dylib=advapi32");
        }
    }

    /// Rule 4: libstdc++ linked statically (GCC Runtime Library Exception), so the binary
    /// gains no `libstdc++.so.6`, no `GLIBCXX` symbol version and no `libstdc++-6.dll`.
    fn link_static_libstdcxx(target: &Target) {
        let compiler = cc::Build::new().cpp(true).get_compiler();
        let output = Command::new(compiler.path())
            .arg("-print-file-name=libstdc++.a")
            .output()
            .unwrap_or_else(|e| {
                panic!(
                    "could not run {} to find libstdc++.a: {e}",
                    compiler.path().display()
                )
            });
        let answer = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let archive = PathBuf::from(&answer);
        if !output.status.success() || !archive.is_absolute() || !archive.is_file() {
            panic!(
                "the static C++ runtime (libstdc++.a) was not found for {}; install it: g++ on \
                 Debian or Ubuntu, gcc on Arch, libstdc++-static on Fedora (on RHEL and Rocky, \
                 from the CRB repository), g++-mingw-w64-x86-64 for the mingw target, and \
                 elsewhere the package that ships libstdc++.a",
                target.triple
            );
        }
        let dir = archive
            .parent()
            .expect("an absolute file path has a parent");
        println!("cargo:rustc-link-search=native={}", dir.display());
        println!("cargo:rustc-link-lib=static=stdc++");
    }
}
