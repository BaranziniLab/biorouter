//! What Biorouter Copilot refuses before anything runs (AD-12 in
//! docs/design/copilot-agent-desktop.md).
//!
//! These are accident prevention around a cooperative agent and injected instructions, not a
//! boundary against a determined local program: the person's own shell can always run what it
//! likes. Every refusal is a sentence written for the model that names the reason and what to do
//! instead, and every list here is matched case-insensitively against both the process name and
//! the bundle identifier (or desktop id) the engine reports.
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// One running application as the engine lists it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppFacts {
    pub name: String,
    pub bundle_id: Option<String>,
    pub pid: u32,
}

pub const BIOROUTER_REFUSAL: &str =
    "That is Biorouter itself. Biorouter Copilot never observes or \
controls Biorouter's own windows, because they show other chats and Biorouter's own approval \
controls. For a web page use the browser_* tools; otherwise ask the person.";

pub const SECURITY_PROMPT_REFUSAL: &str = "That is a system security or password prompt. \
Biorouter Copilot never observes or controls those; ask the person to answer it themselves.";

pub const LOCKED_REFUSAL: &str = "The screen is locked or a secure system prompt holds the \
display, so Biorouter Copilot did nothing. Ask the person to unlock the computer, then try again.";

pub const SECURE_INPUT_REFUSAL: &str = "A password field somewhere on this computer holds the \
keyboard right now (secure keyboard input), so Biorouter Copilot will not send keys or text. Ask \
the person to finish or close it.";

pub const PASSWORD_FIELD_REFUSAL: &str = "That is a password field. Biorouter Copilot never types \
into, sets or clears password fields; ask the person to enter it themselves.";

pub const TERMINAL_REFUSAL: &str = "That app is a terminal. Typing, keys or drops there would run \
commands outside the Developer tools, their permission mode and folder access, so Biorouter \
Copilot only looks and clicks there. Use the Developer shell for commands, or ask the person.";

pub const PRIVACY_SETTINGS_REFUSAL: &str = "That is a privacy, security, users or sign-in \
settings page. Biorouter Copilot can look at it but never changes it; tell the person what to \
change.";

fn lower(value: &str) -> String {
    value.trim().to_lowercase()
}

fn base_name(name: &str) -> String {
    let lowered = lower(name);
    lowered
        .strip_suffix(".exe")
        .map(str::to_owned)
        .unwrap_or(lowered)
}

fn matches_any(facts: &AppFacts, names: &[&str], bundles: &[&str]) -> bool {
    let name = base_name(&facts.name);
    let bundle = facts.bundle_id.as_deref().map(lower).unwrap_or_default();
    names.iter().any(|candidate| name == *candidate)
        || (!bundle.is_empty()
            && bundles.iter().any(|candidate| {
                bundle == *candidate || bundle.starts_with(&format!("{candidate}."))
            }))
}

/// The desktop app's and Copilot's own bundle identifiers (and their helpers').
const BIOROUTER_BUNDLES: &[&str] = &[
    "com.electron.biorouter",
    "edu.ucsf.biorouter",
    "org.biorouter.computer-use",
];

pub fn is_biorouter_bundle(facts: &AppFacts) -> bool {
    matches_any(facts, &[], BIOROUTER_BUNDLES)
}

/// System security and password prompts (AD-12), plus the two password stores macOS ships.
pub fn is_security_prompt(facts: &AppFacts) -> bool {
    const NAMES: &[&str] = &[
        // macOS
        "securityagent",
        "coreautha",
        "coreauthd",
        "usernotificationcenter",
        "loginwindow",
        "screensaverengine",
        "keychain access",
        "passwords",
        // Windows
        "consent",
        "credentialuibroker",
        "logonui",
        "lockapp",
        // Linux
        "polkit-gnome-authentication-agent-1",
        "polkit-kde-authentication-agent-1",
        "polkit-mate-authentication-agent-1",
        "lxpolkit",
        "lxqt-policykit-agent",
        "xfce-polkit",
        "gcr-prompter",
        "gcr-viewer",
        "ssh-askpass",
        "ksshaskpass",
        "seahorse",
    ];
    const BUNDLES: &[&str] = &[
        "com.apple.securityagent",
        "com.apple.loginwindow",
        "com.apple.usernotificationcenter",
        "com.apple.screensaver.engine",
        "com.apple.keychainaccess",
        "com.apple.passwords",
        "org.gnome.seahorse",
    ];
    matches_any(facts, NAMES, BUNDLES) || base_name(&facts.name).starts_with("pinentry")
}

