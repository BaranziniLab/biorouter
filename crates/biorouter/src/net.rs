//! TCP sockets that a child process cannot inherit.
//!
//! Every production TCP listener in the workspace is bound through
//! [`bind_non_inheritable`], never with `tokio::net::TcpListener::bind`, and
//! every production outbound TCP connection (the daemon's tunnel websocket, the
//! CLI's requests to a daemon) dials through [`connect_non_inheritable`], never
//! with `tokio::net::TcpStream::connect`. `scripts/check-non-inheritable-sockets.sh`
//! (clippy's `disallowed_methods` at `--force-warn`, configured in
//! `scripts/clippy-non-inheritable/clippy.toml`) fails CI when production code
//! calls tokio's `bind` or `connect`, or the websocket `connect_async` family.

use std::ffi::c_int;
use std::io;
use std::net::SocketAddr;

use socket2::{Domain, Socket, Type};
use tokio::net::{lookup_host, TcpListener, TcpSocket, TcpStream, ToSocketAddrs};

// The listen backlog mio 1.1.1 passes on each platform, copied cfg for cfg from
// `src/sys/mod.rs` (`LISTEN_BACKLOG_SIZE`), so a listener bound here queues
// exactly as many pending connections as one bound by `tokio::net::TcpListener`.
// This is not the 1024 it is sometimes remembered as, and it is not `std`'s 128:
//   * Linux, FreeBSD, OpenBSD and Apple get `-1`, which the kernel replaces with
//     its own maximum (`net.core.somaxconn` on Linux, 4096 on kernels since 5.4;
//     `kern.ipc.somaxconn` on macOS, 128 by default).
//   * Windows gets 128, the same value `std::net::TcpListener::bind` uses.
// Binding through `std` and converting with `from_std` would also stop the
// inheritance, but it listens with 128 everywhere, which on Linux would quietly
// cut the daemon's accept queue from `somaxconn` to 128.
//
// A hand copy drifts silently, so `tests/listen_backlog_parity.rs`
// pins the mio version it was copied from (it fails when Cargo.lock resolves
// another) and compares this table, row by row, with that mio's own source in
// the cargo registry.
#[cfg(any(
    target_os = "windows",
    target_os = "redox",
    target_os = "espidf",
    target_os = "horizon"
))]
const LISTEN_BACKLOG: c_int = 128;

#[cfg(target_os = "hermit")]
const LISTEN_BACKLOG: c_int = 1024;

#[cfg(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "openbsd",
    target_vendor = "apple",
))]
const LISTEN_BACKLOG: c_int = -1;

#[cfg(not(any(
    target_os = "windows",
    target_os = "redox",
    target_os = "espidf",
    target_os = "horizon",
    target_os = "linux",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "wasi",
    target_os = "hermit",
    target_vendor = "apple",
)))]
const LISTEN_BACKLOG: c_int = libc::SOMAXCONN;

/// Bind a TCP listener that child processes do not inherit.
///
/// This is a drop-in replacement for `tokio::net::TcpListener::bind`. It takes
/// the same arguments and returns the same type. The only thing it changes is
/// the one flag that matters on Windows.
///
/// # Why
///
/// On Windows, `tokio::net::TcpListener::bind` creates a listening socket that
/// **every child process inherits**:
///
/// * tokio binds through mio, and mio 1.1.1 (`src/sys/windows/net.rs`,
///   `new_socket`) creates the socket with a plain `socket(domain, type, 0)`.
///   Microsoft's `WSASocketW` documentation: *"A socket handle created by the
///   WSASocket or the socket function is inheritable by default."*
/// * `std::process::Command`, and tokio's `Command` built on it, spawns with
///   `bInheritHandles = TRUE`.
///
/// So every child spawned while such a listener is open gets its own handle to
/// the listening socket. A socket stays open until its last handle closes, so
/// the port stays in the listening state after BioRouter closes the listener,
/// for as long as that child runs: connections to it still complete, and
/// nothing will ever answer them. `biorouterd` spawns MCP extensions, shells
/// and coding agents all the time, and some of them outlive it. On a fixed port
/// (the default 3000, `BIOROUTER_PORT`, `biorouter serve --port`), such a child
/// stops the next daemon from binding the port at all.
///
/// `socket2::Socket::new` (0.6.1, `src/socket.rs` `set_common_type`) adds
/// `WSA_FLAG_NO_HANDLE_INHERIT` on Windows by default. It adds `SOCK_CLOEXEC` on
/// Linux and the other BSD-socket platforms that support it. Apple has no
/// `SOCK_CLOEXEC`, so `set_common_flags` sets `FD_CLOEXEC` and `SO_NOSIGPIPE`
/// there, the same two flags mio sets. So on Unix the socket comes out the same
/// as tokio's, which was already close-on-exec. Only Windows changes.
///
/// # Faithful to tokio, step by step
///
/// Each step below matches tokio 1.49 `TcpListener::bind` and mio 1.1.1
/// `TcpListener::bind`, so no call site can tell the two apart except through
/// the inheritance flag:
///
/// 1. Address resolution goes through [`tokio::net::lookup_host`], which calls
///    the same `to_socket_addrs` that tokio's `bind` calls. Each resolved
///    address is tried in turn. The first that binds wins; if none binds, the
///    last error is returned, or tokio's own `InvalidInput` "could not resolve
///    to any address" when resolution returned nothing.
/// 2. The socket is `SOCK_STREAM` with protocol 0, as in mio, and non-blocking
///    before it is handed to tokio.
/// 3. `SO_REUSEADDR` is set on Unix and **not on Windows**, where it would let
///    another process bind a port this one is actively using ("socket
///    hijacking"). This matches mio and `std`.
/// 4. The backlog is mio's, per platform. See the constant above.
///
/// Like tokio's `bind`, this must be called from inside a tokio runtime.
pub async fn bind_non_inheritable<A: ToSocketAddrs>(addr: A) -> io::Result<TcpListener> {
    let addrs = lookup_host(addr).await?;

    let mut last_err = None;
    for addr in addrs {
        match bind_addr(addr) {
            Ok(listener) => return Ok(listener),
            Err(e) => last_err = Some(e),
        }
    }

    Err(last_err.unwrap_or_else(no_address))
}

