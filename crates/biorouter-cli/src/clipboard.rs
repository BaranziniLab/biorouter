//! The system clipboard, for `biorouter dictate --copy` (DD-86) and the TUI's copy.
//!
//! One helper, so every platform's rule lives in one place:
//! * macOS: set and exit; the pasteboard keeps the text.
//! * Windows: set, always excluded from the Windows cloud clipboard (arboard's
//!   `exclude_from_cloud`, which adds the `CanUploadToCloudClipboard` format).
//! * Linux: X11 and Wayland keep a copy only while its owner runs, so a short lived
//!   process waits up to 60 s for the paste, saying so (MSG-CLIPBOARD-WAIT) if the copy is
//!   still being served after 300 ms; a long lived one (the TUI) keeps its clipboard open
//!   for as long as it runs, which is the fix for the copy the TUI used to drop as soon as
//!   it was made.
//! * WSL: `powershell.exe Set-Clipboard`, which sets no cloud clipboard exclusion and no
//!   history exclusion, so `--copy --no-history` is refused there (MSG-NO-HISTORY-WSL,
//!   exit 2) and the WSL backend refuses a copy asked to stay out of history.
//! * No display: nothing is tried; the caller prints the text (MSG-NO-CLIPBOARD, exit 5).
//!
//! `--no-history` adds arboard's `exclude_from_history`, which clipboard managers honour.
//! The backend is a parameter, so the rules are tested without a clipboard.

use std::time::{Duration, Instant};

/// How long a short lived process serves its copy on Linux.
pub const LINUX_WAIT: Duration = Duration::from_secs(60);
/// After this long still serving, MSG-CLIPBOARD-WAIT is shown.
pub const WAIT_NOTICE_AFTER: Duration = Duration::from_millis(300);
/// Environment only, and read only by a debug build: `none`, `wsl`, or
/// `expect:<text>` stands in for the platform's clipboard. The last form copies only
/// when the backend receives that exact text. A release build ignores it.
pub const TEST_CLIPBOARD_ENV: &str = "BIOROUTER_DICTATION_TEST_CLIPBOARD";

/// Where the clipboard is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Windows,
    /// X11 or Wayland: a display is set.
    Linux,
    /// Windows Subsystem for Linux: the Windows clipboard, through `powershell.exe`.
    Wsl,
    /// No clipboard reachable: no display and not WSL.
    None,
    /// A debug build's clipboard that checks the text it receives.
    #[cfg(debug_assertions)]
    Test,
}

impl Platform {
    /// This process's clipboard, from the build target and the environment.
    pub fn current() -> Platform {
        if cfg!(debug_assertions) {
            match std::env::var(TEST_CLIPBOARD_ENV).as_deref() {
                Ok("none") => return Platform::None,
                Ok("wsl") => return Platform::Wsl,
                #[cfg(debug_assertions)]
                Ok(value) if value.starts_with("expect:") => return Platform::Test,
                _ => {}
            }
        }
        if cfg!(target_os = "macos") {
            return Platform::Mac;
        }
        if cfg!(target_os = "windows") {
            return Platform::Windows;
        }
        let set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty());
        let wsl = std::env::var_os("WSL_DISTRO_NAME").is_some()
            || std::fs::read_to_string("/proc/version")
                .is_ok_and(|v| v.to_ascii_lowercase().contains("microsoft"));
        Platform::of(wsl, set("DISPLAY") || set("WAYLAND_DISPLAY"))
    }

    /// Linux's choice: WSL first (its X server, if any, has no Windows clipboard), then a
    /// display, else none.
    pub fn of(wsl: bool, display: bool) -> Platform {
        if wsl {
            Platform::Wsl
        } else if display {
            Platform::Linux
        } else {
            Platform::None
        }
    }

    /// The paste key a sentence names.
    pub fn paste_key(self) -> &'static str {
        match self {
            Platform::Mac => "⌘V",
            #[cfg(debug_assertions)]
            Platform::Test if cfg!(target_os = "macos") => "⌘V",
            _ => "Ctrl+V",
        }
    }

    /// Whether `--no-history` can be honoured: everywhere a copy is made except WSL, whose
    /// `powershell.exe Set-Clipboard` cannot mark a copy to be left out of Windows
    /// clipboard history.
    pub fn honours_no_history(self) -> bool {
        self != Platform::Wsl
    }
}

