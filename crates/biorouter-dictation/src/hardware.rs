//! How well each model suits this computer (DD-22, `docs/dictation/engine-and-models.md`,
//! "Suitability and bench").
//!
//! It never reuses the llama sidecar's `is_cpu_only()`, which answers false on Windows
//! because `llama-server` there is a Vulkan build, while the dictation engine is CPU only
//! on Windows. A person may insist on any model that is not `unsupported_cpu`; nothing
//! here ever picks a slow model for them.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::catalog::{Catalog, SttCatalogEntry};
use crate::cpu::{self, HardwareClass};

/// A recorded real time factor above this rates a model `too_slow`.
pub const TOO_SLOW_RTF: f64 = 0.5;
/// Without a bench record, a model other than the class default is `recommended` on a
/// CPU host only with this many physical cores.
pub const CORES_FOR_LARGE_ON_CPU: u32 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Suitability {
    Recommended,
    Suitable,
    TooSlow,
    BelowMinimum,
    UnsupportedCpu,
    Unknown,
}

impl Suitability {
    pub fn as_str(self) -> &'static str {
        match self {
            Suitability::Recommended => "recommended",
            Suitability::Suitable => "suitable",
            Suitability::TooSlow => "too_slow",
            Suitability::BelowMinimum => "below_minimum",
            Suitability::UnsupportedCpu => "unsupported_cpu",
            Suitability::Unknown => "unknown",
        }
    }
}

/// What the rules read about the host.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HostFacts {
    pub class: HardwareClass,
    pub total_ram_bytes: Option<u64>,
    /// Physical cores: hyperthreads do not run two encoders at full speed.
    pub physical_cores: Option<u32>,
}

impl HostFacts {
    /// This computer, read once per call (cheap: a sysctl or two files).
    pub fn current() -> HostFacts {
        HostFacts {
            class: cpu::host_report().class,
            total_ram_bytes: total_ram_bytes(),
            physical_cores: physical_cores(),
        }
    }
}

/// One `bench` result (`<local data>/dictation/bench.json`, keyed by model id).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BenchRecord {
    /// The timed decode's wall time divided by the clip's length.
    pub rtf: f64,
    pub elapsed_ms: u64,
    /// The clip's length.
    pub audio_ms: u64,
    pub threads: u32,
    pub backend: String,
    pub measured_at: String,
    /// The Biorouter version that measured it.
    pub version: String,
}

impl BenchRecord {
    /// A record from another version, backend or thread count says nothing about now.
    pub fn applies(&self, version: &str, backend: &str, threads: u32) -> bool {
        self.version == version && self.backend == backend && self.threads == threads
    }
}

pub type BenchFile = BTreeMap<String, BenchRecord>;