/// Terminal emulators: typing there runs commands outside Developer's shell (AD-12).
pub fn is_terminal(facts: &AppFacts) -> bool {
    const NAMES: &[&str] = &[
        "terminal",
        "iterm2",
        "iterm",
        "warp",
        "ghostty",
        "kitty",
        "alacritty",
        "wezterm",
        "wezterm-gui",
        "hyper",
        "tabby",
        "rio",
        "windowsterminal",
        "openconsole",
        "conhost",
        "cmd",
        "powershell",
        "pwsh",
        "powershell_ise",
        "wt",
        "mintty",
        "putty",
        "gnome-terminal",
        "gnome-terminal-server",
        "konsole",
        "xterm",
        "uxterm",
        "xfce4-terminal",
        "tilix",
        "terminator",
        "foot",
        "urxvt",
        "rxvt",
        "lxterminal",
        "mate-terminal",
        "qterminal",
        "sakura",
        "terminology",
        "guake",
        "yakuake",
        "ptyxis",
        "kgx",
        "blackbox",
        "warp-terminal",
        "cool-retro-term",
        "st",
    ];
    const BUNDLES: &[&str] = &[
        "com.apple.terminal",
        "com.googlecode.iterm2",
        "dev.warp.warp-stable",
        "dev.warp.warp",
        "com.mitchellh.ghostty",
        "net.kovidgoyal.kitty",
        "org.alacritty",
        "io.alacritty",
        "com.github.wez.wezterm",
        "co.zeit.hyper",
        "org.tabby",
        "com.raphaelamorim.rio",
        "org.gnome.terminal",
        "org.gnome.console",
        "org.gnome.ptyxis",
        "org.kde.konsole",
        "com.gexperts.tilix",
        "org.wezfurlong.wezterm",
        "com.mitchellh.ghostty",
    ];
    matches_any(facts, NAMES, BUNDLES)
}

/// The operating system's settings app.
pub fn is_settings_app(facts: &AppFacts) -> bool {
    const NAMES: &[&str] = &[
        "system settings",
        "system preferences",
        "systemsettings",
        "settings",
        "applicationframehost",
        "gnome-control-center",
        "systemsettings5",
        "sechealthui",
    ];
    const BUNDLES: &[&str] = &[
        "com.apple.systempreferences",
        "com.apple.settings",
        "org.gnome.settings",
        "org.kde.systemsettings",
        "windows.immersivecontrolpanel",
    ];
    matches_any(facts, NAMES, BUNDLES)
}

/// A settings window that holds privacy, security, users or sign-in controls. Looking is
/// allowed; acting is refused (AD-12). Windows Security is refused for acting whatever its
/// title.
pub fn is_privacy_settings(facts: &AppFacts, window_title: Option<&str>) -> bool {
    if base_name(&facts.name) == "sechealthui" {
        return true;
    }
    if !is_settings_app(facts) {
        return false;
    }
    const PANES: &[&str] = &[
        "privacy",
        "security",
        "users",
        "login",
        "sign-in",
        "sign in",
        "password",
        "touch id",
        "lock screen",
        "accounts",
        "family",
        "device management",
        "profiles",
        "online accounts",
        "sharing",
    ];
    let title = window_title.map(lower).unwrap_or_default();
    // An unknown title in a settings app is treated as a privacy pane: refusing is the safe
    // answer when the pane cannot be read.
    title.is_empty() || PANES.iter().any(|pane| title.contains(pane))
}

/// Which processes are Biorouter's own: the daemon, its ancestors (the desktop app, or the
/// terminal running the CLI), every descendant of the desktop main process
/// (`BIOROUTER_DESKTOP_MAIN_PID`), and every process whose executable lies inside the running
/// app's install folder.
#[derive(Clone, Debug, Default)]
pub struct Identity {
    pids: HashSet<u32>,
    install_root: Option<PathBuf>,
    executables: HashMap<u32, PathBuf>,
}

