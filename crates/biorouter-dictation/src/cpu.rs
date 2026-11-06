//! The CPU gate (DD-9): whether this processor can run the engine this binary carries.
//!
//! The engine is compiled for `x86-64-v3` on x86 (AVX2, FMA, F16C, BMI2) and for
//! `armv8.2-a+dotprod` on Linux arm64, so on an older processor it would die with
//! `SIGILL`. The parent asks this module before it starts a worker, and the worker asks
//! it again about its own process before it calls into ggml, so an old processor gets a
//! sentence and exit 20 instead of a crash. Nothing here calls into ggml or spawns.

use std::sync::OnceLock;

use serde::Serialize;

/// The features the engine's instruction set needs, as this process sees them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CpuFeatures {
    pub avx2: bool,
    pub fma: bool,
    pub f16c: bool,
    pub bmi2: bool,
    pub dotprod: bool,
}

/// The hardware classes of `docs/dictation/engine-and-models.md` ("Hardware classes").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HardwareClass {
    /// Apple Silicon: Metal.
    As,
    /// A processor the CPU engine runs on.
    Cpu,
    /// A processor the engine cannot run on.
    No,
}

/// The platform families the gate distinguishes. `classify` uses the one this binary was
/// compiled for; tests drive every row through `classify_for`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    MacArm,
    /// arm64 anywhere but macOS: needs `dotprod`.
    OtherArm,
    X86,
    /// A target the engine is not built for.
    Other,
}

impl Platform {
    pub const fn host() -> Platform {
        if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
            Platform::MacArm
        } else if cfg!(target_arch = "aarch64") {
            Platform::OtherArm
        } else if cfg!(target_arch = "x86_64") {
            Platform::X86
        } else {
            Platform::Other
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CpuReport {
    pub class: HardwareClass,
    /// Lower case ids in a fixed order: `avx2`, `fma`, `f16c`, `bmi2`, `dotprod`.
    pub missing: Vec<&'static str>,
    /// This is the x86_64 build translated by Rosetta.
    pub rosetta: bool,
    /// CPUID reports a hypervisor (x86 only).
    pub hypervisor: bool,
}

/// MSG-CPU, with `{missing}` replaced by [`display_list`].
pub const MSG_CPU: &str = "This processor does not support the instructions local dictation needs ({missing}). Your system's own dictation still works, without your Biorouter dictionary.";
/// MSG-CPU-ROSETTA.
pub const MSG_CPU_ROSETTA: &str = "This is the Intel build of Biorouter running under Rosetta, which cannot run local dictation on this version of macOS. Install the Apple Silicon build of Biorouter.";
/// MSG-CPU-VM.
pub const MSG_CPU_VM: &str = "If this is a virtual machine, ask its administrator to expose the host CPU model (for example `host-passthrough`).";

/// The first macOS on which Rosetta translates AVX2.
const ROSETTA_AVX2_MACOS: u32 = 15;

/// This process's own view of the processor's features. No ggml call.
pub fn detect() -> CpuFeatures {
    #[allow(unused_mut)]
    let mut features = CpuFeatures::default();
    #[cfg(target_arch = "x86_64")]
    {
        features.avx2 = std::arch::is_x86_feature_detected!("avx2");
        features.fma = std::arch::is_x86_feature_detected!("fma");
        features.f16c = std::arch::is_x86_feature_detected!("f16c");
        features.bmi2 = std::arch::is_x86_feature_detected!("bmi2");
    }
    #[cfg(target_arch = "aarch64")]
    {
        features.dotprod = std::arch::is_aarch64_feature_detected!("dotprod");
    }
    features
}

/// The features `platform` needs that `f` lacks, in the fixed order.
pub fn missing_features_for(platform: Platform, f: &CpuFeatures) -> Vec<&'static str> {
    match platform {
        Platform::X86 => [
            ("avx2", f.avx2),
            ("fma", f.fma),
            ("f16c", f.f16c),
            ("bmi2", f.bmi2),
        ]
        .into_iter()
        .filter(|(_, present)| !present)
        .map(|(id, _)| id)
        .collect(),
        Platform::OtherArm if !f.dotprod => vec!["dotprod"],
        Platform::MacArm | Platform::OtherArm | Platform::Other => Vec::new(),
    }
}

/// [`missing_features_for`] on this binary's platform.
pub fn missing_features(f: &CpuFeatures) -> Vec<&'static str> {
    missing_features_for(Platform::host(), f)
}

/// The gate's decision on this binary's platform.
pub fn classify(f: &CpuFeatures, rosetta: bool, macos_major: Option<u32>) -> CpuReport {
    classify_for(Platform::host(), f, rosetta, macos_major)
}