/// The bench records in `path`; empty when the file is absent or unreadable.
pub fn read_bench(path: &Path) -> BenchFile {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Adds or replaces `id`'s record in `path`, through a temporary file.
pub fn write_bench(path: &Path, id: &str, record: BenchRecord) -> std::io::Result<()> {
    let mut all = read_bench(path);
    all.insert(id.to_string(), record);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_vec_pretty(&all).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

/// DD-22, in the order `engine-and-models.md` lists: the processor, the RAM floor, a
/// recorded bench, then the class rules that hold until the host has run `bench`.
pub fn rate(
    catalog: &Catalog,
    entry: &SttCatalogEntry,
    host: &HostFacts,
    bench: Option<&BenchRecord>,
) -> Suitability {
    if host.class == HardwareClass::No {
        return Suitability::UnsupportedCpu;
    }
    if !entry.is_speech() {
        return Suitability::Suitable;
    }
    let Some(ram) = host.total_ram_bytes else {
        return Suitability::Unknown;
    };
    if ram < u64::from(entry.min_total_ram_gib) << 30 {
        return Suitability::BelowMinimum;
    }
    if let Some(bench) = bench {
        return if bench.rtf > TOO_SLOW_RTF {
            Suitability::TooSlow
        } else {
            Suitability::Recommended
        };
    }
    if catalog
        .default_for(host.class)
        .is_some_and(|default| default.id == entry.id)
    {
        return Suitability::Recommended;
    }
    match host.class {
        HardwareClass::Cpu => match host.physical_cores {
            Some(cores) if cores >= CORES_FOR_LARGE_ON_CPU => Suitability::Recommended,
            Some(_) => Suitability::TooSlow,
            None => Suitability::Unknown,
        },
        _ => Suitability::Suitable,
    }
}

/// The bench clip: `jfk.wav` from the vendored whisper.cpp samples (11.0 s, 352,078 B).
/// Speech, not a model; the one file the engine crate embeds (`no_model_is_embedded`).
#[cfg(all(feature = "engine", not(br_no_native)))]
pub const BENCH_WAV: &[u8] = include_bytes!("../../../vendor/whisper.cpp/source/samples/jfk.wav");

/// How much of the clip `bench` times: 5.5 s. The encoder always runs Whisper's whole
/// 30 s window, so a short utterance pays it nearly in full; the 11 s clip would halve
/// the real time factor of a slow model and rate turbo on 8 CPU threads (about 4.5 s
/// per utterance [M]) as fast enough. Dictation is mostly 4 to 6 s utterances.
pub const BENCH_CLIP_MS: u64 = 5_500;

/// The first `ms` of the bench clip as 16 kHz mono samples; `None` in a build without the
/// engine, which has nothing to time.
pub fn bench_clip(ms: u64) -> Option<Vec<i16>> {
    #[cfg(all(feature = "engine", not(br_no_native)))]
    {
        let wav = crate::audio::parse_wav_s16(BENCH_WAV).ok()?;
        let mut pcm = wav.to_16k_mono();
        pcm.truncate((ms * crate::audio::SAMPLES_PER_MS) as usize);
        Some(pcm)
    }
    #[cfg(not(all(feature = "engine", not(br_no_native))))]
    {
        let _ = ms;
        None
    }
}

#[cfg(target_os = "macos")]
fn sysctl_u64(name: &str) -> Option<u64> {
    let name = std::ffi::CString::new(name).ok()?;
    let mut value: u64 = 0;
    let mut size = std::mem::size_of::<u64>();
    // SAFETY: `value` is a u64 and `size` says so; sysctl writes at most `size` bytes.
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&mut value as *mut u64).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    match (rc, size) {
        (0, 8) => Some(value),
        (0, 4) => Some(value & 0xffff_ffff),
        _ => None,
    }
}

/// Total physical memory.
pub fn total_ram_bytes() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        sysctl_u64("hw.memsize")
    }
    #[cfg(target_os = "linux")]
    {
        let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
        meminfo_total(&meminfo)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        // SAFETY: a zeroed MEMORYSTATUSEX with its length set is what the call expects.
        let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        // SAFETY: `status` is a valid, initialised MEMORYSTATUSEX.
        (unsafe { GlobalMemoryStatusEx(&mut status) } != 0).then_some(status.ullTotalPhys)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        None
    }
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn meminfo_total(meminfo: &str) -> Option<u64> {
    let line = meminfo.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kib * 1024)
}

/// Physical cores, or the logical count when the system does not say.
pub fn physical_cores() -> Option<u32> {
    #[cfg(target_os = "macos")]
    let physical = sysctl_u64("hw.physicalcpu").map(|n| n as u32);
    #[cfg(target_os = "linux")]
    let physical = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|text| cpuinfo_cores(&text));
    #[cfg(windows)]
    let physical = windows_physical_cores();
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    let physical: Option<u32> = None;
    physical.filter(|n| *n > 0).or_else(|| {
        std::thread::available_parallelism()
            .ok()
            .map(|n| n.get() as u32)
    })
}

