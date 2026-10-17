//! OW-9: a setup job runs only its fixed scripts, over the person's own SSH login, against the
//! account's own files, and installs exactly the program it checked.
//!
//! The job tests run the real scripts: a scripted `ssh` hands each remote argument to
//! `/bin/sh -c` on this computer, as a login shell would, with `HOME` at a fake server home
//! whose path has spaces and repeats the user name, and with `uname` and `getconf` answering
//! as a Linux x86-64 server's do. The program sent is a script standing in for the broker.

use super::*;
use crate::crew::authentication;
use serde_json::json;
use std::fs;
use std::os::unix::fs::PermissionsExt;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const LOGIN: &str = "wanjun@lab-server";
const WORKSPACE_ID: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000000a1";

// ---------------------------------------------------------------------------------------------
// The scripts and what they print
// ---------------------------------------------------------------------------------------------

/// Every script reaches `/bin/sh` unchanged through any login shell's single quotes: no single
/// quote, no `!` (csh history), no newline (csh) and no backslash pair (fish).
#[test]
fn the_scripts_survive_every_login_shell() {
    for script in [
        PROBE_SCRIPT,
        INSTALL_SCRIPT,
        FOLDER_SCRIPT,
        CONTEXT_SCRIPT,
        STORAGE_FIX_SCRIPT,
    ] {
        assert!(!script.contains(['\'', '!', '\n']), "{script}");
        assert!(!script.contains("\\\\"), "{script}");
    }
    // The storage fix substitutes nothing: the server finds the folders itself.
    assert!(!STORAGE_FIX_SCRIPT.contains('@'));
    assert_eq!(
        remote(CONTEXT_SCRIPT),
        "exec /bin/sh -c '\"$HOME/.local/bin/biorouter-crew\" probe --output-format json'"
    );
    // The install's only substitutions are its five placeholders, each once.
    for placeholder in ["@JOB@", "@SIZE@", "@SHA@", "@VER@", "@PREV@"] {
        assert_eq!(
            INSTALL_SCRIPT.matches(placeholder).count(),
            1,
            "{placeholder}"
        );
    }
    assert_eq!(INSTALL_SCRIPT.matches('@').count(), 10);
    assert_eq!(FOLDER_SCRIPT.matches('@').count(), 2);
    assert!(!PROBE_SCRIPT.contains('@'));
}

fn hex_of(text: &str) -> String {
    super::super::hex(text.as_bytes())
}

#[test]
fn the_probe_is_read_strictly() {
    let output = format!(
        "Welcome to lab-server!\nBRCREW_PROBE uid 1022\nBRCREW_PROBE begin 1\nBRCREW_PROBE uid 1022\n\
         BRCREW_PROBE user_hex {}\nBRCREW_PROBE home_hex {}\nBRCREW_PROBE os Linux\n\
         BRCREW_PROBE arch x86_64\nBRCREW_PROBE kernel_hex {}\nBRCREW_PROBE glibc 2.35\n\
         BRCREW_PROBE tool shasum\nBRCREW_PROBE tool rm\nBRCREW_PROBE free_kib 123456\n\
         BRCREW_PROBE bin_dir dir\nBRCREW_PROBE binary file\nBRCREW_PROBE binary_owner 1022\n\
         BRCREW_PROBE binary_sha256 {}\nBRCREW_PROBE binary_exit 0\n\
         BRCREW_PROBE binary_version_hex {}\nnoise\nBRCREW_PROBE end\nBRCREW_PROBE uid 0\n",
        hex_of("wanjun"),
        hex_of("/home/a b/wanjun"),
        hex_of("5.15.0-141-generic"),
        "ab".repeat(32),
        hex_of("biorouter-crew 1.92.1\n"),
    );
    let probe = read_probe(output.as_bytes()).expect("a whole probe");
    assert_eq!(
        probe.uid,
        Some(1022),
        "only lines between begin and end count"
    );
    assert_eq!(probe.user.as_deref(), Some("wanjun"));
    assert_eq!(probe.home.as_deref(), Some("/home/a b/wanjun"));
    assert_eq!(probe.kernel.as_deref(), Some("5.15.0-141-generic"));
    assert_eq!(probe.glibc.as_deref(), Some("2.35"));
    assert_eq!(probe.tools, ["shasum"], "only tools it asks about");
    assert_eq!(probe.checksum_tool(), Some("shasum"));
    assert_eq!(probe.installed_version().as_deref(), Some("1.92.1"));
    // Cut short (no end), never begun, or a value outside its grammar.
    assert!(read_probe(b"BRCREW_PROBE begin 1\nBRCREW_PROBE uid 1\n").is_none());
    assert!(read_probe(b"BRCREW_PROBE uid 1\nBRCREW_PROBE end\n").is_none());
    let odd = read_probe(
        b"BRCREW_PROBE begin 1\nBRCREW_PROBE os Lin ux\nBRCREW_PROBE binary weird\n\
          BRCREW_PROBE binary_sha256 XYZ\nBRCREW_PROBE home_hex 2e2e\nBRCREW_PROBE end\n",
    )
    .unwrap();
    assert_eq!(
        (odd.os, odd.binary, odd.binary_sha256, odd.home),
        (None, None, None, None)
    );
    // The folders the storage walk named: absolute paths, each once.
    let walked = read_probe(
        format!(
            "BRCREW_PROBE begin 1\nBRCREW_PROBE storage_unsafe_hex {}\n\
             BRCREW_PROBE storage_unsafe_hex {}\nBRCREW_PROBE storage_unsafe_hex {}\n\
             BRCREW_PROBE storage_unsafe_hex zz\nBRCREW_PROBE end\n",
            hex_of("/home/a b/wanjun/.local"),
            hex_of("relative/.local"),
            hex_of("/home/a b/wanjun/.local"),
        )
        .as_bytes(),
    )
    .unwrap();
    assert_eq!(walked.storage_unsafe, ["/home/a b/wanjun/.local"]);
}

#[test]
fn the_storage_fix_report_is_read() {
    let report = format!(
        "motd\nBRCREW_STORAGE fixed {}\nBRCREW_STORAGE unsafe {}\nBRCREW_STORAGE done\n",
        hex_of("/home/w/.local"),
        hex_of("/home/w"),
    );
    assert_eq!(
        read_storage_fix(report.as_bytes()),
        Some(StorageFix {
            fixed: vec!["/home/w/.local".into()],
            left: vec!["/home/w".into()],
        })
    );
    assert_eq!(
        read_storage_fix(b"BRCREW_STORAGE fixed 2f68\n"),
        None,
        "no done line"
    );
    // The command a person is shown reads each folder back as one word.
    assert_eq!(
        shell_word("/pool1/home/wanjun/.local"),
        "/pool1/home/wanjun/.local"
    );
    assert_eq!(shell_word("/home/a b/it's"), "'/home/a b/it'\\''s'");
    // What a start printed, line by line, without control characters.
    assert_eq!(
        printed(
            b"{\"state\":\"x\"}\n\x1b[31m\n",
            "Error: unsafe_storage: x\u{7}\n\n"
        )
        .as_deref(),
        Some("{\"state\":\"x\"}\n[31m\nError: unsafe_storage: x")
    );
}

#[test]
fn the_install_report_is_read() {
    let done = format!(
        "BRCREW_INSTALL installed 0 {}\nBRCREW_INSTALL previous 1\nBRCREW_INSTALL done\n",
        hex_of(&format!("biorouter-crew {VERSION}"))
    );
    assert_eq!(
        read_install(done.as_bytes()),
        Installed::Done {
            exit: Some(0),
            output: Some(format!("biorouter-crew {VERSION}")),
            previous: true
        }
    );
    assert_eq!(
        read_install(b"motd\nBRCREW_INSTALL fail checksum_mismatch 0123\n"),
        Installed::Failed {
            reason: "checksum_mismatch".into(),
            rest: vec!["0123".into()]
        }
    );
    assert_eq!(
        read_install(b"BRCREW_INSTALL installed 0 aa\n"),
        Installed::Unknown,
        "no done line"
    );
    assert_eq!(
        read_folder(
            format!(
                "BRCREW_FOLDER path_hex {}\nBRCREW_FOLDER done\n",
                hex_of("/h/a b/crew-work/lab")
            )
            .as_bytes()
        ),
        Ok("/h/a b/crew-work/lab".into())
    );
    assert_eq!(
        read_folder(b"BRCREW_FOLDER fail foreign_owner\n"),
        Err("foreign_owner".into())
    );
    assert_eq!(
        read_facts(b"banner\n{\"schema\":1,\"cpus\":4}\n"),
        Some(json!({"schema": 1, "cpus": 4}))
    );
}

#[test]
fn versions_and_the_program_header() {
    assert_eq!(version_of("1.93.0"), Some((1, 93, 0)));
    for bad in ["1.93", "1.93.0.1", "1.x.0", "", "1..0", "v1.2.3"] {
        assert_eq!(version_of(bad), None, "{bad}");
    }
    assert!(version_of("1.10.0") > version_of("1.9.9"));
    assert_eq!(
        broker_version("biorouter-crew 1.92.1\nmore").as_deref(),
        Some("1.92.1")
    );
    assert_eq!(broker_version("Error: usage"), None);
    assert!(glibc_version("2.35") > glibc_version("2.31"));
    assert!(glibc_version("2.4") < glibc_version("2.31"));
    assert!(x86_64_elf(&elf(0x3e)));
    assert!(!x86_64_elf(&elf(0xb7)), "aarch64");
    assert!(!x86_64_elf(b"#!/bin/sh\necho biorouter-crew\n"));
}

/// A 64-bit little-endian ELF header for `machine`, padded.
fn elf(machine: u16) -> Vec<u8> {
    let mut bytes = vec![0x7f, b'E', b'L', b'F', 2, 1, 1, 0];
    bytes.resize(18, 0);
    bytes.extend_from_slice(&machine.to_le_bytes());
    bytes.resize(4096, 7);
    bytes
}

