//! Command lines from typed arguments (AD-07): the only place an engine command is built.
//!
//! Rules every builder follows, and the tests below hold:
//! - a value the model chose is never a flag: options are always written `--flag=value`, and a
//!   positional value comes after `--`;
//! - `--headed` appears only where the AD-11 table says, and an action the platform cannot do is
//!   refused here instead of run;
//! - `--cdp`, `--force`, `--trace`, `--screenshot`, `--debug`, `--verbose` and launch options are
//!   never written;
//! - typed text goes to standard input.
use super::contract::{input_policy, Action, InputPolicy, Platform};
use super::runner::Invocation;
use std::time::Duration;

/// Observation deadlines (AD-15): 15 s, 30 s for Chromium and Electron apps, whose
/// accessibility tree takes 10 to 25 s to settle when cold.
pub const OBSERVE: Duration = Duration::from_secs(15);
pub const OBSERVE_SLOW: Duration = Duration::from_secs(30);
pub const ACT: Duration = Duration::from_secs(30);
pub const DRAG: Duration = Duration::from_secs(15);
/// The longest `wait_for`.
pub const MAX_WAIT: Duration = Duration::from_secs(30);
/// How long `drag` with `verify` waits for the expected element at the destination.
pub const DRAG_VERIFY: Duration = Duration::from_secs(10);

pub const UNSUPPORTED: &str = "This computer cannot do that action: it needs physical input, \
which this desktop session does not support (on Linux, a Wayland session has no pointer input). \
Use an accessibility action on a reference instead, or tell the person.";

/// What every command of one task shares.
#[derive(Clone, Debug)]
pub struct Ctx {
    pub platform: Platform,
    /// `--session`: the task's generation (1 to 64 letters, digits, `-` or `_`).
    pub session: String,
}

fn ms(duration: Duration) -> u128 {
    duration.as_millis()
}

fn session_id(session: &str) -> String {
    let cleaned: String = session
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(64)
        .collect();
    if cleaned.is_empty() {
        "biorouter".into()
    } else {
        cleaned
    }
}

struct Builder {
    args: Vec<String>,
    positional: Vec<String>,
    deadline: Duration,
    physical: bool,
}

impl Builder {
    fn new(
        ctx: &Ctx,
        command: &str,
        policy: InputPolicy,
        deadline: Duration,
    ) -> Result<Self, String> {
        let mut args = vec![command.to_owned()];
        let physical = match policy {
            InputPolicy::Unsupported => return Err(UNSUPPORTED.into()),
            InputPolicy::Headed => {
                args.push("--headed".into());
                true
            }
            InputPolicy::Headless => false,
        };
        args.push(format!("--session={}", session_id(&ctx.session)));
        Ok(Self {
            args,
            positional: Vec::new(),
            deadline,
            physical,
        })
    }

    fn observe(ctx: &Ctx, command: &str, deadline: Duration) -> Self {
        Self::new(ctx, command, InputPolicy::Headless, deadline).expect("headless never fails")
    }

    fn act(ctx: &Ctx, command: &str, action: Action, deadline: Duration) -> Result<Self, String> {
        Self::new(ctx, command, input_policy(action, ctx.platform), deadline)
    }

    fn opt(mut self, flag: &str, value: impl std::fmt::Display) -> Self {
        self.args.push(format!("--{flag}={value}"));
        self
    }

    fn opt_some(self, flag: &str, value: Option<&str>) -> Self {
        match value {
            Some(value) => self.opt(flag, value),
            None => self,
        }
    }

    fn switch(mut self, flag: &str) -> Self {
        self.args.push(format!("--{flag}"));
        self
    }

    /// A reference is validated by the schema and by provenance before it gets here, and starts
    /// with `@`, so it cannot be read as a flag.
    fn reference(mut self, reference: &str) -> Self {
        debug_assert!(reference.starts_with('@'));
        self.positional.push(reference.to_owned());
        self
    }

    fn positional(mut self, value: impl Into<String>) -> Self {
        self.positional.push(value.into());
        self
    }