impl Identity {
    /// For tests and the readiness probe: an identity built from known facts.
    pub fn from_parts(
        pids: impl IntoIterator<Item = u32>,
        install_root: Option<PathBuf>,
        executables: HashMap<u32, PathBuf>,
    ) -> Self {
        Self {
            pids: pids.into_iter().collect(),
            install_root,
            executables,
        }
    }

    pub fn current() -> Self {
        use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::new().with_exe(UpdateKind::OnlyIfNotSet),
        );
        let parents: HashMap<u32, u32> = system
            .processes()
            .iter()
            .filter_map(|(pid, process)| {
                process
                    .parent()
                    .map(|parent| (pid.as_u32(), parent.as_u32()))
            })
            .collect();
        let executables: HashMap<u32, PathBuf> = system
            .processes()
            .iter()
            .filter_map(|(pid, process)| process.exe().map(|exe| (pid.as_u32(), exe.to_owned())))
            .collect();
        let own = std::process::id();
        let mut pids = HashSet::new();
        let mut cursor = Some(own);
        while let Some(pid) = cursor {
            if pid <= 1 || !pids.insert(pid) {
                break;
            }
            cursor = parents.get(&pid).copied();
        }
        let main = std::env::var("BIOROUTER_DESKTOP_MAIN_PID")
            .ok()
            .and_then(|value| value.trim().parse::<u32>().ok())
            .filter(|pid| *pid > 1);
        if let Some(main) = main {
            if system.process(Pid::from_u32(main)).is_some() {
                pids.insert(main);
                for &pid in parents.keys() {
                    if descends_from(pid, main, &parents) {
                        pids.insert(pid);
                    }
                }
            }
        }
        let install_root = std::env::current_exe()
            .ok()
            .and_then(|exe| dunce::canonicalize(exe).ok())
            .and_then(|exe| install_root(&exe));
        Self {
            pids,
            install_root,
            executables,
        }
    }

    /// Whether the process behind `facts` is Biorouter's own.
    pub fn owns(&self, facts: &AppFacts) -> bool {
        self.owns_pid(facts.pid) || is_biorouter_bundle(facts)
    }

    pub fn owns_pid(&self, pid: u32) -> bool {
        if self.pids.contains(&pid) {
            return true;
        }
        match (&self.install_root, self.executables.get(&pid)) {
            (Some(root), Some(exe)) => exe.starts_with(root),
            _ => false,
        }
    }
}

fn descends_from(pid: u32, ancestor: u32, parents: &HashMap<u32, u32>) -> bool {
    let mut cursor = pid;
    for _ in 0..256 {
        if cursor == ancestor {
            return true;
        }
        match parents.get(&cursor) {
            Some(&parent) if parent > 1 && parent != cursor => cursor = parent,
            _ => return false,
        }
    }
    false
}

/// The running app's install folder: the enclosing `.app` on macOS, or the folder holding
/// `resources/bin/` on Windows and Linux desktop installs. A CLI in a shared folder such as
/// `/usr/bin` has none, so the rule never claims unrelated programs beside it.
pub fn install_root(executable: &Path) -> Option<PathBuf> {
    if let Some(bundle) = executable.ancestors().find(|path| {
        path.extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
    }) {
        // The Copilot bundle sits inside the desktop app's bundle; claim the outermost one.
        let outermost = bundle
            .ancestors()
            .filter(|path| {
                path.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
            })
            .last()
            .unwrap_or(bundle);
        return Some(outermost.to_owned());
    }
    let bin = executable.parent()?;
    let resources = bin.parent()?;
    let named = |path: &Path, name: &str| {
        path.file_name()
            .is_some_and(|value| value.to_string_lossy().eq_ignore_ascii_case(name))
    };
    (named(bin, "bin") && named(resources, "resources"))
        .then(|| resources.parent().map(Path::to_owned))
        .flatten()
}