/// The gate's decision; pure, so the table test reaches every row on any host.
///
/// Rosetta hides AVX, AVX2, FMA, F16C and BMI2 from CPUID but executes them from macOS
/// 15, where the parent starts the worker with `ROSETTA_ADVERTISE_AVX=1` and the worker
/// confirms the features in `hello`. Before 15 it cannot run them at all.
pub fn classify_for(
    platform: Platform,
    f: &CpuFeatures,
    rosetta: bool,
    macos_major: Option<u32>,
) -> CpuReport {
    let missing = missing_features_for(platform, f);
    let class = match platform {
        Platform::MacArm => HardwareClass::As,
        Platform::Other => HardwareClass::No,
        Platform::OtherArm | Platform::X86 if missing.is_empty() => HardwareClass::Cpu,
        Platform::X86 if rosetta && macos_major.is_some_and(|v| v >= ROSETTA_AVX2_MACOS) => {
            HardwareClass::Cpu
        }
        Platform::OtherArm | Platform::X86 => HardwareClass::No,
    };
    CpuReport {
        class,
        missing,
        rosetta,
        hypervisor: false,
    }
}

/// `sysctl.proc_translated == 1`: this x86_64 process runs under Rosetta. Always false
/// elsewhere.
pub fn rosetta_translated() -> bool {
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        sysctl_i32(c"sysctl.proc_translated") == Some(1)
    }
    #[cfg(not(all(target_os = "macos", target_arch = "x86_64")))]
    {
        false
    }
}