/// How one copy is made.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SetOptions {
    pub exclude_from_history: bool,
    /// Windows only.
    pub exclude_from_cloud: bool,
    /// Linux only: serve the copy until it is replaced or until this deadline.
    pub wait_until: Option<Instant>,
}

/// A clipboard to set.
pub trait Backend {
    fn set_text(&mut self, text: &str, options: SetOptions) -> Result<(), String>;
}

/// How a copy went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Copied {
    Copied,
    /// No clipboard was reachable, or setting it failed: the caller prints the text.
    Unreachable,
}

/// The options a platform needs, before `no_history`.
pub fn options_for(platform: Platform, no_history: bool, now: Instant) -> SetOptions {
    SetOptions {
        exclude_from_history: no_history,
        exclude_from_cloud: platform == Platform::Windows,
        wait_until: (platform == Platform::Linux).then_some(now + LINUX_WAIT),
    }
}

/// Copies `text` as `dictate --copy` does. `on_wait` runs once if the copy is still being
/// served after [`WAIT_NOTICE_AFTER`] (Linux).
pub fn copy_with(
    backend: impl Backend + Send + 'static,
    platform: Platform,
    text: &str,
    no_history: bool,
    on_wait: impl FnOnce(),
) -> Copied {
    if platform == Platform::None {
        return Copied::Unreachable;
    }
    let options = options_for(platform, no_history, Instant::now());
    let result = if options.wait_until.is_some() {
        // The copy blocks while it serves; the notice comes from this thread.
        let text = text.to_string();
        let mut backend = backend;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(backend.set_text(&text, options));
        });
        match rx.recv_timeout(WAIT_NOTICE_AFTER) {
            Ok(result) => result,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                on_wait();
                rx.recv().unwrap_or_else(|_| Err("the copy stopped".into()))
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err("the copy stopped".into()),
        }
    } else {
        let mut backend = backend;
        backend.set_text(text, options)
    };
    match result {
        Ok(()) => Copied::Copied,
        Err(_) => Copied::Unreachable,
    }
}

/// The system clipboard through arboard.
pub struct Arboard;

impl Backend for Arboard {
    fn set_text(&mut self, text: &str, options: SetOptions) -> Result<(), String> {
        let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
        let set = clipboard.set();
        #[cfg(target_os = "windows")]
        let set = {
            use arboard::SetExtWindows;
            let set = if options.exclude_from_cloud {
                set.exclude_from_cloud()
            } else {
                set
            };
            if options.exclude_from_history {
                set.exclude_from_history()
            } else {
                set
            }
        };
        #[cfg(target_os = "macos")]
        let set = {
            use arboard::SetExtApple;
            if options.exclude_from_history {
                set.exclude_from_history()
            } else {
                set
            }
        };
        #[cfg(all(unix, not(target_os = "macos")))]
        let set = {
            use arboard::SetExtLinux;
            let set = match options.wait_until {
                Some(deadline) => set.wait_until(deadline),
                None => set,
            };
            if options.exclude_from_history {
                set.exclude_from_history()
            } else {
                set
            }
        };
        #[cfg(not(any(unix, target_os = "windows")))]
        let _ = options;
        set.text(text.to_string()).map_err(|e| e.to_string())
    }
}

/// The Windows clipboard from WSL, through `powershell.exe Set-Clipboard`. The text goes
/// on standard input as UTF-8, never on the command line. `Set-Clipboard` cannot keep a
/// copy out of clipboard history, so a copy asked to stay out of it is refused, never made
/// without the exclusion.
pub struct WslPowershell;