/// Host state the refusals read, behind a trait so tests can stub it.
pub trait HostProbe: Send + Sync {
    fn identity(&self) -> Identity;
    /// The screen is locked, or a secure desktop (sign-in, UAC) holds the display.
    fn screen_locked(&self) -> bool;
    /// macOS secure keyboard input is on: some app's password field holds the keyboard.
    fn secure_input(&self) -> bool;
}

/// The real probes.
pub struct SystemProbe;

impl HostProbe for SystemProbe {
    fn identity(&self) -> Identity {
        Identity::current()
    }

    fn screen_locked(&self) -> bool {
        os::screen_locked()
    }

    fn secure_input(&self) -> bool {
        os::secure_input()
    }
}

#[cfg(target_os = "macos")]
mod os {
    //! CoreGraphics' session dictionary and HIToolbox's secure input flag, looked up at run time
    //! so the daemon links neither framework.
    use std::ffi::{c_char, c_void, CStr};
    use std::sync::OnceLock;

    type CopySession = unsafe extern "C" fn() -> *const c_void;
    type CreateString = unsafe extern "C" fn(*const c_void, *const c_char, u32) -> *const c_void;
    type GetValue = unsafe extern "C" fn(*const c_void, *const c_void) -> *const c_void;
    type GetBoolean = unsafe extern "C" fn(*const c_void) -> u8;
    type Release = unsafe extern "C" fn(*const c_void);
    type SecureInput = unsafe extern "C" fn() -> u8;

    const UTF8: u32 = 0x0800_0100;

    struct CoreFunctions {
        copy_session: CopySession,
        create_string: CreateString,
        get_value: GetValue,
        get_boolean: GetBoolean,
        release: Release,
    }

    fn symbol(library: &CStr, name: &CStr) -> Option<*mut c_void> {
        // SAFETY: dlopen and dlsym receive NUL-terminated strings; the handle is never closed,
        // so the returned symbols stay valid for the life of the process.
        unsafe {
            let handle = libc::dlopen(library.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
            if handle.is_null() {
                return None;
            }
            let symbol = libc::dlsym(handle, name.as_ptr());
            (!symbol.is_null()).then_some(symbol)
        }
    }

    fn core() -> Option<&'static CoreFunctions> {
        static CORE: OnceLock<Option<CoreFunctions>> = OnceLock::new();
        CORE.get_or_init(|| {
            let graphics = c"/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics";
            let foundation = c"/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation";
            // SAFETY: each symbol is transmuted to the signature Apple documents for it.
            unsafe {
                Some(CoreFunctions {
                    copy_session: std::mem::transmute::<*mut c_void, CopySession>(symbol(
                        graphics,
                        c"CGSessionCopyCurrentDictionary",
                    )?),
                    create_string: std::mem::transmute::<*mut c_void, CreateString>(symbol(
                        foundation,
                        c"CFStringCreateWithCString",
                    )?),
                    get_value: std::mem::transmute::<*mut c_void, GetValue>(symbol(
                        foundation,
                        c"CFDictionaryGetValue",
                    )?),
                    get_boolean: std::mem::transmute::<*mut c_void, GetBoolean>(symbol(
                        foundation,
                        c"CFBooleanGetValue",
                    )?),
                    release: std::mem::transmute::<*mut c_void, Release>(symbol(
                        foundation,
                        c"CFRelease",
                    )?),
                })
            }
        })
        .as_ref()
    }

    fn session_flag(key: &CStr) -> Option<bool> {
        let core = core()?;
        // SAFETY: CoreFoundation objects created or copied here are released before return;
        // a value read with CFDictionaryGetValue is borrowed from the dictionary.
        unsafe {
            let session = (core.copy_session)();
            if session.is_null() {
                return None;
            }
            let name = (core.create_string)(std::ptr::null(), key.as_ptr(), UTF8);
            let value = if name.is_null() {
                std::ptr::null()
            } else {
                (core.get_value)(session, name)
            };
            let flag = if value.is_null() {
                Some(false)
            } else {
                Some((core.get_boolean)(value) != 0)
            };
            if !name.is_null() {
                (core.release)(name);
            }
            (core.release)(session);
            flag
        }
    }

    pub fn screen_locked() -> bool {
        session_flag(c"CGSSessionScreenIsLocked").unwrap_or(false)
    }