/// tokio's own error when an address resolves to nothing, word for word.
fn no_address() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "could not resolve to any address",
    )
}

/// One address: mio's `TcpListener::bind`, with socket2 creating the socket.
fn bind_addr(addr: SocketAddr) -> io::Result<TcpListener> {
    let socket = Socket::new(Domain::for_address(addr), Type::STREAM, None)?;
    socket.set_nonblocking(true)?;
    #[cfg(not(windows))]
    socket.set_reuse_address(true)?;
    socket.bind(&addr.into())?;
    socket.listen(LISTEN_BACKLOG)?;
    TcpListener::from_std(socket.into())
}

/// Open a TCP connection that child processes do not inherit.
///
/// The drop-in replacement for `tokio::net::TcpStream::connect`: the same
/// arguments, the same return type, and one flag different on Windows.
///
/// # Why
///
/// The reason [`bind_non_inheritable`] exists, seen from the other end. tokio
/// dials through mio, and mio 1.1.1 creates the Windows socket with a plain
/// `socket()` call, so every child spawned while the connection is open gets
/// its own handle to it. Dropping the stream then closes nothing: the socket
/// stays open until the last handle closes, so no FIN reaches the peer for as
/// long as any such child runs, and the peer keeps a connection that nothing on
/// this side will ever read or write again. `biorouterd` spawns MCP servers,
/// shells and agents all day, and its tunnel websocket is one long-lived
/// connection: without this, a tunnel the daemon dropped (an idle reconnect, or
/// the daemon exiting) would stay open at the relay for as long as any child
/// spawned while it was up kept running. (Read from the mio source and Windows'
/// handle-inheritance rules, as for the listener; the Windows tests below are
/// where it is first run.)
///
/// # Faithful to tokio, step by step
///
/// tokio 1.49 `TcpStream::connect`, with the socket made by socket2:
///
/// 1. Address resolution goes through [`tokio::net::lookup_host`], tokio's own
///    `to_socket_addrs`. Each address is tried in turn; the first that connects
///    wins, and if none does the last error is returned, or tokio's own
///    `InvalidInput` "could not resolve to any address" when there was none.
/// 2. The socket is `SOCK_STREAM` with protocol 0, as mio creates it, and
///    non-blocking.
/// 3. The connect is tokio's own [`TcpSocket::connect`], which ends in the same
///    `connect_mio` that `TcpStream::connect` does: an in-progress connect
///    (`EINPROGRESS`, or `WouldBlock` on Windows) is waited out until the socket
///    is writable, and then `SO_ERROR` decides.
///
/// socket2 adds `WSA_FLAG_NO_HANDLE_INHERIT` on Windows and close-on-exec
/// everywhere else, which mio also sets, so only Windows changes. Like tokio's
/// `connect`, this must be called from inside a tokio runtime.
pub async fn connect_non_inheritable<A: ToSocketAddrs>(addr: A) -> io::Result<TcpStream> {
    let addrs = lookup_host(addr).await?;

    let mut last_err = None;
    for addr in addrs {
        match connect_addr(addr).await {
            Ok(stream) => return Ok(stream),
            Err(e) => last_err = Some(e),
        }
    }

    Err(last_err.unwrap_or_else(no_address))
}

/// One address: socket2 creates the socket, tokio's `TcpSocket` connects it.
async fn connect_addr(addr: SocketAddr) -> io::Result<TcpStream> {
    let socket = Socket::new(Domain::for_address(addr), Type::STREAM, None)?;
    socket.set_nonblocking(true)?;
    TcpSocket::from_std_stream(socket.into())
        .connect(addr)
        .await
}