    fn build(self, stdin: Option<String>) -> Invocation {
        let mut args = self.args;
        if !self.positional.is_empty() {
            args.push("--".into());
            args.extend(self.positional);
        }
        Invocation {
            args,
            stdin,
            deadline: self.deadline,
            physical: self.physical,
            releases_launched_apps: false,
        }
    }
}

pub fn list_apps(ctx: &Ctx) -> Invocation {
    Builder::observe(ctx, "list-apps", OBSERVE)
        .opt("timeout-ms", ms(OBSERVE))
        .build(None)
}

pub fn list_windows(ctx: &Ctx, app: Option<&str>) -> Invocation {
    Builder::observe(ctx, "list-windows", OBSERVE)
        .opt_some("app", app)
        .opt("timeout-ms", ms(OBSERVE))
        .build(None)
}

pub fn list_displays(ctx: &Ctx) -> Invocation {
    Builder::observe(ctx, "list-displays", OBSERVE).build(None)
}

/// `get_app_state`'s tree read.
#[derive(Clone, Debug, Default)]
pub struct SnapshotRequest<'a> {
    pub app: &'a str,
    pub window_id: Option<&'a str>,
    pub surface: Option<&'a str>,
    pub root: Option<&'a str>,
    pub max_depth: Option<u64>,
    pub interactive_only: bool,
    pub wait_for: Option<&'a str>,
    pub wait_for_gone: Option<&'a str>,
    pub wait: Option<Duration>,
    pub slow: bool,
}

pub fn snapshot(ctx: &Ctx, request: &SnapshotRequest<'_>) -> Invocation {
    let deadline = if request.slow { OBSERVE_SLOW } else { OBSERVE };
    let wait = request.wait.unwrap_or_default().min(MAX_WAIT);
    let mut builder = Builder::observe(ctx, "snapshot", deadline + wait)
        .opt("app", request.app)
        .opt_some("window-id", request.window_id)
        .opt_some("surface", request.surface)
        .opt_some("root", request.root)
        .opt("max-depth", request.max_depth.unwrap_or(10).clamp(1, 50))
        .switch("compact")
        .opt("timeout-ms", ms(deadline));
    if request.interactive_only {
        builder = builder.switch("interactive-only");
    }
    if request.wait_for.is_some() || request.wait_for_gone.is_some() {
        builder = builder
            .opt_some("wait-for", request.wait_for)
            .opt_some("wait-for-gone", request.wait_for_gone)
            .opt("wait-timeout", ms(wait));
    }
    builder.build(None)
}

/// A window's image saved to `path`, a file in the task's own folder.
pub fn screenshot_window(ctx: &Ctx, window_id: &str, path: &str) -> Invocation {
    Builder::observe(ctx, "screenshot", OBSERVE)
        .opt("window-id", window_id)
        .positional(path)
        .build(None)
}

pub fn screenshot_display(ctx: &Ctx, display: u64, path: &str) -> Invocation {
    Builder::observe(ctx, "screenshot", OBSERVE)
        .opt("screen", display)
        .positional(path)
        .build(None)
}

#[derive(Clone, Debug, Default)]
pub struct FindRequest<'a> {
    pub app: &'a str,
    pub window_id: Option<&'a str>,
    pub role: Option<&'a str>,
    pub name: Option<&'a str>,
    pub text: Option<&'a str>,
    pub value: Option<&'a str>,
    pub slow: bool,
}

pub fn find(ctx: &Ctx, request: &FindRequest<'_>) -> Invocation {
    let deadline = if request.slow { OBSERVE_SLOW } else { OBSERVE };
    Builder::observe(ctx, "find", deadline)
        .opt("app", request.app)
        .opt_some("window-id", request.window_id)
        .opt_some("role", request.role)
        .opt_some("name", request.name)
        .opt_some("text", request.text)
        .opt_some("value", request.value)
        .opt("limit", 50)
        .opt("timeout-ms", ms(deadline))
        .build(None)
}