fn write_copy(dir: &Path, bytes: &[u8], manifest: Option<Value>) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("biorouter-crew"), bytes).unwrap();
    if let Some(manifest) = manifest {
        fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
    }
}

fn manifest_for(bytes: &[u8], version: &str) -> Value {
    json!({"schema": 1, "name": "biorouter-crew", "version": version,
        "target": "x86_64-unknown-linux-gnu", "glibc_max": "2.31",
        "sha256": super::super::hex(&Sha256::digest(bytes)), "size": bytes.len(),
        "protocol": 1, "capabilities": ["human_chat"], "source_sha": "0".repeat(40)})
}

fn refusal_of(error: anyhow::Error) -> (u16, &'static str, String) {
    let refused = error
        .downcast_ref::<SetupRefused>()
        .unwrap_or_else(|| panic!("not a typed refusal: {error:#}"));
    (refused.status, refused.code, refused.message.clone())
}

#[test]
fn a_copy_is_found_in_order_and_checked_against_its_manifest() {
    let root = tempfile::tempdir().unwrap();
    let exe = root.path().join("bin");
    let bundled = exe.join(BUNDLE_DIR);
    let ours = elf(0x3e);
    // The development folder first, then beside biorouterd, then the CLI packages' libexec.
    let found = places(Some(root.path().join("dev")), Some(&exe));
    assert_eq!(found[0], Place::Dev(root.path().join("dev")));
    assert_eq!(found[1], Place::Bundled(bundled.clone()));
    assert_eq!(
        found[2],
        Place::Bundled(exe.join("../libexec/biorouter").join(BUNDLE_DIR))
    );
    assert!(places(None, None).is_empty());
    // Nothing anywhere.
    let (status, code, message) = refusal_of(find_artifact(&found).err().unwrap());
    assert_eq!((status, code), (409, "crew_install_artifact_unavailable"));
    assert!(message.contains("none was found"), "{message}");

    // A bundled copy with its manifest.
    write_copy(&bundled, &ours, Some(manifest_for(&ours, VERSION)));
    let artifact = find_artifact(&found).unwrap();
    assert_eq!(artifact.info.source, "bundled");
    assert_eq!(artifact.info.version, VERSION);
    assert_eq!(artifact.info.size, 4096);
    assert_eq!(
        artifact.info.sha256,
        super::super::hex(&Sha256::digest(&ours))
    );
    assert_eq!(artifact.bytes, ours);

    // What a bundled copy may not be: unmanifested, another version, another program, or
    // bytes other than its manifest names. Each is the answer, never a fall-through.
    let bad: [(Vec<u8>, Option<Value>, &str); 6] = [
        (ours.clone(), None, "manifest.json"),
        (
            ours.clone(),
            Some(manifest_for(&ours, "0.0.1")),
            "names version 0.0.1",
        ),
        (
            elf(0xb7),
            Some(manifest_for(&elf(0xb7), VERSION)),
            "not an x86-64 Linux program",
        ),
        (
            ours.clone(),
            Some(manifest_for(&elf(0x3e)[..100], VERSION)),
            "SHA-256",
        ),
        (
            ours.clone(),
            Some(
                json!({"schema": 1, "name": "biorouter-crew", "version": VERSION,
                "target": "aarch64-unknown-linux-gnu",
                "sha256": super::super::hex(&Sha256::digest(&ours)), "size": 4096}),
            ),
            "aarch64",
        ),
        (
            ours.clone(),
            Some(
                json!({"schema": 1, "name": "biorouter-crew", "version": VERSION,
                "target": "x86_64-unknown-linux-gnu",
                "sha256": super::super::hex(&Sha256::digest(&ours)), "size": 4095}),
            ),
            "size",
        ),
    ];
    for (bytes, manifest, why) in bad {
        let _ = fs::remove_file(bundled.join("manifest.json"));
        write_copy(&bundled, &bytes, manifest);
        let (_, code, message) = refusal_of(find_artifact(&found).err().unwrap());
        assert_eq!(code, "crew_install_artifact_unavailable");
        assert!(message.contains(why), "{why}: {message}");
    }

    // A development copy comes first, needs no manifest, and may name another version only
    // when its manifest says it is a development copy.
    let dev = root.path().join("dev");
    write_copy(&dev, &ours, None);
    let artifact = find_artifact(&found).unwrap();
    assert_eq!(
        (
            artifact.info.source.as_str(),
            artifact.info.version.as_str()
        ),
        ("dev", VERSION)
    );
    write_copy(&dev, &ours, Some(manifest_for(&ours, "9.9.9")));
    assert!(refusal_of(find_artifact(&found).err().unwrap())
        .2
        .contains("names version 9.9.9"));
    let mut marked = manifest_for(&ours, "9.9.9");
    marked["source"] = json!("dev");
    write_copy(&dev, &ours, Some(marked));
    assert_eq!(find_artifact(&found).unwrap().info.version, "9.9.9");
}

#[test]
fn a_request_names_no_command_and_takes_only_its_fields() {
    let request: SetupRequest = serde_json::from_value(json!({
        "target": {"kind": "pending", "preparation_id": "p", "ssh_target": LOGIN},
        "intent": "host", "workspace_name": "lab"
    }))
    .unwrap();
    assert!(matches!(
        request.target,
        SetupTarget::Pending { port: None, .. }
    ));
    assert!(!request.replace_existing);
    // No agent access named leaves the connection's as it is; a fix is only ever asked for.
    assert_eq!(request.agent_access, None);
    assert!(!request.fix_storage_permissions);
    for (named, level) in [
        ("off", SetupAgentAccess::Off),
        ("folder", SetupAgentAccess::Folder),
        ("full", SetupAgentAccess::Full),
    ] {
        let request: SetupRequest = serde_json::from_value(json!({
            "target": {"kind": "connection", "connection_id": "c"}, "intent": "member",
            "agent_access": named, "fix_storage_permissions": true
        }))
        .unwrap();
        assert_eq!(request.agent_access, Some(level));
        assert!(request.fix_storage_permissions);
    }
    for smuggled in [
        json!({"target": {"kind": "connection", "connection_id": "c"}, "intent": "member", "agent_access": "root"}),
        json!({"target": {"kind": "connection", "connection_id": "c"}, "intent": "member", "command": "id"}),
        json!({"target": {"kind": "connection", "connection_id": "c", "script": "id"}, "intent": "member"}),
        json!({"target": {"kind": "elsewhere"}, "intent": "member"}),
        json!({"target": {"kind": "connection", "connection_id": "c"}, "intent": "root"}),
    ] {
        assert!(
            serde_json::from_value::<SetupRequest>(smuggled.clone()).is_err(),
            "{smuggled}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Jobs, against a fake server
// ---------------------------------------------------------------------------------------------

/// A test's fake server and this computer's side of it.
struct Server {
    root: tempfile::TempDir,
    manager: Arc<CrewManager>,
    _env: env_lock::EnvGuard<'static>,
}

impl Server {
    fn path(&self, relative: &str) -> PathBuf {
        self.root.path().join(relative)
    }
    /// The account's home on the fake server: spaces, and the user name twice.
    fn home(&self) -> PathBuf {
        self.path("remote home with spaces/wanjun/wanjun")
    }
    fn installed(&self) -> PathBuf {
        self.home().join(".local/bin/biorouter-crew")
    }
    fn flag(&self, name: &str) {
        fs::write(self.path(name), "").unwrap();
    }
    /// Put `bytes` in the account as its Crew program.
    fn install(&self, bytes: &[u8]) {
        let path = self.installed();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    /// Every remote argument the scripted `ssh` ran, in order.
    fn remotes(&self) -> Vec<String> {
        let mut calls: Vec<PathBuf> = fs::read_dir(self.path("calls"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| !path.file_name().unwrap().to_string_lossy().starts_with('.'))
            .collect();
        calls.sort();
        calls
            .iter()
            .map(|call| {
                let args: Vec<String> = fs::read(call)
                    .unwrap()
                    .split(|b| *b == 0)
                    .filter(|arg| !arg.is_empty())
                    .map(|arg| String::from_utf8(arg.to_vec()).unwrap())
                    .collect();
                assert!(
                    args.iter().any(|arg| arg == "BatchMode=yes"),
                    "every unattended ssh never prompts: {args:?}"
                );
                assert_eq!(args[args.len() - 2], LOGIN, "{args:?}");
                args.last().unwrap().clone()
            })
            .collect()
    }
    /// The calls' full argument lists, in order.
    fn calls(&self) -> Vec<Vec<String>> {
        let mut calls: Vec<PathBuf> = fs::read_dir(self.path("calls"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| !path.file_name().unwrap().to_string_lossy().starts_with('.'))
            .collect();
        calls.sort();
        calls
            .iter()
            .map(|call| {
                fs::read(call)
                    .unwrap()
                    .split(|b| *b == 0)
                    .filter(|arg| !arg.is_empty())
                    .map(|arg| String::from_utf8(arg.to_vec()).unwrap())
                    .collect()
            })
            .collect()
    }
}

/// The program sent in these tests: answers `--version`, `start`, `status`, `probe` and
/// `bridge` as the real broker would, from a script.
fn broker_script(version: &str, probes: bool) -> Vec<u8> {
    broker_with(version, probes, Arms::default())
}

/// The shell text of a stand-in program's `start`, `status` and `bridge` arms, where a test
/// needs other answers than [`broker_script`]'s.
#[derive(Default)]
struct Arms<'a> {
    start: Option<&'a str>,
    status: Option<&'a str>,
    bridge: Option<&'a str>,
}

/// [`broker_script`], with the arms `arms` gives.
fn broker_with(version: &str, probes: bool, arms: Arms) -> Vec<u8> {
    let probe = if probes {
        r#"printf '%s\n' '{"schema":1,"host":"lab-server","os":"Ubuntu 22.04.5 LTS","cpus":16,"scheduler":{"kind":null,"commands":[]},"storage":[{"path":"/pool1","fs":"zfs","size_gib":100,"avail_gib":90}]}'"#
    } else {
        r#"printf '%s\n' 'Error: usage: biorouter-crew <serve|start|status|stop|bridge>' >&2; exit 1"#
    };
    let start = arms.start.unwrap_or(
        r#"printf '%s\n' '{"workspace_id":"w","started_pid":42,"invitation":"brcrew1:TOKEN"}'"#,
    );
    let status = arms
        .status
        .unwrap_or(r#"printf '%s\n' '{"workspace_id":"w","socket":"/tmp/s"}'"#);
    let bridge = arms
        .bridge
        .unwrap_or(r#"printf '%s\n' 'Error: No such file or directory (os error 2)' >&2; exit 1"#);
    format!(
        r#"#!/bin/sh
case "$1" in
  --version) echo "biorouter-crew {version}" ;;
  start) {start} ;;
  status) {status} ;;
  probe) {probe} ;;
  bridge) {bridge} ;;
  *) printf '%s\n' 'Error: usage' >&2; exit 1 ;;
esac
"#
    )
    .into_bytes()
}

const NONCE: &str = "setup-hello-nonce";
const NODE: &str = "7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e";

fn workspace_key() -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&[31; 32])
}