/// The macOS major version from `kern.osproductversion`; `None` elsewhere.
pub fn macos_major() -> Option<u32> {
    #[cfg(target_os = "macos")]
    {
        let version = sysctl_string(c"kern.osproductversion")?;
        version.split('.').next()?.trim().parse().ok()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// CPUID leaf 1, ECX bit 31: a hypervisor is present. x86 only.
pub fn hypervisor() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: CPUID is available on every x86_64 processor.
        #[allow(unused_unsafe)]
        let leaf = unsafe { std::arch::x86_64::__cpuid(1) };
        leaf.ecx & (1 << 31) != 0
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

/// The gate's decision about this host, computed once. Never spawns.
pub fn host_report() -> &'static CpuReport {
    static REPORT: OnceLock<CpuReport> = OnceLock::new();
    REPORT.get_or_init(|| {
        let mut report = classify(&detect(), rosetta_translated(), macos_major());
        report.hypervisor = hypervisor();
        report
    })
}

/// `["avx2", "fma"]` gives "AVX2 and FMA"; three or more give "AVX2, FMA, F16C and
/// BMI2". `dotprod` keeps its lower case spelling.
pub fn display_list(missing: &[&str]) -> String {
    let names: Vec<String> = missing
        .iter()
        .map(|id| match *id {
            "dotprod" => "dotprod".to_string(),
            other => other.to_ascii_uppercase(),
        })
        .collect();
    match names.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// MSG-CPU or MSG-CPU-ROSETTA for a refused processor, in the catalog's exact words.
pub fn sentence(report: &CpuReport) -> Option<String> {
    if report.class != HardwareClass::No {
        return None;
    }
    if report.rosetta {
        return Some(MSG_CPU_ROSETTA.to_string());
    }
    if report.missing.is_empty() {
        return None;
    }
    Some(MSG_CPU.replace("{missing}", &display_list(&report.missing)))
}

/// MSG-CPU-VM, after MSG-CPU on a refused processor that CPUID says is virtual.
pub fn vm_sentence(report: &CpuReport) -> Option<&'static str> {
    (report.class == HardwareClass::No && report.hypervisor && !report.rosetta)
        .then_some(MSG_CPU_VM)
}

#[cfg(target_os = "macos")]
fn sysctl_string(name: &std::ffi::CStr) -> Option<String> {
    let mut buf = [0u8; 64];
    let mut len = buf.len();
    // SAFETY: the buffer and its length are valid for the call; the name is NUL terminated.
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            buf.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return None;
    }
    let bytes = buf.get(..len.min(buf.len()))?;
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    String::from_utf8(bytes.get(..end)?.to_vec()).ok()
}

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
fn sysctl_i32(name: &std::ffi::CStr) -> Option<i32> {
    let mut value: libc::c_int = 0;
    let mut len = std::mem::size_of::<libc::c_int>();
    // SAFETY: the out pointer and its length describe `value`; the name is NUL terminated.
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&mut value as *mut libc::c_int).cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    (rc == 0).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_X86: CpuFeatures = CpuFeatures {
        avx2: true,
        fma: true,
        f16c: true,
        bmi2: true,
        dotprod: false,
    };
    const NONE: CpuFeatures = CpuFeatures {
        avx2: false,
        fma: false,
        f16c: false,
        bmi2: false,
        dotprod: false,
    };

    /// (platform, features, rosetta, macOS, class, missing)
    type Row = (
        Platform,
        CpuFeatures,
        bool,
        Option<u32>,
        HardwareClass,
        Vec<&'static str>,
    );

    #[test]
    fn the_gate_table() {
        let rows: Vec<Row> = vec![
            // Apple Silicon is always Metal capable.
            (
                Platform::MacArm,
                NONE,
                false,
                Some(12),
                HardwareClass::As,
                vec![],
            ),
            // Linux arm64 needs dotprod.
            (
                Platform::OtherArm,
                CpuFeatures {
                    dotprod: true,
                    ..NONE
                },
                false,
                None,
                HardwareClass::Cpu,
                vec![],
            ),
            (
                Platform::OtherArm,
                NONE,
                false,
                None,
                HardwareClass::No,
                vec!["dotprod"],
            ),
            // x86 needs all four.
            (
                Platform::X86,
                ALL_X86,
                false,
                None,
                HardwareClass::Cpu,
                vec![],
            ),
            (
                Platform::X86,
                NONE,
                false,
                None,
                HardwareClass::No,
                vec!["avx2", "fma", "f16c", "bmi2"],
            ),
            (
                Platform::X86,
                CpuFeatures {
                    bmi2: false,
                    ..ALL_X86
                },
                false,
                None,
                HardwareClass::No,
                vec!["bmi2"],
            ),
            (
                Platform::X86,
                CpuFeatures {
                    avx2: false,
                    f16c: false,
                    ..ALL_X86
                },
                false,
                Some(14),
                HardwareClass::No,
                vec!["avx2", "f16c"],
            ),
            // Under Rosetta, CPUID hides AVX2: from macOS 15 the parent advertises it.
            (
                Platform::X86,
                NONE,
                true,
                Some(15),
                HardwareClass::Cpu,
                vec!["avx2", "fma", "f16c", "bmi2"],
            ),
            (
                Platform::X86,
                NONE,
                true,
                Some(26),
                HardwareClass::Cpu,
                vec!["avx2", "fma", "f16c", "bmi2"],
            ),
            // Before 15 Rosetta cannot run AVX2 at all.
            (
                Platform::X86,
                NONE,
                true,
                Some(14),
                HardwareClass::No,
                vec!["avx2", "fma", "f16c", "bmi2"],
            ),
            (
                Platform::X86,
                NONE,
                true,
                None,
                HardwareClass::No,
                vec!["avx2", "fma", "f16c", "bmi2"],
            ),
            // Translated with ROSETTA_ADVERTISE_AVX=1 already in the environment.
            (
                Platform::X86,
                ALL_X86,
                true,
                Some(15),
                HardwareClass::Cpu,
                vec![],
            ),
            // A target the engine is not built for.
            (
                Platform::Other,
                ALL_X86,
                false,
                None,
                HardwareClass::No,
                vec![],
            ),
        ];
        for (platform, features, rosetta, macos, class, missing) in rows {
            let report = classify_for(platform, &features, rosetta, macos);
            assert_eq!(
                (report.class, report.missing.clone()),
                (class, missing),
                "{platform:?} {features:?} rosetta={rosetta} macos={macos:?}"
            );
            assert_eq!(report.rosetta, rosetta);
        }
    }

    #[test]
    fn sentences_use_the_catalog_text() {
        let refused = classify_for(Platform::X86, &NONE, false, None);
        assert_eq!(
            sentence(&refused).as_deref(),
            Some("This processor does not support the instructions local dictation needs (AVX2, FMA, F16C and BMI2). Your system's own dictation still works, without your Biorouter dictionary.")
        );

        let arm = classify_for(Platform::OtherArm, &NONE, false, None);
        assert_eq!(
            sentence(&arm).as_deref(),
            Some("This processor does not support the instructions local dictation needs (dotprod). Your system's own dictation still works, without your Biorouter dictionary.")
        );

        let old_rosetta = classify_for(Platform::X86, &NONE, true, Some(14));
        assert_eq!(sentence(&old_rosetta).as_deref(), Some(MSG_CPU_ROSETTA));
        assert_eq!(vm_sentence(&old_rosetta), None);

        let supported = classify_for(Platform::X86, &ALL_X86, false, None);
        assert_eq!(sentence(&supported), None);
        let new_rosetta = classify_for(Platform::X86, &NONE, true, Some(15));
        assert_eq!(sentence(&new_rosetta), None);

        let mut vm = refused.clone();
        vm.hypervisor = true;
        assert_eq!(vm_sentence(&vm), Some(MSG_CPU_VM));
        assert_eq!(vm_sentence(&refused), None);
    }

    #[test]
    fn display_list_wording() {
        assert_eq!(display_list(&[]), "");
        assert_eq!(display_list(&["avx2"]), "AVX2");
        assert_eq!(display_list(&["avx2", "fma"]), "AVX2 and FMA");
        assert_eq!(display_list(&["avx2", "fma", "f16c"]), "AVX2, FMA and F16C");
        assert_eq!(
            display_list(&["avx2", "fma", "f16c", "bmi2"]),
            "AVX2, FMA, F16C and BMI2"
        );
        assert_eq!(display_list(&["dotprod"]), "dotprod");
    }

    #[test]
    fn this_host_is_supported() {
        // Every machine that runs this crate's tests is one the engine supports: the
        // three CI legs and the developers' Macs.
        let report = host_report();
        assert_ne!(report.class, HardwareClass::No, "{report:?}");
        if cfg!(target_os = "macos") {
            assert!(macos_major().is_some_and(|v| v >= 12));
        }
    }
}
