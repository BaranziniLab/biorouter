//! Is the process on the other end of this socket code we signed?
//!
//! # Why this exists
//!
//! The macOS prompt is raised by a separate helper (see
//! `biorouter-authprompt`), so its answer has to travel back over local IPC —
//! and a local channel can be written by anything running as the same user. The
//! first version used a nonce in a `0600` file, which raises the bar from
//! "guess a path" to "read the daemon's private directory" and no further: an
//! agent with shell access clears both, and the operations behind this prompt
//! include *turning privacy enforcement off*. A forged approval there is an
//! escalation, not a skipped dialog.
//!
//! So the daemon asks the **kernel** which process is connected
//! (`LOCAL_PEERPID`), and then asks **Security.framework** whether that process
//! is signed by our Developer ID team. Forging now requires a binary signed by
//! `F3YYBXAFJ8`, which a shell script is not.
//!
//! ⚠ **What this still does not stop.** Someone who can debug or inject into a
//! process we did sign, or who holds the signing key, defeats it. It is a
//! meaningful bar, not an unforgeable one — and Biorouter's privacy barrier is
//! documented as safety rather than security for exactly this class of reason.
//! Do not upgrade the claim in the docs on the strength of this file.
//!
//! ⚠ **PID reuse is why the check happens on a CONNECTED socket.** A pid
//! obtained any other way can be recycled between learning it and checking it;
//! `LOCAL_PEERPID` on a live connection names the process holding that end
//! right now, and the connection is kept open across the check.

#![cfg(target_os = "macos")]

use core_foundation::base::TCFType;
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use std::os::unix::io::AsRawFd;

/// The Developer ID team every shipped Biorouter binary is signed with.
///
/// ⚠ A literal, deliberately. Reading it from our own signature at runtime
/// would make an unsigned build "verify" against itself, which is precisely the
/// build an attacker would rather we ran.
const TEAM_ID: &str = "F3YYBXAFJ8";

/// The requirement the peer must satisfy: Apple-rooted chain, our team.
fn requirement_string() -> String {
    format!("anchor apple generic and certificate leaf[subject.OU] = \"{TEAM_ID}\"")
}

#[allow(non_camel_case_types)]
type OSStatus = i32;
const ERR_SEC_SUCCESS: OSStatus = 0;

#[link(name = "Security", kind = "framework")]
extern "C" {
    fn SecCodeCopyGuestWithAttributes(
        host: *const std::ffi::c_void,
        attributes: core_foundation_sys::dictionary::CFDictionaryRef,
        flags: u32,
        guest: *mut *const std::ffi::c_void,
    ) -> OSStatus;
    fn SecRequirementCreateWithString(
        text: core_foundation_sys::string::CFStringRef,
        flags: u32,
        requirement: *mut *const std::ffi::c_void,
    ) -> OSStatus;
    fn SecCodeCheckValidity(
        code: *const std::ffi::c_void,
        flags: u32,
        requirement: *const std::ffi::c_void,
    ) -> OSStatus;
    static kSecGuestAttributePid: core_foundation_sys::string::CFStringRef;
}

/// The pid at the far end of a connected unix socket, per the kernel.
fn peer_pid(stream: &std::os::unix::net::UnixStream) -> Option<i32> {
    let mut pid: libc::pid_t = 0;
    let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    // SAFETY: `LOCAL_PEERPID` writes a `pid_t` into `pid` and updates `len`;
    // both are owned here and correctly sized.
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_LOCAL,
            libc::LOCAL_PEERPID,
            &mut pid as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    (rc == 0 && pid > 0).then_some(pid)
}

/// Whether the process connected to `stream` is signed by our team.
///
/// Fails closed on every error: a peer we cannot identify, a signature we
/// cannot read, or a requirement we cannot compile all return `false`. An
/// answer we could not verify is not an approval.
pub fn peer_is_ours(stream: &std::os::unix::net::UnixStream) -> bool {
    let Some(pid) = peer_pid(stream) else {
        return false;
    };
    pid_is_ours(pid)
}