/// Distinct `(physical id, core id)` pairs; x86 Linux lists both for every logical CPU.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn cpuinfo_cores(cpuinfo: &str) -> Option<u32> {
    let mut cores = std::collections::BTreeSet::new();
    for block in cpuinfo.split("\n\n") {
        let field = |name: &str| {
            block
                .lines()
                .find(|l| l.split(':').next().is_some_and(|k| k.trim() == name))
                .and_then(|l| l.split(':').nth(1))
                .map(|v| v.trim().to_string())
        };
        if let (Some(package), Some(core)) = (field("physical id"), field("core id")) {
            cores.insert((package, core));
        }
    }
    (!cores.is_empty()).then_some(cores.len() as u32)
}

#[cfg(windows)]
fn windows_physical_cores() -> Option<u32> {
    use windows_sys::Win32::System::SystemInformation::{
        GetLogicalProcessorInformation, RelationProcessorCore, SYSTEM_LOGICAL_PROCESSOR_INFORMATION,
    };
    let mut bytes: u32 = 0;
    // SAFETY: a null buffer with length 0 asks for the length it needs.
    unsafe { GetLogicalProcessorInformation(std::ptr::null_mut(), &mut bytes) };
    let item = std::mem::size_of::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION>();
    if bytes == 0 || item == 0 {
        return None;
    }
    let count = bytes as usize / item;
    // SAFETY: the struct is plain data; zeroed values are valid before the call fills them.
    let mut buffer: Vec<SYSTEM_LOGICAL_PROCESSOR_INFORMATION> =
        (0..count).map(|_| unsafe { std::mem::zeroed() }).collect();
    // SAFETY: `buffer` holds `bytes` bytes of the struct the call writes.
    let ok = unsafe { GetLogicalProcessorInformation(buffer.as_mut_ptr(), &mut bytes) };
    if ok == 0 {
        return None;
    }
    Some(
        buffer
            .iter()
            .filter(|info| info.Relationship == RelationProcessorCore)
            .count() as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CATALOG, SMALL_EN_ID, TURBO_ID, VAD_ID};

    const GIB: u64 = 1 << 30;

    fn host(class: HardwareClass, ram_gib: u64, cores: u32) -> HostFacts {
        HostFacts {
            class,
            total_ram_bytes: Some(ram_gib * GIB),
            physical_cores: Some(cores),
        }
    }

    fn bench(rtf: f64) -> BenchRecord {
        BenchRecord {
            rtf,
            elapsed_ms: 1,
            audio_ms: BENCH_CLIP_MS,
            threads: 8,
            backend: "cpu".into(),
            measured_at: "2026-11-06T00:00:00Z".into(),
            version: "1.94.1".into(),
        }
    }

    fn rated(id: &str, facts: &HostFacts, record: Option<&BenchRecord>) -> Suitability {
        rate(&CATALOG, CATALOG.entry(id).unwrap(), facts, record)
    }

    /// DD-22's table, row by row.
    #[test]
    fn the_suitability_rules() {
        use Suitability::*;
        let apple = host(HardwareClass::As, 16, 10);
        assert_eq!(rated(TURBO_ID, &apple, None), Recommended);
        assert_eq!(rated(SMALL_EN_ID, &apple, None), Suitable);

        let laptop = host(HardwareClass::Cpu, 16, 4);
        assert_eq!(rated(SMALL_EN_ID, &laptop, None), Recommended);
        assert_eq!(rated(TURBO_ID, &laptop, None), TooSlow);
        let workstation = host(HardwareClass::Cpu, 64, 16);
        assert_eq!(rated(TURBO_ID, &workstation, None), Recommended);
        let fifteen = host(HardwareClass::Cpu, 64, 15);
        assert_eq!(rated(TURBO_ID, &fifteen, None), TooSlow);

        // RAM floors: turbo below 8 GiB, small.en below 4 GiB.
        let small_ram = HostFacts {
            total_ram_bytes: Some(8 * GIB - 1),
            ..apple
        };
        assert_eq!(rated(TURBO_ID, &small_ram, None), BelowMinimum);
        assert_eq!(rated(SMALL_EN_ID, &small_ram, None), Suitable);
        let tiny_ram = HostFacts {
            total_ram_bytes: Some(4 * GIB - 1),
            ..laptop
        };
        assert_eq!(rated(SMALL_EN_ID, &tiny_ram, None), BelowMinimum);

        // A recorded real time factor decides once it exists, either way.
        assert_eq!(rated(TURBO_ID, &laptop, Some(&bench(0.5))), Recommended);
        assert_eq!(rated(TURBO_ID, &laptop, Some(&bench(0.51))), TooSlow);
        assert_eq!(rated(TURBO_ID, &apple, Some(&bench(0.9))), TooSlow);
        assert_eq!(rated(SMALL_EN_ID, &apple, Some(&bench(0.02))), Recommended);

        // An unsupported processor rates every model the same.
        let old = host(HardwareClass::No, 64, 32);
        for entry in CATALOG.entries {
            assert_eq!(
                rate(&CATALOG, entry, &old, Some(&bench(0.01))),
                UnsupportedCpu
            );
        }
        // Facts that could not be read.
        let unknown = HostFacts {
            total_ram_bytes: None,
            ..laptop
        };
        assert_eq!(rated(SMALL_EN_ID, &unknown, None), Unknown);
        let no_cores = HostFacts {
            physical_cores: None,
            ..laptop
        };
        assert_eq!(rated(TURBO_ID, &no_cores, None), Unknown);
        assert_eq!(rated(VAD_ID, &laptop, None), Suitable);
    }

    #[test]
    fn a_bench_record_applies_only_to_its_own_conditions() {
        let record = bench(0.3);
        assert!(record.applies("1.94.1", "cpu", 8));
        assert!(!record.applies("1.94.2", "cpu", 8));
        assert!(!record.applies("1.94.1", "metal", 8));
        assert!(!record.applies("1.94.1", "cpu", 4));
    }

    #[test]
    fn bench_records_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dictation").join("bench.json");
        assert!(read_bench(&path).is_empty());
        write_bench(&path, TURBO_ID, bench(0.4)).unwrap();
        write_bench(&path, SMALL_EN_ID, bench(0.1)).unwrap();
        let all = read_bench(&path);
        assert_eq!(all.len(), 2);
        assert_eq!(all[TURBO_ID].rtf, 0.4);
        std::fs::write(&path, b"not json").unwrap();
        assert!(read_bench(&path).is_empty());
    }

    #[test]
    fn host_facts_parse() {
        assert_eq!(
            meminfo_total("MemTotal:       16314660 kB\nMemFree: 1 kB\n"),
            Some(16_314_660 * 1024)
        );
        assert_eq!(meminfo_total("MemFree: 1 kB\n"), None);
        let cpuinfo = "processor\t: 0\nphysical id\t: 0\ncore id\t: 0\n\n\
                       processor\t: 1\nphysical id\t: 0\ncore id\t: 0\n\n\
                       processor\t: 2\nphysical id\t: 0\ncore id\t: 1\n\n\
                       processor\t: 3\nphysical id\t: 1\ncore id\t: 0\n";
        assert_eq!(cpuinfo_cores(cpuinfo), Some(3));
        assert_eq!(cpuinfo_cores("processor\t: 0\nBogoMIPS\t: 50\n"), None);
        let facts = HostFacts::current();
        assert!(facts.physical_cores.is_some_and(|n| n >= 1));
        if cfg!(any(target_os = "macos", target_os = "linux", windows)) {
            assert!(facts.total_ram_bytes.is_some_and(|b| b > GIB));
        }
    }

    #[cfg(all(feature = "engine", not(br_no_native)))]
    #[test]
    fn the_bench_clip_is_the_pinned_sample_cut_to_5_5_s() {
        use sha2::{Digest, Sha256};
        let digest: String = Sha256::digest(BENCH_WAV)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(digest, crate::build_info::fixtures::JFK_SHA256);
        assert_eq!(bench_clip(BENCH_CLIP_MS).map(|pcm| pcm.len()), Some(88_000));
    }
}