    pub fn secure_input() -> bool {
        static SECURE: OnceLock<Option<SecureInput>> = OnceLock::new();
        let function = SECURE.get_or_init(|| {
            let toolbox = c"/System/Library/Frameworks/Carbon.framework/Frameworks/HIToolbox.framework/HIToolbox";
            // SAFETY: IsSecureEventInputEnabled takes nothing and returns a Boolean.
            symbol(toolbox, c"IsSecureEventInputEnabled")
                .map(|symbol| unsafe { std::mem::transmute::<*mut c_void, SecureInput>(symbol) })
        });
        // SAFETY: the function pointer came from HIToolbox with the signature above.
        function.is_some_and(|function| unsafe { function() } != 0)
    }
}

#[cfg(windows)]
mod os {
    use windows_sys::Win32::System::StationsAndDesktops::{
        CloseDesktop, GetUserObjectInformationW, OpenInputDesktop, DESKTOP_READOBJECTS, UOI_NAME,
    };

    /// The input desktop is not this session's `Default` desktop: the sign-in screen, the lock
    /// screen or a UAC prompt holds the display, and none of them may be observed or driven.
    pub fn screen_locked() -> bool {
        // SAFETY: OpenInputDesktop returns an owned handle or null; it is closed below.
        let desktop = unsafe { OpenInputDesktop(0, 0, DESKTOP_READOBJECTS) };
        if desktop.is_null() {
            return true;
        }
        let mut name = [0u16; 64];
        let mut needed = 0u32;
        // SAFETY: the buffer is valid for its byte length.
        let read = unsafe {
            GetUserObjectInformationW(
                desktop,
                UOI_NAME,
                name.as_mut_ptr().cast(),
                std::mem::size_of_val(&name) as u32,
                &mut needed,
            )
        };
        // SAFETY: closes the handle opened above.
        unsafe {
            CloseDesktop(desktop);
        }
        if read == 0 {
            return true;
        }
        let length = name
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(name.len());
        !String::from_utf16_lossy(&name[..length]).eq_ignore_ascii_case("Default")
    }

