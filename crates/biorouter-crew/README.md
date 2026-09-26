# BioRouter Crew broker

Crew provides small-team chat, files, and owned agent activity over existing SSH access. The desktop clients connect as their own Unix users to an ordinary-user broker on one Linux node. There is no public chat listener, privileged product service, or shared login credential.

## Build and install

On the intended Linux architecture, with the repository's Rust toolchain available:

```sh
cargo build --locked --release -p biorouter-crew
mkdir -p "$HOME/.local/bin"
install -m 700 target/release/biorouter-crew "$HOME/.local/bin/biorouter-crew"
```

Each member installs their own verified executable. This command builds only the broker and bridge; it does not require the desktop, a database server, Node, or a container runtime on the cluster. Use the correct target directory if `CARGO_TARGET_DIR` is set. A binary built on another operating system cannot be copied to Linux and used as a Linux executable.

## First workspace

In the BioRouter Crew tab, add a connection, expand **Hosting a new workspace?**, and choose **Prepare my hosting identity**. Use that public bootstrap key when starting the broker. Keep the private signer on the desktop.

```sh
umask 077
mkdir -p "$HOME/.local/share/biorouter-crew"
"$HOME/.local/bin/biorouter-crew" start \
  --state-dir "$HOME/.local/share/biorouter-crew/lab" \
  --bootstrap-key DEVICE_PUBLIC_KEY_HEX
"$HOME/.local/bin/biorouter-crew" status \
  --state-dir "$HOME/.local/share/biorouter-crew/lab"
```

Copy the verified status output into the desktop connection, authenticate through native SSH, and initialize the host identity. Invite colleagues using their Unix UIDs and their own desktop device public keys. Enrollment grants workspace presence; team and channel invitations grant their separate memberships. Preparing again recovers the same unused device after a desktop restart.

The host's home directory, journal, and device authority remain private to its Unix account. The shared socket accepts connections from other users, but kernel UID and signed requests authenticate every protected operation. The hosting account and host administrator remain trusted with stored content.

## Operating boundaries

- Linux local persistent storage is required. Network filesystems, automatic node failover, and incompatible kernel capabilities are explicitly refused.
- Native OpenSSH owns host trust, keys, keyboard-interactive authentication and jump routes. Crew never stores MFA answers or accepts changed host keys automatically.
- Private is the initial connection and workspace mode. Existing restricted content stays restricted after a switch to Public.
- Agent grants select context channels, destination, provider and an optional ordinary-user work directory. Local and remote access share these checks. Unsupported confinement disables execution rather than weakening the grant.
- The broker has explicit journal, state, message and attachment limits. Reaching a quota refuses new mutations and leaves existing reads available. Review the capacity and preservation section of the protocol before deploying a long-lived workspace.
- SSH encryption alone does not establish HIPAA compliance. Institutional endpoint approval, retention, access review, workstation controls and the documented host trust boundary remain necessary deployment considerations.

See the [protocol and rootless setup contract](../../docs/research/biorouter-crew/protocol-contract.md), [implementation plan](../../docs/research/biorouter-crew/implementation-plan.md), and [acceptance ledger](../../docs/research/biorouter-crew/implementation-status.md). Unexecuted matrix rows are not compatibility claims.
