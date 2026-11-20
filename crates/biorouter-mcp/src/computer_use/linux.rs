//! The Linux desktop facts Biorouter Copilot reads with a short-lived system tool. Linux only,
//! so none of these children can open a console window on Windows.

/// logind's `LockedHint` for this session, read through `loginctl` with a short bound. An
/// unreadable answer is not treated as locked: AT-SPI reaches no app behind a GNOME or KDE lock
/// screen anyway.
pub fn screen_locked() -> bool {
    let Some(session) = std::env::var_os("XDG_SESSION_ID") else {
        return false;
    };
    let Ok(mut child) = std::process::Command::new("loginctl")
        .arg("show-session")
        .arg(session)
        .args(["-p", "LockedHint", "--value"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
    let mut output = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        use std::io::Read;
        let _ = stdout.read_to_string(&mut output);
    }
    output.trim() == "yes"
}

/// The installed `at-spi2-core` version, read from the package manager.
pub fn atspi_version() -> Option<String> {
    for (program, args) in [
        ("dpkg-query", vec!["-W", "-f=${Version}", "at-spi2-core"]),
        ("rpm", vec!["-q", "--qf", "%{VERSION}", "at-spi2-core"]),
        ("pacman", vec!["-Q", "at-spi2-core"]),
    ] {
        let Ok(output) = std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
        else {
            continue;
        };
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            let version = text.split_whitespace().last().unwrap_or_default();
            let version = version.split(':').next_back().unwrap_or(version);
            if version.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                return Some(version.to_owned());
            }
        }
    }
    None
}