    pub fn secure_input() -> bool {
        false
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod os {
    pub fn screen_locked() -> bool {
        super::super::linux::screen_locked()
    }

    pub fn secure_input() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, bundle: Option<&str>) -> AppFacts {
        AppFacts {
            name: name.into(),
            bundle_id: bundle.map(str::to_owned),
            pid: 4242,
        }
    }

    #[test]
    fn biorouter_is_recognised_by_bundle_whatever_its_name() {
        for bundle in [
            "com.electron.biorouter",
            "com.electron.biorouter.helper.Renderer",
            "org.biorouter.computer-use",
            "EDU.UCSF.BIOROUTER",
        ] {
            assert!(
                is_biorouter_bundle(&app("Anything", Some(bundle))),
                "{bundle}"
            );
        }
        assert!(!is_biorouter_bundle(&app(
            "Biorouter",
            Some("com.electron.biorouterx")
        )));
        assert!(!is_biorouter_bundle(&app(
            "TextEdit",
            Some("com.apple.TextEdit")
        )));
    }

    #[test]
    fn identity_covers_pids_and_the_install_folder() {
        let mut executables = HashMap::new();
        executables.insert(
            10,
            PathBuf::from("/Applications/Biorouter.app/Contents/MacOS/Biorouter"),
        );
        executables.insert(
            11,
            PathBuf::from("/Applications/TextEdit.app/Contents/MacOS/TextEdit"),
        );
        executables.insert(
            12,
            PathBuf::from("/Applications/Biorouter.app/Contents/Frameworks/Biorouter Helper (Renderer).app/Contents/MacOS/Biorouter Helper (Renderer)"),
        );
        let identity = Identity::from_parts(
            [77, 78],
            Some(PathBuf::from("/Applications/Biorouter.app")),
            executables,
        );
        assert!(identity.owns_pid(77));
        assert!(identity.owns_pid(10));
        assert!(identity.owns_pid(12));
        assert!(!identity.owns_pid(11));
        assert!(!identity.owns_pid(999));
        assert!(identity.owns(&app("Electron", Some("com.electron.biorouter"))));
    }

    #[test]
    fn the_install_root_is_the_outermost_app_or_the_desktop_resources_parent() {
        assert_eq!(
            install_root(Path::new(
                "/Applications/Biorouter.app/Contents/Resources/bin/biorouterd"
            )),
            Some(PathBuf::from("/Applications/Biorouter.app"))
        );
        assert_eq!(
            install_root(Path::new(
                "/Applications/Biorouter.app/Contents/Resources/computer-use/Biorouter Copilot.app/Contents/MacOS/agent-desktop"
            )),
            Some(PathBuf::from("/Applications/Biorouter.app"))
        );
        assert_eq!(
            install_root(Path::new("/opt/Biorouter/resources/bin/biorouterd")),
            Some(PathBuf::from("/opt/Biorouter"))
        );
        assert_eq!(install_root(Path::new("/usr/bin/biorouter")), None);
        assert_eq!(
            install_root(Path::new("/work/target/debug/biorouterd")),
            None
        );
    }

    #[test]
    fn descendants_are_found_through_the_parent_chain() {
        let parents: HashMap<u32, u32> = [(20, 10), (30, 20), (40, 1), (50, 50)].into();
        assert!(descends_from(30, 10, &parents));
        assert!(descends_from(10, 10, &parents));
        assert!(!descends_from(40, 10, &parents));
        assert!(
            !descends_from(50, 10, &parents),
            "a self-parented entry ends the walk"
        );
    }

    #[test]
    fn the_current_identity_includes_this_process() {
        let identity = Identity::current();
        assert!(identity.owns_pid(std::process::id()));
    }

    #[test]
    fn security_prompts_terminals_and_settings_are_recognised() {
        for facts in [
            app("SecurityAgent", None),
            app(
                "UserNotificationCenter",
                Some("com.apple.UserNotificationCenter"),
            ),
            app("loginwindow", Some("com.apple.loginwindow")),
            app("consent.exe", None),
            app("CredentialUIBroker.exe", None),
            app("LogonUI", None),
            app("pinentry-gnome3", None),
            app("gcr-prompter", None),
            app("Keychain Access", Some("com.apple.keychainaccess")),
        ] {
            assert!(is_security_prompt(&facts), "{facts:?}");
        }
        assert!(!is_security_prompt(&app(
            "TextEdit",
            Some("com.apple.TextEdit")
        )));
        for facts in [
            app("Terminal", Some("com.apple.Terminal")),
            app("iTerm2", Some("com.googlecode.iterm2")),
            app("Anything", Some("com.mitchellh.ghostty")),
            app("WindowsTerminal.exe", None),
            app("powershell.exe", None),
            app("gnome-terminal-server", Some("org.gnome.Terminal")),
            app("konsole", None),
        ] {
            assert!(is_terminal(&facts), "{facts:?}");
        }
        assert!(!is_terminal(&app("Notes", Some("com.apple.Notes"))));
        assert!(!is_terminal(&app(
            "Visual Studio Code",
            Some("com.microsoft.VSCode")
        )));
    }

    #[test]
    fn privacy_panes_refuse_acting_and_other_panes_do_not() {
        let settings = app("System Settings", Some("com.apple.systempreferences"));
        for title in [
            "Privacy & Security",
            "Users & Groups",
            "Login Items & Extensions",
            "Touch ID & Password",
            "Passwords",
        ] {
            assert!(is_privacy_settings(&settings, Some(title)), "{title}");
        }
        assert!(
            is_privacy_settings(&settings, None),
            "an unreadable pane is refused"
        );
        assert!(!is_privacy_settings(&settings, Some("Appearance")));
        assert!(!is_privacy_settings(&settings, Some("Displays")));
        assert!(!is_privacy_settings(
            &app("Notes", None),
            Some("Privacy notes")
        ));
        assert!(is_privacy_settings(
            &app("SecHealthUI.exe", None),
            Some("Windows Security")
        ));
        let windows = app("SystemSettings.exe", None);
        assert!(is_privacy_settings(&windows, Some("Privacy & security")));
        assert!(!is_privacy_settings(&windows, Some("Bluetooth & devices")));
    }
}