/// `connection`'s workspace `hello` over [`NONCE`], v1-signed by [`workspace_key`].
fn signed_hello(connection: &Connection, capabilities: &[&str]) -> Value {
    use ed25519_dalek::Signer;
    let signature = super::super::hex(
        &workspace_key()
            .sign(&biorouter_crew::hello_v1_payload(
                &connection.workspace_id,
                connection.owner_uid,
                NONCE,
                &connection.workspace_public_key,
                NODE,
            ))
            .to_bytes(),
    );
    json!({"protocol": 1, "workspace_id": connection.workspace_id,
        "host_uid": connection.owner_uid, "workspace_public_key": connection.workspace_public_key,
        "challenge_nonce": NONCE, "node_id": NODE, "capabilities": capabilities,
        "signature": signature})
}

/// A `bridge` arm that answers `hello` with `hello`, `bridge.hello` with `full` (or, once
/// `no-full` exists under `root`, with `folder`), and anything else with an empty result, one
/// line per request, as a bridge and its workspace would.
fn answering_bridge(root: &Path, hello: &Value, full: &Value, folder: &Value) -> String {
    let (hello, full, folder) = (hello.to_string(), full.to_string(), folder.to_string());
    for text in [&hello, &full, &folder] {
        assert!(!text.contains('\''));
    }
    format!(
        r#"while IFS= read -r line; do
      id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
      case "$line" in
        *'"method":"bridge.hello"'*)
          b='{full}'; [ -f "{root}/no-full" ] && b='{folder}'
          printf '{{"id":"%s","result":%s}}\n' "$id" "$b" ;;
        *'"method":"hello"'*) printf '{{"id":"%s","result":%s}}\n' "$id" '{hello}' ;;
        *) printf '{{"id":"%s","result":{{}}}}\n' "$id" ;;
      esac
    done"#,
        root = root.display(),
    )
}

fn write_executable(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// A scripted `ssh` (see the module documentation). `-G` answers settings the preflight
/// accepts; `-M` stands in for a signed-in master; `-O check` reports it; anything else is
/// logged and its last argument run as the fake server's login shell would.
fn write_fake_ssh(root: &Path, home: &Path) {
    let script = format!(
        r#"#!/bin/sh
root='{root}'
home='{home}'
if [ "$1" = "-G" ]; then
  printf '%s\n' 'hostname 127.0.0.1' 'port 22' 'stricthostkeychecking yes' \
    'forwardagent no' 'forwardx11 no' 'permitlocalcommand no' \
    'clearallforwardings yes' 'nohostauthenticationforlocalhost no' \
    'tunnel no' 'forkafterauthentication no' \
    'gssapidelegatecredentials no' 'proxycommand none' \
    'controlmaster no' 'controlpersist no' 'controlpath none'
  exit 0
fi
for last; do :; done
case " $* " in
  *" -O exit "*)
    [ -f "$root/master.pid" ] && kill "$(cat "$root/master.pid")" 2>/dev/null
    exit 0 ;;
  *" -O "*)
    if [ -f "$root/master.pid" ] && kill -0 "$(cat "$root/master.pid")" 2>/dev/null; then
      printf 'Master running (pid=%s)\n' "$(cat "$root/master.pid")" >&2
      exit 0
    fi
    exit 255 ;;
  *" -M "*)
    printf '%s' "$$" > "$root/master.pid"
    : > "$root/authed"
    exec sleep 120 ;;
esac
n=$(ls "$root/calls" | wc -l | tr -d ' ')
# Written aside, then moved in whole, so a test reading the calls never reads half of one.
for arg; do printf '%s\0' "$arg"; done > "$root/calls/.$$"
mv "$root/calls/.$$" "$root/calls/$(printf %03d "$n")"
if [ -f "$root/refuse-key" ]; then
  printf '%s\n' 'wanjun@lab-server: Permission denied (publickey).' >&2
  exit 255
fi
if [ -f "$root/need-auth" ]; then
  # A password-only server: only a live signed-in master gets a command through.
  if [ ! -f "$root/master.pid" ] || ! kill -0 "$(cat "$root/master.pid")" 2>/dev/null; then
    printf '%s\n' 'wanjun@lab-server: Permission denied (publickey,password).' >&2
    exit 255
  fi
fi
path="$root/remote-bin:$PATH"
[ -f "$root/remote-path" ] && path="$root/remote-bin:$(cat "$root/remote-path")"
cd "$home" || exit 1
case "$last" in
  *BRCREW_INSTALL*)
    [ -f "$root/hang-install" ] && exec sleep 30
    if [ -f "$root/drop-install" ]; then
      rm -f "$root/drop-install"
      HOME="$home" PATH="$path" /bin/sh -c "$last" >/dev/null 2>&1
      exit 255
    fi ;;
esac
HOME="$home" PATH="$path" exec /bin/sh -c "$last"
"#,
        root = root.display(),
        home = home.display(),
    );
    write_executable(&root.join("bin/ssh"), &script);
}

