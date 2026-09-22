//! A TCP port for a daemon a test starts, that no other process can be handed
//! between the moment the test chooses it and the moment the daemon binds it.
//!
//! The lifecycle tests used to bind port 0 and release the listener. That puts
//! the port straight back in the kernel's ephemeral pool, which every `bind(0)`
//! and every outgoing `connect()` on the machine draws from — macOS hands the
//! same port out again after one pass through its pool (measured 2026-09-22:
//! 16361 calls, 0.4 s) — and a loaded machine gets through that while a debug
//! `serve` is still starting. When it did, `serve` refused with "port N on
//! 127.0.0.1 is already in use" (2 runs in 24). Holding the listener until the
//! spawn does not close the window: the daemon binds later still.
//!
//! So the port comes from OUTSIDE the range this kernel assigns on its own, and
//! nothing gets it without asking for it by number. The ones that would ask —
//! another test here, the sibling binary, another checkout's run — are kept
//! off it by a UDP socket bound to the same number for as long as the
//! reservation lives. TCP and UDP ports are separate, so that lock is not in
//! the daemon's way; a UDP bind without `SO_REUSEADDR` refuses every other
//! bind of that address, whatever flags it sets; and the kernel releases it if
//! the test dies.
//!
//! ⚠ What this does NOT close: an unrelated program binding this exact TCP port
//! by number in those seconds. That is a real conflict and `serve` reports it
//! as one. It is rare, not impossible: on a busy development machine a few
//! starts in one short stretch failed with "already in use" on reserved-band
//! ports, with no holder identified afterwards, and none failed in the ~2,000
//! starts after it. Closing it needs the daemon to bind port 0 and report the
//! port, which `serve` cannot do today.
//!
//! On a kernel that assigns every port from 10000 up on its own there is
//! nothing to reserve, so [`reserve`] says so on stderr and falls back to the
//! old release-and-rebind port, with its old window.

use std::net::{Ipv4Addr, TcpListener, UdpSocket};
use std::ops::RangeInclusive;
use std::sync::atomic::{AtomicUsize, Ordering};

/// How many reservations this process has made, so each starts its search
/// somewhere new: a port a previous test in this process just released (and
/// whose daemon may still be shutting down) is the last one offered, not the
/// first.
static RESERVATIONS: AtomicUsize = AtomicUsize::new(0);

/// A port nothing is listening on, reserved until this is dropped.
pub struct ReservedPort {
    pub port: u16,
    _lock: UdpSocket,
}

pub fn reserve() -> ReservedPort {
    let ephemeral = ephemeral_range();
    // Clear of the low ports well-known services use, and of the kernel's pool.
    let candidates: Vec<u16> = (10_000..=u16::MAX)
        .filter(|port| !ephemeral.contains(port))
        .collect();
    if candidates.is_empty() {
        eprintln!(
            "reserved_port: this kernel assigns every port from {} to {} on its own, \
             so none can be reserved race-free; falling back to a released port \
             (narrow the ephemeral range to close that window)",
            ephemeral.start(),
            ephemeral.end()
        );
        return released_port();
    }
    // Start somewhere different in each process, so concurrent runs do not all
    // queue for the same few locks, and somewhere new for each reservation in
    // this process. Correctness does not depend on it.
    let nth = RESERVATIONS.fetch_add(1, Ordering::Relaxed);
    let start =
        (std::process::id() as usize).wrapping_add(nth.wrapping_mul(7919)) % candidates.len();
    for &port in candidates[start..].iter().chain(&candidates[..start]) {
        let Ok(lock) = UdpSocket::bind((Ipv4Addr::LOCALHOST, port)) else {
            continue;
        };
        // Bound the way `serve`'s own pre-flight and the daemon bind it (with
        // SO_REUSEADDR), so a port this accepts is one they accept.
        if TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok() {
            return ReservedPort { port, _lock: lock };
        }
    }
    panic!(
        "no port outside the kernel's ephemeral range ({}-{}) is free",
        ephemeral.start(),
        ephemeral.end()
    );
}

/// The old behaviour, for a kernel with no port outside its ephemeral range:
/// a port the kernel just handed out and took back, locked the same way.
fn released_port() -> ReservedPort {
    loop {
        let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .and_then(|l| l.local_addr())
            .expect("bind a loopback port")
            .port();
        if let Ok(lock) = UdpSocket::bind((Ipv4Addr::LOCALHOST, port)) {
            return ReservedPort { port, _lock: lock };
        }
    }
}

/// The ports this kernel hands out to `bind(0)` and `connect()`.
#[cfg(target_os = "linux")]
fn ephemeral_range() -> RangeInclusive<u16> {
    let path = "/proc/sys/net/ipv4/ip_local_port_range";
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let bounds: Vec<u16> = text
        .split_whitespace()
        .filter_map(|n| n.parse().ok())
        .collect();
    match bounds[..] {
        [first, last] => first..=last,
        _ => panic!("{path} is not two ports: {text:?}"),
    }
}

/// The ports this kernel hands out to `bind(0)` and `connect()`: the default
/// range and the "high" one a socket can ask for, taken together.
#[cfg(not(target_os = "linux"))]
fn ephemeral_range() -> RangeInclusive<u16> {
    let names = [
        "net.inet.ip.portrange.first",
        "net.inet.ip.portrange.last",
        "net.inet.ip.portrange.hifirst",
        "net.inet.ip.portrange.hilast",
    ];
    let out = std::process::Command::new("/usr/sbin/sysctl")
        .arg("-n")
        .args(names)
        .output()
        .expect("run sysctl");
    let text = String::from_utf8_lossy(&out.stdout);
    let bounds: Vec<u16> = text
        .split_whitespace()
        .filter_map(|n| n.parse().ok())
        .collect();
    match bounds[..] {
        [first, last, hifirst, hilast] => first.min(hifirst)..=last.max(hilast),
        _ => panic!(
            "sysctl {} did not print four ports: {text:?}",
            names.join(" ")
        ),
    }
}