/// Whether the running process `pid` is signed by our team, by the same requirement and the
/// same fail-closed rules as [`peer_is_ours`]. The caller must know `pid` is still the process
/// it means: the Chrome import's key helper asks it of its own parent, which cannot be recycled
/// while the helper is that parent's child.
pub fn pid_is_ours(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // SAFETY block covers the whole Security.framework sequence below: each call
    // takes CF types we own and writes an out-pointer we own, and each returned
    // object is released before the function ends.
    unsafe {
        let key = CFString::wrap_under_get_rule(kSecGuestAttributePid);
        let value = CFNumber::from(pid);
        let attrs = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), value.as_CFType())]);

        let mut guest: *const std::ffi::c_void = std::ptr::null();
        if SecCodeCopyGuestWithAttributes(
            std::ptr::null(),
            attrs.as_concrete_TypeRef(),
            0,
            &mut guest,
        ) != ERR_SEC_SUCCESS
            || guest.is_null()
        {
            return false;
        }

        let text = CFString::new(&requirement_string());
        let mut requirement: *const std::ffi::c_void = std::ptr::null();
        let made = SecRequirementCreateWithString(text.as_concrete_TypeRef(), 0, &mut requirement)
            == ERR_SEC_SUCCESS
            && !requirement.is_null();
        if !made {
            core_foundation_sys::base::CFRelease(guest as _);
            return false;
        }

        let valid = SecCodeCheckValidity(guest, 0, requirement) == ERR_SEC_SUCCESS;

        core_foundation_sys::base::CFRelease(requirement as _);
        core_foundation_sys::base::CFRelease(guest as _);
        valid
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The requirement text is the whole security property, so it is pinned.
    /// A typo here does not fail loudly — it produces a requirement that either
    /// refuses everything (feature dead) or, worse, one that compiles to
    /// something weaker than intended.
    #[test]
    fn the_requirement_names_our_team_and_an_apple_anchor() {
        let req = requirement_string();
        assert!(req.contains("anchor apple generic"), "got {req}");
        assert!(req.contains("certificate leaf[subject.OU]"), "got {req}");
        assert!(req.contains(TEAM_ID), "got {req}");
    }

    /// ⚠ The team id must be a literal, never read from our own signature: a
    /// self-derived requirement would let an unsigned or differently-signed
    /// build verify against itself, which is exactly the build an attacker
    /// would prefer we ran.
    #[test]
    fn the_team_id_is_a_compile_time_literal() {
        let code: String = include_str!("system_auth_peer.rs")
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            code.contains("const TEAM_ID: &str = \"F3YYBXAFJ8\""),
            "TEAM_ID must stay a literal constant"
        );
        // ⚠ Assembled at runtime, not written as literals. Spelled out here
        // they would appear in this very file and the scan would match its own
        // assertion — the second time in this change that a guard quoting the
        // thing it forbids failed on correct code.
        for forbidden in [
            format!("SecCodeCopySigning{}", "Information"),
            format!("current{}exe", "_"),
        ] {
            assert!(
                !code.contains(&forbidden),
                "{forbidden} suggests the team id is being derived from our own binary"
            );
        }
    }

    /// A socket pair is not a signed peer relationship, but it does exercise
    /// `peer_pid` against a real connected socket — the half that would
    /// silently return `false` forever if the getsockopt arguments were wrong,
    /// making every prompt fail closed and look like the OS refusing.
    #[test]
    fn the_peer_pid_of_a_local_socket_is_this_process() {
        let (a, _b) = std::os::unix::net::UnixStream::pair().expect("socketpair");
        assert_eq!(
            peer_pid(&a),
            Some(std::process::id() as i32),
            "LOCAL_PEERPID did not name this process across a socketpair"
        );
    }
}