/// A fake server, in a process of its own: its `ssh`, a home, and this computer's Crew
/// manager under a development profile. The program sent is `broker_script(VERSION)`.
async fn server() -> Server {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("remote home with spaces/wanjun/wanjun");
    fs::create_dir_all(&home).unwrap();
    // Writable by the account alone whatever this computer's umask, as a home is: a host's job
    // refuses to start under a folder others may write to.
    for folder in [
        "remote home with spaces",
        "remote home with spaces/wanjun",
        "remote home with spaces/wanjun/wanjun",
    ] {
        fs::set_permissions(root.path().join(folder), fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::create_dir_all(root.path().join("calls")).unwrap();
    fs::create_dir_all(root.path().join("profile/home/.ssh")).unwrap();
    write_fake_ssh(root.path(), &home);
    write_executable(
        &root.path().join("remote-bin/uname"),
        "#!/bin/sh\ncase \"$1\" in -s) echo Linux ;; -m) echo x86_64 ;; -r) echo 5.15.0-141-generic ;; *) echo Linux ;; esac\n",
    );
    write_executable(
        &root.path().join("remote-bin/getconf"),
        "#!/bin/sh\n[ \"$1\" = GNU_LIBC_VERSION ] && echo 'glibc 2.35'\n",
    );
    let path = format!(
        "{}:{}",
        root.path().join("bin").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let profile = root.path().join("profile").to_string_lossy().into_owned();
    let env = crate::test_sandbox::relocate_path_root_and(
        profile.as_str(),
        [
            ("BIOROUTER_DEV_PROFILE_ROOT", Some(profile.as_str())),
            ("BIOROUTER_DISABLE_KEYRING", Some("true")),
            ("PATH", Some(path.as_str())),
        ],
    );
    use_test_artifact(broker_script(VERSION, true));
    Server {
        manager: super::super::manager().unwrap(),
        root,
        _env: env,
    }
}

fn pending(preparation_id: &str, intent: SetupIntent) -> SetupRequest {
    SetupRequest {
        target: SetupTarget::Pending {
            preparation_id: preparation_id.into(),
            ssh_target: LOGIN.into(),
            port: None,
            identity_file: None,
            proxy_jump: None,
        },
        intent,
        workspace_name: (intent == SetupIntent::Host).then(|| "lab".into()),
        replace_existing: false,
        agent_access: None,
        fix_storage_permissions: false,
    }
}

/// A member's job for the saved connection `id`, asking for `agent_access`.
fn member(id: &str, agent_access: Option<SetupAgentAccess>) -> SetupRequest {
    SetupRequest {
        target: SetupTarget::Connection {
            connection_id: id.into(),
        },
        intent: SetupIntent::Member,
        workspace_name: Some("lab".into()),
        replace_existing: false,
        agent_access,
        fix_storage_permissions: false,
    }
}

async fn settled(job_id: &str) -> SetupStatus {
    // A minute: these jobs run real scripts, and a busy computer runs them slowly.
    for _ in 0..3000 {
        let status = setup_status(job_id).expect("the job is known");
        if !matches!(status.state, SetupState::Running | SetupState::NeedsSignIn) {
            return status;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the job never settled: {:?}", setup_status(job_id));
}

/// Start a job and wait for it to end.
async fn ran(server: &Server, request: SetupRequest) -> SetupStatus {
    let started = server.manager.start_setup(request).await.unwrap();
    settled(&started.job_id).await
}

fn error_code(status: &SetupStatus) -> &str {
    status
        .error
        .as_ref()
        .map(|error| error.code.as_str())
        .unwrap_or_else(|| panic!("no error: {status:?}"))
}

/// Every remote argument is one of the fixed scripts, wrapped; none carries request text.
fn assert_only_fixed_scripts(server: &Server, preparation_id: &str, slug: &str, key: Option<&str>) {
    let mut allowed = vec![
        remote(PROBE_SCRIPT),
        remote(CONTEXT_SCRIPT),
        remote(&FOLDER_SCRIPT.replace("@SLUG@", slug)),
        remote(STORAGE_FIX_SCRIPT),
    ];
    if let Some(key) = key {
        allowed.push(remote(
            &host_start::host_start_command(slug, key)
                .unwrap()
                .replace('\n', "; "),
        ));
    }
    for argument in server.remotes() {
        assert!(
            argument.starts_with("exec /bin/sh -c '") && argument.ends_with('\''),
            "{argument}"
        );
        assert!(
            !argument.contains(LOGIN) && !argument.contains(preparation_id),
            "{argument}"
        );
        if argument.contains("BRCREW_INSTALL") {
            let job = argument
                .split(".biorouter-crew.")
                .nth(1)
                .and_then(|rest| rest.get(..32))
                .unwrap();
            assert!(lower_hex_digest(job, 32), "{job}");
            let artifact = load_artifact().unwrap();
            let fill = |previous: &str| {
                remote(
                    &INSTALL_SCRIPT
                        .replace("@JOB@", job)
                        .replace("@SIZE@", &artifact.info.size.to_string())
                        .replace("@SHA@", &artifact.info.sha256)
                        .replace("@VER@", VERSION)
                        .replace("@PREV@", previous),
                )
            };
            let previous = argument
                .split("[ \"$(s \"$F\")\" = \"")
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .unwrap();
            assert!(
                previous == "none" || lower_hex_digest(previous, 64),
                "{previous}"
            );
            assert_eq!(argument, fill(previous));
        } else {
            assert!(
                allowed.contains(&argument),
                "not a fixed script: {argument}"
            );
        }
    }
}

fn no_leftovers(server: &Server) {
    let bin = server.home().join(".local/bin");
    let left: Vec<String> = fs::read_dir(&bin)
        .map(|entries| {
            entries
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|name| name.starts_with(".biorouter-crew"))
                .collect()
        })
        .unwrap_or_default();
    assert!(left.is_empty(), "left behind: {left:?}");
}

#[tokio::test]
async fn a_clean_install_puts_exactly_the_checked_program_in_place() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let server = server().await;
    let prepared = server.manager.prepare_device().await.unwrap();
    let done = ran(
        &server,
        pending(&prepared.preparation_id, SetupIntent::Install),
    )
    .await;
    assert_eq!(done.state, SetupState::Finished, "{done:?}");
    assert_eq!(
        fs::read(server.installed()).unwrap(),
        broker_script(VERSION, true)
    );
    assert_eq!(
        fs::metadata(server.installed())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    let outcome = done.outcome.unwrap();
    assert_eq!(outcome.installed_version.as_deref(), Some(VERSION));
    assert!(!outcome.reused && !outcome.previous_kept);
    assert!(!server
        .home()
        .join(".local/bin/biorouter-crew.previous")
        .exists());
    no_leftovers(&server);
    // What the check found, shown.
    assert_eq!(done.server.login, LOGIN);
    assert_eq!(done.server.os.as_deref(), Some("Linux"));
    assert_eq!(
        done.server.home.as_deref(),
        Some(server.home().to_str().unwrap())
    );
    assert_eq!(done.installed.unwrap().state, "missing");
    assert_eq!(done.artifact.unwrap().version, VERSION);
    let stages: Vec<_> = done
        .stages
        .iter()
        .map(|stage| (stage.name, stage.state))
        .collect();
    assert_eq!(
        stages,
        [
            (SetupStageName::Check, SetupStageState::Done),
            (SetupStageName::SignIn, SetupStageState::Skipped),
            (SetupStageName::Install, SetupStageState::Done),
            (SetupStageName::Done, SetupStageState::Done),
        ]
    );
    assert_eq!(server.remotes().len(), 2, "a probe and an install");
    assert_only_fixed_scripts(&server, &prepared.preparation_id, "lab", None);
    // Every step multiplexed over the preparation's own control path.
    let control = server
        .manager
        .control_path(&prepared.preparation_id)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    for call in server.calls() {
        let at = call.iter().position(|arg| arg == "-S").expect("-S");
        assert_eq!(call[at + 1], control);
    }
    // A check changes nothing and sends nothing.
    let checked = ran(
        &server,
        pending(&prepared.preparation_id, SetupIntent::Check),
    )
    .await;
    assert_eq!(checked.state, SetupState::Finished, "{checked:?}");
    assert_eq!(server.remotes().len(), 3);
}

#[tokio::test]
async fn the_same_version_is_reused_and_a_newer_one_never_replaced() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let server = server().await;
    let prepared = server.manager.prepare_device().await.unwrap();
    server.install(&broker_script(VERSION, true));
    let done = ran(
        &server,
        pending(&prepared.preparation_id, SetupIntent::Install),
    )
    .await;
    assert_eq!(done.state, SetupState::Finished, "{done:?}");
    let outcome = done.outcome.unwrap();
    assert!(outcome.reused);
    assert_eq!(outcome.installed_version.as_deref(), Some(VERSION));
    assert!(
        !server
            .remotes()
            .iter()
            .any(|r| r.contains("BRCREW_INSTALL")),
        "nothing was sent"
    );
    let install = done
        .stages
        .iter()
        .find(|stage| stage.name == SetupStageName::Install)
        .unwrap();
    assert_eq!(install.state, SetupStageState::Skipped);

    // A newer program stays, even when replacing was asked for.
    let newer = broker_script("99.0.0", true);
    server.install(&newer);
    let done = ran(
        &server,
        SetupRequest {
            replace_existing: true,
            ..pending(&prepared.preparation_id, SetupIntent::Install)
        },
    )
    .await;
    assert_eq!(done.state, SetupState::Finished, "{done:?}");
    assert_eq!(
        done.outcome.unwrap().installed_version.as_deref(),
        Some("99.0.0")
    );
    assert_eq!(fs::read(server.installed()).unwrap(), newer);
    assert!(!server
        .remotes()
        .iter()
        .any(|r| r.contains("BRCREW_INSTALL")));
}

#[tokio::test]
async fn an_older_program_is_replaced_only_when_asked_and_kept_as_previous() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let server = server().await;
    let prepared = server.manager.prepare_device().await.unwrap();
    let older = broker_script("1.0.0", false);
    server.install(&older);
    let kept = ran(
        &server,
        pending(&prepared.preparation_id, SetupIntent::Install),
    )
    .await;
    assert_eq!(kept.state, SetupState::Finished, "{kept:?}");
    let outcome = kept.outcome.unwrap();
    assert!(outcome.reused);
    assert_eq!(outcome.installed_version.as_deref(), Some("1.0.0"));
    assert_eq!(fs::read(server.installed()).unwrap(), older);

    let replaced = ran(
        &server,
        SetupRequest {
            replace_existing: true,
            ..pending(&prepared.preparation_id, SetupIntent::Install)
        },
    )
    .await;
    assert_eq!(replaced.state, SetupState::Finished, "{replaced:?}");
    let outcome = replaced.outcome.unwrap();
    assert!(!outcome.reused && outcome.previous_kept);
    assert_eq!(outcome.installed_version.as_deref(), Some(VERSION));
    assert_eq!(
        fs::read(server.installed()).unwrap(),
        broker_script(VERSION, true)
    );
    assert_eq!(
        fs::read(server.home().join(".local/bin/biorouter-crew.previous")).unwrap(),
        older
    );
    no_leftovers(&server);
    assert_only_fixed_scripts(&server, &prepared.preparation_id, "lab", None);
}

#[tokio::test]
async fn a_link_is_never_followed_or_replaced() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let server = server().await;
    let prepared = server.manager.prepare_device().await.unwrap();
    let elsewhere = server.path("elsewhere");
    fs::write(&elsewhere, "someone else's").unwrap();
    fs::create_dir_all(server.installed().parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&elsewhere, server.installed()).unwrap();
    let done = ran(
        &server,
        SetupRequest {
            replace_existing: true,
            ..pending(&prepared.preparation_id, SetupIntent::Install)
        },
    )
    .await;
    assert_eq!(done.state, SetupState::Failed);
    assert_eq!(error_code(&done), "crew_install_unexpected_target");
    assert_eq!(done.installed.as_ref().unwrap().state, "symlink");
    assert!(done
        .installed
        .unwrap()
        .problem
        .unwrap()
        .contains("is a link"));
    assert!(fs::symlink_metadata(server.installed())
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read_to_string(&elsewhere).unwrap(), "someone else's");
    assert!(!server
        .remotes()
        .iter()
        .any(|r| r.contains("BRCREW_INSTALL")));
}

#[tokio::test]
async fn a_corrupted_transfer_installs_nothing_and_keeps_the_old_program() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let server = server().await;
    let prepared = server.manager.prepare_device().await.unwrap();
    let older = broker_script("1.0.0", false);
    server.install(&older);
    // The server's `cat` changes bytes on the way in, keeping their count.
    write_executable(&server.path("remote-bin/cat"), "#!/bin/sh\nexec tr e f\n");
    let done = ran(
        &server,
        SetupRequest {
            replace_existing: true,
            ..pending(&prepared.preparation_id, SetupIntent::Install)
        },
    )
    .await;
    assert_eq!(done.state, SetupState::Failed, "{done:?}");
    assert_eq!(error_code(&done), "crew_install_checksum_mismatch");
    assert_eq!(fs::read(server.installed()).unwrap(), older);
    assert!(!server
        .home()
        .join(".local/bin/biorouter-crew.previous")
        .exists());
    no_leftovers(&server);
}