impl Backend for WslPowershell {
    fn set_text(&mut self, text: &str, options: SetOptions) -> Result<(), String> {
        use std::io::Write;
        if options.exclude_from_history {
            return Err("WSL cannot keep a copy out of Windows clipboard history".into());
        }
        let mut child = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[Console]::InputEncoding = [Text.Encoding]::UTF8; Set-Clipboard -Value ([Console]::In.ReadToEnd())",
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(text.as_bytes())
                .map_err(|e| e.to_string())?;
        }
        let status = child.wait().map_err(|e| e.to_string())?;
        status
            .success()
            .then_some(())
            .ok_or_else(|| format!("powershell.exe exited {status}"))
    }
}

#[cfg(debug_assertions)]
struct ExpectedText(String);

#[cfg(debug_assertions)]
impl Backend for ExpectedText {
    fn set_text(&mut self, text: &str, _options: SetOptions) -> Result<(), String> {
        if text == self.0.as_str() {
            Ok(())
        } else {
            Err("the text did not reach the test clipboard".into())
        }
    }
}

/// `dictate --copy` on this computer.
pub fn copy(text: &str, no_history: bool, on_wait: impl FnOnce()) -> (Copied, Platform) {
    let platform = Platform::current();
    #[cfg(debug_assertions)]
    if platform == Platform::Test {
        let Some(expected) = std::env::var(TEST_CLIPBOARD_ENV)
            .ok()
            .and_then(|value| value.strip_prefix("expect:").map(str::to_owned))
        else {
            return (Copied::Unreachable, platform);
        };
        return (
            copy_with(ExpectedText(expected), platform, text, no_history, on_wait),
            platform,
        );
    }
    let copied = match platform {
        Platform::Wsl => copy_with(WslPowershell, platform, text, no_history, on_wait),
        _ => copy_with(Arboard, platform, text, no_history, on_wait),
    };
    (copied, platform)
}

