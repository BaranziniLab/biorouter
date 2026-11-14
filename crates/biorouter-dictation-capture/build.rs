//! Compiles `shim/br_capture.c` over the vendored miniaudio (DD-73).
//!
//! Only the backends dictation can use are compiled: CoreAudio on macOS, WASAPI on
//! Windows, PulseAudio and ALSA on Linux. Every other backend is left out with
//! `MA_ENABLE_ONLY_SPECIFIC_BACKENDS`, the Null backend among them, because it silently
//! "records" silence where no audio system exists [M, the PR-08 build spike]. JACK, sndio,
//! OSS and the rest are left out too, so their library names are not even in the binary.
//!
//! Linking per platform:
//! * Linux: miniaudio's default runtime linking, so `libpulse.so.0` and `libasound.so.2`
//!   are opened with `dlopen` when a recording starts and `DT_NEEDED` stays inside
//!   `BASE_LIBS` (`scripts/check-linux-runtime-deps.sh`).
//! * Windows: runtime linking too; the shim imports only system DLLs.
//! * macOS: `MA_NO_RUNTIME_LINKING` and the three frameworks, because miniaudio's own
//!   notes say its runtime linked build may not pass Apple's notarization.
//!
//! A target outside those three builds no shim and sets `br_no_capture`, so the capture
//! role answers that this build has no microphone.

use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(br_no_capture)");
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let shim = manifest.join("shim").join("br_capture.c");
    let examples = manifest.join("../../vendor/whisper.cpp/source/examples");
    let header = examples.join("miniaudio.h");
    println!("cargo:rerun-if-changed={}", shim.display());
    println!("cargo:rerun-if-changed={}", header.display());
    println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");

    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let backends: &[&str] = match os.as_str() {
        "macos" => &["MA_ENABLE_COREAUDIO"],
        "windows" => &["MA_ENABLE_WASAPI"],
        "linux" => &["MA_ENABLE_PULSEAUDIO", "MA_ENABLE_ALSA"],
        _ => {
            println!("cargo:rustc-cfg=br_no_capture");
            return;
        }
    };
    if os == "macos" && env::var_os("MACOSX_DEPLOYMENT_TARGET").is_none() {
        panic!(
            "MACOSX_DEPLOYMENT_TARGET is not set. The repository's .cargo/config.toml sets it \
             to 12.0 (DD-11). Build from the repository root, or export \
             MACOSX_DEPLOYMENT_TARGET=12.0."
        );
    }
    assert!(
        header.is_file(),
        "{} is missing: the vendored miniaudio header is part of vendor/whisper.cpp",
        header.display()
    );

    let mut build = cc::Build::new();
    build
        .file(&shim)
        .include(&examples)
        // miniaudio is third party code compiled as is; its warnings are not ours.
        .warnings(false)
        .opt_level(2)
        .define("MA_ENABLE_ONLY_SPECIFIC_BACKENDS", None)
        .define("MA_NO_NULL", None)
        .define("MA_NO_DECODING", None)
        .define("MA_NO_ENCODING", None)
        .define("MA_NO_GENERATION", None)
        .define("MA_NO_RESOURCE_MANAGER", None)
        .define("MA_NO_NODE_GRAPH", None)
        .define("MA_NO_ENGINE", None);
    for backend in backends {
        build.define(backend, None);
    }
    if os == "macos" {
        build.define("MA_NO_RUNTIME_LINKING", None);
        for framework in ["CoreFoundation", "CoreAudio", "AudioToolbox"] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
    }
    build.compile("br_capture");
    if os == "linux" {
        // `dlopen` for the sound libraries, and the threads miniaudio starts. Both are in
        // `BASE_LIBS` on the oldest supported glibc.
        println!("cargo:rustc-link-lib=dl");
        println!("cargo:rustc-link-lib=pthread");
        println!("cargo:rustc-link-lib=m");
    }
}