#[cfg(test)]
mod tests {
    use super::isolated;
    use super::*;
    use std::net::{Ipv4Addr, TcpStream};
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A connection to `listener` gets through and carries bytes both ways.
    async fn round_trip(listener: TcpListener) {
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut byte = [0u8; 1];
            stream.read_exact(&mut byte).await.unwrap();
            stream.write_all(&[byte[0] + 1]).await.unwrap();
        });
        let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
        client.write_all(&[41]).await.unwrap();
        let mut reply = [0u8; 1];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply, [42]);
        server.await.unwrap();
    }

    /// Every argument shape the production call sites pass: a `SocketAddr` (the
    /// daemon, OAuth, signup, `biorouter web`), a `(host, port)` tuple and a
    /// `&String` (ACP's `--addr`).
    #[tokio::test]
    async fn binds_every_address_shape_the_call_sites_pass() {
        let listener = bind_non_inheritable(SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .unwrap();
        round_trip(listener).await;

        let listener = bind_non_inheritable((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        round_trip(listener).await;

        let listener = bind_non_inheritable(("127.0.0.1", 0)).await.unwrap();
        round_trip(listener).await;

        let addr = String::from("127.0.0.1:0");
        let listener = bind_non_inheritable(&addr).await.unwrap();
        round_trip(listener).await;
    }

    /// Takes the address from tokio's own resolver, including a name that
    /// needs a lookup, not only a literal.
    #[tokio::test]
    async fn resolves_a_host_name_the_way_tokio_does() {
        let listener = bind_non_inheritable("localhost:0").await.unwrap();
        assert!(listener.local_addr().unwrap().ip().is_loopback());
        round_trip(listener).await;
    }

    /// The error cases produce the same kind and message as tokio's `bind`.
    #[tokio::test]
    async fn fails_the_way_tokio_bind_fails() {
        // Nothing to try: tokio's own InvalidInput, word for word.
        let none: &[SocketAddr] = &[];
        let ours = bind_non_inheritable(none).await.unwrap_err();
        let theirs = TcpListener::bind(none).await.unwrap_err();
        assert_eq!(ours.kind(), theirs.kind());
        assert_eq!(ours.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(ours.to_string(), theirs.to_string());

        // A port another listener holds.
        let holder = bind_non_inheritable((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let taken = holder.local_addr().unwrap();
        let ours = bind_non_inheritable(taken).await.unwrap_err();
        let theirs = TcpListener::bind(taken).await.unwrap_err();
        assert_eq!(ours.kind(), io::ErrorKind::AddrInUse);
        assert_eq!(ours.kind(), theirs.kind());
    }

    /// Each resolved address is tried in turn. A taken first address does not
    /// stop the second from being bound.
    #[tokio::test]
    async fn tries_each_resolved_address_in_turn() {
        let holder = bind_non_inheritable((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let taken = holder.local_addr().unwrap();
        let free = SocketAddr::from(([127, 0, 0, 1], 0));
        let listener = bind_non_inheritable(&[taken, free][..]).await.unwrap();
        assert_ne!(listener.local_addr().unwrap().port(), taken.port());
    }

    /// The socket options tokio's own `bind` sets, read back from both
    /// listeners on this platform and compared, so "the same socket" is
    /// measured here rather than assumed.
    #[tokio::test]
    async fn sets_the_same_socket_options_as_tokio_bind() {
        let ours = bind_non_inheritable((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let theirs = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();

        let reuse = |l: &TcpListener| socket2::SockRef::from(l).reuse_address().unwrap();
        assert_eq!(reuse(&ours), reuse(&theirs));
        // mio and std: set on Unix, deliberately not on Windows.
        assert_eq!(reuse(&ours), cfg!(not(windows)));

        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let fd_flags = |l: &TcpListener| unsafe { libc::fcntl(l.as_raw_fd(), libc::F_GETFD) };
            let fl_flags = |l: &TcpListener| unsafe { libc::fcntl(l.as_raw_fd(), libc::F_GETFL) };
            assert_eq!(fd_flags(&ours) & libc::FD_CLOEXEC, libc::FD_CLOEXEC);
            assert_eq!(
                fd_flags(&ours) & libc::FD_CLOEXEC,
                fd_flags(&theirs) & libc::FD_CLOEXEC
            );
            assert_eq!(fl_flags(&ours) & libc::O_NONBLOCK, libc::O_NONBLOCK);
            assert_eq!(
                fl_flags(&ours) & libc::O_NONBLOCK,
                fl_flags(&theirs) & libc::O_NONBLOCK
            );
        }

        #[cfg(target_vendor = "apple")]
        {
            use std::os::fd::AsRawFd;
            let nosigpipe = |l: &TcpListener| {
                let mut value: libc::c_int = 0;
                let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
                let rc = unsafe {
                    libc::getsockopt(
                        l.as_raw_fd(),
                        libc::SOL_SOCKET,
                        libc::SO_NOSIGPIPE,
                        (&mut value as *mut libc::c_int).cast(),
                        &mut len,
                    )
                };
                assert_eq!(rc, 0, "getsockopt(SO_NOSIGPIPE) failed");
                value != 0
            };
            assert!(nosigpipe(&ours));
            assert_eq!(nosigpipe(&ours), nosigpipe(&theirs));
        }
    }

    // --- connect_non_inheritable -------------------------------------------

    /// A port that refuses: bound, then closed without ever listening.
    ///
    /// ⚠ Neither of the obvious ports works. A dropped LISTENER's port refuses
    /// only until another test's spawn holds a copy of the socket open (see
    /// `the_port_refuses_once_the_listener_is_dropped`). A port held by a bound
    /// socket that never listens refuses on Linux and Windows, but on macOS the
    /// SYN is dropped and the connect times out, measured at 7.8 s. A socket that
    /// never listened leaves no listening copy for a child to hold, so this port
    /// refuses unless another bind is handed this exact port in the next few
    /// milliseconds, out of the whole ephemeral range.
    fn refused_port() -> SocketAddr {
        let socket = Socket::new(Domain::IPV4, Type::STREAM, None).unwrap();
        socket
            .bind(&SocketAddr::from((Ipv4Addr::LOCALHOST, 0)).into())
            .unwrap();
        socket.local_addr().unwrap().as_socket().unwrap()
    }

    /// A stream from `connect_non_inheritable` to `listener` carries bytes both
    /// ways, whatever shape the address was given in.
    ///
    /// The connect is awaited before the accept, not beside it: the kernel
    /// completes the handshake into the listener's backlog either way, and a
    /// connect that fails must fail the test, not leave it waiting on an
    /// accept that nothing will ever satisfy (measured: joining the two hung
    /// for half an hour when the connect was broken on purpose).
    async fn connected_round_trip<A: ToSocketAddrs>(listener: &TcpListener, addr: A) {
        let mut client = connect_non_inheritable(addr).await.unwrap();
        let (mut server, _) = listener.accept().await.unwrap();
        assert_eq!(client.peer_addr().unwrap(), listener.local_addr().unwrap());
        client.write_all(&[41]).await.unwrap();
        let mut byte = [0u8; 1];
        server.read_exact(&mut byte).await.unwrap();
        server.write_all(&[byte[0] + 1]).await.unwrap();
        client.read_exact(&mut byte).await.unwrap();
        assert_eq!(byte, [42]);
    }

    /// Every shape tokio's `connect` takes: a `SocketAddr`, a `(host, port)`
    /// tuple, a `host:port` string, and a name that needs a lookup.
    #[tokio::test]
    async fn connects_to_every_address_shape_tokio_takes() {
        let listener = bind_non_inheritable((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        connected_round_trip(&listener, addr).await;
        connected_round_trip(&listener, ("127.0.0.1", addr.port())).await;
        connected_round_trip(&listener, format!("127.0.0.1:{}", addr.port())).await;
        // `localhost` may resolve to `::1` first, which nothing listens on here:
        // then this also shows the next address being tried.
        connected_round_trip(&listener, format!("localhost:{}", addr.port())).await;
    }

    /// The error cases produce the same kind and message as tokio's `connect`.
    #[tokio::test]
    async fn fails_the_way_tokio_connect_fails() {
        // Nothing to try: tokio's own InvalidInput, word for word.
        let none: &[SocketAddr] = &[];
        let ours = connect_non_inheritable(none).await.unwrap_err();
        let theirs = tokio::net::TcpStream::connect(none).await.unwrap_err();
        assert_eq!(ours.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(ours.kind(), theirs.kind());
        assert_eq!(ours.to_string(), theirs.to_string());

        // An address that does not parse.
        let ours = connect_non_inheritable("no port here").await.unwrap_err();
        let theirs = tokio::net::TcpStream::connect("no port here")
            .await
            .unwrap_err();
        assert_eq!(ours.kind(), theirs.kind());
        assert_eq!(ours.to_string(), theirs.to_string());

        // A port that refuses. Both connects at once: Windows reports a refused
        // loopback connect only after about two seconds of SYN retries, and
        // there is no reason to wait for it twice.
        let refusing = refused_port();
        let (ours, theirs) = tokio::join!(
            connect_non_inheritable(refusing),
            tokio::net::TcpStream::connect(refusing),
        );
        let (ours, theirs) = (ours.unwrap_err(), theirs.unwrap_err());
        assert_eq!(ours.kind(), io::ErrorKind::ConnectionRefused);
        assert_eq!(ours.kind(), theirs.kind());
        assert_eq!(ours.to_string(), theirs.to_string());
    }

    /// Each resolved address is tried in turn. A refusing first address does not
    /// stop the second from connecting.
    #[tokio::test]
    async fn connect_tries_each_resolved_address_in_turn() {
        let refusing = refused_port();
        let listener = bind_non_inheritable((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let good = listener.local_addr().unwrap();
        connected_round_trip(&listener, &[refusing, good][..]).await;
    }

    /// The socket options tokio's own `connect` leaves on a stream, read back
    /// from both and compared.
    #[tokio::test]
    async fn connect_sets_the_same_socket_options_as_tokio_connect() {
        let listener = bind_non_inheritable((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let ours = connect_non_inheritable(addr).await.unwrap();
        let theirs = tokio::net::TcpStream::connect(addr).await.unwrap();
        assert_eq!(ours.nodelay().unwrap(), theirs.nodelay().unwrap());
        assert_eq!(
            ours.local_addr().unwrap().ip(),
            theirs.local_addr().unwrap().ip()
        );

        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let fd_flags =
                |s: &tokio::net::TcpStream| unsafe { libc::fcntl(s.as_raw_fd(), libc::F_GETFD) };
            let fl_flags =
                |s: &tokio::net::TcpStream| unsafe { libc::fcntl(s.as_raw_fd(), libc::F_GETFL) };
            assert_eq!(fd_flags(&ours) & libc::FD_CLOEXEC, libc::FD_CLOEXEC);
            assert_eq!(
                fd_flags(&ours) & libc::FD_CLOEXEC,
                fd_flags(&theirs) & libc::FD_CLOEXEC
            );
            assert_eq!(fl_flags(&ours) & libc::O_NONBLOCK, libc::O_NONBLOCK);
            assert_eq!(
                fl_flags(&ours) & libc::O_NONBLOCK,
                fl_flags(&theirs) & libc::O_NONBLOCK
            );
        }

        #[cfg(target_vendor = "apple")]
        {
            use std::os::fd::AsRawFd;
            let nosigpipe = |s: &tokio::net::TcpStream| {
                let mut value: libc::c_int = 0;
                let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
                let rc = unsafe {
                    libc::getsockopt(
                        s.as_raw_fd(),
                        libc::SOL_SOCKET,
                        libc::SO_NOSIGPIPE,
                        (&mut value as *mut libc::c_int).cast(),
                        &mut len,
                    )
                };
                assert_eq!(rc, 0, "getsockopt(SO_NOSIGPIPE) failed");
                value != 0
            };
            assert!(nosigpipe(&ours));
            assert_eq!(nosigpipe(&ours), nosigpipe(&theirs));
        }
    }

    /// Dropping the listener really closes the port, with no child around.
    ///
    /// ⚠ "No child around" is only true in a process that spawns nothing, so
    /// this runs alone (see `isolated`). In the full test binary, any other
    /// test's spawn copies the descriptor table, and the copy holds the
    /// listening socket open until the new program's exec closes close-on-exec
    /// descriptors; a connect in that window succeeds. Measured on macOS: `net::`
    /// failed here in 17 of 80 runs with three spawning tests beside it, and in 0
    /// of 80 without them.
    #[test]
    fn the_port_refuses_once_the_listener_is_dropped() {
        isolated::assert_passes_alone(
            &isolated::test_name(module_path!(), "isolated_case_the_port_refuses"),
            &[],
            Duration::from_secs(120),
        );
    }

    /// Runs only in its isolated copy; in the normal run it returns at once.
    #[tokio::test]
    async fn isolated_case_the_port_refuses() {
        let Some(report) = isolated::child_report(&isolated::test_name(
            module_path!(),
            "isolated_case_the_port_refuses",
        )) else {
            return;
        };
        let listener = bind_non_inheritable((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let outcome = TcpStream::connect_timeout(&addr, Duration::from_secs(10));
        assert!(
            matches!(&outcome, Err(e) if e.kind() == io::ErrorKind::ConnectionRefused),
            "a dropped listener's port must refuse; got {outcome:?}"
        );
        report.passed();
    }
}

/// Runs one test case alone, in a fresh copy of this test binary, and reports
/// how it went.
///
/// # Why a separate process
///
/// The Windows tests below spawn a child with handle inheritance ON, which is
/// the point: they prove what such a child does and does not keep open. Spawned
/// inside the crowded `biorouter` lib test binary, that child would also
/// inherit every inheritable socket the tests running beside it hold (every
/// tokio listener is one, on Windows), and keep each open for as long as it
/// lives. A sibling that drops a listener and expects its port to refuse, or
/// waits for a peer's EOF, would stall or fail while it ran. So the case runs in
/// a copy of this binary that runs only that one case (`--exact <name>
/// --test-threads=1`), where the only sockets to inherit are the case's own.
///
/// The same crowd breaks a test from the other side, on every OS: a test that
/// drops a listener and expects its port to refuse fails whenever another test
/// spawns a process at that moment, because the new process holds a copy of
/// the listening socket until its exec closes close-on-exec descriptors. So
/// `the_port_refuses_once_the_listener_is_dropped` runs isolated too.
///
/// ⚠ **Re-running the case is not enough on its own, and the Windows spawn below
/// is why it works.** The copy is itself a child of the crowded binary. Started
/// with `std::process::Command`, which always passes `bInheritHandles = TRUE`
/// (`CommandExt::inherit_handles` is unstable in the pinned toolchain), it would
/// inherit exactly the siblings' sockets the isolation exists to keep away from
/// them, for exactly as long as the case runs. On Windows it is therefore
/// started with `CreateProcessW(bInheritHandles = FALSE)`. The cost is that no
/// pipe can reach it either, so it reports through a file it opens itself, by
/// path, and a panic is written there too.
///
/// A case runs only when the environment names it: anywhere else, including in
/// the normal run of the whole binary, it returns at once.
#[cfg(test)]
mod isolated {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    /// Set in the copy to the one case it is to run.
    const CASE_ENV: &str = "BIOROUTER_ISOLATED_TEST_CASE";
    /// Where the copy writes its verdict.
    const REPORT_ENV: &str = "BIOROUTER_ISOLATED_TEST_REPORT";
    const PASSED: &str = "passed";

    /// libtest's name for `function` in `module` (a `module_path!()`): the
    /// module path without the crate, which is what `--exact` matches.
    pub(super) fn test_name(module: &str, function: &str) -> String {
        match module.split_once("::") {
            Some((_, within_crate)) => format!("{within_crate}::{function}"),
            None => function.to_string(),
        }
    }

    /// In the copy started to run `case`, the report to finish it with.
    /// Anywhere else `None`, and the case must return at once.
    pub(super) fn child_report(case: &str) -> Option<Report> {
        if std::env::var_os(CASE_ENV)? != *case {
            return None;
        }
        let path = PathBuf::from(std::env::var_os(REPORT_ENV)?);
        // The copy runs this one case and nothing else, so a process-wide hook
        // affects no other test. On Windows the copy's stderr reaches no one,
        // so this is how an assertion message gets out.
        let hook_path = path.clone();
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = std::fs::write(&hook_path, format!("panicked: {info}"));
            default_hook(info);
        }));
        Some(Report(path))
    }

    /// Finishes an isolated case. Dropped without `passed`, the copy has not
    /// passed, whatever its exit code says.
    pub(super) struct Report(PathBuf);

    impl Report {
        /// Call last, after every guard in the case has run.
        pub(super) fn passed(self) {
            std::fs::write(&self.0, PASSED).expect("the isolated case must be able to report");
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    pub(super) enum Exit {
        Code(i32),
        /// Ended by a signal, with no exit code. Unix only: every Windows
        /// process has an exit code.
        #[cfg(unix)]
        NoCode,
        /// Still running at the deadline, and killed.
        TimedOut,
    }

    #[derive(Debug)]
    pub(super) struct Outcome {
        pub(super) exit: Exit,
        pub(super) report: Option<String>,
    }

    /// Run `case` alone in a fresh copy of this test binary. `extra` adds to the
    /// environment the copy starts with, which is otherwise this process's.
    pub(super) fn run(case: &str, extra: &[(&str, &str)], deadline: Duration) -> Outcome {
        let dir = tempfile::tempdir().expect("a scratch directory for the report");
        let report = dir.path().join("report");
        let exe = std::env::current_exe().expect("the test binary's own path");
        let mut env: Vec<(OsString, OsString)> = vec![
            (CASE_ENV.into(), case.into()),
            (REPORT_ENV.into(), report.clone().into_os_string()),
        ];
        env.extend(extra.iter().map(|(k, v)| ((*k).into(), (*v).into())));
        let args = ["--exact", case, "--test-threads=1", "--nocapture"];
        let exit = spawn_and_wait(&exe, &args, &env, deadline)
            .unwrap_or_else(|e| panic!("could not start the isolated copy for `{case}`: {e}"));
        Outcome {
            exit,
            report: std::fs::read_to_string(&report).ok(),
        }
    }

    /// Whether the copy ran `case` and it passed. A copy that exits 0 without a
    /// report ran no test by that name, which libtest does not treat as an
    /// error; it would otherwise pass while checking nothing.
    pub(super) fn verdict(case: &str, outcome: &Outcome) -> Result<(), String> {
        match (&outcome.exit, outcome.report.as_deref()) {
            (Exit::Code(0), Some(PASSED)) => Ok(()),
            (Exit::Code(0), None) => Err(format!(
                "the isolated copy exited 0 without running `{case}`: libtest found no \
                 test by that exact name, so nothing was checked"
            )),
            (exit, report) => Err(format!(
                "`{case}` did not pass in its isolated copy ({exit:?}): {}",
                report.unwrap_or("it wrote no report")
            )),
        }
    }

    /// Run `case` alone and fail the calling test unless it ran and passed.
    pub(super) fn assert_passes_alone(case: &str, extra: &[(&str, &str)], deadline: Duration) {
        let outcome = run(case, extra, deadline);
        if let Err(why) = verdict(case, &outcome) {
            panic!("{why}");
        }
    }

    /// Unix: `std`'s `Command`. Every descriptor std, tokio and mio create is
    /// close-on-exec, so the copy inherits nothing but its standard streams,
    /// which are all null.
    ///
    /// The deadline is kept without polling: a thread waits for the child to
    /// exit with `WNOWAIT`, which leaves it unreaped, so if the deadline passes
    /// first `Child::kill` still signals this child and never a process that
    /// has since been given its pid. The only reap is the `wait` below.
    #[cfg(unix)]
    fn spawn_and_wait(
        exe: &Path,
        args: &[&str],
        env: &[(OsString, OsString)],
        deadline: Duration,
    ) -> Result<Exit, String> {
        use std::process::{Command, Stdio};

        let mut child = Command::new(exe)
            .args(args)
            .envs(env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        let pid = child.id() as libc::id_t;
        let (exited, exit_seen) = std::sync::mpsc::channel();
        let watcher = std::thread::spawn(move || {
            let outcome = loop {
                // SAFETY: `info` is a valid, zeroed `siginfo_t` for the call.
                let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
                let rc = unsafe {
                    libc::waitid(libc::P_PID, pid, &mut info, libc::WEXITED | libc::WNOWAIT)
                };
                if rc == 0 {
                    break Ok(());
                }
                let error = std::io::Error::last_os_error();
                if error.kind() != std::io::ErrorKind::Interrupted {
                    break Err(error.to_string());
                }
            };
            let _ = exited.send(outcome);
        });
        let watched = exit_seen.recv_timeout(deadline);
        if !matches!(watched, Ok(Ok(()))) {
            // Past the deadline, or the watch failed: stop the child either way,
            // so the `wait` below cannot block for longer than the deadline.
            let _ = child.kill();
        }
        let status = child.wait().map_err(|e| e.to_string());
        let _ = watcher.join();
        match watched {
            Ok(Ok(())) => Ok(status?.code().map_or(Exit::NoCode, Exit::Code)),
            Ok(Err(e)) => Err(format!("could not watch the isolated copy: {e}")),
            Err(_) => Ok(Exit::TimedOut),
        }
    }

    /// Windows: `CreateProcessW` with `bInheritHandles = FALSE`, so the copy
    /// holds no handle of this process's at all. `CREATE_NO_WINDOW` gives it a
    /// console of its own that nobody sees, so the `ping` it starts attaches to
    /// that rather than opening a window, and the environment block is this
    /// process's plus `env`.
    #[cfg(windows)]
    fn spawn_and_wait(
        exe: &Path,
        args: &[&str],
        env: &[(OsString, OsString)],
        deadline: Duration,
    ) -> Result<Exit, String> {
        use std::collections::BTreeMap;
        use std::os::windows::ffi::OsStrExt;
        use windows::core::{PCWSTR, PWSTR};
        use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
        use windows::Win32::System::Threading::{
            CreateProcessW, GetExitCodeProcess, TerminateProcess, WaitForSingleObject,
            CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, INFINITE, PROCESS_INFORMATION,
            STARTUPINFOW,
        };

        let application: Vec<u16> = exe.as_os_str().encode_wide().chain([0]).collect();

        // The program quoted (a Windows path cannot contain `"`), then arguments
        // that need no quoting, which is checked rather than assumed.
        let mut command_line = format!("\"{}\"", exe.display());
        for arg in args {
            if arg.is_empty() || arg.contains([' ', '\t', '"']) {
                return Err(format!("argument {arg:?} would need quoting"));
            }
            command_line.push(' ');
            command_line.push_str(arg);
        }
        let mut command_line: Vec<u16> = command_line.encode_utf16().chain([0]).collect();

        // Names compare case-insensitively on Windows, and the block must be
        // sorted by name, so key by the upper-cased name.
        let key = |name: &std::ffi::OsStr| name.to_string_lossy().to_uppercase();
        let mut vars: BTreeMap<String, (OsString, OsString)> = std::env::vars_os()
            .map(|(k, v)| (key(&k), (k, v)))
            .collect();
        for (k, v) in env {
            vars.insert(key(k), (k.clone(), v.clone()));
        }
        let mut block: Vec<u16> = Vec::new();
        for (k, v) in vars.values() {
            block.extend(k.encode_wide());
            block.push(u16::from(b'='));
            block.extend(v.encode_wide());
            block.push(0);
        }
        block.push(0);

        let startup = STARTUPINFOW {
            cb: std::mem::size_of::<STARTUPINFOW>() as u32,
            ..Default::default()
        };
        let mut info = PROCESS_INFORMATION::default();
        // SAFETY: every pointer is to a live, NUL-terminated buffer owned by this
        // frame; `command_line` is mutable, as `CreateProcessW` requires; the
        // handles it returns are closed below on every path.
        unsafe {
            CreateProcessW(
                PCWSTR(application.as_ptr()),
                Some(PWSTR(command_line.as_mut_ptr())),
                None,
                None,
                false,
                CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
                Some(block.as_ptr().cast()),
                PCWSTR::null(),
                &startup,
                &mut info,
            )
        }
        .map_err(|e| e.to_string())?;
        // SAFETY: `info` holds two handles `CreateProcessW` just opened for us.
        unsafe {
            let _ = CloseHandle(info.hThread);
        }
        let millis = u32::try_from(deadline.as_millis()).unwrap_or(INFINITE - 1);
        // SAFETY: `info.hProcess` is open until the `CloseHandle` below.
        let exit = unsafe {
            if WaitForSingleObject(info.hProcess, millis) == WAIT_OBJECT_0 {
                let mut code = 0u32;
                GetExitCodeProcess(info.hProcess, &mut code)
                    .map(|()| Exit::Code(code as i32))
                    .map_err(|e| e.to_string())
            } else {
                let _ = TerminateProcess(info.hProcess, 1);
                WaitForSingleObject(info.hProcess, INFINITE);
                Ok(Exit::TimedOut)
            }
        };
        // SAFETY: as above; closed exactly once.
        unsafe {
            let _ = CloseHandle(info.hProcess);
        }
        exit
    }
}

/// The isolation harness itself, on the platforms where it can run here.
///
/// The Windows tests below are the reason it exists, and they can only run on
/// Windows. These prove the plumbing they depend on somewhere it can be run on
/// every push: the case really runs in another process, alone, and the harness
/// fails when the case fails and when no case by that name ran at all. What
/// these cannot prove is the Windows half: that `CreateProcessW` is called
/// correctly and that the copy then holds no inherited handle. That runs first
/// on `windows-latest`.
#[cfg(all(test, unix))]
mod isolation_tests {
    use super::isolated;
    use std::time::Duration;

    const DEADLINE: Duration = Duration::from_secs(120);
    const PARENT_ENV: &str = "BIOROUTER_ISOLATED_TEST_PARENT_PID";

    fn case(function: &str) -> String {
        isolated::test_name(module_path!(), function)
    }

    #[test]
    fn a_case_runs_alone_in_a_new_process_and_passes() {
        let parent = std::process::id().to_string();
        isolated::assert_passes_alone(
            &case("isolated_case_that_checks_where_it_runs"),
            &[(PARENT_ENV, &parent)],
            DEADLINE,
        );
    }

    #[test]
    fn a_failing_case_fails_with_its_own_message() {
        let name = case("isolated_case_that_fails");
        let outcome = isolated::run(&name, &[], DEADLINE);
        let why = isolated::verdict(&name, &outcome).expect_err("a failing case must not pass");
        assert!(
            why.contains("the deliberate failure") && outcome.exit != isolated::Exit::Code(0),
            "the verdict must carry the case's panic message and a failing exit: {why} / {outcome:?}"
        );
    }

    #[test]
    fn a_case_that_does_not_exist_is_not_a_pass() {
        let name = case("no_case_has_this_name");
        let outcome = isolated::run(&name, &[], DEADLINE);
        assert_eq!(
            outcome.exit,
            isolated::Exit::Code(0),
            "libtest exits 0 on no match"
        );
        let why = isolated::verdict(&name, &outcome).expect_err("no test ran, so nothing passed");
        assert!(why.contains("without running"), "{why}");
    }

    /// Runs only in its isolated copy; in the normal run it returns at once.
    #[test]
    fn isolated_case_that_checks_where_it_runs() {
        let Some(report) = isolated::child_report(&case("isolated_case_that_checks_where_it_runs"))
        else {
            return;
        };
        let parent: u32 = std::env::var(PARENT_ENV)
            .expect("the parent names itself")
            .parse()
            .unwrap();
        assert_ne!(
            std::process::id(),
            parent,
            "the case must run in a new process"
        );
        assert_eq!(
            std::os::unix::process::parent_id(),
            parent,
            "the new process must be the test binary's own child"
        );
        let args: Vec<String> = std::env::args().collect();
        assert!(
            args.windows(2)
                .any(|w| w[0] == "--exact"
                    && w[1] == case("isolated_case_that_checks_where_it_runs"))
                && args.iter().any(|a| a == "--test-threads=1"),
            "the copy must be told to run this one case and nothing else: {args:?}"
        );
        report.passed();
    }

    /// Runs only in its isolated copy, where it fails on purpose; in the normal
    /// run it returns at once, so this binary still passes.
    #[test]
    fn isolated_case_that_fails() {
        let Some(_report) = isolated::child_report(&case("isolated_case_that_fails")) else {
            return;
        };
        panic!("the deliberate failure");
    }
}

/// The property this module exists for, on the only OS where it differs: for a
/// listener (the port stays bound) and for a connection (no FIN is sent).
///
/// These tests cannot run on macOS or Linux, where the socket was already
/// close-on-exec. `rust.yml` runs them in the `test` job on `windows-latest`
/// (`cargo test --workspace --lib --bins`).
///
/// Each property is a pair: an outer `#[test]` that runs an inner case alone in
/// a fresh copy of this binary (see `isolated`), and the inner case, which does
/// the work there and returns at once anywhere else. The inner case spawns a
/// `ping` that inherits handles, so it must not run beside the other tests in
/// this binary: the `ping` would inherit their sockets too.
///
/// ⚠ One residual the isolation narrows but cannot close. After the listener is
/// dropped, Windows takes about two seconds to report the refused connection
/// (it retries the SYN). A port-0 bind made in that window may be handed the
/// same port. In the isolated copy nothing else binds, so no other test in this
/// binary can be; another process on the machine still can, because ephemeral
/// ports are allocated machine-wide. The fix case would then fail loudly
/// (connected, expected refused); the control could pass on the stranger's
/// listener. Nothing observed so far; it is the reason a single control failure
/// here is worth a re-run before a re-read, and a fix-case failure is not.
#[cfg(all(test, windows))]
mod windows_inheritance_tests {
    use super::*;
    use std::net::{Ipv4Addr, TcpStream};
    use std::process::{Child, Command, Stdio};
    use std::time::Duration;

    /// Long enough for a copy of this binary to start and run one case.
    const DEADLINE: Duration = Duration::from_secs(120);

    /// How long a probe may wait for the refused connection. Windows reports it
    /// after about two seconds of SYN retries; twice that is the margin.
    const PROBE_TIMEOUT: Duration = Duration::from_secs(4);

    fn case(function: &str) -> String {
        isolated::test_name(module_path!(), function)
    }

    /// Kills and reaps the child however the case ends, including when an
    /// assertion fails, so a failed run leaves no `ping` holding a port.
    struct Reaped(Child);

    impl Reaped {
        fn is_running(&mut self) -> bool {
            self.0
                .try_wait()
                .expect("try_wait on the child must succeed")
                .is_none()
        }
    }

    impl Drop for Reaped {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// A child that inherits handles and outlives the probe.
    /// `std::process::Command` spawns with `bInheritHandles = TRUE`, and
    /// redirecting stdio does not change that. `ping -n 7` runs for about six
    /// seconds: the probe's four, plus the time to start. It is killed as soon
    /// as the case has its answer, and the case checks that it was still
    /// running after the probe, so a probe that outlasted it fails rather than
    /// proving nothing.
    fn spawn_inheriting_child() -> Reaped {
        let child = Command::new("ping")
            .args(["-n", "7", "127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("ping.exe is on PATH on every Windows runner");
        Reaped(child)
    }

    /// Open `listener`, spawn a child, close the listener, and connect to its
    /// port while the child is provably still running.
    fn connect_after_close_with_child_alive(listener: TcpListener) -> io::Result<TcpStream> {
        let addr = listener.local_addr().unwrap();
        let mut child = spawn_inheriting_child();
        drop(listener);
        assert!(
            child.is_running(),
            "the child must be alive when the port is probed"
        );
        let outcome = TcpStream::connect_timeout(&addr, PROBE_TIMEOUT);
        assert!(
            child.is_running(),
            "the child must still be alive after the probe, or the probe proves nothing"
        );
        outcome
    }

    /// The fix. Without it, `ping` would hold its own handle to the listening
    /// socket, the port would stay open after the listener was dropped, and the
    /// connect in the isolated case would SUCCEED.
    #[test]
    fn a_child_spawned_while_the_listener_is_open_does_not_keep_its_port() {
        isolated::assert_passes_alone(&case("isolated_case_the_helper_listener"), &[], DEADLINE);
    }

    /// Runs only in its isolated copy; in the normal run it returns at once.
    #[tokio::test]
    async fn isolated_case_the_helper_listener() {
        let Some(report) = isolated::child_report(&case("isolated_case_the_helper_listener"))
        else {
            return;
        };
        let listener = bind_non_inheritable((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let outcome = connect_after_close_with_child_alive(listener);
        assert!(
            matches!(&outcome, Err(e) if e.kind() == io::ErrorKind::ConnectionRefused),
            "the listener was dropped, so its port must refuse even while a child \
             spawned beside it is still running; got {outcome:?}. A successful \
             connect means the child inherited the listening socket."
        );
        report.passed();
    }

    /// The control, which makes the test above mean something. The same steps
    /// with tokio's own `bind` must leave the port open, because the child
    /// inherited it. If this ever fails, mio has stopped creating inheritable
    /// sockets. The helper is then harmless but no longer needed, and this
    /// module's premise should be re-read, not the assertion flipped.
    #[test]
    fn control_a_tokio_bound_listener_is_kept_open_by_the_child() {
        isolated::assert_passes_alone(&case("isolated_case_a_tokio_listener"), &[], DEADLINE);
    }

    /// Runs only in its isolated copy; in the normal run it returns at once.
    #[tokio::test]
    async fn isolated_case_a_tokio_listener() {
        let Some(report) = isolated::child_report(&case("isolated_case_a_tokio_listener")) else {
            return;
        };
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let outcome = connect_after_close_with_child_alive(listener);
        assert!(
            outcome.is_ok(),
            "tokio's bind makes an inheritable socket on Windows (mio 1.1.1 \
             `new_socket` calls plain `socket()`), so the child should have kept \
             the port listening after the listener was dropped; got {outcome:?}"
        );
        // Closed before the report, so the case holds nothing when it says so.
        drop(outcome);
        report.passed();
    }

    /// Accept `client`'s connection on `listener`, spawn a child, drop the
    /// client, and read the server's end while the child is provably still
    /// running. `Ok(0)` is the peer's FIN: the client's socket really closed.
    fn read_after_close_with_child_alive(
        listener: std::net::TcpListener,
        client: tokio::net::TcpStream,
    ) -> io::Result<usize> {
        use std::io::Read;
        let (mut server, _) = listener.accept().unwrap();
        server.set_read_timeout(Some(PROBE_TIMEOUT)).unwrap();
        let mut child = spawn_inheriting_child();
        drop(client);
        assert!(
            child.is_running(),
            "the child must be alive when the connection is probed"
        );
        let mut byte = [0u8; 1];
        let outcome = server.read(&mut byte);
        assert!(
            child.is_running(),
            "the child must still be alive after the probe, or the probe proves nothing"
        );
        outcome
    }

    /// `std`'s listener, which no child inherits, so only the client's socket
    /// is under test.
    fn std_listener() -> (std::net::TcpListener, std::net::SocketAddr) {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        (listener, addr)
    }

    /// The fix, from the connecting side: the tunnel's websocket dials through
    /// `connect_non_inheritable`. Without it, `ping` would hold its own handle to
    /// the client socket, dropping the stream would send nothing, and the read
    /// in the isolated case would time out instead of seeing EOF.
    #[test]
    fn a_child_spawned_while_a_connection_is_open_does_not_hold_it_open() {
        isolated::assert_passes_alone(&case("isolated_case_the_helper_connection"), &[], DEADLINE);
    }

    /// Runs only in its isolated copy; in the normal run it returns at once.
    #[tokio::test]
    async fn isolated_case_the_helper_connection() {
        let Some(report) = isolated::child_report(&case("isolated_case_the_helper_connection"))
        else {
            return;
        };
        let (listener, addr) = std_listener();
        let client = connect_non_inheritable(addr).await.unwrap();
        let outcome = read_after_close_with_child_alive(listener, client);
        assert!(
            matches!(&outcome, Ok(0)),
            "the client was dropped, so its peer must read EOF even while a child \
             spawned beside it is still running; got {outcome:?}. A timeout means the \
             child inherited the client socket and is holding it open."
        );
        report.passed();
    }

    /// The control: the same steps with tokio's own `connect` see no EOF while
    /// the child runs, because the child inherited the socket. If this ever
    /// fails, mio has stopped creating inheritable sockets; re-read this
    /// module's premise rather than flipping the assertion.
    #[test]
    fn control_a_tokio_connection_is_held_open_by_the_child() {
        isolated::assert_passes_alone(&case("isolated_case_a_tokio_connection"), &[], DEADLINE);
    }

    /// Runs only in its isolated copy; in the normal run it returns at once.
    #[tokio::test]
    async fn isolated_case_a_tokio_connection() {
        let Some(report) = isolated::child_report(&case("isolated_case_a_tokio_connection")) else {
            return;
        };
        let (listener, addr) = std_listener();
        let client = tokio::net::TcpStream::connect(addr).await.unwrap();
        let outcome = read_after_close_with_child_alive(listener, client);
        assert!(
            matches!(&outcome, Err(e) if matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock)),
            "tokio's connect makes an inheritable socket on Windows (mio 1.1.1 \
             `new_socket` calls plain `socket()`), so the child should have held the \
             connection open and the read should have timed out; got {outcome:?}"
        );
        report.passed();
    }
}