#[tokio::test]
async fn a_server_with_no_checksum_tool_is_refused_before_anything_is_sent() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let server = server().await;
    let prepared = server.manager.prepare_device().await.unwrap();
    // Every tool the scripts use, but none that computes a SHA-256.
    let tools = server.path("tools");
    fs::create_dir_all(&tools).unwrap();
    for tool in [
        "od", "tr", "id", "head", "cut", "df", "tail", "ls", "cat", "wc", "mkdir", "find", "rmdir",
        "rm", "chmod", "ln", "cp", "mv", "sync", "sleep",
    ] {
        let found = ["/bin", "/usr/bin"]
            .iter()
            .map(|dir| Path::new(dir).join(tool))
            .find(|path| path.exists())
            .unwrap_or_else(|| panic!("{tool}"));
        std::os::unix::fs::symlink(found, tools.join(tool)).unwrap();
    }
    fs::write(server.path("remote-path"), tools.to_str().unwrap()).unwrap();
    let done = ran(
        &server,
        pending(&prepared.preparation_id, SetupIntent::Install),
    )
    .await;
    assert_eq!(done.state, SetupState::Failed, "{done:?}");
    assert_eq!(error_code(&done), "crew_install_checksum_tool_missing");
    assert_eq!(done.server.checksum_tool, None);
    assert!(!server.installed().exists());
    assert!(!server
        .remotes()
        .iter()
        .any(|r| r.contains("BRCREW_INSTALL")));
}

#[tokio::test]
async fn another_install_under_way_is_waited_for_not_broken() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let server = server().await;
    let prepared = server.manager.prepare_device().await.unwrap();
    let lock = server
        .home()
        .join(".local/bin/.biorouter-crew.install-lock");
    fs::create_dir_all(&lock).unwrap();
    let done = ran(
        &server,
        pending(&prepared.preparation_id, SetupIntent::Install),
    )
    .await;
    assert_eq!(done.state, SetupState::Failed, "{done:?}");
    assert_eq!(error_code(&done), "crew_install_busy");
    assert!(lock.is_dir(), "another install's lock is left alone");
    assert!(!server.installed().exists());
}

#[tokio::test]
async fn a_cancelled_install_stops_and_installs_nothing() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let server = server().await;
    let prepared = server.manager.prepare_device().await.unwrap();
    server.flag("hang-install");
    let started = server
        .manager
        .start_setup(pending(&prepared.preparation_id, SetupIntent::Install))
        .await
        .unwrap();
    // A second click answers the job under way; another intent for the same server is refused.
    let again = server
        .manager
        .start_setup(pending(&prepared.preparation_id, SetupIntent::Install))
        .await
        .unwrap();
    assert_eq!(again.job_id, started.job_id);
    let other = server
        .manager
        .start_setup(pending(&prepared.preparation_id, SetupIntent::Check))
        .await
        .unwrap_err();
    assert_eq!(refusal_of(other).1, "crew_setup_busy");
    for _ in 0..500 {
        if server
            .remotes()
            .iter()
            .any(|r| r.contains("BRCREW_INSTALL"))
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(cancel_setup(&started.job_id));
    let done = settled(&started.job_id).await;
    assert_eq!(done.state, SetupState::Cancelled, "{done:?}");
    assert_eq!(error_code(&done), "crew_setup_cancelled");
    assert!(!server.installed().exists());
    assert!(!cancel_setup("unknown-job"));
    assert!(setup_status("unknown-job").is_none());
}

#[tokio::test]
async fn a_connection_lost_after_the_rename_is_settled_by_looking_again() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let server = server().await;
    let prepared = server.manager.prepare_device().await.unwrap();
    server.flag("drop-install");
    let done = ran(
        &server,
        pending(&prepared.preparation_id, SetupIntent::Install),
    )
    .await;
    assert_eq!(done.state, SetupState::Finished, "{done:?}");
    assert_eq!(
        done.outcome.unwrap().installed_version.as_deref(),
        Some(VERSION)
    );
    assert_eq!(
        fs::read(server.installed()).unwrap(),
        broker_script(VERSION, true)
    );
    let remotes = server.remotes();
    assert_eq!(remotes.len(), 3, "probe, install, probe again");
    assert_eq!(remotes[2], remote(PROBE_SCRIPT));
}

#[tokio::test]
async fn a_refused_key_fails_at_once_without_asking_to_sign_in() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let server = server().await;
    let prepared = server.manager.prepare_device().await.unwrap();
    server.flag("refuse-key");
    let done = ran(
        &server,
        pending(&prepared.preparation_id, SetupIntent::Install),
    )
    .await;
    assert_eq!(done.state, SetupState::Failed);
    assert_eq!(error_code(&done), "crew_ssh_key_refused");
    assert!(done
        .error
        .unwrap()
        .message
        .contains("didn't accept this computer's SSH key"));
    let sign_in = done
        .stages
        .iter()
        .find(|stage| stage.name == SetupStageName::SignIn)
        .unwrap();
    assert_eq!(sign_in.state, SetupStageState::Skipped);
}

/// A server that asks for a password parks the job until the person signs in through the
/// native terminal; the job then carries on over that sign-in's own master, and its window
/// says it is done.
#[tokio::test]
async fn a_password_server_waits_for_the_sign_in_then_carries_on() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let server = server().await;
    let prepared = server.manager.prepare_device().await.unwrap();
    server.flag("need-auth");
    let started = server
        .manager
        .start_setup(pending(&prepared.preparation_id, SetupIntent::Install))
        .await
        .unwrap();
    let request = uuid::Uuid::new_v4().to_string();
    let controller = uuid::Uuid::new_v4().to_string();
    let mut waiting = None;
    for _ in 0..500 {
        let status = setup_status(&started.job_id).unwrap();
        if status.state == SetupState::NeedsSignIn {
            waiting = Some(status);
            break;
        }
        assert_eq!(status.state, SetupState::Running, "{status:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let waiting = waiting.expect("the job asked for a sign-in");
    assert_eq!(waiting.stage, SetupStageName::SignIn);
    let session = prepare_setup_sign_in(&started.job_id, &request, &controller, 80, 24)
        .await
        .unwrap();
    assert_eq!(session.connection_id, prepared.preparation_id);
    // The same request is answered again; a second controller is refused.
    assert_eq!(
        prepare_setup_sign_in(&started.job_id, &request, &controller, 80, 24)
            .await
            .unwrap()
            .authentication_id,
        session.authentication_id
    );
    assert!(prepare_setup_sign_in(
        &started.job_id,
        &uuid::Uuid::new_v4().to_string(),
        &uuid::Uuid::new_v4().to_string(),
        80,
        24
    )
    .await
    .is_err());
    // A setup sign-in blocks a Connect on its key until it is done.
    assert!(authentication::ensure_connect_available(&prepared.preparation_id).is_err());
    let _output = authentication::attach(&session.authentication_id, &controller)
        .await
        .unwrap();
    let mut adopted = false;
    for _ in 0..200 {
        if authentication::handoff(&session.authentication_id, &controller)
            .await
            .unwrap()
        {
            adopted = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        adopted,
        "the setup sign-in was adopted once its master was up"
    );
    authentication::ensure_connect_available(&prepared.preparation_id).unwrap();
    let done = settled(&started.job_id).await;
    assert_eq!(done.state, SetupState::Finished, "{done:?}");
    let sign_in = done
        .stages
        .iter()
        .find(|stage| stage.name == SetupStageName::SignIn)
        .unwrap();
    assert_eq!(sign_in.state, SetupStageState::Done);
    assert_eq!(
        fs::read(server.installed()).unwrap(),
        broker_script(VERSION, true)
    );
    // Done, the job no longer takes a sign-in.
    let late = prepare_setup_sign_in(&started.job_id, &request, &controller, 80, 24)
        .await
        .err()
        .expect("refused");
    assert_eq!(refusal_of(late).1, "crew_setup_not_waiting_for_sign_in");
    authentication::cancel_connection(&prepared.preparation_id);
}

/// A host's job starts the workspace with this computer's own hosting key, over the same
/// control path, and answers what `/crew/host/start` would, the command it ran, the work
/// folder it made and the draft guide.
#[tokio::test]
async fn a_host_job_starts_the_workspace_over_its_own_control_path() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let server = server().await;
    let prepared = server.manager.prepare_device().await.unwrap();
    let done = ran(
        &server,
        pending(&prepared.preparation_id, SetupIntent::Host),
    )
    .await;
    assert_eq!(done.state, SetupState::Finished, "{done:?}");
    let outcome = done.outcome.clone().unwrap();
    assert_eq!(
        outcome.host,
        Some(StartOutput::Found {
            text: "brcrew1:TOKEN".into()
        })
    );
    let command = host_start::host_start_command("lab", &prepared.public_key).unwrap();
    assert_eq!(outcome.host_command.as_deref(), Some(command.as_str()));
    let folder = fs::canonicalize(server.home().join("crew-work/lab")).unwrap();
    assert_eq!(
        outcome.work_folder.as_deref(),
        Some(folder.to_str().unwrap())
    );
    assert_eq!(
        fs::metadata(&folder).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        outcome.remote_access, None,
        "a host's job saves no connection"
    );
    let draft = outcome.context_draft.unwrap();
    assert!(draft.starts_with("# Server guide: lab-server\n"), "{draft}");
    assert!(
        draft.contains("| /pool1 | zfs | 100 GiB | 90 GiB |"),
        "{draft}"
    );
    let stages: Vec<_> = done
        .stages
        .iter()
        .map(|stage| (stage.name, stage.state))
        .collect();
    assert_eq!(
        stages,
        [
            (SetupStageName::Check, SetupStageState::Done),
            (SetupStageName::SignIn, SetupStageState::Skipped),
            (SetupStageName::Install, SetupStageState::Done),
            (SetupStageName::Start, SetupStageState::Done),
            (SetupStageName::WorkFolder, SetupStageState::Done),
            (SetupStageName::Context, SetupStageState::Done),
            (SetupStageName::Done, SetupStageState::Done),
        ]
    );
    let start = remote(&command.replace('\n', "; "));
    let calls = server.calls();
    let started = calls
        .iter()
        .find(|call| call.last() == Some(&start))
        .expect("the start ran");
    assert!(
        started.iter().any(|arg| arg == "-S"),
        "multiplexed: {started:?}"
    );
    assert!(!started.iter().any(|arg| arg == "ControlPath=none"));
    assert_only_fixed_scripts(
        &server,
        &prepared.preparation_id,
        "lab",
        Some(&prepared.public_key),
    );

    // Only this computer's own, unused host setup; a host's job needs its name.
    let unknown = server
        .manager
        .start_setup(pending("someone-else", SetupIntent::Host))
        .await
        .unwrap_err();
    assert_eq!(refusal_of(unknown).1, "crew_host_setup_unknown");
    for bad in [
        SetupRequest {
            workspace_name: None,
            ..pending(&prepared.preparation_id, SetupIntent::Host)
        },
        SetupRequest {
            workspace_name: Some("lab; id".into()),
            ..pending(&prepared.preparation_id, SetupIntent::Host)
        },
        pending(&prepared.preparation_id, SetupIntent::Member),
        SetupRequest {
            target: SetupTarget::Pending {
                preparation_id: prepared.preparation_id.clone(),
                ssh_target: "-oProxyCommand=id".into(),
                port: None,
                identity_file: None,
                proxy_jump: None,
            },
            ..pending(&prepared.preparation_id, SetupIntent::Install)
        },
    ] {
        let refused = server.manager.start_setup(bad).await.unwrap_err();
        assert_eq!(refusal_of(refused).1, "crew_request_invalid");
    }
}

/// A saved connection, as the registry keeps it, to a fake server login.
fn saved(id: &str, remote_root: Option<&str>) -> Connection {
    let device = ed25519_dalek::SigningKey::from_bytes(&[7; 32])
        .verifying_key()
        .to_bytes();
    serde_json::from_value(json!({
        "id": id, "node_id": null, "name": "lab", "ssh_target": LOGIN, "port": null,
        "identity_file": null, "proxy_jump": null,
        "socket_path": "/tmp/crew-1000-0123456789abcdef0123456789abcdef/broker.sock",
        "owner_uid": 1000, "workspace_id": WORKSPACE_ID,
        "workspace_public_key": "ab".repeat(32), "remote_root": remote_root,
        "remote_execution": false, "cluster_connection_id": uuid::Uuid::new_v4().to_string(),
        "mode": "private", "institution_id": "ucsf", "policy_epoch": 1,
        "status": "disconnected", "last_error": null,
        "device_id": super::super::hex(&Sha256::digest(device)),
        "public_key": super::super::hex(&device),
    }))
    .unwrap()
}

/// A member's job installs and connects, and never starts a workspace: here the connect
/// finds no workspace server, so the job stops there, with the connect's own code.
#[tokio::test]
async fn a_member_job_never_starts_a_workspace() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let server = server().await;
    server
        .manager
        .registry
        .lock()
        .await
        .connections
        .push(saved("member-connection", None));
    let done = ran(
        &server,
        SetupRequest {
            workspace_name: None,
            ..member("member-connection", Some(SetupAgentAccess::Full))
        },
    )
    .await;
    assert_eq!(done.state, SetupState::Failed, "{done:?}");
    assert_eq!(error_code(&done), "crew_broker_not_running");
    assert_eq!(done.stage, SetupStageName::Connect);
    assert_eq!(
        fs::read(server.installed()).unwrap(),
        broker_script(VERSION, true)
    );
    for call in server.calls() {
        let argument = call.last().unwrap();
        assert!(!argument.contains(" start --state-dir"), "{argument}");
    }
    let unknown = server
        .manager
        .start_setup(member("nobody", None))
        .await
        .unwrap_err();
    assert_eq!(refusal_of(unknown).1, "crew_connection_not_found");
    let host = server
        .manager
        .start_setup(SetupRequest {
            intent: SetupIntent::Host,
            ..member("member-connection", None)
        })
        .await
        .unwrap_err();
    assert_eq!(refusal_of(host).1, "crew_request_invalid");
}