/// `wait_for` with text, a window title, or only a pause.
pub enum WaitRequest<'a> {
    Text(&'a str),
    Window(&'a str),
    Pause,
}

pub fn wait(ctx: &Ctx, app: &str, request: WaitRequest<'_>, wait: Duration) -> Invocation {
    let wait = wait.min(MAX_WAIT);
    let builder = Builder::observe(ctx, "wait", wait);
    match request {
        WaitRequest::Text(text) => builder
            .opt("text", text)
            .opt("app", app)
            .opt("timeout", ms(wait))
            .build(None),
        WaitRequest::Window(title) => builder
            .opt("window", title)
            .opt("app", app)
            .opt("timeout", ms(wait))
            .build(None),
        WaitRequest::Pause => builder.positional(ms(wait).to_string()).build(None),
    }
}

/// `click` on a reference: a press, a context menu, or a physical double or triple click.
pub fn click_ref(
    ctx: &Ctx,
    reference: &str,
    count: u64,
    right: bool,
) -> Result<Invocation, String> {
    let (command, action) = match (count, right) {
        (1, false) => ("click", Action::ClickRef),
        (1, true) => ("right-click", Action::ShowMenu),
        (2, false) => ("double-click", Action::DoubleClickRef),
        (3, false) => ("triple-click", Action::TripleClickRef),
        _ => return Err("A right click is a single click; use count 1 with button right.".into()),
    };
    Ok(Builder::act(ctx, command, action, ACT)?
        .reference(reference)
        .build(None))
}

/// A physical click at a screen point (already converted from screenshot pixels).
pub fn click_point(
    ctx: &Ctx,
    x: f64,
    y: f64,
    count: u64,
    right: bool,
) -> Result<Invocation, String> {
    Ok(Builder::act(ctx, "mouse-click", Action::ClickPoint, ACT)?
        .opt("xy", format!("{},{}", x.round(), y.round()))
        .opt("button", if right { "right" } else { "left" })
        .opt("count", count.clamp(1, 3))
        .build(None))
}

pub fn secondary(
    ctx: &Ctx,
    reference: &str,
    action: &str,
    option: Option<&str>,
) -> Result<Invocation, String> {
    let operation = super::contract::secondary_action(action)
        .ok_or_else(|| format!("Unknown secondary action {action:?}."))?;
    let command = match action {
        "scroll_into_view" => "scroll-to",
        "show_menu" => "right-click",
        other => other,
    };
    let builder = Builder::act(ctx, command, operation, ACT)?.reference(reference);
    if action == "select" {
        let option = option.ok_or("select needs option: the visible name of the choice.")?;
        Ok(builder.positional(option).build(None))
    } else {
        Ok(builder.build(None))
    }
}

pub fn scroll(
    ctx: &Ctx,
    reference: &str,
    direction: &str,
    amount: u64,
) -> Result<Invocation, String> {
    Ok(Builder::act(ctx, "scroll", Action::Scroll, ACT)?
        .opt("direction", direction)
        .opt("amount", amount.clamp(1, 50))
        .reference(reference)
        .build(None))
}

/// One end of a drag: a reference, or a screen point already converted from screenshot pixels.
#[derive(Clone, Debug, PartialEq)]
pub enum End<'a> {
    Ref(&'a str),
    Point(f64, f64),
}

pub fn drag(
    ctx: &Ctx,
    from: End<'_>,
    to: End<'_>,
    verify: Option<&str>,
) -> Result<Invocation, String> {
    let deadline = DRAG
        + if verify.is_some() {
            DRAG_VERIFY
        } else {
            Duration::ZERO
        };
    let mut builder = Builder::act(ctx, "drag", Action::Drag, deadline)?;
    builder = match from {
        End::Ref(reference) => builder.opt("from", reference),
        End::Point(x, y) => builder.opt("from-xy", format!("{},{}", x.round(), y.round())),
    };
    builder = match to {
        End::Ref(reference) => builder.opt("to", reference),
        End::Point(x, y) => builder.opt("to-xy", format!("{},{}", x.round(), y.round())),
    };
    builder = builder.opt("timeout-ms", ms(DRAG));
    if let Some(selector) = verify {
        builder = builder
            .opt("wait-for", selector)
            .opt("wait-for-scope", "to")
            .opt("wait-timeout", ms(DRAG_VERIFY));
    }
    Ok(builder.build(None))
}

pub fn type_text(ctx: &Ctx, reference: &str, text: &str) -> Result<Invocation, String> {
    // The engine's own budget is 5 s plus 30 ms a character.
    let budget = Duration::from_millis(5_000 + 30 * text.chars().count() as u64);
    Ok(Builder::act(ctx, "type", Action::Type, ACT.max(budget))?
        .switch("stdin")
        .reference(reference)
        .build(Some(text.to_owned())))
}

pub fn set_value(ctx: &Ctx, reference: &str, value: &str) -> Result<Invocation, String> {
    Ok(Builder::act(ctx, "set-value", Action::SetState, ACT)?
        .switch("stdin")
        .reference(reference)
        .build(Some(value.to_owned())))
}

pub fn press(
    ctx: &Ctx,
    app: &str,
    combo: &str,
    window_id: Option<&str>,
) -> Result<Invocation, String> {
    let combo = combo.trim();
    if combo.is_empty() || combo.starts_with('-') {
        return Err("Give a key such as return, escape, tab or cmd+s.".into());
    }
    Ok(Builder::act(ctx, "press", Action::Press, ACT)?
        .opt("app", app)
        .opt_some("window-id", window_id)
        .positional(combo)
        .build(None))
}

pub fn open_app(ctx: &Ctx, app: &str) -> Result<Invocation, String> {
    let mut invocation = Builder::act(ctx, "launch", Action::Launch, ACT)?
        .switch("activate")
        .opt("timeout", ms(ACT))
        .positional(app)
        .build(None);
    invocation.releases_launched_apps = true;
    Ok(invocation)
}

/// The readiness probe's permission read; it never prompts.
pub fn permissions(ctx: &Ctx) -> Invocation {
    Builder::observe(ctx, "permissions", OBSERVE).build(None)
}

/// How long the operating system's permission prompts may stay open. macOS's Automation prompt
/// waits for the person's answer, so this is generous.
pub const REQUEST_PERMISSIONS: Duration = Duration::from_secs(120);

/// `permissions --request` (AD-04): the operating system asks the person to allow Biorouter
/// Copilot. Built only for the setup button a person pressed, never from a tool call.
pub fn request_permissions(ctx: &Ctx) -> Invocation {
    Builder::observe(ctx, "permissions", REQUEST_PERMISSIONS)
        .switch("request")
        .build(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(platform: Platform) -> Ctx {
        Ctx {
            platform,
            session: "0b7c1d2e-aaaa-bbbb-cccc-123456789abc".into(),
        }
    }

    const FORBIDDEN: &[&str] = &[
        "--cdp",
        "--force",
        "--trace",
        "--trace-strict",
        "--screenshot",
        "--debug",
        "--verbose",
        "-v",
        "--arg",
        "--env",
        "--cwd",
        "--no-attach",
    ];

    fn all_invocations(platform: Platform) -> Vec<Invocation> {
        let c = ctx(platform);
        let r = "@s1abc:e4";
        let mut out = vec![
            list_apps(&c),
            list_windows(&c, Some("TextEdit")),
            list_displays(&c),
            snapshot(
                &c,
                &SnapshotRequest {
                    app: "TextEdit",
                    interactive_only: true,
                    ..Default::default()
                },
            ),
            screenshot_window(&c, "w1", "/state/shot.png"),
            screenshot_display(&c, 0, "/state/shot.png"),
            find(
                &c,
                &FindRequest {
                    app: "TextEdit",
                    role: Some("button"),
                    ..Default::default()
                },
            ),
            wait(
                &c,
                "TextEdit",
                WaitRequest::Text("Saved"),
                Duration::from_secs(5),
            ),
            wait(
                &c,
                "TextEdit",
                WaitRequest::Pause,
                Duration::from_millis(500),
            ),
            permissions(&c),
            request_permissions(&c),
        ];
        out.extend(
            [
                click_ref(&c, r, 1, false),
                click_ref(&c, r, 1, true),
                click_ref(&c, r, 2, false),
                click_ref(&c, r, 3, false),
                click_point(&c, 10.0, 20.0, 1, false),
                secondary(&c, r, "expand", None),
                secondary(&c, r, "select", Some("Large")),
                secondary(&c, r, "focus", None),
                scroll(&c, r, "down", 3),
                drag(
                    &c,
                    End::Ref(r),
                    End::Point(5.0, 6.0),
                    Some("listitem:Report"),
                ),
                type_text(&c, r, "hello"),
                set_value(&c, r, "v"),
                press(&c, "TextEdit", "cmd+s", None),
                open_app(&c, "TextEdit"),
            ]
            .into_iter()
            .flatten(),
        );
        out
    }

    #[test]
    fn no_invocation_ever_writes_a_forbidden_flag() {
        for platform in [
            Platform::MacOs,
            Platform::Windows,
            Platform::LinuxX11,
            Platform::LinuxWayland,
        ] {
            for invocation in all_invocations(platform) {
                for arg in &invocation.args {
                    let flag = arg.split('=').next().unwrap();
                    assert!(
                        !FORBIDDEN.contains(&flag),
                        "{flag} in {:?}",
                        invocation.args
                    );
                }
                assert!(invocation.args.iter().any(|a| a.starts_with("--session=")));
            }
        }
    }

    #[test]
    fn model_strings_are_never_flags() {
        let c = ctx(Platform::MacOs);
        let snap = snapshot(
            &c,
            &SnapshotRequest {
                app: "--headed",
                window_id: Some("--cdp"),
                ..Default::default()
            },
        );
        assert!(snap.args.contains(&"--app=--headed".to_string()));
        assert!(snap.args.contains(&"--window-id=--cdp".to_string()));
        assert!(!snap.args.contains(&"--headed".to_string()));
        let select = secondary(&c, "@s1:e1", "select", Some("--force")).unwrap();
        let dashdash = select.args.iter().position(|a| a == "--").unwrap();
        assert_eq!(&select.args[dashdash + 1..], ["@s1:e1", "--force"]);
        let launch = open_app(&c, "-evil").unwrap();
        assert_eq!(launch.args.last().unwrap(), "-evil");
        assert_eq!(launch.args[launch.args.len() - 2], "--");
        assert!(press(&c, "TextEdit", "--force", None).is_err());
        let find = find(
            &c,
            &FindRequest {
                app: "A",
                name: Some("-x"),
                ..Default::default()
            },
        );
        assert!(find.args.contains(&"--name=-x".to_string()));
    }

    #[test]
    fn typed_text_goes_to_stdin_and_never_onto_the_command_line() {
        let c = ctx(Platform::MacOs);
        let typed = type_text(&c, "@s1:e2", "secret-looking text").unwrap();
        assert_eq!(typed.stdin.as_deref(), Some("secret-looking text"));
        assert!(typed.args.iter().all(|a| !a.contains("secret-looking")));
        assert!(typed.args.contains(&"--stdin".to_string()));
        let set = set_value(&c, "@s1:e2", "128 KiB of text").unwrap();
        assert_eq!(set.stdin.as_deref(), Some("128 KiB of text"));
        assert!(set.args.iter().all(|a| !a.contains("128 KiB")));
    }

    #[test]
    fn headed_appears_exactly_where_ad11_says() {
        let r = "@s1:e1";
        let headed = |i: &Invocation| i.args.contains(&"--headed".to_string());
        for platform in [Platform::MacOs, Platform::Windows, Platform::LinuxX11] {
            let c = ctx(platform);
            assert!(!headed(&click_ref(&c, r, 1, false).unwrap()));
            assert!(headed(&click_ref(&c, r, 2, false).unwrap()));
            assert!(headed(&click_ref(&c, r, 3, false).unwrap()));
            assert!(headed(&click_point(&c, 1.0, 1.0, 1, false).unwrap()));
            assert!(headed(
                &drag(&c, End::Ref(r), End::Ref("@s1:e2"), None).unwrap()
            ));
            assert!(!headed(&set_value(&c, r, "x").unwrap()));
            assert!(!headed(&secondary(&c, r, "expand", None).unwrap()));
            assert!(!headed(&press(&c, "A", "return", None).unwrap()));
            assert!(!headed(&open_app(&c, "A").unwrap()));
        }
        let mac = ctx(Platform::MacOs);
        let win = ctx(Platform::Windows);
        let x11 = ctx(Platform::LinuxX11);
        assert!(!headed(&type_text(&mac, r, "x").unwrap()));
        assert!(headed(&type_text(&win, r, "x").unwrap()));
        assert!(!headed(&type_text(&x11, r, "x").unwrap()));
        assert!(!headed(&click_ref(&mac, r, 1, true).unwrap()));
        assert!(headed(&click_ref(&win, r, 1, true).unwrap()));
        assert!(!headed(&scroll(&mac, r, "down", 3).unwrap()));
        assert!(headed(&scroll(&x11, r, "down", 3).unwrap()));
        assert!(!headed(&secondary(&mac, r, "focus", None).unwrap()));
        assert!(headed(&secondary(&win, r, "focus", None).unwrap()));
        for observe in all_invocations(Platform::MacOs).into_iter().filter(|i| {
            matches!(
                i.args[0].as_str(),
                "list-apps"
                    | "list-windows"
                    | "list-displays"
                    | "snapshot"
                    | "screenshot"
                    | "find"
                    | "wait"
                    | "permissions"
            )
        }) {
            assert!(!headed(&observe), "{:?}", observe.args);
        }
    }

    #[test]
    fn only_the_setup_request_asks_the_system_for_permissions() {
        let c = ctx(Platform::MacOs);
        assert!(!permissions(&c).args.contains(&"--request".to_string()));
        let request = request_permissions(&c);
        assert_eq!(request.args[0], "permissions");
        assert!(request.args.contains(&"--request".to_string()));
        assert!(!request.args.contains(&"--guide".to_string()));
        assert!(!request.physical);
        assert_eq!(request.deadline, REQUEST_PERMISSIONS);
        for invocation in all_invocations(Platform::MacOs)
            .into_iter()
            .filter(|i| i.args.contains(&"--request".to_string()))
        {
            assert_eq!(
                invocation.args, request.args,
                "only one builder may request"
            );
        }
    }

    #[test]
    fn wayland_refuses_pointer_input_before_anything_runs() {
        let c = ctx(Platform::LinuxWayland);
        assert_eq!(
            click_point(&c, 1.0, 1.0, 1, false).unwrap_err(),
            UNSUPPORTED
        );
        assert_eq!(
            drag(&c, End::Ref("@s1:e1"), End::Ref("@s1:e2"), None).unwrap_err(),
            UNSUPPORTED
        );
        assert_eq!(click_ref(&c, "@s1:e1", 2, false).unwrap_err(), UNSUPPORTED);
        assert_eq!(scroll(&c, "@s1:e1", "down", 1).unwrap_err(), UNSUPPORTED);
        assert!(click_ref(&c, "@s1:e1", 1, false).is_ok());
    }

    #[test]
    fn drag_maps_both_kinds_of_end_and_the_verification() {
        let c = ctx(Platform::MacOs);
        let invocation = drag(
            &c,
            End::Point(10.4, 20.6),
            End::Ref("@s1:e9"),
            Some("listitem:Report.txt"),
        )
        .unwrap();
        assert_eq!(
            invocation.args,
            [
                "drag",
                "--headed",
                "--session=0b7c1d2e-aaaa-bbbb-cccc-123456789abc",
                "--from-xy=10,21",
                "--to=@s1:e9",
                "--timeout-ms=15000",
                "--wait-for=listitem:Report.txt",
                "--wait-for-scope=to",
                "--wait-timeout=10000",
            ]
        );
        assert!(invocation.physical);
        assert_eq!(invocation.deadline, DRAG + DRAG_VERIFY);
        let plain = drag(&c, End::Ref("@s1:e1"), End::Ref("@s1:e2"), None).unwrap();
        assert!(!plain.args.iter().any(|a| a.starts_with("--wait-for")));
    }

    #[test]
    fn each_tool_maps_to_its_command() {
        let c = ctx(Platform::MacOs);
        let first = |i: Invocation| i.args[0].clone();
        assert_eq!(first(click_ref(&c, "@s1:e1", 1, false).unwrap()), "click");
        assert_eq!(
            first(click_ref(&c, "@s1:e1", 1, true).unwrap()),
            "right-click"
        );
        assert_eq!(
            first(click_ref(&c, "@s1:e1", 2, false).unwrap()),
            "double-click"
        );
        assert_eq!(
            first(click_ref(&c, "@s1:e1", 3, false).unwrap()),
            "triple-click"
        );
        assert!(click_ref(&c, "@s1:e1", 2, true).is_err());
        assert_eq!(
            first(click_point(&c, 1.0, 2.0, 2, true).unwrap()),
            "mouse-click"
        );
        for (action, command) in [
            ("expand", "expand"),
            ("collapse", "collapse"),
            ("scroll_into_view", "scroll-to"),
            ("show_menu", "right-click"),
            ("toggle", "toggle"),
            ("check", "check"),
            ("uncheck", "uncheck"),
            ("focus", "focus"),
            ("clear", "clear"),
        ] {
            assert_eq!(
                first(secondary(&c, "@s1:e1", action, None).unwrap()),
                command
            );
        }
        assert!(secondary(&c, "@s1:e1", "select", None).is_err());
        assert!(secondary(&c, "@s1:e1", "close", None).is_err());
        assert_eq!(first(open_app(&c, "Notes").unwrap()), "launch");
        assert!(open_app(&c, "Notes").unwrap().releases_launched_apps);
        assert!(
            !press(&c, "Notes", "return", None)
                .unwrap()
                .releases_launched_apps
        );
        let shot = screenshot_window(&c, "w1", "/s/shot.png");
        assert_eq!(&shot.args[shot.args.len() - 2..], ["--", "/s/shot.png"]);
        let pause = wait(&c, "A", WaitRequest::Pause, Duration::from_secs(90));
        assert_eq!(
            pause.args.last().unwrap(),
            "30000",
            "waits are capped at 30 s"
        );
    }

    #[test]
    fn the_session_is_always_a_valid_namespace() {
        assert_eq!(session_id("abc-DEF_123"), "abc-DEF_123");
        assert_eq!(session_id("a b/c"), "abc");
        assert_eq!(session_id(""), "biorouter");
        assert_eq!(session_id(&"x".repeat(100)).len(), 64);
    }

    /// The pinned engine accepts every option the wrapper writes. `<command> --help` never
    /// touches the desktop. Run with BIOROUTER_COMPUTER_USE_DIR naming a built payload, as the
    /// native workflow does.
    #[test]
    #[ignore = "needs a built payload in BIOROUTER_COMPUTER_USE_DIR; runs only --help"]
    fn every_option_the_wrapper_writes_exists_in_the_pinned_engine() {
        let root = std::env::var_os("BIOROUTER_COMPUTER_USE_DIR").expect("select the payload");
        let payload = super::super::manifest::validate(std::path::Path::new(&root), true)
            .expect("a valid payload");
        let mut checked = 0;
        for platform in [Platform::MacOs, Platform::Windows, Platform::LinuxX11] {
            for invocation in all_invocations(platform) {
                let command = &invocation.args[0];
                let output = std::process::Command::new(&payload.executable)
                    .arg(command)
                    .arg("--help")
                    .env_clear()
                    .output()
                    .expect("the engine runs");
                let help = String::from_utf8_lossy(&output.stdout);
                assert!(output.status.success(), "{command} --help failed");
                for arg in invocation.args.iter().take_while(|arg| *arg != "--") {
                    if let Some(flag) = arg.strip_prefix("--") {
                        let flag = flag.split('=').next().unwrap();
                        assert!(
                            help.contains(&format!("--{flag} "))
                                || help.contains(&format!("--{flag}\n")),
                            "{command} has no --{flag}"
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 50, "only {checked} options checked");
    }
}
