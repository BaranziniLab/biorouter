//! Whether a server an invitation or a route names is this machine (W2-DMN-2).
//!
//! A member who works on the server that runs the workspace (a terminal on an HPC login node)
//! still reaches it over SSH: the bridge identifies them by the peer account on the broker's
//! socket, and that account is the one SSH signs in as. Kept as the invitation wrote it, their
//! route went out through the host's jump host and back, and the jump host's settings refused it.
//! Knowing the server is this machine, the join plans `localhost` with no jump host instead.
//!
//! Only this machine's own names and addresses count: `localhost`, a loopback address, an
//! address on one of its interfaces, and its host name. A name is resolved only to compare its
//! addresses with those.
use std::net::IpAddr;
use std::time::Duration;

/// How long a server name may take to resolve before it is taken as not this machine.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(3);

/// Whether `host` (a name or an address, possibly `[bracketed]`) is this machine.
pub(super) async fn names_this_machine(host: &str) -> bool {
    let host = host
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(host);
    if host.is_empty() {
        return false;
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return is_this_machine_address(ip);
    }
    if host.eq_ignore_ascii_case("localhost") || is_this_machines_name(host) {
        return true;
    }
    let resolved = tokio::time::timeout(RESOLVE_TIMEOUT, tokio::net::lookup_host((host, 22))).await;
    match resolved {
        Ok(Ok(addresses)) => {
            let addresses: Vec<IpAddr> = addresses.map(|address| address.ip()).collect();
            !addresses.is_empty() && addresses.into_iter().all(is_this_machine_address)
        }
        _ => false,
    }
}

/// Whether `ip` is a loopback address or one of this machine's interface addresses.
pub(super) fn is_this_machine_address(ip: IpAddr) -> bool {
    ip.is_loopback() || interface_addresses().contains(&ip)
}

/// Whether `name` is this machine's host name, in full or up to its first dot.
fn is_this_machines_name(name: &str) -> bool {
    let Some(own) = host_name() else {
        return false;
    };
    let short = |value: &str| {
        value
            .split('.')
            .next()
            .unwrap_or(value)
            .to_ascii_lowercase()
    };
    own.eq_ignore_ascii_case(name) || (!name.contains('.') && short(&own) == short(name))
}

#[cfg(unix)]
fn host_name() -> Option<String> {
    let mut buffer = [0u8; 256];
    // SAFETY: the buffer is valid for `len` bytes, and gethostname writes at most that many.
    let status =
        unsafe { libc::gethostname(buffer.as_mut_ptr().cast::<libc::c_char>(), buffer.len()) };
    if status != 0 {
        return None;
    }
    let end = buffer.iter().position(|byte| *byte == 0)?;
    let name = std::str::from_utf8(buffer.get(..end)?).ok()?.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

#[cfg(not(unix))]
fn host_name() -> Option<String> {
    None
}

/// Every address on this machine's interfaces.
#[cfg(unix)]
fn interface_addresses() -> Vec<IpAddr> {
    let mut found = Vec::new();
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills `list` with a linked list it owns until freeifaddrs.
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return found;
    }
    let mut entry = list;
    while !entry.is_null() {
        // SAFETY: `entry` is a node of the list getifaddrs returned, not yet freed.
        let node = unsafe { &*entry };
        if !node.ifa_addr.is_null() {
            // SAFETY: `ifa_addr` points to a sockaddr whose family says how to read it.
            let family = i32::from(unsafe { (*node.ifa_addr).sa_family });
            if family == libc::AF_INET {
                // SAFETY: an AF_INET address is a sockaddr_in.
                let address = unsafe { &*(node.ifa_addr.cast::<libc::sockaddr_in>()) };
                found.push(IpAddr::from(
                    u32::from_be(address.sin_addr.s_addr).to_be_bytes(),
                ));
            } else if family == libc::AF_INET6 {
                // SAFETY: an AF_INET6 address is a sockaddr_in6.
                let address = unsafe { &*(node.ifa_addr.cast::<libc::sockaddr_in6>()) };
                found.push(IpAddr::from(address.sin6_addr.s6_addr));
            }
        }
        entry = node.ifa_next;
    }
    // SAFETY: `list` came from getifaddrs and is freed once.
    unsafe { libc::freeifaddrs(list) };
    found
}

#[cfg(not(unix))]
fn interface_addresses() -> Vec<IpAddr> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn this_machine_is_recognized_by_its_own_names_and_addresses() {
        for own in ["localhost", "127.0.0.1", "::1", "[::1]", "127.0.0.2"] {
            assert!(names_this_machine(own).await, "{own}");
        }
        if let Some(name) = host_name() {
            assert!(names_this_machine(&name).await, "{name}");
        }
        #[cfg(unix)]
        for address in interface_addresses() {
            assert!(is_this_machine_address(address), "{address}");
        }
        // Documentation addresses are never assigned to a real interface.
        for other in ["192.0.2.10", "2001:db8::1", "", "[]"] {
            assert!(!names_this_machine(other).await, "{other}");
        }
    }
}