/// Sign-in worked but the server has no Crew program for this account: the master stays,
/// adopted, so setup rides it with no second password, and the connection says
/// `crew_bridge_missing` under the code older clients map.
#[tokio::test]
async fn a_sign_in_with_no_crew_behind_it_keeps_its_master() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let server = server().await;
    let id = "signed-in-connection";
    server
        .manager
        .registry
        .lock()
        .await
        .connections
        .push(saved(id, None));
    let request = uuid::Uuid::new_v4().to_string();
    let controller = uuid::Uuid::new_v4().to_string();
    let session = authentication::prepare(id, &request, &controller, 80, 24)
        .await
        .unwrap();
    let _output = authentication::attach(&session.authentication_id, &controller)
        .await
        .unwrap();
    let mut failure = None;
    for _ in 0..200 {
        match authentication::handoff(&session.authentication_id, &controller).await {
            Ok(false) => tokio::time::sleep(Duration::from_millis(25)).await,
            Ok(true) => panic!("adopted with no Crew behind it"),
            Err(error) => {
                failure = Some(error);
                break;
            }
        }
    }
    let failure = failure.expect("the handoff failed");
    let typed = failure
        .downcast_ref::<authentication::HandoffFailed>()
        .expect("typed");
    assert!(typed.bridge_missing(), "{}", typed.log_message());
    assert_eq!(typed.api_code(), "crew_handoff_failed");
    // Kept: adopted (a Connect may ride it), and its master is still running.
    authentication::ensure_connect_available(id).unwrap();
    authentication::validate(&session.authentication_id, &controller)
        .await
        .unwrap();
    let pid = fs::read_to_string(server.path("master.pid")).unwrap();
    assert_eq!(
        unsafe { libc::kill(pid.trim().parse().unwrap(), 0) },
        0,
        "the master is alive"
    );
    let connection = server.manager.connection(id).await.unwrap();
    assert_eq!(
        server.manager.last_error_code(&connection),
        Some("crew_bridge_missing")
    );
    assert_eq!(
        connection.last_error.as_deref(),
        Some(authentication::HANDOFF_FAILED_MESSAGE)
    );
    authentication::cancel_connection(id);
}