/// The TUI's copy, from a process that keeps running: the clipboard stays open on this
/// thread until the process ends, so on X11 and Wayland the copy is still there when the
/// person pastes. An empty text copies nothing.
pub fn write_kept(text: &str) {
    if text.is_empty() {
        return;
    }
    thread_local! {
        static KEPT: std::cell::RefCell<Option<arboard::Clipboard>> =
            const { std::cell::RefCell::new(None) };
    }
    KEPT.with(|kept| {
        let mut kept = kept.borrow_mut();
        if kept.is_none() {
            *kept = arboard::Clipboard::new().ok();
        }
        if let Some(clipboard) = kept.as_mut() {
            let _ = clipboard.set_text(text.to_string());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Fake {
        calls: Arc<Mutex<Vec<(String, SetOptions)>>>,
        hold: Duration,
        fail: bool,
    }

    impl Backend for Fake {
        fn set_text(&mut self, text: &str, options: SetOptions) -> Result<(), String> {
            std::thread::sleep(self.hold);
            self.calls.lock().unwrap().push((text.into(), options));
            if self.fail {
                Err("no clipboard".into())
            } else {
                Ok(())
            }
        }
    }

    fn one_call(fake: &Fake) -> SetOptions {
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "EPAS1 levels rose");
        calls[0].1
    }

    #[test]
    fn macos_sets_and_returns() {
        let fake = Fake::default();
        let mut waited = false;
        let copied = copy_with(
            fake.clone(),
            Platform::Mac,
            "EPAS1 levels rose",
            false,
            || waited = true,
        );
        assert_eq!(copied, Copied::Copied);
        assert!(!waited);
        assert_eq!(one_call(&fake), SetOptions::default());
    }

    /// DD-86: native Windows always leaves the text out of the cloud clipboard.
    #[test]
    fn windows_excludes_the_cloud_clipboard() {
        let fake = Fake::default();
        copy_with(
            fake.clone(),
            Platform::Windows,
            "EPAS1 levels rose",
            false,
            || {},
        );
        let options = one_call(&fake);
        assert!(options.exclude_from_cloud);
        assert!(!options.exclude_from_history);
        assert_eq!(options.wait_until, None);
    }

    /// Linux serves the copy for up to 60 s, and says so once 300 ms have passed.
    #[test]
    fn linux_waits_and_says_so_after_300_ms() {
        let fake = Fake {
            hold: Duration::from_millis(450),
            ..Fake::default()
        };
        let mut waited = 0;
        let started = Instant::now();
        let copied = copy_with(
            fake.clone(),
            Platform::Linux,
            "EPAS1 levels rose",
            false,
            || waited += 1,
        );
        assert_eq!(copied, Copied::Copied);
        assert_eq!(waited, 1);
        let deadline = one_call(&fake).wait_until.expect("a deadline");
        let wait = deadline.saturating_duration_since(started);
        assert!(
            wait > Duration::from_secs(59) && wait < LINUX_WAIT + Duration::from_secs(1),
            "{wait:?}"
        );
        // A copy taken at once says nothing.
        let quick = Fake::default();
        let mut said = false;
        copy_with(quick, Platform::Linux, "x", false, || said = true);
        assert!(!said);
    }

    #[test]
    fn no_history_excludes_clipboard_history_everywhere() {
        for platform in [
            Platform::Mac,
            Platform::Windows,
            Platform::Linux,
            Platform::Wsl,
        ] {
            let fake = Fake::default();
            copy_with(fake.clone(), platform, "EPAS1 levels rose", true, || {});
            assert!(one_call(&fake).exclude_from_history, "{platform:?}");
        }
    }

    /// No display and not WSL: nothing is tried, and the caller prints the text (exit 5).
    #[test]
    fn no_display_is_unreachable_without_trying() {
        let fake = Fake::default();
        let copied = copy_with(fake.clone(), Platform::None, "EPAS1", false, || {});
        assert_eq!(copied, Copied::Unreachable);
        assert!(fake.calls.lock().unwrap().is_empty());
        let failing = Fake {
            fail: true,
            ..Fake::default()
        };
        assert_eq!(
            copy_with(failing, Platform::Mac, "EPAS1", false, || {}),
            Copied::Unreachable
        );
    }

    /// WSL goes through `powershell.exe`, chosen before any display.
    #[test]
    fn the_platform_of_linux_hosts() {
        assert_eq!(Platform::of(true, true), Platform::Wsl);
        assert_eq!(Platform::of(false, true), Platform::Linux);
        assert_eq!(Platform::of(false, false), Platform::None);
        assert_eq!(Platform::Mac.paste_key(), "⌘V");
        assert_eq!(Platform::Linux.paste_key(), "Ctrl+V");
        let wsl = options_for(Platform::Wsl, false, Instant::now());
        assert_eq!(wsl, SetOptions::default(), "no cloud exclusion through WSL");
    }

    /// WSL's `Set-Clipboard` cannot keep a copy out of Windows clipboard history, so
    /// `--no-history` is not honoured there, and the WSL backend refuses such a copy before
    /// it starts `powershell.exe` rather than making it without the exclusion.
    #[test]
    fn wsl_refuses_a_copy_that_must_stay_out_of_history() {
        for platform in [Platform::Mac, Platform::Windows, Platform::Linux] {
            assert!(platform.honours_no_history(), "{platform:?}");
        }
        assert!(!Platform::Wsl.honours_no_history());
        let refused = WslPowershell.set_text(
            "EPAS1",
            SetOptions {
                exclude_from_history: true,
                ..SetOptions::default()
            },
        );
        assert!(refused.is_err());
        assert_eq!(
            copy_with(WslPowershell, Platform::Wsl, "EPAS1", true, || {}),
            Copied::Unreachable
        );
    }
}