/// Wait for `job_id` to ask for a sign-in, then sign in through its terminal as a person would
/// (the scripted `ssh -M` stands in for typing the password) until the job adopts the master.
/// Answers the sign-in's ID and controller.
async fn sign_in(job_id: &str) -> (String, String) {
    for _ in 0..500 {
        let status = setup_status(job_id).expect("the job is known");
        if status.state == SetupState::NeedsSignIn {
            break;
        }
        assert_eq!(status.state, SetupState::Running, "{status:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let request = uuid::Uuid::new_v4().to_string();
    let controller = uuid::Uuid::new_v4().to_string();
    let session = prepare_setup_sign_in(job_id, &request, &controller, 80, 24)
        .await
        .unwrap();
    let _output = authentication::attach(&session.authentication_id, &controller)
        .await
        .unwrap();
    for _ in 0..200 {
        if authentication::handoff(&session.authentication_id, &controller)
            .await
            .unwrap()
        {
            return (session.authentication_id, controller);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the setup sign-in was never adopted");
}

/// Wait for `job_id` to stop running or to ask for a sign-in.
async fn asked_or_settled(job_id: &str) -> SetupStatus {
    for _ in 0..500 {
        let status = setup_status(job_id).expect("the job is known");
        if status.state != SetupState::Running {
            return status;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the job never got anywhere: {:?}", setup_status(job_id));
}

/// The pid of the scripted master the last sign-in started, and whether it still runs.
fn master(server: &Server) -> (i32, bool) {
    let pid: i32 = fs::read_to_string(server.path("master.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    (pid, alive(pid))
}

fn alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Whether `pid` ends within a few seconds (a killed master is reaped by its reader).
async fn ended(pid: i32) -> bool {
    for _ in 0..150 {
        if !alive(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

/// Saved connections `ids` to [`workspace_key`]'s workspace, and the program sent answering
/// its bridge: a workspace that takes full access, and a bridge that runs commands as the
/// account on a kernel that can't confine them (one that does not run them, once `no-full`
/// exists).
async fn bridged(server: &Server, ids: &[&str]) {
    *super::super::TEST_HELLO_NONCE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(NONCE.into());
    let public = super::super::hex(&workspace_key().verifying_key().to_bytes());
    let mut connections = Vec::new();
    for id in ids {
        let mut connection = saved(id, None);
        connection.workspace_public_key = public.clone();
        connections.push(connection);
    }
    let hello = signed_hello(
        &connections[0],
        &["human_chat", super::super::REMOTE_ACCESS_CAPABILITY],
    );
    server
        .manager
        .registry
        .lock()
        .await
        .connections
        .extend(connections);
    let full = json!({"bridge_version": VERSION,
        "capabilities": [super::super::BRIDGE_FULL_CAPABILITY, "remote_transfer_v1", "bridge_probe_v1"],
        "landlock_abi": 1, "confined_execution_supported": false, "user": "wanjun",
        "home": server.home().to_str().unwrap()});
    let folder = json!({"bridge_version": VERSION, "capabilities": [], "landlock_abi": 1,
        "confined_execution_supported": false});
    let bridge = answering_bridge(server.root.path(), &hello, &full, &folder);
    use_test_artifact(broker_with(
        VERSION,
        true,
        Arms {
            bridge: Some(&bridge),
            ..Arms::default()
        },
    ));
}

fn stage(status: &SetupStatus, name: SetupStageName) -> SetupStage {
    status
        .stages
        .iter()
        .find(|stage| stage.name == name)
        .cloned()
        .unwrap_or_else(|| panic!("no {name:?} stage: {status:?}"))
}

/// C10: the agent access a member's job sets is the one the person chose, and nothing when
/// they chose none: a connection set to Off, or one whose access the request did not name, is
/// never turned into full access because it has no folder.
#[tokio::test]
async fn a_member_job_sets_only_the_agent_access_asked_for() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let server = server().await;
    bridged(&server, &["unnamed", "off", "full", "fallback"]).await;
    let crew_work = server.home().join("crew-work");
    for (id, asked) in [("unnamed", None), ("off", Some(SetupAgentAccess::Off))] {
        let done = ran(&server, member(id, asked)).await;
        assert_eq!(done.state, SetupState::Finished, "{done:?}");
        assert_eq!(
            stage(&done, SetupStageName::WorkFolder).state,
            SetupStageState::Skipped
        );
        let outcome = done.outcome.unwrap();
        assert_eq!((outcome.work_folder, outcome.remote_access), (None, None));
        let connection = server.manager.connection(id).await.unwrap();
        assert_eq!(connection.status, "connected", "{id}");
        assert_eq!(
            (
                connection.remote_root,
                connection.remote_execution,
                connection.remote_access
            ),
            (None, false, RemoteAccess::Folder),
            "{id}: its agents still have no folder"
        );
    }
    assert!(
        !crew_work.exists(),
        "no folder is made for agents given none"
    );

    // As me on the server, where the workspace and the bridge take it.
    let done = ran(&server, member("full", Some(SetupAgentAccess::Full))).await;
    assert_eq!(done.state, SetupState::Finished, "{done:?}");
    let folder = fs::canonicalize(crew_work.join("lab")).unwrap();
    let folder = folder.to_str().unwrap();
    assert_eq!(
        done.outcome.as_ref().unwrap().remote_access,
        Some(RemoteAccess::Full)
    );
    let detail = stage(&done, SetupStageName::WorkFolder).detail.unwrap();
    assert!(detail.contains("work there as you"), "{detail}");
    let connection = server.manager.connection("full").await.unwrap();
    assert_eq!(connection.remote_root.as_deref(), Some(folder));
    assert!(connection.remote_execution);
    assert_eq!(connection.remote_access, RemoteAccess::Full);
    assert_eq!(connection.status, "connected");

    // Asked for, where the bridge does not take it: the folder, and why.
    server.flag("no-full");
    let done = ran(&server, member("fallback", Some(SetupAgentAccess::Full))).await;
    assert_eq!(done.state, SetupState::Finished, "{done:?}");
    assert_eq!(
        done.outcome.as_ref().unwrap().remote_access,
        Some(RemoteAccess::Folder)
    );
    let detail = stage(&done, SetupStageName::WorkFolder).detail.unwrap();
    assert!(detail.contains("Full access isn't available"), "{detail}");
    let connection = server.manager.connection("fallback").await.unwrap();
    assert_eq!(connection.remote_root.as_deref(), Some(folder));
    assert_eq!(connection.remote_access, RemoteAccess::Folder);
    assert!(
        !connection.remote_execution,
        "this kernel can't keep commands in the folder"
    );
}

/// C10: what [`CrewManager::apply_agent_access`] sets for each level, against what the last
/// verified connect found.
#[tokio::test]
async fn the_agent_access_set_is_the_level_asked_for_where_it_works() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let server = server().await;
    let manager = &server.manager;
    for (id, root) in [
        ("unknown", None),
        ("fresh", None),
        ("chosen", Some("/srv/lab/mine")),
    ] {
        manager
            .registry
            .lock()
            .await
            .connections
            .push(saved(id, root));
    }
    // Nothing known about the bridge yet: full access asked for is the folder, without
    // commands, since nothing says they could be confined.
    let applied = manager
        .apply_agent_access(
            "unknown",
            SetupAgentAccess::Full,
            "/h/crew-work/lab",
            VERSION,
        )
        .await
        .unwrap();
    assert_eq!(
        applied,
        AppliedAccess {
            access: RemoteAccess::Folder,
            execution: false,
            full_unavailable: true
        }
    );
    let unknown = manager.connection("unknown").await.unwrap();
    assert_eq!(unknown.remote_root.as_deref(), Some("/h/crew-work/lab"));
    assert_eq!(unknown.remote_access, RemoteAccess::Folder);
    let full = super::super::BridgeState {
        hello: super::super::BridgeHello {
            capabilities: vec![super::super::BRIDGE_FULL_CAPABILITY.into()],
            confined_execution_supported: Some(true),
            ..Default::default()
        },
        broker_takes_full: true,
    };
    for id in ["fresh", "chosen"] {
        manager.remember_bridge(id, full.clone());
    }
    assert_eq!(
        manager
            .apply_agent_access("fresh", SetupAgentAccess::Full, "/h/crew-work/lab", VERSION)
            .await
            .unwrap()
            .access,
        RemoteAccess::Full
    );
    let fresh = manager.connection("fresh").await.unwrap();
    assert_eq!(fresh.remote_root.as_deref(), Some("/h/crew-work/lab"));
    assert!(fresh.remote_execution);
    assert_eq!(fresh.remote_access, RemoteAccess::Full);
    // A folder the person chose stays the folder, at the level asked for.
    let applied = manager
        .apply_agent_access(
            "chosen",
            SetupAgentAccess::Folder,
            "/h/crew-work/lab",
            VERSION,
        )
        .await
        .unwrap();
    assert_eq!(
        (applied.access, applied.execution),
        (RemoteAccess::Folder, true)
    );
    let chosen = manager.connection("chosen").await.unwrap();
    assert_eq!(chosen.remote_root.as_deref(), Some("/srv/lab/mine"));
    assert_eq!(chosen.remote_access, RemoteAccess::Folder);
    // Off is never a level setup sets: it changes nothing.
    assert!(manager
        .apply_agent_access("chosen", SetupAgentAccess::Off, "/h/crew-work/lab", VERSION)
        .await
        .is_err());
    // A program that names no version takes no full access.
    assert_eq!(
        manager
            .apply_agent_access("chosen", SetupAgentAccess::Full, "/h", "unknown")
            .await
            .unwrap()
            .access,
        RemoteAccess::Folder
    );
}

/// C11: a password-only member signs in once, and ends setup connected with the access it
/// chose: saving the work folder drops the bridge, never the signed-in master it rides.
#[tokio::test]
async fn a_password_only_member_ends_setup_connected_on_its_one_sign_in() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let server = server().await;
    bridged(&server, &["member-pw"]).await;
    server.flag("need-auth");
    let started = server
        .manager
        .start_setup(member("member-pw", Some(SetupAgentAccess::Full)))
        .await
        .unwrap();
    let (authentication_id, controller) = sign_in(&started.job_id).await;
    let (pid, _) = master(&server);
    let done = settled(&started.job_id).await;
    assert_eq!(done.state, SetupState::Finished, "{done:?}");
    let connection = server.manager.connection("member-pw").await.unwrap();
    assert_eq!(connection.status, "connected", "{connection:?}");
    assert_eq!(connection.remote_access, RemoteAccess::Full);
    let outcome = done.outcome.clone().unwrap();
    assert_eq!(outcome.remote_access, Some(RemoteAccess::Full));
    assert!(
        outcome.context_draft.is_some(),
        "the context probe rode the master too: {done:?}"
    );
    assert!(
        alive(pid),
        "the master the person signed in with still runs"
    );
    assert_eq!(master(&server).0, pid, "and nobody signed in again");
    authentication::validate(&authentication_id, &controller)
        .await
        .unwrap();
    authentication::cancel_connection("member-pw");
}

/// C12: a setup sign-in's master serves only the login and route it signed in to: another job
/// for the same server with another login, or a connection saved with another, ends it.
#[tokio::test]
async fn a_setup_master_serves_only_the_route_it_signed_in_to() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let server = server().await;
    let prepared = server.manager.prepare_device().await.unwrap();
    let key = prepared.preparation_id.clone();
    server.flag("need-auth");
    let first = server
        .manager
        .start_setup(pending(&key, SetupIntent::Install))
        .await
        .unwrap();
    sign_in(&first.job_id).await;
    assert_eq!(settled(&first.job_id).await.state, SetupState::Finished);
    let (signed_in, _) = master(&server);
    // The same login again rides it, with no second sign-in.
    let again = ran(&server, pending(&key, SetupIntent::Check)).await;
    assert_eq!(again.state, SetupState::Finished, "{again:?}");
    assert_eq!(
        stage(&again, SetupStageName::SignIn).state,
        SetupStageState::Skipped
    );
    assert!(alive(signed_in));
    // Another login for the same host setup ends it, and asks for its own sign-in.
    let other = SetupRequest {
        target: SetupTarget::Pending {
            preparation_id: key.clone(),
            ssh_target: "labacct@lab-server".into(),
            port: None,
            identity_file: None,
            proxy_jump: None,
        },
        ..pending(&key, SetupIntent::Check)
    };
    let started = server.manager.start_setup(other).await.unwrap();
    // A job under way for one login is never answered for another.
    let busy = server
        .manager
        .start_setup(pending(&key, SetupIntent::Check))
        .await
        .unwrap_err();
    assert_eq!(refusal_of(busy).1, "crew_setup_busy");
    let asked = asked_or_settled(&started.job_id).await;
    assert_eq!(asked.state, SetupState::NeedsSignIn, "{asked:?}");
    assert!(ended(signed_in).await, "the other login's master ended");
    assert!(cancel_setup(&started.job_id));
    settled(&started.job_id).await;

    // Signed in again for the first login, the host setup is then saved with another: the
    // connect never rides the master the setup signed in with.
    let third = server
        .manager
        .start_setup(pending(&key, SetupIntent::Check))
        .await
        .unwrap();
    sign_in(&third.job_id).await;
    assert_eq!(settled(&third.job_id).await.state, SetupState::Finished);
    let (signed_in, running) = master(&server);
    assert!(running);
    authentication::ensure_connect_available(&key).unwrap();
    assert!(alive(signed_in), "nothing is saved under the key yet");
    server
        .manager
        .save(SaveConnection {
            preparation_id: Some(key.clone()),
            name: "lab".into(),
            ssh_target: "labacct@lab-server".into(),
            port: None,
            identity_file: None,
            proxy_jump: None,
            socket_path: "/tmp/crew-1000-0123456789abcdef0123456789abcdef/broker.sock".into(),
            owner_uid: 1000,
            workspace_id: WORKSPACE_ID.into(),
            workspace_public_key: "ab".repeat(32),
            remote_root: None,
            remote_execution: false,
            remote_access: RemoteAccess::Folder,
            cluster_connection_id: None,
            mode: super::super::ClusterMode::Public,
            institution_id: None,
        })
        .await
        .unwrap();
    authentication::ensure_connect_available(&key).unwrap();
    assert!(
        ended(signed_in).await,
        "a connection saved with another login ended the master"
    );
}

/// G2: a sign-in a setup job opened and nobody finished ends with the job, so the next job for
/// the same server can open its own; one left by a job this daemon no longer knows is replaced.
#[tokio::test]
async fn a_sign_in_left_open_by_an_ended_job_is_let_go() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let server = server().await;
    let prepared = server.manager.prepare_device().await.unwrap();
    let key = prepared.preparation_id.clone();
    server.flag("need-auth");
    let first = server
        .manager
        .start_setup(pending(&key, SetupIntent::Install))
        .await
        .unwrap();
    assert_eq!(
        asked_or_settled(&first.job_id).await.state,
        SetupState::NeedsSignIn
    );
    let controller = uuid::Uuid::new_v4().to_string();
    let session = prepare_setup_sign_in(
        &first.job_id,
        &uuid::Uuid::new_v4().to_string(),
        &controller,
        80,
        24,
    )
    .await
    .unwrap();
    let _output = authentication::attach(&session.authentication_id, &controller)
        .await
        .unwrap();
    // Nobody finishes it: the job is stopped.
    assert!(cancel_setup(&first.job_id));
    assert_eq!(settled(&first.job_id).await.state, SetupState::Cancelled);
    let mut gone = false;
    for _ in 0..100 {
        if authentication::validate(&session.authentication_id, &controller)
            .await
            .is_err()
        {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(gone, "the stopped job's sign-in ended with it");
    // A sign-in for a job this daemon no longer knows holds the key for nothing.
    let plan = AuthenticationPlan {
        program: "ssh".into(),
        args: vec!["-N".into()],
        connection_id: key.clone(),
        authentication_id: uuid::Uuid::new_v4().to_string(),
    };
    let orphan_controller = uuid::Uuid::new_v4().to_string();
    let orphan = authentication::prepare_setup(
        &server.manager,
        &key,
        "a-job-long-gone",
        vec![1, 2, 3],
        LOGIN,
        plan,
        &uuid::Uuid::new_v4().to_string(),
        &orphan_controller,
        80,
        24,
    )
    .await
    .unwrap();
    // The next job for the same server signs in afresh.
    let second = server
        .manager
        .start_setup(pending(&key, SetupIntent::Install))
        .await
        .unwrap();
    sign_in(&second.job_id).await;
    assert_eq!(settled(&second.job_id).await.state, SetupState::Finished);
    assert!(
        authentication::validate(&orphan.authentication_id, &orphan_controller)
            .await
            .is_err()
    );
    authentication::cancel_connection(&key);
}

/// L3: a host's job stops before its start when other accounts can write to a folder above
/// where the workspace is kept (the broker would refuse it), naming the folders and the
/// command; asked to, it takes that write access away from exactly the account's own folders
/// inside its home, and starts.
#[tokio::test]
async fn a_host_job_stops_before_its_start_where_others_can_write_above_the_workspace() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let server = server().await;
    let prepared = server.manager.prepare_device().await.unwrap();
    let local = server.home().join(".local");
    fs::create_dir_all(&local).unwrap();
    fs::set_permissions(&local, fs::Permissions::from_mode(0o775)).unwrap();
    let unsafe_folder = fs::canonicalize(&local)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let done = ran(
        &server,
        pending(&prepared.preparation_id, SetupIntent::Host),
    )
    .await;
    assert_eq!(done.state, SetupState::Failed, "{done:?}");
    assert_eq!(error_code(&done), "crew_host_storage_unsafe");
    assert_eq!(done.stage, SetupStageName::Start);
    assert_eq!(
        done.server.storage_unsafe,
        std::slice::from_ref(&unsafe_folder)
    );
    let error = done.error.clone().unwrap();
    assert_eq!(error.detail.as_deref(), Some(unsafe_folder.as_str()));
    assert!(
        error
            .message
            .contains(&format!("chmod go-w {}", shell_word(&unsafe_folder))),
        "{}",
        error.message
    );
    for call in server.calls() {
        assert!(
            !call.last().unwrap().contains(" start --state-dir"),
            "nothing started"
        );
    }
    assert_eq!(
        fs::metadata(&local).unwrap().permissions().mode() & 0o777,
        0o775,
        "nothing changed unasked"
    );

    let done = ran(
        &server,
        SetupRequest {
            fix_storage_permissions: true,
            ..pending(&prepared.preparation_id, SetupIntent::Host)
        },
    )
    .await;
    assert_eq!(done.state, SetupState::Finished, "{done:?}");
    assert_eq!(
        fs::metadata(&local).unwrap().permissions().mode() & 0o777,
        0o755
    );
    let detail = stage(&done, SetupStageName::Start).detail.unwrap();
    assert!(detail.contains(&unsafe_folder), "{detail}");
    assert!(done.server.storage_unsafe.is_empty());
    assert!(server.remotes().contains(&remote(STORAGE_FIX_SCRIPT)));
    assert_only_fixed_scripts(
        &server,
        &prepared.preparation_id,
        "lab",
        Some(&prepared.public_key),
    );

    // A folder above the home is never the fix's to change, even the account's own.
    let above = server.path("remote home with spaces/wanjun");
    fs::set_permissions(&above, fs::Permissions::from_mode(0o775)).unwrap();
    let done = ran(
        &server,
        SetupRequest {
            fix_storage_permissions: true,
            ..pending(&prepared.preparation_id, SetupIntent::Host)
        },
    )
    .await;
    assert_eq!(done.state, SetupState::Failed, "{done:?}");
    assert_eq!(error_code(&done), "crew_host_storage_unsafe");
    let above_real = fs::canonicalize(&above)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        done.error.unwrap().detail.as_deref(),
        Some(above_real.as_str())
    );
    assert_eq!(
        fs::metadata(&above).unwrap().permissions().mode() & 0o777,
        0o775
    );
}

/// L3: a start that fails says what Crew printed, and the broker's own storage refusal is said
/// as one.
#[tokio::test]
async fn a_failed_start_says_what_crew_printed() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let server = server().await;
    let prepared = server.manager.prepare_device().await.unwrap();
    let not_running =
        r#"printf '%s\n' 'Error: not_running: no workspace in this state directory' >&2; exit 1"#;
    use_test_artifact(broker_with(
        VERSION,
        true,
        Arms {
            start: Some(
                r#"printf '%s\n' 'Error: invalid_state: another broker holds this state directory' >&2; exit 1"#,
            ),
            status: Some(not_running),
            ..Arms::default()
        },
    ));
    let done = ran(
        &server,
        pending(&prepared.preparation_id, SetupIntent::Host),
    )
    .await;
    assert_eq!(done.state, SetupState::Failed, "{done:?}");
    assert_eq!(error_code(&done), "crew_setup_start_failed");
    let detail = done.error.unwrap().detail.unwrap();
    assert!(
        detail.contains("Error: invalid_state: another broker holds this state directory"),
        "{detail}"
    );
    assert!(detail.contains("Error: not_running"), "{detail}");

    use_test_artifact(broker_with(
        VERSION,
        true,
        Arms {
            start: Some(
                r#"printf '%s\n' 'Error: unsafe_storage: writable non-sticky ancestor' >&2; exit 1"#,
            ),
            status: Some(not_running),
            ..Arms::default()
        },
    ));
    let done = ran(
        &server,
        pending(&prepared.preparation_id, SetupIntent::Host),
    )
    .await;
    assert_eq!(done.state, SetupState::Failed, "{done:?}");
    assert_eq!(error_code(&done), "crew_host_storage_unsafe");
    assert!(done
        .error
        .unwrap()
        .detail
        .unwrap()
        .contains("unsafe_storage: writable non-sticky ancestor"));
}

/// Another build of this app's version (other bytes, the same `--version`) is replaced by the
/// copy this app checked, the one it replaces kept, and both SHA-256s said.
#[tokio::test]
async fn another_build_of_this_version_is_replaced_and_kept_as_previous() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let server = server().await;
    let prepared = server.manager.prepare_device().await.unwrap();
    let other_build = broker_script(VERSION, false);
    server.install(&other_build);
    let done = ran(
        &server,
        pending(&prepared.preparation_id, SetupIntent::Install),
    )
    .await;
    assert_eq!(done.state, SetupState::Finished, "{done:?}");
    let ours = broker_script(VERSION, true);
    let outcome = done.outcome.clone().unwrap();
    assert!(!outcome.reused && outcome.previous_kept);
    assert_eq!(
        outcome.upgraded_from,
        Some(SetupUpgrade {
            version: Some(VERSION.into()),
            sha256: Some(super::super::hex(&Sha256::digest(&other_build))),
            installed_version: VERSION.into(),
            installed_sha256: super::super::hex(&Sha256::digest(&ours)),
        })
    );
    assert_eq!(fs::read(server.installed()).unwrap(), ours);
    assert_eq!(
        fs::read(server.home().join(".local/bin/biorouter-crew.previous")).unwrap(),
        other_build
    );
    let detail = stage(&done, SetupStageName::Install).detail.unwrap();
    assert!(detail.contains("build"), "{detail}");
    no_leftovers(&server);
    // This app's own build again is kept as it is.
    let again = ran(
        &server,
        pending(&prepared.preparation_id, SetupIntent::Install),
    )
    .await;
    let outcome = again.outcome.unwrap();
    assert!(outcome.reused);
    assert_eq!(outcome.upgraded_from, None);
}
